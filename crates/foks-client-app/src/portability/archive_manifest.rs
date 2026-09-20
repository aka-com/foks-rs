//! Application-owned, closed transfer manifest. The native projection zeroizes on drop.
use super::{files::Artifact, inventory::StateSnapshot};
use crate::{checkpoint::NativeManifestStore, Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileArchive {
    pub initialized: bool,
    pub vault: Vec<String>,
    pub protected: Vec<Vec<u8>>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArchiveManifest {
    pub version: u32,
    pub source_platform: String,
    pub source_root: PathBuf,
    pub source_state_id: String,
    pub created_at: u64,
    pub native: NativeManifestStore,
    pub profiles: BTreeMap<String, ProfileArchive>,
    pub entries: Vec<Artifact>,
}
impl ArchiveManifest {
    pub fn from_snapshot(snapshot: &StateSnapshot) -> Result<Self> {
        snapshot.require_exportable()?;
        let mut native = NativeManifestStore::decode(&snapshot.native.encode()?)?;
        native.records.remove(super::relocation::RECEIPT);
        native.records.remove(super::import::RECEIPT);
        let profiles = snapshot
            .profiles
            .iter()
            .map(|(name, p)| {
                (
                    name.clone(),
                    ProfileArchive {
                        initialized: p.checkpoint.is_some(),
                        vault: p.vault.iter().map(|v| v.key.clone()).collect(),
                        protected: p.protected.values().map(|v| v.key.clone()).collect(),
                    },
                )
            })
            .collect();
        let mut entries = snapshot.artifacts.clone();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let manifest = Self {
            version: 2,
            source_platform: std::env::consts::OS.into(),
            source_root: snapshot.root.clone(),
            source_state_id: snapshot.state_id.clone(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::InvalidConfig("system clock precedes Unix epoch"))?
                .as_secs(),
            native,
            profiles,
            entries,
        };
        manifest.validate()?;
        Ok(manifest)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > foks_keystore::state_archive::MAX_MANIFEST {
            return Err(Error::InvalidConfig("archive manifest exceeds limit"));
        }
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|_| Error::InvalidConfig("archive manifest is invalid"))?;
        manifest.validate()?;
        Ok(manifest)
    }
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.version, 1 | 2)
            || !matches!(self.source_platform.as_str(), "linux" | "macos")
            || !self.source_root.is_absolute()
            || self.profiles.len() > 256
            || self.entries.len() > super::files::MAX_ENTRIES
        {
            return invalid();
        }
        crate::validate_name(&self.source_state_id)?;
        // Decode through the owning native validator, including version and limits.
        let native = NativeManifestStore::decode(&self.native.encode()?)?;
        if native
            .records
            .get(crate::STATE_ROOT_RECORD)
            .map(Vec::as_slice)
            != Some(crate::state_root_binding(&self.source_root).as_slice())
            || native
                .records
                .get(crate::MASTER_KEY_RECORD)
                .is_none_or(|k| k.len() != 32)
            || native.records.contains_key(super::relocation::INTENT)
            || native.records.contains_key(super::import::INTENT)
            || native.records.contains_key(super::import::RECEIPT)
            || native.records.contains_key(super::relocation::RECEIPT)
            || native
                .records
                .keys()
                .any(|k| k.starts_with(super::readiness::PREFIX))
        {
            return invalid();
        }
        // Resolve each logical key once, before walking file entries. This keeps
        // attacker-controlled manifests linear in their bounded inventory size.
        let mut profile_files = BTreeMap::new();
        for (name, p) in &self.profiles {
            crate::validate_name(name)?;
            if p.vault.len() > 8192
                || p.protected.len() > 8192
                || (!p.initialized && (!p.vault.is_empty() || !p.protected.is_empty()))
            {
                return invalid();
            }
            let mut vault = BTreeSet::new();
            for key in &p.vault {
                crate::account::validate_archive_vault_key(key)?;
                if key.len() > 128 || !vault.insert(key.as_str()) {
                    return invalid();
                }
            }
            let mut protected = BTreeSet::new();
            for key in &p.protected {
                if key.is_empty() || key.len() > 4096 {
                    return invalid();
                }
                if !protected.insert(foks_client::EncryptedFileMutationStore::record_filename(
                    key,
                )?) {
                    return invalid();
                }
            }
            profile_files.insert(name.as_str(), (vault, protected));
        }
        let mut paths = BTreeSet::new();
        let mut previous = None;
        let mut total = 0u64;
        for entry in &self.entries {
            if entry.path.len() > 512
                || entry.mode != 0o600
                || entry.size > super::files::MAX_FILE
                || previous.is_some_and(|p: &str| p >= entry.path.as_str())
                || entry.path.contains('\\')
            {
                return invalid();
            }
            let parts = entry.path.split('/').collect::<Vec<_>>();
            if parts
                .iter()
                .any(|p| p.is_empty() || matches!(*p, "." | ".."))
            {
                return invalid();
            }
            let soft = match parts.as_slice() {
                ["client-state.toml" | "profiles.toml"] => false,
                ["chat-intents", "state.fks"] if self.version == 2 => false,
                ["trust", file] => {
                    let hash = file.strip_suffix(".der").ok_or(Error::TrustRoot)?;
                    super::trust::validate_digest(hash)?;
                    false
                }
                ["profiles", name, "hard.sqlite3" | "soft.sqlite3"] => {
                    let profile = self
                        .profiles
                        .get(*name)
                        .ok_or(Error::InvalidConfig("archive names an unknown profile"))?;
                    if !profile.initialized {
                        return invalid();
                    }
                    parts[2] == "soft.sqlite3"
                }
                ["profiles", name, "credentials", file] => {
                    let profile = self
                        .profiles
                        .get(*name)
                        .ok_or(Error::InvalidConfig("archive names an unknown profile"))?;
                    let key = file
                        .strip_suffix(".fks")
                        .ok_or(Error::InvalidConfig("invalid archive vault entry"))?;
                    if !profile.initialized || !profile_files[*name].0.contains(key) {
                        return invalid();
                    }
                    false
                }
                ["profiles", name, "mutations", file] => {
                    let profile = self
                        .profiles
                        .get(*name)
                        .ok_or(Error::InvalidConfig("archive names an unknown profile"))?;
                    if !profile.initialized || !profile_files[*name].1.contains(*file) {
                        return invalid();
                    }
                    false
                }
                _ => return invalid(),
            };
            if entry.soft != soft {
                return invalid();
            }
            total = total
                .checked_add(entry.size)
                .ok_or(Error::StatePathChanged)?;
            if total > super::files::MAX_TOTAL {
                return invalid();
            }
            previous = Some(entry.path.as_str());
            paths.insert(entry.path.as_str());
        }
        if !paths.contains("client-state.toml")
            || (!self.profiles.is_empty() && !paths.contains("profiles.toml"))
        {
            return invalid();
        }
        for (name, p) in &self.profiles {
            if p.initialized != paths.contains(format!("profiles/{name}/hard.sqlite3").as_str()) {
                return invalid();
            }
            for key in &p.vault {
                if !paths.contains(format!("profiles/{name}/credentials/{key}.fks").as_str()) {
                    return invalid();
                }
            }
        }
        Ok(())
    }
}
fn invalid<T>() -> Result<T> {
    Err(Error::InvalidConfig(
        "archive manifest violates the closed state format",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> ArchiveManifest {
        let root = PathBuf::from("/source");
        ArchiveManifest {
            version: 1,
            source_platform: "linux".into(),
            source_root: root.clone(),
            source_state_id: "source".into(),
            created_at: 1,
            native: NativeManifestStore::initialized(&root, &[7; 32]),
            profiles: BTreeMap::new(),
            entries: vec![Artifact {
                path: "client-state.toml".into(),
                size: 1,
                mode: 0o600,
                sha256: [0; 32],
                soft: false,
            }],
        }
    }
    #[test]
    fn shape_rejects_traversal_duplicates_modes_unknown_profiles_and_vaults() {
        manifest().validate().unwrap();
        for path in [
            "../escape",
            "/absolute",
            "profiles/unknown/hard.sqlite3",
            "profiles//hard.sqlite3",
            "profiles/../hard.sqlite3",
            "master.key",
            "client-state.toml/extra",
        ] {
            let mut m = manifest();
            m.entries[0].path = path.into();
            assert!(m.validate().is_err(), "{path}");
        }
        let mut m = manifest();
        m.entries.push(m.entries[0].clone());
        assert!(m.validate().is_err());
        let mut m = manifest();
        m.entries[0].mode = 0o777;
        assert!(m.validate().is_err());
        let mut m = manifest();
        m.entries[0].soft = true;
        assert!(m.validate().is_err());
        assert!(crate::account::validate_archive_vault_key("unknown.account").is_err());
        assert!(crate::account::validate_archive_vault_key("pending.account").is_err());
    }
    #[test]
    fn source_projection_cannot_carry_an_active_intent_or_completion_receipt() {
        for key in [
            super::super::relocation::INTENT,
            super::super::relocation::RECEIPT,
        ] {
            let mut m = manifest();
            m.native.records.insert(key.into(), vec![]);
            assert!(m.validate().is_err());
        }
    }
}
