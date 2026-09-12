//! Pinned invitation values. A certificate preview is not a membership proof.
use crate::{
    array, binary, decode, encode, entity, fixed_blob, list, option, text, unsigned, variant,
    EntityId, Error, FqTeam, Hepk, Result, Role, Signature, UserSharedKey, Value, ENTITY_HOST,
    ENTITY_PTK_VERIFY,
};
pub const TEAM_CERT_TYPE_ID: u64 = 0xbfde7f0ac7a3b707;
pub const TEAM_CERT_SIGNED_TYPE_ID: u64 = 0xd7e2d164a441663b;
pub const TEAM_CERT_PAYLOAD_TYPE_ID: u64 = 0xf88913d42ea72d2a;
pub const LOCAL_VIEW_PERMISSION_TYPE_ID: u64 = 0xf620e4a9845fa063;
pub const TEAM_REMOTE_JOIN_PAYLOAD_TYPE_ID: u64 = 0xae6970de2a147061;
pub const MAX_INVITATION_BYTES: usize = 16 * 1024;
fn bounded(bytes: &[u8]) -> Result<Value> {
    if bytes.len() > MAX_INVITATION_BYTES {
        return Err(Error::IntegerRange("invitation byte limit"));
    }
    Ok(decode(bytes)?)
}
fn version_one(value: Value) -> Value {
    Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((b"0".to_vec(), Box::new(value)))),
    ])
}
fn open_one(value: &Value) -> Result<&Value> {
    let f = array(value, 2)?;
    if unsigned(&f[0])? != 1 {
        return Err(Error::IntegerRange("invitation version"));
    }
    variant(&f[1], "0")
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamInvite {
    pub hash: [u8; 32],
    pub host: EntityId,
}
impl TeamInvite {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = bounded(bytes)?;
        let f = array(open_one(&v)?, 2)?;
        Ok(Self {
            hash: fixed_blob(&f[0], "certificate hash")?,
            host: entity(&f[1])?.require_type(ENTITY_HOST)?,
        })
    }
    pub fn to_value(&self) -> Value {
        version_one(Value::Array(vec![
            Value::Binary(self.hash.to_vec()),
            Value::Binary(self.host.as_bytes().to_vec()),
        ]))
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let b = encode(&self.to_value())?;
        Self::decode(&b)?;
        Ok(b)
    }
    pub fn export(&self) -> Result<String> {
        Ok(crate::encode_base62_strict(&self.encoded()?))
    }
    pub fn import(s: &str) -> Result<Self> {
        if s.len() > 256 {
            return Err(Error::IntegerRange("invite text limit"));
        }
        Self::decode(&crate::decode_base62_strict(s)?)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamCertificate {
    /// Exact future payload, including absence of the older optional name field.
    pub payload: Vec<u8>,
    pub signatures: Vec<Signature>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamCertificatePayload {
    pub team: FqTeam,
    pub key: UserSharedKey,
    pub time: u64,
    pub hepk: Hepk,
    pub name: Option<Vec<u8>>,
}
impl TeamCertificatePayload {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = bounded(bytes)?;
        let Value::Array(f) = v else {
            return Err(Error::IntegerRange("certificate payload"));
        };
        if !(4..=5).contains(&f.len()) {
            return Err(Error::FieldCount {
                expected: 5,
                found: f.len(),
            });
        }
        let k = array(&f[1], 4)?;
        let key = UserSharedKey {
            generation: unsigned(&k[0])?,
            role: Role::decode(&encode(&k[1])?)?,
            verify_key: entity(&k[2])?.require_type(ENTITY_PTK_VERIFY)?,
            hepk_fingerprint: fixed_blob(&k[3], "HEPK fingerprint")?,
        };
        if key.generation == 0 || key.role != Role::ADMIN {
            return Err(Error::IntegerRange("certificate admin generation"));
        }
        let name = f.get(4).map(text).transpose()?.map(|s| s.into_bytes());
        if name.as_ref().is_some_and(|n| n.len() > 1024) {
            return Err(Error::IntegerRange("certificate name limit"));
        }
        Ok(Self {
            team: FqTeam::decode(&encode(&f[0])?)?,
            key,
            time: unsigned(&f[2])?,
            hepk: Hepk::decode(&encode(&f[3])?)?,
            name,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let k = &self.key;
        let mut f = vec![
            self.team.to_value(),
            Value::Array(vec![
                Value::Unsigned(k.generation),
                k.role.to_value(),
                Value::Binary(k.verify_key.as_bytes().to_vec()),
                Value::Binary(k.hepk_fingerprint.to_vec()),
            ]),
            Value::Unsigned(self.time),
            decode(&self.hepk.encoded()?)?,
        ];
        if let Some(n) = &self.name {
            f.push(Value::Text(n.clone()));
        }
        let b = encode(&Value::Array(f))?;
        Self::decode(&b)?;
        Ok(b)
    }
}
impl TeamCertificate {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = bounded(bytes)?;
        let f = array(open_one(&v)?, 2)?;
        let ret = Self {
            payload: binary(&f[0])?.to_vec(),
            signatures: list(&f[1], |v| Signature::decode(&encode(v)?))?,
        };
        if !(1..=2).contains(&ret.signatures.len()) {
            return Err(Error::IntegerRange("certificate signature count"));
        }
        TeamCertificatePayload::decode(&ret.payload)?;
        Ok(ret)
    }
    pub fn signing_bytes(&self, count: usize) -> Result<Vec<u8>> {
        if count > self.signatures.len() {
            return Err(Error::IntegerRange("certificate signature prefix"));
        }
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.payload.clone()),
            if count == 0 {
                Value::Null
            } else {
                Value::Array(
                    self.signatures[..count]
                        .iter()
                        .map(Signature::to_value)
                        .collect(),
                )
            },
        ]))?)
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let b = encode(&version_one(decode(
            &self.signing_bytes(self.signatures.len())?,
        )?))?;
        Self::decode(&b)?;
        Ok(b)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalViewPermissionPayload {
    pub viewee: EntityId,
    pub viewer: EntityId,
    pub time: u64,
    pub viewer_role: Option<Role>,
}
impl LocalViewPermissionPayload {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = bounded(bytes)?;
        let Value::Array(f) = v else {
            return Err(Error::IntegerRange("local permission fields"));
        };
        if !(3..=4).contains(&f.len()) {
            return Err(Error::FieldCount {
                expected: 4,
                found: f.len(),
            });
        }
        let viewee = entity(&f[0])?;
        let viewer = entity(&f[1])?;
        for party in [&viewee, &viewer] {
            if !matches!(party.entity_type(), 1 | 3 | 20) {
                return Err(Error::EntityType(party.entity_type()));
            }
        }
        Ok(Self {
            viewee,
            viewer,
            time: unsigned(&f[2])?,
            viewer_role: f
                .get(3)
                .map(|v| option(v, |r| Role::decode(&encode(r)?)))
                .transpose()?
                .flatten(),
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let b = encode(&Value::Array(vec![
            Value::Binary(self.viewee.as_bytes().to_vec()),
            Value::Binary(self.viewer.as_bytes().to_vec()),
            Value::Unsigned(self.time),
            self.viewer_role.map_or(Value::Null, Role::to_value),
        ]))?;
        Self::decode(&b)?;
        Ok(b)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamCertificateAndMetadata {
    pub certificate: TeamCertificate,
    pub index_range: crate::RationalRange,
}
impl TeamCertificateAndMetadata {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = bounded(bytes)?;
        let f = array(&v, 2)?;
        Ok(Self {
            certificate: TeamCertificate::decode(&encode(&f[0])?)?,
            index_range: crate::RationalRange::decode(&encode(&f[1])?)?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            decode(&self.certificate.encoded()?)?,
            decode(&self.index_range.encoded()?)?,
        ]))?)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteJoinVisible {
    pub index_range: Option<crate::RationalRange>,
}
impl RemoteJoinVisible {
    pub fn from_value(v: &Value) -> Result<Self> {
        let f = array(v, 1)?;
        Ok(Self {
            index_range: option(&f[0], |v| crate::RationalRange::decode(&encode(v)?))?,
        })
    }
    pub fn to_value(&self) -> Result<Value> {
        Ok(Value::Array(vec![self
            .index_range
            .as_ref()
            .map(|r| -> Result<Value> { Ok(decode(&r.encoded()?)?) })
            .transpose()?
            .unwrap_or(Value::Null)]))
    }
}
#[derive(Clone, Eq, PartialEq)]
pub struct RemoteJoinRequest {
    pub hepk_fingerprint: [u8; 32],
    pub encrypted: crate::HybridBox,
    pub visible: RemoteJoinVisible,
}
impl std::fmt::Debug for RemoteJoinRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RemoteJoinRequest([REDACTED])")
    }
}
impl RemoteJoinRequest {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = bounded(bytes)?;
        let f = array(&v, 3)?;
        let ret = Self {
            hepk_fingerprint: fixed_blob(&f[0], "admin HEPK fingerprint")?,
            encrypted: crate::HybridBox::decode(&encode(&f[1])?)?,
            visible: RemoteJoinVisible::from_value(&f[2])?,
        };
        if ret.encrypted.sender_dh.is_none() {
            return Err(Error::IntegerRange("remote request sender key"));
        }
        Ok(ret)
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        let b = encode(&Value::Array(vec![
            Value::Binary(self.hepk_fingerprint.to_vec()),
            decode(&self.encrypted.encoded()?)?,
            self.visible.to_value()?,
        ]))?;
        Self::decode(&b)?;
        Ok(b)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteJoinPayload {
    pub joiner: crate::FqParty,
    pub permission: crate::PermissionToken,
    pub time: u64,
    pub source_role: Role,
    pub visible: RemoteJoinVisible,
}
impl RemoteJoinPayload {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = bounded(bytes)?;
        let f = array(&v, 5)?;
        Ok(Self {
            joiner: crate::FqParty::decode(&encode(&f[0])?)?,
            permission: crate::PermissionToken::decode(&encode(&f[1])?)?,
            time: unsigned(&f[2])?,
            source_role: Role::decode(&encode(&f[3])?)?,
            visible: RemoteJoinVisible::from_value(&f[4])?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            self.joiner.to_value(),
            self.permission.to_value(),
            Value::Unsigned(self.time),
            self.source_role.to_value(),
            self.visible.to_value()?,
        ]))?)
    }
}
/// Stable opaque receipt; its kind is distinct from operation identity.
#[derive(Clone, Eq, PartialEq)]
pub struct TeamRsvp([u8; 17]);
impl TeamRsvp {
    pub fn new(bytes: [u8; 17]) -> Result<Self> {
        if !matches!(bytes[0], 56 | 57) {
            return Err(Error::IntegerRange("team RSVP kind"));
        }
        Ok(Self(bytes))
    }
    pub fn is_remote(&self) -> bool {
        self.0[0] == 56
    }
    pub fn expose(&self) -> &[u8; 17] {
        &self.0
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::new(fixed_blob(&bounded(bytes)?, "team RSVP")?)
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Binary(self.0.to_vec()))?)
    }
}
impl std::fmt::Debug for TeamRsvp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TeamRsvp([REDACTED])")
    }
}
impl Drop for TeamRsvp {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        self.0.zeroize();
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinRequestState {
    Pending = 0,
    Approved = 1,
    Rejected = 2,
    Withdrawn = 3,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InboxPagination {
    pub start: u64,
    pub end: u64,
    pub limit: u64,
}
impl InboxPagination {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v = bounded(bytes)?;
        let f = array(&v, 3)?;
        Ok(Self {
            start: unsigned(&f[0])?,
            end: unsigned(&f[1])?,
            limit: unsigned(&f[2])?,
        })
    }
    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(encode(&Value::Array(vec![
            Value::Unsigned(self.start),
            Value::Unsigned(self.end),
            Value::Unsigned(self.limit),
        ]))?)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RawInboxRequest {
    Local {
        joiner: EntityId,
        source_role: Role,
        permission: crate::PermissionToken,
    },
    Remote(RemoteJoinRequest),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawInboxRow {
    pub time: u64,
    pub state: JoinRequestState,
    pub receipt: TeamRsvp,
    pub request: RawInboxRequest,
}
impl RawInboxRow {
    fn from_value(v: &Value) -> Result<Self> {
        let f = array(v, 3)?;
        let state = match unsigned(&f[1])? {
            0 => JoinRequestState::Pending,
            1 => JoinRequestState::Approved,
            2 => JoinRequestState::Rejected,
            3 => JoinRequestState::Withdrawn,
            _ => return Err(Error::IntegerRange("join request state")),
        };
        let row = array(&f[2], 2)?;
        let tag = unsigned(&row[0])?;
        let (receipt, request) = match tag {
            1 => {
                let x = array(variant(&row[1], "1")?, 4)?;
                let r = TeamRsvp::decode(&encode(&x[0])?)?;
                if r.is_remote() {
                    return Err(Error::IntegerRange("local RSVP"));
                }
                let party = entity(&x[1])?;
                if !matches!(party.entity_type(), 1 | 3 | 20) {
                    return Err(Error::EntityType(party.entity_type()));
                }
                (
                    r,
                    RawInboxRequest::Local {
                        joiner: party,
                        source_role: Role::decode(&encode(&x[2])?)?,
                        permission: crate::PermissionToken::decode(&encode(&x[3])?)?,
                    },
                )
            }
            2 => {
                let x = array(variant(&row[1], "2")?, 2)?;
                let r = TeamRsvp::decode(&encode(&x[0])?)?;
                if !r.is_remote() {
                    return Err(Error::IntegerRange("remote RSVP"));
                }
                (
                    r,
                    RawInboxRequest::Remote(RemoteJoinRequest::decode(&encode(&x[1])?)?),
                )
            }
            _ => return Err(Error::IntegerRange("inbox row kind")),
        };
        Ok(Self {
            time: unsigned(&f[0])?,
            state,
            receipt,
            request,
        })
    }
    fn to_value(&self) -> Result<Value> {
        let receipt = decode(&self.receipt.encoded()?)?;
        let (tag, body) = match &self.request {
            RawInboxRequest::Local {
                joiner,
                source_role,
                permission,
            } => (
                1,
                Value::Array(vec![
                    receipt,
                    Value::Binary(joiner.as_bytes().to_vec()),
                    source_role.to_value(),
                    permission.to_value(),
                ]),
            ),
            RawInboxRequest::Remote(r) => (2, Value::Array(vec![receipt, decode(&r.encoded()?)?])),
        };
        Ok(Value::Array(vec![
            Value::Unsigned(self.time),
            Value::Unsigned(self.state as u64),
            Value::Array(vec![
                Value::Unsigned(tag),
                Value::Variant(Some((tag.to_string().into_bytes(), Box::new(body)))),
            ]),
        ]))
    }
}
pub fn decode_team_inbox(bytes: &[u8]) -> Result<Vec<RawInboxRow>> {
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(Error::IntegerRange("inbox byte limit"));
    }
    let v = decode(bytes)?;
    let f = array(&v, 1)?;
    if matches!(&f[0],Value::Array(rows) if rows.len()>1000) {
        return Err(Error::IntegerRange("inbox row limit"));
    }
    list(&f[0], RawInboxRow::from_value)
}
pub fn encode_team_inbox(rows: &[RawInboxRow]) -> Result<Vec<u8>> {
    let b = encode(&Value::Array(vec![if rows.is_empty() {
        Value::Null
    } else {
        Value::Array(
            rows.iter()
                .map(RawInboxRow::to_value)
                .collect::<Result<_>>()?,
        )
    }]))?;
    decode_team_inbox(&b)?;
    Ok(b)
}
