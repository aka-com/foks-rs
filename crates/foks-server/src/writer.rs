use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use foks_server_db::{Database, DatabasePathIdentity};

use crate::{Error, Result};

trait Task: Send {
    fn run(self: Box<Self>, database: &mut Database);
}

struct Call<F, T> {
    operation: F,
    response: SyncSender<Result<T>>,
    queued_at: Instant,
    timing: Arc<WriterTiming>,
    reconciliation: Option<(
        Arc<crate::ServerMetrics>,
        crate::metrics::realtime::ReconcileSource,
    )>,
}

impl<F, T> Task for Call<F, T>
where
    F: FnOnce(&mut Database) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    fn run(self: Box<Self>, database: &mut Database) {
        let Self {
            operation,
            response,
            queued_at,
            timing,
            reconciliation,
        } = *self;
        let queue_wait = queued_at.elapsed();
        timing.observe_queue_wait(queue_wait);
        let started = Instant::now();
        let result = operation(database);
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
    sender: SyncSender<Message>,
    accepting: Mutex<bool>,
    accepted: AtomicU64,
    rejected: AtomicU64,
    pending: AtomicU64,
    timing: Arc<WriterTiming>,
}

#[derive(Default)]
struct WriterTiming {
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
        let thread = thread::spawn(move || {
            let database = Database::open_with_identity(database_identity, database_config);
            match database {
                Ok(mut database) => {
                    if startup_sender.send(Ok(())).is_ok() {
                        run(&mut database, &receiver);
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
        if self
            .queue
            .sender
            .try_send(Message::Task(Box::new(Call {
                operation,
                response,
                queued_at: Instant::now(),
                timing: Arc::clone(&self.queue.timing),
                reconciliation,
            })))
            .is_err()
        {
            self.queue.pending.fetch_sub(1, Ordering::AcqRel);
            self.queue.rejected.fetch_add(1, Ordering::Relaxed);
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
