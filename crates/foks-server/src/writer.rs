use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use foks_server_db::{Database, DatabasePathIdentity};

use crate::fatal::{CoreThread, FatalSignal};
use crate::{Error, Result};

trait Task: Send {
    fn run(self: Box<Self>, database: &mut Database);
}

struct Call<F, T> {
    operation: F,
    response: SyncSender<Result<T>>,
    queued_at: Instant,
    /// The submitting request's deadline, taken from its ambient scope.
    deadline: Option<crate::deadline::Deadline>,
    timing: Arc<WriterTiming>,
    reconciliation: Option<(
        Arc<crate::ServerMetrics>,
        crate::metrics::realtime::ReconcileSource,
    )>,
    // Drop captured buffers before returning their allocation allowance.
    resources: Option<Arc<crate::net::memory::Request>>,
}

impl<F, T> Task for Call<F, T>
where
    F: FnOnce(&mut Database) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    fn run(self: Box<Self>, database: &mut Database) {
        let Self {
            resources,
            operation,
            response,
            queued_at,
            deadline,
            timing,
            reconciliation,
        } = *self;
        let queue_wait = queued_at.elapsed();
        timing.observe_queue_wait(queue_wait);
        // The request that submitted this job has used its execution budget,
        // and its session closes the connection without a response, so the
        // client already treats the outcome as ambiguous. The job has not
        // opened a transaction, and skipping it leaves no partial effect.
        if deadline.is_some_and(|deadline| deadline.remaining().is_err()) {
            timing.expired.fetch_add(1, Ordering::Relaxed);
            let _ = response.send(Err(crate::deadline::expired()));
            return;
        }
        let started = Instant::now();
        let result = crate::deadline::scope(deadline, || {
            if let Some(resources) = resources.as_ref() {
                resources.sync(|| operation(database))
            } else {
                operation(database)
            }
        });
        let execution = started.elapsed();
        timing.observe_execution(execution);
        if let Some((metrics, source)) = reconciliation {
            metrics
                .realtime
                .execution(source, queue_wait, execution, result.is_err());
        }
        let _ = response.send(result);
    }
}

enum Message {
    Task(Box<dyn Task>),
    Shutdown,
}

pub struct Writer {
    handle: WriterHandle,
    thread: Option<JoinHandle<()>>,
    _process_lock: std::fs::File,
}

#[derive(Clone)]
pub struct WriterHandle {
    authorization: Option<Arc<WriteAuthorization>>,
    queue: Arc<WriterQueue>,
}

struct WriteAuthorization {
    uid: Vec<u8>,
    credential: Vec<u8>,
    clock: Arc<dyn foks_server_db::Clock>,
}

struct WriterQueue {
    database_identity: DatabasePathIdentity,
    sender: SyncSender<Message>,
    accepting: Mutex<bool>,
    accepted: AtomicU64,
    rejected: AtomicU64,
    pending: AtomicU64,
    timing: Arc<WriterTiming>,
}

#[derive(Default)]
struct WriterTiming {
    expired: AtomicU64,
    queue_wait_observations: AtomicU64,
    queue_wait_microseconds_total: AtomicU64,
    queue_wait_microseconds_max: AtomicU64,
    execution_observations: AtomicU64,
    execution_microseconds_total: AtomicU64,
    execution_microseconds_max: AtomicU64,
}

impl WriterTiming {
    fn observe_queue_wait(&self, duration: std::time::Duration) {
        observe_duration(
            duration,
            &self.queue_wait_observations,
            &self.queue_wait_microseconds_total,
            &self.queue_wait_microseconds_max,
        );
    }

