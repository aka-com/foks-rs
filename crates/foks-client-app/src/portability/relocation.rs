//! Same-filesystem move with authority retained in the native namespace.
use super::publication::durable_metadata;
use super::{
    files,
    inventory::StateSnapshot,
    lease::{canonical_reservation, locator_path, DirectoryIdentity},
    ClientStateMaintenanceGuard,
};
use crate::checkpoint::CheckpointStore as _;
use crate::{checkpoint::NativeManifestStore, Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

pub(crate) const INTENT: &str = "state-relocation-v1";
pub(super) const RECEIPT: &str = "state-relocation-receipt-v1";
const MARKER: &str = ".state-relocation-v1";
const MAX_RECORD: u64 = 32 * 1024;

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct MoveIdentity {
    version: u32,
    nonce: [u8; 32],
    state_id: String,
    source: PathBuf,
    destination: PathBuf,
    root: DirectoryIdentity,
    source_parent: DirectoryIdentity,
    destination_parent: DirectoryIdentity,
    source_binding: [u8; 32],
    destination_binding: [u8; 32],
    initial_generation: u64,
    content_digest: [u8; 32],
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Prepared,
    FilesInstalled,
    Verified,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Intent {
    identity: MoveIdentity,
    phase: Phase,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Locator {
    version: u32,
    state_id: String,
    nonce: [u8; 32],
    source: PathBuf,
    destination: PathBuf,
}
impl Intent {
    fn locator(&self) -> Locator {
        Locator {
            version: 1,
            state_id: self.identity.state_id.clone(),
            nonce: self.identity.nonce,
            source: self.identity.source.clone(),
            destination: self.identity.destination.clone(),
        }
    }
    fn validate(&self, state_id: &str) -> Result<()> {
        let i = &self.identity;
        if i.version != 1
            || i.state_id != state_id
            || i.source == i.destination
            || i.source.starts_with(&i.destination)
            || i.destination.starts_with(&i.source)
            || !i.source.is_absolute()
            || !i.destination.is_absolute()
            || i.source_binding != crate::state_root_binding(&i.source)
            || i.destination_binding != crate::state_root_binding(&i.destination)
            || i.root.device != i.source_parent.device
            || i.root.device != i.destination_parent.device
        {
            return Err(Error::StateRecoveryRequired);
        }
        crate::validate_name(state_id)?;
        Ok(())
    }
}
fn decode(bytes: &[u8]) -> Result<Intent> {
    if bytes.len() as u64 > MAX_RECORD {
        return Err(Error::StateRecoveryRequired);
    }
    serde_json::from_slice(bytes).map_err(|_| Error::StateRecoveryRequired)
}
fn intent_in(manifest: &NativeManifestStore, key: &str) -> Result<Option<Intent>> {
    manifest.records.get(key).map(|b| decode(b)).transpose()
}
fn content_digest(snapshot: &StateSnapshot) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    hash.update(b"foks-relocation-content-v1");
    hash.update(serde_json::to_vec(&snapshot.artifacts)?);
    for (key, value) in &snapshot.native.records {
        if matches!(key.as_str(), crate::STATE_ROOT_RECORD | INTENT | RECEIPT) {
            continue;
        }
        hash.update((key.len() as u64).to_be_bytes());
        hash.update(key.as_bytes());
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value);
    }
    Ok(hash.finalize().into())
}

/// Pending maintenance is rejected even when a crash preceded locator publication.
pub(crate) fn require_ready(manifest: &NativeManifestStore) -> Result<()> {
    if manifest.records.contains_key(INTENT) || manifest.records.contains_key(super::import::INTENT)
    {
        Err(Error::StateRecoveryRequired)
    } else {
        Ok(())
    }
}
pub(crate) fn require_namespace_ready(id: &str, root: &Path) -> Result<()> {
    let _lock = crate::runtime::NativeManifestLock::acquire(id)?;
    let mut native = foks_keystore::NativeCredentialStore::open(id)?;
    let manifest =
        NativeManifestStore::decode(&native.get(crate::checkpoint::NATIVE_MANIFEST_RECORD)?)?;
    require_ready(&manifest)?;
    if manifest
        .records
        .get(crate::STATE_ROOT_RECORD)
        .map(Vec::as_slice)
        != Some(crate::state_root_binding(root).as_slice())
    {
        return Err(Error::InvalidConfig(
            "native client state belongs to a different root path",
        ));
    }
    Ok(())
}

/// Allow only this guard's authenticated attempt, never arbitrary maintenance keys.
pub(super) fn inventory_keys(
    guard: &ClientStateMaintenanceGuard,
    manifest: &NativeManifestStore,
    root: &Path,
) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    if let Some(receipt) = intent_in(manifest, RECEIPT)? {
        receipt.validate(guard.namespace_id()?)?;
        if receipt.phase != Phase::Verified {
            return Err(Error::StateRecoveryRequired);
        }
        keys.push(RECEIPT.into());
    }
    if let Some(intent) = intent_in(manifest, INTENT)? {
        intent.validate(guard.namespace_id()?)?;
        if guard.relocation_nonce != Some(intent.identity.nonce)
            || DirectoryIdentity::read(root)? != intent.identity.root
        {
            return Err(Error::StateRecoveryRequired);
        }
        validate_marker(root, &intent)?;
        keys.push(INTENT.into());
    } else if files::exists(&root.join(MARKER))? {
        return Err(Error::StateRecoveryRequired);
    }
    Ok(keys)
}
pub(super) fn validate_inventory_marker(snapshot: &StateSnapshot, path: &Path) -> Result<()> {
    let intent = intent_in(&snapshot.native, INTENT)?.ok_or(Error::StateRecoveryRequired)?;
    if path != snapshot.root.join(MARKER) {
        return Err(Error::StateRecoveryRequired);
    }
    validate_marker(&snapshot.root, &intent)
}
fn validate_marker(root: &Path, intent: &Intent) -> Result<()> {
    let path = root.join(MARKER);
    let bytes = crate::read_bounded_regular_file(&path, MAX_RECORD)?;
    let marker = decode(&bytes)?;
    if marker.identity != intent.identity || marker.phase > intent.phase {
        return Err(Error::StateRecoveryRequired);
    }
    Ok(())
}

