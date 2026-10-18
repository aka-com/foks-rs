//! Real current-certificate resolver for the private A1 qualification surface.
//! No endpoint selects this versus the synthetic fixture resolver. TLS possession
//! and the owner capability remain responsibilities of the trusted frontend/host.
mod admission;

use super::{Caller, Number, Principal, Reply, Request, Resolver};
use crate::personal::{self, CurrentCredential, PeerCertificate, PersonalError};
use crate::{wire, Sql};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub peer: PeerCertificate,
    pub now: Number,
    pub deadline: Number,
    pub request: Request,
}

struct PersonalResolver<'a> {
    peer: &'a PeerCertificate,
    now: Number,
    current: Option<CurrentCredential>,
    context: [u8; 32],
}
impl<'a> PersonalResolver<'a> {
    fn new(peer: &'a PeerCertificate, now: Number) -> personal::Result<Self> {
        let context = admission::peer_context_digest(peer)?;
        Ok(Self {
            peer,
            now,
            current: None,
            context,
        })
    }
    fn now_micros(&self) -> personal::Result<u64> {
        let micros = self
            .now
            .0
            .checked_mul(1000)
            .ok_or(crate::Error::IntegerRange)?;
        u64::try_from(micros).map_err(|_| crate::Error::IntegerRange.into())
    }
}
impl Resolver for PersonalResolver<'_> {
    type Failure = PersonalError;
    fn resolve(&mut self, sql: &mut impl Sql, caller: &Caller) -> personal::Result<Principal> {
        if !matches!(caller, Caller::Frontend { .. }) {
            return Err(PersonalError::Permission);
        }
        let current = personal::authenticate_peer(sql, self.peer, self.now_micros()?)?;
        personal::require_personal_profile(sql, &current)?;
        // Principal identity survives certificate renewal, while the separate
        // context digest fixes the exact original certificate at Register/Execute.
        let id = admission::principal_digest(&current)?;
        let principal = Principal {
            id,
            namespace: crate::personal::admission::copy(current.uid())?,
            role: admission::role(current.role())?,
            enabled: true,
        };
        self.current = Some(current);
        Ok(principal)
    }
    fn authorize_domain(
        &mut self,
        sql: &mut impl Sql,
        command: &wire::Command,
    ) -> personal::Result<()> {
        let current = self.current.as_ref().ok_or(PersonalError::Corrupt)?;
        personal::authorize_personal_put(sql, current, command)
    }
    fn context_digest(&self) -> Option<[u8; 32]> {
        Some(self.context)
    }
}

/// Resolve current database authority inside the caller's already-open outer
/// transaction. Never commit or publish here. Controller requests are rejected;
/// this surface cannot populate or consult the synthetic principal table.
pub fn run(sql: &mut impl Sql, invocation: Invocation) -> personal::Result<Reply> {
    let Invocation {
        peer,
        now,
        deadline,
        request,
    } = invocation;
    let mut resolver = PersonalResolver::new(&peer, now)?;
    // This is an internal frontend marker, not an asserted principal. The sealed
    // resolver derives the actual identity from current issued-certificate rows.
    super::run_resolved(
        sql,
        super::Invocation {
            caller: Caller::Frontend { principal: [0; 32] },
            now,
            deadline,
            request,
        },
        &mut resolver,
    )
}

/// Scalar reads use the same owner/activation/revision gate and current principal
/// as execution. The host keeps this entire operation in one SQL transaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointInvocation {
    pub peer: PeerCertificate,
    pub now: Number,
    pub deadline: Number,
    pub fence: super::Fence,
    pub revision: Number,
    pub point: personal::PersonalPoint,
}

pub fn read_point(
    sql: &mut impl Sql,
    invocation: PointInvocation,
) -> personal::Result<wire::Reply> {
    let mut resolver = PersonalResolver::new(&invocation.peer, invocation.now)?;
    super::run_resolved(
        sql,
        super::Invocation {
            caller: Caller::Frontend { principal: [0; 32] },
            now: invocation.now,
            deadline: invocation.deadline,
            request: Request::Prove {
                fence: invocation.fence,
                revision: invocation.revision,
            },
        },
        &mut resolver,
    )?;
    let current = resolver.current.as_ref().ok_or(PersonalError::Corrupt)?;
    personal::read_personal_point(sql, current, &invocation.point, resolver.now_micros()?)
}

