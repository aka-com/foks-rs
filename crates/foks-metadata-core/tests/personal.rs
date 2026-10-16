#![cfg(feature = "native")]
//! Stored-row and SQL-work qualification only. Exact native TLS/Go certificate
//! parsing and possession are exercised by the server adapter integration gates.
#[path = "../../foks-server-db/tests/common/mod.rs"]
mod common;
use foks_metadata_core::{
    native::NativeSql, personal::*, wire::Command, Limits, Mutation, Statement, Value,
};
use foks_proto::{
    KvDirectory, KvDirectoryStatus, KvDirent, KvNodeId, KvRoot, KvSmallFileBox, Role,
    RoleAndGeneration, SecretBox,
};
use foks_server_db::{KvDirectoryMutation, KvNodeMutation, KvRootMutation};
use rusqlite::{params, Connection, StatementStatus};
const UID: [u8; 33] = [1; 33];
const DEVICE: [u8; 33] = [4; 33];
const ROOT: [u8; 16] = [0x51; 16];
const NOW: u64 = 200;
fn key(role: Role, generation: u64) -> RoleAndGeneration {
    RoleAndGeneration { role, generation }
}
fn peer(serial: Vec<u8>) -> PeerCertificate {
    PeerCertificate::from_frontend_parts(vec![0x30, 0], serial, [4; 32], 1000).unwrap()
}
fn fixture() -> common::TestDatabase {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    test.database
        .record_certificate(
            &[7],
            &UID,
            &DEVICE,
            &DEVICE,
            100,
            1000,
            peer(vec![7]).exact_der(),
        )
        .unwrap();
    let directory = KvDirectory {
        id: ROOT,
        version: 1,
        key: key(Role::OWNER, 1),
        seed_ciphertext: vec![1; 48],
        write_role: Role::ADMIN,
        status: KvDirectoryStatus::Active,
    }
    .encode()
    .unwrap();
    test.database
        .put_kv_directory(&KvDirectoryMutation {
            uid: &UID,
            id: &ROOT,
            version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            status: 0,
            exact: &directory,
        })
        .unwrap();
    let root = KvRoot::new(ROOT, 1, key(Role::OWNER, 1), [2; 32]).unwrap();
    test.database
        .put_kv_root(&KvRootMutation {
            uid: &UID,
            version: 1,
            directory_id: &ROOT,
            directory_version: 1,
            key_role: 3,
            key_visibility: 0,
            key_generation: 1,
            exact: root.encoded(),
        })
        .unwrap();
    let node = KvSmallFileBox {
        key: key(Role::OWNER, 1),
        ciphertext: vec![9; 48],
    }
    .encode()
    .unwrap();
    for kind in [3u8, 4] {
        let mut id = [3; 17];
        id[0] = kind;
        test.database
            .put_kv_node(&KvNodeMutation {
                uid: &UID,
                id: &id,
                node_type: u64::from(kind),
                exact: &node,
            })
            .unwrap();
    }
    test
}
fn auth(c: &Connection, peer: &PeerCertificate) -> Result<CurrentCredential> {
    let tx = c.unchecked_transaction().unwrap();
    authenticate_peer(&mut NativeSql::new(&tx).unwrap(), peer, NOW)
}

#[test]
fn frontend_parts_are_bounded_canonical_and_not_asserted_identity() {
    for serial in [vec![0], vec![0x80], vec![0xff; 20]] {
        let p = peer(serial);
        let encoded = serde_json::to_value(&p).unwrap();
        assert_eq!(encoded["expires_at_micros"], "1000");
        assert_eq!(
            serde_json::from_value::<PeerCertificate>(encoded.clone()).unwrap(),
            p
        );
        let mut unknown = encoded.clone();
        unknown["uid"] = serde_json::json!([1]);
        assert!(serde_json::from_value::<PeerCertificate>(unknown).is_err());
        let mut numeric = encoded;
        numeric["expires_at_micros"] = serde_json::json!(1000);
        assert!(serde_json::from_value::<PeerCertificate>(numeric).is_err());
    }
    for serial in [vec![], vec![0, 7], vec![1; 21]] {
        assert!(PeerCertificate::from_frontend_parts(vec![1], serial, [1; 32], 1000).is_err());
    }
    assert!(PeerCertificate::from_frontend_parts(vec![1; 4097], vec![1], [1; 32], 1000).is_err());
    assert!(PeerCertificate::from_frontend_parts(vec![], vec![1], [1; 32], 1000).is_err());
    assert!(PeerCertificate::from_frontend_parts(vec![1], vec![1], [1; 32], u64::MAX).is_err());
}

