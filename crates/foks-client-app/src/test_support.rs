use super::*;
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;

/// Wall-clock cap for one maintenance turn in tests. Production turns get
/// 50 ms; an owner that makes no progress before that deadline is backed off
/// for 60 s. Under parallel test load a turn can be descheduled past 50 ms,
/// so tests that count turns or drain within a fixed time use this cap. Every
/// maintenance loop is also bounded by record counts (8 removals, 256 scanned
/// entries, 131 probes, 128 prunes), and those bounds still end each turn.
pub(crate) const MAINTENANCE_TURN: Duration = Duration::from_secs(10);

pub(crate) fn maintenance_deadline() -> std::time::Instant {
    std::time::Instant::now() + MAINTENANCE_TURN
}
pub(crate) struct AccountFixture {
    pub(crate) environment: TestEnvironment,
    pub(crate) _server: foks_server_testkit::InProcessServer,
    pub(crate) credentials: ClientCredentials,
    pub(crate) registry: ProfileRegistry,
    // Declared last so the record is removed after the credentials and
    // registry are dropped.
    native_cleanup: NativeManifestCleanup,
}
impl AccountFixture {
    pub(crate) fn start() -> Self {
        Self::start_with_admin(None)
    }
    pub(crate) fn start_native() -> Self {
        Self::start_with_backend(None, CredentialBackend::Native)
    }
    pub(crate) fn start_with_admin(admin: Option<foks_server_testkit::WebAdminConfig>) -> Self {
        Self::start_with_backend(admin, CredentialBackend::PrivateFile)
    }
    fn start_with_backend(
        admin: Option<foks_server_testkit::WebAdminConfig>,
        backend: CredentialBackend,
    ) -> Self {
        let environment = TestEnvironment::new().unwrap();
        let server = match admin {
            Some(admin) => environment.start_web_admin_server(admin, None).unwrap(),
            None => environment.start_server().unwrap(),
        };
        Self::from_server(environment, server, backend)
    }
    pub(crate) fn start_native_oidc(idp: &foks_server_testkit::oidc::TestOidcProvider) -> Self {
        let environment = TestEnvironment::new().unwrap();
        let config = idp.config(environment.client_path("oidc", "secret").unwrap());
        let server = environment.start_oidc_server(config).unwrap();
        Self::from_server(environment, server, CredentialBackend::Native)
    }
    fn from_server(
        environment: TestEnvironment,
        server: foks_server_testkit::InProcessServer,
        backend: CredentialBackend,
    ) -> Self {
        let state = environment.client_path("sso-app", "state").unwrap();
        let root = environment.client_path("sso-app", "root.der").unwrap();
        environment.write_probe_root(&root).unwrap();
        let credentials = ClientCredentials::initialize(&state, backend).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".into(),
                label: None,
                probe: format!(
                    "localhost:{}",
                    environment.addresses().unwrap().probe.port()
                ),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let native_cleanup = NativeManifestCleanup::for_credentials(&credentials);
        let f = Self {
            environment,
            _server: server,
            credentials,
            registry,
            native_cleanup,
        };
        f.run(|s, _, _| {
            s.probe_and_pin()?;
            Ok(())
        });
        f
    }
    pub(crate) fn stop_client(self) -> StoppedAccountFixture {
        let root = self.credentials.root.clone();
        let state_id = self.credentials.state_id.clone();
        drop(self.registry);
        drop(self.credentials);
        StoppedAccountFixture {
            environment: self.environment,
            _server: self._server,
            root,
            state_id,
            _native_cleanup: self.native_cleanup,
        }
    }
    pub(crate) fn run<T>(
        &self,
        op: impl FnOnce(&CheckedProfileSession<'_>, &mut AccountVault<'_>, &[u8; 32]) -> Result<T>,
    ) -> T {
        // Reopen the profile, vault and protected store for every foreground action.
        self.run_in(ProfileSession::open(&self.registry, "local").unwrap(), op)
    }
    /// As [`Self::run`], with `clock` in place of the system adapter clock.
    pub(crate) fn run_with_clock<T>(
        &self,
        clock: std::sync::Arc<dyn AdapterClock>,
        op: impl FnOnce(&CheckedProfileSession<'_>, &mut AccountVault<'_>, &[u8; 32]) -> Result<T>,
    ) -> T {
        let session = ProfileSession::open(&self.registry, "local").unwrap();
        self.run_in(session.with_adapter_clock(clock), op)
    }
    fn run_in<T>(
        &self,
        session: ProfileSession,
        op: impl FnOnce(&CheckedProfileSession<'_>, &mut AccountVault<'_>, &[u8; 32]) -> Result<T>,
    ) -> T {
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

pub(crate) struct StoppedAccountFixture {
    pub(crate) environment: TestEnvironment,
    pub(crate) _server: foks_server_testkit::InProcessServer,
    pub(crate) root: PathBuf,
    pub(crate) state_id: String,
    _native_cleanup: NativeManifestCleanup,
}

/// Removes the native credential manifest of a fixture's client state when
/// the fixture is dropped, so native-backend tests do not leave a record in
/// the developer's Keychain or Secret Service collection. The removal result
/// is ignored: a test may already have removed the record itself.
struct NativeManifestCleanup {
    state_id: Option<String>,
}
impl NativeManifestCleanup {
    fn for_credentials(credentials: &ClientCredentials) -> Self {
        Self {
            state_id: (credentials.backend == CredentialBackend::Native)
                .then(|| credentials.state_id.clone()),
        }
    }
}
impl Drop for NativeManifestCleanup {
    fn drop(&mut self) {
        if let Some(state_id) = &self.state_id {
            if let Ok(mut store) = foks_keystore::NativeCredentialStore::open(state_id) {
                let _ = store.remove(crate::checkpoint::NATIVE_MANIFEST_RECORD);
            }
        }
    }
}
