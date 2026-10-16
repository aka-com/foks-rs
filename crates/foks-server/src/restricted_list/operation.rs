//! One admitted native operation. Controls are retained by blocking work/output.
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use crate::{deadline::Deadline, net::memory::Request};
use foks_metadata_core::{Error, Result};

// Rust ArcInner is repr(C): strong, weak, then the value. These are requested
// layout bytes; allocator rounding and engine/TLS memory are separate owners.
#[repr(C)]
struct ArcLayout<T> {
    strong: AtomicUsize,
    weak: AtomicUsize,
    value: T,
}
pub(super) const fn arc_bytes<T>() -> usize {
    std::mem::size_of::<ArcLayout<T>>()
}

pub(super) struct OperationControl {
    deadline: Deadline,
    memory: Arc<Request>,
    cancelled: AtomicBool,
}
impl OperationControl {
    pub(super) fn new(deadline: Deadline, memory: Arc<Request>) -> Result<Arc<Self>> {
        // The installed rusqlite progress closure owns one Arc, boxed by
        // rusqlite0.40.2. Replacement with the idle zero-capture closure allocates
        // no payload. One hook spans both Main and FreshPublication intervals.
        let charge = arc_bytes::<Self>() + std::mem::size_of::<Arc<Self>>();
        if memory.failed() || !foks_snowpack::AllocationBudget::reserve(memory.as_ref(), charge) {
            return Err(Error::Bounds);
        }
        if deadline.instant() <= std::time::Instant::now() {
            return Err(Error::Expired);
        }
        Ok(Arc::new(Self {
            deadline,
            memory,
            cancelled: AtomicBool::new(false),
        }))
    }

    pub(super) fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(Error::Expired);
        }
        if self.memory.failed() {
            return Err(Error::Bounds);
        }
        if self.deadline.instant() <= std::time::Instant::now() {
            return Err(Error::Expired);
        }
        Ok(())
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(super) fn deadline(&self) -> Deadline {
        self.deadline
    }

    pub(super) fn memory(&self) -> &Arc<Request> {
        &self.memory
    }

    #[cfg(test)]
    pub(super) fn test(duration: std::time::Duration, memory: Arc<Request>) -> Arc<Self> {
        Self::new(Deadline::after(duration).unwrap(), memory).unwrap()
    }
}

#[cfg(test)]
pub(super) fn test_memory() -> Arc<Request> {
    crate::net::memory::Pools::new(crate::SessionLimits::default()).request(None)
}

#[cfg(test)]
mod tests;

use super::{
    bootstrap::NativeOwner,
    certificate::{CertificateFields, CertificateWorkspace},
    connection::{self, SchemaScratch},
    output::{Frame, Pending},
    SessionPeer,
};
use foks_metadata_core::{
    native::NativeSql,
    personal::{
        self,
        native_list::{self, AuthorityDecision, IssuedCredential},
        CredentialKind, PersonalError,
    },
    Sql, SqlCallBudget, Statement, Value,
};
use foks_rpc::{DecodedCall, RpcStatus};
use foks_server_db::Clock;
use tokio::sync::OwnedSemaphorePermit;

