use crate::{Error, Result};
use rusqlite::Connection;

pub const READ_DEPENDENCY_SCHEMA: &str = include_str!("read_dependency.sql");
pub const READ_DEPENDENCY_TRIGGERS: &str = include_str!("read_dependency_triggers.sql");
pub const READ_DEPENDENCY_TABLES: &[&str] = &[
    "issued_certificates",
    "devices",
    "shared_keys",
    "sso_policy",
    "users",
    "user_chain_heads",
    "tree_locations",
    "merkle_root_heads",
    "merkle_roots",
    "kv_namespaces",
    "kv_roots",
    "kv_directories",
    "kv_directory_heads",
    "kv_nodes",
    "kv_dirents",
    "kv_dirent_heads",
    "kv_dirent_current",
    "kv_file_uploads",
    "kv_file_chunks",
    "chunk_objects",
    "chunk_storage_binding",
    "host_metadata",
    "host_key_generations",
    "capability_key_generations",
];

pub(crate) fn fresh_incarnation(previous: Option<&[u8; 32]>) -> Result<[u8; 32]> {
    incarnation_from(previous, |bytes| {
        getrandom::fill(bytes).map_err(|_| Error::ReadDependencySchema("OS randomness unavailable"))
    })
}

fn incarnation_from(
    previous: Option<&[u8; 32]>,
    fill: impl FnOnce(&mut [u8; 32]) -> Result<()>,
) -> Result<[u8; 32]> {
    let mut bytes = [0; 32];
    fill(&mut bytes)?;
    if bytes == [0; 32] || previous == Some(&bytes) {
        return Err(Error::ReadDependencySchema(
            "incarnation randomness rejected",
        ));
    }
    Ok(bytes)
}

pub(super) fn migrate(connection: &Connection) -> Result<()> {
    // All historical rebuilds have completed under the same outer write lock.
    // No zero/default coordinate or missing-hook intermediate state is published.
    let incarnation = fresh_incarnation(None)?;
    install(connection, &incarnation)
}

pub(super) fn install(connection: &Connection, incarnation: &[u8; 32]) -> Result<()> {
    connection.execute_batch(READ_DEPENDENCY_SCHEMA)?;
    connection.execute(
        "INSERT INTO metadata_read_dependency VALUES(1,?1,1)",
        [incarnation.as_slice()],
    )?;
    connection.execute_batch(READ_DEPENDENCY_TRIGGERS)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entropy_errors_zero_and_predecessor_reuse_are_rejected_without_fallback() {
        assert!(incarnation_from(None, |_| Err(Error::ReadDependencySchema(
            "injected entropy failure"
        )))
        .is_err());
        assert!(incarnation_from(None, |_| Ok(())).is_err());
        assert!(incarnation_from(Some(&[7; 32]), |out| {
            *out = [7; 32];
            Ok(())
        })
        .is_err());
        assert_eq!(
            incarnation_from(Some(&[7; 32]), |out| {
                *out = [8; 32];
                Ok(())
            })
            .unwrap(),
            [8; 32]
        );
    }
}
