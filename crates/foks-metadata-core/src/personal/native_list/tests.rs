use super::*;
use std::collections::VecDeque;
#[cfg(feature = "native")]
#[path = "../../../../foks-server-db/tests/common/mod.rs"]
mod common;
#[derive(Default)]
struct Script {
    steps: VecDeque<(S, Option<Vec<V>>)>,
    seen: Vec<S>,
    parameters: Vec<Vec<V>>,
}
impl Sql for Script {
    fn query(&mut self, id: S, parameters: &[V]) -> crate::Result<Option<Vec<V>>> {
        let (expected, row) = self.steps.pop_front().expect("unexpected query");
        assert_eq!(id, expected);
        self.seen.push(id);
        self.parameters.push(parameters.to_vec());
        Ok(row)
    }
    fn execute(&mut self, _: S, _: &[V]) -> crate::Result<u64> {
        panic!("auth is read only")
    }
}
fn credential(kind: u8) -> Vec<u8> {
    let mut v = vec![4; 33];
    v[0] = kind;
    v
}
fn issued(role: i64, visibility: i64, kind: u8) -> IssuedCredential {
    let mut sql = Script::default();
    sql.steps.push_back((
        S::ListIssuedCredential,
        Some(vec![
            V::Blob(vec![1; 33]),
            V::Blob(credential(kind)),
            V::Integer(role),
            V::Integer(visibility),
            V::Integer(1000),
        ]),
    ));
    let value = authenticate(&mut sql, &[0x30, 0], &[7; 20], &[4; 32], 200).unwrap();
    assert!(sql.steps.is_empty());
    value
}
#[test]
fn negative_visibility_is_native_valid_before_and_after_missing_anchor() {
    for visibility in [i16::MIN as i64, -1, 0, i16::MAX as i64] {
        let value = issued(1, visibility, foks_proto::ENTITY_DEVICE);
        let mut sql = Script::default();
        // A raw native owner need not have a supported EntityId prefix.
        let owner = vec![0xee; 34];
        sql.steps.extend([
            (S::ListCurrentOwner, Some(vec![V::Blob(owner.clone())])),
            (S::PersonalAnchor, None),
        ]);
        assert_eq!(
            authority(&mut sql, value).unwrap(),
            AuthorityDecision::NoAnchor { owner }
        );
        assert!(sql.steps.is_empty());
        let value = issued(1, visibility, foks_proto::ENTITY_DEVICE);
        sql.steps.extend([
            (S::ListCurrentOwner, Some(vec![V::Blob(vec![0xee; 34])])),
            (S::PersonalAnchor, Some(vec![V::Integer(1)])),
            (
                S::PersonalCurrentCredential,
                Some(vec![V::Integer(1), V::Integer(visibility)]),
            ),
        ]);
        let AuthorityDecision::Authorized(current) = authority(&mut sql, value).unwrap() else {
            panic!()
        };
        assert_eq!(current.role(), foks_proto::Role::member(visibility as i16));
        assert_eq!(current.owner_id(), &[0xee; 34]);
        assert_eq!(current.expires_at_micros(), 1000);
    }
}
#[test]
fn current_owner_and_anchor_precede_invalid_selected_role() {
    let mut sql = Script::default();
    sql.steps.push_back((S::ListCurrentOwner, None));
    assert_eq!(
        authority(&mut sql, issued(99, i64::MIN, foks_proto::ENTITY_DEVICE)).unwrap(),
        AuthorityDecision::NoOwner
    );
    sql.steps.extend([
        (S::ListCurrentOwner, Some(vec![V::Blob(vec![4; 33])])),
        (S::PersonalAnchor, Some(vec![V::Integer(1)])),
        (
            S::PersonalCurrentCredential,
            Some(vec![V::Integer(99), V::Integer(i64::MIN)]),
        ),
    ]);
    assert_eq!(
        authority(&mut sql, issued(3, 0, foks_proto::ENTITY_DEVICE)).unwrap(),
        AuthorityDecision::InvalidRole {
            owner: vec![4; 33],
            role_type: 99,
            visibility: i64::MIN
        }
    );
}
#[test]
fn native_auth_kind_and_serial_padding_probe_set_are_bounded() {
    for (kind, expected) in [
        (foks_proto::ENTITY_DEVICE, CredentialKind::SoftwareDevice),
        (foks_proto::ENTITY_SUBKEY, CredentialKind::DelegatedSubkey),
        (foks_proto::ENTITY_BACKUP_KEY, CredentialKind::BackupKey),
        (foks_proto::ENTITY_BOT_TOKEN_KEY, CredentialKind::BotToken),
    ] {
        assert_eq!(issued(3, 0, kind).kind(), expected);
    }
    let mut sql = Script::default();
    for _ in 0..20 {
        sql.steps.push_back((S::ListIssuedCredential, None));
    }
    assert_eq!(
        authenticate(&mut sql, &[0x30, 0], &[0], &[4; 32], 200),
        Err(PersonalError::Unauthenticated)
    );
    assert_eq!(sql.seen.len(), 20);
    assert!(sql.steps.is_empty());
}
#[test]
fn mismatched_public_key_and_negative_role_type_do_not_admit() {
    for (kind, public, role, expected) in [
        (
            foks_proto::ENTITY_DEVICE,
            [5; 32],
            3,
            PersonalError::Unauthenticated,
        ),
        (
            foks_proto::ENTITY_DEVICE,
            [4; 32],
            -1,
            PersonalError::Corrupt,
        ),
    ] {
        let mut sql = Script::default();
        sql.steps.push_back((
            S::ListIssuedCredential,
            Some(vec![
                V::Blob(vec![1; 33]),
                V::Blob(credential(kind)),
                V::Integer(role),
                V::Integer(-100),
                V::Integer(1000),
            ]),
        ));
        assert_eq!(
            authenticate(&mut sql, &[0x30, 0], &[7; 20], &public, 200),
            Err(expected)
        );
    }
}

