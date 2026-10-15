//! Explicit, standalone installation layout and client bootstrap artifacts.

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose, PKCS_ED25519};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};

const INSTALLATION_VERSION: u32 = 1;
const MAXIMUM_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_admin: Option<crate::web_admin::WebAdminConfig>,
    #[serde(default)]
    pub vhost_management_host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc: Option<crate::sso::OidcOperatorConfig>,
    pub version: u32,
    pub canonical_name: String,
    pub database: PathBuf,
    pub key_directory: PathBuf,
    pub root_key_file: PathBuf,
    pub probe_certificate_der: PathBuf,
    pub probe_private_key_der: PathBuf,
    pub probe_address: SocketAddr,
    pub public_address: SocketAddr,
    pub authenticated_address: SocketAddr,
    pub management_address: SocketAddr,
    pub backup_directory: PathBuf,
    pub backup_interval_seconds: u64,
    pub backup_retain: usize,
    #[serde(default)]
    pub waitlist: crate::waitlist::WaitlistConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub rate_limits: crate::RateLimitConfig,
    #[serde(default)]
    pub storage: StorageConfig,
}

impl InstallationConfig {
    pub fn validate(&self) -> crate::Result<()> {
        if self.version != INSTALLATION_VERSION {
            return Err(crate::Error::Config(
                "unsupported installation configuration version",
            ));
        }
        validate_hostname(&self.canonical_name)?;
        self.waitlist.validate()?;
        crate::config::validate_vhost_management_host(&self.vhost_management_host)?;
        if let Some(admin) = &self.web_admin {
            admin.validate()?;
            if [
                self.probe_address,
                self.public_address,
                self.authenticated_address,
                self.management_address,
            ]
            .contains(&admin.listen)
                || self.oidc.as_ref().is_some_and(|v| v.listen == admin.listen)
            {
                return Err(crate::Error::Config(
                    "admin listener must have a distinct address",
                ));
            }
            if self.limits.maximum_read_connections < 3 || self.limits.maximum_pending_writes < 2 {
                return Err(crate::Error::Config(
                    "admin requires at least three read connections and two writer slots",
                ));
            }
        }
        if let Some(oidc) = &self.oidc {
            oidc.validate(foks_oidc::NetworkPolicy::default())?;
            if [
                self.probe_address,
                self.public_address,
                self.authenticated_address,
                self.management_address,
            ]
            .contains(&oidc.listen)
            {
                return Err(crate::Error::Config(
                    "OIDC listener must have a distinct address",
                ));
            }
        }
        if self.backup_interval_seconds == 0 {
            return Err(crate::Error::Config("backup interval must be non-zero"));
        }
        if self.backup_retain == 0 {
            return Err(crate::Error::Config("backup retention must be non-zero"));
        }
        if !self.management_address.ip().is_loopback() {
            return Err(crate::Error::Config(
                "management listener must remain loopback-only",
            ));
        }
        if [
            &self.database,
            &self.key_directory,
            &self.root_key_file,
            &self.probe_certificate_der,
            &self.probe_private_key_der,
            &self.backup_directory,
        ]
        .into_iter()
        .any(|path| !path.is_absolute())
        {
            return Err(crate::Error::Config("installation paths must be absolute"));
        }
        self.limits.validate()?;
        self.rate_limits.validate()?;
        self.storage.validate()?;
        let listeners = [
            self.probe_address,
            self.public_address,
            self.authenticated_address,
            self.management_address,
        ];
        if listeners
            .iter()
            .enumerate()
            .any(|(index, address)| listeners[index + 1..].contains(address))
        {
            return Err(crate::Error::Config(
                "installation listeners must use distinct addresses",
            ));
        }
        Ok(())
    }

    /// Database limits from the `[storage]` and `[waitlist]` tables. Every
    /// command that opens the installation database uses them, so offline
    /// commands enforce the same database ceiling as the server.
    pub fn database_config(&self) -> foks_server_db::Config {
        self.waitlist.database_config(
            self.storage
                .database_config(foks_server_db::Config::default()),
        )
    }
}

