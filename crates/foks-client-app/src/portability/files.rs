use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Read as _;
use std::path::{Path, PathBuf};

pub(super) const MAX_ENTRIES: usize = 8192;
pub(super) const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
pub(super) const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Artifact {
    pub path: String,
    pub size: u64,
    pub mode: u32,
    pub sha256: [u8; 32],
    pub soft: bool,
}
pub(super) fn private_directory(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(Error::StatePathChanged);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if m.mode() & 0o077 != 0 || m.uid() != rustix::process::getuid().as_raw() {
            return Err(Error::InvalidConfig(
                "snapshot directory is not private and owned",
            ));
        }
    }
    Ok(())
}
pub(super) fn entries(directory: &Path) -> Result<Vec<(String, PathBuf)>> {
    private_directory(directory)?;
    let mut entries = Vec::new();
    for item in fs::read_dir(directory)? {
        if entries.len() == MAX_ENTRIES {
            return Err(Error::InvalidConfig(
                "snapshot inventory exceeds entry limit",
            ));
        }
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| Error::InvalidConfig("snapshot filename is not UTF8"))?;
        entries.push((name, item.path()));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}
pub(super) fn regular(path: &Path) -> Result<fs::Metadata> {
    regular_bounded(path, MAX_FILE)
}
pub(super) fn regular_bounded(path: &Path, maximum: u64) -> Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err(Error::InvalidConfig("snapshot entry is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if m.nlink() != 1 || m.uid() != rustix::process::getuid().as_raw() {
            return Err(Error::InvalidConfig(
                "snapshot entry is linked or not owned",
            ));
        }
        if m.len() > 4096 && m.blocks().saturating_mul(512) < m.len() {
            return Err(Error::InvalidConfig("snapshot entry is sparse"));
        }
    }
    if m.len() > maximum {
        return Err(Error::InvalidConfig("snapshot file exceeds limit"));
    }
    Ok(m)
}
pub(crate) fn changed(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if a.dev() != b.dev()
            || a.ino() != b.ino()
            || a.ctime() != b.ctime()
            || a.ctime_nsec() != b.ctime_nsec()
        {
            return true;
        }
    }
    a.len() != b.len() || a.modified().ok() != b.modified().ok()
}
pub(super) fn open_regular(path: &Path) -> Result<File> {
    open_regular_bounded(path, MAX_FILE)
}
pub(super) fn open_regular_bounded(path: &Path, maximum: u64) -> Result<File> {
    let before = regular_bounded(path, maximum)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if changed(&before, &file.metadata()?) {
        return Err(Error::StatePathChanged);
    }
    Ok(file)
}
pub(super) fn artifact(root: &Path, path: &Path, soft: bool) -> Result<Artifact> {
    let file = open_regular(path)?;
    let before = file.metadata()?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut total = 0u64;
    let mut reader = &file;
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total = total.checked_add(n as u64).ok_or(Error::StatePathChanged)?;
        if total > before.len() {
            return Err(Error::StatePathChanged);
        }
        hash.update(&buffer[..n]);
    }
    if total != before.len()
        || changed(&before, &file.metadata()?)
        || changed(&before, &regular(path)?)
    {
        return Err(Error::StatePathChanged);
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| Error::StatePathChanged)?
        .to_str()
        .ok_or(Error::StatePathChanged)?
        .to_owned();
    if relative.len() > 512 {
        return Err(Error::InvalidConfig("snapshot path exceeds limit"));
    }
    Ok(Artifact {
        path: relative,
        size: total,
        mode: 0o600,
        sha256: hash.finalize().into(),
        soft,
    })
}

/// Presence checks on authority metadata must not follow dangling symlinks.
pub(super) fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
