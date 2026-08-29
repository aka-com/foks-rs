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
    pub(crate) rate_limits: foks_server::RateLimitConfig,
    pub(crate) backup: Option<foks_server::BackupSchedule>,
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
        let paths = IsolatedPaths::create()?;
        let rate_limits = if matches!(profile, TestProfile::RateLimited) {
            foks_server::RateLimitConfig {
                connection_burst: 16,
                connections_per_second: 1,
                request_burst: 2,
                requests_per_second: 1,
                maximum_tracked_ips: 16,
            }
        } else {
            foks_server::RateLimitConfig::default()
        };
        let backup =
            matches!(profile, TestProfile::BackupAutomation).then(|| foks_server::BackupSchedule {
                directory: paths.backup().join("automatic"),
                interval: std::time::Duration::from_millis(25),
                retain: 2,
            });
        Ok(Self {
            inner: Arc::new(EnvironmentInner {
                paths,
                tls: make_tls(),
                clock: Arc::new(TestClock::new(now)),
                root_key,
                session_faults: Arc::new(foks_server::SessionFaults::default()),
                database_config,
                session_limits,
                maximum_pending_writes,
                rate_limits,
                backup,
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

    #[doc(hidden)]
    pub fn database_path(&self) -> &Path {
        self.inner.paths.database()
    }

    #[doc(hidden)]
    pub fn rotate_host_key(&self) -> foks_server::Result<foks_server::host::HostKeyRotationState> {
        if *self
            .inner
            .running
            .lock()
            .expect("test server lifecycle lock")
        {
            return Err(foks_server::Error::Config(
                "test host rotation requires a stopped server",
            ));
        }
        let provider = foks_server::keys::DirectoryKeyProvider::open_for_rotation(
            self.inner.paths.keys(),
            self.inner.root_key,
        )?;
        let mut database = foks_server_db::Database::open(
            self.inner.paths.database(),
            self.inner.database_config.clone(),
        )?;
        let published = foks_server::host::begin_host_key_rotation(
            &mut database,
            &provider,
            self.advance_clock(1),
        )?;
        foks_server::host::complete_host_key_rotation(
            &mut database,
            &provider,
            published.operation_id,
            foks_server::host::HostKeyRotationObservation {
                add_link_seqno: published
                    .add_link_seqno
                    .ok_or(foks_server::Error::Config("missing test add-link sequence"))?,
            },
            published
                .observation_not_before
                .ok_or(foks_server::Error::Config(
                    "missing test host observation deadline",
                ))?,
        )
    }

    #[doc(hidden)]
    pub fn rotate_capability_key(
        &self,
    ) -> foks_server::Result<foks_server::keys::CapabilityKeyRotationState> {
        if *self
            .inner
            .running
            .lock()
            .expect("test server lifecycle lock")
        {
            return Err(foks_server::Error::Config(
                "test capability rotation requires a stopped server",
            ));
        }
        let provider = foks_server::keys::DirectoryKeyProvider::open_for_rotation(
            self.inner.paths.keys(),
            self.inner.root_key,
        )?;
        let mut database = foks_server_db::Database::open(
            self.inner.paths.database(),
            self.inner.database_config.clone(),
        )?;
        foks_server::keys::rotate_capability_key(&mut database, &provider, self.advance_clock(1))
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

    #[doc(hidden)]
    pub fn write_probe_root(&self, path: impl AsRef<Path>) -> foks_server::Result<()> {
        let root = self
            .inner
            .tls
            .certificate_chain
            .last()
            .ok_or(foks_server::Error::Config("test probe root is missing"))?;
        std::fs::write(path, root)?;
        Ok(())
    }

    pub fn advance_clock(&self, microseconds: u64) -> u64 {
        self.inner.clock.advance(microseconds)
    }

    pub fn set_clock(&self, microseconds: u64) {
        self.inner.clock.set(microseconds);
    }

    #[doc(hidden)]
    pub fn issue_standard_invite(
        &self,
        expires_at: Option<u64>,
    ) -> foks_server::Result<foks_server::invites::IssuedSignupInvite> {
        foks_server::invites::issue_standard_invite(
            self.inner.paths.database(),
            self.inner.database_config.clone(),
            expires_at,
            foks_server_db::Clock::now_micros(self.inner.clock.as_ref())?,
        )
    }

    #[doc(hidden)]
    pub fn issue_multiuse_invite(
        &self,
        code: &str,
        max_uses: Option<u64>,
        expires_at: Option<u64>,
    ) -> foks_server::Result<foks_server::invites::IssuedSignupInvite> {
        foks_server::invites::issue_multiuse_invite(
            self.inner.paths.database(),
            self.inner.database_config.clone(),
            code,
            max_uses,
            expires_at,
            foks_server_db::Clock::now_micros(self.inner.clock.as_ref())?,
        )
    }

    #[doc(hidden)]
    pub fn set_invite_regime(
        &self,
        regime: foks_server_db::InviteRegime,
    ) -> foks_server::Result<()> {
        foks_server::invites::set_invite_regime(
            self.inner.paths.database(),
            self.inner.database_config.clone(),
            regime,
        )
    }

    #[doc(hidden)]
    pub fn disable_invite(&self, code: &str) -> foks_server::Result<bool> {
        foks_server::invites::disable_invite(
            self.inner.paths.database(),
            self.inner.database_config.clone(),
            code,
            foks_server_db::Clock::now_micros(self.inner.clock.as_ref())?,
        )
    }

    #[doc(hidden)]
    pub fn invites(&self) -> foks_server::Result<Vec<foks_server_db::InviteSnapshot>> {
        foks_server::invites::list_invites(
            self.inner.paths.database(),
            self.inner.database_config.clone(),
        )
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
            TestFault::PassphraseSetAfterCommitBeforeResponse => (
                foks_server::SessionFaultPoint::AfterDurableCommitBeforeResponse,
                "User",
                "setPassphrase",
            ),
            TestFault::PassphraseChangeDuringResponseWrite => (
                foks_server::SessionFaultPoint::DuringResponseWrite,
                "User",
                "changePassphrase",
            ),
            TestFault::FederationGrantTeamAfterCommitBeforeResponse => (
                foks_server::SessionFaultPoint::AfterDurableCommitBeforeResponse,
                "TeamMember",
                "grantRemoteViewPermissionForTeam",
            ),
            TestFault::FederationTeamEditAfterCommitBeforeResponse => (
                foks_server::SessionFaultPoint::AfterDurableCommitBeforeResponse,
                "TeamAdmin",
                "editTeam",
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
    PassphraseSetAfterCommitBeforeResponse,
    PassphraseChangeDuringResponseWrite,
    FederationGrantTeamAfterCommitBeforeResponse,
    FederationTeamEditAfterCommitBeforeResponse,
    BetweenLargeFileChunks,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum TestProfile {
    #[default]
    Default,
    SmallCapacity,
    SmallTeamCapacity,
    SmallFederationCapacity,
    QueuePressure,
    TightIo,
    ProductionBenchmark,
    RateLimited,
    BackupAutomation,
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
            Self::SmallTeamCapacity => foks_server_db::Config {
                maximum_team_chain_links: 4,
                maximum_teams: 2,
                maximum_team_members: 2,
                maximum_team_role_bands: 4,
                maximum_team_name_reservations: 2,
                maximum_boxes_per_mutation: 16,
                maximum_team_view_capabilities_per_pair: 2,
                maximum_active_team_view_capabilities: 4,
                maximum_team_admin_capabilities_per_pair: 2,
                maximum_active_team_admin_capabilities: 4,
                maximum_kv_namespace_bytes: 512 * 1024,
                maximum_kv_namespace_objects: 128,
                maximum_database_bytes: 16 * 1024 * 1024,
                ..foks_server_db::Config::default()
            },
            Self::SmallFederationCapacity => foks_server_db::Config {
                maximum_remote_user_view_permissions_per_user: 2,
                maximum_active_remote_user_view_permissions: 3,
                maximum_remote_team_view_permissions_per_team: 2,
                maximum_active_remote_team_view_permissions: 3,
                ..foks_server_db::Config::default()
            },
            Self::Default
            | Self::QueuePressure
            | Self::TightIo
            | Self::ProductionBenchmark
            | Self::RateLimited
            | Self::BackupAutomation => foks_server_db::Config::default(),
        };
        let limits = foks_server::SessionLimits {
            // TightIo intentionally uses one runtime worker: listener boundary
            // tests then prove a slow persistent session cannot occupy it.
            worker_threads: if matches!(self, Self::TightIo | Self::ProductionBenchmark) {
                1
            } else {
                16
            },
            maximum_active_connections: if matches!(self, Self::ProductionBenchmark) {
                128
            } else {
                64
            },
            maximum_pending_connections: 64,
            maximum_frame_bytes: if matches!(self, Self::TightIo) {
                1024
            } else {
                foks_server::SessionLimits::default().maximum_frame_bytes
            },
            io_timeout: if matches!(self, Self::TightIo) {
                std::time::Duration::from_millis(500)
            } else if matches!(self, Self::ProductionBenchmark) {
                std::time::Duration::from_secs(5)
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
