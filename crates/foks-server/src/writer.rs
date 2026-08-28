use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use foks_server_db::Database;

use crate::{Error, Result};

trait Task: Send {
    fn run(self: Box<Self>, database: &mut Database);
}

struct Call<F, T> {
    operation: F,
    response: SyncSender<Result<T>>,
    queued_at: Instant,
    timing: Arc<WriterTiming>,
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
        } = *self;
        timing.observe_queue_wait(queued_at.elapsed());
        let started = Instant::now();
        let result = operation(database);
        timing.observe_execution(started.elapsed());
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
    queue: Arc<WriterQueue>,
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
        let mut lock_path = database_path.as_os_str().to_os_string();
        lock_path.push(".writer-lock");
        let process_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(std::path::PathBuf::from(lock_path))?;
        process_lock
            .try_lock()
            .map_err(|_| Error::Config("database writer is already active"))?;
        let (sender, receiver) = mpsc::sync_channel(maximum_pending);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            let database = Database::open(database_path, database_config);
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
    pub fn call<F, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce(&mut Database) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TestClock(AtomicU64);

    impl foks_server_db::Clock for TestClock {
        fn now_micros(&self) -> foks_server_db::Result<u64> {
            Ok(self.0.load(Ordering::Acquire))
        }
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
