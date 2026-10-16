//! Private read-continuation capability, not a public FOKS protocol token.
//!
//! Verification supplies provenance only. The caller must separately prove the
//! current SQL credential, parent-key access, fence/revision and deadline before
//! reading each page and before publishing any response.
use crate::{
    authority::{Fence, Number},
    personal::PeerCertificate,
};
use foks_proto::{Role, RoleAndGeneration};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

pub const TOKEN_BYTES: usize = 180;
const PAYLOAD_BYTES: usize = 148;
const MAC_DOMAIN: &[u8] = b"foks-metadata-list-token-v1\0";
const BINDING_DOMAIN: &str = "foks-metadata-list-binding-v2";
const MAX_BINDING_BYTES: usize = 32768;
type HmacSha256 = Hmac<sha2_hmac::Sha256>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidBinding,
    InvalidToken,
    BundleLost,
    Expired,
    Bounds,
    Entropy,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InitialCursor {
    None {},
    Mac { value: [u8; 32] },
    Time { value: Number },
}

/// A validated coordinate, not proof that a caller read it from SQL. Begin/Next
/// must obtain and compare it inside their own authoritative transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ReadDependency {
    incarnation: [u8; 32],
    revision: Number,
}
impl ReadDependency {
    pub fn new(incarnation: [u8; 32], revision: u64) -> Result<Self> {
        if incarnation == [0; 32] || revision == 0 || revision > i64::MAX as u64 {
            return Err(Error::InvalidBinding);
        }
        Ok(Self {
            incarnation,
            revision: Number(revision as i64),
        })
    }
    pub fn incarnation(&self) -> &[u8; 32] {
        &self.incarnation
    }
    pub fn revision(&self) -> u64 {
        self.revision.0 as u64
    }
}

/// Private fields prohibit unchecked construction/deserialization. Canonical
/// bytes are a fixed typed struct, never an arbitrary frontend JSON object.
#[derive(Clone, Debug, Serialize)]
pub struct ListBinding {
    domain: &'static str,
    fence: Fence,
    revision: Number,
    dependency: ReadDependency,
    peer: PeerCertificate,
    namespace: Vec<u8>,
    parent: [u8; 16],
    route: &'static str,
    cursor: InitialCursor,
    count: Number,
    load_small: bool,
    deadline: Number,
    clock_domain: [u8; 32],
}
impl ListBinding {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fence: Fence,
        revision: Number,
        dependency: ReadDependency,
        peer: PeerCertificate,
        namespace: [u8; 33],
        parent: [u8; 16],
        cursor: InitialCursor,
        requested_count: u64,
        load_small: bool,
        deadline: Number,
        clock_domain: [u8; 32],
    ) -> Result<Self> {
        if fence.identity.host.len() != 33
            || fence.epoch.0 < 1
            || revision.0 < 1
            || deadline.0 < 0
            || fence.activation.version.0 < 1
            || fence.activation.schema.0 < 1
            || fence.activation.key_generation.0 < 1
            || matches!(cursor,InitialCursor::Time{value:Number(n)} if n<0)
        {
            return Err(Error::InvalidBinding);
        }
        let count = if requested_count == 0 || requested_count > 4096 {
            4096
        } else {
            requested_count
        };
        let binding = Self {
            domain: BINDING_DOMAIN,
            fence,
            revision,
            dependency,
            peer,
            namespace: namespace.to_vec(),
            parent,
            route: "personal_list",
            cursor,
            count: Number(count as i64),
            load_small,
            deadline,
            clock_domain,
        };
        binding.canonical_bytes()?;
        Ok(binding)
    }
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        // All variable fields were bounded before this serialization. This bound
        // also pins future changes rather than accepting an expanding envelope.
        foks_snowpack::reserve_allocation(MAX_BINDING_BYTES).map_err(|_| Error::Bounds)?;
        let bytes = serde_json::to_vec(self).map_err(|_| Error::Bounds)?;
        if bytes.len() > MAX_BINDING_BYTES {
            return Err(Error::Bounds);
        }
        Ok(bytes)
    }
    pub fn digest(&self) -> Result<[u8; 32]> {
        Ok(Sha256::digest(self.canonical_bytes()?).into())
    }
    pub fn count(&self) -> u16 {
        self.count.0 as u16
    }
    pub fn deadline(&self) -> Number {
        self.deadline
    }
    fn check_time(&self, now: Number) -> Result<()> {
        if now.0 < 0 {
            return Err(Error::InvalidBinding);
        }
        if now.0 >= self.deadline.0 {
            return Err(Error::Expired);
        }
        Ok(())
    }
}