#[test]
fn successful_short_serial_still_checks_every_padding_width() {
    let mut sql = Script::default();
    sql.steps.push_back((
        S::ListIssuedCredential,
        Some(vec![
            V::Blob(vec![1; 33]),
            V::Blob(credential(foks_proto::ENTITY_DEVICE)),
            V::Integer(3),
            V::Integer(0),
            V::Integer(1000),
        ]),
    ));
    for _ in 2..=20 {
        sql.steps.push_back((S::ListIssuedCredential, None));
    }
    assert_eq!(
        authenticate(&mut sql, &[0x30, 0], &[7], &[4; 32], 200)
            .unwrap()
            .uid(),
        &[1; 33]
    );
    assert!(sql.steps.is_empty());
    assert_eq!(sql.seen, vec![S::ListIssuedCredential; 20]);
    for (index, parameters) in sql.parameters.iter().enumerate() {
        let mut expected = vec![0; index + 1];
        expected[index] = 7;
        assert_eq!(
            parameters,
            &[V::Blob(expected), V::Blob(vec![0x30, 0]), V::Integer(200)]
        );
    }
}

#[test]
fn missing_anchor_does_not_interpret_invalid_selected_role() {
    let mut sql = Script::default();
    let owner = vec![0xff; 33];
    sql.steps.extend([
        (S::ListCurrentOwner, Some(vec![V::Blob(owner.clone())])),
        (S::PersonalAnchor, None),
    ]);
    assert_eq!(
        authority(&mut sql, issued(99, i64::MIN, foks_proto::ENTITY_DEVICE)).unwrap(),
        AuthorityDecision::NoAnchor { owner }
    );
    assert_eq!(sql.seen, [S::ListCurrentOwner, S::PersonalAnchor]);
    assert!(sql.steps.is_empty());
}

// This is a real current-schema SQLite and ReadDatabase oracle. Synthetic DER bytes
// intentionally isolate stored-row ordering; actual X509 parsing, signatures,
// TLS possession and ordinary-device admission belong to server integration.
#[cfg(feature = "native")]
mod native {
    use super::*;
    use crate::native::NativeSql;
    use foks_server_db::ReadDatabase;
    use rusqlite::{params, Connection, StatementStatus};

    const UID: [u8; 33] = [1; 33];
    const DEVICE: [u8; 33] = [4; 33];
    const DER: &[u8] = &[0x30, 0];
    const NOW: u64 = 200;

    struct Fixture {
        connection: Connection,
        reader: ReadDatabase,
        _database: common::TestDatabase,
    }

