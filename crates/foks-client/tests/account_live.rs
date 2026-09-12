//! Explicit pinned-Go gate; normal tests perform no network access.
use foks_client::{
    EncryptedFileMutationStore, FederationCredential, FoksClient, MutationCoordinator, ProbeTarget,
    SoftwareAccountRequest, SoftwareAccountSecrets,
};
use foks_client_db::MutationState;
use foks_proto::{InviteCode, SecretSeed};
use zeroize::Zeroizing;
#[test]
fn durable_rename_against_go() {
    let Ok(probe) = std::env::var("FOKS_TEST_ACCOUNT_PROBE") else {
        return;
    };
    let ca = std::fs::read(std::env::var("FOKS_TEST_ACCOUNT_CA").unwrap()).unwrap();
    let dir = std::path::PathBuf::from(std::env::var("FOKS_TEST_ACCOUNT_STATE").unwrap());
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(ca))
        .unwrap();
    let client = FoksClient::with_roots(roots);
    let target = ProbeTarget::parse(&probe).unwrap();
    let db = dir.join("account-client-hard.sqlite3");
    let soft = dir.join("account-client-soft.sqlite3");
    client.probe_and_pin(&target, &db).unwrap();
    let host = client.pinned_host(target.hostname(), &db).unwrap();
    let mut protected =
        EncryptedFileMutationStore::open(dir.join("account-protected"), Zeroizing::new([23; 32]))
            .unwrap();

    let username = std::env::var("FOKS_TEST_ACCOUNT_USERNAME").unwrap();
    let created = client
        .create_software_account(
            &host,
            SoftwareAccountRequest {
                username_utf8: username,
                device_name: "rename device".into(),
                invite_code: InviteCode::Empty,
                email: "rename@example.test".into(),
                passphrase: None,
            },
            SoftwareAccountSecrets::new(
                SecretSeed::new([42; 32]),
                SecretSeed::new([43; 32]),
                [35; 17],
            ),
            &soft,
            &mut protected,
        )
        .unwrap();
    let credential = FederationCredential::Software(&created.credential);
    for name in ["rustr ename", "rustrenamed", "RustRenamed"] {
        let prepared = client.prepare_username_change(&host, credential, name, &mut protected);
        if name.contains(' ') {
            assert!(prepared.is_err());
            continue;
        }
        let prepared = prepared.unwrap();
        let id = prepared.operation.operation_id;
        let mut progress = client
            .username_change_progress(&host, credential, id, true, &mut protected)
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while progress.operation.state != MutationState::RemoteVerified {
            assert!(
                std::time::Instant::now() < deadline,
                "rename proof was not published"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
            progress = client
                .username_change_progress(&host, credential, id, false, &mut protected)
                .unwrap();
        }
        assert_eq!(progress.operation.state, MutationState::RemoteVerified);
        assert_eq!(progress.operation.attempt_count, 1);
        assert_eq!(progress.current.unwrap().username_utf8(), name.as_bytes());
        MutationCoordinator::new(&db, &mut protected)
            .finalize(&id)
            .unwrap();
        assert_eq!(
            client
                .username_change_progress(&host, credential, id, true, &mut protected)
                .unwrap()
                .operation
                .state,
            MutationState::Finalized
        );
    }
    for (index, role) in [foks_proto::Role::OWNER, foks_proto::Role::member(0)]
        .into_iter()
        .enumerate()
    {
        let token = foks_crypto::BotToken::generate().unwrap();
        let mut op = client
            .prepare_bot_enrollment(&host, credential, role, &token, &mut protected)
            .unwrap();
        op = client
            .bot_enrollment_progress(&host, credential, op.operation_id, true, &mut protected)
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while op.state != MutationState::RemoteVerified {
            assert!(
                std::time::Instant::now() < deadline,
                "bot proof not published"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
            op = client
                .bot_enrollment_progress(&host, credential, op.operation_id, false, &mut protected)
                .unwrap();
        }
        assert_eq!(op.attempt_count, 1);
        MutationCoordinator::new(&db, &mut protected)
            .finalize(&op.operation_id)
            .unwrap();
        let bot = client.load_bot_token(&host, &token).unwrap();
        let auth = client.authenticate_and_pin(&host, &bot).unwrap();
        assert_eq!(auth.puks.last().unwrap().role, role);
        client.kv_usage(&host, &bot, None).unwrap();
        if index == 0 {
            let permanent = client
                .provision_software_device(
                    &host,
                    &bot,
                    foks_client::SoftwareDeviceProvisionRequest {
                        device_name: "bot permanent".into(),
                        role,
                        serial: 1,
                    },
                    foks_client::NewSoftwareDeviceSecrets::new(
                        SecretSeed::new([88; 32]),
                        None,
                        [0x36; 17],
                    ),
                    &mut protected,
                )
                .unwrap();
            assert_eq!(client.ping(&host, &permanent.credential).unwrap(), bot.uid);
        } else {
            assert!(!auth.puks.iter().any(|k| k.role == foks_proto::Role::OWNER));
        }
    }
    #[cfg(feature = "test-support")]
    if let Ok(destination) = std::env::var("FOKS_TEST_ADMIN_DESTINATION") {
        use foks_client::{AdminDestination, WebAdminError};
        let policy = AdminDestination::loopback_test(&destination).unwrap();
        let handoff = client
            .new_web_admin_handoff(&host, credential, &policy)
            .unwrap();
        let attacker = format!(
            "https://attacker.example/?{}",
            handoff.expose().split_once('?').unwrap().1
        );
        assert!(
            client
                .check_web_admin_session(&host, credential, &attacker)
                .is_ok(),
            "Go checks session ownership, not origin"
        );
        assert!(policy.validate_handoff(&attacker).is_err());
        let other = client
            .create_software_account(
                &host,
                SoftwareAccountRequest {
                    username_utf8: "adminother".into(),
                    device_name: "other admin".into(),
                    invite_code: InviteCode::Empty,
                    email: "adminother@example.test".into(),
                    passphrase: None,
                },
                SoftwareAccountSecrets::new(
                    SecretSeed::new([91; 32]),
                    SecretSeed::new([92; 32]),
                    [0x35; 17],
                ),
                &soft,
                &mut protected,
            )
            .unwrap();
        assert_eq!(
            client
                .check_web_admin_session(
                    &host,
                    FederationCredential::Software(&other.credential),
                    handoff.expose()
                )
                .unwrap_err(),
            WebAdminError::WrongAccount
        );
        let mut tampered = Zeroizing::new(handoff.expose().to_owned());
        let last = tampered.pop().unwrap();
        tampered.push(if last == '0' { '1' } else { '0' });
        assert_eq!(
            client
                .check_web_admin_session(&host, credential, &tampered)
                .unwrap_err(),
            WebAdminError::Expired
        );
        let control = std::env::var("FOKS_TEST_ADMIN_CONTROL").unwrap();
        let address = control.strip_prefix("http://").unwrap();
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        use std::io::{Read, Write};
        write!(stream,"POST /expire HTTP/1.1\r\nHost: {address}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.take(1024).read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 204"));
        assert_eq!(
            client
                .check_web_admin_session(&host, credential, handoff.expose())
                .unwrap_err(),
            WebAdminError::Expired
        );
    }
}