#[test]
fn serial_padding_is_bounded_and_duplicate_binding_never_selects_arbitrarily() {
    let test = fixture();
    let c = Connection::open(&test.path).unwrap();
    for magnitude in [vec![0], vec![7], vec![0x80], vec![0xff; 20]] {
        for width in magnitude.len()..=20 {
            c.execute("DELETE FROM issued_certificates", []).unwrap();
            let mut serial = vec![0; width];
            serial[width - magnitude.len()..].copy_from_slice(&magnitude);
            c.execute(
                "INSERT INTO issued_certificates VALUES(?1,?2,?3,?3,100,1000,?4)",
                params![serial, UID, DEVICE, peer(magnitude.clone()).exact_der()],
            )
            .unwrap();
            let tx = c.unchecked_transaction().unwrap();
            let mut sql = NativeSql::new(&tx).unwrap();
            let resolved = authenticate_peer(&mut sql, &peer(magnitude.clone()), NOW).unwrap();
            assert_eq!(resolved.uid(), &UID);
            assert_eq!(resolved.role(), Role::OWNER);
            assert_eq!(sql.calls(), 21 - magnitude.len());
            tx.rollback().unwrap();
        }
    }
    c.execute("DELETE FROM issued_certificates", []).unwrap();
    for serial in [vec![7u8], vec![0, 7]] {
        c.execute(
            "INSERT INTO issued_certificates VALUES(?1,?2,?3,?3,100,1000,?4)",
            params![serial, UID, DEVICE, peer(vec![7]).exact_der()],
        )
        .unwrap();
    }
    assert_eq!(auth(&c, &peer(vec![7])), Err(PersonalError::Corrupt));
}

#[test]
fn certificate_binding_interval_role_and_ordinary_credential_boundaries() {
    let test = fixture();
    let c = Connection::open(&test.path).unwrap();
    let p = peer(vec![7]);
    for (now, okay) in [(99, false), (100, true), (999, true), (1000, false)] {
        let tx = c.unchecked_transaction().unwrap();
        assert_eq!(
            authenticate_peer(&mut NativeSql::new(&tx).unwrap(), &p, now).is_ok(),
            okay
        );
    }
    assert_eq!(
        auth(&c, &peer(vec![8])),
        Err(PersonalError::Unauthenticated)
    );
    let wrong =
        PeerCertificate::from_frontend_parts(p.exact_der().to_vec(), vec![7], [5; 32], 1000)
            .unwrap();
    assert_eq!(auth(&c, &wrong), Err(PersonalError::Unauthenticated));
    let wrong =
        PeerCertificate::from_frontend_parts(vec![0x30, 1], vec![7], [4; 32], 1000).unwrap();
    assert_eq!(auth(&c, &wrong), Err(PersonalError::Unauthenticated));
    let resolved = auth(&c, &p).unwrap();
    c.execute(
        "UPDATE devices SET role_type=2 WHERE device_id=?1",
        [DEVICE],
    )
    .unwrap();
    let tx = c.unchecked_transaction().unwrap();
    assert_eq!(
        recheck_current_credential(&mut NativeSql::new(&tx).unwrap(), &resolved, NOW),
        Err(PersonalError::StalePreparation)
    );
    tx.rollback().unwrap();
    c.execute("UPDATE devices SET active=0 WHERE device_id=?1", [DEVICE])
        .unwrap();
    assert_eq!(auth(&c, &p), Err(PersonalError::Unauthenticated));
    c.execute(
        "UPDATE devices SET active=1,role_type=3 WHERE device_id=?1",
        [DEVICE],
    )
    .unwrap();
    for (kind, wanted, ordinary) in [
        (16u8, CredentialKind::BackupKey, false),
        (19, CredentialKind::BotToken, true),
    ] {
        let mut id = DEVICE;
        id[0] = kind;
        c.execute("INSERT INTO devices(device_id,uid,active,role_type,visibility,subkey_id,hepk_fingerprint,self_token,exact_hepk,exact_name) VALUES(?1,?2,1,3,0,NULL,zeroblob(32),?3,x'',x'')",params![id,UID,[kind;17]]).unwrap();
        c.execute(
            "UPDATE issued_certificates SET device_id=?1,credential_id=?1",
            [id],
        )
        .unwrap();
        let tx = c.unchecked_transaction().unwrap();
        let mut sql = NativeSql::new(&tx).unwrap();
        let credential = authenticate_peer(&mut sql, &p, NOW).unwrap();
        assert_eq!(credential.kind(), wanted);
        assert_eq!(
            require_personal_profile(&mut sql, &credential).is_ok(),
            ordinary
        );
    }
}

