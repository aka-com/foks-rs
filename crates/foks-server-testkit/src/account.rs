use foks_client::{
    CreatedSoftwareAccount, PinnedHost, SoftwareAccountRequest, SoftwareAccountSecrets,
};
use foks_proto::{InviteCode, SecretSeed};

use crate::TestClient;

#[derive(Clone, Debug)]
pub struct TestAccountSpec {
    pub username: String,
    pub seed_byte: u8,
    pub invite_code: InviteCode,
}

impl TestAccountSpec {
    pub fn new(username: impl Into<String>, seed_byte: u8) -> Self {
        Self {
            username: username.into(),
            seed_byte,
            invite_code: InviteCode::Empty,
        }
    }

    pub fn with_invite(mut self, invite_code: InviteCode) -> Self {
        self.invite_code = invite_code;
        self
    }
}

impl TestClient {
    pub fn create_account(
        &self,
        host: &PinnedHost,
        spec: &TestAccountSpec,
    ) -> foks_client::Result<CreatedSoftwareAccount> {
        self.create_account_with_optional_passphrase(host, spec, None)
    }

    pub fn create_account_with_passphrase(
        &self,
        host: &PinnedHost,
        spec: &TestAccountSpec,
        passphrase: &str,
    ) -> foks_client::Result<CreatedSoftwareAccount> {
        self.create_account_with_optional_passphrase(
            host,
            spec,
            Some(foks_client::Passphrase::new(passphrase)?),
        )
    }

    fn create_account_with_optional_passphrase(
        &self,
        host: &PinnedHost,
        spec: &TestAccountSpec,
        passphrase: Option<foks_client::Passphrase>,
    ) -> foks_client::Result<CreatedSoftwareAccount> {
        let mut protected = self
            .open_protected_store()
            .map_err(|error| foks_client::Error::ProtectedMaterial(error.to_string()))?;
        self.foks().create_software_account(
            host,
            SoftwareAccountRequest {
                username_utf8: spec.username.clone(),
                device_name: format!("{} device", spec.username),
                invite_code: spec.invite_code.clone(),
                email: format!("{}@example.test", spec.username),
                passphrase,
            },
            SoftwareAccountSecrets::new(
                SecretSeed::new([spec.seed_byte; 32]),
                SecretSeed::new([spec.seed_byte.wrapping_add(1); 32]),
                [spec.seed_byte.wrapping_add(2); 17],
            ),
            self.soft_state_path(),
            &mut protected,
        )
    }
}