/// The `[limits]` table: RPC session limits and the writer queue length.
/// The defaults equal `SessionLimits::default()` and the `serve` flag
/// defaults. Session limits that the table omits keep their
/// `SessionLimits::default()` values.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LimitsConfig {
    pub worker_threads: usize,
    pub maximum_read_connections: usize,
    pub maximum_active_connections: usize,
    pub maximum_pending_connections: usize,
    pub maximum_in_flight_requests: usize,
    pub request_timeout_seconds: u64,
    /// Writes queued for the SQLite writer before callers are refused.
    pub maximum_pending_writes: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        let session = crate::SessionLimits::default();
        Self {
            worker_threads: session.worker_threads,
            maximum_read_connections: session.maximum_read_connections,
            maximum_active_connections: session.maximum_active_connections,
            maximum_pending_connections: session.maximum_pending_connections,
            maximum_in_flight_requests: session.maximum_in_flight_requests,
            request_timeout_seconds: session.request_timeout.as_secs(),
            maximum_pending_writes: 64,
        }
    }
}

impl LimitsConfig {
    pub fn session_limits(&self) -> crate::SessionLimits {
        crate::SessionLimits {
            worker_threads: self.worker_threads,
            maximum_read_connections: self.maximum_read_connections,
            maximum_active_connections: self.maximum_active_connections,
            maximum_pending_connections: self.maximum_pending_connections,
            maximum_in_flight_requests: self.maximum_in_flight_requests,
            request_timeout: Duration::from_secs(self.request_timeout_seconds),
            ..crate::SessionLimits::default()
        }
    }

    fn validate(&self) -> crate::Result<()> {
        if self.worker_threads == 0
            || self.maximum_read_connections == 0
            || self.maximum_active_connections == 0
            || self.maximum_pending_connections == 0
            || self.maximum_in_flight_requests == 0
            || self.request_timeout_seconds == 0
            || self.maximum_pending_writes == 0
        {
            return Err(crate::Error::Config("[limits] values must be non-zero"));
        }
        self.session_limits().validate()
    }
}

/// The `[storage]` table: database capacity limits. The defaults equal
/// `foks_server_db::Config::default()`; database limits that the table omits
/// keep their compiled defaults.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    /// SQLite `max_page_count` ceiling for the main database file.
    pub maximum_database_bytes: u64,
    pub maximum_teams: usize,
    /// Entries in one team roster. At most
    /// `foks_server_db::RealtimeLimits::FANOUT_MEMBERS` (1,024), the roster
    /// rows a realtime mutation scans, so that every team can create channels
    /// and send messages.
    pub maximum_team_members: usize,
    pub maximum_kv_namespace_bytes: u64,
    pub maximum_kv_namespace_objects: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        let database = foks_server_db::Config::default();
        Self {
            maximum_database_bytes: database.maximum_database_bytes,
            maximum_teams: database.maximum_teams,
            maximum_team_members: database.maximum_team_members,
            maximum_kv_namespace_bytes: database.maximum_kv_namespace_bytes,
            maximum_kv_namespace_objects: database.maximum_kv_namespace_objects,
        }
    }
}

impl StorageConfig {
    /// Applies the table to `database`.
    pub fn database_config(&self, database: foks_server_db::Config) -> foks_server_db::Config {
        foks_server_db::Config {
            maximum_database_bytes: self.maximum_database_bytes,
            maximum_teams: self.maximum_teams,
            maximum_team_members: self.maximum_team_members,
            maximum_kv_namespace_bytes: self.maximum_kv_namespace_bytes,
            maximum_kv_namespace_objects: self.maximum_kv_namespace_objects,
            ..database
        }
    }

