//! Pinned Go OIDC wire objects. Decoding establishes shape, never authorization.
use crate::{
    array, binary, boolean, decode, encode, entity, fixed_blob, option, text, unsigned, variant,
    EntityId, Error, Result, Signature, TreeRoot, UsernameReservation, Value, ENTITY_HOST,
    ENTITY_USER,
};
use zeroize::Zeroizing;

pub const OAUTH2_TOKEN_SET_TYPE_ID: u64 = 0x9c72_432b_d3f5_bfc8;
pub const OAUTH2_BINDING_TYPE_ID: u64 = 0xa785_bb21_f4d7_13b6;
pub const OAUTH2_ID_TOKEN_BINDING_BLOB_TYPE_ID: u64 = 0x81c5_c069_5efd_e1c9;

/// Secrets never enter derived diagnostics. Encoded wire buffers remain the caller's responsibility.
#[derive(Clone, Eq, PartialEq)]
pub struct OAuth2Secret(Zeroizing<String>);
impl OAuth2Secret {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    fn value(&self) -> Value {
        Value::Text(self.expose().as_bytes().to_vec())
    }
    fn parse(value: &Value) -> Result<Self> {
        Ok(Self::new(text(value)?))
    }
}
impl std::fmt::Debug for OAuth2Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OAuth2Secret([REDACTED])")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct OAuth2SessionId(pub [u8; 17]);
impl std::fmt::Debug for OAuth2SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OAuth2SessionId([REDACTED])")
    }
}
impl OAuth2SessionId {
    fn value(&self) -> Value {
        Value::Binary(self.0.to_vec())
    }
    fn parse(v: &Value) -> Result<Self> {
        Ok(Self(fixed_blob(v, "OAuth2 session ID")?))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OAuth2Config {
    pub id: [u8; 17],
    pub config_uri: String,
    pub client_id: String,
    pub client_secret: OAuth2Secret,
    pub redirect_uri: String,
}
impl OAuth2Config {
    fn value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.id.to_vec()),
            string(&self.config_uri),
            string(&self.client_id),
            self.client_secret.value(),
            string(&self.redirect_uri),
        ])
    }
    fn parse(v: &Value) -> Result<Self> {
        let f = array(v, 5)?;
        Ok(Self {
            id: fixed_blob(&f[0], "SSO config ID")?,
            config_uri: text(&f[1])?,
            client_id: text(&f[2])?,
            client_secret: OAuth2Secret::parse(&f[3])?,
            redirect_uri: text(&f[4])?,
        })
    }
    pub fn public(&self) -> Self {
        Self {
            client_secret: OAuth2Secret::new(String::new()),
            ..self.clone()
        }
    }
}

