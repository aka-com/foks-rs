//! Compiled native qualification profile. No caller-supplied expected hash or DDL.
mod objects;
#[cfg(test)]
mod tests;

use crate::net::memory::Request;
use foks_metadata_core::{Error, Result, Statement};
use foks_schema_contract::{Contract, ContractObject, SchemaObject};
use sha2::{Digest, Sha256};
use std::{io, mem::size_of, sync::Arc};

pub(super) const CONNECTION_POLICY: &[u8] = include_bytes!("profile/connection-policy.json");
const ENTRY_POLICY: &[u8] = include_bytes!("profile/entry-contract.json");
const CERTIFICATE_POLICY: &[u8] = include_bytes!("profile/certificate-policy.json");
pub(super) const MAX_ARGUMENT: usize = 16_384;
pub(super) const MAX_SQL: usize = 2048;

pub(super) struct Profile {
    pub(super) contract: Contract<'static>,
    pub(super) record: [u8; 400],
    pub(super) digest: [u8; 32],
    pub(super) catalog_digest: [u8; 32],
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn composite(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(domain);
    for part in parts {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part);
    }
    h.finalize().into()
}
struct HashWriter(Sha256);
impl io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn scalar_hash() -> Result<[u8; 32]> {
    let specs = Statement::ALL.map(Statement::spec);
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, specs.as_slice()).map_err(|_| Error::Storage)?;
    Ok(writer.0.finalize().into())
}
fn component(objects: &'static [SchemaObject<'static>]) -> Result<Contract<'static>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(objects.len())
        .map_err(|_| Error::Bounds)?;
    values.extend(objects.iter().copied().map(ContractObject::borrowed));
    Contract::new(values).map_err(|_| Error::Storage)
}
impl Profile {
    pub(super) fn new(owner: &Arc<Request>) -> Result<Self> {
        if rusqlite::version() != "3.53.2" {
            return Err(Error::Storage);
        }
        let count = objects::FOKS
            .len()
            .checked_add(objects::AUTHORITY.len())
            .ok_or(Error::Bounds)?;
        // Component vectors and the retained union can coexist. SQL strings are
        // static compiled source; sorting is unstable/in-place and allocates no heap.
        let bytes = count
            .checked_mul(2)
            .and_then(|n| n.checked_mul(size_of::<ContractObject<'static>>()))
            .ok_or(Error::Bounds)?;
        if owner.failed() || !foks_snowpack::AllocationBudget::reserve(owner.as_ref(), bytes) {
            return Err(Error::Bounds);
        }
        let foks = component(objects::FOKS)?;
        let authority = component(objects::AUTHORITY)?;
        let mut union = Vec::new();
        union.try_reserve_exact(count).map_err(|_| Error::Bounds)?;
        union.extend(
            objects::FOKS
                .iter()
                .chain(objects::AUTHORITY)
                .copied()
                .map(ContractObject::borrowed),
        );
        union.sort_unstable_by(|a, b| {
            let a = a.as_object();
            let b = b.as_object();
            (a.kind, a.name, a.table).cmp(&(b.kind, b.name, b.table))
        });
        let contract = Contract::new(union).map_err(|_| Error::Storage)?;
        let scalar = scalar_hash()?;
        // The reviewed page ABI is immutable; its exact artifact is reproduced by
        // the metadata-core generator and checked by the integrated profile tests.
        let page = [
            0xa2, 0xb8, 0x3e, 0xb4, 0x58, 0xc3, 0x8a, 0x39, 0xb3, 0x24, 0x5f, 0xbc, 0xaa, 0x66,
            0x9b, 0xd9, 0xbd, 0x85, 0x25, 0x5d, 0xb0, 0xbe, 0x08, 0xc4, 0x12, 0x48, 0x13, 0x24,
            0x8b, 0x4f, 0x95, 0x5b,
        ];
        let storage = hash(include_bytes!(
            "../../../foks-schema-contract/src/storage/generated.rs"
        ));
        let entry = composite(
            b"foks-native-list-entry-contract-v1\0",
            &[
                ENTRY_POLICY,
                include_bytes!("operation.rs"),
                include_bytes!("output.rs"),
                include_bytes!("../../../foks-metadata-core/src/personal/native_list.rs"),
            ],
        );
        let certificate = composite(
            b"foks-native-list-certificate-import-v1\0",
            &[
                CERTIFICATE_POLICY,
                include_bytes!("certificate.rs"),
                include_bytes!("../restricted_storage/import/certificates.rs"),
            ],
        );
        let components = [
            *foks.hash(),
            *authority.hash(),
            *contract.hash(),
            scalar,
            page,
            storage,
            super::super::policy::fingerprint(),
            hash(CONNECTION_POLICY),
            entry,
            certificate,
        ];
        let mut record = [0u8; 400];
        record[..4].copy_from_slice(b"FKNP");
        record[4..6].copy_from_slice(&1u16.to_le_bytes());
        record[6..8].copy_from_slice(&400u16.to_le_bytes());
        for (slot, value) in record[8..32]
            .chunks_exact_mut(4)
            .zip([1u32, 1, 59, 1, 1, 1])
        {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        for (slot, value) in record[32..352].chunks_exact_mut(32).zip(components) {
            slot.copy_from_slice(&value);
        }
        for (slot, value) in record[352..]
            .chunks_exact_mut(8)
            .zip([16_384u64, 1_048_576, 65_536, 4096, 4096, 2048])
        {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        let mut digest = Sha256::new();
        digest.update(b"foks-native-personal-list-profile-v1\0");
        digest.update(record);
        let mut catalog = Sha256::new();
        catalog.update(b"foks-native-personal-list-catalog-v1\0");
        catalog.update(scalar);
        catalog.update(page);
        Ok(Self {
            contract,
            record,
            digest: digest.finalize().into(),
            catalog_digest: catalog.finalize().into(),
        })
    }
}