/// Explicit qualification error codec. Protocol stale vectors retain their exact
/// Snowpack representation. Kernel errors remain separately tagged infrastructure
/// or domain codes; the host never treats this outer payload as a domain callback
/// rejection token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ErrorPayload {
    Unauthenticated,
    Permission,
    Missing,
    UnsupportedProfile,
    Corrupt,
    BadArguments,
    StalePreparation,
    CachedObjectMissing,
    StaleCache { exact_hex: String },
    Kernel { code: &'static str },
}
impl From<&PersonalError> for ErrorPayload {
    fn from(error: &PersonalError) -> Self {
        match error {
            PersonalError::Unauthenticated => Self::Unauthenticated,
            PersonalError::Permission => Self::Permission,
            PersonalError::Missing => Self::Missing,
            PersonalError::UnsupportedProfile => Self::UnsupportedProfile,
            PersonalError::Corrupt => Self::Corrupt,
            PersonalError::BadArguments => Self::BadArguments,
            PersonalError::StalePreparation => Self::StalePreparation,
            PersonalError::CachedObjectMissing => Self::CachedObjectMissing,
            PersonalError::StaleCache(vector) => match vector.encode() {
                Ok(exact) => stale_payload(&exact),
                Err(_) => Self::Corrupt,
            },
            PersonalError::Kernel(error) => Self::Kernel { code: error.code() },
        }
    }
}

// Keep the existing 64KiB outer ABI; A2 must qualify larger public vectors.
// Hex has exact 2x expansion, allowing rejection before allocating that string.
const MAX_ERROR_ENVELOPE: usize = 65536;
const STALE_ENVELOPE_OVERHEAD: usize = r#"{"kind":"stale_cache","exact_hex":""}"#.len();
fn stale_payload(exact: &[u8]) -> ErrorPayload {
    if exact.len() > (MAX_ERROR_ENVELOPE - STALE_ENVELOPE_OVERHEAD) / 2 {
        return ErrorPayload::UnsupportedProfile;
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut exact_hex = String::with_capacity(exact.len() * 2);
    for byte in exact {
        exact_hex.push(HEX[usize::from(byte >> 4)] as char);
        exact_hex.push(HEX[usize::from(byte & 15)] as char);
    }
    ErrorPayload::StaleCache { exact_hex }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_error_codec_is_exact_and_bounded_before_hex_allocation() {
        let maximum = (MAX_ERROR_ENVELOPE - STALE_ENVELOPE_OVERHEAD) / 2;
        for size in [maximum - 1, maximum] {
            let bytes = vec![255; size];
            let payload = stale_payload(&bytes);
            let encoded = serde_json::to_vec(&payload).unwrap();
            assert_eq!(encoded.len(), STALE_ENVELOPE_OVERHEAD + 2 * size);
            assert!(encoded.len() <= MAX_ERROR_ENVELOPE);
            let ErrorPayload::StaleCache { exact_hex } = payload else {
                panic!("in-bound vector rejected")
            };
            assert_eq!(exact_hex, "ff".repeat(size));
        }
        assert_eq!(
            stale_payload(&vec![0; maximum + 1]),
            ErrorPayload::UnsupportedProfile
        );
        let vector = foks_proto::KvPathVersionVector {
            root_version: u64::MAX,
            directories: (0..256)
                .map(|n| foks_proto::KvDirectoryVersion {
                    id: [n as u8; 16],
                    version: u64::MAX,
                    entries: (0..4)
                        .map(|e| foks_proto::KvDirentVersion {
                            id: [e; 16],
                            version: u64::MAX,
                        })
                        .collect(),
                })
                .collect(),
        };
        let exact = vector.encode().unwrap();
        assert!(
            exact.len() > maximum,
            "max public vector must exercise explicit A1 bound"
        );
        assert_eq!(
            ErrorPayload::from(&PersonalError::StaleCache(vector)),
            ErrorPayload::UnsupportedProfile
        );
        let small = foks_proto::KvPathVersionVector {
            root_version: 1,
            directories: vec![],
        };
        let ErrorPayload::StaleCache { exact_hex } =
            ErrorPayload::from(&PersonalError::StaleCache(small.clone()))
        else {
            panic!("small vector rejected")
        };
        let decoded: Vec<u8> = exact_hex
            .as_bytes()
            .chunks_exact(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect();
        assert_eq!(
            foks_proto::KvPathVersionVector::decode(&decoded).unwrap(),
            small
        );
    }
}