    fn validate(&self) -> crate::Result<()> {
        if self.maximum_database_bytes == 0
            || self.maximum_teams == 0
            || self.maximum_team_members == 0
            || self.maximum_kv_namespace_bytes == 0
            || self.maximum_kv_namespace_objects == 0
        {
            return Err(crate::Error::Config("[storage] values must be non-zero"));
        }
        if self.maximum_team_members > foks_server_db::RealtimeLimits::FANOUT_MEMBERS {
            return Err(crate::Error::Config(
                "maximum_team_members exceeds the realtime fanout limit",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InstallationStatus {
    pub initialized: bool,
    pub canonical_name: String,
    pub host_id_hex: Option<String>,
    pub database_bytes: u64,
    pub wal_bytes: u64,
    pub integrity_ok: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ClientBootstrap {
    pub version: u32,
    pub canonical_name: String,
    pub probe: String,
    pub host_id_hex: String,
    pub probe_ca_der: PathBuf,
}

pub fn initialize(
    directory: impl AsRef<Path>,
    canonical_name: &str,
    probe_address: SocketAddr,
    public_address: SocketAddr,
    authenticated_address: SocketAddr,
    management_address: SocketAddr,
) -> crate::Result<PathBuf> {
    validate_hostname(canonical_name)?;
    if !management_address.ip().is_loopback() {
        return Err(crate::Error::Config(
            "management listener must remain loopback-only",
        ));
    }
    let directory = prepare_empty_private_directory(directory.as_ref())?;
    let data = directory.join("data");
    let keys = directory.join("keys");
    let backups = directory.join("backups");
    create_private_directory(&data)?;
    create_private_directory(&keys)?;
    create_private_directory(&backups)?;

    let root_key_file = directory.join("operator-root.key");
    let mut root_key = zeroize::Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut *root_key)
        .map_err(|_| crate::Error::Config("OS randomness unavailable"))?;
    write_new_file(&root_key_file, &*root_key, 0o600)?;

    let probe_private_key_der = directory.join("probe-private-key.der");
    let probe_certificate_der = directory.join("probe-certificate.der");
    let key = KeyPair::generate_for(&PKCS_ED25519)?;
    let mut parameters = CertificateParams::new(vec![canonical_name.to_owned()])?;
    parameters.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    parameters.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let certificate = parameters.self_signed(&key)?;
    let private_der = zeroize::Zeroizing::new(key.serialize_der());
    write_new_file(&probe_private_key_der, &private_der, 0o600)?;
    write_new_file(&probe_certificate_der, certificate.der(), 0o644)?;

    let config = InstallationConfig {
        vhost_management_host: String::new(),
        oidc: None,
        web_admin: None,
        version: INSTALLATION_VERSION,
        canonical_name: canonical_name.to_owned(),
        database: data.join("foks-server.sqlite"),
        key_directory: keys,
        root_key_file,
        probe_certificate_der,
        probe_private_key_der,
        probe_address,
        public_address,
        authenticated_address,
        management_address,
        backup_directory: backups,
        backup_interval_seconds: 86_400,
        backup_retain: 7,
        waitlist: crate::waitlist::WaitlistConfig::default(),
        limits: LimitsConfig::default(),
        rate_limits: crate::RateLimitConfig::default(),
        storage: StorageConfig::default(),
    };
    config.validate()?;
    let path = directory.join("server.toml");
    write_new_file(&path, toml::to_string_pretty(&config)?.as_bytes(), 0o600)?;
    sync_directory(&directory)?;
    Ok(path)
}

pub fn load_config(path: impl AsRef<Path>) -> crate::Result<InstallationConfig> {
    let bytes = read_bounded_regular(path.as_ref(), MAXIMUM_CONFIG_BYTES, true)?;
    let config: InstallationConfig = toml::from_slice(&bytes)?;
    config.validate()?;
    Ok(config)
}

pub fn validate_artifacts(config: &InstallationConfig) -> crate::Result<()> {
    if let Some(oidc) = &config.oidc {
        crate::keys::read_secret_file(&oidc.client_secret_file, 4096)?;
    }

    config.validate()?;
    crate::keys::read_root_key_file(&config.root_key_file)?;
    validate_private_directory(&config.key_directory)?;
    validate_private_directory(&config.backup_directory)?;
    if config.database.exists() {
        validate_regular_path(&config.database)?;
    }
    validate_probe_tls(config)?;
    Ok(())
}

pub fn status(config: &InstallationConfig) -> crate::Result<InstallationStatus> {
    config.validate()?;
    if !config.database.exists() {
        return Ok(InstallationStatus {
            initialized: false,
            canonical_name: config.canonical_name.clone(),
            host_id_hex: None,
            database_bytes: 0,
            wal_bytes: 0,
            integrity_ok: None,
        });
    }
    validate_regular_path(&config.database)?;
    let database = foks_server_db::ReadDatabase::open(&config.database, config.database_config())?;
    let integrity_ok = database.integrity_check()?;
    let database_bytes = fs::metadata(&config.database)?.len();
    let mut wal_name = config.database.as_os_str().to_os_string();
    wal_name.push("-wal");
    let wal_bytes = match fs::metadata(PathBuf::from(wal_name)) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    let bootstrap = database.host_bootstrap()?;
    Ok(InstallationStatus {
        initialized: bootstrap.is_some(),
        canonical_name: config.canonical_name.clone(),
        host_id_hex: bootstrap.map(|bootstrap| hex(&bootstrap.host_id)),
        database_bytes,
        wal_bytes,
        integrity_ok: Some(integrity_ok),
    })
}

pub fn client_bootstrap(config: &InstallationConfig) -> crate::Result<ClientBootstrap> {
    config.validate()?;
    let database = foks_server_db::ReadDatabase::open(&config.database, config.database_config())?;
    let stored = database.host_bootstrap()?.ok_or(crate::Error::Config(
        "server has not completed first bootstrap",
    ))?;
    let verified = foks_verify::verify_public_host(&stored.canonical_name, &stored.probe_response)?;
    if verified.snapshot.host_id() != stored.host_id
        || verified.snapshot.canonical_name() != config.canonical_name
    {
        return Err(crate::Error::Config(
            "stored client bootstrap binding does not match configuration",
        ));
    }
    Ok(ClientBootstrap {
        version: INSTALLATION_VERSION,
        canonical_name: config.canonical_name.clone(),
        probe: verified.public_zone.services.probe.clone(),
        host_id_hex: hex(&stored.host_id),
        probe_ca_der: config.probe_certificate_der.clone(),
    })
}

pub fn write_client_bootstrap(
    config: &InstallationConfig,
    path: impl AsRef<Path>,
) -> crate::Result<()> {
    let bootstrap = client_bootstrap(config)?;
    write_new_file(
        path.as_ref(),
        toml::to_string_pretty(&bootstrap)?.as_bytes(),
        0o600,
    )
}

fn prepare_empty_private_directory(path: &Path) -> crate::Result<PathBuf> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(crate::Error::Config("installation path is unsafe"));
        }
        if fs::read_dir(path)?.next().is_some() {
            return Err(crate::Error::Config("installation directory is not empty"));
        }
    } else {
        create_private_directory(path)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    path.canonicalize().map_err(Into::into)
}

fn create_private_directory(path: &Path) -> crate::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(path)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path)?;
    Ok(())
}

pub(crate) fn write_new_file(path: &Path, bytes: &[u8], unix_mode: u32) -> crate::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(unix_mode).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    let _ = unix_mode;
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn read_bounded_regular(path: &Path, maximum: u64, private: bool) -> crate::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > maximum
    {
        return Err(crate::Error::Config(
            "installation file is unsafe or excessive",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if private && metadata.permissions().mode() & 0o077 != 0 {
            return Err(crate::Error::Config(
                "installation file permissions are unsafe",
            ));
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file).take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(crate::Error::Config(
            "installation file grew beyond its limit",
        ));
    }
    Ok(bytes)
}

fn validate_private_directory(path: &Path) -> crate::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(crate::Error::Config(
            "installation directory is missing or unsafe",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(crate::Error::Config(
                "installation directory permissions are unsafe",
            ));
        }
    }
    Ok(())
}

fn validate_regular_path(path: &Path) -> crate::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(crate::Error::Config("installation database path is unsafe"));
    }
    Ok(())
}

