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
    pub issuer: String,
    pub discovery_uri: String,
    pub client_id: String,
    pub client_secret_file: PathBuf,
    pub redirect_uri: String,
    pub listen: SocketAddr,
    #[serde(default)]
    pub client_secret_post: bool,
}
impl OidcOperatorConfig {
    pub fn validate(&self, policy: NetworkPolicy) -> Result<()> {
        for value in [&self.issuer, &self.discovery_uri, &self.redirect_uri] {
            policy.check_url(value)?;
        }
        let redirect = policy.check_url(&self.redirect_uri)?;
        if self.config_id[0] != 50
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
    pub(crate) fn fingerprint(&self, secret: &str) -> Result<[u8; 32]> {
        // Secret rotation fences outstanding flows even when public config is accidentally reused.
        let mut material = zeroize::Zeroizing::new(toml::to_string(self)?.into_bytes());
        material.extend_from_slice(secret.as_bytes());
        Ok(foks_crypto::prefixed_hash(0x9125_9bfe_5daf_a940, &material))
    }
}
