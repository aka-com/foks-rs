use rusqlite::{params, Connection, OptionalExtension as _, Transaction, TransactionBehavior};

use crate::{error::sql_integer, error::unsigned, Database, Error, Result};

type StoredBootstrap = (Vec<u8>, String, Vec<u8>, Vec<u8>);
type StoredGenesisRoot = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, i64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapService {
    pub service_type: u64,
    pub endpoint: String,
    pub advertised_blob: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostBootstrap {
    pub host_id: Vec<u8>,
    pub canonical_name: String,
    pub probe_response: Vec<u8>,
    pub key_manifest: Vec<u8>,
    pub host_key_generation: [u8; 16],
    pub hostchain_link_hash: [u8; 32],
    pub exact_hostchain_link: Vec<u8>,
    pub services: Vec<BootstrapService>,
    pub root_hash: [u8; 32],
    pub root_node: [u8; 32],
    pub root_epoch: u64,
    pub exact_root: Vec<u8>,
    pub exact_signed_root: Vec<u8>,
    pub created_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredHostBootstrap {
    pub host_id: Vec<u8>,
    pub canonical_name: String,
    pub probe_response: Vec<u8>,
    pub key_manifest: Vec<u8>,
}

impl Database {
    /// Atomically installs the immutable genesis hostchain, advertised service
    /// map, signed empty Merkle root, and key-generation binding.
    pub fn bootstrap_host(&mut self, bootstrap: &HostBootstrap) -> Result<bool> {
        validate(bootstrap, self.config.maximum_blob_bytes)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if stored_metadata_transaction(&transaction)?.is_some() {
            if bootstrap_matches(&transaction, bootstrap)? {
                return Ok(false);
            }
            return Err(Error::Invalid("conflicting host bootstrap"));
        }

        transaction.execute(
            "INSERT INTO host_metadata
             (singleton, host_id, canonical_name, bootstrap_blob, key_manifest_blob)
             VALUES (1, ?1, ?2, ?3, ?4)",
            params![
                bootstrap.host_id,
                bootstrap.canonical_name,
                bootstrap.probe_response,
                bootstrap.key_manifest
            ],
        )?;
        transaction.execute(
            "INSERT INTO host_key_generations
             (generation_id, purpose, encrypted_file_name, public_entity_id, state,
              created_at, activated_hostchain_seqno)
             VALUES (?1, 1, 'host.key', ?2, 2, ?3, 1)",
            params![
                bootstrap.host_key_generation,
                bootstrap.host_id,
                sql_integer(bootstrap.created_at)?
            ],
        )?;
        transaction.execute(
            "INSERT INTO hostchain_links(seqno, link_hash, exact_link) VALUES (1, ?1, ?2)",
            params![
                bootstrap.hostchain_link_hash,
                bootstrap.exact_hostchain_link
            ],
        )?;
        for service in &bootstrap.services {
            transaction.execute(
                "INSERT INTO services(service_type, endpoint, advertised_blob)
                 VALUES (?1, ?2, ?3)",
                params![
                    sql_integer(service.service_type)?,
                    service.endpoint,
                    service.advertised_blob
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO merkle_roots
             (epoch, root_hash, root_node, exact_root, exact_signed_root, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                sql_integer(bootstrap.root_epoch)?,
                bootstrap.root_hash,
                bootstrap.root_node,
                bootstrap.exact_root,
                bootstrap.exact_signed_root,
                sql_integer(bootstrap.created_at)?
            ],
        )?;
        transaction.execute(
            "INSERT INTO merkle_root_heads(singleton, epoch, root_hash) VALUES (1, ?1, ?2)",
            params![sql_integer(bootstrap.root_epoch)?, bootstrap.root_hash],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn host_bootstrap(&self) -> Result<Option<StoredHostBootstrap>> {
        stored_metadata_connection(&self.connection).map(|stored| {
            stored.map(|(host_id, canonical_name, probe_response, key_manifest)| {
                StoredHostBootstrap {
                    host_id,
                    canonical_name,
                    probe_response,
                    key_manifest,
                }
            })
        })
    }
}

impl crate::ReadDatabase {
    pub fn host_bootstrap(&self) -> Result<Option<StoredHostBootstrap>> {
        stored_metadata_connection(&self.connection).map(|stored| {
            stored.map(|(host_id, canonical_name, probe_response, key_manifest)| {
                StoredHostBootstrap {
                    host_id,
                    canonical_name,
                    probe_response,
                    key_manifest,
                }
            })
        })
    }
}

fn validate(bootstrap: &HostBootstrap, maximum_blob_bytes: usize) -> Result<()> {
    let mut service_types = std::collections::BTreeSet::new();
    if bootstrap.host_id.len() != 33
        || bootstrap.canonical_name.is_empty()
        || bootstrap.canonical_name.len() > 255
        || bootstrap.root_node != [0; 32]
        || bootstrap.root_epoch != 1
        || bootstrap.host_key_generation == [0; 16]
        || bootstrap.exact_hostchain_link.is_empty()
        || bootstrap.services.len() != 6
        || bootstrap.services.iter().any(|service| {
            !matches!(service.service_type, 1 | 2 | 5 | 10 | 12 | 16)
                || service.endpoint.is_empty()
                || !service_types.insert(service.service_type)
        })
        || [
            bootstrap.probe_response.as_slice(),
            bootstrap.key_manifest.as_slice(),
            bootstrap.exact_hostchain_link.as_slice(),
            bootstrap.exact_root.as_slice(),
            bootstrap.exact_signed_root.as_slice(),
        ]
        .into_iter()
        .any(|blob| blob.is_empty() || blob.len() > maximum_blob_bytes)
    {
        return Err(Error::Invalid("host bootstrap"));
    }
    Ok(())
}

fn stored_metadata_connection(connection: &Connection) -> Result<Option<StoredBootstrap>> {
    connection
        .query_row(
            "SELECT host_id, canonical_name, bootstrap_blob, key_manifest_blob
             FROM host_metadata WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(Into::into)
}

fn stored_metadata_transaction(transaction: &Transaction<'_>) -> Result<Option<StoredBootstrap>> {
    transaction
        .query_row(
            "SELECT host_id, canonical_name, bootstrap_blob, key_manifest_blob
             FROM host_metadata WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(Into::into)
}

fn bootstrap_matches(transaction: &Transaction<'_>, bootstrap: &HostBootstrap) -> Result<bool> {
    let Some(metadata) = stored_metadata_transaction(transaction)? else {
        return Ok(false);
    };
    if metadata
        != (
            bootstrap.host_id.clone(),
            bootstrap.canonical_name.clone(),
            bootstrap.probe_response.clone(),
            bootstrap.key_manifest.clone(),
        )
    {
        return Ok(false);
    }
    let link: Option<(Vec<u8>, Vec<u8>)> = transaction
        .query_row(
            "SELECT link_hash, exact_link FROM hostchain_links WHERE seqno = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if link
        != Some((
            bootstrap.hostchain_link_hash.to_vec(),
            bootstrap.exact_hostchain_link.clone(),
        ))
    {
        return Ok(false);
    }
    let root: Option<StoredGenesisRoot> = transaction
        .query_row(
            "SELECT root_hash, root_node, exact_root, exact_signed_root, created_at
             FROM merkle_roots WHERE epoch = ?1",
            [sql_integer(bootstrap.root_epoch)?],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    if root
        != Some((
            bootstrap.root_hash.to_vec(),
            bootstrap.root_node.to_vec(),
            bootstrap.exact_root.clone(),
            bootstrap.exact_signed_root.clone(),
            sql_integer(bootstrap.created_at)?,
        ))
    {
        return Ok(false);
    }
    let stored_services = transaction
        .prepare(
            "SELECT service_type, endpoint, advertised_blob FROM services ORDER BY service_type",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let services = stored_services
        .into_iter()
        .map(|(service_type, endpoint, advertised_blob)| {
            Ok(BootstrapService {
                service_type: unsigned(service_type)?,
                endpoint,
                advertised_blob,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut expected = bootstrap.services.clone();
    expected.sort_by_key(|service| service.service_type);
    let head: Option<(i64, Vec<u8>)> = transaction
        .query_row(
            "SELECT epoch, root_hash FROM merkle_root_heads WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let generation: Option<(Vec<u8>, Vec<u8>, i64, i64)> = transaction
        .query_row(
            "SELECT generation_id, public_entity_id, state, activated_hostchain_seqno
             FROM host_key_generations WHERE purpose = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    Ok(services == expected
        && head
            == Some((
                sql_integer(bootstrap.root_epoch)?,
                bootstrap.root_hash.to_vec(),
            ))
        && generation
            == Some((
                bootstrap.host_key_generation.to_vec(),
                bootstrap.host_id.clone(),
                2,
                1,
            )))
}