    fn observe_execution(&self, duration: std::time::Duration) {
        observe_duration(
            duration,
            &self.execution_observations,
            &self.execution_microseconds_total,
            &self.execution_microseconds_max,
        );
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriterMetrics {
    pub accepted: u64,
    pub rejected: u64,
    pub pending: u64,
    /// Jobs skipped because their request deadline passed before they
    /// started, either before submission or while they waited in the queue.
    pub expired: u64,
    pub queue_wait_observations: u64,
    pub queue_wait_microseconds_total: u64,
    pub queue_wait_microseconds_max: u64,
    pub execution_observations: u64,
    pub execution_microseconds_total: u64,
    pub execution_microseconds_max: u64,
}

impl Writer {
    pub fn start(
        database_path: PathBuf,
        database_config: foks_server_db::Config,
        maximum_pending: usize,
    ) -> Result<Self> {
        Self::start_monitored(
            database_path,
            database_config,
            maximum_pending,
            FatalSignal::default(),
        )
    }

    /// Starts the writer thread and reports a panic in any queued operation
    /// to `fatal`. The thread does not continue after such a panic: unwinding
    /// drops the operation's open rusqlite transaction, which rolls it back,
    /// then the connection and the queue receiver, so queued and later calls
    /// fail with `Error::WriterQueue`.
    pub(crate) fn start_monitored(
        database_path: PathBuf,
        database_config: foks_server_db::Config,
        maximum_pending: usize,
        fatal: FatalSignal,
    ) -> Result<Self> {
        if maximum_pending == 0 {
            return Err(Error::Config("zero writer queue limit"));
        }
        let guard = DatabaseWriterGuard::acquire(&database_path)?;
        let DatabaseWriterGuard {
            database_identity,
            process_lock,
        } = guard;
        let (sender, receiver) = mpsc::sync_channel(maximum_pending);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let writer_identity = database_identity.clone();
        let thread = thread::spawn(move || {
            let database = Database::open_with_identity(database_identity, database_config);
            match database {
                Ok(mut database) => {
                    if startup_sender.send(Ok(())).is_ok() {
                        let exit = fatal.exit_guard(CoreThread::Writer);
                        run(&mut database, &receiver);
                        exit.end(true);
                    }
                }
                Err(error) => {
                    let _ = startup_sender.send(Err(error));
                }
            }
        });
        match startup_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                handle: WriterHandle {
                    authorization: None,
                    queue: Arc::new(WriterQueue {
                        database_identity: writer_identity,
                        sender,
                        accepting: Mutex::new(true),
                        accepted: AtomicU64::new(0),
                        rejected: AtomicU64::new(0),
                        pending: AtomicU64::new(0),
                        timing: Arc::new(WriterTiming::default()),
                    }),
                },
                thread: Some(thread),
                _process_lock: process_lock,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error.into())
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::Thread)
            }
        }
    }

    pub fn call<F, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce(&mut Database) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.handle.call(operation)
    }

    pub fn handle(&self) -> WriterHandle {
        self.handle.clone()
    }

    pub fn shutdown(mut self) -> Result<()> {
        self.stop()
    }

    fn stop(&mut self) -> Result<()> {
        let mut accepting = self
            .handle
            .queue
            .accepting
            .lock()
            .map_err(|_| Error::Thread)?;
        if *accepting {
            *accepting = false;
            let _ = self.handle.queue.sender.send(Message::Shutdown);
        }
        drop(accepting);
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| Error::Thread)?;
        }
        Ok(())
    }
}

impl WriterHandle {
    pub(crate) fn database_identity(&self) -> &DatabasePathIdentity {
        &self.queue.database_identity
    }

    // These mutations only account an already admitted physical side effect.
    // Revocation must not make published bytes disappear from the capacity ledger.
    pub(crate) fn settle_chunk_publication(
        &self,
        token: foks_server_db::ChunkReservation,
        success: bool,
    ) -> Result<()> {
        let accounting = Self {
            queue: Arc::clone(&self.queue),
            authorization: None,
        };
        // This is completion of already admitted work, not a new mutation.
        crate::deadline::scope(None, || {
            accounting.call(move |db| {
                db.settle_chunk_publication(&token, success)?;
                Ok(())
            })
        })
    }
    pub(crate) fn confirm_chunk_presence(
        &self,
        token: foks_server_db::ChunkReservation,
    ) -> Result<()> {
        let accounting = Self {
            queue: Arc::clone(&self.queue),
            authorization: None,
        };
        crate::deadline::scope(None, || {
            accounting.call(move |db| {
                db.confirm_chunk_presence(&token)?;
                Ok(())
            })
        })
    }

