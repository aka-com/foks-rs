use std::collections::BTreeMap;

use super::{HostKeyProvider, KeyGenerationId, KeyPurpose};
use crate::{Error, Result};

pub const MANIFEST_PURPOSES: [KeyPurpose; 7] = [
    KeyPurpose::Host,
    KeyPurpose::Metadata,
    KeyPurpose::Merkle,
    KeyPurpose::ClientCa,
    KeyPurpose::DelegatedTls,
    KeyPurpose::Recovery,
    KeyPurpose::Capability,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyGenerationManifest {
    generations: BTreeMap<KeyPurpose, KeyGenerationId>,
}

impl KeyGenerationManifest {
    pub fn load_or_create(provider: &dyn HostKeyProvider) -> Result<Self> {
        let mut generations = BTreeMap::new();
        for purpose in MANIFEST_PURPOSES {
            generations.insert(purpose, provider.load_or_create(purpose)?.generation());
        }
        Ok(Self { generations })
    }

    pub fn generation(&self, purpose: KeyPurpose) -> Option<KeyGenerationId> {
        self.generations.get(&purpose).copied()
    }

    /// Validates the immutable bootstrap manifest without recreating a
    /// retired genesis host key. All other bootstrap-purpose keys remain
    /// required because their public-key rotations are not implemented.
    pub(crate) fn validate_existing(
        &self,
        provider: &dyn HostKeyProvider,
        require_genesis_host: bool,
        require_genesis_capability: bool,
        allow_fenced_recovery: bool,
    ) -> Result<()> {
        for purpose in MANIFEST_PURPOSES {
            if purpose == KeyPurpose::Recovery && allow_fenced_recovery {
                continue;
            }
            if purpose == KeyPurpose::Host && !require_genesis_host {
                continue;
            }
            if purpose == KeyPurpose::Capability && !require_genesis_capability {
                continue;
            }
            if provider.load_existing(purpose)?.generation()
                != self
                    .generations
                    .get(&purpose)
                    .copied()
                    .ok_or(Error::Key("incomplete stored key generation manifest"))?
            {
                return Err(Error::Key("stored key generation mismatch"));
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = b"foks-key-manifest-v1\n".to_vec();
        for purpose in MANIFEST_PURPOSES {
            let generation = self.generations[&purpose].as_bytes();
            encoded.extend_from_slice(purpose.label().as_bytes());
            encoded.push(b' ');
            for byte in generation {
                encoded.extend_from_slice(format!("{byte:02x}").as_bytes());
            }
            encoded.push(b'\n');
        }
        encoded
    }

    pub fn decode(encoded: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(encoded).map_err(|_| Error::Key("invalid key manifest"))?;
        let mut lines = text.lines();
        if lines.next() != Some("foks-key-manifest-v1") {
            return Err(Error::Key("invalid key manifest"));
        }
        let mut generations = BTreeMap::new();
        for purpose in MANIFEST_PURPOSES {
            let line = lines.next().ok_or(Error::Key("incomplete key manifest"))?;
            let (label, hex) = line
                .split_once(' ')
                .ok_or(Error::Key("invalid key manifest entry"))?;
            if label != purpose.label() || hex.len() != 32 {
                return Err(Error::Key("invalid key manifest entry"));
            }
            let mut generation = [0; 16];
            for (index, output) in generation.iter_mut().enumerate() {
                *output = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                    .map_err(|_| Error::Key("invalid key manifest generation"))?;
            }
            if generations
                .insert(purpose, KeyGenerationId::from_bytes(generation))
                .is_some()
            {
                return Err(Error::Key("duplicate key manifest entry"));
            }
        }
        if lines.next().is_some() {
            return Err(Error::Key("unexpected key manifest entry"));
        }
        Ok(Self { generations })
    }
}
