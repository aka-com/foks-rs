use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebAdminConfig {
    pub origin: String,
    pub listen: SocketAddr,
}
impl WebAdminConfig {
    pub fn validate(&self) -> Result<()> {
        let url =
            url::Url::parse(&self.origin).map_err(|_| Error::Config("invalid admin origin"))?;
        let canonical = url.origin().ascii_serialization();
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || (self.origin != canonical && self.origin != format!("{canonical}/"))
            || !self.listen.ip().is_loopback()
            || self.listen.port() == 0
        {
            return Err(Error::Config("admin requires a canonical HTTPS root origin and loopback listener with a nonzero port"));
        }
        Ok(())
    }
    pub(crate) fn origin(&self) -> String {
        self.origin.trim_end_matches('/').to_owned()
    }
    pub(crate) fn authority(&self) -> String {
        self.origin()[8..].to_owned()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_authority_confusion() {
        for origin in [
            "http://admin.test",
            "https://user@admin.test",
            "https://admin.test/path",
            "https://admin.test/?x=1",
            "https://ADMIN.test",
            "https://admin.test#fragment",
        ] {
            assert!(WebAdminConfig {
                origin: origin.into(),
                listen: "127.0.0.1:8445".parse().unwrap()
            }
            .validate()
            .is_err());
        }
        assert!(WebAdminConfig {
            origin: "https://admin.test".into(),
            listen: "127.0.0.1:8445".parse().unwrap()
        }
        .validate()
        .is_ok());
    }
}