#[test]
fn rotated_subkey_cannot_borrow_a_different_current_owners_role() {
    let test = fixture();
    let c = Connection::open(&test.path).unwrap();
    let mut owner = [8; 34];
    owner[1] = 2;
    let mut subkey = DEVICE;
    subkey[0] = 13;
    c.execute("INSERT INTO devices(device_id,uid,active,role_type,visibility,subkey_id,hepk_fingerprint,self_token,exact_hepk,exact_name) VALUES(?1,?2,1,3,0,?3,zeroblob(32),?4,x'',x'')",params![owner,UID,subkey,[8u8;17]]).unwrap();
    c.execute(
        "UPDATE issued_certificates SET device_id=?1,credential_id=?2",
        params![owner, subkey],
    )
    .unwrap();
    let credential = auth(&c, &peer(vec![7])).unwrap();
    assert_eq!(credential.owner_id(), owner);
    assert_eq!(credential.kind(), CredentialKind::DelegatedSubkey);
    c.execute(
        "UPDATE devices SET subkey_id=NULL WHERE device_id=?1",
        [owner],
    )
    .unwrap();
    let mut other = owner;
    other[2] = 9;
    c.execute("INSERT INTO devices(device_id,uid,active,role_type,visibility,subkey_id,hepk_fingerprint,self_token,exact_hepk,exact_name) VALUES(?1,?2,1,3,0,?3,zeroblob(32),?4,x'',x'')",params![other,UID,subkey,[9u8;17]]).unwrap();
    assert_eq!(
        auth(&c, &peer(vec![7])),
        Err(PersonalError::Unauthenticated)
    );
    // Existing native current-owner lookup deliberately differs for this
    // historical rebinding. Do not tighten its generic history verifier here.
    assert_eq!(
        test.database
            .active_credential_owner(&UID, &subkey)
            .unwrap(),
        Some(other.to_vec())
    );
    let tx = c.unchecked_transaction().unwrap();
    assert_eq!(
        recheck_current_credential(&mut NativeSql::new(&tx).unwrap(), &credential, NOW),
        Err(PersonalError::Permission)
    );
}

#[test]
fn targeted_key_decisions_match_native_full_map_across_role_and_generation_boundaries() {
    let test = fixture();
    let c = Connection::open(&test.path).unwrap();
    let roles = [
        Role::member(i16::MIN),
        Role::member(0),
        Role::member(i16::MAX),
        Role::ADMIN,
        Role::OWNER,
    ];
    for role in roles {
        for generation in 1..=3i64 {
            c.execute(
                "INSERT OR IGNORE INTO shared_keys VALUES(?1,?2,?3,?4,?5,x'')",
                params![
                    UID,
                    role.protocol_value() as i64,
                    i64::from(role.visibility().unwrap_or(0)),
                    generation,
                    [14u8; 33]
                ],
            )
            .unwrap();
        }
    }
    let native = test.database.user_authority(&UID).unwrap().unwrap();
    for caller in roles {
        c.execute(
            "UPDATE devices SET role_type=?1,visibility=?2 WHERE device_id=?3",
            params![
                caller.protocol_value() as i64,
                i64::from(caller.visibility().unwrap_or(0)),
                DEVICE
            ],
        )
        .unwrap();
        let tx = c.unchecked_transaction().unwrap();
        let mut sql = NativeSql::new(&tx).unwrap();
        let credential = authenticate_peer(&mut sql, &peer(vec![7]), NOW).unwrap();
        for role in roles.into_iter().chain([Role::member(17)]) {
            for generation in 0..=4 {
                for access in [KeyAccess::Read, KeyAccess::Write] {
                    let current = native
                        .shared_keys
                        .iter()
                        .filter(|k| {
                            k.role_type == role.protocol_value()
                                && k.visibility == i64::from(role.visibility().unwrap_or(0))
                        })
                        .map(|k| k.generation)
                        .max();
                    let expected = role <= caller
                        && match access {
                            KeyAccess::Read => {
                                generation > 0 && current.is_some_and(|v| generation <= v)
                            }
                            KeyAccess::Write => current == Some(generation),
                        };
                    assert_eq!(
                        require_key(
                            &mut sql,
                            &credential,
                            KeyRequirement {
                                key: key(role, generation),
                                access
                            }
                        )
                        .is_ok(),
                        expected,
                        "caller={caller:?} role={role:?} gen={generation} access={access:?}"
                    );
                }
            }
        }
    }
    c.execute(
        "UPDATE devices SET role_type=2,visibility=1 WHERE device_id=?1",
        [DEVICE],
    )
    .unwrap();
    assert_eq!(auth(&c, &peer(vec![7])), Err(PersonalError::Corrupt));
}

