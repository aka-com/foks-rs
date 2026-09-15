use super::{files, lease::DirectoryIdentity, trust, ClientStateMaintenanceGuard};
use crate::checkpoint::{NativeManifestStore, RollbackCheckpoint};
use crate::{AccountVault, CredentialBackend, Error, Profile, ProfilePaths, Result};
use foks_client::{
    ProtectedMutationStore as _, ProtectedPresence, ProtectedRecordDescriptor,
    ProtectedRecordInventory,
};
use foks_client_db::HardStateStore;
use foks_keystore::SecretStore as _;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub(super) struct ProfileSnapshot {
    pub profile: Profile,
    pub trust_digest: Option<String>,
    pub checkpoint: Option<RollbackCheckpoint>,
    pub protected: BTreeMap<String, ProtectedRecordDescriptor>,
    pub vault: Vec<crate::account::VaultRecordDescriptor>,
    pub blockers: Vec<String>,
}
pub(super) struct StateSnapshot {
    pub root: PathBuf,
    pub identity: DirectoryIdentity,
    pub state_id: String,
    pub native: NativeManifestStore,
    pub profiles: BTreeMap<String, ProfileSnapshot>,
    pub trust: BTreeMap<String, Vec<u8>>,
    pub artifacts: Vec<files::Artifact>,
    validated_files: BTreeMap<PathBuf, std::fs::Metadata>,
    allowed_import_marker: Option<[u8; 32]>,
}
impl StateSnapshot {
    pub fn require_exportable(&self) -> Result<()> {
        if self.profiles.values().any(|p| {
            !p.blockers.is_empty()
                || p.vault.iter().any(|v| !v.exportable)
                || p.protected.values().any(|v| !v.exportable())
        }) {
            return Err(Error::InvalidConfig("export requires completed local workflows; inspect the owning profile and resume or cancel pending work"));
        }
        Ok(())
    }
    pub fn master(&self) -> Result<Zeroizing<[u8; 32]>> {
        let bytes = self
            .native
            .records
            .get(crate::MASTER_KEY_RECORD)
            .ok_or(Error::StateRecoveryRequired)?;
        Ok(Zeroizing::new(
            bytes
                .as_slice()
                .try_into()
                .map_err(|_| Error::StateRecoveryRequired)?,
        ))
    }
    pub fn digest(&self) -> Result<[u8; 32]> {
        let mut hash = Sha256::new();
        hash.update(b"foks-state-snapshot-v1");
        hash.update(self.native.encode()?.as_slice());
        hash.update(serde_json::to_vec(&self.artifacts)?);
        for (digest, bytes) in &self.trust {
            hash.update(digest.as_bytes());
            hash.update(bytes);
        }
        Ok(hash.finalize().into())
    }
}
impl ClientStateMaintenanceGuard {
    pub(super) fn inspect(&mut self, root: &Path) -> Result<StateSnapshot> {
        self.inspect_with_projection(root, None)
    }
    /// Only authenticated archive/import orchestration may supply external native
    /// authority. It never enrolls or reads the source's live native namespace.
    pub(super) fn inspect_with_projection(
        &mut self,
        root: &Path,
        projection: Option<(&Path, &NativeManifestStore)>,
    ) -> Result<StateSnapshot> {
        self.inspect_projection(root, projection)
            .map_err(|error| match error {
                Error::Json(_) | Error::TomlDecode(_) => {
                    Error::InvalidConfig("state record is invalid")
                }
                other => other,
            })
    }
    // Parser diagnostics may quote decrypted input. Keep them inside this boundary.
    fn inspect_projection(
        &mut self,
        root: &Path,
        projection: Option<(&Path, &NativeManifestStore)>,
    ) -> Result<StateSnapshot> {
        self.require_path(root)?;
        files::private_directory(root)?;
        let mut validated_files = BTreeMap::new();
        let envelope = root.join(crate::STATE_CONFIG_FILE);
        validated_files.insert(envelope.clone(), files::regular(&envelope)?);
        let state = crate::checkpoint::inspect_state_file(root)?.ok_or(Error::InvalidConfig(
            "source client state is not initialized",
        ))?;
        if state.credential_backend != CredentialBackend::Native {
            return Err(Error::PortabilityUnsupported);
        }
        if projection.is_none() {
            self.reserve_namespace(&state.state_id)?;
        }
        for name in [
            ".foks-rs.lock",
            "agent.desktop-agent.lock",
            "foks-rs.desktop-agent.lock",
            crate::REGISTRY_LOCK_FILE,
        ] {
            self.reserve_local_lock(&root.join(name))?;
        }
        let registry_path = root.join("profiles.toml");
        if registry_path.exists() {
            validated_files.insert(registry_path.clone(), files::regular(&registry_path)?);
        }
        let configured = crate::registry::load_registry(root)?;
        if projection.is_some()
            && configured
                .values()
                .any(|p| matches!(p.trust, crate::TrustRoot::CertificateDer { .. }))
        {
            return Err(Error::InvalidConfig(
                "archive trust must reference root-owned certificate artifacts",
            ));
        }
        if configured.len() > 256 {
            return Err(Error::InvalidConfig("snapshot profile limit exceeded"));
        }
        // Reserve every initialized profile before reading the native manifest.
        for name in configured.keys() {
            let directory = root.join("profiles").join(name);
            if directory.exists() {
                files::private_directory(&directory)?;
                self.reserve_local_lock(&directory.join(".profile-operation.lock"))?;
                self.reserve_local_lock(&directory.join(".scheduler-run.lock"))?;
                let hard = directory.join("hard.sqlite3");
                if hard.exists() {
                    let id = HardStateStore::inspect_existing(&hard)?
                        .metadata()?
                        .database_id;
                    let locks = root.join(".database-operation-locks");
                    if locks.exists() {
                        files::private_directory(&locks)?;
                    } else {
                        crate::prepare_private_directory(&locks)?;
                    }
                    self.reserve_local_lock(&locks.join(format!("{}.lock", crate::hex(&id))))?;
                }
            }
        }
        let native = match projection {
            Some((_, native)) => NativeManifestStore::decode(&native.encode()?)?,
            None => self.with_native_manifest(|m| NativeManifestStore::decode(&m.encode()?))?,
        };
        let binding_root = projection.map(|(root, _)| root).unwrap_or(root);
        if native
            .records
            .get(crate::STATE_ROOT_RECORD)
            .map(Vec::as_slice)
            != Some(crate::state_root_binding(binding_root).as_slice())
        {
            return Err(Error::InvalidConfig(
                "native client state belongs to a different root path",
            ));
        }
        let mut snapshot = StateSnapshot {
            root: root.into(),
            identity: DirectoryIdentity::read(root)?,
            state_id: state.state_id,
            native,
            profiles: BTreeMap::new(),
            trust: BTreeMap::new(),
            artifacts: Vec::new(),
            validated_files,
            allowed_import_marker: self.import_marker_digest,
        };
        let master = snapshot.master()?;
        let mut expected = BTreeSet::from([
            crate::MASTER_KEY_RECORD.to_owned(),
            crate::STATE_ROOT_RECORD.to_owned(),
        ]);
        expected.extend(super::import::inventory_keys(self, &snapshot.native, root)?);
        expected.extend(super::relocation::inventory_keys(
            self,
            &snapshot.native,
            root,
        )?);
        for (name, profile) in configured {
            let mut trust_digest = None;
            if let Some(certificate) = trust::read_certificate(root, &profile.trust)? {
                trust_digest = Some(crate::hex(&Sha256::digest(&certificate)));
                snapshot
                    .trust
                    .insert(crate::hex(&Sha256::digest(&certificate)), certificate);
            }
            let paths = ProfilePaths::for_directory(root.join("profiles").join(&name));
            let mut p = ProfileSnapshot {
                profile,
                trust_digest,
                checkpoint: None,
                protected: BTreeMap::new(),
                vault: Vec::new(),
                blockers: Vec::new(),
            };
            if paths.hard_database.exists() {
                snapshot.validated_files.insert(
                    paths.hard_database.clone(),
                    files::regular(&paths.hard_database)?,
                );
                let hard = HardStateStore::inspect_existing(&paths.hard_database)?;
                let checkpoint = crate::registry::checkpoint_for_store(&p.profile, &hard)?;
                if let Some(key) = super::readiness::validate(
                    &name,
                    hard.import_readiness()?.as_ref(),
                    &snapshot.native,
                )? {
                    expected.insert(key);
                }

                let rollback = crate::checkpoint::rollback_record_key(&name)?;
                let claim = crate::checkpoint::database_claim_record_key(&checkpoint.database_id);
                let stored: RollbackCheckpoint = serde_json::from_slice(
                    snapshot
                        .native
                        .records
                        .get(&rollback)
                        .ok_or(Error::StateRecoveryRequired)?,
                )?;
                if stored.digest()? != checkpoint.digest()?
                    || snapshot.native.records.get(&claim).map(Vec::as_slice)
                        != Some(name.as_bytes())
                {
                    return Err(Error::InvalidConfig("snapshot checkpoint or database claim differs; run normal checked recovery"));
                }
                expected.insert(rollback);
                expected.insert(claim);
                let mut inventory = ProtectedRecordInventory::default();
                loop {
                    let progress = inventory.advance(
                        &hard,
                        std::time::Instant::now() + std::time::Duration::from_millis(50),
                    )?;
                    if progress.saturated {
                        return Err(Error::InvalidConfig("protected inventory is saturated"));
                    }
                    if progress.complete {
                        break;
                    }
                }
                p.protected = inventory.records(hard.metadata()?)?.clone();
                let mut protected = if paths.protected_mutations.exists() {
                    Some(foks_client::EncryptedFileMutationStore::inspect_existing(
                        &paths.protected_mutations,
                        crate::derive_mutation_key(&master),
                    )?)
                } else {
                    None
                };
                for (filename, record) in &p.protected {
                    let path = paths.protected_mutations.join(filename);
                    if path.exists() {
                        snapshot
                            .validated_files
                            .insert(path.clone(), files::regular(&path)?);
                    }
                    let material = match &mut protected {
                        Some(store) => store.get(&record.key),
                        None => Err(foks_client::ProtectedStoreError::Missing),
                    };
                    match material {
                        Ok(bytes) => record.validate_material(&bytes)?,
                        Err(foks_client::ProtectedStoreError::Missing)
                            if record.presence != ProtectedPresence::Required => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                if paths.credential_store.exists() {
                    let mut store = foks_keystore::EncryptedFileSecretStore::inspect_existing(
                        &paths.credential_store,
                        crate::derive_vault_key(&master),
                    )?;
                    let keys = store.keys()?;
                    let mut vault = AccountVault::new(&mut store);
                    let host = checkpoint
                        .host
                        .as_ref()
                        .map(|h| h.host_id.as_slice())
                        .unwrap_or(&[]);
                    for key in keys {
                        let path = paths.credential_store.join(format!("{key}.fks"));
                        snapshot
                            .validated_files
                            .insert(path.clone(), files::regular(&path)?);
                        p.vault.push(vault.inventory_record(&key, &hard, host)?);
                    }
                }
                p.blockers = hard
                    .portability_blockers()?
                    .into_iter()
                    .map(|(workflow, n)| format!("{workflow}: {n}"))
                    .collect();
                p.checkpoint = Some(checkpoint);
            } else if snapshot
                .native
                .records
                .contains_key(&crate::checkpoint::rollback_record_key(&name)?)
            {
                return Err(Error::InvalidConfig("claimed profile database is missing"));
            }
            if paths.soft_database.exists() {
                if p.checkpoint.is_none() {
                    return Err(Error::InvalidConfig("registry-only profile has soft state"));
                }
                snapshot.validated_files.insert(
                    paths.soft_database.clone(),
                    files::regular(&paths.soft_database)?,
                );
                foks_client_db::SoftStateStore::inspect_existing(&paths.soft_database)?;
            }
            snapshot.profiles.insert(name, p);
        }
        if snapshot
            .native
            .records
            .keys()
            .any(|key| !expected.contains(key))
        {
            return Err(Error::InvalidConfig(
                "native manifest contains pending publication or unsupported ownership records",
            ));
        }
        snapshot.walk()?;
        if DirectoryIdentity::read(root)? != snapshot.identity {
            return Err(Error::StatePathChanged);
        }
        Ok(snapshot)
    }
}
impl StateSnapshot {
    fn add(&mut self, path: &Path, soft: bool) -> Result<()> {
        if self.artifacts.len() >= files::MAX_ENTRIES {
            return Err(Error::InvalidConfig("snapshot entry limit exceeded"));
        }
        if let Some(before) = self.validated_files.get(path) {
            if files::changed(before, &files::regular(path)?) {
                return Err(Error::StatePathChanged);
            }
        } else if path.parent() != Some(self.root.join("trust").as_path()) {
            return Err(Error::StatePathChanged);
        }
        let artifact = files::artifact(&self.root, path, soft)?;
        let total = self
            .artifacts
            .iter()
            .try_fold(artifact.size, |n, a| n.checked_add(a.size))
            .ok_or(Error::StatePathChanged)?;
        if total > files::MAX_TOTAL {
            return Err(Error::InvalidConfig("snapshot byte limit exceeded"));
        }
        self.artifacts.push(artifact);
        Ok(())
    }
    fn walk(&mut self) -> Result<()> {
        for (name, path) in files::entries(&self.root)? {
            match name.as_str() {
                "client-state.toml" | "profiles.toml" => self.add(&path, false)?,
                "profiles" => self.walk_profiles(&path)?,
                ".state-import-v1" => {
                    if self.allowed_import_marker
                        != Some(files::artifact(&self.root, &path, false)?.sha256)
                    {
                        return Err(Error::StateRecoveryRequired);
                    }
                }
                ".state-relocation-v1" => {
                    super::relocation::validate_inventory_marker(self, &path)?
                }
                "trust" => {
                    for (name, path) in files::entries(&path)? {
                        let digest = name.strip_suffix(".der").ok_or(Error::TrustRoot)?;
                        let certificate = trust::read_certificate(
                            &self.root,
                            &crate::TrustRoot::CertificateArtifact {
                                sha256: digest.into(),
                            },
                        )?
                        .ok_or(Error::TrustRoot)?;
                        // Unreferenced, digest-valid artifacts can remain after a
                        // profile removal or an interrupted normalization. They
                        // grant no authority until a profile references them.
                        self.trust.insert(digest.into(), certificate);
                        self.add(&path, false)?;
                    }
                }
                ".database-operation-locks" => {
                    for (name, path) in files::entries(&path)? {
                        let id = name.strip_suffix(".lock").ok_or(Error::StatePathChanged)?;
                        if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                            return Err(Error::StatePathChanged);
                        }
                        files::regular(&path)?;
                    }
                }
                ".foks-rs.lock"
                | "agent.desktop-agent.lock"
                | "foks-rs.desktop-agent.lock"
                | ".profiles.lock"
                | ".native-manifest.lock"
                | "agent.log" => {
                    files::regular(&path)?;
                }
                "crashes" => files::private_directory(&path)?,
                "agent.sock" | "foks-rs.sock" => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::FileTypeExt as _;
                        if !std::fs::symlink_metadata(&path)?.file_type().is_socket() {
                            return Err(Error::StatePathChanged);
                        }
                    }
                }
                _ => {
                    return Err(Error::InvalidConfig(
                        "unknown root artifact blocks portability",
                    ))
                }
            }
        }
        self.artifacts.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(())
    }
    fn walk_profiles(&mut self, directory: &Path) -> Result<()> {
        for (name, path) in files::entries(directory)? {
            let p = self.profiles.get(&name).ok_or(Error::InvalidConfig(
                "unpublished or unknown profile blocks portability",
            ))?;
            let initialized = p.checkpoint.is_some();
            let vault: BTreeSet<_> = p.vault.iter().map(|v| format!("{}.fks", v.key)).collect();
            let protected: BTreeSet<_> = p.protected.keys().cloned().collect();
            for (name, path) in files::entries(&path)? {
                match name.as_str() {
                    ".profile-operation.lock" | ".scheduler-run.lock" => {
                        files::regular(&path)?;
                    }
                    "hard.sqlite3" if initialized => self.add(&path, false)?,
                    "soft.sqlite3" if initialized => self.add(&path, true)?,
                    "hard.sqlite3-wal"
                    | "hard.sqlite3-journal"
                    | "soft.sqlite3-wal"
                    | "soft.sqlite3-journal"
                        if initialized =>
                    {
                        if files::regular(&path)?.len() != 0 {
                            return Err(Error::InvalidConfig(
                                "nonempty SQLite journal blocks portability",
                            ));
                        }
                    }
                    "hard.sqlite3-shm" | "soft.sqlite3-shm" if initialized => {
                        if files::regular(&path)?.len() > 1024 * 1024 {
                            return Err(Error::StatePathChanged);
                        }
                    }
                    "credentials" | "mutations" if initialized => {
                        let allowed = if name == "credentials" {
                            &vault
                        } else {
                            &protected
                        };
                        for (name, path) in files::entries(&path)? {
                            if !allowed.contains(&name) {
                                return Err(Error::InvalidConfig("unattributed record blocks portability; complete owning retention cleanup"));
                            }
                            self.add(&path, false)?;
                        }
                    }
                    _ => {
                        return Err(Error::InvalidConfig(
                            "unknown or uninitialized profile artifact blocks portability",
                        ))
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct StateInventoryReport {
    pub backend: CredentialBackend,
    pub profiles: Vec<ProfileInventoryReport>,
    pub bytes: u64,
    pub files: usize,
    pub exportable: bool,
    pub digest: String,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct ProfileInventoryReport {
    pub name: String,
    pub initialized: bool,
    pub accounts: Vec<String>,
    pub blockers: Vec<String>,
}
impl StateSnapshot {
    pub(super) fn report(&self) -> Result<StateInventoryReport> {
        let profiles = self
            .profiles
            .iter()
            .map(|(name, p)| {
                let mut blockers = p.blockers.clone();
                blockers.extend(p.vault.iter().filter(|v| !v.exportable).map(|v| {
                    format!(
                        "pending {}",
                        v.key.split('.').next().unwrap_or("vault workflow")
                    )
                }));
                blockers.sort();
                blockers.dedup();
                ProfileInventoryReport {
                    name: name.clone(),
                    initialized: p.checkpoint.is_some(),
                    accounts: p
                        .vault
                        .iter()
                        .filter_map(|v| {
                            v.key
                                .strip_prefix("account.")
                                .or_else(|| v.key.strip_prefix("yubi-account."))
                                .or_else(|| v.key.strip_prefix("bot-account."))
                                .map(str::to_owned)
                        })
                        .collect(),
                    blockers,
                }
            })
            .collect();
        Ok(StateInventoryReport {
            backend: CredentialBackend::Native,
            profiles,
            bytes: self.artifacts.iter().map(|a| a.size).sum(),
            files: self.artifacts.len(),
            exportable: self.require_exportable().is_ok(),
            digest: crate::hex(&self.digest()?),
        })
    }
}
/// Passive security-state inspection under maintenance exclusion. Runtime lock files
/// may be created, but roots, databases, registry and native records are never repaired.
pub fn inspect_native_state(root: impl AsRef<Path>) -> Result<StateInventoryReport> {
    let root = super::lease::canonical_reservation(root.as_ref())?;
    let mut guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&root))?;
    guard.inspect(&root)?.report()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientCredentials, ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot};
    fn enabled() -> bool {
        std::env::var_os("FOKS_TEST_NATIVE_PORTABILITY").is_some()
    }
    fn native_root(root: &Path) -> String {
        let credentials = ClientCredentials::initialize(root, CredentialBackend::Native).unwrap();
        let mut registry = ProfileRegistry::open(root).unwrap();
        for name in ["initialized", "registry-only"] {
            registry
                .add(Profile {
                    name: name.into(),
                    label: None,
                    probe: "foks.app".into(),
                    protocol: ProtocolPolicy::V019,
                    trust: TrustRoot::WebPki,
                })
                .unwrap();
        }
        let session = ProfileSession::open(&registry, "initialized").unwrap();
        credentials.with_checked_session(&session,|_|{
            let verified=foks_verify::verify_public_host("foks.app",include_bytes!("../../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"))?.snapshot;
            let mut hard=HardStateStore::open(&session.paths.hard_database)?;
            hard.accept_verified_host(&verified)?;
            Ok::<_,Error>(())
        }).unwrap();
        credentials.state_id.clone()
    }
    #[test]
    fn native_snapshot_accepts_initialized_and_registry_only_profiles_without_repair() {
        if !enabled() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state");
        let id = native_root(&root);
        let report = inspect_native_state(&root).unwrap();
        assert!(report.exportable);
        assert_eq!(report.profiles.len(), 2);
        assert!(!root.join("profiles/registry-only/hard.sqlite3").exists());
        let before = std::fs::read(root.join("profiles/initialized/hard.sqlite3")).unwrap();
        assert_eq!(report.digest, inspect_native_state(&root).unwrap().digest);
        assert_eq!(
            before,
            std::fs::read(root.join("profiles/initialized/hard.sqlite3")).unwrap()
        );
        foks_keystore::NativeCredentialStore::open(&id)
            .unwrap()
            .remove(crate::checkpoint::NATIVE_MANIFEST_RECORD)
            .unwrap();
    }
    #[test]
    fn normalization_binds_preview_bytes_and_shares_artifacts_across_profiles() {
        if !enabled() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state");
        let id = native_root(&root);
        let external = dir.path().join("external.der");
        foks_server_testkit::TestEnvironment::new()
            .unwrap()
            .write_probe_root(&external)
            .unwrap();
        let original = std::fs::read(&external).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        for name in ["initialized", "registry-only"] {
            let mut p = registry.profile(name).unwrap().clone();
            p.trust = TrustRoot::CertificateDer {
                path: external.clone(),
            };
            registry.replace(p).unwrap();
        }
        drop(registry);
        let canonical = root.canonicalize().unwrap();
        let mut guard =
            ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&canonical)).unwrap();
        let snapshot = guard.inspect(&canonical).unwrap();
        std::fs::write(&external, b"changed after preview").unwrap();
        let normalized = snapshot.normalize_trust(&mut guard).unwrap();
        assert_eq!(normalized.trust.len(), 1);
        assert_eq!(std::fs::read_dir(root.join("trust")).unwrap().count(), 1);
        for p in normalized.profiles.values() {
            assert!(matches!(
                p.profile.trust,
                TrustRoot::CertificateArtifact { .. }
            ));
            assert_eq!(
                trust::read_certificate(&root, &p.profile.trust)
                    .unwrap()
                    .unwrap(),
                original
            );
        }
        let artifact = root
            .join("trust")
            .join(format!("{}.der", crate::hex(&Sha256::digest(&original))));
        std::fs::write(&artifact, b"invalid replacement").unwrap();
        assert!(
            trust::read_certificate(&root, &normalized.profiles["initialized"].profile.trust)
                .is_err()
        );
        foks_keystore::NativeCredentialStore::open(&id)
            .unwrap()
            .remove(crate::checkpoint::NATIVE_MANIFEST_RECORD)
            .unwrap();
    }

    #[test]
    fn native_snapshot_rejects_orphans_unknown_vaults_and_missing_claimed_databases() {
        if !enabled() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state");
        let id = native_root(&root);
        let credentials = ClientCredentials::open(&root).unwrap();
        let master = credentials.master_key().unwrap();
        drop(credentials);
        let mutations = root.join("profiles/initialized/mutations");
        let mut protected = foks_client::EncryptedFileMutationStore::open(
            &mutations,
            crate::derive_mutation_key(&master),
        )
        .unwrap();
        protected
            .put_if_absent(b"unattributed", b"authenticated")
            .unwrap();
        drop(protected);
        assert!(inspect_native_state(&root).is_err());
        std::fs::remove_dir_all(&mutations).unwrap();
        let vault = root.join("profiles/initialized/credentials");
        let mut store =
            foks_keystore::EncryptedFileSecretStore::open(&vault, crate::derive_vault_key(&master))
                .unwrap();
        store.put("future.owner", b"authenticated").unwrap();
        drop(store);
        assert!(inspect_native_state(&root).is_err());
        std::fs::remove_dir_all(&vault).unwrap();
        std::fs::rename(
            root.join("profiles/initialized/hard.sqlite3"),
            dir.path().join("withheld"),
        )
        .unwrap();
        assert!(inspect_native_state(&root).is_err());
        assert!(!root.join("profiles/initialized/hard.sqlite3").exists());
        foks_keystore::NativeCredentialStore::open(&id)
            .unwrap()
            .remove(crate::checkpoint::NATIVE_MANIFEST_RECORD)
            .unwrap();
    }
    #[test]
    fn native_copy_and_private_file_are_not_portability_authority() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("private");
        drop(ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap());
        assert!(matches!(
            inspect_native_state(&root),
            Err(Error::PortabilityUnsupported)
        ));
        if !enabled() {
            return;
        }
        let source = dir.path().join("native");
        let id = native_root(&source);
        let copy = dir.path().join("copy");
        crate::prepare_private_directory(&copy).unwrap();
        crate::create_private_config(
            &copy.join(crate::STATE_CONFIG_FILE),
            &std::fs::read(source.join(crate::STATE_CONFIG_FILE)).unwrap(),
        )
        .unwrap();
        assert!(inspect_native_state(&copy).is_err());
        foks_keystore::NativeCredentialStore::open(&id)
            .unwrap()
            .remove(crate::checkpoint::NATIVE_MANIFEST_RECORD)
            .unwrap();
    }
    #[test]
    fn relocation_preserves_erased_uncertain_oidc_without_replay() {
        if !enabled() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("source");
        let destination = dir.path().join("destination");
        let id = native_root(&root);
        {
            let credentials = ClientCredentials::open(&root).unwrap();
            let registry = ProfileRegistry::open(&root).unwrap();
            let session = ProfileSession::open(&registry, "initialized").unwrap();
            credentials
                .with_checked_session(&session, |checked| {
                    use foks_client_db::{SsoFlow, SsoFlowState as S};
                    let mut db = HardStateStore::open(&session.paths.hard_database)?;
                    let flow = SsoFlow {
                        id: [11; 16],
                        host: checked.pinned_host()?.host_id().as_bytes().to_vec(),
                        uid: vec![1; 33],
                        device: vec![4; 33],
                        purpose: foks_proto::SsoPurpose::LinkExisting,
                        state: S::Prepared,
                        material_hash: [5; 32],
                        config_hash: [6; 32],
                        expires_at_ms: 600_000,
                        commitment: None,
                        final_operation: None,
                    };
                    // Simulate the legitimately erased Unknown stage without network
                    // work; the public journal remains nonterminal and export-blocking.
                    db.sso_record_with_material(&flow, 1, || Ok::<_, Error>(()))?;
                    db.sso_transition(&flow.id, S::Prepared, S::AwaitingBrowser, None)?;
                    db.sso_transition(&flow.id, S::AwaitingBrowser, S::Ready, None)?;
                    db.sso_set_commitment(&flow.id, &[7; 32])?;
                    db.sso_transition(&flow.id, S::Ready, S::Binding, None)?;
                    db.sso_transition(&flow.id, S::Binding, S::Unknown, None)?;
                    Ok::<_, Error>(())
                })
                .unwrap();
        }
        let before = std::fs::read(root.join("profiles/initialized/hard.sqlite3")).unwrap();
        assert!(!inspect_native_state(&root).unwrap().exportable);
        super::super::relocate_state(&root, &destination).unwrap();
        assert_eq!(
            before,
            std::fs::read(destination.join("profiles/initialized/hard.sqlite3")).unwrap()
        );
        assert!(!inspect_native_state(&destination).unwrap().exportable);
        assert_eq!(
            HardStateStore::inspect_existing(
                &destination.join("profiles/initialized/hard.sqlite3")
            )
            .unwrap()
            .sso_flow(&[11; 16])
            .unwrap()
            .unwrap()
            .state,
            foks_client_db::SsoFlowState::Unknown
        );
        foks_keystore::NativeCredentialStore::open(&id)
            .unwrap()
            .remove(crate::checkpoint::NATIVE_MANIFEST_RECORD)
            .unwrap();
    }

    #[test]
    fn relocation_uses_root_owned_custom_ca_after_external_file_is_gone() {
        if !enabled() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("source");
        let destination = dir.path().join("destination");
        let id = native_root(&root);
        let external = dir.path().join("ca.der");
        foks_server_testkit::TestEnvironment::new()
            .unwrap()
            .write_probe_root(&external)
            .unwrap();
        let original = std::fs::read(&external).unwrap();
        {
            let mut registry = ProfileRegistry::open(&root).unwrap();
            let mut profile = registry.profile("initialized").unwrap().clone();
            profile.trust = TrustRoot::CertificateDer {
                path: external.clone(),
            };
            registry.replace(profile).unwrap();
        }
        super::super::relocate_state(&root, &destination).unwrap();
        std::fs::remove_file(&external).unwrap();
        {
            let registry = ProfileRegistry::open(&destination).unwrap();
            let session = ProfileSession::open(&registry, "initialized").unwrap();
            assert_eq!(
                trust::read_certificate(&destination, &session.profile.trust)
                    .unwrap()
                    .unwrap(),
                original
            );
        }
        foks_keystore::NativeCredentialStore::open(&id)
            .unwrap()
            .remove(crate::checkpoint::NATIVE_MANIFEST_RECORD)
            .unwrap();
    }
}
