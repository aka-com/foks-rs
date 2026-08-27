use std::path::{Path, PathBuf};

pub(crate) struct IsolatedPaths {
    root: tempfile::TempDir,
    database: PathBuf,
    keys: PathBuf,
    backup: PathBuf,
    logs: PathBuf,
}

impl IsolatedPaths {
    pub(crate) fn create() -> std::io::Result<Self> {
        let root = tempfile::tempdir()?;
        let database = root.path().join("database/foks-server.sqlite");
        let keys = root.path().join("keys");
        let backup = root.path().join("backup");
        let logs = root.path().join("logs");
        for path in [database.parent().unwrap(), &keys, &backup, &logs] {
            std::fs::create_dir_all(path)?;
        }
        Ok(Self {
            root,
            database,
            keys,
            backup,
            logs,
        })
    }

    pub(crate) fn root(&self) -> &Path {
        self.root.path()
    }

    pub(crate) fn all(&self) -> [&Path; 4] {
        [&self.database, &self.keys, &self.backup, &self.logs]
    }

    pub(crate) fn database(&self) -> &Path {
        &self.database
    }

    pub(crate) fn keys(&self) -> &Path {
        &self.keys
    }
}