    pub(crate) fn for_authenticated_request(
        &self,
        uid: &[u8],
        credential: &[u8],
        clock: Arc<dyn foks_server_db::Clock>,
    ) -> Self {
        Self {
            queue: self.queue.clone(),
            authorization: Some(Arc::new(WriteAuthorization {
                uid: uid.to_vec(),
                credential: credential.to_vec(),
                clock,
            })),
        }
    }

    pub fn call<F, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce(&mut Database) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.call_observed(None, operation)
    }

    pub(crate) fn call_observed<F, T>(
        &self,
        reconciliation: Option<(
            Arc<crate::ServerMetrics>,
            crate::metrics::realtime::ReconcileSource,
        )>,
        operation: F,
    ) -> Result<T>
    where
        F: FnOnce(&mut Database) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        // A call takes the deadline of the request scope it runs in, if any.
        // The writer skips the call with `Error::DeadlineExceeded`, without
        // running its operation, if it is submitted at or after the deadline
        // or the writer reaches it then. The session closes the connection of
        // an expired request without a response, so the client already treats
        // the outcome as ambiguous, and a skipped call has opened no
        // transaction. An operation that has started runs to completion. A
        // write that must follow an already committed step regardless of the
        // request deadline runs under `crate::deadline::scope(None, ..)`, as
        // the chunk settlement calls below and the SSO access refresh do.
        let deadline = crate::deadline::current();
        if deadline.is_some_and(|deadline| deadline.remaining().is_err()) {
            self.queue.timing.expired.fetch_add(1, Ordering::Relaxed);
            return Err(crate::deadline::expired());
        }
        let authorization = self.authorization.clone();
        let operation = move |database: &mut Database| {
            if let Some(auth) = authorization {
                if database
                    .active_credential_owner(&auth.uid, &auth.credential)?
                    .is_none()
                {
                    return Err(Error::AuthorizationChanged);
                }
                database
                    .sso_require_access(&auth.uid, auth.clock.now_micros()? / 1000)
                    .map_err(|e| match e {
                        foks_server_db::Error::AuthorizationChanged => {
                            Error::Sso("reauthentication required before queued write")
                        }
                        other => Error::Database(other),
                    })?;
            }
            operation(database)
        };
        let (response, receiver) = mpsc::sync_channel(1);
        let accepting = self
            .queue
            .accepting
            .lock()
            .map_err(|_| Error::WriterQueue)?;
        if !*accepting {
            return Err(Error::WriterQueue);
        }
        self.queue.pending.fetch_add(1, Ordering::AcqRel);
        if let Err(error) = self.queue.sender.try_send(Message::Task(Box::new(Call {
            resources: crate::net::memory::current(),
            operation,
            response,
            queued_at: Instant::now(),
            deadline,
            timing: Arc::clone(&self.queue.timing),
            reconciliation,
        }))) {
            self.queue.pending.fetch_sub(1, Ordering::AcqRel);
            // A disconnected queue means the writer thread has exited, which
            // is reported as a fatal thread exit rather than as saturation.
            if matches!(error, mpsc::TrySendError::Full(_)) {
                self.queue.rejected.fetch_add(1, Ordering::Relaxed);
            }
            return Err(Error::WriterQueue);
        }
        self.queue.accepted.fetch_add(1, Ordering::Relaxed);
        drop(accepting);
        let result = receiver.recv();
        self.queue.pending.fetch_sub(1, Ordering::AcqRel);
        result.map_err(|_| Error::WriterQueue)?
    }

    pub(crate) fn call_with_current_time<F, T>(
        &self,
        clock: Arc<dyn foks_server_db::Clock>,
        operation: F,
    ) -> Result<T>
    where
        F: FnOnce(&mut Database, u64) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.call(move |database| {
            let now = clock.now_micros()?;
            operation(database, now)
        })
    }

    pub fn metrics(&self) -> WriterMetrics {
        let timing = &self.queue.timing;
        WriterMetrics {
            accepted: self.queue.accepted.load(Ordering::Relaxed),
            rejected: self.queue.rejected.load(Ordering::Relaxed),
            pending: self.queue.pending.load(Ordering::Acquire),
            expired: timing.expired.load(Ordering::Relaxed),
            queue_wait_observations: timing.queue_wait_observations.load(Ordering::Relaxed),
            queue_wait_microseconds_total: timing
                .queue_wait_microseconds_total
                .load(Ordering::Relaxed),
            queue_wait_microseconds_max: timing.queue_wait_microseconds_max.load(Ordering::Relaxed),
            execution_observations: timing.execution_observations.load(Ordering::Relaxed),
            execution_microseconds_total: timing
                .execution_microseconds_total
                .load(Ordering::Relaxed),
            execution_microseconds_max: timing.execution_microseconds_max.load(Ordering::Relaxed),
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn run(database: &mut Database, receiver: &Receiver<Message>) {
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Task(task) => task.run(database),
            Message::Shutdown => break,
        }
    }
}

