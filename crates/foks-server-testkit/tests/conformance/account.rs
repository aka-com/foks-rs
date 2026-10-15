use crate::{
    authorization::{authenticated_stream, public_stream},
    support::Fixture,
};
use foks_client::DeviceCredential;
use foks_proto::{ChangeUsernameArgument, ChangedUsernameFullUpdate, UsernameReservation};
use foks_server_testkit::{TestAccountSpec, TestClient};
use std::io::Write;

fn request(f: &Fixture, c: &DeviceCredential, bytes: Vec<u8>) -> foks_rpc::Result<Vec<u8>> {
    let mut tls = authenticated_stream(f, c);
    tls.write_all(&bytes).unwrap();
    foks_rpc::read_response(&mut tls, 16 * 1024 * 1024, 0)
}
fn rename(
    f: &Fixture,
    c: &DeviceCredential,
    arg: &ChangeUsernameArgument,
) -> foks_rpc::Result<Vec<u8>> {
    let mut tls = authenticated_stream(f, c);
    tls.write_all(&foks_rpc::encode_change_username_request_at(arg, 0).unwrap())
        .unwrap();
    foks_rpc::read_void_response(&mut tls, 16 * 1024 * 1024, 0).map(|()| Vec::new())
}
#[track_caller]
fn status(result: foks_rpc::Result<Vec<u8>>, expected: u64) {
    assert!(
        matches!(result, Err(foks_rpc::Error::RemoteStatus {code,..}) if code==expected),
        "{result:?}"
    );
}
fn prepare(f: &Fixture, c: &DeviceCredential, name: &str, nonce: u8) -> ChangeUsernameArgument {
    let normalized = foks_verify::normalize_username(name.as_bytes()).unwrap();
    let reservation = UsernameReservation::decode(
        &request(
            f,
            c,
            foks_rpc::encode_reserve_username_for_change_request_at(&normalized, 0).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let current = f
        .client
        .foks()
        .authenticate_and_pin(f.host(), c)
        .unwrap()
        .verified;
    let root = current.tree_root();
    let location = [nonce; 32];
    let commitment_key = [nonce; 16];
    let link = foks_crypto::make_software_username_change(
        &foks_crypto::UsernameChangeInput {
            base: foks_crypto::UserMutationBase {
                uid: &c.uid,
                host: f.host().host_id(),
                seqno: current.chain_seqno() + 1,
                previous: current.chain_tail_hash(),
                root: &root,
                time: f.environment.advance_clock(1) / 1000,
                next_tree_location: location,
            },
            normalized_name: &normalized,
            name_sequence: reservation.sequence,
            commitment_key,
        },
        &c.seed,
    )
    .unwrap();
    ChangeUsernameArgument {
        username: name.into(),
        full: Some(ChangedUsernameFullUpdate {
            link,
            commitment_key,
            reservation,
            next_tree_location: location,
        }),
    }
}

#[test]
pub(crate) fn account_conveniences() {
    let f = Fixture::start("account-routes");
    let created = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("accountstart", 0x31))
        .unwrap();
    let c = &created.credential;
    for (frame, expected) in [
        (
            foks_rpc::encode_get_host_id_request_at(0).unwrap(),
            foks_snowpack::Value::Binary(f.host().host_id().as_bytes().to_vec()),
        ),
        (
            foks_rpc::encode_get_vhost_mgmt_host_request_at(0).unwrap(),
            foks_snowpack::Value::Text(Vec::new()),
        ),
    ] {
        let mut tls = public_stream(&f);
        tls.write_all(&frame).unwrap();
        assert_eq!(
            foks_snowpack::decode(&foks_rpc::read_response(&mut tls, 4096, 0).unwrap()).unwrap(),
            expected
        );
    }
    let location = request(
        &f,
        c,
        foks_rpc::encode_get_tree_location_request_at(2, 0).unwrap(),
    )
    .unwrap();
    assert!(
        matches!(foks_snowpack::decode(&location).unwrap(),foks_snowpack::Value::Binary(b) if b.len()==32)
    );
    status(
        request(
            &f,
            c,
            foks_rpc::encode_get_tree_location_request_at(1, 0).unwrap(),
        ),
        1049,
    );
    let arg = prepare(&f, c, "Account_New", 0x58);
    rename(&f, c, &arg).unwrap();
    let verified = f
        .client
        .foks()
        .authenticate_and_pin(f.host(), c)
        .unwrap()
        .verified;
    assert_eq!(verified.username(), b"account_new");
    assert_eq!(verified.chain_seqno(), 2);
    status(
        request(
            &f,
            c,
            foks_rpc::encode_reserve_username_for_change_request_at(b"accountstart", 0).unwrap(),
        ),
        1023,
    );
    status(
        rename(
            &f,
            c,
            &ChangeUsernameArgument {
                username: "Account_New".into(),
                full: arg.full.clone(),
            },
        ),
        1029,
    );
    status(
        rename(
            &f,
            c,
            &ChangeUsernameArgument {
                username: "ACCOUNT_NEW".into(),
                full: arg.full.clone(),
            },
        ),
        1030,
    );
    rename(
        &f,
        c,
        &ChangeUsernameArgument {
            username: "ACCOUNT_NEW".into(),
            full: None,
        },
    )
    .unwrap();
    let fresh = TestClient::new(&f.environment, "account-fresh").unwrap();
    let host = fresh.probe_and_pin().unwrap();
    let verified = fresh
        .foks()
        .authenticate_and_pin(&host.pinned, c)
        .unwrap()
        .verified;
    assert_eq!(verified.username_utf8(), b"ACCOUNT_NEW");
    assert_eq!(verified.chain_seqno(), 2);
    let sql = rusqlite::Connection::open(f.environment.database_path()).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM user_name_history WHERE uid=?1",
            [c.uid.as_bytes()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        sql.query_row(
            "SELECT dead FROM names WHERE normalized_name=?1",
            [b"accountstart".as_slice()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn rename_rejects_foreign_expired_and_racing_intent() {
    let f = Fixture::start("account-races");
    let a = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("accountoriginal", 0x41))
        .unwrap();
    let c = &a.credential;
    let first = prepare(&f, c, "accountnext", 0x61);
    let second = prepare(&f, c, "accountother", 0x62);
    let mut bad = first.clone();
    bad.full.as_mut().unwrap().reservation.token = second.full.as_ref().unwrap().reservation.token;
    status(rename(&f, c, &bad), 1030);
    let mut bad = first.clone();
    bad.full.as_mut().unwrap().commitment_key = [0x11; 16];
    status(rename(&f, c, &bad), 1030);
    status(
        rename(
            &f,
            c,
            &ChangeUsernameArgument {
                username: "accountnext".into(),
                full: None,
            },
        ),
        1030,
    );
    rename(&f, c, &first).unwrap();
    status(rename(&f, c, &second), 1044);
    let expired = prepare(&f, c, "accountexpired", 0x63);
    f.environment.advance_clock(11 * 60 * 1_000_000);
    let renewed = DeviceCredential {
        key_kind: foks_client::SoftwareKeyKind::Device,
        uid: c.uid.clone(),
        seed: foks_proto::SecretSeed::new(*c.seed.as_bytes()),
        certificate_chain: f
            .client
            .foks()
            .fetch_device_certificate_chain(f.host(), &c.uid, &c.seed)
            .unwrap(),
    };
    status(rename(&f, &renewed, &expired), 1030);
    let sql = rusqlite::Connection::open(f.environment.database_path()).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT normalized_name FROM users WHERE uid=?1",
            [c.uid.as_bytes()],
            |r| r.get::<_, Vec<u8>>(0)
        )
        .unwrap(),
        b"accountnext"
    );
    let mut pooled = authenticated_stream(&f, &renewed);
    pooled
        .write_all(&foks_rpc::encode_user_ping_request().unwrap())
        .unwrap();
    foks_rpc::read_response(&mut pooled, 4096, 0).unwrap();
    sql.execute(
        "UPDATE devices SET active=0 WHERE device_id=?1",
        [foks_crypto::derive_device_public(&c.seed)
            .unwrap()
            .id
            .as_bytes()],
    )
    .unwrap();
    pooled
        .write_all(
            &foks_rpc::encode_change_username_request_at(
                &ChangeUsernameArgument {
                    username: "ACCOUNTNEXT".into(),
                    full: None,
                },
                0,
            )
            .unwrap(),
        )
        .unwrap();
    let rejected = foks_rpc::read_void_response(&mut pooled, 4096, 0);
    assert!(
        matches!(
            rejected,
            Err(foks_rpc::Error::RemoteStatus { code: 1013, .. }) | Err(foks_rpc::Error::Io(_))
        ),
        "{rejected:?}"
    );
    assert_eq!(
        sql.query_row(
            "SELECT username_utf8 FROM users WHERE uid=?1",
            [c.uid.as_bytes()],
            |r| r.get::<_, Vec<u8>>(0)
        )
        .unwrap(),
        b"accountnext"
    );
}

#[test]
fn reservation_race_and_tree_locations_are_caller_scoped() {
    let f = Fixture::start("account-reservation-race");
    let a = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("accountracea", 0x21))
        .unwrap();
    let b = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("accountraceb", 0x22))
        .unwrap();
    let streams = [
        authenticated_stream(&f, &a.credential),
        authenticated_stream(&f, &b.credential),
    ];
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers = streams
        .into_iter()
        .map(|mut stream| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                stream
                    .write_all(
                        &foks_rpc::encode_reserve_username_for_change_request_at(
                            b"contendedname",
                            0,
                        )
                        .unwrap(),
                    )
                    .unwrap();
                foks_rpc::read_response(&mut stream, 4096, 0)
            })
        })
        .collect::<Vec<_>>();
    let results = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(foks_rpc::Error::RemoteStatus { code: 1023, .. })))
            .count(),
        1
    );
    let rename_arg = prepare(&f, &a.credential, "accountracechanged", 0x42);
    rename(&f, &a.credential, &rename_arg).unwrap();
    request(
        &f,
        &a.credential,
        foks_rpc::encode_get_tree_location_request_at(3, 0).unwrap(),
    )
    .unwrap();
    status(
        request(
            &f,
            &b.credential,
            foks_rpc::encode_get_tree_location_request_at(3, 0).unwrap(),
        ),
        1049,
    );
}

