//! Closed extraction and typed rekey, before any native publication.
use super::*;
use foks_client::ProtectedMutationStore as _;
use foks_client_db::{HardStateStore, ImportAccount, ImportAccountKind};
use foks_keystore::SecretStore as _;
use std::os::unix::fs::OpenOptionsExt as _;

pub(super) const MAX_ARCHIVE: u64 = files::MAX_TOTAL + 10 * 1024 * 1024;
pub(super) fn open_archive(
    path: &Path,
    key: &StateTransferKey,
) -> Result<(ArchiveReader<File>, ArchiveManifest)> {
    let reader = ArchiveReader::new(files::open_regular_bounded(path, MAX_ARCHIVE)?, key)?;
    let manifest = ArchiveManifest::decode(reader.manifest())?;
    Ok((reader, manifest))
}
pub(super) fn extract(
    reader: &mut ArchiveReader<File>,
    manifest: &ArchiveManifest,
    identity: &Identity,
    hook: &mut Hook<'_>,
) -> Result<()> {
    for entry in &manifest.entries {
        identity.verify_parent()?;
        if DirectoryIdentity::read(&identity.staging)? != identity.root {
            return Err(Error::StatePathChanged);
        }
        let path = identity.staging.join(&entry.path);
        let relative = Path::new(&entry.path)
            .parent()
            .ok_or(Error::StatePathChanged)?;
        let mut parent = identity.staging.clone();
        for component in relative.components() {
            parent.push(component.as_os_str());
            if files::exists(&parent)? {
                files::private_directory(&parent)?;
            } else {
                hook("before-extract-directory")?;
                crate::prepare_private_directory(&parent)?;
                File::open(parent.parent().ok_or(Error::StatePathChanged)?)?.sync_all()?;
                hook("after-extract-directory")?;
            }
        }
        hook("before-extract-file")?;
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        let mut output = HashWriter {
            file,
            hash: Sha256::new(),
        };
        reader.read_entry(entry.size, &mut output)?;
        if <[u8; 32]>::from(output.hash.finalize()) != entry.sha256 {
            return Err(Error::StatePathChanged);
        }
        hook("before-extract-file-sync")?;
        output.file.sync_all()?;
        hook("after-extract-file-sync")?;
        hook("before-extract-parent-sync")?;
        File::open(path.parent().ok_or(Error::StatePathChanged)?)?.sync_all()?;
        hook("after-extract-parent-sync")?;
    }
    Ok(())
}
struct HashWriter {
    file: File,
    hash: Sha256,
}
impl std::io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(bytes)?;
        self.hash.update(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
pub(super) fn validate_source(
    guard: &mut ClientStateMaintenanceGuard,
    manifest: &ArchiveManifest,
    identity: &Identity,
) -> Result<StateSnapshot> {
    let snapshot = guard.inspect_with_projection(
        &identity.staging,
        Some((&manifest.source_root, &manifest.native)),
    )?;
    if snapshot.state_id != manifest.source_state_id
        || snapshot.profiles.len() != manifest.profiles.len()
    {
        return Err(Error::StateRecoveryRequired);
    }
    let mut artifacts = snapshot.artifacts.clone();
    artifacts.sort_by(|a, b| a.path.cmp(&b.path));
    if artifacts != manifest.entries {
        return Err(Error::StatePathChanged);
    }
    for (name, profile) in &snapshot.profiles {
        let declared = manifest
            .profiles
            .get(name)
            .ok_or(Error::StateRecoveryRequired)?;
        let vault = profile
            .vault
            .iter()
            .map(|v| v.key.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let protected = profile
            .protected
            .values()
            .map(|v| v.key.as_slice())
            .collect::<std::collections::BTreeSet<_>>();
        if profile.checkpoint.is_some() != declared.initialized
            || vault != declared.vault.iter().map(String::as_str).collect()
            || protected != declared.protected.iter().map(Vec::as_slice).collect()
        {
            return Err(Error::StateRecoveryRequired);
        }
    }
    snapshot.require_exportable()?;
    Ok(snapshot)
}
pub(super) struct DestinationAuthority {
    pub projection: NativeManifestStore,
    pub claims: BTreeMap<String, Vec<u8>>,
}
pub(super) fn rekey(
    snapshot: &StateSnapshot,
    identity: &Identity,
    new_master: &[u8; 32],
    hook: &mut Hook<'_>,
) -> Result<DestinationAuthority> {
    let old_master = snapshot.master()?;
    let mut projection = NativeManifestStore::initialized(&identity.destination, new_master);
    let pending = identity.staging.join(crate::pending_chat::DIRECTORY);
    if files::exists(&pending)? {
        let temporary = identity.staging.join(format!(
            ".import-chat-intents-{}",
            crate::hex(&identity.nonce)
        ));
        hook("before-chat-intents-rekey")?;
        crate::pending_chat::rekey(&pending, &temporary, &old_master, new_master)?;
        hook("after-chat-intents-rekey")?;
        replace_rekey_directory(identity, &pending, &temporary, hook)?;
    }
    for (name, profile) in &snapshot.profiles {
        let directory = identity.staging.join("profiles").join(name);
        let vault_path = directory.join("credentials");
        if files::exists(&vault_path)? {
            let temporary = directory.join(format!(
                ".import-credentials-{}",
                crate::hex(&identity.nonce)
            ));
            let mut old = foks_keystore::EncryptedFileSecretStore::inspect_existing(
                &vault_path,
                crate::derive_vault_key(&old_master),
            )?;
            let mut new = foks_keystore::EncryptedFileSecretStore::open(
                &temporary,
                crate::derive_vault_key(new_master),
            )?;
            for record in &profile.vault {
                let bytes = old.get(&record.key)?;
                hook("before-vault-rekey")?;
                new.put(&record.key, &bytes)?;
                hook("after-vault-rekey")?;
            }
            drop(old);
            drop(new);
            replace_rekey_directory(identity, &vault_path, &temporary, hook)?;
        }
        let mutations = directory.join("mutations");
        if files::exists(&mutations)? {
            let temporary =
                directory.join(format!(".import-mutations-{}", crate::hex(&identity.nonce)));
            let mut old = foks_client::EncryptedFileMutationStore::inspect_existing(
                &mutations,
                crate::derive_mutation_key(&old_master),
            )?;
            let mut new = foks_client::EncryptedFileMutationStore::open(
                &temporary,
                crate::derive_mutation_key(new_master),
            )?;
            for record in profile.protected.values() {
                match old.get(&record.key) {
                    Ok(bytes) => {
                        record.validate_payload(&bytes)?;
                        hook("before-mutation-rekey")?;
                        new.put_if_absent(&record.key, &bytes)?;
                        hook("after-mutation-rekey")?;
                    }
                    Err(foks_client::ProtectedStoreError::Missing)
                        if record.presence != foks_client::ProtectedPresence::Required => {}
                    Err(e) => return Err(e.into()),
                }
            }
            drop(old);
            drop(new);
            replace_rekey_directory(identity, &mutations, &temporary, hook)?;
        }
        if profile.checkpoint.is_some() {
            let mut accounts = Vec::new();
            for record in &profile.vault {
                let Some((family, alias)) = record.key.split_once('.') else {
                    return Err(Error::StateRecoveryRequired);
                };
                let kind = match family {
                    "account" => ImportAccountKind::Software,
                    "yubi-account" => ImportAccountKind::Yubi,
                    "bot-account" => ImportAccountKind::Bot,
                    _ => continue,
                };
                accounts.push(ImportAccount {
                    alias: alias.into(),
                    kind,
                });
            }
            let hard = directory.join("hard.sqlite3");
            let mut db = HardStateStore::open(&hard)?;
            hook("before-readiness-write")?;
            db.install_import_readiness(identity.archive_id, identity.nonce, &accounts)?;
            hook("after-readiness-write")?;
            let gate = super::super::readiness::NativeReadiness::from_state(
                &db.import_readiness()?.ok_or(Error::StateRecoveryRequired)?,
            )?;
            projection.put(
                &super::super::readiness::key(name)?,
                &serde_json::to_vec(&gate)?,
            )?;
            let checkpoint = crate::registry::checkpoint_for_store(&profile.profile, &db)?;
            projection.put(
                &crate::checkpoint::rollback_record_key(name)?,
                &serde_json::to_vec(&checkpoint)?,
            )?;
            projection.put(
                &crate::checkpoint::database_claim_record_key(&checkpoint.database_id),
                name.as_bytes(),
            )?;
            hook("before-import-checkpoint")?;
            db.checkpoint_for_snapshot()?;
            hook("after-import-checkpoint")?;
            drop(db);
            hook("before-import-database-sync")?;
            File::open(&hard)?.sync_all()?;
            File::open(&directory)?.sync_all()?;
            hook("after-import-database-sync")?;
        }
    }
    let state = crate::checkpoint::ClientStateFile {
        version: crate::STATE_CONFIG_VERSION,
        state_id: identity.state_id.clone(),
        credential_backend: crate::CredentialBackend::Native,
    };
    hook("before-import-state-config")?;
    crate::atomic_private_write(
        &identity.staging.join(crate::STATE_CONFIG_FILE),
        toml::to_string_pretty(&state)?.as_bytes(),
    )?;
    hook("after-import-state-config")?;
    let claims = projection
        .records
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                crate::MASTER_KEY_RECORD | crate::STATE_ROOT_RECORD
            )
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok(DestinationAuthority { projection, claims })
}
fn replace_rekey_directory(
    identity: &Identity,
    old: &Path,
    new: &Path,
    hook: &mut Hook<'_>,
) -> Result<()> {
    identity.verify_parent()?;
    if DirectoryIdentity::read(&identity.staging)? != identity.root {
        return Err(Error::StatePathChanged);
    }
    hook("before-rekey-remove")?;
    fs::remove_dir_all(old)?;
    hook("after-rekey-remove")?;
    hook("before-rekey-rename")?;
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        new,
        rustix::fs::CWD,
        old,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(std::io::Error::from)?;
    hook("after-rekey-rename")?;
    File::open(old.parent().ok_or(Error::StatePathChanged)?)?.sync_all()?;
    hook("after-rekey-parent-sync")
}