    impl Fixture {
        fn new() -> Self {
            let mut database = common::TestDatabase::new();
            database.reserve(1_000_000);
            database.commit(None).unwrap();
            database
                .database
                .record_certificate(&[7], &UID, &DEVICE, &DEVICE, 100, 1000, DER)
                .unwrap();
            let connection = Connection::open(&database.path).unwrap();
            connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; PRAGMA trusted_schema=OFF").unwrap();
            let reader = ReadDatabase::open(&database.path, Default::default()).unwrap();
            let schema: i64 = connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(schema, foks_server_db::SCHEMA_VERSION);
            Self {
                connection,
                reader,
                _database: database,
            }
        }
        fn authenticate(
            &self,
            magnitude: &[u8],
            public: &[u8; 32],
            now: u64,
        ) -> Result<IssuedCredential> {
            let transaction = self.connection.unchecked_transaction().unwrap();
            authenticate(
                &mut NativeSql::new(&transaction).unwrap(),
                DER,
                magnitude,
                public,
                now,
            )
        }
        fn decision(&self) -> Result<AuthorityDecision> {
            let transaction = self.connection.unchecked_transaction().unwrap();
            let mut sql = NativeSql::new(&transaction).unwrap();
            let issued = authenticate(&mut sql, DER, &[7], &[4; 32], NOW)?;
            authority(&mut sql, issued)
        }
        fn add_device(
            &self,
            owner: &[u8],
            subkey: Option<&[u8]>,
            role: i64,
            visibility: i64,
            active: bool,
        ) {
            self.connection.execute("INSERT INTO devices(device_id,uid,active,role_type,visibility,subkey_id,hepk_fingerprint,self_token,exact_hepk,exact_name) VALUES(?1,?2,?3,?4,?5,?6,zeroblob(32),?7,X'00',X'00')",
                params![owner,UID,active,role,visibility,subkey,&owner[..17]]).unwrap();
        }
        fn set_credential(&self, credential: &[u8]) {
            self.connection
                .execute(
                    "UPDATE issued_certificates SET credential_id=?1",
                    [credential],
                )
                .unwrap();
        }
    }

    #[test]
    fn native_issuance_preserves_signed_visibility_and_typed_role_is_deferred() {
        let fixture = Fixture::new();
        for visibility in [i16::MIN as i64, -1, 0, i16::MAX as i64, i64::MIN, i64::MAX] {
            fixture
                .connection
                .execute(
                    "UPDATE devices SET role_type=1,visibility=?1 WHERE device_id=?2",
                    params![visibility, DEVICE],
                )
                .unwrap();
            let native = fixture
                .reader
                .credential_for_certificate(DER, NOW)
                .unwrap()
                .unwrap();
            assert_eq!((native.role_type, native.visibility), (1, visibility));
            let issued = fixture.authenticate(&[7], &[4; 32], NOW).unwrap();
            assert_eq!(issued.uid().as_slice(), native.uid);
            assert_eq!(issued.credential_id(), native.credential_id);
            let snapshot = fixture.reader.user_authority(&UID).unwrap().unwrap();
            let selected = snapshot
                .devices
                .iter()
                .find(|d| d.device_id == DEVICE)
                .unwrap();
            assert_eq!(selected.visibility, visibility);
            match i16::try_from(visibility) {
                Ok(visibility) => {
                    let AuthorityDecision::Authorized(value) = fixture.decision().unwrap() else {
                        panic!("native-valid custom visibility was refused");
                    };
                    assert_eq!(value.role(), foks_proto::Role::member(visibility));
                    assert_eq!(value.owner_id(), DEVICE);
                }
                Err(_) => assert_eq!(
                    fixture.decision().unwrap(),
                    AuthorityDecision::InvalidRole {
                        owner: DEVICE.to_vec(),
                        role_type: 1,
                        visibility
                    }
                ),
            }
        }
        fixture
            .connection
            .execute("DELETE FROM tree_locations WHERE uid=?1", [UID])
            .unwrap();
        assert!(fixture.reader.user_authority(&UID).unwrap().is_none());
        assert_eq!(
            fixture.decision().unwrap(),
            AuthorityDecision::NoAnchor {
                owner: DEVICE.to_vec()
            }
        );
    }

