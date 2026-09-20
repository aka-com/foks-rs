use crate::{ClientCredentials, CredentialBackend, Error, Result};
use foks_keystore::{EncryptedFileSecretStore, SecretStore};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, path::Path};
use zeroize::{Zeroize, Zeroizing};

const MAX_INTENTS: usize = 128;
const KEY_DOMAIN: u64 = 0xa536_6912_2a5f_909c;

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalChatIntent {
    pub submission: String,
    pub text: String,
}

pub(crate) fn deserialize_secret_string<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Zeroizing<String>, D::Error> {
    String::deserialize(deserializer).map(Zeroizing::new)
}
impl<'de> Deserialize<'de> for LocalChatIntent {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            submission: String,
            #[serde(deserialize_with = "deserialize_secret_string")]
            text: Zeroizing<String>,
        }
        let mut wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            submission: wire.submission,
            text: std::mem::take(&mut *wire.text),
        })
    }
}

impl Drop for LocalChatIntent {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    profile: String,
    binding: Vec<u8>,
    intent: LocalChatIntent,
}

pub struct LocalChatIntentStore {
    store: EncryptedFileSecretStore,
    _credentials: ClientCredentials,
    source_key: Zeroizing<[u8; 32]>,
    _lock: Option<File>,
}

impl LocalChatIntentStore {
    /// Retain the already authorized legacy key while releasing the writer
    /// lock before IPC. Every subsequent store access reacquires and checks it.
    pub fn release_lock(&mut self) {
        self._lock = None;
    }

    fn acquire_lock(&mut self) -> Result<()> {
        if self._lock.is_none() {
            let directory = &self._credentials.root;
            crate::pending_chat::private_path(directory, true)?;
            let lock = crate::pending_chat::lock(&directory.join(".intent.lock"))?;
            legacy_inventory(directory)?;
            if crate::checkpoint::inspect_state_file(directory)?
                .is_some_and(|state| state.state_id != self._credentials.state_id)
            {
                return Err(Error::InvalidConfig(
                    "legacy saved message credentials changed",
                ));
            }
            self._lock = Some(lock);
        }
        Ok(())
    }

