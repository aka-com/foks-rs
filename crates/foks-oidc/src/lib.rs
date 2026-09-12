//! Shared OIDC policy. A validated provider token is not a FOKS device credential.
#![forbid(unsafe_code)]
mod provider;
use chrono::{DateTime, Utc};
use openidconnect::{
    core::{CoreIdToken, CoreIdTokenVerifier, CoreJsonWebKeySet, CoreJwsSigningAlgorithm},
    ClientId, IssuerUrl, Nonce,
};
pub use provider::*;
use thiserror::Error;

pub const MAX_PROVIDER_BYTES: usize = 1024 * 1024;
pub const SESSION_LIFETIME_MS: u64 = 600_000;
pub const MAX_POLL_MS: u64 = 30_000;
pub const PROVIDER_DEADLINE_MS: u64 = 10_000;
pub const MAX_HOST_SESSIONS: usize = 1000;
pub const MAX_IDENTITY_SESSIONS: usize = 4;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum Error {
    #[error("OIDC provider is unavailable")]
    ProviderUnavailable,
    #[error("invalid OIDC provider configuration")]
    Configuration,
    #[error("OIDC provider document exceeds its bound")]
    DocumentLimit,
    #[error("OIDC ID token validation failed")]
    Token,
}

/// Only these validated fields can select the durable host/issuer/subject linkage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedIdentity {
    pub issuer: String,
    pub subject: String,
    pub username: String,
    pub email: Option<String>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

/// Keys come from the configured provider's bounded discovery/JWKS policy.
/// This object does not fetch URLs supplied by the token or browser.
pub struct TokenValidator {
    issuer: IssuerUrl,
    client_id: ClientId,
    keys: CoreJsonWebKeySet,
}
impl TokenValidator {
    pub fn new(issuer: String, client_id: String, jwks: &[u8]) -> Result<Self, Error> {
        if jwks.len() > MAX_PROVIDER_BYTES {
            return Err(Error::DocumentLimit);
        }
        if client_id.is_empty() {
            return Err(Error::Configuration);
        }
        let issuer = IssuerUrl::new(issuer).map_err(|_| Error::Configuration)?;
        let keys = serde_json::from_slice(jwks).map_err(|_| Error::Configuration)?;
        Ok(Self {
            issuer,
            client_id: ClientId::new(client_id),
            keys,
        })
    }
    pub fn validate(&self, raw: &str, nonce: &str, now_ms: u64) -> Result<VerifiedIdentity, Error> {
        if raw.len() > MAX_PROVIDER_BYTES {
            return Err(Error::DocumentLimit);
        }
        if nonce.is_empty() {
            return Err(Error::Token);
        }
        let now = DateTime::<Utc>::from_timestamp_millis(
            i64::try_from(now_ms).map_err(|_| Error::Token)?,
        )
        .ok_or(Error::Token)?;
        let token: CoreIdToken = raw.parse().map_err(|_| Error::Token)?;
        let verifier = CoreIdTokenVerifier::new_public_client(
            self.client_id.clone(),
            self.issuer.clone(),
            self.keys.clone(),
        )
        .set_allowed_algs([CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256])
        .set_other_audience_verifier_fn(|_| true)
        .set_time_fn(move || now)
        .set_issue_time_verifier_fn(move |issued| {
            if issued > now || issued.timestamp_millis() < 0 {
                Err("invalid issue time".into())
            } else {
                Ok(())
            }
        });
        let claims = token
            .claims(&verifier, &Nonce::new(nonce.to_owned()))
            .map_err(|_| Error::Token)?;
        // SDK deliberately leaves azp to callers. Multi-audience tokens require it.
        if claims
            .authorized_party()
            .is_some_and(|azp| azp.as_str() != self.client_id.as_str())
            || (claims.audiences().len() > 1 && claims.authorized_party().is_none())
            || claims.subject().as_str().is_empty()
            || claims.subject().as_str().len() > 255
            || claims.expiration() <= claims.issue_time()
            || claims.issuer().as_str() != self.issuer.as_str()
        {
            return Err(Error::Token);
        }
        let email = claims.email().map(|s| s.as_str().to_owned());
        let username = claims
            .preferred_username()
            .map(|s| s.as_str().to_owned())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                email
                    .as_ref()
                    .and_then(|s| s.split_once('@').map(|(local, _)| local.to_owned()))
            })
            .unwrap_or_default();
        Ok(VerifiedIdentity {
            issuer: self.issuer.as_str().to_owned(),
            subject: claims.subject().as_str().to_owned(),
            username,
            email,
            issued_at_ms: u64::try_from(claims.issue_time().timestamp_millis())
                .map_err(|_| Error::Token)?,
            expires_at_ms: u64::try_from(claims.expiration().timestamp_millis())
                .map_err(|_| Error::Token)?,
        })
    }
}

