//! Recovery authority comes from the native attempt, never its filesystem locator.
use super::*;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};

pub(super) fn remove_prepublication(
    guard: &ClientStateMaintenanceGuard,
    marker: &Marker,
) -> Result<()> {
    let identity = &marker.identity;
    identity.verify_parent()?;
    if read_native(guard)?.is_some() {
        return Err(Error::StateRecoveryRequired);
    }
    if files::exists(&identity.empty_hold)? {
        return Err(Error::StateRecoveryRequired);
    }
    if files::exists(&identity.staging)? {
        if DirectoryIdentity::read(&identity.staging)? != identity.root {
            return Err(Error::StatePathChanged);
        }
        let path = identity.staging.join(MARKER);
        if files::exists(&path)? {
            let stored: Marker =
                serde_json::from_slice(&crate::read_bounded_regular_file(&path, MAX_METADATA)?)?;
            if stored.identity != *identity || stored.identity_mac != marker.identity_mac {
                return Err(Error::StateRecoveryRequired);
            }
        }
        fs::remove_dir_all(&identity.staging)?;
        File::open(
            identity
                .destination
                .parent()
                .ok_or(Error::StatePathChanged)?,
        )?
        .sync_all()?;
    }
    let path = locator_path(&guard.base, &identity.destination);
    if files::exists(&path)? {
        let existing = Locator::decode(&path)?;
        if existing.marker.identity != *identity
            || existing.marker.identity_mac != marker.identity_mac
        {
            return Err(Error::StateRecoveryRequired);
        }
        fs::remove_file(path)?;
        File::open(&guard.base)?.sync_all()?;
    }
    Ok(())
}
fn validate_native(native: &NativeManifestStore, intent: &Intent) -> Result<()> {
    if native_intent(native, INTENT)?.as_ref() != Some(intent)
        || native.generation
            != (intent.marker.phase as u64)
                .checked_sub(1)
                .ok_or(Error::StateRecoveryRequired)?
    {
        return Err(Error::StateRecoveryRequired);
    }
    Ok(())
}
fn pending_projection(
    native: &NativeManifestStore,
    intent: &Intent,
) -> Result<NativeManifestStore> {
    let master = native
        .records
        .get(crate::MASTER_KEY_RECORD)
        .ok_or(Error::StateRecoveryRequired)?;
    let master = Zeroizing::new(
        <[u8; 32]>::try_from(master.as_slice()).map_err(|_| Error::StateRecoveryRequired)?,
    );
    let mut projection =
        NativeManifestStore::initialized(&intent.marker.identity.destination, &master);
    let mut claims = BTreeMap::new();
    for (key, value) in &native.records {
        if matches!(
            key.as_str(),
            crate::MASTER_KEY_RECORD | crate::STATE_ROOT_RECORD | INTENT
        ) {
            continue;
        }
        let key = key
            .strip_prefix(CLAIM_PREFIX)
            .ok_or(Error::StateRecoveryRequired)?;
        if matches!(
            key,
            crate::MASTER_KEY_RECORD | crate::STATE_ROOT_RECORD | INTENT | RECEIPT
        ) {
            return Err(Error::StateRecoveryRequired);
        }
        claims.insert(key.to_owned(), value.clone());
        projection.put(key, value)?;
    }
    if claims_digest(&claims)? != intent.claims_digest {
        return Err(Error::StateRecoveryRequired);
    }
    Ok(projection)
}
pub(super) fn finish(
    guard: &mut ClientStateMaintenanceGuard,
    mut intent: Intent,
    recovered: bool,
    hook: &mut Hook<'_>,
) -> Result<StateImportReport> {
    let identity = intent.marker.identity.clone();
    identity.verify_parent()?;
    if guard.namespace_id()? != identity.state_id {
        return Err(Error::StateRecoveryRequired);
    }
    guard.import_nonce = Some(identity.nonce);
    write_locator(guard, &intent.marker, hook)?;
    let staged = files::exists(&identity.staging)?;
    let root = if staged {
        &identity.staging
    } else {
        &identity.destination
    };
    if DirectoryIdentity::read(root)? != identity.root {
        return Err(Error::StatePathChanged);
    }
    if staged && intent.marker.phase != Phase::Prepared {
        return Err(Error::StateRecoveryRequired);
    }
    write_marker(guard, root, &intent.marker, hook)?;
    let native = read_native(guard)?.ok_or(Error::StateRecoveryRequired)?;
    validate_native(&native, &intent)?;
    if intent.marker.phase <= Phase::FilesInstalled {
        let projection = pending_projection(&native, &intent)?;
        let snapshot =
            guard.inspect_with_projection(root, Some((&identity.destination, &projection)))?;
        if content_digest(&snapshot)? != intent.content_digest {
            return Err(Error::StatePathChanged);
        }
    }
    if staged {
        reserve_empty_destination(&identity, hook)?;
        hook("before-import-rename")?;
        rename_owned(
            &identity,
            &identity.staging,
            &identity.destination,
            identity.root,
        )?;
        hook("after-import-rename")?;
    }
    verify_installed(&identity)?;
    sync_parent(&identity, hook)?;
    if intent.marker.phase == Phase::Prepared {
        transition(guard, &mut intent, Phase::FilesInstalled, hook)?;
        write_marker(guard, &identity.destination, &intent.marker, hook)?;
    }
    if intent.marker.phase == Phase::FilesInstalled {
        transition(guard, &mut intent, Phase::ClaimsInstalled, hook)?;
        write_marker(guard, &identity.destination, &intent.marker, hook)?;
    }
    let snapshot = guard.inspect(&identity.destination)?;
    if content_digest(&snapshot)? != intent.content_digest {
        return Err(Error::StatePathChanged);
    }
    let profiles = snapshot.profiles.len();
    let required = snapshot.profiles.values().any(|p| p.checkpoint.is_some());
    if intent.marker.phase == Phase::ClaimsInstalled {
        transition(guard, &mut intent, Phase::Verified, hook)?;
    }
    write_marker(guard, &identity.destination, &intent.marker, hook)?;
    cleanup_empty_hold(&identity, hook)?;
    hook("before-import-marker-remove")?;
    fs::remove_file(identity.destination.join(MARKER))?;
    hook("before-import-marker-remove-sync")?;
    File::open(&identity.destination)?.sync_all()?;
    hook("after-import-marker-remove-sync")?;
    hook("before-import-native-receipt")?;
    guard.with_native_manifest(|native| {
        validate_native(native, &intent)?;
        native.put(RECEIPT, &serde_json::to_vec(&intent)?)?;
        native.remove(INTENT)?;
        Ok(())
    })?;
    hook("after-import-native-receipt")?;
    finalize_selection_and_locator(guard, &intent, hook)?;
    Ok(StateImportReport {
        destination: identity.destination,
        profiles,
        verification_required: required,
        recovered,
    })
}
fn transition(
    guard: &ClientStateMaintenanceGuard,
    intent: &mut Intent,
    next: Phase,
    hook: &mut Hook<'_>,
) -> Result<()> {
    hook("before-import-native-phase")?;
    guard.with_native_manifest(|native| {
        validate_native(native, intent)?;
        if next == Phase::ClaimsInstalled {
            let projection = pending_projection(native, intent)?;
            let keys = native
                .records
                .keys()
                .filter(|key| key.starts_with(CLAIM_PREFIX))
                .cloned()
                .collect::<Vec<_>>();
            for key in keys {
                native.remove(&key)?;
            }
            for (key, value) in &projection.records {
                if key == crate::MASTER_KEY_RECORD {
                    continue;
                }
                native.put(key, value)?;
            }
        }
        let mut updated = intent.clone();
        updated.marker.phase = next;
        native.put(INTENT, &serde_json::to_vec(&updated)?)?;
        Ok(())
    })?;
    intent.marker.phase = next;
    hook("after-import-native-phase")
}
fn reserve_empty_destination(identity: &Identity, hook: &mut Hook<'_>) -> Result<()> {
    identity.verify_parent()?;
    if let Some(empty) = identity.empty_destination {
        if files::exists(&identity.empty_hold)? {
            if DirectoryIdentity::read(&identity.empty_hold)? != empty
                || fs::read_dir(&identity.empty_hold)?.next().is_some()
            {
                return Err(Error::StatePathChanged);
            }
        } else {
            if DirectoryIdentity::read(&identity.destination)? != empty
                || fs::read_dir(&identity.destination)?.next().is_some()
            {
                return Err(Error::StatePathChanged);
            }
            hook("before-import-empty-rename")?;
            rename_owned(identity, &identity.destination, &identity.empty_hold, empty)?;
            hook("after-import-empty-rename")?;
            sync_parent(identity, hook)?;
        }
    } else if files::exists(&identity.empty_hold)? {
        return Err(Error::StatePathChanged);
    }
    Ok(())
}
fn verify_installed(identity: &Identity) -> Result<()> {
    identity.verify_parent()?;
    if files::exists(&identity.staging)?
        || DirectoryIdentity::read(&identity.destination)? != identity.root
    {
        return Err(Error::StatePathChanged);
    }
    if files::exists(&identity.empty_hold)?
        && (Some(DirectoryIdentity::read(&identity.empty_hold)?) != identity.empty_destination
            || fs::read_dir(&identity.empty_hold)?.next().is_some())
    {
        return Err(Error::StatePathChanged);
    }
    Ok(())
}
fn cleanup_empty_hold(identity: &Identity, hook: &mut Hook<'_>) -> Result<()> {
    if files::exists(&identity.empty_hold)? {
        verify_installed(identity)?;
        hook("before-import-empty-remove")?;
        fs::remove_dir(&identity.empty_hold)?;
        hook("after-import-empty-remove")?;
        sync_parent(identity, hook)?;
    }
    Ok(())
}
fn rename_owned(
    identity: &Identity,
    source: &Path,
    destination: &Path,
    expected: DirectoryIdentity,
) -> Result<()> {
    identity.verify_parent()?;
    let parent_path = identity
        .destination
        .parent()
        .ok_or(Error::StatePathChanged)?;
    if source.parent() != Some(parent_path) || destination.parent() != Some(parent_path) {
        return Err(Error::StatePathChanged);
    }
    let parent = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(parent_path)?;
    let metadata = parent.metadata()?;
    if metadata.dev() != identity.parent.device
        || metadata.ino() != identity.parent.inode
        || DirectoryIdentity::read(source)? != expected
    {
        return Err(Error::StatePathChanged);
    }
    rustix::fs::renameat_with(
        &parent,
        source.file_name().ok_or(Error::StatePathChanged)?,
        &parent,
        destination.file_name().ok_or(Error::StatePathChanged)?,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(std::io::Error::from)?;
    if DirectoryIdentity::read(destination)? != expected {
        return Err(Error::StatePathChanged);
    }
    Ok(())
}
fn finalize_selection_and_locator(
    guard: &ClientStateMaintenanceGuard,
    intent: &Intent,
    hook: &mut Hook<'_>,
) -> Result<()> {
    if intent.marker.identity.select_desktop {
        hook("before-import-selection")?;
        super::super::selection::select_for_import(guard, &intent.marker.identity.destination)?;
        hook("after-import-selection")?;
    }
    let path = locator_path(&guard.base, &intent.marker.identity.destination);
    if files::exists(&path)? {
        let locator = Locator::decode(&path)?;
        if locator.marker.identity != intent.marker.identity
            || locator.marker.identity_mac != intent.marker.identity_mac
        {
            return Err(Error::StateRecoveryRequired);
        }
        hook("before-import-locator-remove")?;
        fs::remove_file(&path)?;
        hook("before-import-locator-remove-sync")?;
        File::open(&guard.base)?.sync_all()?;
        hook("after-import-locator-remove-sync")?;
    }
    Ok(())
}

pub fn recover_import(destination: impl AsRef<Path>) -> Result<StateImportReport> {
    recover(destination.as_ref(), &mut |_| Ok(()))
}
fn discover(destination: &Path) -> Result<(Intent, bool)> {
    let mut guard = ClientStateMaintenanceGuard::acquire(&[destination.to_owned()])?;
    let locator_path = locator_path(&guard.base, destination);
    let locator = if files::exists(&locator_path)? {
        Some(Locator::decode(&locator_path)?)
    } else {
        None
    };
    let id = match &locator {
        Some(locator) if locator.marker.identity.destination == destination => {
            locator.marker.identity.state_id.clone()
        }
        Some(_) => return Err(Error::StateRecoveryRequired),
        None => {
            crate::checkpoint::inspect_state_file(destination)?
                .ok_or(Error::StateRecoveryRequired)?
                .state_id
        }
    };
    guard.reserve_namespace(&id)?;
    let native=read_native(&guard)?.ok_or(Error::InvalidConfig("staged import requires the original archive and transfer key; retry import to reauthenticate staging"))?;
    let active = native_intent(&native, INTENT)?;
    let completed = active.is_none();
    let intent = active
        .or(native_intent(&native, RECEIPT)?)
        .ok_or(Error::StateRecoveryRequired)?;
    if intent.marker.identity.state_id != id
        || intent.marker.identity.destination != destination
        || locator.as_ref().is_some_and(|l| {
            l.marker.identity != intent.marker.identity
                || l.marker.identity_mac != intent.marker.identity_mac
        })
    {
        return Err(Error::StateRecoveryRequired);
    }
    Ok((intent, completed))
}
fn recover(destination: &Path, hook: &mut Hook<'_>) -> Result<StateImportReport> {
    let destination = canonical_reservation(destination)?;
    let (intent, completed) = discover(&destination)?;
    let identity = &intent.marker.identity;
    let mut guard = ClientStateMaintenanceGuard::acquire(&identity.paths())?;
    guard.reserve_namespace(&identity.state_id)?;
    let native = read_native(&guard)?.ok_or(Error::StateRecoveryRequired)?;
    if completed {
        if native_intent(&native, INTENT)?.is_some()
            || native_intent(&native, RECEIPT)?.as_ref() != Some(&intent)
            || intent.marker.phase != Phase::Verified
        {
            return Err(Error::StateRecoveryRequired);
        }
        verify_installed(identity)?;
        let snapshot = guard.inspect(&destination)?;
        if files::exists(&locator_path(&guard.base, &destination))?
            && content_digest(&snapshot)? != intent.content_digest
        {
            return Err(Error::StatePathChanged);
        }
        cleanup_empty_hold(identity, hook)?;
        finalize_selection_and_locator(&guard, &intent, hook)?;
        return Ok(StateImportReport {
            destination,
            profiles: snapshot.profiles.len(),
            verification_required: snapshot
                .native
                .records
                .keys()
                .any(|key| key.starts_with(super::super::readiness::PREFIX)),
            recovered: true,
        });
    }
    validate_native(&native, &intent)?;
    finish(&mut guard, intent, true, hook)
}

#[derive(serde::Serialize)]
pub struct StateImportStatus {
    pub destination: PathBuf,
    pub phase: &'static str,
    pub recovery_required: bool,
    pub requires_archive: bool,
}
pub fn import_status(destination: impl AsRef<Path>) -> Result<Option<StateImportStatus>> {
    let destination = canonical_reservation(destination.as_ref())?;
    let mut guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&destination))?;
    let path = locator_path(&guard.base, &destination);
    let locator = if files::exists(&path)? {
        Some(Locator::decode(&path)?)
    } else {
        None
    };
    let id = match &locator {
        Some(l) if l.marker.identity.destination == destination => {
            l.marker.identity.state_id.clone()
        }
        Some(_) => return Err(Error::StateRecoveryRequired),
        None => match crate::checkpoint::inspect_state_file(&destination)? {
            Some(s) => {
                if s.credential_backend != crate::CredentialBackend::Native {
                    return Err(Error::PortabilityUnsupported);
                }
                s.state_id
            }
            None => return Ok(None),
        },
    };
    guard.reserve_namespace(&id)?;
    let Some(native) = read_native(&guard)? else {
        return if locator.is_some() {
            Ok(Some(StateImportStatus {
                destination,
                phase: "staging",
                recovery_required: true,
                requires_archive: true,
            }))
        } else {
            Err(Error::StateRecoveryRequired)
        };
    };
    let active = native_intent(&native, INTENT)?;
    let pending = active.is_some() || locator.is_some();
    let Some(intent) = active.or(native_intent(&native, RECEIPT)?) else {
        return if pending {
            Err(Error::StateRecoveryRequired)
        } else {
            Ok(None)
        };
    };
    if !pending && intent.marker.identity.destination != destination {
        return Ok(None);
    }
    if intent.marker.identity.destination != destination
        || intent.marker.identity.state_id != id
        || locator.as_ref().is_some_and(|l| {
            l.marker.identity != intent.marker.identity
                || l.marker.identity_mac != intent.marker.identity_mac
        })
    {
        return Err(Error::StateRecoveryRequired);
    }
    let phase = match intent.marker.phase {
        Phase::Staging => "staging",
        Phase::Prepared => "prepared",
        Phase::FilesInstalled => "files-installed",
        Phase::ClaimsInstalled => "claims-installed",
        Phase::Verified => "verified",
    };
    Ok(Some(StateImportStatus {
        destination,
        phase,
        recovery_required: pending,
        requires_archive: false,
    }))
}
