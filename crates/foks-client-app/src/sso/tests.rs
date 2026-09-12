use super::*;
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::{oidc::TestOidcProvider, TestEnvironment};
struct Fixture {
    environment: TestEnvironment,
    _server: foks_server_testkit::InProcessServer,
    credentials: ClientCredentials,
    registry: ProfileRegistry,
    http: ProviderHttp,
}
impl Fixture {
    fn start(idp: &TestOidcProvider) -> Self {
        let environment = TestEnvironment::new().unwrap();
        let cfg = idp.config(environment.client_path("oidc", "secret").unwrap());
        let server = environment.start_oidc_server(cfg).unwrap();
        let state = environment.client_path("sso-app", "state").unwrap();
        let root = environment.client_path("sso-app", "root.der").unwrap();
        environment.write_probe_root(&root).unwrap();
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".into(),
                probe: format!(
                    "localhost:{}",
                    environment.addresses().unwrap().probe.port()
                ),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let f = Self {
            environment,
            _server: server,
            credentials,
            registry,
            http: ProviderHttp::new(foks_oidc::NetworkPolicy::loopback_test()).unwrap(),
        };
        f.run(|s, _, _| {
            s.probe_and_pin()?;
            Ok(())
        });
        f
    }
    fn run<T>(
        &self,
        op: impl FnOnce(&CheckedProfileSession<'_>, &mut AccountVault<'_>, &[u8; 32]) -> Result<T>,
    ) -> T {
        // Reopen the profile, vault and protected store for every foreground action.
        let session = ProfileSession::open(&self.registry, "local").unwrap();
        self.credentials
            .with_checked_session(&session, |s| {
                let master = self.credentials.master_key()?;
                let mut store = EncryptedFileSecretStore::open(
                    &s.paths.credential_store,
                    derive_vault_key(&master),
                )?;
                op(s, &mut AccountVault::new(&mut store), &master)
            })
            .unwrap()
    }
}
fn id(p: &SsoReport) -> [u8; 16] {
    let mut out = [0; 16];
    for (o, p) in out
        .iter_mut()
        .zip(p.operation_id.as_ref().unwrap().as_bytes().chunks_exact(2))
    {
        *o = u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap();
    }
    out
}
#[test]
fn software_browser_flow_survives_reopen_and_uses_provider_identity() {
    let idp = TestOidcProvider::start();
    let f = Fixture::start(&idp);
    let start =
        f.run(|s, v, k| s.begin_account_sso("work", foks_proto::SsoPurpose::Signup, v, k, &f.http));
    let handle = id(&start);
    assert_eq!(start.state, "waiting");
    let again = f.run(|s, v, k| {
        assert!(s.resume_account("work", v, k).is_err());
        s.begin_account_sso("work", foks_proto::SsoPurpose::Signup, v, k, &f.http)
    });
    assert_eq!(again.operation_id, start.operation_id);
    assert_eq!(again.browser_url, start.browser_url);
    assert_eq!(
        f.run(|s, v, k| s.account_sso("work", handle, SsoAction::Poll, v, k, &f.http))
            .state,
        "waiting"
    );
    f.run(|s, v, k| {
        assert!(s
            .account_sso("other", handle, SsoAction::Status, v, k, &f.http)
            .is_err());
        Ok(())
    });
    idp.complete(start.browser_url.as_ref().unwrap());
    assert_eq!(
        f.run(|s, v, k| s.account_sso("work", handle, SsoAction::Poll, v, k, &f.http))
            .state,
        "ready"
    );
    let done = f.run(|s, v, k| {
        s.finish_account_sso_signup(
            "work",
            handle,
            SsoSignupInput {
                device_name: "work laptop".into(),
                invite: String::new(),
                passphrase: None,
            },
            v,
            k,
            &f.http,
        )
    });
    assert_eq!(done.state, "complete");
    assert!(done.service_access);
    assert!(done.browser_url.is_none());
    f.run(|_, v, _| {
        assert_eq!(v.account("work")?.username, "ssoalice");
        Ok(())
    });
    let login = f.run(|s, v, k| {
        s.begin_account_sso(
            "work",
            foks_proto::SsoPurpose::Reauthenticate,
            v,
            k,
            &f.http,
        )
    });
    idp.complete(login.browser_url.as_ref().unwrap());
    f.run(|s, v, k| s.account_sso("work", id(&login), SsoAction::Poll, v, k, &f.http));
    assert!(
        f.run(|s, v, k| s.account_sso("work", id(&login), SsoAction::FinishLogin, v, k, &f.http))
            .service_access
    );
    let cancelled = f.run(|s, v, k| {
        s.begin_account_sso(
            "work",
            foks_proto::SsoPurpose::Reauthenticate,
            v,
            k,
            &f.http,
        )
    });
    for _ in 0..2 {
        assert_eq!(
            f.run(|s, v, k| s.account_sso(
                "work",
                id(&cancelled),
                SsoAction::Cancel,
                v,
                k,
                &f.http
            ))
            .state,
            "cancelled"
        );
    }
    let raw = std::fs::read(f.environment.database_path()).unwrap();
    assert!(!raw
        .windows(b"opaque-access".len())
        .any(|w| w == b"opaque-access"));
}
#[test]
fn hardware_browser_signup_reuses_prepared_slots_and_finishes_existing_lifecycle() {
    use foks_yubi::{MockYubiProvider, Pin, SlotId, YubiProvider as _};
    let idp = TestOidcProvider::start();
    let f = Fixture::start(&idp);
    let pin = Pin::new("654321").unwrap();
    let provider = MockYubiProvider::with_card("sso", 777, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let begin = || {
        f.run(|s, v, k| {
            s.begin_yubi_sso_signup(
                YubiSignupInput {
                    alias: "key".into(),
                    username: "ignored".into(),
                    device_name: "work key".into(),
                    email: String::new(),
                    invite: String::new(),
                    passphrase: None,
                    card: card.clone(),
                    signing_slot: SlotId::new(0x82).unwrap(),
                    pq_slot: SlotId::new(0x83).unwrap(),
                    retry_configuration: None,
                },
                Pin::new("654321")?,
                &provider,
                v,
                k,
                &f.http,
            )
        })
    };
    let start = begin();
    assert_eq!(begin().operation_id, start.operation_id);
    idp.complete(start.browser_url.as_ref().unwrap());
    assert_eq!(
        f.run(|s, v, k| s.account_sso("key", id(&start), SsoAction::Poll, v, k, &f.http))
            .state,
        "ready"
    );
    let done = f.run(|s, v, k| {
        s.finish_yubi_sso_signup(
            "key",
            id(&start),
            Pin::new("654321")?,
            &provider,
            v,
            k,
            &f.http,
        )
    });
    assert!(done.service_access);
    assert_eq!(done.state, "complete");
    let login = f.run(|s, v, k| {
        s.begin_account_sso("key", foks_proto::SsoPurpose::Reauthenticate, v, k, &f.http)
    });
    idp.complete(login.browser_url.as_ref().unwrap());
    f.run(|s, v, k| s.account_sso("key", id(&login), SsoAction::Poll, v, k, &f.http));
    f.run(|s, v, k| {
        let loaded = v.yubi_account("key")?;
        let device = provider.open(&loaded.locator, Some(&pin))?;
        assert!(
            s.finish_yubi_sso_login("key", id(&login), device.as_ref(), v, k, &f.http)?
                .service_access
        );
        Ok(())
    });
}
#[test]
fn denied_browser_result_remains_denied_after_resume() {
    let idp = TestOidcProvider::start();
    let f = Fixture::start(&idp);
    let start =
        f.run(|s, v, k| s.begin_account_sso("work", foks_proto::SsoPurpose::Signup, v, k, &f.http));
    idp.set_denied(true);
    idp.complete(start.browser_url.as_ref().unwrap());
    for action in [SsoAction::Poll, SsoAction::Status, SsoAction::Cancel] {
        let p = f.run(|s, v, k| s.account_sso("work", id(&start), action, v, k, &f.http));
        assert_eq!(p.state, "denied");
        assert!(p.browser_url.is_none());
    }
    assert_ne!(
        f.run(|s, v, k| s.begin_account_sso("work", foks_proto::SsoPurpose::Signup, v, k, &f.http))
            .operation_id,
        start.operation_id
    );
}
