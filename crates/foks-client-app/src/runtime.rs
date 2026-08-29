use std::fs::{File, OpenOptions};

use foks_client::{FoksScheduler, ScheduledJobRegistration, SchedulerConfig};
use foks_client_db::ScheduledJobKind;
use fs2::FileExt as _;
use serde::Serialize;

use crate::{
    hex, now_microseconds, AccountVault, Capability, CheckedProfileSession, Error, ProfilePaths,
    Result,
};

const USER_REFRESH_JOB_TYPE_ID: u64 = 0xb1a8_c09a_d2b9_4de7;
const OPERATION_LOCK_FILE: &str = ".profile-operation.lock";
const SCHEDULER_LOCK_FILE: &str = ".scheduler-run.lock";
const DATABASE_LOCK_DIRECTORY: &str = ".database-operation-locks";

pub(crate) struct ProfileLock {
    file: File,
}

pub(crate) struct DatabaseLock {
    file: File,
}

impl ProfileLock {
    pub(crate) fn operation(paths: &ProfilePaths) -> Result<Self> {
        Self::acquire(paths, OPERATION_LOCK_FILE)
    }

    pub(crate) fn try_operation(paths: &ProfilePaths) -> Result<Option<Self>> {
        Self::try_acquire(paths, OPERATION_LOCK_FILE)
    }

    pub(crate) fn scheduler(paths: &ProfilePaths) -> Result<Self> {
        Self::acquire(paths, SCHEDULER_LOCK_FILE)
    }

    fn try_scheduler(paths: &ProfilePaths) -> Result<Option<Self>> {
        Self::try_acquire(paths, SCHEDULER_LOCK_FILE)
    }

    fn acquire(paths: &ProfilePaths, name: &str) -> Result<Self> {
        let file = open_lock(paths, name)?;
        file.lock_exclusive()?;
        Ok(Self { file })
    }

    fn try_acquire(paths: &ProfilePaths, name: &str) -> Result<Option<Self>> {
        let file = open_lock(paths, name)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn release(self) -> Result<()> {
        self.file.unlock()?;
        Ok(())
    }
}

impl DatabaseLock {
    pub(crate) fn acquire(root: &std::path::Path, database_id: &[u8; 16]) -> Result<Self> {
        let file = open_database_lock(root, database_id)?;
        file.lock_exclusive()?;
        Ok(Self { file })
    }