    #[test]
    fn native_admission_kind_window_revocation_and_unsigned_role_match_read_database() {
        let fixture = Fixture::new();
        for (kind, expected) in [
            (foks_proto::ENTITY_DEVICE, CredentialKind::SoftwareDevice),
            (foks_proto::ENTITY_SUBKEY, CredentialKind::DelegatedSubkey),
            (foks_proto::ENTITY_BACKUP_KEY, CredentialKind::BackupKey),
            (foks_proto::ENTITY_BOT_TOKEN_KEY, CredentialKind::BotToken),
        ] {
            let credential = credential(kind);
            fixture.set_credential(&credential);
            let native = fixture
                .reader
                .credential_for_certificate(DER, NOW)
                .unwrap()
                .unwrap();
            let ours = fixture.authenticate(&[7], &[4; 32], NOW).unwrap();
            assert_eq!(ours.credential_id(), native.credential_id);
            assert_eq!(ours.kind(), expected);
            assert_eq!(ours.stored_expiry_micros(), 1000);
            assert_eq!(
                fixture.authenticate(&[7], &[5; 32], NOW),
                Err(PersonalError::Unauthenticated)
            );
        }
        for now in [0, 99, 100, 999, 1000, u64::from(u32::MAX)] {
            let native = fixture.reader.credential_for_certificate(DER, now).unwrap();
            let ours = fixture.authenticate(&[7], &[4; 32], now);
            assert_eq!(ours.is_ok(), native.is_some(), "now={now}");
        }
        fixture.set_credential(&credential(foks_proto::ENTITY_HOST));
        assert!(fixture
            .reader
            .credential_for_certificate(DER, NOW)
            .unwrap()
            .is_some());
        assert_eq!(
            fixture.authenticate(&[7], &[4; 32], NOW),
            Err(PersonalError::Unauthenticated)
        );
        fixture.set_credential(&DEVICE);
        fixture
            .connection
            .execute("UPDATE devices SET active=0 WHERE device_id=?1", [DEVICE])
            .unwrap();
        assert!(fixture
            .reader
            .credential_for_certificate(DER, NOW)
            .unwrap()
            .is_none());
        assert_eq!(
            fixture.authenticate(&[7], &[4; 32], NOW),
            Err(PersonalError::Unauthenticated)
        );
        fixture.connection.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE devices SET active=1,role_type=-1; PRAGMA ignore_check_constraints=OFF").unwrap();
        assert!(fixture.reader.credential_for_certificate(DER, NOW).is_err());
        assert_eq!(
            fixture.authenticate(&[7], &[4; 32], NOW),
            Err(PersonalError::Corrupt)
        );
    }

    #[test]
    fn catalog_45_enumerates_zero_and_all_valid_padding_widths_with_real_pk_probes() {
        let fixture = Fixture::new();
        for magnitude in [vec![0], vec![7], vec![0x80], vec![0xff; 20]] {
            for width in magnitude.len()..=20 {
                let mut stored = vec![0; width];
                stored[width - magnitude.len()..].copy_from_slice(&magnitude);
                fixture
                    .connection
                    .execute("UPDATE issued_certificates SET serial=?1", [stored])
                    .unwrap();
                let native = fixture
                    .reader
                    .credential_for_certificate(DER, NOW)
                    .unwrap()
                    .unwrap();
                let transaction = fixture.connection.unchecked_transaction().unwrap();
                let mut sql = NativeSql::new(&transaction).unwrap();
                let ours = authenticate(&mut sql, DER, &magnitude, &[4; 32], NOW).unwrap();
                assert_eq!(ours.credential_id(), native.credential_id);
                assert_eq!(sql.calls(), 21 - magnitude.len());
            }
        }
        fixture
            .connection
            .execute("UPDATE issued_certificates SET serial=X'07'", [])
            .unwrap();
        fixture.connection.execute("INSERT INTO issued_certificates SELECT X'0007',uid,device_id,credential_id,not_before,not_after,exact_certificate FROM issued_certificates",[]).unwrap();
        // Native exact-DER lookup alone does not disambiguate this. The import
        // audit forbids it; the request helper also fails closed if it recurs.
        assert!(fixture
            .reader
            .credential_for_certificate(DER, NOW)
            .unwrap()
            .is_some());
        assert_eq!(
            fixture.authenticate(&[7], &[4; 32], NOW),
            Err(PersonalError::Corrupt)
        );
    }

