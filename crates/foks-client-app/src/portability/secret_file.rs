//! Native-only transfer key file handling; no secret-bearing DTO exists.
use super::{
    files, lease::canonical_reservation, publication::PrivatePublication, StateExportAuthorization,
    StateExportReport, StateTransferKey,
};
use crate::{Error, Result};
use std::{
    io::{Read as _, Write as _},
    path::Path,
};
use zeroize::Zeroizing;
pub fn read_private_secret_file(path: impl AsRef<Path>, maximum: u64) -> Result<Zeroizing<String>> {
    use std::os::unix::fs::MetadataExt as _;
    if maximum > 4096 {
        return Err(Error::InvalidConfig("secret input limit exceeded"));
    }
    let mut file = files::open_regular_bounded(path.as_ref(), maximum)?;
    if file.metadata()?.mode() & 0o077 != 0 {
        return Err(Error::InvalidConfig("secret file must be private"));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    std::io::Read::by_ref(&mut file)
        .take(maximum + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(Error::InvalidConfig("secret input limit exceeded"));
    }
    if bytes.ends_with(b"\n") {
        bytes.pop();
        if bytes.ends_with(b"\r") {
            bytes.pop();
        }
    }
    Ok(Zeroizing::new(
        std::str::from_utf8(&bytes)
            .map_err(|_| Error::InvalidConfig("secret file is not UTF8"))?
            .to_owned(),
    ))
}
pub fn read_transfer_key(path: impl AsRef<Path>) -> Result<StateTransferKey> {
    Ok(StateTransferKey::parse(&read_private_secret_file(
        path, 128,
    )?)?)
}
impl StateExportAuthorization {
    /// Publish the key first so a completed archive can never lose its only key.
    /// If archive publication fails the private key file is retained for the user.
    pub fn export_with_key_file(
        self,
        archive: impl AsRef<Path>,
        key_file: impl AsRef<Path>,
    ) -> Result<StateExportReport> {
        let archive = canonical_reservation(archive.as_ref())?;
        let path = canonical_reservation(key_file.as_ref())?;
        if archive == path
            || path.starts_with(self.source_root())
            || path.starts_with(super::lease::lock_directory()?)
        {
            return Err(Error::InvalidConfig(
                "key output must be separate from archive and client state",
            ));
        }
        if files::exists(&archive)? {
            return Err(Error::InvalidConfig("archive output already exists"));
        }
        let mut output = PrivatePublication::reserve(&path)?;
        let key = StateTransferKey::generate()?;
        output.file.write_all(key.expose_encoding().as_bytes())?;
        output.file.write_all(b"\n")?;
        output.publish()?;
        self.export(archive, &key)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_files_are_bounded_private_no_follow_and_never_serialized() {
        use std::os::unix::fs::{symlink, PermissionsExt as _};
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("key");
        let key = StateTransferKey::generate().unwrap();
        let mut publication = PrivatePublication::reserve(&path).unwrap();
        publication
            .file
            .write_all(key.expose_encoding().as_bytes())
            .unwrap();
        publication.publish().unwrap();
        assert_eq!(
            *read_transfer_key(&path).unwrap().expose_encoding(),
            *key.expose_encoding()
        );
        let link = d.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(read_transfer_key(link).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_transfer_key(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&path, [b'a'; 129]).unwrap();
        assert!(read_transfer_key(&path).is_err());
        assert!(!format!("{key:?}").contains(key.expose_encoding().as_str()));
    }
}
