//! Exact v0.1.9 federation identities, bearer permissions, and discovery values.

use zeroize::Zeroize as _;

use crate::{
    array, decode, encode, entity, fixed_blob, text, unsigned, EntityId, Error, Result, Role,
    SecretBox, Value, ENTITY_AD_HOC_TEAM, ENTITY_HOST, ENTITY_NAMED_TEAM, ENTITY_USER,
};

pub const TEAM_REMOTE_MEMBER_VIEW_TOKEN_BOX_PAYLOAD_TYPE_ID: u64 = 0xb869_945e_21b2_d379;
pub const REMOTE_VIEW_PERMISSION_PAYLOAD_TYPE_ID: u64 = 0xc83d_a756_0434_c870;
const TEAM_RSVP_REMOTE_TAG: u8 = 56;

/// A v0.1.9 remote-view bearer token.
///
/// Bearer token representing access authority. Omitted from debug output and
/// zeroized on drop.
#[derive(Clone, Eq, PartialEq)]
pub struct PermissionToken([u8; 17]);

impl PermissionToken {
    pub fn new(bytes: [u8; 17]) -> Self {
        Self(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Ok(Self(fixed_blob(&decode(bytes)?, "permission token")?))
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    pub fn expose(&self) -> &[u8; 17] {
        &self.0
    }

    pub fn to_value(&self) -> Value {
        Value::Binary(self.0.to_vec())
    }
}

impl std::fmt::Debug for PermissionToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PermissionToken([REDACTED])")
    }
}

impl Drop for PermissionToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// The remote-join RSVP carried beside an opaque member-view token.
///
/// The SQLite slice does not yet implement the upstream join-request inbox,
/// but it preserves this exact v0.1.9 field so stored tokens remain wire
/// compatible when that lifecycle is added.
#[derive(Clone, Eq, PartialEq)]
pub struct RemoteTeamRsvp([u8; 17]);

impl RemoteTeamRsvp {
    pub fn new(bytes: [u8; 17]) -> Result<Self> {
        if bytes[0] != TEAM_RSVP_REMOTE_TAG {
            return Err(Error::Type {
                expected: "remote team RSVP ID",
                found: "another ID16 type",
            });
        }
        Ok(Self(bytes))
    }

    pub fn expose(&self) -> &[u8; 17] {
        &self.0
    }

    pub fn to_value(&self) -> Value {
        Value::Binary(self.0.to_vec())
    }

    fn from_value(value: &Value) -> Result<Self> {
        Self::new(fixed_blob(value, "remote team RSVP")?)
    }
}

impl std::fmt::Debug for RemoteTeamRsvp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RemoteTeamRsvp([REDACTED])")
    }
}

impl Drop for RemoteTeamRsvp {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// A party whose identity is scoped to an authenticated host.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FqParty {
    pub party: EntityId,
    pub host: EntityId,
}

impl FqParty {
    pub fn new(party: EntityId, host: EntityId) -> Result<Self> {
        if !matches!(
            party.entity_type(),
            ENTITY_USER | ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM
        ) {
            return Err(Error::EntityType(party.entity_type()));
        }
        Ok(Self {
            party,
            host: host.require_type(ENTITY_HOST)?,
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_value(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.party.as_bytes().to_vec()),
            Value::Binary(self.host.as_bytes().to_vec()),
        ])
    }

    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 2)?;
        Self::new(entity(&fields[0])?, entity(&fields[1])?)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FqTeam {
    pub team: EntityId,
    pub host: EntityId,
}

impl FqTeam {
    pub fn new(team: EntityId, host: EntityId) -> Result<Self> {
        if !matches!(team.entity_type(), ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM) {
            return Err(Error::EntityType(team.entity_type()));
        }
        Ok(Self {
            team,
            host: host.require_type(ENTITY_HOST)?,
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_value(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.team.as_bytes().to_vec()),
            Value::Binary(self.host.as_bytes().to_vec()),
        ])
    }

    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 2)?;
        Self::new(entity(&fields[0])?, entity(&fields[1])?)
    }
}