/// No Clone, Debug or serde implementation: secret key ownership is singular.
/// Host setup copies are consumed and wiped; public request schemas cannot set it.
pub struct ReadTokenAuthority {
    key: Zeroizing<[u8; 32]>,
    nonce: [u8; 16],
    active: bool,
}
impl ReadTokenAuthority {
    #[cfg(feature = "native")]
    pub fn generate() -> Result<Self> {
        let mut entropy = Zeroizing::new([0u8; 48]);
        getrandom::fill(entropy.as_mut()).map_err(|_| Error::Entropy)?;
        Self::from_host_entropy(entropy)
    }
    #[cfg(any(
        test,
        target_arch = "wasm32",
        feature = "native",
        feature = "read-token-qualification"
    ))]
    pub(crate) fn from_host_entropy(mut entropy: Zeroizing<[u8; 48]>) -> Result<Self> {
        if entropy[..32].iter().all(|b| *b == 0) || entropy[32..].iter().all(|b| *b == 0) {
            return Err(Error::Entropy);
        }
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&entropy[..32]);
        let mut nonce = [0; 16];
        nonce.copy_from_slice(&entropy[32..]);
        entropy.zeroize();
        Ok(Self {
            key,
            nonce,
            active: true,
        })
    }
    #[cfg(feature = "native")]
    pub fn rotate(&mut self) -> Result<()> {
        let old_nonce = self.nonce;
        self.invalidate();
        let next = Self::generate()?;
        if next.nonce == old_nonce {
            return Err(Error::Entropy);
        }
        *self = next;
        Ok(())
    }
    pub fn invalidate(&mut self) {
        self.key.zeroize();
        self.nonce.zeroize();
        self.active = false;
    }
    pub fn verify(
        &self,
        bytes: &[u8],
        binding: &ListBinding,
        now: Number,
    ) -> Result<VerifiedToken> {
        if !self.active {
            return Err(Error::BundleLost);
        }
        if bytes.len() != TOKEN_BYTES {
            return Err(Error::InvalidToken);
        }
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref()).map_err(|_| Error::Entropy)?;
        mac.update(MAC_DOMAIN);
        mac.update(&bytes[..PAYLOAD_BYTES]);
        mac.verify_slice(&bytes[PAYLOAD_BYTES..])
            .map_err(|_| Error::BundleLost)?;
        // No derived role/cursor/count is consumed before the constant-time MAC.
        if &bytes[..4] != b"FKLT" || bytes[4] != 1 || bytes[5..8] != [0; 3] {
            return Err(Error::InvalidToken);
        }
        let binding_digest = binding.digest()?;
        if bytes[8..24] != self.nonce || bytes[40..72] != binding_digest {
            return Err(Error::BundleLost);
        }
        binding.check_time(now)?;
        let role = u64::from_le_bytes(bytes[72..80].try_into().map_err(|_| Error::InvalidToken)?);
        let visibility =
            i32::from_le_bytes(bytes[80..84].try_into().map_err(|_| Error::InvalidToken)?);
        let role = match (role, visibility) {
            (1, v) => Role::member(i16::try_from(v).map_err(|_| Error::InvalidToken)?),
            (2, 0) => Role::ADMIN,
            (3, 0) => Role::OWNER,
            _ => return Err(Error::InvalidToken),
        };
        let generation =
            u64::from_le_bytes(bytes[84..92].try_into().map_err(|_| Error::InvalidToken)?);
        if generation == 0 || generation > i64::MAX as u64 {
            return Err(Error::InvalidToken);
        }
        let remaining =
            u16::from_le_bytes(bytes[92..94].try_into().map_err(|_| Error::InvalidToken)?);
        let returned =
            u16::from_le_bytes(bytes[94..96].try_into().map_err(|_| Error::InvalidToken)?);
        let ordinal =
            u16::from_le_bytes(bytes[96..98].try_into().map_err(|_| Error::InvalidToken)?);
        if bytes[99] & !3 != 0 {
            return Err(Error::InvalidToken);
        }
        let complete = bytes[99] & 1 != 0;
        let final_page = bytes[99] & 2 != 0;
        let id = bytes[132..148]
            .try_into()
            .map_err(|_| Error::InvalidToken)?;
        let cursor = match bytes[98] {
            0 if bytes[100..148].iter().all(|b| *b == 0) => Position::None,
            1 => Position::Mac(
                bytes[100..132]
                    .try_into()
                    .map_err(|_| Error::InvalidToken)?,
                id,
            ),
            2 if bytes[108..132].iter().all(|b| *b == 0) => {
                let time = u64::from_le_bytes(
                    bytes[100..108]
                        .try_into()
                        .map_err(|_| Error::InvalidToken)?,
                );
                if time > i64::MAX as u64 {
                    return Err(Error::InvalidToken);
                }
                Position::Time(time, id)
            }
            _ => return Err(Error::InvalidToken),
        };
        let verified = VerifiedToken {
            raw: bytes.try_into().map_err(|_| Error::InvalidToken)?,
            bundle: bytes[24..40].try_into().map_err(|_| Error::InvalidToken)?,
            binding: binding_digest,
            key: RoleAndGeneration { role, generation },
            remaining,
            returned,
            ordinal,
            complete,
            final_page,
            cursor,
        };
        verified.validate(binding)?;
        Ok(verified)
    }
    #[cfg(any(test, feature = "read-token-qualification"))]
    pub(crate) fn issue(
        &self,
        binding: &ListBinding,
        key: RoleAndGeneration,
        bundle: [u8; 16],
        now: Number,
    ) -> Result<Token> {
        binding.check_time(now)?;
        let state = VerifiedToken {
            raw: [0; TOKEN_BYTES],
            bundle,
            binding: binding.digest()?,
            key,
            remaining: binding.count(),
            returned: 0,
            ordinal: 0,
            complete: false,
            final_page: false,
            cursor: Position::None,
        };
        self.sign(&state, binding)
    }
    #[cfg(any(test, feature = "read-token-qualification"))]
    pub(crate) fn advance(
        &self,
        previous: &VerifiedToken,
        binding: &ListBinding,
        consumed: u16,
        requested_limit: u16,
        last: Position,
        now: Number,
    ) -> Result<Token> {
        let previous = self.verify(&previous.raw, binding, now)?;
        if previous.complete
            || requested_limit == 0
            || requested_limit > previous.remaining
            || consumed > requested_limit
        {
            return Err(Error::InvalidToken);
        }
        if consumed == 0 {
            if last != previous.cursor {
                return Err(Error::InvalidToken);
            }
        } else if !last.forward_from(&previous.cursor, &binding.cursor) {
            return Err(Error::InvalidToken);
        }
        let remaining = previous
            .remaining
            .checked_sub(consumed)
            .ok_or(Error::InvalidToken)?;
        let state = VerifiedToken {
            raw: [0; TOKEN_BYTES],
            bundle: previous.bundle,
            binding: previous.binding,
            key: previous.key,
            remaining,
            returned: previous
                .returned
                .checked_add(consumed)
                .ok_or(Error::InvalidToken)?,
            ordinal: previous.ordinal.checked_add(1).ok_or(Error::InvalidToken)?,
            complete: remaining == 0 || consumed < requested_limit,
            final_page: consumed < requested_limit,
            cursor: last,
        };
        self.sign(&state, binding)
    }
    #[cfg(any(test, feature = "read-token-qualification"))]
    fn sign(&self, state: &VerifiedToken, binding: &ListBinding) -> Result<Token> {
        if !self.active {
            return Err(Error::BundleLost);
        }
        state.validate(binding)?;
        let mut bytes = [0; TOKEN_BYTES];
        bytes[..4].copy_from_slice(b"FKLT");
        bytes[4] = 1;
        bytes[8..24].copy_from_slice(&self.nonce);
        bytes[24..40].copy_from_slice(&state.bundle);
        bytes[40..72].copy_from_slice(&state.binding);
        bytes[72..80].copy_from_slice(&state.key.role.protocol_value().to_le_bytes());
        bytes[80..84]
            .copy_from_slice(&i32::from(state.key.role.visibility().unwrap_or(0)).to_le_bytes());
        bytes[84..92].copy_from_slice(&state.key.generation.to_le_bytes());
        bytes[92..94].copy_from_slice(&state.remaining.to_le_bytes());
        bytes[94..96].copy_from_slice(&state.returned.to_le_bytes());
        bytes[96..98].copy_from_slice(&state.ordinal.to_le_bytes());
        bytes[99] = u8::from(state.complete) | (u8::from(state.final_page) << 1);
        match state.cursor {
            Position::None => {}
            Position::Mac(mac, id) => {
                bytes[98] = 1;
                bytes[100..132].copy_from_slice(&mac);
                bytes[132..148].copy_from_slice(&id);
            }
            Position::Time(time, id) => {
                bytes[98] = 2;
                bytes[100..108].copy_from_slice(&time.to_le_bytes());
                bytes[132..148].copy_from_slice(&id);
            }
        }
        let mut mac = HmacSha256::new_from_slice(self.key.as_ref()).map_err(|_| Error::Entropy)?;
        mac.update(MAC_DOMAIN);
        mac.update(&bytes[..PAYLOAD_BYTES]);
        bytes[PAYLOAD_BYTES..].copy_from_slice(&mac.finalize().into_bytes());
        Ok(Token(bytes))
    }
}