#[derive(Serialize)]
pub struct RelocationReport {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub recovered: bool,
}

/// Move only stopped, exclusively reserved native state. Pending work is preserved.
pub fn relocate_state(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<RelocationReport> {
    relocate(source.as_ref(), destination.as_ref(), &mut |_| Ok(()))
}
fn relocate(
    source: &Path,
    destination: &Path,
    hook: &mut dyn FnMut(&'static str) -> Result<()>,
) -> Result<RelocationReport> {
    let source = canonical_reservation(source)?;
    let destination = canonical_reservation(destination)?;
    if source == destination || source.starts_with(&destination) || destination.starts_with(&source)
    {
        return Err(Error::InvalidConfig(
            "source and destination must be separate state roots",
        ));
    }
    let mut guard = ClientStateMaintenanceGuard::acquire(&[source.clone(), destination.clone()])?;
    for path in [&source, &destination] {
        if files::exists(&locator_path(&guard.base, path))? {
            return Err(Error::StateRecoveryRequired);
        }
    }
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(Error::InvalidConfig(
            "relocation destination must be absent",
        ));
    }
    let source_parent = DirectoryIdentity::read(parent(&source)?)?;
    let destination_parent = DirectoryIdentity::read(parent(&destination)?)?;
    let root = DirectoryIdentity::read(&source)?;
    if root.device != source_parent.device || root.device != destination_parent.device {
        return Err(Error::InvalidConfig(
            "cross-filesystem relocation requires encrypted export/import",
        ));
    }
    let snapshot = guard.inspect(&source)?.normalize_trust(&mut guard)?;
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(|_| Error::Randomness)?;
    let intent = Intent {
        identity: MoveIdentity {
            version: 1,
            nonce,
            state_id: snapshot.state_id.clone(),
            source,
            destination,
            root,
            source_parent,
            destination_parent,
            source_binding: crate::state_root_binding(&snapshot.root),
            destination_binding: crate::state_root_binding(destination_path(
                &guard,
                &snapshot.root,
            )?),
            initial_generation: snapshot.native.generation,
            content_digest: content_digest(&snapshot)?,
        },
        phase: Phase::Prepared,
    };
    intent.validate(guard.namespace_id()?)?;
    hook("before-native-prepared")?;
    guard.with_native_manifest(|m| {
        require_ready(m)?;
        if m.generation != intent.identity.initial_generation {
            return Err(Error::StatePathChanged);
        }
        m.put(INTENT, &serde_json::to_vec(&intent)?)?;
        Ok(())
    })?;
    hook("after-native-prepared")?;
    finish(&mut guard, intent, false, hook)
}
fn destination_path<'a>(guard: &'a ClientStateMaintenanceGuard, source: &Path) -> Result<&'a Path> {
    guard
        .paths
        .iter()
        .find(|p| p.as_path() != source)
        .map(PathBuf::as_path)
        .ok_or(Error::StatePathChanged)
}
fn parent(path: &Path) -> Result<&Path> {
    path.parent().ok_or(Error::StatePathChanged)
}