#[test]
fn scalar_reads_preserve_exact_nodes_and_fail_closed_on_unsupported_or_corrupt_state() {
    let test = fixture();
    let c = Connection::open(&test.path).unwrap();
    let p = peer(vec![7]);
    let tx = c.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&tx).unwrap();
    let credential = authenticate_peer(&mut sql, &p, NOW).unwrap();
    for point in [
        PersonalPoint::Root {},
        PersonalPoint::Directory { id: ROOT },
        PersonalPoint::Node { id: [3; 17] },
        PersonalPoint::QuotaCounters {},
    ] {
        assert!(read_personal_point(&mut sql, &credential, &point, NOW).is_ok());
    }
    assert_eq!(
        read_personal_point(
            &mut sql,
            &credential,
            &PersonalPoint::Node { id: [2; 17] },
            NOW
        ),
        Err(PersonalError::UnsupportedProfile)
    );
    tx.rollback().unwrap();
    c.execute("UPDATE kv_roots SET exact_root=zeroblob(4097)", [])
        .unwrap();
    let tx = c.unchecked_transaction().unwrap();
    assert_eq!(
        read_personal_point(
            &mut NativeSql::new(&tx).unwrap(),
            &credential,
            &PersonalPoint::Root {},
            NOW
        ),
        Err(PersonalError::UnsupportedProfile)
    );
    tx.rollback().unwrap();
    c.execute("UPDATE kv_roots SET exact_root=x'00'", [])
        .unwrap();
    let tx = c.unchecked_transaction().unwrap();
    assert_eq!(
        read_personal_point(
            &mut NativeSql::new(&tx).unwrap(),
            &credential,
            &PersonalPoint::Root {},
            NOW
        ),
        Err(PersonalError::Corrupt)
    );
    tx.rollback().unwrap();
    for mode in [0i64, 1] {
        c.execute("INSERT INTO sso_policy VALUES(?1,zeroblob(32),zeroblob(16),'https://issuer.invalid',?2,1,1,1) ON CONFLICT(host) DO UPDATE SET mode=excluded.mode",params![[2u8;33],mode]).unwrap();
        let tx = c.unchecked_transaction().unwrap();
        assert_eq!(
            require_personal_profile(&mut NativeSql::new(&tx).unwrap(), &credential),
            Err(PersonalError::UnsupportedProfile)
        );
    }
    c.execute("DELETE FROM sso_policy", []).unwrap();
    c.execute("DELETE FROM tree_locations", []).unwrap();
    let tx = c.unchecked_transaction().unwrap();
    assert_eq!(
        require_personal_profile(&mut NativeSql::new(&tx).unwrap(), &credential),
        Err(PersonalError::Permission)
    );
}

