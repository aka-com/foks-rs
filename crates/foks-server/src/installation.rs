//! Explicit, standalone installation layout and client bootstrap artifacts.

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose, PKCS_ED25519};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};

const INSTALLATION_VERSION: u32 = 1;
const MAXIMUM_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
}

impl InstallationConfig {
    pub fn validate(&self) -> crate::Result<()> {
        validate_hostname(&self.canonical_name)?;
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
        if self.version != INSTALLATION_VERSION
            || self.backup_interval_seconds == 0
            || self.backup_retain == 0
            || !self.management_address.ip().is_loopback()
            || [
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
    let database =
        foks_server_db::ReadDatabase::open(&config.database, foks_server_db::Config::default())?;
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
    let database =
        foks_server_db::ReadDatabase::open(&config.database, foks_server_db::Config::default())?;
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

fn write_new_file(path: &Path, bytes: &[u8], unix_mode: u32) -> crate::Result<()> {
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

fn hex(bytes: &[u8]) -> String {
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
        let config = load_config(config_path).unwrap();
        validate_artifacts(&config).unwrap();
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
}
