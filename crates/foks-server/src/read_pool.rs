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
        let database = foks_server_db::ReadDatabase::open(&config.path, config.database.clone())?;
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

        match foks_server_db::ReadDatabase::open(
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
}