fn mutation(version: u64) -> Mutation {
    let d = KvDirent::new(
        ROOT,
        [8; 16],
        KvNodeId([3; 17]),
        version,
        1,
        Role::ADMIN,
        [8; 32],
        SecretBox {
            nonce: [7; 16],
            ciphertext: vec![8; 16],
        },
        KvDirectoryStatus::Active,
        [9; 32],
        42,
    )
    .unwrap();
    Mutation {
        parent: d.parent,
        id: d.id,
        version: d.version,
        directory_version: d.directory_version,
        node_id: d.value.0,
        name_mac: d.name_mac,
        creation_time: d.creation_time,
        exact: d.encode().unwrap(),
    }
}
fn command(m: Mutation) -> Command {
    Command::PutDirents {
        uid: UID.to_vec(),
        precondition: None,
        role: Role::OWNER.encoded().unwrap(),
        mutations: vec![m],
        normalize_at: None,
        limits: Limits {
            blob_bytes: 4096,
            dirent_bytes: 4096,
            namespace_bytes: 1_000_000,
            namespace_objects: 1000,
            directories: 100,
            dirents: 100,
        },
    }
}
fn authorize(c: &Connection, command: &Command) -> Result<()> {
    let tx = c.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&tx).unwrap();
    let credential = authenticate_peer(&mut sql, &peer(vec![7]), NOW)?;
    authorize_personal_put(&mut sql, &credential, command)
}
#[test]
fn put_derives_permissions_and_preserves_read_only_failures_and_exact_replay() {
    let test = fixture();
    let c = Connection::open(&test.path).unwrap();
    let cmd = command(mutation(1));
    assert_eq!(authorize(&c, &cmd), Ok(()));
    let before = c.total_changes();
    let mut wrong_uid = cmd.clone();
    if let Command::PutDirents { uid, .. } = &mut wrong_uid {
        uid[1] ^= 1;
    }
    assert_eq!(authorize(&c, &wrong_uid), Err(PersonalError::Permission));
    let mut wrong_role = cmd.clone();
    if let Command::PutDirents { role, .. } = &mut wrong_role {
        *role = Role::ADMIN.encoded().unwrap();
    }
    assert_eq!(authorize(&c, &wrong_role), Err(PersonalError::Permission));
    let mut mismatched = mutation(1);
    mismatched.creation_time += 1;
    assert_eq!(
        authorize(&c, &command(mismatched)),
        Err(PersonalError::BadArguments)
    );
    let mut missing = mutation(1);
    let mut d = KvDirent::decode(&missing.exact).unwrap();
    d.parent = [7; 16];
    missing.parent = d.parent;
    missing.exact = d.encode().unwrap();
    assert_eq!(
        authorize(&c, &command(missing)),
        Err(PersonalError::Missing)
    );
    let mut large = mutation(1);
    let mut d = KvDirent::decode(&large.exact).unwrap();
    d.value = KvNodeId([2; 17]);
    large.node_id = d.value.0;
    large.exact = d.encode().unwrap();
    assert_eq!(
        authorize(&c, &command(large)),
        Err(PersonalError::UnsupportedProfile)
    );
    assert_eq!(
        c.total_changes(),
        before,
        "authorization must not mutate or create namespace state"
    );
    let encoded = serde_json::to_vec(&cmd).unwrap();
    let tx = c.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&tx).unwrap();
    let credential = authenticate_peer(&mut sql, &peer(vec![7]), NOW).unwrap();
    authorize_personal_put(&mut sql, &credential, &cmd).unwrap();
    let first = foks_metadata_core::wire::run(&mut sql, &encoded).unwrap();
    tx.commit().unwrap();
    let tx = c.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&tx).unwrap();
    let credential = authenticate_peer(&mut sql, &peer(vec![7]), NOW).unwrap();
    authorize_personal_put(&mut sql, &credential, &cmd).unwrap();
    assert_eq!(
        foks_metadata_core::wire::run(&mut sql, &encoded).unwrap(),
        first
    );
    tx.commit().unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM kv_dirents", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    // A current lower role cannot read the OWNER-keyed parent even when its
    // proposed write_role would otherwise permit the metadata mutation.
    c.execute(
        "UPDATE devices SET role_type=2 WHERE device_id=?1",
        [DEVICE],
    )
    .unwrap();
    let mut admin = cmd;
    if let Command::PutDirents { role, .. } = &mut admin {
        *role = Role::ADMIN.encoded().unwrap();
    }
    assert_eq!(authorize(&c, &admin), Err(PersonalError::Permission));
    c.execute("UPDATE devices SET active=0 WHERE device_id=?1", [DEVICE])
        .unwrap();
    assert_eq!(authorize(&c, &admin), Err(PersonalError::Unauthenticated));
}