/// SAML is representable in Go's configuration enum but has no implemented flow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SsoConfig {
    pub active: SsoProtocol,
    pub oauth2: Option<OAuth2Config>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum SsoProtocol {
    None = 0,
    Oauth2 = 1,
    Saml = 2,
}
impl SsoConfig {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 2)?;
        let active = match unsigned(&f[0])? {
            0 => SsoProtocol::None,
            1 => SsoProtocol::Oauth2,
            2 => SsoProtocol::Saml,
            value => {
                return Err(Error::UnknownEnum {
                    kind: "SSO protocol",
                    value,
                })
            }
        };
        Ok(Self {
            active,
            oauth2: option(&f[1], OAuth2Config::parse)?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Unsigned(self.active as u64),
            self.oauth2
                .as_ref()
                .map_or(Value::Null, OAuth2Config::value),
        ]))?)
    }
    pub fn public(&self) -> Self {
        Self {
            active: self.active,
            oauth2: self.oauth2.as_ref().map(OAuth2Config::public),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OAuth2Binding {
    pub uid: EntityId,
    pub host: EntityId,
    pub root: TreeRoot,
    pub random: [u8; 16],
}
impl OAuth2Binding {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 3)?;
        let fqu = array(&f[0], 2)?;
        Ok(Self {
            uid: entity(&fqu[0])?.require_type(ENTITY_USER)?,
            host: entity(&fqu[1])?.require_type(ENTITY_HOST)?,
            root: TreeRoot::decode(&encode(&f[1])?)?,
            random: fixed_blob(&f[2], "OAuth2 binding randomness")?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.uid.clone().require_type(ENTITY_USER)?;
        self.host.clone().require_type(ENTITY_HOST)?;
        Ok(encode(&Value::Array(vec![
            Value::Array(vec![
                Value::Binary(self.uid.as_bytes().to_vec()),
                Value::Binary(self.host.as_bytes().to_vec()),
            ]),
            self.root.to_value(),
            Value::Binary(self.random.to_vec()),
        ]))?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OAuth2TokenSet {
    pub access_token: OAuth2Secret,
    pub id_token: OAuth2Secret,
    pub expires_at: u64,
    pub username: String,
}
impl OAuth2TokenSet {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 4)?;
        Ok(Self {
            access_token: OAuth2Secret::parse(&f[0])?,
            id_token: OAuth2Secret::parse(&f[1])?,
            expires_at: unsigned(&f[2])?,
            username: text(&f[3])?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            self.access_token.value(),
            self.id_token.value(),
            Value::Unsigned(self.expires_at),
            string(&self.username),
        ]))?)
    }
}
#[derive(Clone, Eq, PartialEq)]
pub struct OAuth2PollResult {
    pub tokens: OAuth2TokenSet,
    pub reservation: UsernameReservation,
}
impl std::fmt::Debug for OAuth2PollResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OAuth2PollResult([REDACTED])")
    }
}
impl OAuth2PollResult {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 2)?;
        Ok(Self {
            tokens: OAuth2TokenSet::decode(&encode(&f[0])?)?,
            reservation: UsernameReservation::decode(&encode(&f[1])?)?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            decode(&self.tokens.encoded()?)?,
            decode(&self.reservation.encoded()?)?,
        ]))?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OAuth2IdTokenBindingPayload {
    pub id_token: OAuth2Secret,
    pub binding: OAuth2Binding,
}
impl OAuth2IdTokenBindingPayload {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 2)?;
        Ok(Self {
            id_token: OAuth2Secret::parse(&f[0])?,
            binding: OAuth2Binding::decode(&encode(&f[1])?)?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            self.id_token.value(),
            decode(&self.binding.encoded()?)?,
        ]))?)
    }
}
#[derive(Clone, Eq, PartialEq)]
pub struct OAuth2IdTokenBinding {
    pub inner: Zeroizing<Vec<u8>>,
    pub signature: Signature,
    pub key: EntityId,
}
impl std::fmt::Debug for OAuth2IdTokenBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2IdTokenBinding")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}
impl OAuth2IdTokenBinding {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 3)?;
        let inner = Zeroizing::new(binary(&f[0])?.to_vec());
        OAuth2IdTokenBindingPayload::decode(&inner)?;
        Ok(Self {
            inner,
            signature: Signature::decode(&encode(&f[1])?)?,
            key: crate::user_member_entity(&f[2])?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        OAuth2IdTokenBindingPayload::decode(&self.inner)?;
        crate::user_member_entity(&Value::Binary(self.key.as_bytes().to_vec()))?;
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.inner.to_vec()),
            self.signature.to_value(),
            Value::Binary(self.key.as_bytes().to_vec()),
        ]))?)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegSsoArgs {
    None,
    Oauth2 {
        id: OAuth2SessionId,
        binding: OAuth2IdTokenBinding,
    },
}
impl RegSsoArgs {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 2)?;
        match unsigned(&f[0])? {
            0 if f[1] == Value::Variant(None) => Ok(Self::None),
            1 => {
                let a = array(variant(&f[1], "1")?, 2)?;
                Ok(Self::Oauth2 {
                    id: OAuth2SessionId::parse(&a[0])?,
                    binding: OAuth2IdTokenBinding::decode(&encode(&a[1])?)?,
                })
            }
            value => Err(Error::UnknownEnum {
                kind: "registration SSO protocol",
                value,
            }),
        }
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let v = match self {
            Self::None => Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
            Self::Oauth2 { id, binding } => Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((
                    b"1".to_vec(),
                    Box::new(Value::Array(vec![id.value(), decode(&binding.encoded()?)?])),
                ))),
            ]),
        };
        Ok(encode(&v)?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitOAuth2SessionArgument {
    pub id: OAuth2SessionId,
    pub pkce_verifier: OAuth2Secret,
    pub nonce: OAuth2Secret,
    pub uid: Option<EntityId>,
}
impl InitOAuth2SessionArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 4)?;
        Ok(Self {
            id: OAuth2SessionId::parse(&f[0])?,
            pkce_verifier: OAuth2Secret::parse(&f[1])?,
            nonce: OAuth2Secret::parse(&f[2])?,
            uid: option(&f[3], |v| entity(v)?.require_type(ENTITY_USER))?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        if let Some(uid) = &self.uid {
            uid.clone().require_type(ENTITY_USER)?;
        }
        Ok(encode(&Value::Array(vec![
            self.id.value(),
            self.pkce_verifier.value(),
            self.nonce.value(),
            self.uid
                .as_ref()
                .map_or(Value::Null, |id| Value::Binary(id.as_bytes().to_vec())),
        ]))?)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PollOAuth2SessionArgument {
    pub id: OAuth2SessionId,
    pub wait_duration_ms: u64,
    pub for_login: bool,
}
impl PollOAuth2SessionArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 3)?;
        Ok(Self {
            id: OAuth2SessionId::parse(&f[0])?,
            wait_duration_ms: unsigned(&f[1])?,
            for_login: boolean(&f[2])?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            self.id.value(),
            Value::Unsigned(self.wait_duration_ms),
            Value::Bool(self.for_login),
        ]))?)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SsoLoginArgument {
    pub uid: EntityId,
    pub args: RegSsoArgs,
}
impl SsoLoginArgument {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 2)?;
        Ok(Self {
            uid: entity(&f[0])?.require_type(ENTITY_USER)?,
            args: RegSsoArgs::decode(&encode(&f[1])?)?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        self.uid.clone().require_type(ENTITY_USER)?;
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.uid.as_bytes().to_vec()),
            decode(&self.args.encoded()?)?,
        ]))?)
    }
}
fn string(s: &str) -> Value {
    Value::Text(s.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "../foks-snowpack/tests/fixtures/foks-v0.1.9/sso/{name}.snowp"
        ))
        .unwrap()
    }
    #[test]
    fn exact_go_structures_and_nullable_arguments() {
        macro_rules! check {
            ($ty:ty, $name:expr) => {
                let b = fixture($name);
                assert_eq!(<$ty>::decode(&b).unwrap().encoded().unwrap(), b);
            };
        }
        check!(OAuth2Binding, "binding");
        check!(OAuth2IdTokenBindingPayload, "binding-payload");
        check!(OAuth2IdTokenBinding, "signed-binding");
        check!(SsoConfig, "config");
        check!(SsoConfig, "public-config");
        check!(OAuth2TokenSet, "tokens");
        check!(OAuth2PollResult, "poll-result");
        check!(OAuth2PollResult, "poll-login-result");
        check!(RegSsoArgs, "reg-sso");
        check!(RegSsoArgs, "reg-none");
        check!(InitOAuth2SessionArgument, "init-login");
        check!(InitOAuth2SessionArgument, "init-signup");
        check!(PollOAuth2SessionArgument, "poll");
        check!(SsoLoginArgument, "login");
        let config = SsoConfig::decode(&fixture("config")).unwrap();
        assert_eq!(config.public().encoded().unwrap(), fixture("public-config"));
        assert!(!format!("{config:?}").contains("fixture-secret"));
        let login = SsoLoginArgument::decode(&fixture("login")).unwrap();
        assert!(!format!("{login:?}").contains("fixture.id.token"));
        let mut malformed = decode(&fixture("init-login")).unwrap();
        let Value::Array(ref mut f) = malformed else {
            unreachable!()
        };
        f[0] = Value::Binary(vec![0; 16]);
        assert!(InitOAuth2SessionArgument::decode(&encode(&malformed).unwrap()).is_err());
        assert!(RegSsoArgs::decode(
            &encode(&Value::Array(vec![
                Value::Unsigned(2),
                Value::Variant(None)
            ]))
            .unwrap()
        )
        .is_err());
    }
}
