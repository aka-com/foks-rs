use crate::{ClientCredentials, CredentialBackend, Error, Result};
use foks_keystore::{EncryptedFileSecretStore, SecretStore};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, path::Path};
use zeroize::{Zeroize, Zeroizing};

const MAX_INTENTS: usize = 128;
const KEY_DOMAIN: u64 = 0xa536_6912_2a5f_909c;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalChatIntent {
    pub submission: String,
    pub text: String,
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
    _lock: File,
}

impl LocalChatIntentStore {
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
            _lock: lock,
        })
    }

    pub fn load(&mut self, profile: &str, binding: &[u8]) -> Result<Option<LocalChatIntent>> {
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

fn validate_intent(intent: &LocalChatIntent) -> Result<()> {
    if intent.submission.len() != 32
        || !intent
            .submission
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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