fn observe_duration(
    duration: std::time::Duration,
    observations: &AtomicU64,
    total: &AtomicU64,
    maximum: &AtomicU64,
) {
    let microseconds = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
    observations.fetch_add(1, Ordering::Relaxed);
    total.fetch_add(microseconds, Ordering::Relaxed);
    maximum.fetch_max(microseconds, Ordering::Relaxed);
}

/// Shared exclusive mutation guard for the server and offline operator commands.
/// The canonical path and no-follow lock handling are identical for both callers.
pub struct DatabaseWriterGuard {
    database_identity: DatabasePathIdentity,
    process_lock: std::fs::File,
}
impl DatabaseWriterGuard {
    pub fn acquire(database_path: &std::path::Path) -> Result<Self> {
        let database_identity = DatabasePathIdentity::prepare(database_path)?;
        let database_path = database_identity.path().to_path_buf();
        let mut lock_path = database_path.as_os_str().to_os_string();
        lock_path.push(".writer-lock");
        let mut lock_options = std::fs::OpenOptions::new();
        lock_options
            .read(true)
            .write(true)
            .create(true)
            .truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            lock_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let process_lock = lock_options.open(std::path::PathBuf::from(lock_path))?;
        process_lock
            .try_lock()
            .map_err(|_| Error::Config("database writer is already active"))?;
        database_identity.recheck()?;
        Ok(Self {
            database_identity,
            process_lock,
        })
    }
    pub fn open_database(&self, config: foks_server_db::Config) -> Result<GuardedDatabase<'_>> {
        self.database_identity.recheck()?;
        let db = Database::open(self.database_identity.path(), config)?;
        self.database_identity.recheck()?;
        Ok(GuardedDatabase {
            database: db,
            _guard: self,
        })
    }
}

