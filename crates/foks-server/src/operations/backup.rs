use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::{BackupSchedule, Error, Result, ServerMetrics};

pub(crate) struct BackupScheduler {
    stop: SyncSender<()>,
    cancelled: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct BackupSource {
    database_path: PathBuf,
    key_directory: PathBuf,
    manifest: Vec<u8>,
    database_config: foks_server_db::Config,
    clock: Arc<dyn foks_server_db::Clock>,
}

impl BackupScheduler {
    pub(crate) fn start(
        schedule: BackupSchedule,
        database_path: PathBuf,
        source_key_directory: PathBuf,
        manifest: Vec<u8>,
        database_config: foks_server_db::Config,
        clock: Arc<dyn foks_server_db::Clock>,
        metrics: Arc<ServerMetrics>,
    ) -> Result<Self> {
        schedule.validate()?;
        std::fs::create_dir_all(&schedule.directory)?;
        let metadata = std::fs::symlink_metadata(&schedule.directory)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::Key("backup root is not a regular directory"));
        }
        cleanup_staging(&schedule.directory)?;
        let (stop, receiver) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let thread_cancelled = Arc::clone(&cancelled);
        let source = BackupSource {
            database_path,
            key_directory: source_key_directory,
            manifest,
            database_config,
            clock,
        };
        let thread = thread::spawn(move || {
            let mut sequence = 0_u64;
            loop {
                match receiver.recv_timeout(schedule.interval) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {
                        if thread_cancelled.load(Ordering::Acquire) {
                            break;
                        }
                        sequence = sequence.wrapping_add(1);
                        metrics.backup_attempted();
                        let started = Instant::now();
                        let result =
                            run_backup(&schedule, &source, sequence, thread_cancelled.as_ref());
                        match result {
                            Ok(now) => metrics.backup_succeeded(now / 1_000_000, started.elapsed()),
                            Err(_) => metrics.backup_failed(started.elapsed()),
                        }
                    }
                }
            }
        });
        Ok(Self {
            stop,
            cancelled,
            thread: Some(thread),
        })
    }

    pub(crate) fn shutdown(mut self) -> Result<()> {
        self.cancelled.store(true, Ordering::Release);
        let _ = self.stop.try_send(());
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| Error::Thread)?;
        }
        Ok(())
    }
}

impl Drop for BackupScheduler {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let _ = self.stop.try_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_backup(
    schedule: &BackupSchedule,
    source: &BackupSource,
    sequence: u64,
    cancelled: &AtomicBool,
) -> Result<u64> {
    let now = source.clock.now_micros()?;
    let name = format!("backup-{now:020}-{sequence:020}");
    let destination = schedule.directory.join(&name);
    let staging = schedule.directory.join(format!(".{name}.tmp"));
    let database =
        foks_server_db::ReadDatabase::open(&source.database_path, source.database_config.clone())?;
    let host_key_files = crate::standalone::host_key_backup_files(&database)?;
    let created = crate::standalone::create_backup(
        &staging,
        &source.key_directory,
        &source.manifest,
        &host_key_files,
        source.database_config.clone(),
        |backup_database| {
            database.online_backup_until(backup_database, || cancelled.load(Ordering::Acquire))?;
            Ok(())
        },
    );
    if let Err(error) = created {
        if std::fs::symlink_metadata(&staging).is_ok_and(|metadata| metadata.file_type().is_dir()) {
            let _ = std::fs::remove_dir_all(&staging);
        }
        return Err(error);
    }
    std::fs::rename(&staging, &destination)?;
    std::fs::File::open(&schedule.directory)?.sync_all()?;
    enforce_retention(&schedule.directory, schedule.retain)?;
    Ok(now)
}

fn cleanup_staging(root: &Path) -> Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(".backup-") || !name.ends_with(".tmp") {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() && !file_type.is_symlink() {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

fn enforce_retention(root: &Path, retain: usize) -> Result<()> {
    let mut backups = std::fs::read_dir(root)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            if !name.starts_with("backup-") {
                return None;
            }
            let file_type = entry.file_type().ok()?;
            (file_type.is_dir() && !file_type.is_symlink()).then(|| (name.to_owned(), entry.path()))
        })
        .collect::<Vec<_>>();
    backups.sort_by(|left, right| left.0.cmp(&right.0));
    let remove = backups.len().saturating_sub(retain);
    for (_, path) in backups.into_iter().take(remove) {
        std::fs::remove_dir_all(path)?;
    }
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}
