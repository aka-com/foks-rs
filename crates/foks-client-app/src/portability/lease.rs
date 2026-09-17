use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DirectoryIdentity {
    pub device: u64,
    pub inode: u64,
}
impl DirectoryIdentity {
    pub(super) fn read(path: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::StatePathChanged);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Ok(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        }
        #[cfg(not(unix))]
        {
            Err(Error::PortabilityUnsupported)
        }
    }
}

struct PathLeaseInner {
    root: PathBuf,
    identity: DirectoryIdentity,
    _path: File,
}

/// Shared reservation for filesystem-only infrastructure below a client-state root.
///
/// This deliberately does not inspect the client-state envelope or reserve its
/// native credential namespace. Callers must not use it for credential-backed
/// state operations.
#[derive(Clone)]
pub struct ClientStatePathLease(Arc<PathLeaseInner>);
impl ClientStatePathLease {
    pub fn acquire(root: impl AsRef<Path>) -> Result<Self> {
        let root = canonical_reservation(root.as_ref())?;
        let base = lock_directory()?;
        reject_lock_overlap(&base, &root)?;
        let path = path_lock_file(&base, &root)?;
        try_lock(&path, false)?;
        require_no_locator(&base, &root)?;
        #[cfg(unix)]
        if let Ok(metadata) = fs::symlink_metadata(&root) {
            use std::os::unix::fs::MetadataExt as _;
            if metadata.uid() != rustix::process::getuid().as_raw() {
                return Err(Error::InvalidConfig("state root is not owned by this user"));
            }
        }
        let prepared = crate::prepare_private_directory(&root)?;
        if prepared != root {
            return Err(Error::StatePathChanged);
        }
        let identity = DirectoryIdentity::read(&root)?;
        Ok(Self(Arc::new(PathLeaseInner {
            root,
            identity,
            _path: path,
        })))
    }
    pub fn root(&self) -> &Path {
        &self.0.root
    }
    pub fn validate(&self) -> Result<()> {
        if DirectoryIdentity::read(self.root()).map_err(|_| Error::StatePathChanged)?
            != self.0.identity
            || self.root().canonicalize()? != self.0.root
        {
            return Err(Error::StatePathChanged);
        }
        require_no_locator(&lock_directory()?, self.root())
    }
}

struct LeaseInner {
    path: ClientStatePathLease,
    _namespace: Option<File>,
}

/// Shared lifetime reservation, acquired before ordinary state side effects.
/// Cloning preserves the same reservation; maintenance fails busy until all users drop it.
#[derive(Clone)]
pub struct ClientStateLease(Arc<LeaseInner>);
impl ClientStateLease {
    pub fn acquire(root: impl AsRef<Path>) -> Result<Self> {
        let path = ClientStatePathLease::acquire(root)?;
        let base = lock_directory()?;
        // Read configuration only after path exclusion. No native get or registry
        // recovery may precede this namespace-use reservation.
        let namespace = match crate::checkpoint::inspect_state_file(path.root())? {
            Some(state) if state.credential_backend == crate::CredentialBackend::Native => {
                let file = namespace_lock_file(&base, &state.state_id, "use")?;
                try_lock(&file, false)?;
                super::relocation::require_namespace_ready(&state.state_id, path.root())?;
                Some(file)
            }
            _ => None,
        };
        Ok(Self(Arc::new(LeaseInner {
            path,
            _namespace: namespace,
        })))
    }
    pub fn root(&self) -> &Path {
        self.0.path.root()
    }
    pub fn validate(&self) -> Result<()> {
        self.0.path.validate()
    }
}

/// Exclusive path reservations. These never create the source/destination root.
/// Namespace exclusion is acquired only after all paths have been reserved.
pub struct ClientStateMaintenanceGuard {
    pub(super) base: PathBuf,
    pub(super) paths: Vec<PathBuf>,
    _reservations: Vec<File>,
    namespace: Option<(String, File)>,
    local_locks: Vec<File>,
    pub(super) relocation_nonce: Option<[u8; 32]>,
    pub(super) import_nonce: Option<[u8; 32]>,
    pub(super) import_marker_digest: Option<[u8; 32]>,
}
impl ClientStateMaintenanceGuard {
    pub fn acquire(paths: &[PathBuf]) -> Result<Self> {
        if paths.is_empty() || paths.len() > 4 {
            return Err(Error::InvalidConfig("invalid maintenance path count"));
        }
        let base = lock_directory()?;
        let mut canonical = paths
            .iter()
            .map(|p| canonical_reservation(p))
            .collect::<Result<Vec<_>>>()?;
        canonical.sort();
        canonical.dedup();
        let mut reservations = Vec::with_capacity(canonical.len());
        for path in &canonical {
            reject_lock_overlap(&base, path)?;
            let file = path_lock_file(&base, path)?;
            try_lock(&file, true)?;
            reservations.push(file);
        }
        Ok(Self {
            base,
            paths: canonical,
            _reservations: reservations,
            namespace: None,
            local_locks: Vec::new(),
            relocation_nonce: None,
            import_nonce: None,
            import_marker_digest: None,
        })
    }
    pub(super) fn reserve_local_lock(&mut self, path: &Path) -> Result<()> {
        let file = private_lock(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let current = file.metadata()?;
            for held in &self.local_locks {
                let metadata = held.metadata()?;
                if metadata.dev() == current.dev() && metadata.ino() == current.ino() {
                    return Ok(());
                }
            }
        }
        try_lock(&file, true)?;
        self.local_locks.push(file);
        Ok(())
    }