pub struct Token([u8; TOKEN_BYTES]);
impl Token {
    pub fn as_bytes(&self) -> &[u8; TOKEN_BYTES] {
        &self.0
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Position {
    None,
    Mac([u8; 32], [u8; 16]),
    Time(u64, [u8; 16]),
}
impl Position {
    fn forward_from(&self, previous: &Self, initial: &InitialCursor) -> bool {
        match (self, previous, initial) {
            (
                Self::Mac(m, id),
                Self::Mac(old, old_id),
                InitialCursor::None {} | InitialCursor::Mac { .. },
            ) => (m, id) > (old, old_id),
            (Self::Time(t, id), Self::Time(old, old_id), InitialCursor::Time { .. }) => {
                (t, id) > (old, old_id)
            }
            (Self::Mac(..), Self::None, InitialCursor::None {}) => true,
            (Self::Mac(m, _), Self::None, InitialCursor::Mac { value }) => m > value,
            (Self::Time(t, _), Self::None, InitialCursor::Time { value }) => *t >= value.0 as u64,
            _ => false,
        }
    }
}
#[derive(Clone)]
pub struct VerifiedToken {
    raw: [u8; TOKEN_BYTES],
    bundle: [u8; 16],
    binding: [u8; 32],
    key: RoleAndGeneration,
    remaining: u16,
    returned: u16,
    ordinal: u16,
    complete: bool,
    final_page: bool,
    cursor: Position,
}
impl VerifiedToken {
    pub fn encoded(&self) -> &[u8; TOKEN_BYTES] {
        &self.raw
    }
    pub fn binding_digest(&self) -> [u8; 32] {
        self.binding
    }
    pub fn check_expected(&self, bundle: [u8; 16], ordinal: u16) -> Result<()> {
        if self.bundle != bundle || self.ordinal != ordinal {
            return Err(Error::BundleLost);
        }
        Ok(())
    }
    pub fn bundle(&self) -> [u8; 16] {
        self.bundle
    }
    pub fn key(&self) -> RoleAndGeneration {
        self.key
    }
    pub fn remaining(&self) -> u16 {
        self.remaining
    }
    pub fn returned(&self) -> u16 {
        self.returned
    }
    pub fn ordinal(&self) -> u16 {
        self.ordinal
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub fn final_page(&self) -> bool {
        self.final_page
    }
    pub fn position(&self) -> Position {
        self.cursor
    }
    fn validate(&self, b: &ListBinding) -> Result<()> {
        if self.bundle == [0; 16]
            || self.key.role == Role::NONE
            || self.key.generation == 0
            || self.key.generation > i64::MAX as u64
            || self.remaining.checked_add(self.returned) != Some(b.count())
            || self.returned > 4096
            || self.ordinal > self.returned.saturating_add(1)
            || (self.ordinal > self.returned && !self.final_page)
            || matches!(self.cursor, Position::Time(time, _) if time > i64::MAX as u64)
            || self.complete != (self.remaining == 0 || self.final_page)
            || (self.remaining == 0 && self.final_page)
        {
            return Err(Error::InvalidToken);
        }
        if self.ordinal == 0
            && (self.returned != 0 || self.complete || self.cursor != Position::None)
        {
            return Err(Error::InvalidToken);
        }
        if self.ordinal > 0
            && self.returned == 0
            && !(self.ordinal == 1
                && self.complete
                && self.final_page
                && self.cursor == Position::None)
        {
            return Err(Error::InvalidToken);
        }
        if self.returned > 0 && !self.cursor.forward_from(&Position::None, &b.cursor) {
            return Err(Error::InvalidToken);
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "read-token-qualification"))]
pub mod qualification {
    //! Explicit fixture façade. It must never be selected by a frontend request.
    use super::*;
    pub fn authority(entropy: Zeroizing<[u8; 48]>) -> Result<ReadTokenAuthority> {
        ReadTokenAuthority::from_host_entropy(entropy)
    }
    pub fn issue(
        authority: &ReadTokenAuthority,
        binding: &ListBinding,
        key: RoleAndGeneration,
        bundle: [u8; 16],
        now: Number,
    ) -> Result<Token> {
        authority.issue(binding, key, bundle, now)
    }
    pub fn advance(
        authority: &ReadTokenAuthority,
        previous: &VerifiedToken,
        binding: &ListBinding,
        consumed: u16,
        limit: u16,
        last: Position,
        now: Number,
    ) -> Result<Token> {
        authority.advance(previous, binding, consumed, limit, last, now)
    }
}

#[cfg(test)]
mod tests;