/// Cleartext sealed by a target team's member-load-floor PTK. The party is
/// repeated inside the authenticated box so a server cannot swap opaque
/// boxes between remote roster entries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRemoteMemberViewTokenBoxPayload {
    pub token: PermissionToken,
    pub party: FqParty,
    pub time: u64,
}

impl TeamRemoteMemberViewTokenBoxPayload {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 3)?;
        Ok(Self {
            token: PermissionToken::decode(&encode(&fields[0])?)?,
            party: FqParty::from_value(&fields[1])?,
            time: unsigned(&fields[2])?,
        })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            self.token.to_value(),
            self.party.to_value(),
            Value::Unsigned(self.time),
        ]))?)
    }
}

/// The opaque portion returned by `TeamLoader.loadTeamRemoteViewTokens`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRemoteMemberViewTokenInner {
    pub member: FqParty,
    pub ptk_generation: u64,
    pub secret_box: SecretBox,
    pub ptk_role: Role,
}

impl TeamRemoteMemberViewTokenInner {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_value(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            self.member.to_value(),
            Value::Unsigned(self.ptk_generation),
            self.secret_box.to_value(),
            self.ptk_role.to_value(),
        ])
    }

    fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 4)?;
        let ptk_generation = unsigned(&fields[1])?;
        let ptk_role = Role::decode(&encode(&fields[3])?)?;
        if ptk_generation == 0 || ptk_role == Role::NONE {
            return Err(Error::IntegerRange("remote member-view PTK metadata"));
        }
        Ok(Self {
            member: FqParty::from_value(&fields[0])?,
            ptk_generation,
            secret_box: SecretBox::decode(&encode(&fields[2])?)?,
            ptk_role,
        })
    }
}

/// Exact off-chain token submitted with a team edit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRemoteMemberViewToken {
    pub team: EntityId,
    pub inner: TeamRemoteMemberViewTokenInner,
    pub join_request: RemoteTeamRsvp,
}

impl TeamRemoteMemberViewToken {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::from_value(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.team.as_bytes().to_vec()),
            self.inner.to_value(),
            self.join_request.to_value(),
        ])
    }

    fn from_value(value: &Value) -> Result<Self> {
        let fields = array(value, 3)?;
        let team = entity(&fields[0])?;
        if !matches!(team.entity_type(), ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM) {
            return Err(Error::EntityType(team.entity_type()));
        }
        Ok(Self {
            team,
            inner: TeamRemoteMemberViewTokenInner::from_value(&fields[1])?,
            join_request: RemoteTeamRsvp::from_value(&fields[2])?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRemoteViewTokenSet {
    pub tokens: Vec<TeamRemoteMemberViewTokenInner>,
}

impl TeamRemoteViewTokenSet {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 1)?;
        let tokens = match &fields[0] {
            Value::Null => Vec::new(),
            Value::Array(values) => values
                .iter()
                .map(TeamRemoteMemberViewTokenInner::from_value)
                .collect::<Result<Vec<_>>>()?,
            _ => {
                return Err(Error::Type {
                    expected: "remote member-view token list",
                    found: "another value",
                });
            }
        };
        if tokens.len() > 256 {
            return Err(Error::IntegerRange("remote member-view token count"));
        }
        Ok(Self { tokens })
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        if self.tokens.len() > 256 {
            return Err(Error::IntegerRange("remote member-view token count"));
        }
        let tokens = if self.tokens.is_empty() {
            Value::Null
        } else {
            Value::Array(self.tokens.iter().map(|token| token.to_value()).collect())
        };
        Ok(encode(&Value::Array(vec![tokens]))?)
    }
}

/// The signed v0.1.9 payload authorizing one remote party to view another.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteViewPermissionPayload {
    pub viewee: EntityId,
    pub viewer: FqParty,
    pub time: u64,
}