fn validate_probe_tls(config: &InstallationConfig) -> crate::Result<()> {
    let certificate = read_bounded_regular(&config.probe_certificate_der, 1024 * 1024, false)?;
    let private_key = crate::keys::read_secret_file(&config.probe_private_key_der, 64 * 1024)?;
    let private_key = PrivateKeyDer::try_from(private_key.as_slice())
        .map_err(|_| crate::Error::Config("probe private key is not valid DER"))?
        .clone_key();
    let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(certificate)], private_key)?;
    Ok(())
}

fn sync_directory(path: &Path) -> crate::Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn validate_hostname(name: &str) -> crate::Result<()> {
    if name.is_empty()
        || !name.is_ascii()
        || name.len() > 253
        || name.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(crate::Error::Config("invalid canonical DNS name"));
    }
    Ok(())
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
        output
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialization_is_explicit_private_and_non_overwriting() {
        let temporary = tempfile::tempdir().unwrap();
        let install = temporary.path().join("server");
        let config_path = initialize(
            &install,
            "localhost",
            "127.0.0.1:4430".parse().unwrap(),
            "127.0.0.1:4431".parse().unwrap(),
            "127.0.0.1:4432".parse().unwrap(),
            "127.0.0.1:9090".parse().unwrap(),
        )
        .unwrap();
        let text = std::fs::read_to_string(&config_path).unwrap();
        assert!(text.contains("[waitlist]\nenabled = false\n"), "{text}");
        let config = load_config(config_path).unwrap();
        validate_artifacts(&config).unwrap();
        assert_eq!(config.waitlist, crate::waitlist::WaitlistConfig::default());
        let mut colliding = config.clone();
        colliding.public_address = colliding.probe_address;
        assert!(colliding.validate().is_err());
        assert!(!status(&config).unwrap().initialized);
        assert!(initialize(
            &install,
            "localhost",
            "127.0.0.1:4430".parse().unwrap(),
            "127.0.0.1:4431".parse().unwrap(),
            "127.0.0.1:4432".parse().unwrap(),
            "127.0.0.1:9090".parse().unwrap(),
        )
        .is_err());

        let other = temporary.path().join("other");
        let other_config = initialize(
            &other,
            "localhost",
            "127.0.0.1:5430".parse().unwrap(),
            "127.0.0.1:5431".parse().unwrap(),
            "127.0.0.1:5432".parse().unwrap(),
            "127.0.0.1:9190".parse().unwrap(),
        )
        .unwrap();
        let other_config = load_config(other_config).unwrap();
        std::fs::copy(
            other_config.probe_private_key_der,
            &config.probe_private_key_der,
        )
        .unwrap();
        assert!(validate_artifacts(&config).is_err());
    }

    #[test]
    fn waitlist_table_is_optional_validated_and_strict() {
        let temporary = tempfile::tempdir().unwrap();
        let config_path = initialize(
            temporary.path().join("server"),
            "localhost",
            "127.0.0.1:4430".parse().unwrap(),
            "127.0.0.1:4431".parse().unwrap(),
            "127.0.0.1:4432".parse().unwrap(),
            "127.0.0.1:9090".parse().unwrap(),
        )
        .unwrap();
        let text = std::fs::read_to_string(&config_path).unwrap();
        let (base, _) = text.split_once("[waitlist]").unwrap();
        let parse = |suffix: &str| -> crate::Result<InstallationConfig> {
            let config: InstallationConfig = toml::from_str(&format!("{base}{suffix}"))?;
            config.validate()?;
            Ok(config)
        };
        assert_eq!(
            parse("").unwrap().waitlist,
            crate::waitlist::WaitlistConfig::default()
        );
        let enabled = parse("[waitlist]\nenabled = true\nmaximum_entries = 50\n").unwrap();
        assert!(enabled.waitlist.enabled);
        assert_eq!(enabled.waitlist.maximum_entries, 50);
        assert_eq!(
            enabled.waitlist.retention_days,
            crate::waitlist::WaitlistConfig::default().retention_days
        );
        assert!(parse("[waitlist]\nmaximum_entries = 0\n").is_err());
        assert!(parse("[waitlist]\nenable = true\n").is_err());
    }

    fn initialized_config() -> (tempfile::TempDir, String, InstallationConfig) {
        let temporary = tempfile::tempdir().unwrap();
        let config_path = initialize(
            temporary.path().join("server"),
            "localhost",
            "127.0.0.1:4430".parse().unwrap(),
            "127.0.0.1:4431".parse().unwrap(),
            "127.0.0.1:4432".parse().unwrap(),
            "127.0.0.1:9090".parse().unwrap(),
        )
        .unwrap();
        let text = std::fs::read_to_string(&config_path).unwrap();
        let config = load_config(config_path).unwrap();
        (temporary, text, config)
    }

    #[test]
    fn limit_tables_default_to_the_compiled_limits() {
        let (_temporary, text, config) = initialized_config();
        for table in ["[limits]\n", "[rate_limits]\n", "[storage]\n"] {
            assert!(text.contains(table), "{text}");
        }
        let (base, _) = text.split_once("[waitlist]").unwrap();
        let omitted: InstallationConfig = toml::from_str(base).unwrap();
        assert_eq!(omitted, config);
        assert_eq!(
            format!("{:?}", config.limits.session_limits()),
            format!("{:?}", crate::SessionLimits::default())
        );
        assert_eq!(config.limits.maximum_pending_writes, 64);
        assert_eq!(config.rate_limits, crate::RateLimitConfig::default());
        assert_eq!(
            format!("{:?}", config.database_config()),
            format!("{:?}", foks_server_db::Config::default())
        );
    }

    #[test]
    fn every_limit_table_field_round_trips_to_its_runtime_value() {
        let (_temporary, text, _) = initialized_config();
        let (base, _) = text.split_once("[waitlist]").unwrap();
        let tables = "\
            [limits]\n\
            worker_threads = 3\n\
            maximum_read_connections = 5\n\
            maximum_active_connections = 7\n\
            maximum_pending_connections = 11\n\
            maximum_in_flight_requests = 13\n\
            request_timeout_seconds = 17\n\
            maximum_pending_writes = 19\n\
            [rate_limits]\n\
            connection_burst = 23\n\
            connections_per_second = 29\n\
            request_burst = 31\n\
            requests_per_second = 37\n\
            maximum_tracked_ips = 41\n\
            [storage]\n\
            maximum_database_bytes = 43\n\
            maximum_teams = 47\n\
            maximum_team_members = 53\n\
            maximum_kv_namespace_bytes = 59\n\
            maximum_kv_namespace_objects = 61\n";
        let config: InstallationConfig = toml::from_str(&format!("{base}{tables}")).unwrap();
        config.validate().unwrap();
        assert_eq!(
            config.limits,
            LimitsConfig {
                worker_threads: 3,
                maximum_read_connections: 5,
                maximum_active_connections: 7,
                maximum_pending_connections: 11,
                maximum_in_flight_requests: 13,
                request_timeout_seconds: 17,
                maximum_pending_writes: 19,
            }
        );
        assert_eq!(
            config.rate_limits,
            crate::RateLimitConfig {
                connection_burst: 23,
                connections_per_second: 29,
                request_burst: 31,
                requests_per_second: 37,
                maximum_tracked_ips: 41,
            }
        );
        assert_eq!(
            config.storage,
            StorageConfig {
                maximum_database_bytes: 43,
                maximum_teams: 47,
                maximum_team_members: 53,
                maximum_kv_namespace_bytes: 59,
                maximum_kv_namespace_objects: 61,
            }
        );

        let session = config.limits.session_limits();
        assert_eq!(
            (
                session.worker_threads,
                session.maximum_read_connections,
                session.maximum_active_connections,
                session.maximum_pending_connections,
                session.maximum_in_flight_requests,
                session.request_timeout,
            ),
            (3, 5, 7, 11, 13, Duration::from_secs(17))
        );
        let database = config.database_config();
        assert_eq!(
            (
                database.maximum_database_bytes,
                database.maximum_teams,
                database.maximum_team_members,
                database.maximum_kv_namespace_bytes,
                database.maximum_kv_namespace_objects,
                database.maximum_waitlist_entries,
            ),
            (43, 47, 53, 59, 61, config.waitlist.maximum_entries)
        );

        let serialized = toml::to_string_pretty(&config).unwrap();
        assert_eq!(
            toml::from_str::<InstallationConfig>(&serialized).unwrap(),
            config
        );
    }

    #[test]
    fn unknown_keys_and_tables_are_rejected() {
        let (_temporary, text, _) = initialized_config();
        let (base, _) = text.split_once("[waitlist]").unwrap();
        for (scalars, tables) in [
            ("backup_retention = 7\n", ""),
            ("", "[limit]\nworker_threads = 4\n"),
            ("", "[limits]\nworker_thread = 4\n"),
            ("", "[rate_limits]\nconnection_rate_burst = 512\n"),
            ("", "[storage]\nmaximum_users = 10\n"),
        ] {
            let text = format!("{scalars}{base}{tables}");
            assert!(
                toml::from_str::<InstallationConfig>(&text).is_err(),
                "{scalars}{tables}"
            );
        }
        toml::from_str::<InstallationConfig>(base).unwrap();
    }

    #[test]
    fn validation_errors_name_the_failing_condition() {
        let (_temporary, _, config) = initialized_config();
        let message = |change: &dyn Fn(&mut InstallationConfig)| {
            let mut changed = config.clone();
            change(&mut changed);
            match changed.validate() {
                Err(crate::Error::Config(message)) => message,
                other => panic!("unexpected validation result: {other:?}"),
            }
        };
        assert_eq!(
            message(&|c| c.version = INSTALLATION_VERSION + 1),
            "unsupported installation configuration version"
        );
        assert_eq!(
            message(&|c| c.backup_interval_seconds = 0),
            "backup interval must be non-zero"
        );
        assert_eq!(
            message(&|c| c.backup_retain = 0),
            "backup retention must be non-zero"
        );
        assert_eq!(
            message(&|c| c.management_address = "192.0.2.1:9090".parse().unwrap()),
            "management listener must remain loopback-only"
        );
        assert_eq!(
            message(&|c| c.backup_directory = PathBuf::from("backups")),
            "installation paths must be absolute"
        );
        assert_eq!(
            message(&|c| c.limits.request_timeout_seconds = 0),
            "[limits] values must be non-zero"
        );
        assert_eq!(
            message(
                &|c| c.limits.maximum_in_flight_requests = tokio::sync::Semaphore::MAX_PERMITS + 1
            ),
            "invalid session limit"
        );
        assert_eq!(
            message(&|c| c.rate_limits.request_burst = 0),
            "rate limit parameters and tracked IP capacity must be non-zero"
        );
        assert_eq!(
            message(&|c| c.storage.maximum_teams = 0),
            "[storage] values must be non-zero"
        );
        assert_eq!(
            message(&|c| c.storage.maximum_team_members =
                foks_server_db::RealtimeLimits::FANOUT_MEMBERS + 1),
            "maximum_team_members exceeds the realtime fanout limit"
        );
        let with_admin = |change: &dyn Fn(&mut InstallationConfig)| {
            message(&|c| {
                c.web_admin = Some(crate::web_admin::WebAdminConfig {
                    origin: "https://admin.example.test".to_owned(),
                    listen: "127.0.0.1:9443".parse().unwrap(),
                });
                change(c);
            })
        };
        assert_eq!(
            with_admin(&|c| c.limits.maximum_read_connections = 2),
            "admin requires at least three read connections and two writer slots"
        );
        assert_eq!(
            with_admin(&|c| c.limits.maximum_pending_writes = 1),
            "admin requires at least three read connections and two writer slots"
        );
        let mut largest = config.clone();
        largest.storage.maximum_team_members = foks_server_db::RealtimeLimits::FANOUT_MEMBERS;
        largest.validate().unwrap();
    }
}