    #[test]
    fn reassigned_subkey_keeps_issuance_admission_and_selects_current_raw_owner() {
        let fixture = Fixture::new();
        let credential = credential(foks_proto::ENTITY_SUBKEY);
        fixture.set_credential(&credential);
        // Historical issuer stays active, but no longer owns the subkey. Native
        // admission does not impose a current device/subkey equality here.
        fixture
            .connection
            .execute(
                "UPDATE devices SET subkey_id=NULL WHERE device_id=?1",
                [DEVICE],
            )
            .unwrap();
        for width in [33, 34] {
            let owner = vec![0xee; width];
            fixture.add_device(&owner, Some(&credential), 1, -17, true);
            let native = fixture
                .reader
                .credential_for_certificate(DER, NOW)
                .unwrap()
                .unwrap();
            let issued = fixture.authenticate(&[7], &[4; 32], NOW).unwrap();
            assert_eq!(issued.credential_id(), native.credential_id);
            let snapshot = fixture.reader.user_authority(&UID).unwrap().unwrap();
            let selected = snapshot
                .devices
                .iter()
                .find(|device| {
                    device.active
                        && (device.device_id == credential
                            || device.subkey_id.as_deref() == Some(credential.as_slice()))
                })
                .unwrap();
            assert_eq!(selected.device_id, owner);
            let AuthorityDecision::Authorized(current) = fixture.decision().unwrap() else {
                panic!("current reassigned owner was refused")
            };
            assert_eq!(current.owner_id(), selected.device_id);
            assert_eq!(current.role(), foks_proto::Role::member(-17));
            fixture
                .connection
                .execute("DELETE FROM devices WHERE device_id=?1", [owner])
                .unwrap();
        }
        assert!(fixture
            .reader
            .active_credential_owner(&UID, &credential)
            .unwrap()
            .is_none());
        assert_eq!(fixture.decision().unwrap(), AuthorityDecision::NoOwner);
    }

    #[test]
    fn native_ordered_first_owner_wins_even_if_its_role_is_invalid() {
        let fixture = Fixture::new();
        let smaller = vec![0; 33];
        fixture.add_device(&smaller, Some(&DEVICE), 2, 1, true);
        let native = fixture.reader.user_authority(&UID).unwrap().unwrap();
        let selected = native
            .devices
            .iter()
            .find(|d| {
                d.active
                    && (d.device_id == DEVICE || d.subkey_id.as_deref() == Some(DEVICE.as_slice()))
            })
            .unwrap();
        assert_eq!(selected.device_id, smaller);
        assert_eq!((selected.role_type, selected.visibility), (2, 1));
        assert_eq!(
            fixture.decision().unwrap(),
            AuthorityDecision::InvalidRole {
                owner: smaller.clone(),
                role_type: 2,
                visibility: 1
            }
        );
        fixture
            .connection
            .execute("UPDATE devices SET active=0 WHERE device_id=?1", [&smaller])
            .unwrap();
        let AuthorityDecision::Authorized(value) = fixture.decision().unwrap() else {
            panic!("active direct owner was skipped")
        };
        assert_eq!(value.owner_id(), DEVICE);
        fixture
            .connection
            .execute("UPDATE devices SET active=1 WHERE device_id=?1", [&smaller])
            .unwrap();
        fixture
            .connection
            .execute("DELETE FROM tree_locations WHERE uid=?1", [UID])
            .unwrap();
        assert!(fixture.reader.user_authority(&UID).unwrap().is_none());
        assert_eq!(
            fixture.decision().unwrap(),
            AuthorityDecision::NoAnchor { owner: smaller }
        );
    }

    #[test]
    fn reassigned_owner_from_another_user_cannot_authorize_the_issued_uid() {
        let fixture = Fixture::new();
        let credential = credential(foks_proto::ENTITY_SUBKEY);
        fixture.set_credential(&credential);
        let owner = vec![0xee; 34];
        fixture.add_device(&owner, Some(&credential), 3, 0, true);
        let mut other_uid = [0x77; 33];
        other_uid[0] = foks_proto::ENTITY_USER;
        fixture.connection.execute("INSERT INTO names(normalized_name,reservation_sequence,uid) VALUES(X'6F74686572',1,?1)",[other_uid]).unwrap();
        fixture.connection.execute("INSERT INTO users(uid,normalized_name,username_utf8,username_sequence,username_commitment_key,created_at) VALUES(?1,X'6F74686572',X'6F74686572',1,zeroblob(16),0)",[other_uid]).unwrap();
        fixture
            .connection
            .execute(
                "UPDATE devices SET uid=?1 WHERE device_id=?2",
                params![other_uid, owner],
            )
            .unwrap();
        assert!(fixture
            .reader
            .active_credential_owner(&UID, &credential)
            .unwrap()
            .is_none());
        assert!(fixture
            .reader
            .active_credential_owner(&other_uid, &credential)
            .unwrap()
            .is_some());
        assert!(fixture
            .reader
            .credential_for_certificate(DER, NOW)
            .unwrap()
            .is_some());
        assert_eq!(fixture.decision().unwrap(), AuthorityDecision::NoOwner);
    }

