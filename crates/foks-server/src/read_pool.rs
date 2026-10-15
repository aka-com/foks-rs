use std::ops::Deref;
use std::sync::{Arc, Mutex};

use crate::{Error, ReadDatabaseConfig, Result};

#[derive(Clone)]
pub(crate) struct ReadPool {
    inner: Arc<Inner>,
}

struct Inner {
    config: ReadDatabaseConfig,
    maximum_connections: usize,
    state: Mutex<State>,
}

struct State {
    idle: Vec<foks_server_db::ReadDatabase>,
    open_connections: usize,
}

pub(crate) struct ReadLease {
    pool: Arc<Inner>,
    database: Option<foks_server_db::ReadDatabase>,
}

impl ReadPool {
    pub(crate) fn new(config: ReadDatabaseConfig, maximum_connections: usize) -> Result<Self> {
        if maximum_connections == 0 {
            return Err(Error::Config(
                "maximum read connections must be greater than zero",
            ));
        }
        let database =
            foks_server_db::ReadDatabase::open_pooled(&config.path, config.database.clone())?;
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                maximum_connections,
                state: Mutex::new(State {
                    idle: vec![database],
                    open_connections: 1,
                }),
            }),
        })
    }

    pub(crate) fn checkout(&self) -> Result<ReadLease> {
        let mut state = self.inner.state.lock().map_err(|_| Error::Thread)?;
        if let Some(database) = state.idle.pop() {
            return Ok(ReadLease {
                pool: Arc::clone(&self.inner),
                database: Some(database),
            });
        }
        if state.open_connections == self.inner.maximum_connections {
            return Err(Error::ReaderPool);
        }
        state.open_connections += 1;
        drop(state);

        match foks_server_db::ReadDatabase::open_pooled(
            &self.inner.config.path,
            self.inner.config.database.clone(),
        ) {
            Ok(database) => Ok(ReadLease {
                pool: Arc::clone(&self.inner),
                database: Some(database),
            }),
            Err(error) => {
                let mut state = self.inner.state.lock().map_err(|_| Error::Thread)?;
                state.open_connections = state.open_connections.saturating_sub(1);
                Err(error.into())
            }
        }
    }

    /// Checks out a connection for an RPC request. An exhausted pool is
    /// reported as `RateLimited` and counted in
    /// `foks_reader_pool_exhausted_total`; any other failure is a server fault
    /// counted at the `read_pool` site.
    pub(crate) fn lease(
        &self,
        metrics: &crate::ServerMetrics,
    ) -> std::result::Result<ReadLease, foks_rpc::RpcStatus> {
        match self.checkout() {
            Ok(lease) => Ok(lease),
            Err(Error::ReaderPool) => {
                metrics.reader_pool_exhausted();
                Err(foks_rpc::RpcStatus::RateLimited)
            }
            Err(error) => Err(crate::status::internal(error, "read_pool")),
        }
    }
}

impl Deref for ReadLease {
    type Target = foks_server_db::ReadDatabase;

    fn deref(&self) -> &Self::Target {
        self.database
            .as_ref()
            .expect("read lease always contains a database before drop")
    }
}

impl Drop for ReadLease {
    fn drop(&mut self) {
        let Some(database) = self.database.take() else {
            return;
        };
        if let Ok(mut state) = self.pool.state.lock() {
            state.idle.push(database);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_is_bounded_and_reuses_returned_connections() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("pool.sqlite");
        foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap();
        let pool = ReadPool::new(
            ReadDatabaseConfig {
                path,
                database: foks_server_db::Config::default(),
            },
            1,
        )
        .unwrap();

        let first = pool.checkout().unwrap();
        assert!(matches!(pool.checkout(), Err(Error::ReaderPool)));
        drop(first);
        pool.checkout().unwrap();
    }

    #[test]
    fn exhausted_lease_is_rate_limited_and_counted() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("pool.sqlite");
        foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap();
        let pool = ReadPool::new(
            ReadDatabaseConfig {
                path,
                database: foks_server_db::Config::default(),
            },
            1,
        )
        .unwrap();
        let metrics = crate::ServerMetrics::default();

        let first = pool.lease(&metrics).unwrap();
        assert!(matches!(
            pool.lease(&metrics),
            Err(foks_rpc::RpcStatus::RateLimited)
        ));
        assert_eq!(metrics.snapshot().reader_pool_exhausted, 1);
        drop(first);
        pool.lease(&metrics).unwrap();
        assert_eq!(metrics.snapshot().reader_pool_exhausted, 1);
    }

    #[test]
    fn pooled_connections_skip_the_kv_capacity_scan() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("pool.sqlite");
        drop(foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap());
        // Only the KV capacity scan in ReadDatabase::open visits this row; its
        // node exceeds the tighter limit below.
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        let mut node_id = [0u8; 17];
        node_id[0] = 3;
        connection
            .execute(
                "INSERT INTO kv_nodes(uid, node_id, node_type, exact_node) VALUES (?1, ?2, 3, ?3)",
                rusqlite::params![[1u8; 33], node_id, [0u8; 5]],
            )
            .unwrap();
        drop(connection);
        let config = foks_server_db::Config {
            maximum_kv_node_bytes: 4,
            ..foks_server_db::Config::default()
        };
        assert!(matches!(
            foks_server_db::ReadDatabase::open(&path, config.clone()),
            Err(foks_server_db::Error::QuotaExceeded)
        ));

        let pool = ReadPool::new(
            ReadDatabaseConfig {
                path,
                database: config,
            },
            2,
        )
        .unwrap();
        let first = pool.checkout().unwrap();
        let second = pool.checkout().unwrap();
        assert!(first.host_bootstrap().unwrap().is_none());
        assert!(second.host_bootstrap().unwrap().is_none());
    }
}
