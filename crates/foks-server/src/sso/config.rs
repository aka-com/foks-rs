use crate::{Error, Result};
use foks_oidc::NetworkPolicy;
use foks_proto::{OAuth2Config, OAuth2Secret, SsoConfig, SsoProtocol};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf};

/// Public operator settings. The client secret is loaded from a separate mode-0600 file.
/// A loopback listener is intended for an HTTPS reverse proxy exposing only these two paths.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OidcOperatorConfig {
    pub config_id: [u8; 17],
    pub rollout_id: [u8; 16],
    #[serde(default)]
    pub rollout_mode: OidcRolloutMode,
    pub issuer: String,
    pub discovery_uri: String,
    pub client_id: String,
    pub client_secret_file: PathBuf,
    pub redirect_uri: String,
    pub listen: SocketAddr,
    #[serde(default)]
    pub client_secret_post: bool,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum OidcRolloutMode {
    #[default]
    Migration,
    Enforced,
}
impl From<OidcRolloutMode> for foks_server_db::SsoRolloutMode {
    fn from(mode: OidcRolloutMode) -> Self {
        match mode {
            OidcRolloutMode::Migration => Self::Migration,
            OidcRolloutMode::Enforced => Self::Enforced,
        }
    }
}
impl OidcOperatorConfig {
    pub fn validate(&self, policy: NetworkPolicy) -> Result<()> {
        for value in [&self.issuer, &self.discovery_uri, &self.redirect_uri] {
            policy.check_url(value)?;
        }
        let redirect = policy.check_url(&self.redirect_uri)?;
        if self.rollout_id == [0; 16]
            || self.config_id[0] != 50
            || self.config_id[1..].iter().all(|b| *b == 0)
            || self.client_id.is_empty()
            || self.client_id.len() > 1024
            || !self.client_secret_file.is_absolute()
            || !self.listen.ip().is_loopback()
            || redirect.query().is_some()
            || redirect.path() == "/"
            || redirect.path() == "/oauth2/start"
        {
            return Err(Error::Config("invalid OIDC operator configuration"));
        }
        Ok(())
    }
    pub fn public(&self) -> SsoConfig {
        SsoConfig {
            active: SsoProtocol::Oauth2,
            oauth2: Some(OAuth2Config {
                id: self.config_id,
                config_uri: self.discovery_uri.clone(),
                client_id: self.client_id.clone(),
                client_secret: OAuth2Secret::new(String::new()),
                redirect_uri: self.redirect_uri.clone(),
            }),
        }
    }
    pub fn configured_fingerprint(&self) -> Result<[u8; 32]> {
        self.fingerprint(&self.load_secret()?)
    }
    pub(crate) fn load_secret(&self) -> Result<zeroize::Zeroizing<String>> {
        let raw = crate::keys::read_secret_file(&self.client_secret_file, 4096)?;
        let secret = zeroize::Zeroizing::new(
            std::str::from_utf8(&raw)
                .map_err(|_| Error::Config("OIDC secret must be UTF-8"))?
                .to_owned(),
        );
        if secret.is_empty() || secret.contains(['\r', '\n', '\0']) {
            return Err(Error::Config(
                "OIDC secret must be nonempty with no newline or NUL",
            ));
        }
        Ok(secret)
    }
    /// Validates provider reachability and security inputs before an operator transition.
    pub fn validate_provider(
        &self,
        policy: NetworkPolicy,
        keys: &dyn crate::keys::HostKeyProvider,
    ) -> Result<[u8; 32]> {
        self.validate(policy)?;
        let secret = self.load_secret()?;
        keys.load_existing(crate::keys::KeyPurpose::Recovery)?;
        let http = foks_oidc::ProviderHttp::new(policy)?;
        let provider = foks_oidc::Provider::discover(&http, &self.discovery_uri, &self.client_id)?;
        if provider.metadata.issuer().as_str() != self.issuer {
            return Err(Error::Config("OIDC discovery issuer mismatch"));
        }
        self.fingerprint(&secret)
    }
    pub(crate) fn fingerprint(&self, secret: &str) -> Result<[u8; 32]> {
        // Versioned, length-delimited security projection. Operational file paths and
        // rollout state are deliberately absent; secret bytes remain zeroized.
        let mut hash_input = zeroize::Zeroizing::new(b"fennec-oidc-provider-v1".to_vec());
        for value in [
            self.config_id.as_slice(),
            self.issuer.as_bytes(),
            self.discovery_uri.as_bytes(),
            self.client_id.as_bytes(),
            self.redirect_uri.as_bytes(),
            secret.as_bytes(),
        ] {
            hash_input.extend_from_slice(&(value.len() as u64).to_be_bytes());
            hash_input.extend_from_slice(value);
        }
        hash_input.push(u8::from(self.client_secret_post));
        Ok(foks_crypto::prefixed_hash(
            0x9125_9bfe_5daf_a940,
            &hash_input,
        ))
    }
}
