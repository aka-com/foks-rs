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
/// Reserve before requesting the one-time secret from the agent.
pub fn reserve_export(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(
            (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC).bits() as i32,
        );
    }
    options.open(path)
}
pub fn write_export(file: &mut File, secret: &str) -> io::Result<()> {
    if secret.len() != 37 {
        return Err(invalid());
    }
    file.write_all(secret.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()
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
        assert!(reserve_export(&path).is_err());
        write_export(&mut file, &token).unwrap();
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
}
