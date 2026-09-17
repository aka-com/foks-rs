//! Desktop root selection is stored outside the state it selects.
use super::{
    files,
    lease::{canonical_reservation, lock_directory, DirectoryIdentity},
    ClientStateMaintenanceGuard,
};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
const SELECTION: &str = "desktop-root-v1.json";
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    version: u32,
    root: PathBuf,
    state_id: String,
    identity: DirectoryIdentity,
}

pub fn default_desktop_state_root() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(Error::InvalidConfig("home directory is unavailable"))?;
    #[cfg(target_os = "macos")]
    {
        Ok(home.join("Library/Application Support/foks-rs"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("foks-rs"))
    }
}
fn read(base: &Path) -> Result<Option<Selection>> {
    let path = base.join(SELECTION);
    if !files::exists(&path)? {
        return Ok(None);
    }
    files::regular(&path)?;
    let selection: Selection =
        serde_json::from_slice(&crate::read_bounded_regular_file(&path, 16 * 1024)?)?;
    if selection.version != 1 || !selection.root.is_absolute() {
        return Err(Error::StateRecoveryRequired);
    }
    crate::validate_name(&selection.state_id)?;
    Ok(Some(selection))
}
/// Invalid or unavailable selected state never falls back to a fresh default root.
pub fn selected_desktop_state_root() -> Result<PathBuf> {
    resolve(&lock_directory()?, default_desktop_state_root()?)
}
fn resolve(base: &Path, default: PathBuf) -> Result<PathBuf> {
    let Some(selection) = read(base)? else {
        return Ok(default);
    };
    if canonical_reservation(&selection.root)? != selection.root
        || DirectoryIdentity::read(&selection.root)? != selection.identity
    {
        return Err(Error::StateRecoveryRequired);
    }
    let state = crate::checkpoint::inspect_state_file(&selection.root)?
        .ok_or(Error::StateRecoveryRequired)?;
    if state.state_id != selection.state_id
        || state.credential_backend != crate::CredentialBackend::Native
    {
        return Err(Error::StateRecoveryRequired);
    }
    Ok(selection.root)
}
pub(super) fn relocated(
    guard: &ClientStateMaintenanceGuard,
    source: &Path,
    destination: &Path,
) -> Result<()> {
    use fs2::FileExt as _;
    let lock = super::lease::private_lock(&guard.base.join("desktop-root-selection.lock"))?;
    lock.try_lock_exclusive().map_err(|_| Error::StateBusy)?;
    let selected = read(&guard.base)?;
    let applies = match selected {
        Some(s) => {
            s.root == source || (s.root == destination && s.state_id == guard.namespace_id()?)
        }
        None => canonical_reservation(&default_desktop_state_root()?)? == source,
    };
    if applies {
        select_guarded(guard, destination)?;
    }
    Ok(())
}
pub(super) fn select_for_import(guard: &ClientStateMaintenanceGuard, root: &Path) -> Result<()> {
    use fs2::FileExt as _;
    let lock = super::lease::private_lock(&guard.base.join("desktop-root-selection.lock"))?;
    lock.try_lock_exclusive().map_err(|_| Error::StateBusy)?;
    select_guarded(guard, root)
}
pub(super) fn select_guarded(guard: &ClientStateMaintenanceGuard, root: &Path) -> Result<()> {
    guard.require_path(root)?;
    let selection = Selection {
        version: 1,
        root: root.into(),
        state_id: guard.namespace_id()?.into(),
        identity: DirectoryIdentity::read(root)?,
    };
    crate::atomic_private_write(
        &guard.base.join(SELECTION),
        &serde_json::to_vec(&selection)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn selected_root_is_persistent_and_missing_root_never_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let parent = crate::prepare_private_directory(&dir.path().join("parent")).unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&parent, &alias).unwrap();
        let root = crate::prepare_private_directory(&alias.join("moved")).unwrap();
        assert_eq!(root, parent.join("moved"));
        crate::create_private_config(
            &root.join(crate::STATE_CONFIG_FILE),
            b"version = 3\nstate_id = \"aabb\"\ncredential_backend = \"native\"\n",
        )
        .unwrap();
        let selection = Selection {
            version: 1,
            root: root.clone(),
            state_id: "aabb".into(),
            identity: DirectoryIdentity::read(&root).unwrap(),
        };
        crate::create_private_config(
            &dir.path().join(SELECTION),
            &serde_json::to_vec(&selection).unwrap(),
        )
        .unwrap();
        let default = dir.path().join("old");
        assert_eq!(resolve(dir.path(), default.clone()).unwrap(), root);
        std::fs::rename(&root, dir.path().join("substituted")).unwrap();
        assert!(resolve(dir.path(), default.clone()).is_err());
        assert!(!default.exists());
    }
    #[test]
    fn malformed_selection_is_not_a_missing_selection() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(dir.path()).unwrap().is_none());
        crate::create_private_config(&dir.path().join(SELECTION), b"{}").unwrap();
        assert!(read(dir.path()).is_err());
    }
}
