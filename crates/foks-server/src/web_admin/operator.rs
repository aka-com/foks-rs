//! Offline grants share the live server's writer exclusion and local-host checks.
use crate::{installation::InstallationConfig, DatabaseWriterGuard, Error, Result};
pub fn set_grant(
    config: &InstallationConfig,
    uid: &[u8; 33],
    active: bool,
    reason: &str,
    now: u64,
) -> Result<foks_server_db::HostAdminGrant> {
    let guard = DatabaseWriterGuard::acquire(&config.database)?;
    let mut db = guard.open_database(Default::default())?;
    let host: [u8; 33] = db
        .host_bootstrap()?
        .ok_or(Error::Config("host has not bootstrapped"))?
        .host_id
        .try_into()
        .map_err(|_| Error::Config("invalid host identity"))?;
    Ok(db.admin_set_grant(&host, uid, active, reason, now)?)
}
pub fn grants(
    config: &InstallationConfig,
    after: &[u8],
) -> Result<Vec<foks_server_db::HostAdminGrant>> {
    Ok(
        foks_server_db::ReadDatabase::open(&config.database, Default::default())?
            .admin_grants(after)?,
    )
}