#[test]
fn precondition_status_and_exact_delta_match_native_version_semantics() {
    use foks_proto::{KvDirectoryVersion as D, KvDirentVersion as E, KvPathVersionVector as P};
    use foks_server_db::KvVersionCheck;
    let test = fixture();
    let c = Connection::open(&test.path).unwrap();
    let encoded = serde_json::to_vec(&command(mutation(1))).unwrap();
    let tx = c.unchecked_transaction().unwrap();
    foks_metadata_core::wire::run(&mut NativeSql::new(&tx).unwrap(), &encoded).unwrap();
    tx.commit().unwrap();
    let vectors = [
        P {
            root_version: 0,
            directories: vec![],
        },
        P {
            root_version: 1,
            directories: vec![],
        },
        P {
            root_version: 2,
            directories: vec![],
        },
        P {
            root_version: 1,
            directories: vec![D {
                id: ROOT,
                version: 0,
                entries: vec![E {
                    id: [8; 16],
                    version: 0,
                }],
            }],
        },
        P {
            root_version: 1,
            directories: vec![D {
                id: ROOT,
                version: 2,
                entries: vec![],
            }],
        },
        P {
            root_version: 1,
            directories: vec![D {
                id: ROOT,
                version: 1,
                entries: vec![E {
                    id: [8; 16],
                    version: 2,
                }],
            }],
        },
        P {
            root_version: 1,
            directories: vec![D {
                id: ROOT,
                version: 1,
                entries: vec![E {
                    id: [9; 16],
                    version: 0,
                }],
            }],
        },
        P {
            root_version: 1,
            directories: vec![D {
                id: [9; 16],
                version: 0,
                entries: vec![],
            }],
        },
    ];
    let native =
        foks_server_db::ReadDatabase::open(&test.path, foks_server_db::Config::default()).unwrap();
    for vector in vectors {
        let expected = match native.kv_version_check(&UID, &vector).unwrap() {
            KvVersionCheck::Current => Ok(()),
            KvVersionCheck::Stale(delta) => Err(PersonalError::StaleCache(delta)),
            KvVersionCheck::Future => Err(PersonalError::BadArguments),
            KvVersionCheck::Missing => Err(PersonalError::CachedObjectMissing),
        };
        let tx = c.unchecked_transaction().unwrap();
        let mut sql = NativeSql::new(&tx).unwrap();
        let credential = authenticate_peer(&mut sql, &peer(vec![7]), NOW).unwrap();
        assert_eq!(
            check_personal_precondition(&mut sql, &credential, Some(&vector)),
            expected
        );
    }
    // Read authorization precedes revealing a stale/future delta.
    c.execute(
        "UPDATE devices SET role_type=2 WHERE device_id=?1",
        [DEVICE],
    )
    .unwrap();
    let tx = c.unchecked_transaction().unwrap();
    let mut sql = NativeSql::new(&tx).unwrap();
    let credential = authenticate_peer(&mut sql, &peer(vec![7]), NOW).unwrap();
    assert_eq!(
        check_personal_precondition(
            &mut sql,
            &credential,
            Some(&P {
                root_version: 2,
                directories: vec![]
            })
        ),
        Err(PersonalError::Permission)
    );
}

