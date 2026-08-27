use std::path::{Path, PathBuf};
use std::time::Duration;

use foks_client::{EncryptedFileMutationStore, FoksClient, ProbeOutcome, ProbeTarget};
use zeroize::Zeroizing;

use crate::TestEnvironment;

pub struct TestClient {
    client: FoksClient,
    hard_state: PathBuf,
    soft_state: PathBuf,
    protected_state: PathBuf,
    protected_key: [u8; 32],
    target: ProbeTarget,
}

impl TestClient {
    pub fn new(environment: &TestEnvironment, client_id: &str) -> foks_server::Result<Self> {
        Self::new_with_soft_state(environment, client_id, client_id)
    }

    pub fn new_with_fresh_soft_state(
        environment: &TestEnvironment,
        durable_client_id: &str,
        soft_state_id: &str,
    ) -> foks_server::Result<Self> {
        if durable_client_id == soft_state_id {
            return Err(foks_server::Error::Config(
                "fresh soft-state ID must be distinct",
            ));
        }
        Self::new_with_soft_state(environment, durable_client_id, soft_state_id)
    }

    fn new_with_soft_state(
        environment: &TestEnvironment,
        durable_client_id: &str,
        soft_state_id: &str,
    ) -> foks_server::Result<Self> {
        let addresses = environment
            .addresses()
            .ok_or(foks_server::Error::Config("test server is not started"))?;
        let mut client = FoksClient::with_roots(environment.probe_roots());
        client.set_timeout(Duration::from_secs(5));
        Ok(Self {
            client,
            hard_state: environment.client_path(durable_client_id, "hard.sqlite")?,
            soft_state: environment.client_path(soft_state_id, "soft.sqlite")?,
            protected_state: environment.client_path(durable_client_id, "protected")?,
            protected_key: derive_test_key(durable_client_id),
            target: ProbeTarget::parse(&format!("localhost:{}", addresses.probe.port()))
                .map_err(|_| foks_server::Error::Config("invalid test probe target"))?,
        })
    }

    pub fn foks(&self) -> &FoksClient {
        &self.client
    }

    pub fn probe_and_pin(&self) -> foks_client::Result<ProbeOutcome> {
        self.client.probe_and_pin(&self.target, &self.hard_state)
    }

    pub fn pinned_host(&self) -> foks_client::Result<foks_client::PinnedHost> {
        self.client.pinned_host("localhost", &self.hard_state)
    }

    pub fn hard_state_path(&self) -> &Path {
        &self.hard_state
    }

    pub fn soft_state_path(&self) -> &Path {
        &self.soft_state
    }

    pub fn open_protected_store(
        &self,
    ) -> Result<EncryptedFileMutationStore, foks_client::ProtectedStoreError> {
        EncryptedFileMutationStore::open(&self.protected_state, Zeroizing::new(self.protected_key))
    }
}

fn derive_test_key(client_id: &str) -> [u8; 32] {
    let mut key = [0x61; 32];
    for (index, byte) in client_id.bytes().enumerate() {
        key[index % key.len()] ^= byte;
    }
    key
}
