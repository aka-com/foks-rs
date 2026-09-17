use super::*;
use crate::{Error, Result};
use std::path::Path;
#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "status", rename_all = "kebab-case")]
pub enum MaintenanceStatus {
    Import(StateImportStatus),
    Relocation(RelocationStatus),
}
fn is_import(root: &Path) -> Result<bool> {
    let root = lease::canonical_reservation(root)?;
    let mut guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&root))?;
    let path = lease::locator_path(&guard.base, &root);
    if files::exists(&path)? {
        let value: serde_json::Value =
            serde_json::from_slice(&crate::read_bounded_regular_file(&path, 32 * 1024)?)?;
        return match value.get("kind").and_then(|v| v.as_str()) {
            Some("import") => Ok(true),
            None => Ok(false),
            _ => Err(Error::StateRecoveryRequired),
        };
    }
    let Some(state) = crate::checkpoint::inspect_state_file(&root)? else {
        return Ok(false);
    };
    if state.credential_backend != crate::CredentialBackend::Native {
        return Err(Error::PortabilityUnsupported);
    }
    guard.reserve_namespace(&state.state_id)?;
    guard.with_native_manifest(|native| {
        Ok(native.records.contains_key(import::INTENT)
            || (!native.records.contains_key(relocation::INTENT)
                && native.records.contains_key(import::RECEIPT)))
    })
}
pub fn maintenance_status(root: impl AsRef<Path>) -> Result<Option<MaintenanceStatus>> {
    let root = root.as_ref();
    if is_import(root)? {
        if let Some(status) = import_status(root)? {
            return Ok(Some(MaintenanceStatus::Import(status)));
        }
    }
    Ok(relocation_status(root)?.map(MaintenanceStatus::Relocation))
}
pub fn recover_state(root: impl AsRef<Path>) -> Result<serde_json::Value> {
    match maintenance_status(root.as_ref())? {
        Some(MaintenanceStatus::Import(_)) => Ok(serde_json::to_value(recover_import(root)?)?),
        Some(MaintenanceStatus::Relocation(_)) => {
            Ok(serde_json::to_value(recover_relocation(root)?)?)
        }
        None => Err(Error::InvalidConfig("no state maintenance attempt exists")),
    }
}
