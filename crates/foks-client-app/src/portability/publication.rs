//! Private create-new output with descriptor-bound no-replace publication.
use super::{
    files,
    lease::{canonical_reservation, DirectoryIdentity},
};
use crate::{Error, Result};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

pub(super) struct PrivatePublication {
    pub file: File,
    path: PathBuf,
    parent: File,
    parent_identity: DirectoryIdentity,
    temporary: OsString,
    published: bool,
}
impl PrivatePublication {
    pub fn reserve(path: &Path) -> Result<Self> {
        let path = canonical_reservation(path)?;
        if files::exists(&path)? {
            return Err(Error::InvalidConfig("archive output already exists"));
        }
        let parent_path = path.parent().ok_or(Error::StatePathChanged)?;
        let parent_identity = DirectoryIdentity::read(parent_path)?;
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(parent_path)?;
        let metadata = parent.metadata()?;
        if metadata.dev() != parent_identity.device || metadata.ino() != parent_identity.inode {
            return Err(Error::StatePathChanged);
        }
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| Error::Randomness)?;
        let temporary = OsString::from(format!(".foks-state-export-{}.tmp", crate::hex(&nonce)));
        let fd = rustix::fs::openat(
            &parent,
            &temporary,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_raw_mode(0o600),
        )
        .map_err(std::io::Error::from)?;
        Ok(Self {
            file: File::from(fd),
            path,
            parent,
            parent_identity,
            temporary,
            published: false,
        })
    }
    fn same_temporary(&self) -> bool {
        let Ok(named) = rustix::fs::statat(
            &self.parent,
            &self.temporary,
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        ) else {
            return false;
        };
        rustix::fs::fstat(&self.file).is_ok_and(|opened| {
            opened.st_dev == named.st_dev && opened.st_ino == named.st_ino && opened.st_nlink == 1
        })
    }
    pub fn publish(&mut self) -> Result<()> {
        self.file.sync_all()?;
        if !self.same_temporary()
            || DirectoryIdentity::read(self.path.parent().ok_or(Error::StatePathChanged)?)?
                != self.parent_identity
        {
            return Err(Error::StatePathChanged);
        }
        rustix::fs::renameat_with(
            &self.parent,
            &self.temporary,
            &self.parent,
            self.path.file_name().ok_or(Error::StatePathChanged)?,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from)?;
        self.published = true;
        self.parent.sync_all()?;
        if DirectoryIdentity::read(self.path.parent().ok_or(Error::StatePathChanged)?)?
            != self.parent_identity
        {
            return Err(Error::StatePathChanged);
        }
        Ok(())
    }
}
impl Drop for PrivatePublication {
    fn drop(&mut self) {
        if !self.published && self.same_temporary() {
            let _ =
                rustix::fs::unlinkat(&self.parent, &self.temporary, rustix::fs::AtFlags::empty());
            let _ = self.parent.sync_all();
        }
    }
}
// A deterministic, nonce-owned temporary is recoverable after a torn metadata write.
pub(super) fn durable_metadata(
    path: &Path,
    bytes: &[u8],
    nonce: &[u8; 32],
    hook: &mut dyn FnMut(&'static str) -> Result<()>,
    before: &'static str,
    after: &'static str,
) -> Result<()> {
    use std::io::Write as _;
    hook(before)?;
    let temporary = path.with_extension(format!("pending-{}", crate::hex(nonce)));
    if files::exists(&temporary)? {
        files::regular(&temporary)?;
        std::fs::remove_file(&temporary)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)?;
    hook("after-metadata-create")?;
    file.write_all(bytes)?;
    hook("before-metadata-data-sync")?;
    file.sync_all()?;
    hook("after-metadata-data-sync")?;
    hook("before-metadata-rename")?;
    std::fs::rename(&temporary, path)?;
    hook("after-metadata-rename")?;
    hook("before-metadata-parent-sync")?;
    File::open(path.parent().ok_or(Error::StatePathChanged)?)?.sync_all()?;
    hook("after-metadata-parent-sync")?;
    hook(after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write as _;
    #[test]
    fn output_collision_and_abandoned_stream_preserve_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive");
        {
            let mut output = PrivatePublication::reserve(&path).unwrap();
            output.file.write_all(b"private").unwrap();
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        let mut output = PrivatePublication::reserve(&path).unwrap();
        output.file.write_all(b"archive").unwrap();
        fs::write(&path, b"racer").unwrap();
        assert!(output.publish().is_err());
        drop(output);
        assert_eq!(fs::read(&path).unwrap(), b"racer");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn publication_is_private_and_parent_substitution_fails() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("archive");
        let mut output = PrivatePublication::reserve(&path).unwrap();
        output.file.write_all(b"archive").unwrap();
        fs::rename(&parent, dir.path().join("moved")).unwrap();
        fs::create_dir(&parent).unwrap();
        assert!(output.publish().is_err());
        drop(output);
        assert!(!path.exists());
        assert_eq!(fs::read_dir(dir.path().join("moved")).unwrap().count(), 0);
        let mut output = PrivatePublication::reserve(&path).unwrap();
        output.file.write_all(b"archive").unwrap();
        output.publish().unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    }
}
