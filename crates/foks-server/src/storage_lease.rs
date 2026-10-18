//! Host-local exclusion for object reads versus offline deletion. This is not a
//! distributed ownership fence. Cloned owners retain the OS lock through I/O.
use crate::{Error, Result};
use std::path::Path;
pub(crate) struct StorageLease {
    _file: std::fs::File,
    identity: foks_server_db::DatabasePathIdentity,
    namespace: Option<crate::chunks::ChunkNamespaceLease>,
}
impl StorageLease {
    #[cfg(test)]
    pub(crate) fn shared(path: &Path) -> Result<Self> {
        Self::acquire(path, false, None)
    }
    #[cfg(test)]
    pub(crate) fn exclusive(path: &Path) -> Result<Self> {
        Self::acquire(path, true, None)
    }
    pub(crate) fn with_store(
        path: &Path,
        store: Option<&dyn crate::chunks::ChunkStore>,
        exclusive: bool,
    ) -> Result<Self> {
        Self::acquire(path, exclusive, store)
    }
    pub(crate) fn require_writer(&self, writer: &crate::WriterHandle) -> Result<()> {
        self.identity.recheck()?;
        writer.database_identity().recheck()?;
        if &self.identity != writer.database_identity() {
            return Err(Error::Chunk("writer and storage source identities differ"));
        }
        Ok(())
    }
    pub(crate) fn authority_identity(&self) -> Result<Option<[u8; 32]>> {
        self.identity.recheck()?;
        #[cfg(unix)]
        {
            self.namespace
                .as_ref()
                .map(|_| self.identity.authority_identity().map_err(Error::from))
                .transpose()
        }
        #[cfg(not(unix))]
        {
            Ok(None)
        }
    }
    pub(crate) fn verify_authority(&self, expected: Option<[u8; 32]>) -> Result<bool> {
        self.identity.recheck()?;
        if let Some(expected) = expected {
            if self.authority_identity()? != Some(expected) {
                return Err(Error::Chunk(
                    "object namespace belongs to another database identity",
                ));
            }
            return Ok(true);
        }
        Ok(false)
    }
    fn acquire(
        path: &Path,
        exclusive: bool,
        store: Option<&dyn crate::chunks::ChunkStore>,
    ) -> Result<Self> {
        let identity = foks_server_db::DatabasePathIdentity::existing(path)?;
        let mut lock_path = identity.path().as_os_str().to_os_string();
        lock_path.push(".storage-lock");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(std::path::PathBuf::from(lock_path))?;
        if !file.metadata()?.is_file() {
            return Err(Error::Chunk("storage lock is not a regular file"));
        }
        if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        }
        .map_err(|_| Error::Chunk("storage is in use; retry after readers and backups finish"))?;
        identity.recheck()?;
        let namespace = store
            .map(|s| s.namespace_lease(exclusive))
            .transpose()?
            .flatten();
        identity.recheck()?;
        Ok(Self {
            _file: file,
            identity,
            namespace,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_owners_exclude_collection_until_last_reader_drops() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db");
        std::fs::write(&db, []).unwrap();
        let reader = std::sync::Arc::new(StorageLease::shared(&db).unwrap());
        let detached = reader.clone();
        assert!(StorageLease::exclusive(&db).is_err());
        drop(reader);
        assert!(StorageLease::exclusive(&db).is_err());
        drop(detached);
        let exclusive = StorageLease::exclusive(&db).unwrap();
        assert!(StorageLease::shared(&db).is_err());
        drop(exclusive);
        assert!(StorageLease::shared(&db).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn storage_lock_never_follows_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db");
        std::fs::write(&db, []).unwrap();
        std::fs::write(dir.path().join("other"), []).unwrap();
        std::os::unix::fs::symlink("other", dir.path().join("db.storage-lock")).unwrap();
        assert!(StorageLease::shared(&db).is_err());
    }
}