/// A writable database cannot outlive its process-level exclusion guard.
pub struct GuardedDatabase<'a> {
    database: Database,
    _guard: &'a DatabaseWriterGuard,
}
impl std::ops::Deref for GuardedDatabase<'_> {
    type Target = Database;
    fn deref(&self) -> &Database {
        &self.database
    }
}
impl std::ops::DerefMut for GuardedDatabase<'_> {
    fn deref_mut(&mut self) -> &mut Database {
        &mut self.database
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_queued_task_cannot_mutate_but_cleanup_can_complete() {
        let temporary = tempfile::tempdir().unwrap();
        let mut db = Database::open(temporary.path().join("db"), Default::default()).unwrap();
        let deadline = crate::deadline::Deadline::after(std::time::Duration::ZERO).unwrap();
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let executed = Arc::clone(&ran);
        let (response, receiver) = mpsc::sync_channel(1);
        // Inject a task at the dequeue boundary with an already consumed budget;
        // no sleeps or scheduler timing are needed to exercise the real worker.
        let task = Call {
            operation: move |_: &mut Database| {
                executed.store(true, Ordering::Release);
                Ok(())
            },
            response,
            queued_at: Instant::now(),
            deadline: Some(deadline),
            timing: Arc::new(WriterTiming::default()),
            reconciliation: None,
            resources: None,
        };
        Box::new(task).run(&mut db);
        assert!(matches!(
            receiver.recv().unwrap(),
            Err(Error::DeadlineExceeded)
        ));
        assert!(!ran.load(Ordering::Acquire));
        let writer_path = temporary.path().join("writer");
        let mut accounting_db = Database::open(&writer_path, Default::default()).unwrap();
        let connection = rusqlite::Connection::open(&writer_path).unwrap();
        connection
            .execute(
                "INSERT INTO host_metadata VALUES(1,?1,'test',X'',X'')",
                [[4u8; 33].as_slice()],
            )
            .unwrap();
        drop(connection);
        let binding = accounting_db
            .prepare_chunk_storage_binding(&[5; 16], "test", false)
            .unwrap();
        accounting_db
            .activate_chunk_storage_binding(&binding)
            .unwrap();
        accounting_db.confirm_empty_chunk_inventory().unwrap();
        let token = accounting_db
            .reserve_chunk_object(&[1; 32], 10, &[2; 16], 1)
            .unwrap();
        drop(accounting_db);
        let writer = Writer::start(writer_path, Default::default(), 2).unwrap();
        crate::deadline::scope(Some(deadline), || {
            assert!(writer.call(|_| Ok(())).is_err());
            writer
                .handle()
                .settle_chunk_publication(token, true)
                .unwrap();
            assert!(crate::deadline::check().is_err());
        });
        let usage = writer.call(|db| Ok(db.chunk_physical_usage()?)).unwrap();
        assert_eq!(usage.present_bytes, 10);
        assert_eq!(usage.unresolved_attempts, 0);
    }

    #[test]
    fn schema_upgrade_waits_for_exclusive_writer_ownership() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("upgrade.sqlite");
        drop(Database::open(&path, Default::default()).unwrap());
        let connection = rusqlite::Connection::open(&path).unwrap();
        for index in [
            "names_reservation_expiry",
            "team_names_reservation_expiry",
            "recovery_challenges_cleanup",
            "team_view_tokens_expiry",
            "team_view_challenges_expiry",
            "team_admin_tokens_expiry",
            "log_sends_created_at",
        ] {
            connection
                .execute_batch(&format!("DROP INDEX {index}"))
                .unwrap();
        }
        connection
            .execute_batch(concat!(
                include_str!("../../foks-server-db/tests/fixtures/remove_waitlist_count.sql"),
                include_str!("../../foks-server-db/tests/fixtures/remove_read_dependency.sql"),
                include_str!("../../foks-server-db/tests/fixtures/remove_kv_usage.sql"),
                include_str!("../../foks-server-db/tests/fixtures/remove_kv_current.sql")
            ))
            .unwrap();
        connection
            .execute_batch(include_str!(
                "../../foks-server-db/tests/fixtures/remove_external_kv_chunks.sql"
            ))
            .unwrap();
        connection
            .execute_batch(include_str!(
                "../../foks-server-db/tests/fixtures/remove_kv_accounting.sql"
            ))
            .unwrap();
        connection.execute_batch("DROP TRIGGER rt_membership_insert; DROP TRIGGER rt_membership_delete; DROP TRIGGER rt_membership_update; DROP TRIGGER rt_team_access_update; ALTER TABLE rt_user_inboxes DROP COLUMN reconcile_dirty; PRAGMA user_version=43;").unwrap();
        let guard = DatabaseWriterGuard::acquire(&path).unwrap();
        assert!(Writer::start(path.clone(), Default::default(), 2).is_err());
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            43
        );
        drop(guard);
        let writer = Writer::start(path.clone(), Default::default(), 2).unwrap();
        assert!(foks_server_db::ReadDatabase::open(&path, Default::default()).is_ok());
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            foks_server_db::SCHEMA_VERSION
        );
        drop(writer);
    }

    struct TestClock(AtomicU64);

    impl foks_server_db::Clock for TestClock {
        fn now_micros(&self) -> foks_server_db::Result<u64> {
            Ok(self.0.load(Ordering::Acquire))
        }
    }

    #[test]
    fn offline_guard_and_running_writer_exclude_each_other() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("server.sqlite");
        {
            let guard = DatabaseWriterGuard::acquire(&path).unwrap();
            let _database = guard
                .open_database(foks_server_db::Config::default())
                .unwrap();
            assert!(Writer::start(path.clone(), foks_server_db::Config::default(), 2).is_err());
            assert!(DatabaseWriterGuard::acquire(&path).is_err());
        }
        let writer = Writer::start(path.clone(), foks_server_db::Config::default(), 2).unwrap();
        assert!(DatabaseWriterGuard::acquire(&path).is_err());
        writer.shutdown().unwrap();
        assert!(DatabaseWriterGuard::acquire(&path).is_ok());
    }

    #[test]
    fn panicking_operation_stops_the_writer_and_reports_it() {
        let temporary = tempfile::tempdir().unwrap();
        let fatal = FatalSignal::default();
        let writer = Writer::start_monitored(
            temporary.path().join("foks-server.sqlite"),
            foks_server_db::Config::default(),
            2,
            fatal.clone(),
        )
        .unwrap();
        writer
            .call(|database| {
                database.reserve_name(b"committed", &[1; 17], 1, 1, 10_000_000)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(fatal.exited(), None);
        let panicked = writer.call::<_, ()>(|_| panic!("injected writer panic"));
        assert!(matches!(panicked, Err(Error::WriterQueue)));
        // The response channel closes while the thread unwinds, before the
        // guard in its outer frame reports the exit.
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while fatal.exited().is_none() {
            assert!(Instant::now() < deadline, "writer panic was not reported");
            thread::yield_now();
        }
        assert_eq!(fatal.exited(), Some(CoreThread::Writer));
        // A later call either finds the queue disconnected or, if it was
        // queued before the unwinding thread dropped its receiver, is dropped
        // unanswered. Neither runs the operation.
        assert!(matches!(
            writer.call(|_| -> Result<()> { unreachable!("writer ran after a panic") }),
            Err(Error::WriterQueue)
        ));
        assert_eq!(writer.handle().metrics().rejected, 0);
        assert!(matches!(writer.shutdown(), Err(Error::Thread)));
    }

    #[test]
    fn queued_call_past_its_deadline_is_skipped() {
        let temporary = tempfile::tempdir().unwrap();
        let writer = Arc::new(
            Writer::start(
                temporary.path().join("foks-server.sqlite"),
                foks_server_db::Config::default(),
                4,
            )
            .unwrap(),
        );
        let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
        let (release_sender, release_receiver) = mpsc::sync_channel(1);
        let blocker_writer = Arc::clone(&writer);
        let blocker = thread::spawn(move || {
            blocker_writer.call(move |_| {
                entered_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
                Ok(())
            })
        });
        entered_receiver.recv().unwrap();

        // The credential is unknown, so the authorization check would fail
        // with AuthorizationChanged if the expired job reached it.
        let deadline =
            crate::deadline::Deadline::after(std::time::Duration::from_millis(50)).unwrap();
        let clock: Arc<dyn foks_server_db::Clock> = Arc::new(TestClock(AtomicU64::new(0)));
        let expiring = writer
            .handle()
            .for_authenticated_request(&[1; 33], &[2; 33], clock);
        let expired = thread::spawn(move || {
            crate::deadline::scope(Some(deadline), || {
                expiring.call(|_| -> Result<()> { unreachable!("expired job ran") })
            })
        });
        let current = writer.handle();
        let live_deadline =
            crate::deadline::Deadline::after(std::time::Duration::from_secs(60)).unwrap();
        let live = thread::spawn(move || {
            crate::deadline::scope(Some(live_deadline), || current.call(|_| Ok(7)))
        });
        let queued = Instant::now() + std::time::Duration::from_secs(2);
        while writer.handle().metrics().pending != 3 {
            assert!(Instant::now() < queued, "jobs were not queued");
            thread::yield_now();
        }
        thread::sleep(deadline.instant().saturating_duration_since(Instant::now()));
        release_sender.send(()).unwrap();

        blocker.join().unwrap().unwrap();
        assert!(matches!(
            expired.join().unwrap(),
            Err(Error::DeadlineExceeded)
        ));
        assert_eq!(live.join().unwrap().unwrap(), 7);
        let metrics = writer.handle().metrics();
        assert_eq!(metrics.expired, 1);
        assert_eq!(metrics.pending, 0);
        assert_eq!(metrics.queue_wait_observations, 3);
        assert_eq!(metrics.execution_observations, 2);
        Arc::into_inner(writer).unwrap().shutdown().unwrap();
    }

    #[test]
    fn call_submitted_after_its_deadline_is_not_queued() {
        let temporary = tempfile::tempdir().unwrap();
        let writer = Writer::start(
            temporary.path().join("foks-server.sqlite"),
            foks_server_db::Config::default(),
            4,
        )
        .unwrap();
        let expired = crate::deadline::Deadline::at(Instant::now());
        assert!(matches!(
            crate::deadline::scope(Some(expired), || {
                writer.call(|_| -> Result<()> { unreachable!("expired job ran") })
            }),
            Err(Error::DeadlineExceeded)
        ));
        let metrics = writer.handle().metrics();
        assert_eq!(metrics.expired, 1);
        assert_eq!(metrics.accepted, 0);
        assert_eq!(metrics.rejected, 0);
        assert_eq!(metrics.pending, 0);
        assert_eq!(metrics.queue_wait_observations, 0);
        writer.shutdown().unwrap();
    }

    #[test]
    fn current_time_is_sampled_after_a_queued_task_starts() {
        let temporary = tempfile::tempdir().unwrap();
        let writer = Arc::new(
            Writer::start(
                temporary.path().join("foks-server.sqlite"),
                foks_server_db::Config::default(),
                2,
            )
            .unwrap(),
        );
        let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
        let (release_sender, release_receiver) = mpsc::sync_channel(1);
        let blocker_writer = Arc::clone(&writer);
        let blocker = thread::spawn(move || {
            blocker_writer.call(move |_| {
                entered_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
                Ok(())
            })
        });
        entered_receiver.recv().unwrap();

        let clock = Arc::new(TestClock(AtomicU64::new(10)));
        let timed_clock: Arc<dyn foks_server_db::Clock> = clock.clone();
        let timed_writer = writer.handle();
        let timed = thread::spawn(move || {
            timed_writer.call_with_current_time(timed_clock, |_, now| Ok(now))
        });
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while writer.handle().metrics().pending != 2 {
            assert!(Instant::now() < deadline, "timed task was not queued");
            thread::yield_now();
        }
        clock.0.store(20, Ordering::Release);
        release_sender.send(()).unwrap();

        blocker.join().unwrap().unwrap();
        assert_eq!(timed.join().unwrap().unwrap(), 20);
        Arc::into_inner(writer).unwrap().shutdown().unwrap();
    }
}