    pub(crate) fn namespace_id(&self) -> Result<&str> {
        self.namespace
            .as_ref()
            .map(|(id, _)| id.as_str())
            .ok_or(Error::StateRecoveryRequired)
    }

    pub(super) fn reserve_namespace(&mut self, state_id: &str) -> Result<()> {
        if let Some((current, _)) = &self.namespace {
            return if current == state_id {
                Ok(())
            } else {
                Err(Error::StatePathChanged)
            };
        }
        let file = namespace_lock_file(&self.base, state_id, "use")?;
        try_lock(&file, true)?;
        self.namespace = Some((state_id.into(), file));
        Ok(())
    }
    pub(super) fn require_path(&self, path: &Path) -> Result<()> {
        if !self.paths.iter().any(|p| p == path) {
            return Err(Error::StatePathChanged);
        }
        Ok(())
    }
}

fn try_lock(file: &File, exclusive: bool) -> Result<()> {
    let result = if exclusive {
        file.try_lock_exclusive()
    } else {
        FileExt::try_lock_shared(file)
    };
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Err(Error::StateBusy),
        Err(e) => Err(e.into()),
    }
}
use fs2::FileExt;

/// Canonicalize the longest existing ancestor without creating missing paths.
/// Dot/dotdot aliases are resolved before hashing; a symlink at the requested leaf
/// is always rejected, including dangling symlinks.
pub(super) fn canonical_reservation(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(Error::InvalidConfig("state path is empty"));
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                // Canonicalize before .. so symlink spelling cannot change what .. means.
                if normalized.exists() {
                    normalized = normalized.canonicalize()?;
                }
                if !normalized.pop() {
                    return Err(Error::StatePathChanged);
                }
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if fs::symlink_metadata(&normalized).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(Error::StatePathChanged);
    }
    let mut ancestor = normalized.clone();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(&ancestor) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or(Error::StatePathChanged)?
                        .to_owned(),
                );
                if !ancestor.pop() {
                    return Err(Error::StatePathChanged);
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    let mut resolved = ancestor.canonicalize()?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

fn reject_lock_overlap(base: &Path, root: &Path) -> Result<()> {
    if base.starts_with(root) || root.starts_with(base) {
        return Err(Error::InvalidConfig(
            "client state overlaps its external maintenance directory",
        ));
    }
    Ok(())
}

pub(super) fn lock_directory() -> Result<PathBuf> {
    // Deliberately independent of movable app data and XDG state selection.
    let home =
        std::env::var_os("HOME").ok_or(Error::InvalidConfig("home directory is unavailable"))?;
    let path = PathBuf::from(home).join(".foks-state-maintenance-v1");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => File::open(path.parent().ok_or(Error::StatePathChanged)?)?.sync_all()?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err(Error::InvalidConfig(
                "external maintenance directory is not private",
            ));
        }
        path.canonicalize().map_err(Into::into)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(Error::PortabilityUnsupported)
    }
}

