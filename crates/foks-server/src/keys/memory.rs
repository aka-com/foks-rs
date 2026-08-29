use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::sync::Mutex;

use super::{HostKeyProvider, KeyGenerationId, KeyPurpose, SecretKey};

#[derive(Clone, Copy)]
struct StoredMemoryKey {
    bytes: [u8; 32],
    generation: [u8; 16],
}

#[derive(Default)]
pub struct MemoryKeyProvider {
    keys: Mutex<BTreeMap<KeyPurpose, StoredMemoryKey>>,
    generations: Mutex<BTreeMap<(KeyPurpose, KeyGenerationId), StoredMemoryKey>>,
}

impl HostKeyProvider for MemoryKeyProvider {
    fn is_pristine(&self) -> crate::Result<bool> {
        if !self
            .keys
            .lock()
            .map_err(|_| crate::Error::Key("lock poisoned"))?
            .is_empty()
        {
            return Ok(false);
        }
        Ok(self
            .generations
            .lock()
            .map_err(|_| crate::Error::Key("lock poisoned"))?
            .is_empty())
    }

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

    fn load_existing(&self, purpose: KeyPurpose) -> crate::Result<SecretKey> {
        let keys = self
            .keys
            .lock()
            .map_err(|_| crate::Error::Key("lock poisoned"))?;
        let key = keys
            .get(&purpose)
            .ok_or(crate::Error::Key("canonical key does not exist"))?;
        Ok(SecretKey::new(key.bytes, key.generation))
    }

    fn create_generation(&self, purpose: KeyPurpose) -> crate::Result<SecretKey> {
        let mut generations = self
            .generations
            .lock()
            .map_err(|_| crate::Error::Key("lock poisoned"))?;
        for _ in 0..8 {
            let mut key = [0; 32];
            getrandom::fill(&mut key).map_err(|_| crate::Error::Key("operating-system entropy"))?;
            let mut generation = [0; 16];
            getrandom::fill(&mut generation)
                .map_err(|_| crate::Error::Key("operating-system entropy"))?;
            let id = KeyGenerationId::from_bytes(generation);
            if let Entry::Vacant(entry) = generations.entry((purpose, id)) {
                entry.insert(StoredMemoryKey {
                    bytes: key,
                    generation,
                });
                return Ok(SecretKey::new(key, generation));
            }
        }
        Err(crate::Error::Key("repeated key generation collision"))
    }

    fn load_generation(
        &self,
        purpose: KeyPurpose,
        generation: KeyGenerationId,
    ) -> crate::Result<SecretKey> {
        if let Some(key) = self
            .keys
            .lock()
            .map_err(|_| crate::Error::Key("lock poisoned"))?
            .get(&purpose)
            .copied()
            .filter(|key| key.generation == generation.as_bytes())
        {
            return Ok(SecretKey::new(key.bytes, key.generation));
        }
        let generations = self
            .generations
            .lock()
            .map_err(|_| crate::Error::Key("lock poisoned"))?;
        let key = generations
            .get(&(purpose, generation))
            .ok_or(crate::Error::Key("key generation does not exist"))?;
        Ok(SecretKey::new(key.bytes, key.generation))
    }

    fn list_generations(&self, purpose: KeyPurpose) -> crate::Result<Vec<KeyGenerationId>> {
        Ok(self
            .generations
            .lock()
            .map_err(|_| crate::Error::Key("lock poisoned"))?
            .keys()
            .filter_map(|(stored_purpose, generation)| {
                (*stored_purpose == purpose).then_some(*generation)
            })
            .collect())
    }

    fn remove_generation(
        &self,
        purpose: KeyPurpose,
        generation: KeyGenerationId,
    ) -> crate::Result<()> {
        self.generations
            .lock()
            .map_err(|_| crate::Error::Key("lock poisoned"))?
            .remove(&(purpose, generation));
        let mut keys = self
            .keys
            .lock()
            .map_err(|_| crate::Error::Key("lock poisoned"))?;
        if keys
            .get(&purpose)
            .is_some_and(|key| key.generation == generation.as_bytes())
        {
            keys.remove(&purpose);
        }
        Ok(())
    }
}
