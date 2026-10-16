//! Private authority qualification kernel. Caller and time are trusted host inputs,
//! not public FOKS claims. The host owns the outer transaction and durable sync.
pub(crate) mod catalog;
mod personal;
pub use personal::{
    read_point as read_personal_point, run as run_personal, ErrorPayload as PersonalErrorPayload,
    Invocation as PersonalInvocation, PointInvocation as PersonalPointInvocation,
};

use crate::{wire, Error, Result, Sql, Statement as S, Value as V};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = include_str!("schema.sql");
pub const OUTCOME_RESERVE: usize = 4096;
pub const INTENT_CHARGE: i64 = 4608; // fixed binding metadata plus worst-case output
pub const MAX_PRUNE: i64 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Number(#[serde(with = "crate::catalog::decimal")] pub i64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub installation: [u8; 16],
    pub tenant: [u8; 16],
    pub host: Vec<u8>,
    pub cell: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    pub version: Number,
    pub schema: Number,
    pub schema_hash: [u8; 32],
    pub kernel_hash: [u8; 32],
    pub catalog_hash: [u8; 32],
    pub key_generation: Number,
    pub key_digest: [u8; 32],
    pub storage_namespace: [u8; 16],
    pub storage_placement: [u8; 32],
    pub limits: crate::Limits,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fence {
    pub identity: Identity,
    pub activation: Activation,
    pub owner: [u8; 16],
    pub epoch: Number,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub id: [u8; 32],
    pub namespace: Vec<u8>,
    pub role: Vec<u8>,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Caller {
    Controller {},
    Frontend { principal: [u8; 32] },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub epoch: Number,
    pub sequence: Number,
    pub revision: Number,
    pub expires: Number,
    // Parsed and reserialized by Rust before hashing, never interpreted by JS.
    pub command: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Initialize {
        identity: Identity,
        activation: Activation,
        owner: [u8; 16],
        max_rows: Number,
        max_bytes: Number,
    },
    Inspect {
        fence: Fence,
    },
    Promote {
        fence: Fence,
        new_owner: [u8; 16],
    },
    Activate {
        fence: Fence,
        revision: Number,
        activation: Activation,
    },
    SetPrincipal {
        fence: Fence,
        revision: Number,
        principal: Principal,
    },
    Register {
        fence: Fence,
        intent: Intent,
    },
    Execute {
        fence: Fence,
        intent: Intent,
    },
    Resolve {
        fence: Fence,
        epoch: Number,
        sequence: Number,
        digest: [u8; 32],
    },
    ObserveUsage {
        fence: Fence,
        revision: Number,
    },
    Prove {
        fence: Fence,
        revision: Number,
    },
    Prune {
        fence: Fence,
        limit: Number,
    },
    Settle {
        fence: Fence,
        limit: Number,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub caller: Caller,
    pub now: Number,
    pub deadline: Number,
    pub request: Request,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub revision: Number,
    pub reply: Option<wire::Reply>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    State {
        epoch: Number,
        revision: Number,
        high_water: Number,
        rows: Number,
        reserved_bytes: Number,
    },
    Registered {
        digest: [u8; 32],
    },
    Outcome {
        outcome: Outcome,
    },
    Pending,
    RetiredUnknown,
    Observation {
        revision: Number,
        values: Vec<V>,
    },
    Pruned {
        count: Number,
    },
    Settled {
        count: Number,
    },
}

struct State {
    identity: Identity,
    activation: Activation,
    owner: [u8; 16],
    epoch: i64,
    revision: i64,
    high_water: i64,
    rows: i64,
    reserved: i64,
    max_rows: i64,
    max_bytes: i64,
    clock: i64,
}
fn num(v: &V) -> Result<i64> {
    match v {
        V::Integer(n) if *n >= 0 => Ok(*n),
        _ => Err(Error::Storage),
    }
}
fn blob(v: &V) -> Result<&[u8]> {
    match v {
        V::Blob(bytes) => Ok(bytes),
        _ => Err(Error::Storage),
    }
}
fn text(v: &V) -> Result<&str> {
    match v {
        V::Text(text) => Ok(text),
        _ => Err(Error::Storage),
    }
}
fn encoded<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(v).map_err(|_| Error::Bounds)
}
fn json<T: Serialize>(v: &T) -> Result<V> {
    let bytes = encoded(v)?;
    if bytes.len() > 2048 {
        return Err(Error::Bounds);
    }
    Ok(V::Text(
        String::from_utf8(bytes).map_err(|_| Error::Storage)?,
    ))
}
fn load(sql: &mut impl Sql) -> Result<State> {
    let r = sql.query(S::AuthorityState, &[])?.ok_or(Error::Storage)?;
    if r.len() != 11 {
        return Err(Error::Storage);
    }
    Ok(State {
        identity: serde_json::from_str(text(&r[0])?).map_err(|_| Error::Storage)?,
        activation: serde_json::from_str(text(&r[1])?).map_err(|_| Error::Storage)?,
        owner: blob(&r[2])?.try_into().map_err(|_| Error::Storage)?,
        epoch: num(&r[3])?,
        revision: num(&r[4])?,
        high_water: num(&r[5])?,
        rows: num(&r[6])?,
        reserved: num(&r[7])?,
        max_rows: num(&r[8])?,
        max_bytes: num(&r[9])?,
        clock: num(&r[10])?,
    })
}
fn state_reply(s: &State) -> Reply {
    Reply::State {
        epoch: Number(s.epoch),
        revision: Number(s.revision),
        high_water: Number(s.high_water),
        rows: Number(s.rows),
        reserved_bytes: Number(s.reserved),
    }
}
fn fence(s: &State, f: &Fence) -> Result<()> {
    if s.identity != f.identity
        || s.activation != f.activation
        || s.owner != f.owner
        || s.epoch != f.epoch.0
    {
        return Err(Error::Permission);
    }
    Ok(())
}
fn revision(s: &State, expected: Number) -> Result<()> {
    if s.revision != expected.0 {
        return Err(Error::Conflict);
    }
    Ok(())
}
fn controller(caller: &Caller) -> Result<()> {
    if !matches!(caller, Caller::Controller {}) {
        return Err(Error::Permission);
    }
    Ok(())
}
// Fixture policy: an enabled principal has namespace aggregate-read access.
// Its role separately controls PutDirents. This is not FOKS credential/team auth.
fn fixture_principal(sql: &mut impl Sql, caller: &Caller) -> Result<Principal> {
    let Caller::Frontend { principal: id } = caller else {
        return Err(Error::Permission);
    };
    let r = sql
        .query(S::AuthorityPrincipal, &[V::Blob(id.to_vec())])?
        .ok_or(Error::Permission)?;
    if r.len() != 3 || num(&r[2])? != 1 {
        return Err(Error::Permission);
    }
    let p = Principal {
        id: *id,
        namespace: blob(&r[0])?.to_vec(),
        role: blob(&r[1])?.to_vec(),
        enabled: true,
    };
    validate_principal(&p)?;
    Ok(p)
}
// Only these crate-owned adapters may choose a resolution policy. Public
// personal invocations contain neither a fixture selector nor asserted identity.
trait Resolver {
    type Failure: From<Error>;
    fn resolve(
        &mut self,
        sql: &mut impl Sql,
        caller: &Caller,
    ) -> std::result::Result<Principal, Self::Failure>;
    fn authorize_domain(
        &mut self,
        sql: &mut impl Sql,
        command: &wire::Command,
    ) -> std::result::Result<(), Self::Failure>;
    fn context_digest(&self) -> Option<[u8; 32]> {
        None
    }
}
struct FixtureResolver;
impl Resolver for FixtureResolver {
    type Failure = Error;
    fn resolve(&mut self, sql: &mut impl Sql, caller: &Caller) -> Result<Principal> {
        fixture_principal(sql, caller)
    }
    fn authorize_domain(&mut self, _sql: &mut impl Sql, _command: &wire::Command) -> Result<()> {
        Ok(())
    }
}
fn validate_principal(p: &Principal) -> Result<()> {
    if p.namespace.len() != 33 || p.role.len() > 64 {
        return Err(Error::Bounds);
    }
    foks_proto::Role::decode(&p.role).map_err(|_| Error::Invalid("principal role"))?;
    Ok(())
}
fn activation(a: &Activation) -> Result<()> {
    if a.version.0 < 1 || a.schema.0 < 1 || a.key_generation.0 < 1 {
        return Err(Error::Invalid("activation"));
    }
    Ok(())
}
fn bump(sql: &mut impl Sql, s: &State) -> Result<i64> {
    let next = s.revision.checked_add(1).ok_or(Error::IntegerRange)?;
    if sql.execute(S::AuthorityRevision, &[])? != 1 {
        return Err(Error::Storage);
    }
    Ok(next)
}
fn domain(active: &Activation, intent: &Intent, p: &Principal) -> Result<wire::Command> {
    if intent.command.len() > crate::MAX_INPUT_BYTES
        || intent.epoch.0 < 1
        || intent.sequence.0 < 1
        || intent.revision.0 < 1
        || intent.expires.0 < 0
    {
        return Err(Error::Bounds);
    }
    let c: wire::Command =
        serde_json::from_str(&intent.command).map_err(|_| Error::Invalid("intent command"))?;
    match &c {
        wire::Command::PutDirents {
            uid, role, limits, ..
        } if *uid == p.namespace && *role == p.role && *limits == active.limits => Ok(c),
        _ => Err(Error::Permission),
    }
}
fn binding(
    f: &Fence,
    intent: &Intent,
    p: &Principal,
    c: &wire::Command,
    context: Option<[u8; 32]>,
) -> Result<[u8; 32]> {
    // Versioned typed serialization fixes field order and rejects unknown fields.
    // The raw command's whitespace/member order is not part of intent identity.
    let base = encoded(&(
        1u32,
        &f.identity,
        &f.activation,
        f.owner,
        intent.epoch,
        intent.sequence,
        intent.revision,
        intent.expires,
        p.id,
        &p.namespace,
        c,
    ))?;
    Ok(match context {
        None => Sha256::digest(base).into(),
        Some(context) => Sha256::digest(encoded(&(2u32, base, context))?).into(),
    })
}
struct Stored {
    digest: [u8; 32],
    principal: [u8; 32],
    namespace: Vec<u8>,
    revision: i64,
    expires: i64,
    outcome: Option<Outcome>,
}
fn stored(sql: &mut impl Sql, epoch: Number, sequence: Number) -> Result<Option<Stored>> {
    let Some(r) = sql.query(
        S::AuthorityIntent,
        &[V::Integer(epoch.0), V::Integer(sequence.0)],
    )?
    else {
        return Ok(None);
    };
    if r.len() != 7 {
        return Err(Error::Storage);
    }
    let outcome = match (&r[6], num(&r[5])?) {
        (V::Null, 0) => None,
        (V::Blob(b), 1) => Some(serde_json::from_slice(b).map_err(|_| Error::Storage)?),
        _ => return Err(Error::Storage),
    };
    Ok(Some(Stored {
        digest: blob(&r[0])?.try_into().map_err(|_| Error::Storage)?,
        principal: blob(&r[1])?.try_into().map_err(|_| Error::Storage)?,
        namespace: blob(&r[2])?.to_vec(),
        revision: num(&r[3])?,
        expires: num(&r[4])?,
        outcome,
    }))
}
fn match_stored(row: &Stored, digest: &[u8; 32], p: &Principal) -> Result<()> {
    if row.digest != *digest || row.principal != p.id || row.namespace != p.namespace {
        return Err(Error::Conflict);
    }
    Ok(())
}
fn finish(
    sql: &mut impl Sql,
    epoch: Number,
    sequence: Number,
    digest: &[u8; 32],
    outcome: Outcome,
) -> Result<Reply> {
    let bytes = encoded(&outcome)?;
    if bytes.len() > OUTCOME_RESERVE {
        return Err(Error::Bounds);
    }
    if sql.execute(
        S::AuthorityOutcome,
        &[
            V::Blob(bytes),
            V::Integer(epoch.0),
            V::Integer(sequence.0),
            V::Blob(digest.to_vec()),
        ],
    )? != 1
    {
        return Err(Error::Storage);
    }
    Ok(Reply::Outcome { outcome })
}

/// All errors require rollback of the host-owned outer transaction. An Ok reply
/// is publishable only after host COMMIT and durable sync; it is not a receipt yet.
pub fn run(sql: &mut impl Sql, invocation: Invocation) -> Result<Reply> {
    run_resolved(sql, invocation, &mut FixtureResolver)
}

fn run_resolved<R: Resolver>(
    sql: &mut impl Sql,
    invocation: Invocation,
    resolver: &mut R,
) -> std::result::Result<Reply, R::Failure> {
    let Invocation {
        caller,
        now,
        deadline,
        request,
    } = invocation;
    if now.0 < 0 || deadline.0 <= now.0 || deadline.0.checked_sub(now.0).is_none_or(|n| n > 15000) {
        return Err(Error::Expired.into());
    }
    if let Request::Initialize {
        identity,
        activation: active,
        owner,
        max_rows,
        max_bytes,
    } = &request
    {
        controller(&caller)?;
        activation(active)?;
        if identity.host.len() != 33
            || max_rows.0 < 1
            || max_rows.0 > 256
            || max_bytes.0 < INTENT_CHARGE
            || max_bytes.0 > 1048576
        {
            return Err(Error::Bounds.into());
        }
        if sql.query(S::AuthorityState, &[])?.is_some() {
            return Err(Error::Conflict.into());
        }
        sql.execute(
            S::AuthorityInitialize,
            &[
                json(identity)?,
                json(active)?,
                V::Blob(owner.to_vec()),
                V::Integer(max_rows.0),
                V::Integer(max_bytes.0),
                V::Integer(now.0),
            ],
        )?;
        return Ok(state_reply(&load(sql)?));
    }
    let s = load(sql)?;
    if now.0 < s.clock {
        return Err(Error::Expired.into());
    }
    // This floor is fixture-authoritative time, not a claim about host clock sync.
    sql.execute(S::AuthorityClock, &[V::Integer(now.0)])?;
    match request {
        Request::Initialize { .. } => unreachable!(),
        Request::Inspect { fence: f } => {
            fence(&s, &f)?;
            if !matches!(caller, Caller::Controller {}) {
                resolver.resolve(sql, &caller)?;
            }
            Ok(state_reply(&s))
        }
        Request::Promote {
            fence: f,
            new_owner,
        } => {
            controller(&caller)?;
            fence(&s, &f)?;
            s.epoch.checked_add(1).ok_or(Error::IntegerRange)?;
            s.revision.checked_add(1).ok_or(Error::IntegerRange)?;
            if new_owner == s.owner {
                return Err(Error::Conflict.into());
            }
            if sql.execute(
                S::AuthorityPromote,
                &[
                    V::Blob(new_owner.to_vec()),
                    V::Integer(s.epoch),
                    V::Blob(s.owner.to_vec()),
                ],
            )? != 1
            {
                return Err(Error::Conflict.into());
            }
            Ok(state_reply(&load(sql)?))
        }
        Request::Activate {
            fence: f,
            revision: r,
            activation: active,
        } => {
            controller(&caller)?;
            fence(&s, &f)?;
            revision(&s, r)?;
            activation(&active)?;
            if active.version.0
                != s.activation
                    .version
                    .0
                    .checked_add(1)
                    .ok_or(Error::IntegerRange)?
                || active.schema.0 < s.activation.schema.0
                || active.key_generation.0 < s.activation.key_generation.0
                || (active.key_generation == s.activation.key_generation
                    && active.key_digest != s.activation.key_digest)
            {
                return Err(Error::Conflict.into());
            }
            s.revision.checked_add(1).ok_or(Error::IntegerRange)?;
            sql.execute(S::AuthorityActivate, &[json(&active)?])?;
            Ok(state_reply(&load(sql)?))
        }
        Request::SetPrincipal {
            fence: f,
            revision: r,
            principal: p,
        } => {
            controller(&caller)?;
            fence(&s, &f)?;
            revision(&s, r)?;
            validate_principal(&p)?;
            sql.execute(
                S::AuthoritySetPrincipal,
                &[
                    V::Blob(p.id.to_vec()),
                    V::Blob(p.namespace),
                    V::Blob(p.role),
                    V::Integer(i64::from(p.enabled)),
                ],
            )?;
            bump(sql, &s)?;
            Ok(state_reply(&load(sql)?))
        }
        Request::Register { fence: f, intent } => {
            fence(&s, &f)?;
            let p = resolver.resolve(sql, &caller)?;
            let c = domain(&s.activation, &intent, &p)?;
            let digest = binding(&f, &intent, &p, &c, resolver.context_digest())?;
            if intent.epoch.0 != s.epoch {
                return Err(Error::Permission.into());
            }
            if let Some(row) = stored(sql, intent.epoch, intent.sequence)? {
                match_stored(&row, &digest, &p)?;
                return Ok(Reply::Registered { digest });
            }
            if intent.sequence.0 <= s.high_water {
                return Ok(Reply::RetiredUnknown);
            }
            revision(&s, intent.revision)?;
            resolver.authorize_domain(sql, &c)?;
            // Invocation clocks are milliseconds; FOKS dirent creation time is
            // microseconds. Known exact registrations returned above, preserving
            // their original bound time rather than resampling it on retry.
            if let wire::Command::PutDirents {
                normalize_at: Some(created),
                ..
            } = &c
            {
                let micros = now.0.checked_mul(1000).ok_or(Error::IntegerRange)?;
                if *created != u64::try_from(micros).map_err(|_| Error::IntegerRange)? {
                    return Err(Error::Invalid("normalization time").into());
                }
            }
            if intent.expires.0 <= now.0 || intent.expires.0 > deadline.0 {
                return Err(Error::Expired.into());
            }
            if intent.sequence.0 != s.high_water.checked_add(1).ok_or(Error::IntegerRange)? {
                return Err(Error::Conflict.into());
            }
            if s.rows >= s.max_rows || s.reserved > s.max_bytes - INTENT_CHARGE {
                return Err(Error::Quota.into());
            }
            if sql.execute(
                S::AuthorityAllocate,
                &[V::Integer(intent.sequence.0), V::Integer(s.high_water)],
            )? != 1
            {
                return Err(Error::Conflict.into());
            }
            sql.execute(
                S::AuthorityRegister,
                &[
                    V::Integer(intent.epoch.0),
                    V::Integer(intent.sequence.0),
                    V::Blob(digest.to_vec()),
                    V::Blob(p.id.to_vec()),
                    V::Blob(p.namespace),
                    V::Integer(intent.revision.0),
                    V::Integer(intent.expires.0),
                ],
            )?;
            Ok(Reply::Registered { digest })
        }
        Request::Execute { fence: f, intent } => {
            fence(&s, &f)?;
            let p = resolver.resolve(sql, &caller)?;
            let c = domain(&s.activation, &intent, &p)?;
            let digest = binding(&f, &intent, &p, &c, resolver.context_digest())?;
            if intent.epoch.0 != s.epoch {
                return Err(Error::Permission.into());
            }
            let Some(row) = stored(sql, intent.epoch, intent.sequence)? else {
                return Ok(Reply::RetiredUnknown);
            };
            match_stored(&row, &digest, &p)?;
            if let Some(outcome) = row.outcome {
                return Ok(Reply::Outcome { outcome });
            }
            if row.expires <= now.0 {
                return Err(Error::Expired.into());
            }
            revision(&s, Number(row.revision))?;
            resolver.authorize_domain(sql, &c)?;
            let result = sql.domain(&encoded(&c)?);
            let (reply, error) = match result {
                Ok(reply) => (Some(reply), None),
                Err(error @ (Error::Conflict | Error::Permission | Error::Quota)) => {
                    (None, Some(error.code().to_owned()))
                }
                // Bounds/cancellation/corruption/host errors are not a durable
                // public rejection. Preserve ambiguity and roll back everything.
                Err(error) => return Err(error.into()),
            };
            let revision = Number(bump(sql, &s)?);
            finish(
                sql,
                intent.epoch,
                intent.sequence,
                &digest,
                Outcome {
                    revision,
                    reply,
                    error,
                },
            )
            .map_err(Into::into)
        }
        Request::Resolve {
            fence: f,
            epoch,
            sequence,
            digest,
        } => {
            fence(&s, &f)?;
            let p = resolver.resolve(sql, &caller)?;
            if epoch.0 < 1 || epoch.0 > s.epoch || sequence.0 < 1 {
                return Err(Error::Bounds.into());
            }
            let Some(row) = stored(sql, epoch, sequence)? else {
                return Ok(Reply::RetiredUnknown);
            };
            match_stored(&row, &digest, &p)?;
            if let Some(outcome) = row.outcome {
                return Ok(Reply::Outcome { outcome });
            }
            // SQL-only intents: serialization + closed epoch / immutable expiry
            // proves an absent outcome cannot later acquire domain effects.
            if epoch.0 < s.epoch || row.expires <= now.0 || row.revision != s.revision {
                return finish(
                    sql,
                    epoch,
                    sequence,
                    &digest,
                    Outcome {
                        revision: Number(s.revision),
                        reply: None,
                        error: Some("not_executed".into()),
                    },
                )
                .map_err(Into::into);
            }
            Ok(Reply::Pending)
        }
        Request::ObserveUsage {
            fence: f,
            revision: r,
        } => {
            fence(&s, &f)?;
            revision(&s, r)?;
            let p = resolver.resolve(sql, &caller)?;
            let values = sql
                .query(S::Usage, &[V::Blob(p.namespace)])?
                .ok_or(Error::Conflict)?;
            Ok(Reply::Observation {
                revision: r,
                values,
            })
        }
        Request::Prove {
            fence: f,
            revision: r,
        } => {
            fence(&s, &f)?;
            revision(&s, r)?;
            resolver.resolve(sql, &caller)?;
            Ok(Reply::Observation {
                revision: r,
                values: vec![],
            })
        }
        Request::Settle { fence: f, limit } => {
            controller(&caller)?;
            fence(&s, &f)?;
            if limit.0 < 1 || limit.0 > MAX_PRUNE {
                return Err(Error::Bounds.into());
            }
            let outcome = Outcome {
                revision: Number(s.revision),
                reply: None,
                error: Some("not_executed".into()),
            };
            let n = sql.execute(
                S::AuthoritySettle,
                &[
                    V::Blob(encoded(&outcome)?),
                    V::Integer(s.epoch),
                    V::Integer(now.0),
                    V::Integer(s.revision),
                    V::Integer(limit.0),
                ],
            )?;
            Ok(Reply::Settled {
                count: Number(i64::try_from(n).map_err(|_| Error::IntegerRange)?),
            })
        }
        Request::Prune { fence: f, limit } => {
            controller(&caller)?;
            fence(&s, &f)?;
            if limit.0 < 1 || limit.0 > MAX_PRUNE {
                return Err(Error::Bounds.into());
            }
            let n = sql.execute(S::AuthorityPrune, &[V::Integer(limit.0)])?;
            Ok(Reply::Pruned {
                count: Number(i64::try_from(n).map_err(|_| Error::IntegerRange)?),
            })
        }
    }
}
