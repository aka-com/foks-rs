//! Authenticated staging and independent native publication.
use super::{
    archive_manifest::ArchiveManifest,
    files,
    inventory::StateSnapshot,
    lease::{canonical_reservation, locator_path, DirectoryIdentity},
    publication::durable_metadata,
    ClientStateMaintenanceGuard,
};
use crate::{
    checkpoint::{CheckpointStore as _, NativeManifestStore},
    Error, Result,
};
use foks_keystore::state_archive::{ArchiveReader, StateTransferKey};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
mod identity;
mod staging;
use identity::*;
pub(crate) use identity::{IMPORT_COMPLETION_MARKER, INTENT};
type Hook<'a> = dyn FnMut(&'static str) -> Result<()> + 'a;

mod recovery;
use recovery::{finish, remove_prepublication};
pub use recovery::{import_status, recover_import, StateImportStatus};

#[derive(serde::Serialize)]
pub struct StateImportReport {
    pub destination: PathBuf,
    pub profiles: usize,
    pub verification_required: bool,
    pub recovered: bool,
}
/// Authenticate and install an independent native copy. Existing state is never merged.
pub fn import_state(
    archive: impl AsRef<Path>,
    key: &StateTransferKey,
    destination: impl AsRef<Path>,
) -> Result<StateImportReport> {
    run(
        archive.as_ref(),
        key,
        destination.as_ref(),
        false,
        &mut |_| Ok(()),
    )
}
pub fn import_state_for_desktop(
    archive: impl AsRef<Path>,
    key: &StateTransferKey,
    destination: impl AsRef<Path>,
) -> Result<StateImportReport> {
    run(
        archive.as_ref(),
        key,
        destination.as_ref(),
        true,
        &mut |_| Ok(()),
    )
}
fn run(
    archive: &Path,
    key: &StateTransferKey,
    destination: &Path,
    mut select_desktop: bool,
    external_hook: &mut Hook<'_>,
) -> Result<StateImportReport> {
    let destination = canonical_reservation(destination)?;
    let (mut reader, manifest) = staging::open_archive(archive, key)?;
    let source_master = Zeroizing::new(
        <[u8; 32]>::try_from(
            manifest
                .native
                .records
                .get(crate::MASTER_KEY_RECORD)
                .ok_or(Error::StateRecoveryRequired)?
                .as_slice(),
        )
        .map_err(|_| Error::StateRecoveryRequired)?,
    );
    let projection_digest = Sha256::digest(manifest.native.encode()?.as_slice()).into();
    // A retry can authorize only the staging identity MACed by this exact archive.
    let prior = {
        let guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&destination))?;
        let path = locator_path(&guard.base, &destination);
        if files::exists(&path)? {
            Some(Locator::decode(&path)?.marker)
        } else {
            None
        }
    };
    if let Some(prior) = prior {
        let input = canonical_reservation(archive)?;
        if input.starts_with(&prior.identity.staging)
            || input.starts_with(&prior.identity.empty_hold)
        {
            return Err(Error::InvalidConfig(
                "archive input must remain outside import staging",
            ));
        }
        if prior.identity.destination != destination
            || prior.identity.archive_id != reader.archive_id()
            || prior.identity.source_projection_digest != projection_digest
            || prior.identity.tag(&source_master)? != prior.identity_mac
        {
            return Err(Error::StateRecoveryRequired);
        }
        let mut guard = ClientStateMaintenanceGuard::acquire(&prior.identity.paths())?;
        guard.reserve_namespace(&prior.identity.state_id)?;
        if read_native(&guard)?.is_some() {
            drop(guard);
            return recover_import(&destination);
        }
        select_desktop = prior.identity.select_desktop;
        remove_prepublication(&guard, &prior)?;
    }
    let parent = destination
        .parent()
        .ok_or(Error::StatePathChanged)?
        .to_owned();
    // The private staging root protects payloads; the parent must be owned and
    // cannot allow another user to replace its children.
    validate_parent(&parent)?;
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(|_| Error::Randomness)?;
    let mut namespace = [0; 16];
    getrandom::fill(&mut namespace).map_err(|_| Error::Randomness)?;
    let state_id = crate::hex(&namespace);
    let staging = parent.join(format!(".foks-import-{}", crate::hex(&nonce)));
    let empty_hold = parent.join(format!(".foks-import-empty-{}", crate::hex(&nonce)));
    let mut guard = ClientStateMaintenanceGuard::acquire(&[
        destination.clone(),
        staging.clone(),
        empty_hold.clone(),
    ])?;
    if files::exists(&locator_path(&guard.base, &destination))?
        || files::exists(&staging)?
        || files::exists(&empty_hold)?
    {
        return Err(Error::StateRecoveryRequired);
    }
    let empty_destination = if files::exists(&destination)? {
        files::private_directory(&destination)?;
        if fs::read_dir(&destination)?.next().is_some() {
            return Err(Error::InvalidConfig(
                "import destination must be absent or empty",
            ));
        }
        Some(DirectoryIdentity::read(&destination)?)
    } else {
        None
    };
    guard.reserve_namespace(&state_id)?;
    if read_native(&guard)?.is_some() {
        return Err(Error::StateRecoveryRequired);
    }
    let mut interrupted = false;
    let mut hook = |point| {
        let result = external_hook(point);
        if result.is_err() {
            interrupted = true;
        }
        result
    };
    hook("before-import-staging-create")?;
    crate::prepare_private_directory(&staging)?;
    let parent_identity = DirectoryIdentity::read(&parent)?;
    let root_identity = DirectoryIdentity::read(&staging)?;
    let identity = Identity {
        version: 1,
        nonce,
        state_id,
        archive_id: reader.archive_id(),
        source_state_id: manifest.source_state_id.clone(),
        source_projection_digest: projection_digest,
        destination,
        staging,
        empty_hold,
        parent: parent_identity,
        root: root_identity,
        empty_destination,
        select_desktop,
    };
    hook("after-import-staging-create")?;
    let marker = Marker {
        identity_mac: identity.tag(&source_master)?,
        identity,
        phase: Phase::Staging,
    };
    let mut native_attempted = false;
    let result = (|| {
        write_marker(&mut guard, &marker.identity.staging, &marker, &mut hook)?;
        write_locator(&guard, &marker, &mut hook)?;
        sync_parent(&marker.identity, &mut hook)?;
        staging::extract(&mut reader, &manifest, &marker.identity, &mut hook)?;
        reader.finish()?;
        let source = staging::validate_source(&mut guard, &manifest, &marker.identity)?;
        let mut master = Zeroizing::new([0; 32]);
        getrandom::fill(&mut *master).map_err(|_| Error::Randomness)?;
        let authority = staging::rekey(&source, &marker.identity, &master, &mut hook)?;
        let destination = guard.inspect_with_projection(
            &marker.identity.staging,
            Some((&marker.identity.destination, &authority.projection)),
        )?;
        let mut prepared = marker.clone();
        prepared.phase = Phase::Prepared;
        let intent = Intent {
            marker: prepared,
            content_digest: content_digest(&destination)?,
            claims_digest: claims_digest(&authority.claims)?,
        };
        let mut native = NativeManifestStore::initialized(&marker.identity.staging, &master);
        for (key, value) in authority.claims {
            native.put(&format!("{CLAIM_PREFIX}{key}"), &value)?;
        }
        native.put(INTENT, &serde_json::to_vec(&intent)?)?;
        // A synced locator and matching filesystem marker precede the first
        // native publication, including an uncertain put outcome.
        write_marker(
            &mut guard,
            &marker.identity.staging,
            &intent.marker,
            &mut hook,
        )?;
        sync_parent(&marker.identity, &mut hook)?;
        hook("before-import-native-prepared")?;
        native_attempted = true;
        guard.publish_new_native_manifest(&native)?;
        hook("after-import-native-prepared")?;
        drop(source);
        drop(destination);
        drop(master);
        finish(&mut guard, intent, false, &mut hook)
    })();
    if result.is_err() && !native_attempted && !interrupted {
        // Never clean up an uncertain native publication. Before that boundary,
        // this invocation still owns its authenticated private staging identity.
        let _ = remove_prepublication(&guard, &marker);
    }
    result
}
fn validate_parent(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o022 != 0
    {
        return Err(Error::InvalidConfig(
            "import parent must be owned and not writable by other users",
        ));
    }
    Ok(())
}
fn read_native(guard: &ClientStateMaintenanceGuard) -> Result<Option<NativeManifestStore>> {
    let id = guard.namespace_id()?;
    let _lock = crate::runtime::NativeManifestLock::acquire(id)?;
    let mut store = foks_keystore::NativeCredentialStore::open(id)?;
    match store.get(crate::checkpoint::NATIVE_MANIFEST_RECORD) {
        Ok(bytes) => Ok(Some(NativeManifestStore::decode(&bytes)?)),
        Err(foks_keystore::Error::Missing) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn inventory_keys(
    guard: &ClientStateMaintenanceGuard,
    native: &NativeManifestStore,
    root: &Path,
) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    if let Some(completion_marker) = native_completion_marker(native)? {
        if completion_marker.marker.phase != Phase::Verified
            || completion_marker.marker.identity.state_id != guard.namespace_id()?
        {
            return Err(Error::StateRecoveryRequired);
        }
        keys.push(IMPORT_COMPLETION_MARKER.into());
    }
    if let Some(intent) = native_intent(native, INTENT)? {
        if guard.import_nonce != Some(intent.marker.identity.nonce)
            || intent.marker.identity.state_id != guard.namespace_id()?
            || intent.marker.phase < Phase::ClaimsInstalled
            || root != intent.marker.identity.destination
        {
            return Err(Error::StateRecoveryRequired);
        }
        keys.push(INTENT.into());
    }
    Ok(keys)
}

#[cfg(test)]
mod tests;
