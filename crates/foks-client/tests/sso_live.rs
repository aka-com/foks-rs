//! Explicit local Go service gate. Ordinary tests never contact an external provider.
use foks_client::{
    EncryptedFileMutationStore, FederationCredential, FoksClient, ProbeTarget,
    SoftwareAccountRequest, SoftwareAccountSecrets, SsoSigningKey,
};
use foks_client_db::{HardStateStore, SsoFlowState};
use foks_oidc::{NetworkPolicy, ProviderHttp};
use foks_proto::{EntityId, InviteCode, SecretSeed, ENTITY_PUK_VERIFY, ENTITY_USER};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
fn browser(dir: &Path, step: u8, url: &str) {
    // Test-only browser broker; the official Go environment owns the ephemeral web CA.
    let path = dir.join(format!("sso-browser-{step}.url"));
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, url).unwrap();
    std::fs::rename(temp, path).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !dir.join(format!("sso-browser-{step}.done")).exists() {
        assert!(Instant::now() < deadline, "test browser did not complete");
        std::thread::sleep(Duration::from_millis(25));
    }
}
#[test]
fn signup_and_reauthentication_against_go_with_strict_idp() {
    let Ok(probe) = std::env::var("FOKS_TEST_SSO_PROBE") else {
        return;
    };
    let ca = std::fs::read(std::env::var("FOKS_TEST_SSO_CA").unwrap()).unwrap();
    let dir = std::path::PathBuf::from(std::env::var("FOKS_TEST_SSO_STATE").unwrap());
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(ca))
        .unwrap();
    let client = FoksClient::with_roots(roots);
    let target = ProbeTarget::parse(&probe).unwrap();
    let db = dir.join("sso-client-hard.sqlite3");
    let soft = dir.join("sso-client-soft.sqlite3");
    client.probe_and_pin(&target, &db).unwrap();
    let host = client.pinned_host(target.hostname(), &db).unwrap();
    let mut protected =
        EncryptedFileMutationStore::open(dir.join("sso-protected"), Zeroizing::new([23; 32]))
            .unwrap();
    let device = SecretSeed::new([42; 32]);
    let puk = SecretSeed::new([43; 32]);
    let mut uid = foks_crypto::derive_shared_verify_key(&puk, ENTITY_PUK_VERIFY)
        .unwrap()
        .as_bytes()
        .to_vec();
    uid[0] = ENTITY_USER;
    let uid = EntityId::from_bytes(uid).unwrap();
    let device_id = foks_crypto::derive_device_public(&device).unwrap().id;
    let http = ProviderHttp::new(NetworkPolicy::loopback_test()).unwrap();
    let flow = client
        .begin_sso(
            &host,
            foks_client::SsoIntent {
                uid: uid.clone(),
                device: device_id.clone(),
                purpose: foks_proto::SsoPurpose::Signup,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    assert_eq!(flow.state, SsoFlowState::AwaitingBrowser);
    let id = flow.operation_id;
    drop(protected);
    let mut protected =
        EncryptedFileMutationStore::open(dir.join("sso-protected"), Zeroizing::new([23; 32]))
            .unwrap();
    let resumed = client.sso_progress(&host, id, &mut protected).unwrap();
    assert_eq!(resumed.browser_url, flow.browser_url);
    browser(&dir, 1, resumed.browser_url.as_ref().unwrap());
    assert_eq!(
        client
            .poll_sso(&host, id, 1000, &http, &mut protected)
            .unwrap()
            .state,
        SsoFlowState::Ready
    );
    assert_eq!(
        client
            .poll_sso(&host, id, 0, &http, &mut protected)
            .unwrap()
            .state,
        SsoFlowState::Ready
    );
    assert!(client
        .authorize_sso_signup(
            &host,
            id,
            SsoSigningKey::Software(&SecretSeed::new([44; 32])),
            &http,
            &mut protected
        )
        .is_err());
    let authorization = client
        .authorize_sso_signup(
            &host,
            id,
            SsoSigningKey::Software(&device),
            &http,
            &mut protected,
        )
        .unwrap();
    let created = client
        .create_software_account_with_sso(
            &host,
            SoftwareAccountRequest {
                username_utf8: "rustsso".into(),
                device_name: "sso device".into(),
                invite_code: InviteCode::Empty,
                email: "rustsso@example.test".into(),
                passphrase: None,
            },
            SoftwareAccountSecrets::new(device, puk, [35; 17]),
            &soft,
            &authorization,
            &mut protected,
        )
        .unwrap();
    assert_eq!(created.credential.uid, uid);
    let recorded = HardStateStore::open(&db)
        .unwrap()
        .sso_flow(&id)
        .unwrap()
        .unwrap();
    assert_eq!(recorded.state, SsoFlowState::Complete);
    assert_eq!(recorded.final_operation, Some(created.operation_id));
    assert_ne!(id, created.operation_id);
    assert_eq!(client.ping(&host, &created.credential).unwrap(), uid);
    let login = client
        .begin_sso(
            &host,
            foks_client::SsoIntent {
                uid: uid.clone(),
                device: device_id,
                purpose: foks_proto::SsoPurpose::Reauthenticate,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    browser(&dir, 2, login.browser_url.as_ref().unwrap());
    client
        .poll_sso(&host, login.operation_id, 1000, &http, &mut protected)
        .unwrap();
    let result = client
        .finish_sso_login(
            &host,
            login.operation_id,
            FederationCredential::Software(&created.credential),
            &http,
            &mut protected,
        )
        .unwrap();
    assert_eq!(result.state, SsoFlowState::Complete);
    assert_eq!(
        client
            .finish_sso_login(
                &host,
                login.operation_id,
                FederationCredential::Software(&created.credential),
                &http,
                &mut protected
            )
            .unwrap()
            .state,
        SsoFlowState::Complete
    );
    assert_eq!(client.ping(&host, &created.credential).unwrap(), uid);
    assert!(client
        .cancel_sso(&host, login.operation_id, &mut protected)
        .is_err());
    use foks_yubi::{MockYubiProvider, Pin, PivPolicy, SlotId, YubiProvider as _};
    let pin = Pin::new("654321").unwrap();
    let provider = MockYubiProvider::with_card("sso-yubi", 73001, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let hardware = provider
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
    let parent = hardware.device.as_ref();
    let puk = SecretSeed::new([49; 32]);
    let mut yub_uid = foks_crypto::derive_shared_verify_key(&puk, ENTITY_PUK_VERIFY)
        .unwrap()
        .as_bytes()
        .to_vec();
    yub_uid[0] = ENTITY_USER;
    let yub_uid = EntityId::from_bytes(yub_uid).unwrap();
    let flow = client
        .begin_sso(
            &host,
            foks_client::SsoIntent {
                uid: yub_uid.clone(),
                device: parent.entity_id().clone(),
                purpose: foks_proto::SsoPurpose::Signup,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    browser(&dir, 3, flow.browser_url.as_ref().unwrap());
    client
        .poll_sso(&host, flow.operation_id, 1000, &http, &mut protected)
        .unwrap();
    let auth = client
        .authorize_sso_signup(
            &host,
            flow.operation_id,
            SsoSigningKey::Yubi(parent),
            &http,
            &mut protected,
        )
        .unwrap();
    // A repeated ready authorization reuses the stored hardware signature.
    client
        .authorize_sso_signup(
            &host,
            flow.operation_id,
            SsoSigningKey::Yubi(parent),
            &http,
            &mut protected,
        )
        .unwrap();
    let created = client
        .create_yubi_account_with_sso(
            &host,
            parent,
            foks_client::YubiAccountRequest {
                username_utf8: "rustssoyubi".into(),
                device_name: "sso key".into(),
                invite_code: InviteCode::Empty,
                email: "rustssoyubi@example.test".into(),
                passphrase: None,
                pq_hint: foks_proto::YubiSlotAndPqKeyId {
                    slot: 0x83,
                    id: hardware.locator.pq_key_id,
                },
            },
            foks_client::YubiAccountSecrets::new(SecretSeed::new([50; 32]), puk, [36; 17]),
            &soft,
            &auth,
            &mut protected,
        )
        .unwrap();
    assert_eq!(created.credential.uid, yub_uid);
    let login = client
        .begin_sso(
            &host,
            foks_client::SsoIntent {
                uid: yub_uid,
                device: parent.entity_id().clone(),
                purpose: foks_proto::SsoPurpose::Reauthenticate,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    browser(&dir, 4, login.browser_url.as_ref().unwrap());
    client
        .poll_sso(&host, login.operation_id, 1000, &http, &mut protected)
        .unwrap();
    assert_eq!(
        client
            .finish_sso_login(
                &host,
                login.operation_id,
                FederationCredential::Yubi(&created.credential),
                &http,
                &mut protected
            )
            .unwrap()
            .state,
        SsoFlowState::Complete
    );
}
