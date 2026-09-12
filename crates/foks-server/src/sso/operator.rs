//! Offline rollout mutations and online, read-only status.
use crate::{
    installation::InstallationConfig,
    keys::{DirectoryKeyProvider, KeyGenerationManifest, KeyPurpose},
    DatabaseWriterGuard, Error, Result,
};
use foks_server_db::{SsoPolicy, SsoPolicyTransition, SsoRolloutStatus};

pub fn status(config: &InstallationConfig) -> Result<Option<SsoRolloutStatus>> {
    let db = foks_server_db::ReadDatabase::open(&config.database, Default::default())?;
    let host = db
        .host_bootstrap()?
        .ok_or(Error::Config("host has not bootstrapped"))?
        .host_id;
    let host = host
        .as_slice()
        .try_into()
        .map_err(|_| Error::Config("invalid host ID"))?;
    let snapshot = db.snapshot()?;
    Ok(snapshot.sso_rollout_status(host)?)
}
pub fn start(config: &InstallationConfig, confirm_host: &[u8; 33]) -> Result<SsoPolicy> {
    let guard = DatabaseWriterGuard::acquire(&config.database)?;
    let mut db = guard.open_database(Default::default())?;
    let stored = db
        .host_bootstrap()?
        .ok_or(Error::Config("host has not bootstrapped"))?;
    if stored.host_id != confirm_host {
        return Err(Error::Config("host confirmation mismatch"));
    }
    let operator = config
        .oidc
        .as_ref()
        .ok_or(Error::Config("OIDC configuration missing"))?;
    let hash = validated_provider(config, &stored.key_manifest)?;
    Ok(db.sso_activate(
        confirm_host,
        &operator.rollout_id,
        &hash,
        &operator.issuer,
        operator.rollout_mode.into(),
    )?)
}
pub fn transition(
    config: &InstallationConfig,
    expected: &SsoPolicy,
    transition: &SsoPolicyTransition,
) -> Result<SsoPolicy> {
    let guard = DatabaseWriterGuard::acquire(&config.database)?;
    let mut db = guard.open_database(Default::default())?;
    let stored = db
        .host_bootstrap()?
        .ok_or(Error::Config("host has not bootstrapped"))?;
    if stored.host_id != expected.host {
        return Err(Error::Config("host confirmation mismatch"));
    }
    let operator = config
        .oidc
        .as_ref()
        .ok_or(Error::Config("OIDC configuration missing"))?;
    if operator.rollout_id != expected.rollout_id {
        return Err(Error::Config("rollout ID mismatch"));
    }
    let target_mode = match transition {
        SsoPolicyTransition::Enforce { .. } => foks_server_db::SsoRolloutMode::Enforced,
        _ => expected.mode,
    };
    if foks_server_db::SsoRolloutMode::from(operator.rollout_mode) != target_mode {
        return Err(Error::Config(
            "configuration must specify the resulting rollout mode",
        ));
    }
    let hash = validated_provider(config, &stored.key_manifest)?;
    let expected_hash = match transition {
        SsoPolicyTransition::ReplaceProvider { config_hash, .. } => *config_hash,
        _ => expected.config_hash,
    };
    if hash != expected_hash || operator.issuer != expected.issuer {
        return Err(Error::Config("provider fingerprint or issuer mismatch"));
    }
    Ok(db.sso_transition_policy(expected, transition)?)
}
fn validated_provider(config: &InstallationConfig, manifest: &[u8]) -> Result<[u8; 32]> {
    let root = crate::keys::read_root_key_file(&config.root_key_file)?;
    let keys = DirectoryKeyProvider::open(&config.key_directory, *root)?;
    let manifest = KeyGenerationManifest::decode(manifest)?;
    if Some(keys.load_existing(KeyPurpose::Recovery)?.generation())
        != manifest.generation(KeyPurpose::Recovery)
    {
        return Err(Error::Key("stored recovery key generation mismatch"));
    }
    config
        .oidc
        .as_ref()
        .ok_or(Error::Config("OIDC configuration missing"))?
        .validate_provider(foks_oidc::NetworkPolicy::default(), &keys)
}