pub(super) fn private_lock(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::StatePathChanged);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(Error::InvalidConfig(
                "external maintenance lock is not a private single-link file",
            ));
        }
        let named = fs::symlink_metadata(path)?;
        if named.dev() != metadata.dev() || named.ino() != metadata.ino() {
            return Err(Error::StatePathChanged);
        }
    }
    Ok(file)
}
fn path_lock_file(base: &Path, path: &Path) -> Result<File> {
    private_lock(&base.join(format!(
        "path-{}.lock",
        crate::hex(&crate::state_root_binding(path))
    )))
}
fn namespace_lock_file(base: &Path, state_id: &str, kind: &str) -> Result<File> {
    crate::validate_name(state_id)?;
    private_lock(&base.join(format!("namespace-{state_id}-{kind}.lock")))
}
pub(crate) fn manifest_lock_file(state_id: &str) -> Result<File> {
    namespace_lock_file(&lock_directory()?, state_id, "manifest")
}
pub(super) fn locator_path(base: &Path, root: &Path) -> PathBuf {
    base.join(format!(
        "path-{}.locator",
        crate::hex(&crate::state_root_binding(root))
    ))
}
fn require_no_locator(base: &Path, root: &Path) -> Result<()> {
    match fs::symlink_metadata(locator_path(base, root)) {
        Ok(_) => Err(Error::StateRecoveryRequired),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_only_lease_ignores_credential_schema_and_excludes_maintenance() {
        let temporary = tempfile::tempdir().unwrap();
        let root = crate::prepare_private_directory(&temporary.path().join("state")).unwrap();
        crate::create_private_config(
            &root.join(crate::STATE_CONFIG_FILE),
            b"version = 2\nstate_id = \"legacy\"\ncredential_backend = \"native\"\n",
        )
        .unwrap();

        let lease = ClientStatePathLease::acquire(&root).unwrap();
        assert_eq!(lease.root(), root);
        assert!(matches!(
            ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&root)),
            Err(Error::StateBusy)
        ));
        drop(lease);
        assert!(ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&root)).is_ok());
    }

    #[test]
    fn retained_registry_credentials_and_session_exclude_maintenance() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            crate::ClientCredentials::initialize(&root, crate::CredentialBackend::PrivateFile)
                .unwrap();
        let mut registry = crate::ProfileRegistry::open(&root).unwrap();
        registry
            .add(crate::Profile {
                name: "local".into(),
                probe: "foks.app".into(),
                protocol: crate::ProtocolPolicy::V019,
                trust: crate::TrustRoot::WebPki,
            })
            .unwrap();
        let session = crate::ProfileSession::open(&registry, "local").unwrap();
        drop(registry);
        drop(credentials);
        assert!(matches!(
            ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&root)),
            Err(Error::StateBusy)
        ));
        drop(session);
        let guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&root)).unwrap();
        assert!(matches!(
            crate::ProfileRegistry::open(&root),
            Err(Error::StateBusy)
        ));
        drop(guard);
        crate::ProfileRegistry::open(&root).unwrap();
    }

    #[test]
    fn stale_handles_do_not_recreate_externally_moved_state() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let moved = temporary.path().join("moved");
        let credentials =
            crate::ClientCredentials::initialize(&root, crate::CredentialBackend::PrivateFile)
                .unwrap();
        let registry = crate::ProfileRegistry::open(&root).unwrap();
        fs::rename(&root, &moved).unwrap();
        assert!(matches!(
            credentials.master_key(),
            Err(Error::StatePathChanged)
        ));
        assert!(matches!(
            registry.prepare_profile_directory("local"),
            Err(Error::StatePathChanged)
        ));
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        assert!(matches!(
            credentials.master_key(),
            Err(Error::StatePathChanged)
        ));
    }

    #[test]
    fn absent_destination_reservation_precedes_initialization() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("destination");
        let guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&root)).unwrap();
        assert!(!root.exists());
        assert!(matches!(
            crate::ClientCredentials::initialize(&root, crate::CredentialBackend::PrivateFile),
            Err(Error::StateBusy)
        ));
        assert!(!root.exists());
        drop(guard);
    }

    #[test]
    fn missing_source_locator_blocks_initialization_before_creation() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("missing");
        let guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&root)).unwrap();
        let locator = locator_path(&guard.base, &guard.paths[0]);
        crate::create_private_config(&locator, b"untrusted discovery only").unwrap();
        drop(guard);
        let result =
            crate::ClientCredentials::initialize(&root, crate::CredentialBackend::PrivateFile);
        fs::remove_file(locator).unwrap();
        assert!(matches!(result, Err(Error::StateRecoveryRequired)));
        assert!(!root.exists());
    }

    #[test]
    fn canonical_aliases_share_reservations_and_symlink_leaves_fail() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let lease = ClientStateLease::acquire(&root).unwrap();
        let alias = root.join("..").join("state");
        assert!(matches!(
            ClientStateMaintenanceGuard::acquire(&[alias]),
            Err(Error::StateBusy)
        ));
        #[cfg(unix)]
        {
            let symlink = temporary.path().join("alias");
            std::os::unix::fs::symlink(&root, &symlink).unwrap();
            assert!(ClientStateLease::acquire(symlink).is_err());
        }
        drop(lease);
    }

    #[test]
    fn cross_process_exclusion_and_namespace_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let _lease = ClientStateLease::acquire(&root).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "portability::lease::tests::child_process_maintenance",
                "--exact",
            ])
            .env("FOKS_LEASE_TEST_ROOT", &root)
            .status()
            .unwrap();
        assert!(result.success());
        let other = temporary.path().join("other");
        let mut first = ClientStateMaintenanceGuard::acquire(&[other]).unwrap();
        let mut second =
            ClientStateMaintenanceGuard::acquire(&[temporary.path().join("third")]).unwrap();
        let id = crate::hex(&crate::random_array::<16>().unwrap());
        first.reserve_namespace(&id).unwrap();
        assert!(matches!(
            second.reserve_namespace(&id),
            Err(Error::StateBusy)
        ));
    }

    #[test]
    fn child_process_maintenance() {
        let Some(root) = std::env::var_os("FOKS_LEASE_TEST_ROOT") else {
            return;
        };
        assert!(matches!(
            ClientStateMaintenanceGuard::acquire(&[root.into()]),
            Err(Error::StateBusy)
        ));
    }
}
