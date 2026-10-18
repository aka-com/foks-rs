//! Private, bounded token handoff shared by native frontends. Paths are chosen locally.
use foks_agent_proto::SecretString;
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};
use zeroize::Zeroizing;
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "invalid or non-private bot token file",
    )
}
pub fn read_token(reader: impl Read) -> io::Result<SecretString> {
    let mut bytes = Zeroizing::new(Vec::new());
    reader.take(40).read_to_end(&mut bytes)?;
    if bytes.ends_with(b"\n") {
        bytes.pop();
        if bytes.ends_with(b"\r") {
            bytes.pop();
        }
    }
    if bytes.len() != 37 {
        return Err(invalid());
    }
    Ok(SecretString::new(
        std::str::from_utf8(&bytes).map_err(|_| invalid())?,
    ))
}
pub fn import_token(path: &Path) -> io::Result<SecretString> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(
            (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC).bits() as i32,
        );
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > 39 {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid());
        }
    }
    read_token(file)
}
/// An unpublished private temporary file and the directory owning both names.
/// Dropping it removes only the temporary name, never a published destination.
pub struct ExportReservation {
    file: File,
    parent: File,
    temporary: std::ffi::OsString,
    target: std::ffi::OsString,
    started: bool,
    prepared: bool,
    published: bool,
}
impl Drop for ExportReservation {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = rustix::fs::unlinkat(&self.parent, &self.temporary, rustix::fs::AtFlags::empty());
    }
}

/// Reserve a hidden private file, leaving the requested destination untouched.
#[cfg(unix)]
pub fn reserve_export(path: &Path) -> io::Result<ExportReservation> {
    use rustix::fs::{AtFlags, Mode, OFlags};
    use std::os::unix::fs::OpenOptionsExt;
    let target = path
        .file_name()
        .filter(|n| !n.is_empty() && *n != "." && *n != "..")
        .ok_or_else(invalid)?;
    let parent_path = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC).bits() as i32)
        .open(parent_path)?;
    match rustix::fs::statat(&parent, target, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "export destination exists",
            ))
        }
        Err(rustix::io::Errno::NOENT) => {}
        Err(error) => return Err(error.into()),
    }
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| io::Error::other("export randomness unavailable"))?;
    let suffix = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let temporary = std::ffi::OsString::from(format!(".foks-export-{suffix}.tmp"));
    let fd = rustix::fs::openat(
        &parent,
        &temporary,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )?;
    Ok(ExportReservation {
        file: fd.into(),
        parent,
        temporary,
        target: target.to_owned(),
        started: false,
        prepared: false,
        published: false,
    })
}
#[cfg(not(unix))]
pub fn reserve_export(_path: &Path) -> io::Result<ExportReservation> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "durable private export is unsupported on this OS",
    ))
}

/// Write and sync the private temporary file outside the publication gate.
pub fn prepare_export(reservation: &mut ExportReservation, secret: &str) -> io::Result<()> {
    prepare_export_with_sync(reservation, secret, File::sync_all)
}
fn prepare_export_with_sync(
    reservation: &mut ExportReservation,
    secret: &str,
    sync: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<()> {
    if secret.len() != 37 || reservation.started || reservation.published {
        return Err(invalid());
    }
    reservation.started = true;
    reservation.file.write_all(secret.as_bytes())?;
    reservation.file.write_all(b"\n")?;
    sync(&reservation.file)?;
    reservation.prepared = true;
    Ok(())
}
/// Final no-clobber publication. The caller holds its generation/deadline gate.
#[cfg(unix)]
pub fn publish_export(reservation: &mut ExportReservation) -> io::Result<()> {
    if !reservation.prepared || reservation.published {
        return Err(invalid());
    }
    rustix::fs::linkat(
        &reservation.parent,
        &reservation.temporary,
        &reservation.parent,
        &reservation.target,
        rustix::fs::AtFlags::empty(),
    )?;
    // No fallible work may intervene before recording successful publication.
    reservation.published = true;
    Ok(())
}
#[cfg(not(unix))]
pub fn publish_export(_reservation: &mut ExportReservation) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "durable private export is unsupported on this OS",
    ))
}
/// Complete directory durability outside the publication gate, before acknowledgement.
pub fn finish_export(reservation: &mut ExportReservation) -> io::Result<()> {
    finish_export_with_sync(reservation, File::sync_all)
}
fn finish_export_with_sync(
    reservation: &mut ExportReservation,
    sync: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<()> {
    if !reservation.published {
        return Err(invalid());
    }
    #[cfg(unix)]
    match rustix::fs::unlinkat(
        &reservation.parent,
        &reservation.temporary,
        rustix::fs::AtFlags::empty(),
    ) {
        Ok(()) | Err(rustix::io::Errno::NOENT) => {}
        Err(error) => return Err(error.into()),
    }
    sync(&reservation.parent)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_private_file_handoff() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        let token = "a".repeat(37);
        let mut file = reserve_export(&path).unwrap();
        assert!(!path.exists());
        prepare_export(&mut file, &token).unwrap();
        assert!(!path.exists());
        publish_export(&mut file).unwrap();
        finish_export(&mut file).unwrap();
        assert!(reserve_export(&path).is_err());
        assert_eq!(import_token(&path).unwrap().expose(), token);
        assert!(read_token(&b"too short"[..]).is_err());
        assert!(read_token(&[b'a'; 40][..]).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt};
            let link = dir.path().join("link");
            symlink(&path, &link).unwrap();
            assert!(import_token(&link).is_err());
            std::fs::set_permissions(path.clone(), std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(import_token(&path).is_err());
        }
    }
    #[test]
    fn publication_failure_and_discard_preserve_destination_and_cleanup_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        let token = "a".repeat(37);
        {
            let mut reservation = reserve_export(&path).unwrap();
            assert!(
                prepare_export_with_sync(&mut reservation, &token, |_| Err(io::Error::other(
                    "injected file sync"
                )))
                .is_err()
            );
            assert!(publish_export(&mut reservation).is_err());
            assert!(!path.exists());
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        {
            let mut reservation = reserve_export(&path).unwrap();
            prepare_export(&mut reservation, &token).unwrap();
            std::fs::write(&path, b"existing").unwrap();
            assert!(publish_export(&mut reservation).is_err());
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        std::fs::remove_file(&path).unwrap();
        {
            let mut reservation = reserve_export(&path).unwrap();
            prepare_export(&mut reservation, &token).unwrap();
            publish_export(&mut reservation).unwrap();
            assert!(
                finish_export_with_sync(&mut reservation, |_| Err(io::Error::other(
                    "injected parent sync"
                )))
                .is_err()
            );
        }
        assert_eq!(import_token(&path).unwrap().expose(), token);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn parent_directory_rename_does_not_redirect_creation_or_sync() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("parent");
        let moved = dir.path().join("moved");
        std::fs::create_dir(&original).unwrap();
        let mut reservation = reserve_export(&original.join("token")).unwrap();
        std::fs::rename(&original, &moved).unwrap();
        std::fs::create_dir(&original).unwrap();
        prepare_export(&mut reservation, &"a".repeat(37)).unwrap();
        publish_export(&mut reservation).unwrap();
        finish_export(&mut reservation).unwrap();
        assert!(!original.join("token").exists());
        assert!(moved.join("token").exists());
    }
}