    fn owner_work(
        connection: &Connection,
        credential: &[u8],
    ) -> (Vec<Vec<u8>>, i32, i32, Vec<String>) {
        let sql = S::ListCurrentOwner.spec().sql;
        let mut statement = connection.prepare(sql).unwrap();
        let rows = statement
            .query_map(params![UID, credential], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<Vec<u8>>>>()
            .unwrap();
        let steps = statement.get_status(StatementStatus::VmStep);
        let scans = statement.get_status(StatementStatus::FullscanStep);
        let mut explain = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let plan = explain
            .query_map(params![UID, credential], |row| row.get(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<String>>>()
            .unwrap();
        (rows, steps, scans, plan)
    }

    fn assert_two_unique_owner_searches(plan: &[String]) {
        let searches: Vec<_> = plan
            .iter()
            .filter(|detail| detail.contains("SEARCH devices"))
            .collect();
        assert_eq!(
            searches.len(),
            2,
            "owner must perform exactly two unique searches: {plan:?}"
        );
        assert!(
            searches
                .iter()
                .any(|detail| detail.contains("(device_id=?)")),
            "missing device PK search: {plan:?}"
        );
        assert!(
            searches
                .iter()
                .any(|detail| detail.contains("(subkey_id=?)")),
            "missing unique subkey search: {plan:?}"
        );
        assert!(
            !plan.iter().any(|detail| detail.contains("SCAN devices")),
            "unbounded devices scan: {plan:?}"
        );
        // A SEARCH on the uid prefix of UNIQUE(uid,self_token) is still a
        // range scan over this user's devices. LIMIT1 cannot bound a miss.
        assert!(
            !searches.iter().any(|detail| detail.contains("(uid=?)")),
            "unbounded uid range: {plan:?}"
        );
    }

    #[test]
    fn catalog_46_has_two_unique_searches_and_constant_work_for_dense_owner_misses() {
        assert_eq!(rusqlite::version(), "3.53.2");
        let fixture = Fixture::new();
        let missing = vec![0xfa; 33];
        let subkey = credential(foks_proto::ENTITY_SUBKEY);
        fixture
            .connection
            .execute(
                "UPDATE devices SET subkey_id=?1 WHERE device_id=?2",
                params![subkey, DEVICE],
            )
            .unwrap();
        let before_hit = owner_work(&fixture.connection, &DEVICE);
        let before_subkey = owner_work(&fixture.connection, &subkey);
        let before_miss = owner_work(&fixture.connection, &missing);
        let population = fixture.connection.unchecked_transaction().unwrap();
        for n in 0u64..4096 {
            let mut owner = [0x66; 33];
            owner[1..9].copy_from_slice(&n.to_be_bytes());
            fixture.add_device(&owner, None, 3, 0, n % 2 == 0);
        }
        population.commit().unwrap();
        for (credential, before, expected) in [
            (DEVICE.as_slice(), before_hit, vec![DEVICE.to_vec()]),
            (subkey.as_slice(), before_subkey, vec![DEVICE.to_vec()]),
            (missing.as_slice(), before_miss, Vec::new()),
        ] {
            let after = owner_work(&fixture.connection, credential);
            assert_eq!(before.0, expected);
            assert_eq!(after.0, expected);
            assert_two_unique_owner_searches(&before.3);
            assert_two_unique_owner_searches(&after.3);
            assert_eq!((before.2, after.2), (0, 0));
            assert!(
                before.1 <= 256 && after.1 <= 256,
                "bounded point lookup exceeded: before={} after={}",
                before.1,
                after.1
            );
            assert!(
                (after.1 - before.1).abs() <= 32,
                "work grew with unrelated devices: before={} after={}",
                before.1,
                after.1
            );
        }
    }
}