/// Resume using either recorded path. Locator contents are discovery hints only.
pub fn recover_relocation(path: impl AsRef<Path>) -> Result<RelocationReport> {
    recover(path.as_ref(), &mut |_| Ok(()))
}
fn recover(
    path: &Path,
    hook: &mut dyn FnMut(&'static str) -> Result<()>,
) -> Result<RelocationReport> {
    let path = canonical_reservation(path)?;
    // Discover under a short path reservation; acquire both in canonical order next.
    let (id, hint) = {
        let guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&path))?;
        let locator = locator_path(&guard.base, &path);
        if files::exists(&locator)? {
            let hint: Locator =
                serde_json::from_slice(&crate::read_bounded_regular_file(&locator, MAX_RECORD)?)?;
            if hint.version != 1 || (hint.source != path && hint.destination != path) {
                return Err(Error::StateRecoveryRequired);
            }
            crate::validate_name(&hint.state_id)?;
            (hint.state_id.clone(), Some(hint))
        } else {
            let state = crate::checkpoint::inspect_state_file(&path)?
                .ok_or(Error::StateRecoveryRequired)?;
            if state.credential_backend != crate::CredentialBackend::Native {
                return Err(Error::PortabilityUnsupported);
            }
            (state.state_id, None)
        }
    };
    // Read only for discovery. All authority is reread under the final reservations.
    let discovered = {
        let _lock = crate::runtime::NativeManifestLock::acquire(&id)?;
        let mut native = foks_keystore::NativeCredentialStore::open(&id)?;
        let m =
            NativeManifestStore::decode(&native.get(crate::checkpoint::NATIVE_MANIFEST_RECORD)?)?;
        intent_in(&m, INTENT)?
            .or(intent_in(&m, RECEIPT)?)
            .ok_or(Error::StateRecoveryRequired)?
    };
    discovered.validate(&id)?;
    if path != discovered.identity.source && path != discovered.identity.destination
        || hint.is_some_and(|h| h != discovered.locator())
    {
        return Err(Error::StateRecoveryRequired);
    }
    let mut guard = ClientStateMaintenanceGuard::acquire(&[
        discovered.identity.source.clone(),
        discovered.identity.destination.clone(),
    ])?;
    guard.reserve_namespace(&id)?;
    let (intent, completed) = guard.with_native_manifest(|m| {
        let active = intent_in(m, INTENT)?;
        let complete = active.is_none();
        let intent = active
            .or(intent_in(m, RECEIPT)?)
            .ok_or(Error::StateRecoveryRequired)?;
        if intent.identity != discovered.identity {
            return Err(Error::StateRecoveryRequired);
        }
        let binding = if intent.phase == Phase::Prepared {
            intent.identity.source_binding
        } else {
            intent.identity.destination_binding
        };
        if m.records.get(crate::STATE_ROOT_RECORD).map(Vec::as_slice) != Some(binding.as_slice()) {
            return Err(Error::StateRecoveryRequired);
        }
        Ok((intent, complete))
    })?;
    if completed {
        if intent.phase != Phase::Verified {
            return Err(Error::StateRecoveryRequired);
        }
        verify_destination_identity(&intent)?;
        if content_digest(&guard.inspect(&intent.identity.destination)?)?
            != intent.identity.content_digest
        {
            return Err(Error::StatePathChanged);
        }
        super::selection::relocated(
            &guard,
            &intent.identity.source,
            &intent.identity.destination,
        )?;
        cleanup_locators(&guard, &intent, hook)?;
        return Ok(report(&intent, true));
    }
    finish(&mut guard, intent, true, hook)
}
fn report(intent: &Intent, recovered: bool) -> RelocationReport {
    RelocationReport {
        source: intent.identity.source.clone(),
        destination: intent.identity.destination.clone(),
        recovered,
    }
}
fn verify_parents(intent: &Intent) -> Result<()> {
    if canonical_reservation(&intent.identity.source)? != intent.identity.source
        || canonical_reservation(&intent.identity.destination)? != intent.identity.destination
    {
        return Err(Error::StatePathChanged);
    }
    if DirectoryIdentity::read(parent(&intent.identity.source)?)? != intent.identity.source_parent
        || DirectoryIdentity::read(parent(&intent.identity.destination)?)?
            != intent.identity.destination_parent
    {
        return Err(Error::StatePathChanged);
    }
    Ok(())
}
fn verify_destination_identity(intent: &Intent) -> Result<()> {
    verify_parents(intent)?;
    if fs::symlink_metadata(&intent.identity.source).is_ok()
        || DirectoryIdentity::read(&intent.identity.destination)? != intent.identity.root
    {
        return Err(Error::StatePathChanged);
    }
    Ok(())
}
fn finish(
    guard: &mut ClientStateMaintenanceGuard,
    mut intent: Intent,
    recovered: bool,
    hook: &mut dyn FnMut(&'static str) -> Result<()>,
) -> Result<RelocationReport> {
    intent.validate(guard.namespace_id()?)?;
    verify_parents(&intent)?;
    guard.relocation_nonce = Some(intent.identity.nonce);
    for path in [&intent.identity.source, &intent.identity.destination] {
        let locator = locator_path(&guard.base, path);
        if files::exists(&locator)? {
            let existing: Locator =
                serde_json::from_slice(&crate::read_bounded_regular_file(&locator, MAX_RECORD)?)?;
            if existing != intent.locator() {
                return Err(Error::StateRecoveryRequired);
            }
        }
        durable_metadata(
            &locator,
            &serde_json::to_vec(&intent.locator())?,
            &intent.identity.nonce,
            hook,
            "before-locator",
            "after-locator",
        )?;
    }
    let at_source = fs::symlink_metadata(&intent.identity.source).is_ok();
    let root = if at_source {
        &intent.identity.source
    } else {
        &intent.identity.destination
    };
    if DirectoryIdentity::read(root)? != intent.identity.root {
        return Err(Error::StatePathChanged);
    }
    if files::exists(&root.join(MARKER))? {
        validate_marker(root, &intent)?;
    }
    durable_metadata(
        &root.join(MARKER),
        &serde_json::to_vec(&intent)?,
        &intent.identity.nonce,
        hook,
        "before-marker",
        "after-marker",
    )?;
    // Reserves agent/registry/profile locks and validates all security state before
    // the first rename. Recovery after rename verifies after binding publication.
    if at_source {
        if intent.phase != Phase::Prepared
            || content_digest(&guard.inspect(root)?)? != intent.identity.content_digest
        {
            return Err(Error::StatePathChanged);
        }
        hook("before-rename")?;
        rename_root(&intent)?;
        hook("after-rename")?;
    }
    verify_destination_identity(&intent)?;
    for path in [&intent.identity.source, &intent.identity.destination] {
        hook("before-parent-sync")?;
        File::open(parent(path)?)?.sync_all()?;
        hook("after-parent-sync")?;
    }
    if intent.phase == Phase::Prepared {
        transition(guard, &mut intent, Phase::FilesInstalled, hook)?;
        durable_metadata(
            &intent.identity.destination.join(MARKER),
            &serde_json::to_vec(&intent)?,
            &intent.identity.nonce,
            hook,
            "before-marker",
            "after-marker",
        )?;
    }
    let snapshot = guard.inspect(&intent.identity.destination)?;
    if content_digest(&snapshot)? != intent.identity.content_digest {
        return Err(Error::StatePathChanged);
    }
    if intent.phase != Phase::Verified {
        transition(guard, &mut intent, Phase::Verified, hook)?;
    }
    durable_metadata(
        &intent.identity.destination.join(MARKER),
        &serde_json::to_vec(&intent)?,
        &intent.identity.nonce,
        hook,
        "before-verified-marker",
        "after-verified-marker",
    )?;
    hook("before-marker-remove")?;
    fs::remove_file(intent.identity.destination.join(MARKER))?;
    hook("before-marker-remove-sync")?;
    File::open(&intent.identity.destination)?.sync_all()?;
    hook("after-marker-remove-sync")?;
    hook("after-marker-remove")?;
    hook("before-native-receipt")?;
    guard.with_native_manifest(|m| {
        if intent_in(m, INTENT)?.as_ref() != Some(&intent) {
            return Err(Error::StateRecoveryRequired);
        }
        m.put(RECEIPT, &serde_json::to_vec(&intent)?)?;
        m.remove(INTENT)?;
        Ok(())
    })?;
    hook("after-native-receipt")?;
    hook("before-root-selection")?;
    super::selection::relocated(guard, &intent.identity.source, &intent.identity.destination)?;
    hook("after-root-selection")?;
    cleanup_locators(guard, &intent, hook)?;
    Ok(report(&intent, recovered))
}
fn transition(
    guard: &ClientStateMaintenanceGuard,
    intent: &mut Intent,
    phase: Phase,
    hook: &mut dyn FnMut(&'static str) -> Result<()>,
) -> Result<()> {
    hook("before-native-phase")?;
    guard.with_native_manifest(|m| {
        if intent_in(m, INTENT)?.as_ref() != Some(intent) {
            return Err(Error::StateRecoveryRequired);
        }
        let mut next = intent.clone();
        next.phase = phase;
        m.put(INTENT, &serde_json::to_vec(&next)?)?;
        m.put(
            crate::STATE_ROOT_RECORD,
            &intent.identity.destination_binding,
        )?;
        Ok(())
    })?;
    intent.phase = phase;
    hook("after-native-phase")
}
fn cleanup_locators(
    guard: &ClientStateMaintenanceGuard,
    intent: &Intent,
    hook: &mut dyn FnMut(&'static str) -> Result<()>,
) -> Result<()> {
    for root in [&intent.identity.destination, &intent.identity.source] {
        let path = locator_path(&guard.base, root);
        if files::exists(&path)? {
            let locator: Locator =
                serde_json::from_slice(&crate::read_bounded_regular_file(&path, MAX_RECORD)?)?;
            if locator != intent.locator() {
                return Err(Error::StateRecoveryRequired);
            }
            hook("before-locator-remove")?;
            fs::remove_file(&path)?;
            hook("before-locator-remove-sync")?;
            File::open(&guard.base)?.sync_all()?;
            hook("after-locator-remove-sync")?;
            hook("after-locator-remove")?;
        }
    }
    Ok(())
}
fn rename_root(intent: &Intent) -> Result<()> {
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    verify_parents(intent)?;
    let open = |path: &Path, expected: DirectoryIdentity| -> Result<File> {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)?;
        let m = file.metadata()?;
        if m.dev() != expected.device || m.ino() != expected.inode {
            return Err(Error::StatePathChanged);
        }
        Ok(file)
    };
    let source = open(
        parent(&intent.identity.source)?,
        intent.identity.source_parent,
    )?;
    let destination = open(
        parent(&intent.identity.destination)?,
        intent.identity.destination_parent,
    )?;
    if DirectoryIdentity::read(&intent.identity.source)? != intent.identity.root {
        return Err(Error::StatePathChanged);
    }
    rustix::fs::renameat_with(
        &source,
        intent
            .identity
            .source
            .file_name()
            .ok_or(Error::StatePathChanged)?,
        &destination,
        intent
            .identity
            .destination
            .file_name()
            .ok_or(Error::StatePathChanged)?,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(std::io::Error::from)?;
    Ok(())
}

#[derive(Serialize)]
pub struct RelocationStatus {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub phase: &'static str,
    pub recovery_required: bool,
}
/// Read public recovery facts without creating, repairing or advancing state.
pub fn relocation_status(path: impl AsRef<Path>) -> Result<Option<RelocationStatus>> {
    let path = canonical_reservation(path.as_ref())?;
    let mut guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&path))?;
    let locator = locator_path(&guard.base, &path);
    let hint = if files::exists(&locator)? {
        Some(serde_json::from_slice::<Locator>(
            &crate::read_bounded_regular_file(&locator, MAX_RECORD)?,
        )?)
    } else {
        None
    };
    let id = if let Some(h) = &hint {
        if h.version != 1 || (h.source != path && h.destination != path) {
            return Err(Error::StateRecoveryRequired);
        }
        h.state_id.clone()
    } else {
        let Some(state) = crate::checkpoint::inspect_state_file(&path)? else {
            return Ok(None);
        };
        if state.credential_backend != crate::CredentialBackend::Native {
            return Err(Error::PortabilityUnsupported);
        }
        state.state_id
    };
    guard.reserve_namespace(&id)?;
    guard.with_native_manifest(|m| {
        let active = intent_in(m, INTENT)?;
        let required = active.is_some() || hint.is_some();
        let Some(intent) = active.or(intent_in(m, RECEIPT)?) else {
            return if required {
                Err(Error::StateRecoveryRequired)
            } else {
                Ok(None)
            };
        };
        intent.validate(&id)?;
        if path != intent.identity.source && path != intent.identity.destination
            || hint.as_ref().is_some_and(|h| *h != intent.locator())
        {
            return Err(Error::StateRecoveryRequired);
        }
        Ok(Some(RelocationStatus {
            source: intent.identity.source,
            destination: intent.identity.destination,
            phase: match intent.phase {
                Phase::Prepared => "prepared",
                Phase::FilesInstalled => "files-installed",
                Phase::Verified => "verified",
            },
            recovery_required: required,
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn enabled() -> bool {
        std::env::var_os("FOKS_TEST_NATIVE_PORTABILITY").is_some()
    }
    fn initialize(root: &Path) -> String {
        let credentials =
            crate::ClientCredentials::initialize(root, crate::CredentialBackend::Native).unwrap();
        let id = credentials.state_id.clone();
        drop(credentials);
        let mut registry = crate::ProfileRegistry::open(root).unwrap();
        registry
            .add(crate::Profile {
                name: "registry-only".into(),
                label: None,
                probe: "foks.app".into(),
                protocol: crate::ProtocolPolicy::V019,
                trust: crate::TrustRoot::WebPki,
            })
            .unwrap();
        id
    }
    fn remove_native(id: &str) {
        foks_keystore::NativeCredentialStore::open(id)
            .unwrap()
            .remove(crate::checkpoint::NATIVE_MANIFEST_RECORD)
            .unwrap();
    }
    #[test]
    fn relocation_preserves_native_namespace_and_rejects_destination_races() {
        if !enabled() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        let id = initialize(&source);
        let credentials = crate::ClientCredentials::open(&source).unwrap();
        assert!(matches!(
            relocate_state(&source, &destination),
            Err(Error::StateBusy)
        ));
        drop(credentials);
        let result = relocate(&source, &destination, &mut |point| {
            if point == "before-rename" {
                fs::create_dir(&destination)?;
            }
            Ok(())
        });
        assert!(result.is_err());
        assert!(source.exists());
        assert!(destination.exists());
        assert!(matches!(
            crate::ProfileRegistry::open(&source),
            Err(Error::StateRecoveryRequired)
        ));
        assert_eq!(
            super::super::maintenance_readiness(&source).unwrap(),
            super::super::MaintenanceReadiness::RecoveryRequired
        );
        fs::remove_dir(&destination).unwrap();
        recover_relocation(&destination).unwrap();
        assert_eq!(
            super::super::maintenance_readiness(&destination).unwrap(),
            super::super::MaintenanceReadiness::Openable
        );
        assert!(!source.exists());
        assert_eq!(
            crate::ClientCredentials::open(&destination)
                .unwrap()
                .state_id,
            id
        );
        assert_eq!(
            super::super::inspect_native_state(&destination)
                .unwrap()
                .profiles
                .len(),
            1
        );
        assert!(recover_relocation(&source).is_err()); // no locator and source absent: no invention.
        recover_relocation(&destination).unwrap();
        remove_native(&id);
    }
    #[test]
    fn every_relocation_boundary_recovers_without_recreating_source() {
        if !enabled() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("baseline-source");
        let destination = dir.path().join("baseline-destination");
        let id = initialize(&source);
        let mut events = Vec::new();
        relocate(&source, &destination, &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
        remove_native(&id);
        for (failure, event) in events.iter().enumerate().skip(1) {
            let source = dir.path().join(format!("source-{failure}"));
            let destination = dir.path().join(format!("destination-{failure}"));
            let id = initialize(&source);
            let mut count = 0;
            let result = relocate(&source, &destination, &mut |_| {
                let here = count;
                count += 1;
                if here == failure {
                    Err(Error::StateRecoveryRequired)
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err(), "boundary {failure}");
            let base = super::super::lease::lock_directory().unwrap();
            if !source.exists() && locator_path(&base, &source).exists() {
                assert!(
                    crate::ClientStateLease::acquire(&source).is_err(),
                    "{}",
                    event
                );
                assert!(!source.exists());
            }
            let base = super::super::lease::lock_directory().unwrap();
            let path = if locator_path(&base, &destination).exists() {
                &destination
            } else if source.exists() {
                &source
            } else {
                &destination
            };
            recover_relocation(path)
                .unwrap_or_else(|e| panic!("boundary {failure} {}: {e}", event));
            assert!(!source.exists());
            assert_eq!(
                crate::ClientCredentials::open(&destination)
                    .unwrap()
                    .state_id,
                id
            );
            remove_native(&id);
        }
    }
    #[test]
    fn stale_attempt_and_substituted_parent_never_authorize_rebinding() {
        if !enabled() {
            return;
        }
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let parent = dir.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let destination = parent.join("destination");
        let moved_parent = dir.path().join("moved-parent");
        let id = initialize(&source);
        assert!(relocate(&source, &destination, &mut |event| {
            if event == "before-rename" {
                fs::rename(&parent, &moved_parent)?;
                symlink(&moved_parent, &parent)?;
            }
            Ok(())
        })
        .is_err());
        assert!(source.exists());
        assert!(recover_relocation(&source).is_err());
        fs::remove_file(&parent).unwrap();
        fs::rename(&moved_parent, &parent).unwrap();
        let base = super::super::lease::lock_directory().unwrap();
        let stale = fs::read(locator_path(&base, &destination)).unwrap();
        recover_relocation(&source).unwrap();
        let third = dir.path().join("third");
        assert!(relocate(
            &destination,
            &third,
            &mut |event| if event == "before-rename" {
                Err(Error::StateRecoveryRequired)
            } else {
                Ok(())
            }
        )
        .is_err());
        let path = locator_path(&base, &destination);
        let current = fs::read(&path).unwrap();
        crate::atomic_private_write(&path, &stale).unwrap();
        assert!(recover_relocation(&destination).is_err());
        assert!(destination.exists());
        assert!(!third.exists());
        crate::atomic_private_write(&path, &current).unwrap();
        recover_relocation(&destination).unwrap();
        remove_native(&id);
    }

    #[test]
    fn cross_filesystem_move_fails_before_publishing_intent() {
        if !enabled() || !Path::new("/dev/shm").is_dir() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let id = initialize(&source);
        let destination = Path::new("/dev/shm").join(format!("foks-portability-test-{id}"));
        if DirectoryIdentity::read(&source).unwrap().device
            != DirectoryIdentity::read(Path::new("/dev/shm"))
                .unwrap()
                .device
        {
            assert!(matches!(
                relocate_state(&source, &destination),
                Err(Error::InvalidConfig(
                    "cross-filesystem relocation requires encrypted export/import"
                ))
            ));
            assert!(!destination.exists());
            assert!(relocation_status(&source).unwrap().is_none());
            assert!(crate::ClientCredentials::open(&source).is_ok());
        }
        remove_native(&id);
    }

    #[test]
    fn recovery_rejects_wrong_nonce_substitution_and_torn_marker_temporary() {
        if !enabled() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        let id = initialize(&source);
        assert!(relocate(
            &source,
            &destination,
            &mut |event| if event == "before-rename" {
                Err(Error::StateRecoveryRequired)
            } else {
                Ok(())
            }
        )
        .is_err());
        let base = super::super::lease::lock_directory().unwrap();
        let locator = locator_path(&base, &destination);
        let original = fs::read(&locator).unwrap();
        let mut changed: Locator = serde_json::from_slice(&original).unwrap();
        changed.nonce[0] ^= 1;
        crate::atomic_private_write(&locator, &serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(recover_relocation(&destination).is_err());
        assert!(source.exists());
        crate::atomic_private_write(&locator, &original).unwrap();
        let intent = decode(&fs::read(source.join(MARKER)).unwrap()).unwrap();
        let temporary = source
            .join(MARKER)
            .with_extension(format!("pending-{}", crate::hex(&intent.identity.nonce)));
        crate::create_private_config(&temporary, b"torn").unwrap();
        recover_relocation(&source).unwrap();
        assert!(!destination.join(temporary.file_name().unwrap()).exists());
        remove_native(&id);
    }
}