impl RemoteViewPermissionPayload {
    pub fn new(viewee: EntityId, viewer: FqParty, time: u64) -> Result<Self> {
        if !matches!(
            viewee.entity_type(),
            ENTITY_USER | ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM
        ) {
            return Err(Error::EntityType(viewee.entity_type()));
        }
        Ok(Self {
            viewee,
            viewer,
            time,
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode(bytes)?;
        let fields = array(&value, 3)?;
        Self::new(
            entity(&fields[0])?,
            FqParty::from_value(&fields[1])?,
            unsigned(&fields[2])?,
        )
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&self.to_value())?)
    }

    pub fn to_value(&self) -> Value {
        Value::Array(vec![
            Value::Binary(self.viewee.as_bytes().to_vec()),
            self.viewer.to_value(),
            Value::Unsigned(self.time),
        ])
    }
}

/// An untrusted Beacon result. Identity is established only by probing the
/// returned endpoint and checking its hostchain against `host_id`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BeaconHint {
    pub host_id: EntityId,
    pub address: String,
}

impl BeaconHint {
    pub fn new(host_id: EntityId, address: String) -> Result<Self> {
        let host_id = host_id.require_type(ENTITY_HOST)?;
        if address.is_empty()
            || !address.is_ascii()
            || address.len() > 512
            || address.bytes().any(|byte| byte == 0)
        {
            return Err(Error::Type {
                expected: "bounded ASCII TCP address",
                found: "invalid Beacon address",
            });
        }
        Ok(Self { host_id, address })
    }

    pub fn decode_address(host_id: EntityId, bytes: &[u8]) -> Result<Self> {
        Self::new(host_id, text(&decode(bytes)?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(kind: u8, fill: u8) -> EntityId {
        EntityId::from_bytes([vec![kind], vec![fill; 32]].concat()).unwrap()
    }

    #[test]
    fn exact_federation_values_round_trip() {
        let party = FqParty::new(entity(ENTITY_USER, 7), entity(ENTITY_HOST, 8)).unwrap();
        assert_eq!(FqParty::decode(&party.encoded().unwrap()).unwrap(), party);
        let payload = RemoteViewPermissionPayload::new(entity(ENTITY_USER, 9), party, 42).unwrap();
        assert_eq!(
            RemoteViewPermissionPayload::decode(&payload.encoded().unwrap()).unwrap(),
            payload
        );
        let token = TeamRemoteMemberViewToken {
            team: entity(ENTITY_NAMED_TEAM, 10),
            inner: TeamRemoteMemberViewTokenInner {
                member: FqParty::new(entity(ENTITY_USER, 11), entity(ENTITY_HOST, 12)).unwrap(),
                ptk_generation: 1,
                secret_box: SecretBox {
                    nonce: [13; 16],
                    ciphertext: vec![14; 64],
                },
                ptk_role: Role::member(0),
            },
            join_request: RemoteTeamRsvp::new(
                [vec![TEAM_RSVP_REMOTE_TAG], vec![15; 16]]
                    .concat()
                    .try_into()
                    .unwrap(),
            )
            .unwrap(),
        };
        assert_eq!(
            TeamRemoteMemberViewToken::decode(&token.encoded().unwrap()).unwrap(),
            token
        );
    }

    #[test]
    fn permission_tokens_are_redacted() {
        let token = PermissionToken::new([7; 17]);
        assert_eq!(format!("{token:?}"), "PermissionToken([REDACTED])");
        assert_eq!(
            PermissionToken::decode(&token.encoded().unwrap()).unwrap(),
            token
        );
    }

    #[test]
    fn host_scope_and_address_are_strict() {
        assert!(FqParty::new(entity(ENTITY_USER, 1), entity(ENTITY_USER, 2)).is_err());
        assert!(BeaconHint::new(entity(ENTITY_HOST, 1), "bad\0address".to_owned()).is_err());
    }
}
