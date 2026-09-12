//! Go hosted-admin handoff: session ownership and destination policy are separate checks.
use crate::*;
use url::Url;
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum WebAdminError {
    #[error("host administration is unsupported")]
    Unsupported,
    #[error("host administration is unavailable")]
    Unavailable,
    #[error("admin session is expired or invalid")]
    Expired,
    #[error("admin session belongs to another account")]
    WrongAccount,
    #[error("admin destination is not allowed")]
    DestinationRejected,
    #[error("organization sign-in is required")]
    ReauthenticationRequired,
}
impl WebAdminError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
            Self::Expired => "expired",
            Self::WrongAccount => "wrong-account",
            Self::DestinationRejected => "destination-rejected",
            Self::ReauthenticationRequired => "reauthentication-required",
        }
    }
}
#[derive(Clone, Debug)]
pub struct AdminDestination(Url);
impl AdminDestination {
    pub fn new(input: &str) -> std::result::Result<Self, WebAdminError> {
        Self::parse(input, false)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_test(input: &str) -> std::result::Result<Self, WebAdminError> {
        Self::parse(input, true)
    }
    fn parse(input: &str, loopback: bool) -> std::result::Result<Self, WebAdminError> {
        let url = Url::parse(input).map_err(|_| WebAdminError::DestinationRejected)?;
        let local = loopback && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if input.len() > 2048
            || input
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
            || (url.scheme() != "https" && !(local && url.scheme() == "http"))
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || (input != url.as_str() && input != url.as_str().trim_end_matches('/'))
        {
            return Err(WebAdminError::DestinationRejected);
        }
        Ok(Self(url))
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
    pub fn validate_handoff(&self, input: &str) -> std::result::Result<(), WebAdminError> {
        if input.len() > 8192
            || input
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        {
            return Err(WebAdminError::DestinationRejected);
        }
        let (base, query) = input
            .split_once('?')
            .ok_or(WebAdminError::DestinationRejected)?;
        if base != self.as_str() && base != self.as_str().trim_end_matches('/') {
            return Err(WebAdminError::DestinationRejected);
        }
        let (key, value) = query
            .split_once('=')
            .ok_or(WebAdminError::DestinationRejected)?;
        if key.is_empty()
            || key.len() > 64
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'~'))
            || value.is_empty()
            || value.len() > 256
            || !value.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(WebAdminError::DestinationRejected);
        }
        Ok(())
    }
}
pub struct WebAdminHandoff {
    url: Zeroizing<String>,
}
impl std::fmt::Debug for WebAdminHandoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebAdminHandoff(<redacted>)")
    }
}
impl WebAdminHandoff {
    pub fn expose(&self) -> &str {
        &self.url
    }
}
fn classify(e: Error) -> WebAdminError {
    match e {
        Error::Rpc(foks_rpc::Error::RemoteStatus { code, .. }) => match code {
            1020 => WebAdminError::Unsupported,
            1018 => WebAdminError::WrongAccount,
            1058 | 1062 => WebAdminError::Expired,
            1069 => WebAdminError::ReauthenticationRequired,
            _ => WebAdminError::Unavailable,
        },
        _ => WebAdminError::Unavailable,
    }
}
impl FoksClient {
    pub fn new_web_admin_handoff(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        destination: &AdminDestination,
    ) -> std::result::Result<WebAdminHandoff, WebAdminError> {
        // Bot admin admission remains separate from ordinary token transport support.
        if matches!(credential,FederationCredential::Software(c) if c.key_kind==SoftwareKeyKind::BotToken)
        {
            return Err(WebAdminError::Unsupported);
        }
        self.authenticate_credential_and_pin(host, credential)
            .map_err(classify)?;
        let (seed, certs) = credential.transport();
        let frame = foks_rpc::encode_new_web_admin_panel_url_request_at(0)
            .map_err(|_| WebAdminError::Unavailable)?;
        let bytes = Zeroizing::new(
            self.call_with_material(host, &host.user, &frame, seed, certs)
                .map_err(classify)?,
        );
        let Value::Text(raw) = decode(&bytes).map_err(|_| WebAdminError::Unavailable)? else {
            return Err(WebAdminError::Unavailable);
        };
        let raw = Zeroizing::new(raw);
        let url = Zeroizing::new(
            std::str::from_utf8(&raw)
                .map_err(|_| WebAdminError::Unavailable)?
                .to_owned(),
        );
        destination.validate_handoff(&url)?;
        self.check_web_admin_session(host, credential, &url)?;
        Ok(WebAdminHandoff { url })
    }
    /// Verifies only Go's session ownership/validity. Does not authorize navigation.
    pub fn check_web_admin_session(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        url: &str,
    ) -> std::result::Result<(), WebAdminError> {
        if url.len() > 8192 {
            return Err(WebAdminError::DestinationRejected);
        }
        let request = Zeroizing::new(
            foks_rpc::encode_check_url_request_at(url, 0)
                .map_err(|_| WebAdminError::Unavailable)?,
        );
        let (seed, certs) = credential.transport();
        self.call_void_with_material(host, &host.user, &request, seed, certs)
            .map_err(classify)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_bytes_do_not_authorize_a_destination() {
        let p = AdminDestination::new("https://admin.example/").unwrap();
        assert!(p
            .validate_handoff("https://admin.example?session=abc123")
            .is_ok());
        for s in [
            "https://evil.example/?session=abc123",
            "https://admin.example.evil/?session=abc123",
            "https://admin.example:444/?session=abc123",
            "https://admin.example/a/..?session=abc123",
            "https://admin.example/?session=abc&next=https://evil.test",
            "https://admin.example/?session=abc#fragment",
            "https://admin.example/?session=abc%0a",
        ] {
            assert!(p.validate_handoff(s).is_err());
        }
        assert!(AdminDestination::new("http://localhost/").is_err());
        assert!(AdminDestination::new("https://admin.example/a/..").is_err());
    }
    #[test]
    fn bearer_debug_is_redacted() {
        let h = WebAdminHandoff {
            url: Zeroizing::new("secret".into()),
        };
        assert!(!format!("{h:?}").contains("secret"));
    }
}