#[test]
fn rename_rolls_back_name_history_and_merkle_at_publication_failures() {
    for table in ["user_name_history", "user_chain_links", "merkle_roots"] {
        let f = Fixture::start("account-atomic");
        let created = f
            .client
            .create_account(f.host(), &TestAccountSpec::new("accountatomic", 0x51))
            .unwrap();
        let arg = prepare(&f, &created.credential, "accountcommitted", 0x71);
        let sql = rusqlite::Connection::open(f.environment.database_path()).unwrap();
        sql.execute_batch(&format!("CREATE TRIGGER fail_rename BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, 'injected rename failure'); END;")).unwrap();
        assert!(rename(&f, &created.credential, &arg).is_err());
        assert_eq!(
            sql.query_row(
                "SELECT normalized_name FROM users WHERE uid=?1",
                [created.credential.uid.as_bytes()],
                |r| r.get::<_, Vec<u8>>(0)
            )
            .unwrap(),
            b"accountatomic"
        );
        assert_eq!(
            sql.query_row(
                "SELECT count(*) FROM user_name_history WHERE uid=?1",
                [created.credential.uid.as_bytes()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            sql.query_row(
                "SELECT seqno FROM user_chain_heads WHERE uid=?1",
                [created.credential.uid.as_bytes()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            sql.query_row(
                "SELECT dead FROM names WHERE normalized_name=?1",
                [b"accountatomic".as_slice()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        sql.execute_batch("DROP TRIGGER fail_rename").unwrap();
        rename(&f, &created.credential, &arg).unwrap();
        assert_eq!(
            f.client
                .foks()
                .authenticate_and_pin(f.host(), &created.credential)
                .unwrap()
                .verified
                .username(),
            b"accountcommitted"
        );
    }
}

#[test]
fn management_helper_returns_only_configured_authority() {
    let environment = foks_server_testkit::TestEnvironment::new().unwrap();
    let server = environment
        .start_server_with_management("admin.example:443")
        .unwrap();
    let client = TestClient::new(&environment, "management-discovery").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let f = Fixture {
        environment,
        server,
        client,
        probe,
    };
    let mut tls = public_stream(&f);
    tls.write_all(&foks_rpc::encode_get_vhost_mgmt_host_request_at(0).unwrap())
        .unwrap();
    assert_eq!(
        foks_snowpack::decode(&foks_rpc::read_response(&mut tls, 4096, 0).unwrap()).unwrap(),
        foks_snowpack::Value::Text(b"admin.example:443".to_vec())
    );
    let bad = foks_server_testkit::TestEnvironment::new().unwrap();
    assert!(bad
        .start_server_with_management("https://attacker.example/path")
        .is_err());
}

#[test]
fn client_rename_reconciles_exact_history_without_replaying() {
    use foks_client::FederationCredential;
    use foks_client_db::{HardStateStore, MutationState};
    let f = Fixture::start("account-journal");
    let a = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("accountjournal", 0x61))
        .unwrap();
    let c = FederationCredential::Software(&a.credential);
    let mut protected = f.client.open_protected_store().unwrap();
    let prepared = f
        .client
        .foks()
        .prepare_username_change(f.host(), c, "Account_Journal_Next", &mut protected)
        .unwrap();
    let id = prepared.operation.operation_id;
    assert_eq!(prepared.operation.state, MutationState::Prepared);
    f.environment
        .arm_fault(foks_server_testkit::TestFault::RenameAfterCommitBeforeResponse);
    assert!(f
        .client
        .foks()
        .username_change_progress(f.host(), c, id, true, &mut protected)
        .is_err());
    drop(protected);
    let mut protected = f.client.open_protected_store().unwrap();
    // A later signed rename does not erase evidence of the earlier committed link.
    let later = prepare(&f, &a.credential, "accountjournallater", 0x78);
    rename(&f, &a.credential, &later).unwrap();
    let done = f
        .client
        .foks()
        .username_change_progress(f.host(), c, id, true, &mut protected)
        .unwrap();
    assert_eq!(done.operation.state, MutationState::RemoteVerified);
    assert_eq!(done.operation.attempt_count, 1);
    assert_eq!(done.current.unwrap().username(), b"accountjournallater");
    foks_client::MutationCoordinator::new(f.client.hard_state_path(), &mut protected)
        .finalize(&id)
        .unwrap();
    assert_eq!(
        f.client
            .foks()
            .username_change_progress(f.host(), c, id, true, &mut protected)
            .unwrap()
            .operation
            .state,
        MutationState::Finalized
    );
    // A lost display-only update followed by another spelling is intentionally unresolved.
    let display = f
        .client
        .foks()
        .prepare_username_change(f.host(), c, "ACCOUNTJOURNALLATER", &mut protected)
        .unwrap();
    f.environment
        .arm_fault(foks_server_testkit::TestFault::RenameAfterCommitBeforeResponse);
    assert!(f
        .client
        .foks()
        .username_change_progress(
            f.host(),
            c,
            display.operation.operation_id,
            true,
            &mut protected
        )
        .is_err());
    rename(
        &f,
        &a.credential,
        &ChangeUsernameArgument {
            username: "Accountjournallater".into(),
            full: None,
        },
    )
    .unwrap();
    let unknown = f
        .client
        .foks()
        .username_change_progress(
            f.host(),
            c,
            display.operation.operation_id,
            true,
            &mut protected,
        )
        .unwrap();
    assert_eq!(unknown.operation.state, MutationState::SubmissionUnknown);
    assert_eq!(unknown.operation.attempt_count, 1);
    assert!(f
        .client
        .foks()
        .cancel_username_change(
            f.host(),
            &a.credential.uid,
            &c.device_id().unwrap(),
            display.operation.operation_id,
            &mut protected
        )
        .is_err());
    assert_eq!(
        HardStateStore::open(f.client.hard_state_path())
            .unwrap()
            .mutation(&id)
            .unwrap()
            .unwrap()
            .state,
        MutationState::Finalized
    );
}

// Capture every durable projection a rename can change, excluding deliberately
// changed device authority and certificate/session maintenance.
fn rename_state(f: &Fixture) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    let connection = rusqlite::Connection::open(f.environment.database_path()).unwrap();
    [
        "users",
        "user_chain_heads",
        "user_chain_links",
        "user_name_history",
        "names",
        "merkle_root_heads",
        "merkle_roots",
        "request_receipts",
    ]
    .into_iter()
    .map(|table| {
        let mut query = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1,2"))
            .unwrap();
        let width = query.column_count();
        query
            .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
    .collect()
}

fn prepare_with_signer(
    f: &Fixture,
    owner: &DeviceCredential,
    signer: &DeviceCredential,
    name: &str,
) -> ChangeUsernameArgument {
    let mut arg = prepare(f, owner, name, 0x61);
    let full = arg.full.as_mut().unwrap();
    let change = full.link.decode_group_change().unwrap();
    let public = signer.public_material().unwrap();
    let unsigned = foks_crypto::username_change_unsigned(
        &foks_crypto::UsernameChangeInput {
            base: foks_crypto::UserMutationBase {
                uid: &signer.uid,
                host: f.host().host_id(),
                seqno: change.seqno,
                previous: change.previous.unwrap(),
                root: &change.root,
                time: change.time,
                next_tree_location: full.next_tree_location,
            },
            normalized_name: &foks_verify::normalize_username(name.as_bytes()).unwrap(),
            name_sequence: full.reservation.sequence,
            commitment_key: full.commitment_key,
        },
        &public.id,
    )
    .unwrap();
    let signature = foks_crypto::sign_shared_key_typed(
        &signer.seed,
        foks_proto::LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[]).unwrap(),
    )
    .unwrap();
    full.link = unsigned.finish(vec![signature]).unwrap();
    arg
}

#[test]
fn username_operations_require_interactive_owner_raw_rpc() {
    use foks_client::{
        FederationCredential, NewSoftwareDeviceSecrets, SoftwareDeviceProvisionRequest,
    };
    use foks_proto::{Role, SecretSeed};
    for (bot, role) in [
        (true, Role::OWNER),
        (true, Role::member(0)),
        (false, Role::ADMIN),
        (false, Role::member(0)),
    ] {
        let f = Fixture::start("rename-credential-matrix");
        let created = f
            .client
            .create_account(f.host(), &TestAccountSpec::new("renameowner", 0x39))
            .unwrap();
        let owner = &created.credential;
        let mut protected = f.client.open_protected_store().unwrap();
        let actor = if bot {
            let token = foks_crypto::BotToken::generate().unwrap();
            let operation = f
                .client
                .foks()
                .prepare_bot_enrollment(
                    f.host(),
                    FederationCredential::Software(owner),
                    role,
                    &token,
                    &mut protected,
                )
                .unwrap();
            f.client
                .foks()
                .bot_enrollment_progress(
                    f.host(),
                    FederationCredential::Software(owner),
                    operation.operation_id,
                    true,
                    &mut protected,
                )
                .unwrap();
            f.client.foks().load_bot_token(f.host(), &token).unwrap()
        } else {
            f.client
                .foks()
                .provision_software_device(
                    f.host(),
                    owner,
                    SoftwareDeviceProvisionRequest {
                        device_name: "nonowner".into(),
                        role,
                        serial: 1,
                    },
                    NewSoftwareDeviceSecrets::new(
                        SecretSeed::new([0x72; 32]),
                        Some(SecretSeed::new([0x74; 32])),
                        [0x73; 17],
                    ),
                    &mut protected,
                )
                .unwrap()
                .credential
        };
        // These are real enrolled credentials, not malformed requests or certs.
        assert_eq!(f.client.foks().ping(f.host(), &actor).unwrap(), owner.uid);
        let full = prepare_with_signer(&f, owner, &actor, "renameattempt");
        let before = rename_state(&f);
        status(
            request(
                &f,
                &actor,
                foks_rpc::encode_reserve_username_for_change_request_at(b"renameclaim", 0).unwrap(),
            ),
            1013,
        );
        assert_eq!(
            rename_state(&f),
            before,
            "reservation: bot={bot}, role={role:?}"
        );
        status(
            rename(
                &f,
                &actor,
                &ChangeUsernameArgument {
                    username: "RENAMEOWNER".into(),
                    full: None,
                },
            ),
            1013,
        );
        assert_eq!(
            rename_state(&f),
            before,
            "display: bot={bot}, role={role:?}"
        );
        status(rename(&f, &actor, &full), 1013);
        assert_eq!(rename_state(&f), before, "full: bot={bot}, role={role:?}");
    }
}

#[test]
fn queued_username_operations_recheck_current_owner_before_mutating() {
    use std::time::{Duration, Instant};
    for revoke in [false, true] {
        for operation in ["reservation", "display", "full"] {
            let f = Fixture::start("rename-queued-authority");
            let created = f
                .client
                .create_account(f.host(), &TestAccountSpec::new("queuedowner", 0x49))
                .unwrap();
            let owner = &created.credential;
            let full = prepare(&f, owner, "queuedrename", 0x62);
            let frame = match operation {
                "reservation" => {
                    foks_rpc::encode_reserve_username_for_change_request_at(b"queuedclaim", 0)
                        .unwrap()
                }
                "display" => foks_rpc::encode_change_username_request_at(
                    &ChangeUsernameArgument {
                        username: "QUEUEDOWNER".into(),
                        full: None,
                    },
                    0,
                )
                .unwrap(),
                _ => foks_rpc::encode_change_username_request_at(&full, 0).unwrap(),
            };
            let mut stream = authenticated_stream(&f, owner);
            // Complete TLS and authenticate before the writer is held.
            stream
                .write_all(&foks_rpc::encode_user_ping_request().unwrap())
                .unwrap();
            foks_rpc::read_response(&mut stream, 4096, 0).unwrap();
            let before = rename_state(&f);
            let writer = f.server.writer_handle();
            let control = writer.clone();
            let path = f.environment.database_path().to_path_buf();
            let uid = owner.uid.as_bytes().to_vec();
            let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
            let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
            let predecessor = std::thread::spawn(move || {
                control.call(move |_| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    let sql = rusqlite::Connection::open(path).unwrap();
                    sql.execute(
                        if revoke {
                            "UPDATE devices SET active=0 WHERE uid=?1"
                        } else {
                            "UPDATE devices SET role_type=1, visibility=0 WHERE uid=?1"
                        },
                        [uid],
                    )
                    .unwrap();
                    Ok(())
                })
            });
            entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let waiting = std::thread::spawn(move || {
                stream.write_all(&frame).unwrap();
                if operation == "reservation" {
                    foks_rpc::read_response(&mut stream, 4096, 0)
                } else {
                    foks_rpc::read_void_response(&mut stream, 4096, 0).map(|()| Vec::new())
                }
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            while writer.metrics().pending != 2 && Instant::now() < deadline {
                std::thread::yield_now();
            }
            let queued = writer.metrics().pending == 2;
            release_tx.send(()).unwrap();
            predecessor.join().unwrap().unwrap();
            let outcome = waiting.join().unwrap();
            assert!(queued, "request did not pass admission into writer queue");
            status(outcome, 1013);
            assert_eq!(
                rename_state(&f),
                before,
                "operation={operation}, revoke={revoke}"
            );
        }
    }
}

#[test]
fn full_username_change_cannot_borrow_another_owner_signature() {
    let f = Fixture::start("rename-signer-binding");
    let created = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("signedowner", 0x51))
        .unwrap();
    let owner = &created.credential;
    let mut protected = f.client.open_protected_store().unwrap();
    let second = f
        .client
        .foks()
        .provision_software_device(
            f.host(),
            owner,
            foks_client::SoftwareDeviceProvisionRequest {
                device_name: "other-owner".into(),
                role: foks_proto::Role::OWNER,
                serial: 1,
            },
            foks_client::NewSoftwareDeviceSecrets::new(
                foks_proto::SecretSeed::new([0x75; 32]),
                None,
                [0x76; 17],
            ),
            &mut protected,
        )
        .unwrap();
    let full = prepare(&f, owner, "signedrename", 0x64);
    let before = rename_state(&f);
    assert!(rename(&f, &second.credential, &full).is_err());
    assert_eq!(rename_state(&f), before);
    rename(&f, owner, &full).unwrap();
}

#[test]
fn interactive_yubi_owner_keeps_parent_signer_and_subkey_transport() {
    use foks_yubi::{MockYubiProvider, Pin, PivPolicy, SlotId, YubiProvider as _};
    let f = Fixture::start("rename-yubi-owner");
    let created = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("yubirename", 0x41))
        .unwrap();
    let pin = Pin::new("123456").unwrap();
    let provider = MockYubiProvider::with_card("rename-card", 71008, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let prepared = provider
        .prepare(
            &card,
            SlotId::new(0x82).unwrap(),
            SlotId::new(0x83).unwrap(),
            &pin,
            None,
            PivPolicy::Once,
            PivPolicy::Never,
        )
        .unwrap();
    let mut protected = f.client.open_protected_store().unwrap();
    let provisioned = f
        .client
        .foks()
        .provision_yubi_device(
            f.host(),
            &created.credential,
            prepared.device.as_ref(),
            foks_client::YubiDeviceProvisionRequest {
                role: foks_proto::Role::OWNER,
                device_name: "rename owner".into(),
                serial: 2,
                pq_hint: foks_proto::YubiSlotAndPqKeyId {
                    slot: 0x83,
                    id: prepared.locator.pq_key_id,
                },
            },
            foks_client::NewYubiDeviceSecrets::new(
                foks_proto::SecretSeed::new([0x52; 32]),
                [0x53; 17],
            ),
            &mut protected,
        )
        .unwrap();
    // This adapter is transport-only: the certificate identifies the subkey,
    // while the full request below is explicitly signed by the Yubi parent.
    let transport = DeviceCredential {
        key_kind: foks_client::SoftwareKeyKind::Device,
        uid: created.credential.uid.clone(),
        seed: foks_proto::SecretSeed::new([0x52; 32]),
        certificate_chain: provisioned.credential.certificate_chain.clone(),
    };
    let reservation = UsernameReservation::decode(
        &request(
            &f,
            &transport,
            foks_rpc::encode_reserve_username_for_change_request_at(b"yubirenamed", 0).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let current = f
        .client
        .foks()
        .authenticate_yubi_and_pin(f.host(), &provisioned.credential)
        .unwrap()
        .verified;
    let root = current.tree_root();
    let link = foks_crypto::make_yubi_username_change(
        &foks_crypto::UsernameChangeInput {
            base: foks_crypto::UserMutationBase {
                uid: &transport.uid,
                host: f.host().host_id(),
                seqno: current.chain_seqno() + 1,
                previous: current.chain_tail_hash(),
                root: &root,
                time: f.environment.advance_clock(1) / 1000,
                next_tree_location: [0x54; 32],
            },
            normalized_name: b"yubirenamed",
            name_sequence: reservation.sequence,
            commitment_key: [0x55; 16],
        },
        prepared.device.as_ref(),
    )
    .unwrap();
    let argument = ChangeUsernameArgument {
        username: "YubiRenamed".into(),
        full: Some(ChangedUsernameFullUpdate {
            link,
            commitment_key: [0x55; 16],
            reservation,
            next_tree_location: [0x54; 32],
        }),
    };
    // Admission sees the current subkey; a preceding queued write rotates only
    // that subkey, leaving the signed parent active and OWNER throughout.
    let mut stream = authenticated_stream(&f, &transport);
    stream
        .write_all(&foks_rpc::encode_user_ping_request().unwrap())
        .unwrap();
    foks_rpc::read_response(&mut stream, 4096, 0).unwrap();
    let before = rename_state(&f);
    let writer = f.server.writer_handle();
    let control = writer.clone();
    let path = f.environment.database_path().to_path_buf();
    let parent_id = prepared.device.entity_id().as_bytes().to_vec();
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let predecessor = std::thread::spawn(move || {
        control.call(move |_| {
            entered_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            let replacement =
                foks_crypto::derive_subkey_id(&foks_proto::SecretSeed::new([0x56; 32])).unwrap();
            rusqlite::Connection::open(path)
                .unwrap()
                .execute(
                    "UPDATE devices SET subkey_id=?1 WHERE device_id=?2",
                    rusqlite::params![replacement.as_bytes(), parent_id],
                )
                .unwrap();
            Ok(())
        })
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let frame = foks_rpc::encode_change_username_request_at(&argument, 0).unwrap();
    let waiting = std::thread::spawn(move || {
        stream.write_all(&frame).unwrap();
        foks_rpc::read_void_response(&mut stream, 4096, 0).map(|()| Vec::new())
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while writer.metrics().pending != 2 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let queued = writer.metrics().pending == 2;
    release_tx.send(()).unwrap();
    predecessor.join().unwrap().unwrap();
    let result = waiting.join().unwrap();
    assert!(
        queued,
        "Yubi rename did not pass admission into writer queue"
    );
    status(result, 1013);
    assert_eq!(rename_state(&f), before);
    let subkey = foks_crypto::derive_subkey_id(&transport.seed).unwrap();
    rusqlite::Connection::open(f.environment.database_path())
        .unwrap()
        .execute(
            "UPDATE devices SET subkey_id=?1 WHERE device_id=?2",
            rusqlite::params![subkey.as_bytes(), prepared.device.entity_id().as_bytes()],
        )
        .unwrap();
    rename(&f, &transport, &argument).unwrap();
    rename(
        &f,
        &transport,
        &ChangeUsernameArgument {
            username: "YUBIRENAMED".into(),
            full: None,
        },
    )
    .unwrap();
    let verified = f
        .client
        .foks()
        .authenticate_yubi_and_pin(f.host(), &provisioned.credential)
        .unwrap()
        .verified;
    assert_eq!(verified.username_utf8(), b"YUBIRENAMED");
    assert_eq!(verified.chain_seqno(), current.chain_seqno() + 1);
}
