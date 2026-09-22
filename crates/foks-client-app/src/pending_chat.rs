//! Agent-owned pending submissions. Callers hold a state-use lease and, for
//! profile access, a checked session before taking the root-wide ledger lock.
//! One encrypted envelope atomically publishes an import and its completion marker.
use crate::{Error, LocalChatIntent, Result};
use foks_keystore::{EncryptedFileSecretStore, SecretStore};
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs::File, path::Path};
use zeroize::Zeroizing;

const KEY_DOMAIN: u64 = 0x6043_baf7_198d_c225;
pub(crate) const DIRECTORY: &str = "chat-intents";
pub(crate) const LOCK: &str = ".chat-intents.lock";
const MAX_PENDING: usize = 128;
const MAX_TEXT: usize = 8 * 1024 * 1024;
const MAX_COMPLETION_MARKERS: usize = 4096;
const MAX_ENVELOPE: u64 = 16 * 1024 * 1024 + 128;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingChatBinding {
    pub host: String,
    pub actor: String,
    pub team: String,
    pub channel: String,
}
impl PendingChatBinding {
    fn validate(&self) -> Result<()> {
        let entity = |s: &str| s.len() == 66 && hex(s);
        if !entity(&self.host)
            || !self.host.starts_with("02")
            || !self.actor.starts_with("01")
            || !self.team.starts_with("03")
            || !entity(&self.actor)
            || !entity(&self.team)
            || self.channel.len() != 32
            || !hex(&self.channel)
            || self.channel.bytes().all(|b| b == b'0')
        {
            return Err(invalid());
        }
        Ok(())
    }
}
fn hex(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn invalid() -> Error {
    Error::InvalidConfig("saved chat state is invalid")
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    profile: String,
    binding: PendingChatBinding,
    #[serde(with = "stored_intent")]
    intent: LocalChatIntent,
}
// Base64 bounds JSON expansion even for control characters: all 128 maximum
// desktop messages (8 MiB) plus completion_markers fit the encrypted store's 16 MiB cap.
mod stored_intent {
    use super::*;
    use base64::Engine as _;
    use serde::ser::SerializeStruct as _;
    pub fn serialize<S: serde::Serializer>(
        intent: &LocalChatIntent,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let encoded = Zeroizing::new(
            base64::engine::general_purpose::STANDARD.encode(intent.text.as_bytes()),
        );
        let mut record = serializer.serialize_struct("Intent", 2)?;
        record.serialize_field("submission", &intent.submission)?;
        record.serialize_field("text_base64", encoded.as_str())?;
        record.end()
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<LocalChatIntent, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            submission: String,
            #[serde(deserialize_with = "crate::chat_intent::deserialize_secret_string")]
            text_base64: Zeroizing<String>,
        }
        let encoded = Encoded::deserialize(deserializer)?;
        let text = encoded.text_base64;
        if text.len() > foks_proto::RT_MAX_BODY_BYTES.div_ceil(3) * 4 {
            return Err(serde::de::Error::custom("saved message exceeds size limit"));
        }
        let bytes = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(text.as_bytes())
                .map_err(|_| serde::de::Error::custom("invalid saved message encoding"))?,
        );
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| serde::de::Error::custom("invalid saved message text"))?;
        Ok(LocalChatIntent {
            submission: encoded.submission,
            text: text.to_owned(),
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportCompletionMarker {
    source: String,
    binding: PendingChatBinding,
    commitment: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    pending: Vec<Pending>,
    #[serde(alias = "receipts")]
    completion_markers: Vec<ImportCompletionMarker>,
}
impl Default for Envelope {
    fn default() -> Self {
        Self {
            version: 1,
            pending: Vec::new(),
            completion_markers: Vec::new(),
        }
    }
}
impl Envelope {
    fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.pending.len() > MAX_PENDING
            || self.completion_markers.len() > MAX_COMPLETION_MARKERS
        {
            return Err(invalid());
        }
        let mut bindings = BTreeSet::new();
        let mut bytes = 0usize;
        for p in &self.pending {
            crate::validate_name(&p.profile)?;
            p.binding.validate()?;
            crate::chat_intent::validate_intent(&p.intent)?;
            bytes = bytes.checked_add(p.intent.text.len()).ok_or_else(invalid)?;
            if bytes > MAX_TEXT || !bindings.insert(serde_json::to_vec(&p.binding)?) {
                return Err(invalid());
            }
        }
        let mut sources = BTreeSet::new();
        for r in &self.completion_markers {
            r.binding.validate()?;
            if r.source.len() != 64
                || !hex(&r.source)
                || r.commitment.len() != 64
                || !hex(&r.commitment)
                || !sources.insert(&r.source)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

pub struct PendingChatStore {
    store: EncryptedFileSecretStore,
    envelope: Envelope,
    _lock: File,
    failed_write: bool,
}
pub(crate) fn derive_key(master: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(foks_crypto::prefixed_hash(KEY_DOMAIN, master))
}
/// Validate rather than repair permissions on existing recovery state.
pub(crate) fn private_path(path: &Path, directory: bool) -> Result<std::fs::Metadata> {
    let m = std::fs::symlink_metadata(path)?;
    if m.file_type().is_symlink() || (directory && !m.is_dir()) || (!directory && !m.is_file()) {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if m.uid() != rustix::process::getuid().as_raw()
            || m.mode() & 0o077 != 0
            || (!directory && m.nlink() != 1)
        {
            return Err(invalid());
        }
    }
    Ok(m)
}
fn open_lock(path: &Path) -> Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    private_path(path, false)?;
    Ok(file)
}
pub(crate) fn lock(path: &Path) -> Result<File> {
    let file = open_lock(path)?;
    file.lock_exclusive()?;
    Ok(file)
}
pub(crate) fn try_lock(path: &Path) -> Result<File> {
    let file = open_lock(path)?;
    file.try_lock_exclusive()?;
    Ok(file)
}
/// Called only under the writer lock or the exclusive maintenance lease.
/// These are encrypted files that were never atomically published or acknowledged.
pub(crate) fn remove_uncommitted(path: &Path) -> Result<()> {
    let mut count = 0;
    let mut removed = false;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        count += 1;
        if count > 256 {
            return Err(invalid());
        }
        if entry.file_name().to_str().is_some_and(|name| {
            name.strip_prefix(".tmp-")
                .is_some_and(|s| s.len() == 32 && hex(s))
        }) {
            if private_path(&entry.path(), false)?.len() > MAX_ENVELOPE {
                return Err(invalid());
            }
            std::fs::remove_file(entry.path())?;
            removed = true;
        }
    }
    if removed {
        File::open(path)?.sync_all()?;
    }
    Ok(())
}
fn inspect_directory(path: &Path) -> Result<()> {
    private_path(path, true)?;
    remove_uncommitted(path)?;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_name() != "state.fks"
            || private_path(&entry.path(), false)?.len() > MAX_ENVELOPE
        {
            return Err(invalid());
        }
    }
    Ok(())
}
fn read(store: &mut EncryptedFileSecretStore) -> Result<Envelope> {
    let envelope: Envelope = match store.get("state") {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid())?,
        Err(foks_keystore::Error::Missing) => Envelope::default(),
        Err(e) => return Err(e.into()),
    };
    envelope.validate()?;
    Ok(envelope)
}
impl PendingChatStore {
    pub fn open(root: &Path, master: &[u8; 32]) -> Result<Self> {
        private_path(root, true)?;
        let lock = lock(&root.join(LOCK))?;
        let directory = root.join(DIRECTORY);
        match std::fs::symlink_metadata(&directory) {
            Ok(_) => inspect_directory(&directory)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                crate::prepare_private_directory(&directory)?;
                File::open(root)?.sync_all()?;
            }
            Err(e) => return Err(e.into()),
        }
        let mut store = EncryptedFileSecretStore::inspect_existing(directory, derive_key(master))?;
        let envelope = read(&mut store)?;
        Ok(Self {
            store,
            envelope,
            _lock: lock,
            failed_write: false,
        })
    }
    pub fn load(&self, binding: &PendingChatBinding) -> Result<Option<LocalChatIntent>> {
        self.usable()?;
        binding.validate()?;
        Ok(self
            .envelope
            .pending
            .iter()
            .find(|p| &p.binding == binding)
            .map(|p| p.intent.clone()))
    }
    pub fn save(
        &mut self,
        profile: &str,
        binding: &PendingChatBinding,
        intent: &LocalChatIntent,
    ) -> Result<()> {
        self.insert(profile, binding, intent)?;
        self.persist()
    }
    fn insert(
        &mut self,
        profile: &str,
        binding: &PendingChatBinding,
        intent: &LocalChatIntent,
    ) -> Result<()> {
        crate::validate_name(profile)?;
        binding.validate()?;
        crate::chat_intent::validate_intent(intent)?;
        if let Some(existing) = self.load(binding)? {
            return if existing == *intent {
                Ok(())
            } else {
                Err(Error::InvalidConfig(
                    "another message is already saved for this channel; recover it first",
                ))
            };
        }
        if self.envelope.pending.len() >= MAX_PENDING
            || self
                .envelope
                .pending
                .iter()
                .map(|p| p.intent.text.len())
                .sum::<usize>()
                + intent.text.len()
                > MAX_TEXT
        {
            return Err(Error::InvalidConfig(
                "saved message capacity reached; recover existing messages first",
            ));
        }
        self.envelope.pending.push(Pending {
            profile: profile.into(),
            binding: binding.clone(),
            intent: intent.clone(),
        });
        Ok(())
    }
    pub fn clear(&mut self, binding: &PendingChatBinding, submission: &str) -> Result<()> {
        if let Some(existing) = self.load(binding)? {
            if existing.submission != submission {
                return Err(Error::InvalidConfig("saved message changed before cleanup"));
            }
            self.envelope.pending.retain(|p| &p.binding != binding);
        }
        // A retry also completes durability after a prior lost/failed sync.
        self.persist()
    }
    pub fn import(
        &mut self,
        profile: &str,
        binding: &PendingChatBinding,
        intent: &LocalChatIntent,
        source: &str,
    ) -> Result<()> {
        self.usable()?;
        binding.validate()?;
        crate::chat_intent::validate_intent(intent)?;
        if source.len() != 64 || !hex(source) {
            return Err(invalid());
        }
        use sha2::{Digest as _, Sha256};
        let input = Zeroizing::new(serde_json::to_vec(&(source, binding, intent))?);
        let commitment = crate::hex(&Sha256::digest(&input));
        if let Some(completion_marker) = self
            .envelope
            .completion_markers
            .iter()
            .find(|marker| marker.source == source)
        {
            if &completion_marker.binding != binding || completion_marker.commitment != commitment {
                return Err(invalid());
            }
            // A prior rename may have succeeded before directory sync failed.
            // Republish durably before permitting deletion of the only source.
            return self.persist();
        }
        if self.envelope.completion_markers.len() >= MAX_COMPLETION_MARKERS {
            return Err(Error::InvalidConfig(
                "saved message migration completion marker capacity reached; legacy messages remain intact",
            ));
        }
        self.insert(profile, binding, intent)?;
        self.envelope
            .completion_markers
            .push(ImportCompletionMarker {
                source: source.into(),
                binding: binding.clone(),
                commitment,
            });
        // The completion marker and pending body become visible together. Clearing the body
        // never clears the marker, even after profile removal or archive import.
        self.persist()
    }
    pub(crate) fn profile_summary(
        &self,
        profile: &str,
        master: &[u8; 32],
    ) -> Result<(usize, usize, [u8; 32])> {
        self.usable()?;
        let pending = self
            .envelope
            .pending
            .iter()
            .filter(|p| p.profile == profile)
            .collect::<Vec<_>>();
        let bytes = Zeroizing::new(serde_json::to_vec(&pending)?);
        Ok((
            pending.len(),
            pending.iter().map(|p| p.intent.text.len()).sum(),
            foks_crypto::capability_mac(master, 0x42cf_f5bd_a9b1_680c, &bytes),
        ))
    }
    pub(crate) fn forget_profile(&mut self, profile: &str) -> Result<()> {
        self.usable()?;
        self.envelope.pending.retain(|p| p.profile != profile);
        self.persist()
    }
    fn usable(&self) -> Result<()> {
        if self.failed_write {
            return Err(Error::InvalidConfig(
                "saved message write failed; reopen storage before retrying",
            ));
        }
        Ok(())
    }
    fn persist(&mut self) -> Result<()> {
        self.envelope.validate()?;
        let bytes = Zeroizing::new(serde_json::to_vec(&self.envelope)?);
        if let Err(error) = self.store.put("state", &bytes) {
            self.failed_write = true;
            return Err(error.into());
        }
        Ok(())
    }
}
/// Maintenance already owns the exclusive state lease; never acquire a second
/// credential/state lease here. Completion markers remain portable after bodies are cleared.
pub(crate) fn inspect(root: &Path, master: &[u8; 32]) -> Result<Vec<String>> {
    let directory = root.join(DIRECTORY);
    inspect_directory(&directory)?;
    let mut store = EncryptedFileSecretStore::inspect_existing(directory, derive_key(master))?;
    let envelope = read(&mut store)?;
    Ok(envelope.pending.into_iter().map(|p| p.profile).collect())
}
pub(crate) fn rekey(source: &Path, target: &Path, old: &[u8; 32], new: &[u8; 32]) -> Result<()> {
    inspect_directory(source)?;
    let mut old_store = EncryptedFileSecretStore::inspect_existing(source, derive_key(old))?;
    let envelope = read(&mut old_store)?;
    if !envelope.pending.is_empty() {
        return Err(Error::InvalidConfig(
            "pending chat messages block archive import",
        ));
    }
    let mut new_store = EncryptedFileSecretStore::open(target, derive_key(new))?;
    if source.join("state.fks").try_exists()? {
        new_store.put("state", &Zeroizing::new(serde_json::to_vec(&envelope)?))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> PendingChatBinding {
        PendingChatBinding {
            host: format!("02{}", "ab".repeat(32)),
            actor: format!("01{}", "cd".repeat(32)),
            team: format!("03{}", "ef".repeat(32)),
            channel: "12".repeat(16),
        }
    }
    fn intent() -> LocalChatIntent {
        LocalChatIntent {
            submission: "34".repeat(16),
            text: "saved private message".into(),
        }
    }
    #[test]
    fn main_key_reopen_alias_independence_and_conflicting_saves() {
        let temporary = tempfile::tempdir().unwrap();
        let root = crate::prepare_private_directory(&temporary.path().join("state")).unwrap();
        let key = [42; 32];
        let mut store = PendingChatStore::open(&root, &key).unwrap();
        store.save("profile", &binding(), &intent()).unwrap();
        store.save("renamed-route", &binding(), &intent()).unwrap();
        let mut other = intent();
        other.text = "replacement".into();
        assert!(store.save("profile", &binding(), &other).is_err());
        assert!(store.clear(&binding(), &"56".repeat(16)).is_err());
        drop(store);
        assert!(!root.join("client-state.toml").exists());
        assert!(PendingChatStore::open(&root, &[43; 32]).is_err());
        let store = PendingChatStore::open(&root, &key).unwrap();
        assert!(store
            .load(&binding())
            .unwrap()
            .is_some_and(|i| i == intent()));
        let bytes = std::fs::read(root.join(DIRECTORY).join("state.fks")).unwrap();
        assert!(!bytes
            .windows(intent().text.len())
            .any(|w| w == intent().text.as_bytes()));
    }
    #[test]
    fn migration_completion_marker_survives_consumption_profile_removal_and_rekey() {
        let temporary = tempfile::tempdir().unwrap();
        let root = crate::prepare_private_directory(&temporary.path().join("state")).unwrap();
        let key = [42; 32];
        let new_key = [43; 32];
        let source = "ab".repeat(32);
        let mut store = PendingChatStore::open(&root, &key).unwrap();
        store
            .import("profile", &binding(), &intent(), &source)
            .unwrap();
        store.clear(&binding(), &intent().submission).unwrap();
        store.forget_profile("profile").unwrap();
        drop(store);
        let target_temporary = tempfile::tempdir().unwrap();
        let target = target_temporary.path().join("state");
        rekey(
            &root.join(DIRECTORY),
            &target.join(DIRECTORY),
            &key,
            &new_key,
        )
        .unwrap();
        let mut store = PendingChatStore::open(&target, &new_key).unwrap();
        store
            .import("new-route", &binding(), &intent(), &source)
            .unwrap();
        assert!(store.load(&binding()).unwrap().is_none());
        let mut changed = intent();
        changed.text = "different".into();
        assert!(store
            .import("new-route", &binding(), &changed, &source)
            .is_err());
        let mut moved = binding();
        moved.channel = "78".repeat(16);
        assert!(store
            .import("new-route", &moved, &intent(), &source)
            .is_err());
    }
    #[test]
    fn every_legacy_desktop_slot_fits_even_with_control_characters() {
        let temporary = tempfile::tempdir().unwrap();
        let root = crate::prepare_private_directory(&temporary.path().join("state")).unwrap();
        let key = [42; 32];
        let mut store = PendingChatStore::open(&root, &key).unwrap();
        // Construct one maximum envelope, avoiding repeated publication of it.
        for n in 1..=MAX_PENDING {
            let mut bound = binding();
            bound.channel = format!("{n:032x}");
            let message = LocalChatIntent {
                submission: format!("{n:032x}"),
                text: "\0".repeat(65536),
            };
            store.insert("profile", &bound, &message).unwrap();
        }
        store.persist().unwrap();
        assert!(store.insert("profile", &binding(), &intent()).is_err());
        assert!(
            std::fs::metadata(root.join(DIRECTORY).join("state.fks"))
                .unwrap()
                .len()
                < MAX_ENVELOPE
        );
        drop(store);
        let store = PendingChatStore::open(&root, &key).unwrap();
        assert_eq!(store.envelope.pending.len(), MAX_PENDING);
    }
}