fn query_cost(c: &Connection, statement: Statement, parameters: &[Value]) -> i32 {
    use rusqlite::types::Value as Q;
    let parameters: Vec<Q> = parameters
        .iter()
        .map(|v| match v {
            Value::Integer(v) => Q::Text(v.to_string()),
            Value::Blob(v) => Q::Blob(v.clone()),
            Value::Text(v) => Q::Text(v.clone()),
            Value::Null => Q::Null,
        })
        .collect();
    let mut query = c.prepare(statement.spec().sql).unwrap();
    {
        let mut rows = query
            .query(rusqlite::params_from_iter(&parameters))
            .unwrap();
        while rows.next().unwrap().is_some() {}
    }
    let steps = query.get_status(StatementStatus::VmStep);
    let mut explain = c
        .prepare(&format!("EXPLAIN QUERY PLAN {}", statement.spec().sql))
        .unwrap();
    let plan: Vec<String> = explain
        .query_map(rusqlite::params_from_iter(&parameters), |r| r.get(3))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(
        !plan.iter().any(|line| line.contains("USE TEMP B-TREE")),
        "{statement:?}: {plan:?}"
    );
    steps
}
#[test]
fn production_point_queries_do_not_scan_certificate_role_or_directory_history() {
    let mut costs = Vec::new();
    for count in [1i64, 100, 10_000] {
        let test = fixture();
        let mut c = Connection::open(&test.path).unwrap();
        let tx = c.transaction().unwrap();
        for n in 1..=count {
            let mut serial = vec![0x20];
            serial.extend_from_slice(&n.to_be_bytes());
            let mut device = [4u8; 33];
            device[1..9].copy_from_slice(&n.to_be_bytes());
            let mut token = [0u8; 17];
            token[0] = 4;
            token[1..9].copy_from_slice(&n.to_be_bytes());
            tx.execute("INSERT INTO devices(device_id,uid,active,role_type,visibility,subkey_id,hepk_fingerprint,self_token,exact_hepk,exact_name) VALUES(?1,?2,1,3,0,NULL,zeroblob(32),?3,x'',x'')",params![device,UID,token]).unwrap();
            tx.execute(
                "INSERT INTO issued_certificates VALUES(?1,?2,?3,?3,100,1000,x'3000')",
                params![serial, UID, device],
            )
            .unwrap();
            tx.execute(
                "INSERT OR IGNORE INTO shared_keys VALUES(?1,3,0,?2,?3,x'')",
                params![UID, n, [14u8; 33]],
            )
            .unwrap();
            let exact = KvDirectory {
                id: ROOT,
                version: u64::try_from(n).unwrap(),
                key: key(Role::OWNER, 1),
                seed_ciphertext: vec![1; 48],
                write_role: Role::ADMIN,
                status: KvDirectoryStatus::Active,
            }
            .encode()
            .unwrap();
            tx.execute("INSERT OR IGNORE INTO kv_directories SELECT uid,directory_id,?1,key_role,key_visibility,key_generation,status,?4 FROM kv_directories WHERE uid=?2 AND directory_id=?3 AND version=1",params![n,UID,ROOT,exact]).unwrap();
        }
        tx.commit().unwrap();
        let credential_cost = query_cost(
            &c,
            Statement::PersonalIssuedCredential,
            &[
                Value::Blob(vec![7]),
                Value::Blob(peer(vec![7]).exact_der().to_vec()),
                Value::Integer(NOW as i64),
            ],
        );
        let generation_cost = query_cost(
            &c,
            Statement::PersonalRoleGeneration,
            &[
                Value::Blob(UID.to_vec()),
                Value::Integer(3),
                Value::Integer(0),
            ],
        );
        let directory_cost = query_cost(
            &c,
            Statement::PersonalDirectory,
            &[Value::Blob(UID.to_vec()), Value::Blob(ROOT.to_vec())],
        );
        let missing_cost = query_cost(
            &c,
            Statement::PersonalIssuedCredential,
            &[
                Value::Blob(vec![9]),
                Value::Blob(peer(vec![7]).exact_der().to_vec()),
                Value::Integer(NOW as i64),
            ],
        );
        let serial_loop_cost: i32 = (1..=20)
            .map(|width| {
                let mut serial = vec![0; width];
                serial[width - 1] = 7;
                query_cost(
                    &c,
                    Statement::PersonalIssuedCredential,
                    &[
                        Value::Blob(serial),
                        Value::Blob(peer(vec![7]).exact_der().to_vec()),
                        Value::Integer(NOW as i64),
                    ],
                )
            })
            .sum();
        costs.push((
            credential_cost,
            generation_cost,
            directory_cost,
            missing_cost,
            serial_loop_cost,
        ));
        eprintln!(
            "personal point rows={count} VM steps={:?}",
            costs.last().unwrap()
        );
    }
    assert!(costs.iter().all(|cost| *cost == costs[0]), "{costs:?}");
}
