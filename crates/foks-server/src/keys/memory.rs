use std::collections::BTreeMap;
use std::sync::Mutex;

use super::{HostKeyProvider, KeyPurpose, SecretKey};

#[derive(Clone, Copy)]
struct StoredMemoryKey {
    bytes: [u8; 32],
    generation: [u8; 16],
}

#[derive(Default)]
pub struct MemoryKeyProvider {
    keys: Mutex<BTreeMap<KeyPurpose, StoredMemoryKey>>,
}

impl HostKeyProvider for MemoryKeyProvider {
    fn load_or_create(&self, purpose: KeyPurpose) -> crate::Result<SecretKey> {
        let mut keys = self
            .keys
            .lock()
            .map_err(|_| crate::Error::Key("lock poisoned"))?;
        if let Some(key) = keys.get(&purpose) {
            return Ok(SecretKey::new(key.bytes, key.generation));
        }
        let mut key = [0; 32];
        getrandom::fill(&mut key).map_err(|_| crate::Error::Key("operating-system entropy"))?;
        let mut generation = [0; 16];
        getrandom::fill(&mut generation)
            .map_err(|_| crate::Error::Key("operating-system entropy"))?;
        keys.insert(
            purpose,
            StoredMemoryKey {
                bytes: key,
                generation,
            },
        );
        Ok(SecretKey::new(key, generation))
    }
}
