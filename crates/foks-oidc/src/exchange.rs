//! SDK-managed authorization and token requests through the bounded transport.
use crate::{Error, Provider, ProviderHttp, VerifiedIdentity};
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreTokenResponse},
    AuthType, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, Nonce, OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, RefreshToken, Scope, TokenResponse,
};
use zeroize::Zeroizing;

type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;
/// Owned strings are cleared on drop; Debug never exposes provider credentials.
pub struct ProviderTokens {
    pub access: Zeroizing<String>,
    pub id: Zeroizing<String>,
    pub refresh: Option<Zeroizing<String>>,
    pub access_expires_at_ms: u64,
}
impl std::fmt::Debug for ProviderTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderTokens([REDACTED])")
    }
}
pub struct ProviderClient<'a> {
    pub client_id: &'a str,
    pub secret: &'a str,
    pub redirect_uri: &'a str,
    pub secret_in_body: bool,
}
impl Provider {
    fn client(&self, config: &ProviderClient<'_>) -> Result<Client, Error> {
        Ok(CoreClient::from_provider_metadata(
            self.metadata.clone(),
            ClientId::new(config.client_id.into()),
            if config.secret.is_empty() {
                None
            } else {
                Some(ClientSecret::new(config.secret.into()))
            },
        )
        .set_redirect_uri(
            RedirectUrl::new(config.redirect_uri.into()).map_err(|_| Error::Configuration)?,
        )
        .set_auth_type(if config.secret_in_body {
            AuthType::RequestBody
        } else {
            AuthType::BasicAuth
        }))
    }
    pub fn authorization_url(
        &self,
        config: &ProviderClient<'_>,
        state: &str,
        nonce: &str,
        verifier: &str,
    ) -> Result<String, Error> {
        let client = self.client(config)?;
        let state = CsrfToken::new(state.into());
        let nonce = Nonce::new(nonce.into());
        let mut request = client.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            move || state,
            move || nonce,
        );
        if (43..=128).contains(&verifier.len()) {
            request = request.set_pkce_challenge(PkceCodeChallenge::from_code_verifier_sha256(
                &PkceCodeVerifier::new(verifier.into()),
            ));
        } else if verifier.len() == 27 {
            // The pinned Go client emits 20 random bytes (27 characters). Its exact S256
            // construction is wire-compatible, but the SDK's RFC-length assertion would
            // panic. Pass this explicit compatibility exception through the SDK parameter
            // API; the provider still enforces its verifier policy during exchange.
            use base64::Engine as _;
            use sha2::Digest as _;
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(sha2::Sha256::digest(verifier.as_bytes()));
            request = request
                .add_extra_param("code_challenge", challenge)
                .add_extra_param("code_challenge_method", "S256");
        } else {
            return Err(Error::Configuration);
        }
        // Only request claims/scopes the provider advertises, besides required openid.
        for scope in ["profile", "email", "offline_access"] {
            if self
                .metadata
                .scopes_supported()
                .is_some_and(|v| v.iter().any(|s| s.as_str() == scope))
            {
                request = request.add_scope(Scope::new(scope.into()));
            }
        }
        Ok(request.url().0.into())
    }
    pub fn exchange(
        &self,
        http: &ProviderHttp,
        config: &ProviderClient<'_>,
        code: &str,
        verifier: &str,
        nonce: &str,
        now_ms: u64,
    ) -> Result<(ProviderTokens, VerifiedIdentity), Error> {
        let response = self
            .client(config)?
            .exchange_code(AuthorizationCode::new(code.into()))
            .map_err(|_| Error::Configuration)?
            .set_pkce_verifier(PkceCodeVerifier::new(verifier.into()))
            .request(http)
            .map_err(|e| match e {
                openidconnect::RequestTokenError::ServerResponse(_) => Error::GrantRejected,
                _ => Error::ProviderUnavailable,
            })?;
        self.checked_tokens(http, response, nonce, now_ms)
    }
    pub fn refresh(
        &self,
        http: &ProviderHttp,
        config: &ProviderClient<'_>,
        refresh: &str,
        nonce: &str,
        now_ms: u64,
    ) -> Result<(ProviderTokens, Option<VerifiedIdentity>), Error> {
        let token = RefreshToken::new(refresh.into());
        let client = self.client(config)?;
        let response = client
            .exchange_refresh_token(&token)
            .map_err(|_| Error::Configuration)?
            .request(http)
            .map_err(|e| match e {
                openidconnect::RequestTokenError::ServerResponse(_) => Error::GrantRejected,
                _ => Error::ProviderUnavailable,
            })?;
        let id = Zeroizing::new(
            response
                .id_token()
                .map(|t| t.to_string())
                .unwrap_or_default(),
        );
        let identity = if id.is_empty() {
            None
        } else {
            Some(match self.validator.validate_refresh(&id, nonce, now_ms) {
                Ok(v) => v,
                Err(Error::Token) => {
                    let keys = http.get(self.metadata.jwks_uri().as_str())?;
                    crate::TokenValidator::new(
                        self.metadata.issuer().as_str().to_owned(),
                        self.validator.client_id().to_owned(),
                        &keys,
                    )?
                    .validate_refresh(&id, nonce, now_ms)?
                }
                Err(e) => return Err(e),
            })
        };
        let expires = now_ms
            .checked_add(
                response
                    .expires_in()
                    .ok_or(Error::Token)?
                    .as_millis()
                    .try_into()
                    .map_err(|_| Error::Token)?,
            )
            .ok_or(Error::Token)?;
        let expires = identity
            .as_ref()
            .map_or(expires, |i| expires.min(i.expires_at_ms));
        if expires <= now_ms || response.access_token().secret().is_empty() {
            return Err(Error::Token);
        }
        Ok((
            ProviderTokens {
                access: Zeroizing::new(response.access_token().secret().clone()),
                id,
                refresh: response
                    .refresh_token()
                    .map(|r| Zeroizing::new(r.secret().clone())),
                access_expires_at_ms: expires,
            },
            identity,
        ))
    }
    fn checked_tokens(
        &self,
        http: &ProviderHttp,
        response: CoreTokenResponse,
        nonce: &str,
        now_ms: u64,
    ) -> Result<(ProviderTokens, VerifiedIdentity), Error> {
        let id = Zeroizing::new(response.id_token().ok_or(Error::Token)?.to_string());
        let identity = match self.validator.validate(&id, nonce, now_ms) {
            Ok(identity) => identity,
            Err(Error::Token) => {
                // A provider may rotate signing keys between discovery and code exchange.
                // Refresh only the trusted JWKS once; never replay the consumed code.
                let keys = http.get(self.metadata.jwks_uri().as_str())?;
                crate::TokenValidator::new(
                    self.metadata.issuer().as_str().to_owned(),
                    self.validator.client_id().to_owned(),
                    &keys,
                )?
                .validate(&id, nonce, now_ms)?
            }
            Err(error) => return Err(error),
        };
        let expires = now_ms
            .checked_add(
                response
                    .expires_in()
                    .ok_or(Error::Token)?
                    .as_millis()
                    .try_into()
                    .map_err(|_| Error::Token)?,
            )
            .ok_or(Error::Token)?;
        if expires <= now_ms || response.access_token().secret().is_empty() {
            return Err(Error::Token);
        }
        Ok((
            ProviderTokens {
                access: Zeroizing::new(response.access_token().secret().clone()),
                id,
                refresh: response
                    .refresh_token()
                    .map(|r| Zeroizing::new(r.secret().clone())),
                access_expires_at_ms: expires.min(identity.expires_at_ms),
            },
            identity,
        ))
    }
}