/// The diagnostic allocator envelope is additional to the generic decoder's
/// element/tag/Box charges. Keep the actual source/toolchain evidence in profile.
pub(super) fn diagnostic_bytes() -> usize {
    193 * std::mem::size_of::<foks_snowpack::PathSegment>() + 4030 + 18545 + 287
}
pub(super) fn ingress_diagnostic_bytes() -> usize {
    // Packet validation precedes the per-method bound. Conservatively allow a
    // twenty-digit usize offset; actual errors are discarded without formatting.
    diagnostic_bytes() + 3 * (20 - 5)
}
fn charge(memory: &Request, bytes: usize) -> Result<()> {
    if memory.failed() || !foks_snowpack::AllocationBudget::reserve(memory, bytes) {
        return Err(Error::Bounds);
    }
    Ok(())
}
fn fixed_status(memory: &Request, text: &'static str, permission: bool) -> Result<RpcStatus> {
    charge(memory, text.len())?;
    Ok(if permission {
        RpcStatus::PermissionDenied(text.to_owned())
    } else {
        RpcStatus::BadArguments(text.to_owned())
    })
}
fn permission(memory: &Request) -> Result<RpcStatus> {
    fixed_status(
        memory,
        "active device does not authorize this KV namespace",
        true,
    )
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FixedId {
    bytes: [u8; 34],
    length: u8,
}
impl FixedId {
    fn new(value: &[u8]) -> Result<Self> {
        if !matches!(value.len(), 33 | 34) {
            return Err(Error::Storage);
        }
        let mut bytes = [0; 34];
        bytes[..value.len()].copy_from_slice(value);
        Ok(Self {
            bytes,
            length: value.len() as u8,
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IssuedStamp {
    uid: [u8; 33],
    credential: FixedId,
    kind: CredentialKind,
    expiry: u64,
}
impl IssuedStamp {
    fn from(issued: &IssuedCredential) -> Result<Self> {
        Ok(Self {
            uid: *issued.uid(),
            credential: FixedId::new(issued.credential_id())?,
            kind: issued.kind(),
            expiry: issued.stored_expiry_micros(),
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Journal {
    InitialRoute,
    OuterSyntax,
    AuthorityClock,
    NoOwner,
    NoAnchor(FixedId),
    InvalidSelectedRole {
        owner: FixedId,
        role_type: i64,
        visibility: i64,
    },
    Authorized {
        owner: FixedId,
        role: foks_proto::Role,
    },
}
impl Journal {
    fn from(decision: &AuthorityDecision) -> Result<Self> {
        Ok(match decision {
            AuthorityDecision::NoOwner => Self::NoOwner,
            AuthorityDecision::NoAnchor { owner } => Self::NoAnchor(FixedId::new(owner)?),
            AuthorityDecision::InvalidRole {
                owner,
                role_type,
                visibility,
            } => Self::InvalidSelectedRole {
                owner: FixedId::new(owner)?,
                role_type: *role_type,
                visibility: *visibility,
            },
            AuthorityDecision::Authorized(credential) => Self::Authorized {
                owner: FixedId::new(credential.owner_id())?,
                role: credential.role(),
            },
        })
    }
    fn observes_authority(self) -> bool {
        !matches!(
            self,
            Self::InitialRoute | Self::OuterSyntax | Self::AuthorityClock
        )
    }
}

fn state(owner: &NativeOwner, budget: &mut SqlCallBudget, floor: u64) -> Result<()> {
    let mut sql = NativeSql::new_with_budget(&owner.connection, budget)?;
    let row = sql
        .query(Statement::AuthorityState, &[])?
        .ok_or(Error::Storage)?;
    let [
        Value::Text(identity),
        Value::Text(activation),
        Value::Blob(held_owner),
        Value::Integer(epoch),
        Value::Integer(revision),
        Value::Integer(high_water),
        Value::Integer(rows),
        Value::Integer(reserved),
        Value::Integer(max_rows),
        Value::Integer(max_bytes),
        Value::Integer(clock_floor),
    ]: [Value; 11] = row.try_into().map_err(|_| Error::Storage)?
    else {
        return Err(Error::Storage);
    };
    if identity != owner.expected.identity_json
        || activation != owner.expected.activation_json
        || held_owner.as_slice() != owner.expected.owner
        || epoch != 1
        || revision != 1
        || high_water != 0
        || rows != 0
        || reserved != 0
        || max_rows != 1
        || max_bytes != 4608
        || u64::try_from(clock_floor).ok() != Some(floor)
    {
        return Err(Error::Storage);
    }
    Ok(())
}
fn no_sso(sql: &mut impl Sql) -> Result<()> {
    if sql.query(Statement::PersonalSsoPolicy, &[])?.is_some() {
        return Err(Error::Storage);
    }
    Ok(())
}
fn initial_auth(
    sql: &mut impl Sql,
    peer: &SessionPeer,
    workspace: &mut CertificateWorkspace,
    clock: &dyn Clock,
) -> Result<(IssuedCredential, u64, CertificateFields)> {
    let now = clock.now_micros().map_err(|_| Error::Storage)?;
    if now > i64::MAX as u64 {
        return Err(Error::IntegerRange);
    }
    let fields = workspace.parse(&peer.der).map_err(|_| Error::Storage)?;
    let issued =
        native_list::authenticate(sql, &peer.der, fields.serial(), &fields.public_key, now)
            .map_err(private_error)?;
    // Preserve Principal::authenticate: DER expiry conversion follows SQL,
    // credential parsing, public-key binding and credential-kind validation.
    fields.native_expiry_micros().map_err(|_| Error::Storage)?;
    Ok((issued, now, fields))
}
fn private_error(error: PersonalError) -> Error {
    match error {
        PersonalError::Kernel(error) => error,
        _ => Error::Storage,
    }
}
fn frame_status(status: RpcStatus, sequence: u64) -> Result<Frame> {
    foks_rpc::encode_status_response_at(&status, sequence)
        .map(Frame::Status)
        .map_err(|_| Error::Bounds)
}
struct Candidate {
    frame: Frame,
    journal: Journal,
    issued: IssuedStamp,
    auth_micros: u64,
    certificate: CertificateFields,
}

fn candidate(
    owner: &NativeOwner,
    budget: &mut SqlCallBudget,
    call: &DecodedCall,
    peer: &SessionPeer,
    workspace: &mut CertificateWorkspace,
    control: &OperationControl,
    clock: &dyn Clock,
) -> Result<Candidate> {
    let memory = control.memory();
    let mut sql = NativeSql::new_with_budget(&owner.connection, budget)?;
    let (issued, auth_micros, certificate) = initial_auth(&mut sql, peer, workspace, clock)?;
    if auth_micros / 1000 < owner.expected.floor {
        return Err(Error::Expired);
    }
    let stamp = IssuedStamp::from(&issued)?;
    let status = |status, journal| -> Result<Candidate> {
        Ok(Candidate {
            frame: frame_status(status, call.sequence())?,
            journal,
            issued: stamp,
            auth_micros,
            certificate,
        })
    };
    // Native routing errors follow Principal authentication but precede SSO.
    let Some(route) = crate::rpc::route(call.protocol_id(), call.method_position()) else {
        return status(RpcStatus::Unsupported, Journal::InitialRoute);
    };
    if !route.listeners.contains(&"authenticated") {
        return status(RpcStatus::Unsupported, Journal::InitialRoute);
    }
    if call.argument().len() > route.max_request_bytes {
        return status(
            fixed_status(memory, "request exceeds the method limit", false)?,
            Journal::InitialRoute,
        );
    }
    if !route.supported || route.id != crate::rpc::RouteId::KvStoreList {
        return status(RpcStatus::Unsupported, Journal::InitialRoute);
    }
    if clock.now_micros().is_err() {
        return status(RpcStatus::TransactionRetry, Journal::InitialRoute);
    }
    no_sso(&mut sql)?;
    if issued.kind() == CredentialKind::BackupKey {
        return status(
            fixed_status(memory, "credential is recovery-only", true)?,
            Journal::InitialRoute,
        );
    }
    if call.argument().len() > super::profile::MAX_ARGUMENT {
        return Err(Error::Bounds);
    }
    charge(memory, diagnostic_bytes())?;
    let outer = match foks_rpc::arguments::KvAuthorityArgument::decode(call.argument()) {
        Ok(outer) => outer,
        Err(error) => return status(error, Journal::OuterSyntax),
    };
    if clock.now_micros().is_err() {
        drop(outer);
        return status(RpcStatus::TransactionRetry, Journal::AuthorityClock);
    }
    let selected = match outer.classify() {
        Ok(selected) => selected,
        Err(error) => {
            drop(outer);
            return status(error, Journal::AuthorityClock);
        }
    };
    if matches!(selected, foks_rpc::arguments::KvAuthoritySelection::Team(_)) {
        // R3 audited an empty team-token ledger. This private owner exposes only
        // the authority floor write and retains the exclusive writer lease;
        // no team-token mutation is reachable. Dependency triggers do not cover
        // team tokens. Native lookup is absent; no team backend exists.
        drop(outer);
        return status(RpcStatus::Expired, Journal::AuthorityClock);
    }
    let decision = native_list::authority(&mut sql, issued).map_err(private_error)?;
    let journal = Journal::from(&decision)?;
    // Retain the complete F0 tree through authority SQL, then actually destroy it
    // before the second decode. Lexical/NLL borrow expiration is not destruction.
    drop(outer);
    let AuthorityDecision::Authorized(credential) = decision else {
        return status(permission(memory)?, journal);
    };
    charge(memory, diagnostic_bytes())?;
    let arguments = match foks_rpc::arguments::decode_kv_list_arguments(call.argument()) {
        Ok(arguments) => arguments,
        Err(error) => return status(error, journal),
    };
    let cursor = match arguments.cursor {
        foks_rpc::KvListCursor::None => personal::ListCursor::Initial,
        foks_rpc::KvListCursor::Mac(mac) => personal::ListCursor::Mac(mac),
        foks_rpc::KvListCursor::Time(time) => personal::ListCursor::Time(time),
    };
    let request = personal::ListRequest::new(
        arguments.parent,
        cursor,
        arguments.number,
        arguments.load_small,
    );
    let value = personal::read_personal_list_owned(
        &mut sql,
        &credential,
        &request,
        auth_micros,
        1_048_576,
        memory.clone(),
    );
    let frame = match value {
        Ok(value) => Frame::List(
            foks_metadata_core::owned_list::frame_owned_list(value, call.sequence())
                .map_err(|_| Error::Bounds)?,
        ),
        Err(PersonalError::Permission) => return status(permission(memory)?, journal),
        Err(PersonalError::Missing) => return status(RpcStatus::KvNoEnt, journal),
        // This classification occurs only after the actual List helper ran under
        // valid C4/auth admission. Its typed Corrupt covers native data/encoding
        // and cursor range failures, which List maps to TransactionRetry. SQL,
        // admission, ownership and expiry errors remain private Kernel failures.
        Err(PersonalError::Corrupt) => return status(RpcStatus::TransactionRetry, journal),
        Err(error) => return Err(private_error(error)),
    };
    control.check()?;
    Ok(Candidate {
        frame,
        journal,
        issued: stamp,
        auth_micros,
        certificate,
    })
}

fn begin(
    connection: &rusqlite::Connection,
    budget: &mut SqlCallBudget,
    control: &OperationControl,
    write: bool,
) -> Result<()> {
    control.check()?;
    if !connection.is_autocommit() {
        return Err(Error::Storage);
    }
    budget.begin_cleanup_reservation()?;
    budget.attempt()?;
    connection
        .execute_batch(if write { "BEGIN IMMEDIATE" } else { "BEGIN" })
        .map_err(|_| Error::Storage)?;
    control.check()
}
fn commit(
    connection: &rusqlite::Connection,
    budget: &mut SqlCallBudget,
    control: &OperationControl,
) -> Result<()> {
    control.check()?;
    budget.attempt()?;
    connection
        .execute_batch("COMMIT")
        .map_err(|_| Error::Storage)?;
    budget.finish_cleanup_reservation()?;
    control.check()
}
fn rollback(connection: &rusqlite::Connection, budget: &mut SqlCallBudget) {
    // Only this fixed cleanup may ignore an expired/cancelled execution hook.
    // Any failing operation destroys its owner; there is no failed-scope reuse.
    if connection
        .progress_handler(1, None::<fn() -> bool>)
        .is_err()
    {
        return;
    }
    if !connection.is_autocommit() {
        if budget.attempt_rollback().is_ok() {
            let _ = connection.execute_batch("ROLLBACK");
        }
    } else {
        let _ = budget.finish_cleanup_reservation();
    }
}
fn install(connection: &rusqlite::Connection, control: &Arc<OperationControl>) -> Result<()> {
    let owned = control.clone();
    connection
        .progress_handler(1000, Some(move || owned.check().is_err()))
        .map_err(|_| Error::Storage)
}
pub(super) fn idle(connection: &rusqlite::Connection) -> Result<()> {
    // Supplemental defense only. The guarantee is the private Idle owner with
    // no exposed connection or live statement; PRAGMA prepare may precede hooks.
    connection
        .progress_handler(1, Some(|| true))
        .map_err(|_| Error::Storage)
}

/// One move-only owner formed before task allocation. Field order also governs
/// an unstarted task, an early error and panic unwinding: close the database and
/// drop the request body before its memory owner, releasing execution last.
pub(super) struct Work {
    pub(super) owner: NativeOwner,
    pub(super) call: DecodedCall,
    pub(super) peer: Arc<SessionPeer>,
    pub(super) control: Arc<OperationControl>,
    pub(super) execution: OwnedSemaphorePermit,
}

pub(super) const MAX_TASK_PAYLOAD: usize = 16 * 1024;
/// Admit the owned values stored in this task before spawning it. The running
/// capture and returned result are alternative live phases, so use their maximum
/// and never charge their contained fields again. Tokio's Option<F>/Stage tags,
/// padding, scheduler/header and allocation retention are process overhead;
/// this is deliberately not a complete Tokio allocation or RSS estimate.
pub(super) fn admit_task<F: FnOnce() -> Result<Pending>>(
    control: &OperationControl,
    task: &F,
) -> Result<()> {
    let bytes = std::mem::size_of_val(task).max(std::mem::size_of::<Result<Pending>>());
    let alignment = std::mem::align_of_val(task).max(std::mem::align_of::<Result<Pending>>());
    let result = if bytes > MAX_TASK_PAYLOAD || alignment > 16 {
        Err(Error::Bounds)
    } else {
        control
            .check()
            .and_then(|()| charge(control.memory(), bytes))
    };
    if result.is_err() {
        foks_metadata_core::owned_list::ListRetainedOwner::fail(control.memory().as_ref());
    }
    result
}

/// Nonadditive source-type observations. Contained values are named for review,
/// not charged twice; stack observations do not claim compiler frame/RSS bounds.
pub(super) fn control_layouts() -> Result<[(&'static str, usize, usize, &'static str); 20]> {
    fn of<T>(
        name: &'static str,
        phase: &'static str,
    ) -> (&'static str, usize, usize, &'static str) {
        (
            name,
            std::mem::size_of::<T>(),
            std::mem::align_of::<T>(),
            phase,
        )
    }
    let c4 = connection::control_layouts()?;
    let values = [
        of::<Work>("Work", "main task capture; contained fields not additive"),
        of::<Pending>("Pending", "main result, fresh capture, physical output"),
        of::<Result<Pending>>("Result<Pending>", "task result; alternative to capture"),
        of::<NativeOwner>("NativeOwner", "contained in Work or Pending"),
        of::<NativeSql<'static>>("NativeSql", "transaction-local stack; Counter included"),
        of::<SqlCallBudget>("SqlCallBudget", "main stack then contained in Pending"),
        of::<Candidate>("Candidate", "main stack before Pending construction"),
        of::<Request>("Request", "contained in its charged Arc allocation"),
        of::<ArcLayout<Request>>("Request Arc", "preadmitted by Pools before Arc::new"),
        of::<OperationControl>(
            "OperationControl",
            "contained in its charged Arc allocation",
        ),
        of::<ArcLayout<OperationControl>>(
            "OperationControl Arc",
            "preadmitted; main through output",
        ),
        of::<Arc<OperationControl>>("progress closure", "boxed hook preadmitted with control"),
        of::<SessionPeer>("SessionPeer", "contained in its charged Arc allocation"),
        of::<ArcLayout<SessionPeer>>("SessionPeer Arc", "preadmitted; TLS session through output"),
        of::<SchemaScratch>("SchemaScratch", "handle contained in Pending"),
        of::<CertificateWorkspace>(
            "CertificateWorkspace",
            "main/import stack; parser allowance separate",
        ),
        (
            "schema Cursor",
            c4.cursor.size(),
            c4.cursor.align(),
            "stack; contained in PageOutcome",
        ),
        (
            "schema PageOutcome",
            c4.page_outcome.size(),
            c4.page_outcome.align(),
            "stack; previous and next page phases",
        ),
        (
            "SchemaVerifier",
            c4.verifier.size(),
            c4.verifier.align(),
            "schema scan stack; Strings separately charged",
        ),
        (
            "schema budget Arc",
            c4.scratch_budget_arc.size(),
            c4.scratch_budget_arc.align(),
            "preadmitted with reusable schema workspace",
        ),
    ];
    if values
        .iter()
        .any(|(_, bytes, alignment, _)| *bytes > MAX_TASK_PAYLOAD || *alignment > 16)
    {
        return Err(Error::Bounds);
    }
    Ok(values)
}

pub(super) fn execute(mut work: Work, clock: &dyn Clock) -> Result<Pending> {
    let memory = work.control.memory().clone();
    if foks_metadata_core::MAX_SQL_CALLS != super::profile::MAX_SQL {
        return Err(Error::Bounds);
    }
    let mut budget = SqlCallBudget::new();
    let result = memory.sync(|| {
        crate::deadline::scope(Some(work.control.deadline()), || {
            let owner = &mut work.owner;
            let control = &work.control;
            control.check()?;
            owner.recheck_path()?;
            control.check()?;
            let scratch = SchemaScratch::new(memory.clone())?;
            let mut workspace = CertificateWorkspace::new(memory.clone(), work.peer.der.len())?;
            install(&owner.connection, control)?;
            begin(&owner.connection, &mut budget, control, true)?;
            let observed = connection::entry(
                &owner.connection,
                &mut budget,
                &owner.profile.contract,
                &scratch,
                control,
            )?;
            state(owner, &mut budget, owner.expected.floor)?;
            let result = candidate(
                owner,
                &mut budget,
                &work.call,
                &work.peer,
                &mut workspace,
                control,
                clock,
            )?;
            control.check()?;
            let floor = result.auth_micros / 1000;
            let changed = NativeSql::new_with_budget(&owner.connection, &mut budget)?.execute(
                Statement::AuthorityClock,
                &[Value::Integer(
                    i64::try_from(floor).map_err(|_| Error::IntegerRange)?,
                )],
            )?;
            if changed != 1 {
                return Err(Error::Storage);
            }
            state(owner, &mut budget, floor)?;
            connection::exit(
                &owner.connection,
                &mut budget,
                &owner.profile.contract,
                &scratch,
                control,
                observed,
            )?;
            #[cfg(test)]
            tests::stage(tests::Stage::MainBeforeCommit, &owner.connection, control)?;
            commit(&owner.connection, &mut budget, control)?;
            owner.expected.floor = floor;
            #[cfg(test)]
            tests::stage(tests::Stage::MainCommitted, &owner.connection, control)?;
            Ok((result, scratch, observed))
        })
    });
    match result {
        Ok((result, scratch, observed)) => Ok(Pending {
            frame: result.frame,
            owner: work.owner,
            call: work.call,
            peer: work.peer,
            control: work.control,
            budget,
            scratch,
            certificate: result.certificate,
            journal: result.journal,
            issued: result.issued,
            observed,
            _execution: work.execution,
        }),
        Err(error) => {
            rollback(&work.owner.connection, &mut budget);
            Err(error)
        }
    }
}

pub(super) fn fresh(pending: &mut Pending, clock: &dyn Clock) -> Result<()> {
    let memory = pending.control.memory().clone();
    let result = memory.sync(|| {
        crate::deadline::scope(Some(pending.control.deadline()), || {
            let owner = &pending.owner;
            let control = &pending.control;
            control.check()?;
            owner.recheck_path()?;
            control.check()?;
            begin(&owner.connection, &mut pending.budget, control, false)?;
            let observed = connection::entry(
                &owner.connection,
                &mut pending.budget,
                &owner.profile.contract,
                &pending.scratch,
                control,
            )?;
            if observed != pending.observed {
                return Err(Error::Storage);
            }
            state(owner, &mut pending.budget, owner.expected.floor)?;
            #[cfg(test)]
            tests::stage(tests::Stage::FreshEntered, &owner.connection, control)?;
            {
                let mut sql = NativeSql::new_with_budget(&owner.connection, &mut pending.budget)?;
                let now = clock.now_micros().map_err(|_| Error::Storage)?;
                if now > i64::MAX as u64
                    || now / 1000 < owner.expected.floor
                    || now
                        >= pending
                            .certificate
                            .native_expiry_micros()
                            .map_err(|_| Error::Storage)?
                {
                    return Err(Error::Expired);
                }
                let issued = native_list::authenticate(
                    &mut sql,
                    &pending.peer.der,
                    pending.certificate.serial(),
                    &pending.certificate.public_key,
                    now,
                )
                .map_err(private_error)?;
                if IssuedStamp::from(&issued)? != pending.issued {
                    return Err(Error::Permission);
                }
                no_sso(&mut sql)?;
                if pending.journal.observes_authority() {
                    let decision =
                        native_list::authority(&mut sql, issued).map_err(private_error)?;
                    if Journal::from(&decision)? != pending.journal {
                        return Err(Error::Permission);
                    }
                }
            }
            state(owner, &mut pending.budget, owner.expected.floor)?;
            connection::exit(
                &owner.connection,
                &mut pending.budget,
                &owner.profile.contract,
                &pending.scratch,
                control,
                observed,
            )?;
            #[cfg(test)]
            tests::stage(tests::Stage::FreshBeforeCommit, &owner.connection, control)?;
            commit(&owner.connection, &mut pending.budget, control)?;
            idle(&owner.connection)?;
            control.check()
        })
    });
    if result.is_err() {
        rollback(&pending.owner.connection, &mut pending.budget);
    }
    result
}
