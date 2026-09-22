//! FOKS Rust identity extension. Public status requires an active owner-device proof.
use crate::{
    array, binary, decode, encode, entity, fixed_blob, text, unsigned, EntityId, Error, Result,
    Signature, Value, ENTITY_HOST, ENTITY_USER,
};

pub const IDENTITY_PROOF_TYPE_ID: u64 = 0xf04b_a81e_c057_0001;
pub const IDENTITY_EXTENSION_VERSION: u64 = 1;
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[repr(u8)]
pub enum SsoPurpose {
    Signup = 0,
    LinkExisting = 1,
    Reauthenticate = 2,
}
impl SsoPurpose {
    pub fn is_existing(self) -> bool {
        self != Self::Signup
    }
    pub fn from_code(code: u8) -> Result<Self> {
        match code {
            0 => Ok(Self::Signup),
            1 => Ok(Self::LinkExisting),
            2 => Ok(Self::Reauthenticate),
            _ => Err(Error::Invalid("SSO purpose")),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[repr(u8)]
pub enum SsoAccountState {
    DeviceOnly = 0,
    MigrationEligible = 1,
    Linked = 2,
    LockedOut = 3,
    NotEligible = 4,
}
impl SsoAccountState {
    fn parse(v: &Value) -> Result<Self> {
        match unsigned(v)? {
            0 => Ok(Self::DeviceOnly),
            1 => Ok(Self::MigrationEligible),
            2 => Ok(Self::Linked),
            3 => Ok(Self::LockedOut),
            4 => Ok(Self::NotEligible),
            _ => Err(Error::Invalid("identity account state")),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityClaim {
    pub host: EntityId,
    pub uid: EntityId,
    pub signer: EntityId,
    /// None requests account status; Some requests an exact committed authorization binding.
    pub authorization_binding_commitment: Option<[u8; 32]>,
}
impl IdentityClaim {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        encode(&Value::Array(vec![
            Value::Unsigned(1),
            Value::Binary(self.host.as_bytes().to_vec()),
            Value::Binary(self.uid.as_bytes().to_vec()),
            Value::Binary(self.signer.as_bytes().to_vec()),
            Value::Unsigned(u64::from(self.authorization_binding_commitment.is_some())),
            Value::Binary(
                self.authorization_binding_commitment
                    .map_or_else(Vec::new, |v| v.to_vec()),
            ),
        ]))
        .map_err(Into::into)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 6)?;
        if unsigned(&f[0])? != 1 {
            return Err(Error::Invalid("identity extension version"));
        }
        let authorization_binding_commitment = match unsigned(&f[4])? {
            0 if binary(&f[5])?.is_empty() => None,
            1 => Some(fixed_blob(&f[5], "authorization binding commitment")?),
            _ => return Err(Error::Invalid("identity action")),
        };
        Ok(Self {
            host: entity(&f[1])?.require_type(ENTITY_HOST)?,
            uid: entity(&f[2])?.require_type(ENTITY_USER)?,
            signer: entity(&f[3])?,
            authorization_binding_commitment,
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityChallenge {
    pub claim: IdentityClaim,
    pub challenge: [u8; 32],
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}
impl IdentityChallenge {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        encode(&Value::Array(vec![
            Value::Binary(self.claim.encoded()?),
            Value::Binary(self.challenge.to_vec()),
            Value::Unsigned(self.expires_at_ms),
            Value::Unsigned(self.issued_at_ms),
        ]))
        .map_err(Into::into)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 4)?;
        Ok(Self {
            claim: IdentityClaim::decode(binary(&f[0])?)?,
            challenge: fixed_blob(&f[1], "identity challenge")?,
            expires_at_ms: unsigned(&f[2])?,
            issued_at_ms: unsigned(&f[3])?,
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityProof {
    pub challenge: IdentityChallenge,
    pub signature: Signature,
}
impl IdentityProof {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        encode(&Value::Array(vec![
            Value::Binary(self.challenge.encoded()?),
            self.signature.to_value(),
        ]))
        .map_err(Into::into)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 2)?;
        Ok(Self {
            challenge: IdentityChallenge::decode(binary(&f[0])?)?,
            signature: crate::host::signature(&f[1])?,
        })
    }
}
/// Status contains no provider subject, email, tokens, or protected ciphertext.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityStatus {
    pub challenge: [u8; 32],
    pub account_state: SsoAccountState,
    pub rollout_id: Option<[u8; 16]>,
    /// 0 = no policy, 1 = migration, 2 = enforced.
    pub rollout_mode: u8,
    /// 0 = enabled, 1..=4 = durable provider block reason.
    pub provider_blocked_reason: u8,
    pub issuer: String,
    pub authorization_epoch: u64,
    pub authorization_generation: u64,
    pub access_available: bool,
    /// A retained exact commitment proves success; absence means evidence unavailable.
    pub committed_authorization_binding: Option<[u8; 32]>,
}
impl IdentityStatus {
    pub fn encoded(&self) -> Result<Vec<u8>> {
        encode(&Value::Array(vec![
            Value::Unsigned(1),
            Value::Binary(self.challenge.to_vec()),
            Value::Unsigned(self.account_state as u64),
            Value::Binary(self.rollout_id.map_or_else(Vec::new, |v| v.to_vec())),
            Value::Unsigned(self.rollout_mode as u64),
            Value::Unsigned(self.provider_blocked_reason as u64),
            Value::Text(self.issuer.as_bytes().to_vec()),
            Value::Unsigned(self.authorization_epoch),
            Value::Unsigned(self.authorization_generation),
            Value::Bool(self.access_available),
            Value::Binary(
                self.committed_authorization_binding
                    .map_or_else(Vec::new, |v| v.to_vec()),
            ),
        ]))
        .map_err(Into::into)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let f = array(&v, 11)?;
        if unsigned(&f[0])? != 1
            || unsigned(&f[4])? > 2
            || unsigned(&f[5])? > 4
            || text(&f[6])?.len() > 4096
        {
            return Err(Error::Invalid("identity status"));
        }
        Ok(Self {
            challenge: fixed_blob(&f[1], "identity challenge")?,
            account_state: SsoAccountState::parse(&f[2])?,
            rollout_id: if binary(&f[3])?.is_empty() {
                None
            } else {
                Some(fixed_blob(&f[3], "rollout ID")?)
            },
            rollout_mode: unsigned(&f[4])? as u8,
            provider_blocked_reason: unsigned(&f[5])? as u8,
            issuer: text(&f[6])?,
            authorization_epoch: unsigned(&f[7])?,
            authorization_generation: unsigned(&f[8])?,
            access_available: crate::boolean(&f[9])?,
            committed_authorization_binding: if binary(&f[10])?.is_empty() {
                None
            } else {
                Some(fixed_blob(&f[10], "authorization binding commitment")?)
            },
        })
    }
}

/// Safe local UI projection, independent of the proof nonce and authorization binding commitment.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SsoAccountStatusView {
    pub state: SsoAccountState,
    pub rollout_mode: u8,
    pub provider_blocked_reason: u8,
    pub issuer: String,
    pub authorization_epoch: u64,
    pub authorization_generation: u64,
}
impl From<&IdentityStatus> for SsoAccountStatusView {
    fn from(s: &IdentityStatus) -> Self {
        Self {
            state: s.account_state,
            rollout_mode: s.rollout_mode,
            provider_blocked_reason: s.provider_blocked_reason,
            issuer: s.issuer.clone(),
            authorization_epoch: s.authorization_epoch,
            authorization_generation: s.authorization_generation,
        }
    }
}