    /// Passive discovery: no credentials, initialization, or permission repair.
    pub fn has_existing(directory: &Path) -> Result<bool> {
        match std::fs::symlink_metadata(directory) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
            Ok(_) => {
                crate::pending_chat::private_path(directory, true)?;
            }
        }
        let _lock = crate::pending_chat::try_lock(&directory.join(".intent.lock"))?;
        let keys = legacy_inventory(directory)?;
        Ok(!keys.is_empty())
    }

    pub fn open_existing(directory: &Path) -> Result<Self> {
        crate::pending_chat::private_path(directory, true)?;
        let lock = crate::pending_chat::lock(&directory.join(".intent.lock"))?;
        legacy_inventory(directory)?;
        if directory.join("retired-client-state.toml").try_exists()? {
            return Err(Error::InvalidConfig(
                "legacy saved message store is retired",
            ));
        }
        let credentials = ClientCredentials::open(directory)?;
        if credentials.backend() != CredentialBackend::Native {
            return Err(Error::InvalidConfig(
                "legacy saved message credential backend changed",
            ));
        }
        let master = credentials.master_key()?;
        let store = EncryptedFileSecretStore::inspect_existing(
            directory.join("records"),
            Zeroizing::new(foks_crypto::prefixed_hash(KEY_DOMAIN, master.as_ref())),
        )?;
        Ok(Self {
            store,
            _credentials: credentials,
            source_key: Zeroizing::new(foks_crypto::prefixed_hash(
                0x95cc_e62a_e83f_684a,
                master.as_ref(),
            )),
            _lock: Some(lock),
        })
    }

    /// Authenticated records, with a keyed source commitment that reveals no
    /// guessable message text. The caller releases this store before agent IPC.
    pub fn existing_records(&mut self) -> Result<Vec<LegacyChatIntent>> {
        self.acquire_lock()?;
        let keys = self.store.keys()?;
        if keys.len() > MAX_INTENTS {
            return Err(Error::InvalidConfig("legacy saved message limit exceeded"));
        }
        keys.iter().map(|key| self.existing_record(key)).collect()
    }
    fn existing_record(&mut self, key: &str) -> Result<LegacyChatIntent> {
        let bytes = self.store.get(key)?;
        let record: Record = serde_json::from_slice(&bytes)
            .map_err(|_| Error::InvalidConfig("legacy saved message is invalid"))?;
        validate_intent(&record.intent)?;
        if record_key(&record.profile, &record.binding)? != key {
            return Err(Error::InvalidConfig(
                "legacy saved message identity changed",
            ));
        }
        let input = Zeroizing::new(serde_json::to_vec(&(
            &self._credentials.state_id,
            key,
            &record,
        ))?);
        let source = crate::hex(&foks_crypto::capability_mac(
            self.source_key.as_ref(),
            0x758c_e491_025a_d366,
            &input,
        ));
        Ok(LegacyChatIntent {
            profile: record.profile,
            binding: record.binding,
            intent: record.intent,
            source,
        })
    }

    /// Recheck the full authenticated commitment under the legacy writer lock.
    pub fn clear_imported(&mut self, record: &LegacyChatIntent) -> Result<()> {
        self.acquire_lock()?;
        let key = record_key(&record.profile, &record.binding)?;
        let current = match self.existing_record(&key) {
            Ok(record) => record,
            Err(Error::Keystore(foks_keystore::Error::Missing)) => return Ok(()),
            Err(error) => return Err(error),
        };
        if current.source != record.source {
            return Err(Error::InvalidConfig(
                "legacy saved message changed during migration",
            ));
        }
        self.store.remove(&key)?;
        Ok(())
    }

    /// Keep the empty old namespace as a harmless residual. Moving its config
    /// prevents older apps from silently creating a second recovery store:
    /// their initialize-on-open path refuses an existing records directory.
    pub fn retire_empty(directory: &Path) -> Result<()> {
        match std::fs::symlink_metadata(directory) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
            Ok(_) => {
                crate::pending_chat::private_path(directory, true)?;
            }
        }
        let _lock = crate::pending_chat::try_lock(&directory.join(".intent.lock"))?;
        if !legacy_inventory(directory)?.is_empty() {
            return Err(Error::InvalidConfig(
                "legacy saved messages still require recovery",
            ));
        }
        let source = directory.join(crate::STATE_CONFIG_FILE);
        if source.try_exists()? {
            crate::prepare_private_directory(&directory.join("records"))?;
            File::open(directory)?.sync_all()?;
            std::fs::rename(source, directory.join("retired-client-state.toml"))?;
            File::open(directory)?.sync_all()?;
        }
        Ok(())
    }

    pub fn open(directory: &Path) -> Result<Self> {
        Self::open_with_backend(directory, CredentialBackend::Native)
    }

    fn open_with_backend(directory: &Path, backend: CredentialBackend) -> Result<Self> {
        let directory = crate::prepare_private_directory(directory)?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(directory.join(".intent.lock"))?;
        lock.lock_exclusive()?;
        let credentials = if ClientCredentials::is_initialized(&directory)? {
            ClientCredentials::open(&directory)?
        } else {
            if directory.join("records").try_exists()? {
                return Err(Error::InvalidConfig(
                    "saved chat intent key configuration is missing",
                ));
            }
            ClientCredentials::initialize(&directory, backend)?
        };
        if credentials.backend() != backend {
            return Err(Error::InvalidConfig(
                "saved chat intent credential backend changed",
            ));
        }
        let master = credentials.master_key()?;
        let key = Zeroizing::new(foks_crypto::prefixed_hash(KEY_DOMAIN, master.as_ref()));
        let store = EncryptedFileSecretStore::open(directory.join("records"), key)?;
        Ok(Self {
            store,
            _credentials: credentials,
            source_key: Zeroizing::new(foks_crypto::prefixed_hash(
                0x95cc_e62a_e83f_684a,
                master.as_ref(),
            )),
            _lock: Some(lock),
        })
    }

    pub fn load(&mut self, profile: &str, binding: &[u8]) -> Result<Option<LocalChatIntent>> {
        self.acquire_lock()?;
        let key = record_key(profile, binding)?;
        let bytes = match self.store.get(&key) {
            Ok(bytes) => bytes,
            Err(foks_keystore::Error::Missing) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let record: Record = serde_json::from_slice(&bytes)?;
        validate_intent(&record.intent)?;
        if record.profile != profile || record.binding != binding {
            return Err(Error::InvalidConfig("saved chat intent identity changed"));
        }
        Ok(Some(record.intent))
    }

    pub fn save(&mut self, profile: &str, binding: &[u8], intent: &LocalChatIntent) -> Result<()> {
        validate_intent(intent)?;
        let key = record_key(profile, binding)?;
        if let Some(existing) = self.load(profile, binding)? {
            return if existing == *intent {
                Ok(())
            } else {
                Err(Error::InvalidConfig(
                    "another message is already saved for this channel; recover it first",
                ))
            };
        }
        if self.store.keys()?.len() >= MAX_INTENTS {
            return Err(Error::InvalidConfig(
                "saved chat intent capacity reached; recover existing messages first",
            ));
        }
        let bytes = Zeroizing::new(serde_json::to_vec(&Record {
            profile: profile.to_owned(),
            binding: binding.to_vec(),
            intent: intent.clone(),
        })?);
        self.store.put(&key, &bytes)?;
        Ok(())
    }

    pub fn clear(&mut self, profile: &str, binding: &[u8], submission: &str) -> Result<()> {
        let key = record_key(profile, binding)?;
        if let Some(existing) = self.load(profile, binding)? {
            if existing.submission != submission {
                return Err(Error::InvalidConfig(
                    "saved chat intent changed before cleanup",
                ));
            }
            self.store.remove(&key)?;
        }
        Ok(())
    }

    pub fn forget_profile(&mut self, profile: &str) -> Result<()> {
        self.acquire_lock()?;
        crate::validate_name(profile)?;
        let prefix = profile_prefix(profile);
        for key in self.store.keys()? {
            if key.starts_with(&prefix) {
                self.store.remove(&key)?;
            }
        }
        Ok(())
    }
}

