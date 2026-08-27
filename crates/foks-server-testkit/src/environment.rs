use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::certs::{make_tls, TestTls};
use crate::clock::TestClock;
use crate::config::IsolatedPaths;

pub(crate) struct EnvironmentInner {
    pub(crate) paths: IsolatedPaths,
    pub(crate) tls: TestTls,
    pub(crate) clock: Arc<TestClock>,
    pub(crate) root_key: [u8; 32],
    pub(crate) session_faults: Arc<foks_server::SessionFaults>,
    pub(crate) database_config: foks_server_db::Config,
    pub(crate) session_limits: foks_server::SessionLimits,
    pub(crate) maximum_pending_writes: usize,
    pub(crate) addresses: Mutex<Option<foks_server::ServerAddresses>>,
    pub(crate) running: Mutex<bool>,
}

#[derive(Clone)]
pub struct TestEnvironment {
    pub(crate) inner: Arc<EnvironmentInner>,
}

impl TestEnvironment {
    pub fn new() -> foks_server::Result<Self> {
        Self::with_profile(TestProfile::Default)
    }

    pub fn with_profile(profile: TestProfile) -> foks_server::Result<Self> {
        Self::new_with_installation([0x51; 32], None, profile)
    }

    pub(crate) fn new_with_installation(
        root_key: [u8; 32],
        addresses: Option<foks_server::ServerAddresses>,
        profile: TestProfile,
    ) -> foks_server::Result<Self> {
        let now = foks_server_db::Clock::now_micros(&foks_server_db::SystemClock)?;
        let (database_config, session_limits, maximum_pending_writes) = profile.configuration();
        Ok(Self {
            inner: Arc::new(EnvironmentInner {
                paths: IsolatedPaths::create()?,
                tls: make_tls(),
                clock: Arc::new(TestClock::new(now)),
                root_key,
                session_faults: Arc::new(foks_server::SessionFaults::default()),
                database_config,
                session_limits,
                maximum_pending_writes,
                addresses: Mutex::new(addresses),
                running: Mutex::new(false),
            }),
        })
    }

    pub fn restore_backup(
        artifacts: &foks_server::BackupArtifacts,
        addresses: foks_server::ServerAddresses,
    ) -> foks_server::Result<Self> {
        Self::restore_backup_with_root_key(artifacts, addresses, [0x51; 32])
    }

    #[doc(hidden)]
    pub fn restore_backup_with_root_key(
        artifacts: &foks_server::BackupArtifacts,
        addresses: foks_server::ServerAddresses,
        root_key: [u8; 32],
    ) -> foks_server::Result<Self> {
        let environment =
            Self::new_with_installation(root_key, Some(addresses), TestProfile::Default)?;
        foks_server::restore_backup(
            artifacts,
            environment.inner.paths.database(),
            environment.inner.paths.keys(),
            foks_server_db::Config::default(),
        )?;
        Ok(environment)
    }

    pub fn start_server(&self) -> foks_server::Result<crate::InProcessServer> {
        crate::InProcessServer::start(self.clone())
    }

    pub fn start_probe_override(
        &self,
        probe_response: Vec<u8>,
    ) -> foks_server::Result<crate::ProbeOverrideServer> {
        crate::ProbeOverrideServer::start(self.clone(), probe_response)
    }

    pub fn root(&self) -> &Path {
        self.inner.paths.root()
    }

    pub fn client_path(&self, client_id: &str, name: &str) -> foks_server::Result<PathBuf> {
        if !valid_component(client_id) || !valid_file_name(name) {
            return Err(foks_server::Error::Config("invalid test client path"));
        }
        let directory = self.root().join("clients").join(client_id);
        std::fs::create_dir_all(&directory)?;
        Ok(directory.join(name))
    }

    pub fn advance_clock(&self, microseconds: u64) -> u64 {
        self.inner.clock.advance(microseconds)
    }

    pub fn set_clock(&self, microseconds: u64) {
        self.inner.clock.set(microseconds);
    }

    pub fn probe_roots(&self) -> rustls::RootCertStore {
        self.inner.tls.roots.clone()
    }

    pub fn addresses(&self) -> Option<foks_server::ServerAddresses> {
        *self.inner.addresses.lock().expect("test address lock")
    }

    pub fn arm_fault(&self, fault: TestFault) -> u64 {
        let (point, protocol, method) = match fault {
            TestFault::SignupBeforeCommit => (
                foks_server::SessionFaultPoint::BeforeDurableMutation,
                "Reg",
                "signup",
            ),
            TestFault::SignupAfterCommitBeforeResponse => (
                foks_server::SessionFaultPoint::AfterDurableCommitBeforeResponse,
                "Reg",
                "signup",
            ),
            TestFault::SignupDuringResponseWrite => (
                foks_server::SessionFaultPoint::DuringResponseWrite,
                "Reg",
                "signup",
            ),
            TestFault::BetweenLargeFileChunks => (
                foks_server::SessionFaultPoint::BetweenLargeFileChunks,
                "KvStore",
                "fileUploadChunk",
            ),
        };
        let before = self.inner.session_faults.hits();
        self.inner.session_faults.arm(point, protocol, method);
        before
    }

    pub fn fault_hits(&self) -> u64 {
        self.inner.session_faults.hits()
    }

    pub(crate) fn configured_addresses(&self) -> [SocketAddr; 3] {
        let loopback = SocketAddr::from(([127, 0, 0, 1], 0));
        self.addresses()
            .map(|addresses| {
                [
                    addresses.probe,
                    addresses.public_services,
                    addresses.authenticated,
                ]
            })
            .unwrap_or([loopback; 3])
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TestFault {
    SignupBeforeCommit,
    SignupAfterCommitBeforeResponse,
    SignupDuringResponseWrite,
    BetweenLargeFileChunks,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum TestProfile {
    #[default]
    Default,
    SmallCapacity,
    QueuePressure,
    TightIo,
}

impl TestProfile {
    fn configuration(self) -> (foks_server_db::Config, foks_server::SessionLimits, usize) {
        let database = match self {
            Self::SmallCapacity => foks_server_db::Config {
                maximum_kv_namespace_bytes: 512 * 1024,
                maximum_kv_namespace_objects: 128,
                maximum_database_bytes: 16 * 1024 * 1024,
                ..foks_server_db::Config::default()
            },
            Self::Default | Self::QueuePressure | Self::TightIo => {
                foks_server_db::Config::default()
            }
        };
        let limits = foks_server::SessionLimits {
            worker_threads: 16,
            maximum_pending_connections: 64,
            maximum_frame_bytes: if matches!(self, Self::TightIo) {
                1024
            } else {
                foks_server::SessionLimits::default().maximum_frame_bytes
            },
            io_timeout: if matches!(self, Self::TightIo) {
                std::time::Duration::from_millis(200)
            } else {
                foks_server::SessionLimits::default().io_timeout
            },
            ..foks_server::SessionLimits::default()
        };
        let maximum_pending_writes = if matches!(self, Self::QueuePressure) {
            1
        } else {
            16
        };
        (database, limits, maximum_pending_writes)
    }
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_file_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