/// Durable owners CAS these states; this graph cannot grant device or service authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Created,
    AwaitingBrowser,
    Exchanging,
    Ready,
    Binding,
    Completed,
    Expired,
    Cancelled,
    ExchangeUnknown,
    Denied,
    Rejected,
}
impl SessionState {
    pub fn permits(self, next: Self) -> bool {
        use SessionState::*;
        matches!(
            (self, next),
            (Created, AwaitingBrowser | Cancelled | Expired)
                | (AwaitingBrowser, Exchanging | Cancelled | Expired | Denied)
                | (Exchanging, Ready | ExchangeUnknown | Expired)
                | (Ready, Binding | Cancelled | Expired)
                | (Binding, Completed | Rejected)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("tests/fixtures/{name}")).unwrap()
    }
    #[test]
    fn signed_claims_require_exact_identity_and_session_authority() {
        let validator = TokenValidator::new(
            "https://idp.example".into(),
            "fennec".into(),
            fixture("jwks.json").as_bytes(),
        )
        .unwrap();
        for name in ["valid", "multiple-audience"] {
            let claims = validator
                .validate(
                    &fixture(&format!("{name}.jwt")),
                    "fixture-nonce",
                    1_700_000_100_000,
                )
                .unwrap();
            assert_eq!(claims.subject, "stable-subject");
            assert_eq!(claims.username, "alice");
        }
        for name in [
            "wrong-issuer",
            "issuer-slash",
            "wrong-audience",
            "wrong-nonce",
            "expired",
            "missing-subject",
            "empty-subject",
            "missing-expiry",
            "wrong-azp",
            "multiple-audience-no-azp",
            "future-issued",
            "claim-type",
            "missing-nonce",
            "wrong-key-id",
            "wrong-algorithm",
        ] {
            assert_eq!(
                validator.validate(
                    &fixture(&format!("{name}.jwt")),
                    "fixture-nonce",
                    1_700_000_100_000
                ),
                Err(Error::Token),
                "{name}"
            );
        }
        assert_eq!(
            validator.validate(&"x".repeat(MAX_PROVIDER_BYTES + 1), "n", 0),
            Err(Error::DocumentLimit)
        );
        assert_eq!(
            validator.validate(&fixture("valid.jwt"), "", 1_700_000_100_000),
            Err(Error::Token)
        );
    }
    #[test]
    fn uncertain_exchange_and_final_submission_cannot_restart() {
        use SessionState::*;
        assert!(AwaitingBrowser.permits(Exchanging));
        assert!(Exchanging.permits(ExchangeUnknown));
        for state in [
            Completed,
            Expired,
            Cancelled,
            ExchangeUnknown,
            Denied,
            Rejected,
        ] {
            for next in [
                Created,
                AwaitingBrowser,
                Exchanging,
                Ready,
                Binding,
                Completed,
            ] {
                assert!(!state.permits(next));
            }
        }
        assert!(!Binding.permits(Cancelled));
        assert!(!Binding.permits(Expired));
        assert!(Binding.permits(Completed));
    }
}