pub struct LegacyChatIntent {
    pub profile: String,
    pub binding: Vec<u8>,
    pub intent: LocalChatIntent,
    pub source: String,
}
fn legacy_inventory(directory: &Path) -> Result<Vec<String>> {
    let mut count = 0;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        count += 1;
        if count > 16 {
            return Err(Error::InvalidConfig(
                "legacy saved message directory is invalid",
            ));
        }
        let name = entry.file_name();
        if name == "records" {
            continue;
        }
        let meta = crate::pending_chat::private_path(&entry.path(), false)?;
        if meta.len() > 64 * 1024
            || !matches!(
                name.to_str(),
                Some(
                    "client-state.toml"
                        | "retired-client-state.toml"
                        | ".intent.lock"
                        | ".native-manifest.lock"
                )
            )
        {
            return Err(Error::InvalidConfig(
                "legacy saved message directory is invalid",
            ));
        }
    }
    let retired = directory.join("retired-client-state.toml").try_exists()?;
    let configured = crate::checkpoint::inspect_state_file(directory)?.is_some();
    if retired && configured {
        return Err(Error::InvalidConfig(
            "legacy saved message retirement is inconsistent",
        ));
    }
    let records = directory.join("records");
    let mut keys = Vec::new();
    match std::fs::symlink_metadata(&records) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !retired => return Ok(keys),
        Err(e) => return Err(e.into()),
        Ok(_) => {
            crate::pending_chat::private_path(&records, true)?;
        }
    }
    crate::pending_chat::remove_uncommitted(&records)?;
    for entry in std::fs::read_dir(records)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::InvalidConfig("legacy saved message filename is invalid"))?;
        let valid = name.strip_suffix(".fks").is_some_and(|k| {
            k.len() == 97
                && k.as_bytes()[32] == b'-'
                && k.bytes()
                    .enumerate()
                    .all(|(i, b)| i == 32 || b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
        if !valid
            || keys.len() >= MAX_INTENTS
            || crate::pending_chat::private_path(&entry.path(), false)?.len() > 8 * 1024 * 1024
        {
            return Err(Error::InvalidConfig(
                "legacy saved message inventory is invalid",
            ));
        }
        keys.push(name);
    }
    if (!configured || retired) && !keys.is_empty() {
        return Err(Error::InvalidConfig(
            "legacy saved message key configuration is missing",
        ));
    }
    Ok(keys)
}

pub(crate) fn validate_intent(intent: &LocalChatIntent) -> Result<()> {
    if intent.submission.len() != 32
        || !intent
            .submission
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || intent.submission.bytes().all(|b| b == b'0')
        || intent.text.is_empty()
        || intent.text.len() > foks_proto::RT_MAX_BODY_BYTES
    {
        return Err(Error::InvalidConfig("invalid saved chat intent"));
    }
    Ok(())
}

fn profile_prefix(profile: &str) -> String {
    format!(
        "{}-",
        &crate::hex(&Sha256::digest(profile.as_bytes()))[..32]
    )
}

fn record_key(profile: &str, binding: &[u8]) -> Result<String> {
    crate::validate_name(profile)?;
    if binding.is_empty() || binding.len() > 4096 {
        return Err(Error::InvalidConfig("invalid saved chat intent binding"));
    }
    Ok(format!(
        "{}{}",
        profile_prefix(profile),
        crate::hex(&Sha256::digest(binding))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(path: &Path) -> LocalChatIntentStore {
        LocalChatIntentStore::open_with_backend(path, CredentialBackend::PrivateFile).unwrap()
    }
    fn intent() -> LocalChatIntent {
        LocalChatIntent {
            submission: "12".repeat(16),
            text: "unsent private text".into(),
        }
    }

    #[test]
    fn saved_intent_is_encrypted_durable_immutable_and_scope_bound() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        store
            .save("profile", b"host/actor/team/channel", &intent())
            .unwrap();
        store
            .save("profile", b"host/actor/team/channel", &intent())
            .unwrap();
        let key = record_key("profile", b"host/actor/team/channel").unwrap();
        let ciphertext =
            std::fs::read(root.path().join("records").join(format!("{key}.fks"))).unwrap();
        assert!(!ciphertext
            .windows(intent().text.len())
            .any(|bytes| bytes == intent().text.as_bytes()));
        drop(store);
        let mut store = open(root.path());
        let restored = store
            .load("profile", b"host/actor/team/channel")
            .unwrap()
            .unwrap();
        assert_eq!(restored.text, intent().text);
        assert_eq!(restored.submission, intent().submission);
        assert!(store
            .load("other", b"host/actor/team/channel")
            .unwrap()
            .is_none());
        assert!(store
            .load("profile", b"host/replacement/team/channel")
            .unwrap()
            .is_none());
        let mut changed = intent();
        changed.text = "different text".into();
        assert!(store
            .save("profile", b"host/actor/team/channel", &changed)
            .is_err());
        assert!(store
            .clear("profile", b"host/actor/team/channel", &"34".repeat(16))
            .is_err());
        store
            .clear("profile", b"host/actor/team/channel", &intent().submission)
            .unwrap();
        store
            .clear("profile", b"host/actor/team/channel", &intent().submission)
            .unwrap();
        assert!(store
            .load("profile", b"host/actor/team/channel")
            .unwrap()
            .is_none());
    }

    #[test]
    fn capacity_validation_and_profile_cleanup_preserve_other_intents() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for i in 0..MAX_INTENTS {
            store
                .save(
                    if i == 0 { "other" } else { "profile" },
                    &i.to_le_bytes(),
                    &intent(),
                )
                .unwrap();
        }
        assert!(store.save("profile", b"extra", &intent()).is_err());
        store.forget_profile("profile").unwrap();
        assert!(store
            .load("other", &0usize.to_le_bytes())
            .unwrap()
            .is_some());
        store.save("profile", b"extra", &intent()).unwrap();
        let mut bad = intent();
        bad.submission = "no".into();
        assert!(store.save("profile", b"bad", &bad).is_err());
        bad.submission = "0".repeat(32);
        assert!(store.save("profile", b"zero", &bad).is_err());
        bad = intent();
        bad.text = "x".repeat(foks_proto::RT_MAX_BODY_BYTES + 1);
        assert!(store.save("profile", b"big", &bad).is_err());
    }

    #[test]
    fn corrupted_ciphertext_is_not_replaced_by_a_new_intent() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        store.save("profile", b"binding", &intent()).unwrap();
        let path = root.path().join("records").join(format!(
            "{}.fks",
            record_key("profile", b"binding").unwrap()
        ));
        let mut bytes = std::fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        std::fs::write(&path, bytes).unwrap();
        assert!(store.load("profile", b"binding").is_err());
        assert!(store.save("profile", b"binding", &intent()).is_err());
    }
}