    pub(crate) fn try_acquire(
        root: &std::path::Path,
        database_id: &[u8; 16],
    ) -> Result<Option<Self>> {
        let file = open_database_lock(root, database_id)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn release(self) -> Result<()> {
        self.file.unlock()?;
        Ok(())
    }
}

fn open_lock(paths: &ProfilePaths, name: &str) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(paths.directory.join(name)).map_err(Into::into)
}

fn open_database_lock(root: &std::path::Path, database_id: &[u8; 16]) -> Result<File> {
    let directory = crate::prepare_private_directory(&root.join(DATABASE_LOCK_DIRECTORY))?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(directory.join(format!("{}.lock", crate::hex(database_id))))
        .map_err(Into::into)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobRunReport {
    pub runs: Vec<JobRun>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobRun {
    pub job_id_hex: String,
    pub completed: bool,
    pub error: Option<String>,
    pub next_run_at: u64,
}

impl CheckedProfileSession<'_> {
    pub fn schedule_user_refresh(
        &self,
        alias: &str,
        interval_micros: u64,
        first_run_at: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<[u8; 16]> {
        self.profile.require(Capability::UserSync)?;
        if interval_micros == 0 {
            return Err(Error::InvalidConfig("job interval is zero"));
        }
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let mut binding = Vec::with_capacity(66);
        binding.extend_from_slice(host.host_id().as_bytes());
        binding.extend_from_slice(loaded.credential.uid.as_bytes());
        let digest = foks_crypto::prefixed_hash(USER_REFRESH_JOB_TYPE_ID, &binding);
        let job_id = digest[..16]
            .try_into()
            .expect("hash prefix has fixed length");
        FoksScheduler::new(&self.paths.hard_database, SchedulerConfig::default())?.register(
            ScheduledJobRegistration {
                job_id,
                kind: ScheduledJobKind::UserRefresh,
                host_id: host.host_id().as_bytes().to_vec(),
                scope_id: loaded.credential.uid.as_bytes().to_vec(),
                interval_micros,
                first_run_at,
                registered_at: now_microseconds()?,
            },
        )?;
        Ok(job_id)
    }

    pub fn run_due_jobs(&self, now: u64, vault: &mut AccountVault<'_>) -> Result<JobRunReport> {
        self.profile.require(Capability::UserSync)?;
        // The scheduler lock prevents overlapping manual and periodic runs.
        // The rollback/operation lock remains a separate checked-session
        // concern on a separate inode.
        let _scheduler_lock = ProfileLock::scheduler(&self.paths)?;
        self.run_due_jobs_locked(now, vault)
    }

    /// Runs due work only if another process is not already scheduling this
    /// profile. Periodic schedulers use this to avoid blocking a worker.
    pub fn try_run_due_jobs(
        &self,
        now: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<Option<JobRunReport>> {
        self.profile.require(Capability::UserSync)?;
        let Some(_scheduler_lock) = ProfileLock::try_scheduler(&self.paths)? else {
            return Ok(None);
        };
        self.run_due_jobs_locked(now, vault).map(Some)
    }

    fn run_due_jobs_locked(&self, now: u64, vault: &mut AccountVault<'_>) -> Result<JobRunReport> {
        self.run_due_jobs_locked_with(now, vault, |_, _| {
            Err("federation reconciliation requires access to the remote profile".to_owned())
        })
    }

    pub(super) fn run_due_jobs_locked_with(
        &self,
        now: u64,
        vault: &mut AccountVault<'_>,
        mut federation: impl FnMut(
            &foks_client_db::ScheduledJob,
            &mut AccountVault<'_>,
        ) -> std::result::Result<(), String>,
    ) -> Result<JobRunReport> {
        let scheduler = FoksScheduler::new(&self.paths.hard_database, SchedulerConfig::default())?;
        let report = scheduler.run_due(now, |job| match job.kind {
            ScheduledJobKind::UserRefresh => {
                let aliases = vault.aliases().map_err(|error| error.to_string())?;
                let alias = aliases
                    .into_iter()
                    .find(|alias| {
                        vault
                            .account(alias)
                            .is_ok_and(|account| account.credential.uid.as_bytes() == job.scope_id)
                    })
                    .ok_or_else(|| "scheduled account credential is unavailable".to_owned())?;
                self.sync_account(&alias, vault)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
            ScheduledJobKind::YubiManagementRefresh => {
                self.profile
                    .require(Capability::DeviceAdministration)
                    .map_err(|error| error.to_string())?;
                let scope: super::yubi::YubiRefreshScope = serde_json::from_slice(&job.scope_id)
                    .map_err(|_| "scheduled Yubi refresh scope is invalid".to_owned())?;
                self.refresh_yubi_management_envelope(
                    &scope.yubi_alias,
                    &scope.software_alias,
                    vault,
                )
                .map_err(|error| error.to_string())
            }
            ScheduledJobKind::MutationReconcile => {
                Err("mutation reconciliation is driven by explicit resume flows".to_owned())
            }
            ScheduledJobKind::FederationRefresh => federation(job, vault),
        })?;
        Ok(JobRunReport {
            runs: report
                .runs
                .into_iter()
                .map(|run| {
                    let (completed, error) = match run.status {
                        foks_client::ScheduledRunStatus::Completed => (true, None),
                        foks_client::ScheduledRunStatus::Failed { error } => (false, Some(error)),
                    };
                    JobRun {
                        job_id_hex: hex(&run.job_id),
                        completed,
                        error,
                        next_run_at: run.next_run_at,
                    }
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(root: &std::path::Path, name: &str) -> ProfilePaths {
        let directory = root.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        ProfilePaths {
            hard_database: directory.join("hard.sqlite3"),
            soft_database: directory.join("soft.sqlite3"),
            protected_mutations: directory.join("mutations"),
            credential_store: directory.join("credentials"),
            directory,
        }
    }

    #[test]
    fn operation_locks_are_per_profile_and_scheduler_locks_are_independent() {
        let temporary = tempfile::tempdir().unwrap();
        let first = paths(temporary.path(), "first");
        let second = paths(temporary.path(), "second");
        let operation = ProfileLock::operation(&first).unwrap();
        assert!(ProfileLock::try_acquire(&first, OPERATION_LOCK_FILE)
            .unwrap()
            .is_none());
        let other_profile = ProfileLock::try_acquire(&second, OPERATION_LOCK_FILE)
            .unwrap()
            .unwrap();
        let scheduler = ProfileLock::try_acquire(&first, SCHEDULER_LOCK_FILE)
            .unwrap()
            .unwrap();
        assert!(ProfileLock::try_acquire(&first, SCHEDULER_LOCK_FILE)
            .unwrap()
            .is_none());
        assert!(first.directory.join(OPERATION_LOCK_FILE).is_file());
        assert!(first.directory.join(SCHEDULER_LOCK_FILE).is_file());
        assert!(!temporary.path().join(".rollback-checkpoint.lock").exists());
        scheduler.release().unwrap();
        other_profile.release().unwrap();
        operation.release().unwrap();
    }

    #[test]
    fn database_locks_are_keyed_by_identity_within_one_client_root() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        std::fs::create_dir(&root).unwrap();
        let first = DatabaseLock::acquire(&root, &[1; 16]).unwrap();
        assert!(DatabaseLock::try_acquire(&root, &[1; 16])
            .unwrap()
            .is_none());
        let other = DatabaseLock::try_acquire(&root, &[2; 16]).unwrap().unwrap();
        assert!(root
            .join(DATABASE_LOCK_DIRECTORY)
            .join(format!("{}.lock", crate::hex(&[1; 16])))
            .is_file());
        other.release().unwrap();
        first.release().unwrap();
    }

    #[test]
    fn database_locks_contend_across_processes() {
        const ROOT_ENV: &str = "FOKS_DATABASE_LOCK_TEST_ROOT";
        const CHILD_ENV: &str = "FOKS_DATABASE_LOCK_TEST_CHILD";

        if std::env::var_os(CHILD_ENV).is_some() {
            let root = std::path::PathBuf::from(std::env::var_os(ROOT_ENV).unwrap());
            assert!(DatabaseLock::try_acquire(&root, &[3; 16])
                .unwrap()
                .is_none());
            return;
        }

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        std::fs::create_dir(&root).unwrap();
        let lock = DatabaseLock::acquire(&root, &[3; 16]).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::tests::database_locks_contend_across_processes",
                "--nocapture",
            ])
            .env(ROOT_ENV, &root)
            .env(CHILD_ENV, "1")
            .status()
            .unwrap();
        assert!(status.success());
        lock.release().unwrap();
    }

    #[test]
    fn profile_locks_contend_across_processes() {
        const ROOT_ENV: &str = "FOKS_PROFILE_LOCK_TEST_ROOT";
        const CHILD_ENV: &str = "FOKS_PROFILE_LOCK_TEST_CHILD";

        if std::env::var_os(CHILD_ENV).is_some() {
            let root = std::path::PathBuf::from(std::env::var_os(ROOT_ENV).unwrap());
            let profile = paths(&root, "profile");
            assert!(ProfileLock::try_operation(&profile).unwrap().is_none());
            assert!(ProfileLock::try_scheduler(&profile).unwrap().is_none());
            return;
        }

        let temporary = tempfile::tempdir().unwrap();
        let profile = paths(temporary.path(), "profile");
        let operation = ProfileLock::operation(&profile).unwrap();
        let scheduler = ProfileLock::scheduler(&profile).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::tests::profile_locks_contend_across_processes",
                "--nocapture",
            ])
            .env(ROOT_ENV, temporary.path())
            .env(CHILD_ENV, "1")
            .status()
            .unwrap();
        assert!(status.success());
        scheduler.release().unwrap();
        operation.release().unwrap();
    }
}
