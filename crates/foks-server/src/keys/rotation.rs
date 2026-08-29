use std::collections::BTreeSet;

use foks_server_db::{CapabilityKeyGenerationState, Database, ReadDatabase};

use super::{HostKeyProvider, KeyGenerationId, KeyPurpose};
use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityKeyRotationState {
    pub active_generation_id: [u8; 16],
    pub retiring_generation_ids: Vec<[u8; 16]>,
}

/// Creates and atomically selects a new symmetric capability generation.
/// The old key remains readable until every capability bounded to it expires.
pub fn rotate_capability_key(
    database: &mut Database,
    provider: &dyn HostKeyProvider,
    now: u64,
) -> Result<CapabilityKeyRotationState> {
    cleanup_untracked_generations(database, provider)?;
    validate_capability_key_generations(database, provider)?;
    let key = provider.create_generation(KeyPurpose::Capability)?;
    let generation = key.generation();
    let file = generation_file_name(generation.as_bytes());
    if let Err(error) = database.rotate_capability_key(generation.as_bytes(), &file, now) {
        provider.remove_generation(KeyPurpose::Capability, generation)?;
        return Err(error.into());
    }
    validate_capability_key_generations(database, provider)?;
    state(database)
}

/// Revokes no-longer-referenced generations in SQLite before idempotently
/// erasing their encrypted files. Run with the server stopped and an
/// exclusive key-provider rotation lock.
pub fn retire_capability_keys(
    database: &mut Database,
    provider: &dyn HostKeyProvider,
    now: u64,
) -> Result<CapabilityKeyRotationState> {
    cleanup_untracked_generations(database, provider)?;
    database.revoke_retired_capability_keys(now)?;
    for generation in database.capability_key_generations()? {
        if generation.state == CapabilityKeyGenerationState::Revoked {
            provider.remove_generation(
                KeyPurpose::Capability,
                KeyGenerationId::from_bytes(generation.generation_id),
            )?;
        }
    }
    validate_capability_key_generations(database, provider)?;
    state(database)
}

/// A crash after publishing an immutable file but before selecting it in
/// SQLite leaves no durable reference. The ledger is authoritative, so an
/// exclusive retry may erase only those generation-qualified orphans.
fn cleanup_untracked_generations(
    database: &impl CapabilityGenerationReader,
    provider: &dyn HostKeyProvider,
) -> Result<()> {
    let known = database
        .capability_generations()?
        .into_iter()
        .filter(|generation| generation.encrypted_file_name != "capability.key")
        .map(|generation| KeyGenerationId::from_bytes(generation.generation_id))
        .collect::<BTreeSet<_>>();
    for generation in provider.list_generations(KeyPurpose::Capability)? {
        if !known.contains(&generation) {
            provider.remove_generation(KeyPurpose::Capability, generation)?;
        }
    }
    Ok(())
}

pub fn validate_capability_key_generations(
    database: &impl CapabilityGenerationReader,
    provider: &dyn HostKeyProvider,
) -> Result<()> {
    let generations = database.capability_generations()?;
    if generations
        .iter()
        .filter(|generation| generation.state == CapabilityKeyGenerationState::Active)
        .count()
        != 1
        || generations
            .iter()
            .filter(|generation| generation.encrypted_file_name == "capability.key")
            .count()
            != 1
    {
        return Err(Error::Key("invalid capability generation ledger"));
    }
    let listed = provider
        .list_generations(KeyPurpose::Capability)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut known_generated = BTreeSet::new();
    for generation in &generations {
        let id = KeyGenerationId::from_bytes(generation.generation_id);
        let canonical = generation.encrypted_file_name == "capability.key";
        if !canonical {
            if generation.encrypted_file_name != generation_file_name(generation.generation_id) {
                return Err(Error::Key("invalid capability generation filename"));
            }
            known_generated.insert(id);
        }
        if generation.state != CapabilityKeyGenerationState::Revoked {
            let key = super::load_capability_generation(provider, generation.generation_id)?;
            if key.generation() != id || (!canonical && !listed.contains(&id)) {
                return Err(Error::Key("missing capability key generation"));
            }
        }
    }
    if !listed.is_subset(&known_generated) {
        return Err(Error::Key("untracked capability key generation"));
    }
    Ok(())
}

pub trait CapabilityGenerationReader {
    fn capability_generations(&self) -> Result<Vec<foks_server_db::CapabilityKeyGeneration>>;
}

impl CapabilityGenerationReader for Database {
    fn capability_generations(&self) -> Result<Vec<foks_server_db::CapabilityKeyGeneration>> {
        Ok(self.capability_key_generations()?)
    }
}

impl CapabilityGenerationReader for ReadDatabase {
    fn capability_generations(&self) -> Result<Vec<foks_server_db::CapabilityKeyGeneration>> {
        Ok(self.capability_key_generations()?)
    }
}

fn state(database: &Database) -> Result<CapabilityKeyRotationState> {
    let generations = database.capability_key_generations()?;
    let active_generation_id = generations
        .iter()
        .find(|generation| generation.state == CapabilityKeyGenerationState::Active)
        .ok_or(Error::Key("capability key has no active generation"))?
        .generation_id;
    let retiring_generation_ids = generations
        .into_iter()
        .filter(|generation| generation.state == CapabilityKeyGenerationState::Retiring)
        .map(|generation| generation.generation_id)
        .collect();
    Ok(CapabilityKeyRotationState {
        active_generation_id,
        retiring_generation_ids,
    })
}

pub(crate) fn generation_file_name(generation: [u8; 16]) -> String {
    let hex = generation
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("capability.{hex}.key")
}
