use foks_merkle_store::{NodeReader, Result as MerkleResult};
use rusqlite::{params, Connection, OptionalExtension as _};

use crate::{Database, ReadDatabase, ReadSnapshot};

pub struct SqliteNodeReader<'a> {
    connection: &'a Connection,
}

impl Database {
    pub fn node_reader(&self) -> SqliteNodeReader<'_> {
        SqliteNodeReader {
            connection: &self.connection,
        }
    }
}

impl ReadDatabase {
    pub fn node_reader(&self) -> SqliteNodeReader<'_> {
        SqliteNodeReader {
            connection: &self.connection,
        }
    }
}

impl ReadSnapshot<'_> {
    pub fn node_reader(&self) -> SqliteNodeReader<'_> {
        SqliteNodeReader {
            connection: self.connection(),
        }
    }

    pub fn merkle_leaf_epoch(&self, key: &[u8; 32]) -> crate::Result<Option<u64>> {
        self.connection()
            .query_row(
                "SELECT epoch FROM merkle_leaves WHERE leaf_key = ?1",
                params![key],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map(crate::error::unsigned)
            .transpose()
    }
}

impl NodeReader for SqliteNodeReader<'_> {
    fn get_node(&self, hash: &[u8; 32]) -> MerkleResult<Option<Vec<u8>>> {
        self.connection
            .query_row(
                "SELECT exact_node FROM merkle_nodes WHERE node_hash = ?1",
                [hash],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| foks_merkle_store::Error::Storage(error.to_string()))
    }
}
