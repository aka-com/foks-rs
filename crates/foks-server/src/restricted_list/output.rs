//! Physical request/output ownership for the native qualification listener.
use super::{
    bootstrap::{self, NativeOwner},
    certificate::CertificateFields,
    connection::{Observation, SchemaScratch},
    operation::{self, IssuedStamp, Journal, OperationControl},
    SessionPeer,
};
use crate::{
    net::memory::Pools,
    restricted_storage::{Result, ScanControl, ScanError},
};
use foks_metadata_core::{owned_list::OwnedFrameBytes, SqlCallBudget};
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::{
    io::AsyncReadExt,
    net::TcpListener,
    sync::{oneshot, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
};
use tokio_rustls::TlsAcceptor;
use zeroize::Zeroizing;

pub(super) enum Frame {
    List(OwnedFrameBytes),
    Status(Vec<u8>),
}
impl Frame {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::List(v) => v.as_bytes(),
            Self::Status(v) => v,
        }
    }
}

// Field order keeps buffers/call and the SQLite owner ahead of the original
// request/execution lease on abandonment, including an unpolled detached result.
pub(super) struct Pending {
    pub(super) frame: Frame,
    pub(super) owner: NativeOwner,
    pub(super) call: foks_rpc::DecodedCall,
    pub(super) peer: Arc<SessionPeer>,
    pub(super) control: Arc<OperationControl>,
    pub(super) budget: SqlCallBudget,
    pub(super) scratch: SchemaScratch,
    pub(super) certificate: CertificateFields,
    pub(super) journal: Journal,
    pub(super) issued: IssuedStamp,
    pub(super) observed: Observation,
    pub(super) _execution: OwnedSemaphorePermit,
}
impl Pending {
    pub(super) async fn transmit<W: tokio::io::AsyncWrite + Unpin>(
        self,
        stream: &mut W,
        limits: crate::SessionLimits,
    ) -> Option<NativeOwner> {
        self.control.check().ok()?;
        let _original_sequence = self.call.sequence();
        crate::net::session::write_response_until(
            stream,
            self.frame.bytes(),
            self.control.deadline(),
            limits.io_timeout,
        )
        .await
        .ok()?;
        self.control.check().ok()?;
        // Consume the physical output owner only after write_all plus flush.
        // All remaining request fields drop here; no bare frame leaves this type.
        let Self { owner, .. } = self;
        Some(owner)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeListQualificationReport {
    pub profile_digest: [u8; 32],
    pub compiled_profile: [u8; 400],
    pub source_digest: [u8; 32],
    pub target_incarnation: [u8; 32],
    pub target_revision: u64,
    pub sqlite_version: &'static str,
    pub connection_program_names: [&'static str; 8],
    pub shared_connection_programs: usize,
    pub schema_workspace_bytes: usize,
    pub schema_control_bytes: usize,
    pub schema_verifier_bytes: usize,
    pub certificate_workspace_bytes: usize,
    pub maximum_task_payload_bytes: usize,
    /// Source-type size/alignment and live phase; contained terms are not summed.
    pub control_layouts: [(&'static str, usize, usize, &'static str); 20],
    pub accepted_connections: u64,
    pub completed_requests: u64,
    pub rejected_requests: u64,
    pub maximum_sql_charged: u64,
    pub maximum_request_micros: u64,
    pub stopped_after_failure: bool,
}
#[derive(Default)]
struct Statistics {
    connections: AtomicU64,
    completed: AtomicU64,
    rejected: AtomicU64,
    sql: AtomicU64,
    micros: AtomicU64,
    failed: AtomicBool,
}
/// Disposable loopback qualification service. It serves only admitted personal
/// List requests through real mutual TLS and its one imported physical owner.
/// This is not wired into ordinary listeners or an HA/recovery qualification.
pub struct NativeListQualification {
    address: SocketAddr,
    canonical_name: String,
    report: NativeListQualificationReport,
    statistics: Arc<Statistics>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}
impl NativeListQualification {
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn canonical_name(&self) -> &str {
        &self.canonical_name
    }
    pub fn report(&self) -> NativeListQualificationReport {
        let mut r = self.report;
        r.accepted_connections = self.statistics.connections.load(Ordering::Acquire);
        r.completed_requests = self.statistics.completed.load(Ordering::Acquire);
        r.rejected_requests = self.statistics.rejected.load(Ordering::Acquire);
        r.maximum_sql_charged = self.statistics.sql.load(Ordering::Acquire);
        r.maximum_request_micros = self.statistics.micros.load(Ordering::Acquire);
        r.stopped_after_failure = self.statistics.failed.load(Ordering::Acquire);
        r
    }
    /// Stops the listener and its async waiter. A detached blocking operation
    /// retains its imported database and request until that operation finishes;
    /// this return value is not evidence that such work has already terminated.
    pub async fn shutdown(mut self) -> NativeListQualificationReport {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        self.report()
    }
}
impl Drop for NativeListQualification {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        // Dropping/aborting an async waiter cannot free resources owned by a
        // running blocking operation. Tokio drops its whole result on completion.
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Captures/imports an actual finalized backup under the caller's original
/// finite maintenance job, then starts a loopback mutual-TLS qualification service.
/// `limits` controls local admission; it cannot change the compiled data profile.
pub async fn start_native_personal_list_qualification(
    artifacts: &crate::BackupArtifacts,
    root_key: Zeroizing<[u8; 32]>,
    config: foks_server_db::Config,
    control: &ScanControl,
    limits: crate::SessionLimits,
) -> Result<NativeListQualification> {
    limits
        .validate()
        .map_err(|_| ScanError::Unsupported("qualification session limits"))?;
    control.check()?;
    let artifacts = artifacts.clone();
    let job = control.clone();
    let pools = Pools::new(limits);
    let construction_pools = pools.clone();
    let (owner, tls) = tokio::task::spawn_blocking(move || {
        let mut scanner = crate::restricted_storage::RestrictedScanner::capture_finalized_backup(
            &artifacts,
            crate::restricted_storage::CaptureLimits {
                maximum_database_bytes: config
                    .maximum_database_bytes
                    .min(super::super::super::MAX_DATABASE),
            },
            &job,
        )?;
        while !scanner.advance(&job)?.complete {}
        let source = scanner.finish(&job)?;
        let imported = super::super::import_personal_list(source, root_key, config, &job)?;
        bootstrap::consume(imported, &job, &construction_pools)
    })
    .await
    .map_err(|_| ScanError::Incomplete)??;
    // A cancelled/expired original job cannot acquire a listener after construction.
    control.check()?;
    let qualification = owner.qualification();
    let terms = SchemaScratch::terms().map_err(|_| ScanError::Allocation)?;
    let report = NativeListQualificationReport {
        profile_digest: owner.profile.digest,
        compiled_profile: owner.profile.record,
        source_digest: qualification.source_digest,
        target_incarnation: qualification.target_incarnation,
        target_revision: qualification.target_revision,
        sqlite_version: rusqlite::version(),
        connection_program_names: std::array::from_fn(|i| super::connection::FIXED_CHECKS[i].name),
        shared_connection_programs: super::connection::FIXED_CHECKS
            .iter()
            .filter(|check| !check.native_only)
            .count(),
        schema_workspace_bytes: terms.workspace_bytes,
        schema_control_bytes: terms.control_bytes,
        schema_verifier_bytes: terms.verifier_bytes,
        certificate_workspace_bytes: super::certificate::allowance(super::certificate::MAX_DER)
            .ok_or(ScanError::Allocation)?,
        maximum_task_payload_bytes: operation::MAX_TASK_PAYLOAD,
        control_layouts: operation::control_layouts().map_err(|_| ScanError::Allocation)?,
        accepted_connections: 0,
        completed_requests: 0,
        rejected_requests: 0,
        maximum_sql_charged: 0,
        maximum_request_micros: 0,
        stopped_after_failure: false,
    };
    let canonical_name = owner.canonical_name().to_owned();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let statistics = Arc::new(Statistics::default());
    let task_statistics = statistics.clone();
    let (stop, mut stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        // One exclusive imported SQLite owner. Parallel capacity is deliberately
        // a later multi-tenant harness concern, not hidden behind a second copy.
        let run = serve(
            listener,
            owner,
            TlsAcceptor::from(tls.authenticated),
            pools,
            limits,
            task_statistics,
        );
        tokio::pin!(run);
        tokio::select! {_=&mut stopped=>{},_=&mut run=>{}}
    });
    Ok(NativeListQualification {
        address,
        canonical_name,
        report,
        statistics,
        stop: Some(stop),
        task: Some(task),
    })
}

async fn serve(
    listener: TcpListener,
    mut owner: NativeOwner,
    acceptor: TlsAcceptor,
    pools: Arc<Pools>,
    limits: crate::SessionLimits,
    stats: Arc<Statistics>,
) {
    let input = Arc::new(Semaphore::new(limits.maximum_request_memory_bytes));
    let execution = Arc::new(Semaphore::new(1));
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        stats.connections.fetch_add(1, Ordering::AcqRel);
        let stream = tokio::time::timeout(limits.io_timeout, acceptor.accept(socket)).await;
        let Ok(Ok(mut stream)) = stream else {
            stats.rejected.fetch_add(1, Ordering::AcqRel);
            continue;
        };
        let peer = match SessionPeer::from_tls(&stream, &pools) {
            Ok(peer) => peer,
            Err(_) => {
                stats.rejected.fetch_add(1, Ordering::AcqRel);
                continue;
            }
        };
        for _ in 0..limits.maximum_requests {
            let ingress = tokio::time::timeout(
                limits.io_timeout,
                crate::net::session::read_frame_ingress(
                    &mut stream,
                    limits.maximum_frame_bytes,
                    limits.request_timeout,
                ),
            )
            .await;
            let Ok(Ok((length, deadline))) = ingress else {
                break;
            };
            let started = deadline
                .instant()
                .checked_sub(limits.request_timeout)
                .unwrap_or_else(Instant::now);
            let Some(input_bytes) = length.checked_mul(2).and_then(|n| u32::try_from(n).ok())
            else {
                break;
            };
            let Ok(input_permit) = input.clone().try_acquire_many_owned(input_bytes) else {
                break;
            };
            let Ok(memory) = pools.admitted_request(Some(input_permit)) else {
                break;
            };
            memory.classify_input(length);
            // Input and the decoded argument each fit one frame length. Decode
            // metadata/Value allocations additionally use this original owner.
            let mut bytes = Vec::new();
            if bytes.try_reserve_exact(length).is_err() {
                break;
            }
            bytes.resize(length, 0);
            let read =
                tokio::time::timeout_at(deadline.instant().into(), stream.read_exact(&mut bytes))
                    .await;
            if !matches!(read, Ok(Ok(_))) {
                break;
            }
            if !foks_snowpack::AllocationBudget::reserve(
                memory.as_ref(),
                operation::ingress_diagnostic_bytes(),
            ) {
                break;
            }
            let message = memory.sync(|| foks_rpc::decode_message(&bytes));
            let call = match message {
                Ok(foks_rpc::InboundMessage::Control) => continue,
                Ok(foks_rpc::InboundMessage::Call(call)) => call,
                Err(_) => {
                    stats.rejected.fetch_add(1, Ordering::AcqRel);
                    break;
                }
            };
            drop(bytes);
            // Keep ingress ownership alive if control admission fails while the
            // decoded call still owns its argument allocation.
            let Ok(control) = OperationControl::new(deadline, memory.clone()) else {
                break;
            };
            let Ok(permit) = execution.clone().try_acquire_owned() else {
                break;
            };
            let work = operation::Work {
                owner,
                call,
                peer: peer.clone(),
                control: control.clone(),
                execution: permit,
            };
            let task = move || operation::execute(work, &foks_server_db::SystemClock);
            if operation::admit_task(&control, &task).is_err() {
                stats.rejected.fetch_add(1, Ordering::AcqRel);
                stats.failed.store(true, Ordering::Release);
                return;
            }
            let mut work = tokio::task::spawn_blocking(task);
            let outcome = tokio::time::timeout_at(deadline.instant().into(), &mut work).await;
            let mut pending = match outcome {
                Ok(Ok(Ok(pending))) => pending,
                _ => {
                    control.cancel();
                    stats.rejected.fetch_add(1, Ordering::AcqRel);
                    stats.failed.store(true, Ordering::Release);
                    return;
                }
            };
            drop(work);
            let task = move || {
                operation::fresh(&mut pending, &foks_server_db::SystemClock)?;
                Ok::<_, foks_metadata_core::Error>(pending)
            };
            if operation::admit_task(&control, &task).is_err() {
                stats.rejected.fetch_add(1, Ordering::AcqRel);
                stats.failed.store(true, Ordering::Release);
                return;
            }
            let mut fresh = tokio::task::spawn_blocking(task);
            let result = tokio::time::timeout_at(deadline.instant().into(), &mut fresh).await;
            let pending = match result {
                Ok(Ok(Ok(pending))) => pending,
                _ => {
                    control.cancel();
                    stats.rejected.fetch_add(1, Ordering::AcqRel);
                    stats.failed.store(true, Ordering::Release);
                    return;
                }
            };
            drop(fresh);
            stats
                .sql
                .fetch_max(pending.budget.charged() as u64, Ordering::AcqRel);
            let Some(returned) = pending.transmit(&mut stream, limits).await else {
                stats.rejected.fetch_add(1, Ordering::AcqRel);
                stats.failed.store(true, Ordering::Release);
                return;
            };
            owner = returned;
            stats.completed.fetch_add(1, Ordering::AcqRel);
            stats.micros.fetch_max(
                u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
                Ordering::AcqRel,
            );
        }
    }
}
