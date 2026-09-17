use super::*;
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;
pub(crate) struct AccountFixture {
    pub(crate) environment: TestEnvironment,
    pub(crate) _server: foks_server_testkit::InProcessServer,
    pub(crate) credentials: ClientCredentials,
    pub(crate) registry: ProfileRegistry,
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
        let native = self.credentials.backend == CredentialBackend::Native;
        drop(self.registry);
        drop(self.credentials);
        StoppedAccountFixture {
            environment: self.environment,
            _server: self._server,
            root,
            state_id,
            native,
        }
    }
    pub(crate) fn run<T>(
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

pub(crate) struct StoppedAccountFixture {
    pub(crate) environment: TestEnvironment,
    pub(crate) _server: foks_server_testkit::InProcessServer,
    pub(crate) root: PathBuf,
    pub(crate) state_id: String,
    native: bool,
}
impl Drop for StoppedAccountFixture {
    fn drop(&mut self) {
        if self.native {
            if let Ok(mut store) = foks_keystore::NativeCredentialStore::open(&self.state_id) {
                let _ = store.remove(crate::checkpoint::NATIVE_MANIFEST_RECORD);
            }
        }
    }
}
