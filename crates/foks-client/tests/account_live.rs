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
}
