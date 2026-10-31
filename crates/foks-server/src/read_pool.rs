use std::ops::Deref;
use std::sync::{Arc, Mutex};

use crate::{Error, ReadDatabaseConfig, Result};

/// Returns a reader opener for `config`. When `writer` owns the same file
/// and validated KV limits that cover the reader's, its startup capacity scan
/// stands for the reader's and the opener runs none. Otherwise the opener
/// runs the scan once.
pub(crate) fn opener(
    config: &ReadDatabaseConfig,
    writer: Option<&crate::WriterHandle>,
) -> Result<foks_server_db::ReadDatabaseOpener> {
    if let Some(writer) = writer {
        let path = config.path.clone();
        let database = config.database.clone();
        if let Some(opener) =
            writer.call(move |writer| Ok(writer.read_opener(&path, &database)?))?
        {
            return Ok(opener);
        }
    }
    Ok(foks_server_db::ReadDatabaseOpener::new(
        &config.path,
        config.database.clone(),
    )?)
}

#[derive(Clone)]
pub(crate) struct ReadPool {
    inner: Arc<Inner>,
}

struct Inner {
    opener: foks_server_db::ReadDatabaseOpener,
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
    /// Opens a pool whose opener runs the KV capacity scan once.
    #[cfg(any(test, feature = "celld-proof"))]
    pub(crate) fn new(config: ReadDatabaseConfig, maximum_connections: usize) -> Result<Self> {
        Self::with_opener(opener(&config, None)?, maximum_connections)
    }

    /// Opens a pool from an opener that has already validated the database.
    /// Connections check the file identity, the schema version and the schema
    /// contract; none repeats the KV capacity scan.
    pub(crate) fn with_opener(
        opener: foks_server_db::ReadDatabaseOpener,
        maximum_connections: usize,
    ) -> Result<Self> {
        if maximum_connections == 0 {
            return Err(Error::Config(
                "maximum read connections must be greater than zero",
            ));
        }
        let database = opener.open()?;
        Ok(Self {
            inner: Arc::new(Inner {
                opener,
                maximum_connections,
                state: Mutex::new(State {
                    idle: vec![database],
                    open_connections: 1,
                }),
            }),
        })
    }

    /// The number of connections the pool holds open, idle or leased.
    pub(crate) fn open_connections(&self) -> Result<usize> {
        let state = self.inner.state.lock().map_err(|_| Error::Thread)?;
        Ok(state.open_connections)
    }

    pub(crate) fn checkout(&self) -> Result<ReadLease> {
        self.checkout_counted().map(|(lease, _)| lease)
    }

    /// Checks out a connection and returns it with the number of connections
    /// the pool held open when it was checked out. A request whose deadline
    /// has passed fails with `Error::DeadlineExceeded` before it checks out.
    fn checkout_counted(&self) -> Result<(ReadLease, usize)> {
        crate::deadline::check()?;
        let mut state = self.inner.state.lock().map_err(|_| Error::Thread)?;
        if let Some(database) = state.idle.pop() {
            let lease = ReadLease {
                pool: Arc::clone(&self.inner),
                database: Some(database),
            };
            return Ok((lease, state.open_connections));
        }
        if state.open_connections == self.inner.maximum_connections {
            return Err(Error::ReaderPool);
        }
        state.open_connections += 1;
        let open = state.open_connections;
        drop(state);

        match self.inner.opener.open() {
            Ok(database) => Ok((
                ReadLease {
                    pool: Arc::clone(&self.inner),
                    database: Some(database),
                },
                open,
            )),
            Err(error) => {
                let mut state = self.inner.state.lock().map_err(|_| Error::Thread)?;
                state.open_connections = state.open_connections.saturating_sub(1);
                Err(error.into())
            }
        }
    }

    /// Checks out a connection for an RPC request and records the number of
    /// open connections in `foks_reader_pool_open`. An exhausted pool is
    /// reported as `RateLimited` and counted in
    /// `foks_reader_pool_exhausted_total`. An expired request deadline is
    /// reported as `RateLimited` through the shared statuses; any other failure
    /// is a server fault counted at the `read_pool` site.
    pub(crate) fn lease(
        &self,
        metrics: &crate::ServerMetrics,
    ) -> std::result::Result<ReadLease, foks_rpc::RpcStatus> {
        match self.checkout_counted() {
            Ok((lease, open)) => {
                metrics.reader_pool_opened(open);
                Ok(lease)
            }
            Err(Error::ReaderPool) => {
                metrics.reader_pool_exhausted();
                Err(foks_rpc::RpcStatus::RateLimited)
            }
            Err(error) => Err(crate::status::unmapped(error, "read_pool")),
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
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.reader_pool_exhausted, 1);
        assert_eq!(snapshot.rate_limited_requests, 0);
        drop(first);
        pool.lease(&metrics).unwrap();
        assert_eq!(metrics.snapshot().reader_pool_exhausted, 1);
    }

    #[test]
    fn leases_report_the_open_connection_count() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("pool.sqlite");
        foks_server_db::Database::open(&path, foks_server_db::Config::default()).unwrap();
        let pool = ReadPool::new(
            ReadDatabaseConfig {
                path,
                database: foks_server_db::Config::default(),
            },
            3,
        )
        .unwrap();
        let metrics = crate::ServerMetrics::default();

        let first = pool.lease(&metrics).unwrap();
        assert_eq!(metrics.snapshot().reader_pool_open, 1);
        let second = pool.lease(&metrics).unwrap();
        assert_eq!(metrics.snapshot().reader_pool_open, 2);
        drop((first, second));
        // Returned connections stay open, so reusing one keeps the count.
        let _reused = pool.lease(&metrics).unwrap();
        assert_eq!(metrics.snapshot().reader_pool_open, 2);
    }
}
