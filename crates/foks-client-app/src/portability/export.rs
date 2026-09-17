//! One-use export approval over a typed, exclusively reserved snapshot.
use super::{
    archive_manifest::ArchiveManifest,
    files,
    inventory::StateSnapshot,
    lease::{canonical_reservation, DirectoryIdentity},
    publication::PrivatePublication,
    ClientStateMaintenanceGuard, StateInventoryReport,
};
use crate::{Error, Result};
use foks_keystore::state_archive::{ArchiveWriter, StateTransferKey};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub struct StateExportPreview {
    guard: ClientStateMaintenanceGuard,
    snapshot: StateSnapshot,
}
pub struct StateExportAuthorization {
    guard: ClientStateMaintenanceGuard,
    snapshot: StateSnapshot,
}
#[derive(Serialize)]
pub struct StateExportReport {
    pub path: PathBuf,
    pub profiles: usize,
    pub files: usize,
    pub bytes: u64,
    pub archive_id: String,
}

pub fn prepare_state_export(root: impl AsRef<Path>) -> Result<StateExportPreview> {
    let root = canonical_reservation(root.as_ref())?;
    let mut guard = ClientStateMaintenanceGuard::acquire(std::slice::from_ref(&root))?;
    let snapshot = guard.inspect(&root)?;
    Ok(StateExportPreview { guard, snapshot })
}
impl StateExportPreview {
    pub fn report(&self) -> Result<StateInventoryReport> {
        self.snapshot.report()
    }
    /// Consumes the preview. Approval must name the exact digest shown to the user.
    pub fn authorize(self, digest: &str) -> Result<StateExportAuthorization> {
        if crate::hex(&self.snapshot.digest()?) != digest {
            return Err(Error::StatePathChanged);
        }
        self.snapshot.require_exportable()?;
        Ok(StateExportAuthorization {
            guard: self.guard,
            snapshot: self.snapshot,
        })
    }
}
impl StateExportAuthorization {
    pub(super) fn source_root(&self) -> &Path {
        &self.snapshot.root
    }
    pub fn export(
        self,
        destination: impl AsRef<Path>,
        key: &StateTransferKey,
    ) -> Result<StateExportReport> {
        let Self {
            mut guard,
            snapshot,
        } = self;
        let destination = canonical_reservation(destination.as_ref())?;
        if destination.starts_with(&snapshot.root) || destination.starts_with(&guard.base) {
            return Err(Error::InvalidConfig(
                "archive output must be outside client state and maintenance storage",
            ));
        }
        // Reserve output before any approved trust normalization, so collisions
        // do not alter the source even through a reversible config update.
        let mut output = PrivatePublication::reserve(&destination)?;
        let snapshot = snapshot.normalize_trust(&mut guard)?;
        let manifest = ArchiveManifest::from_snapshot(&snapshot)?;
        let bytes = Zeroizing::new(serde_json::to_vec(&manifest)?);
        let mut archive = ArchiveWriter::new(&mut output.file, key, &bytes)?;
        let archive_id = crate::hex(&archive.archive_id());
        for entry in &manifest.entries {
            let path = snapshot.root.join(&entry.path);
            let file = files::open_regular(&path)?;
            let before = file.metadata()?;
            let mut reader = HashReader {
                file,
                hash: Sha256::new(),
            };
            archive.write_entry(entry.size, &mut reader)?;
            if files::changed(&before, &reader.file.metadata()?)
                || files::changed(&before, &files::regular(&path)?)
                || <[u8; 32]>::from(reader.hash.finalize()) != entry.sha256
            {
                return Err(Error::StatePathChanged);
            }
        }
        archive.finish()?;
        if DirectoryIdentity::read(&snapshot.root)? != snapshot.identity {
            return Err(Error::StatePathChanged);
        }
        guard.with_native_manifest(|m| {
            if m.generation == snapshot.native.generation {
                Ok(())
            } else {
                Err(Error::StatePathChanged)
            }
        })?;
        output.publish()?;
        Ok(StateExportReport {
            path: destination,
            profiles: manifest.profiles.len(),
            files: manifest.entries.len(),
            bytes: manifest.entries.iter().map(|e| e.size).sum(),
            archive_id,
        })
    }
}
struct HashReader {
    file: File,
    hash: Sha256,
}
impl Read for HashReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let n = self.file.read(bytes)?;
        self.hash.update(&bytes[..n]);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_keystore::state_archive::ArchiveReader;
    fn enabled() -> bool {
        std::env::var_os("FOKS_TEST_NATIVE_PORTABILITY").is_some()
    }
    #[test]
    fn complete_native_account_export_preserves_source_and_authenticates_inventory() {
        if !enabled() {
            return;
        }
        let fixture = crate::test_support::AccountFixture::start_native();
        fixture
            .run(|s, v, k| s.create_account("work", "stateexport", "laptop", "", "", None, v, k));
        fixture.run(|s, v, _| s.configure_web_admin("work", "https://admin.example/", v));
        let fixture = fixture.stop_client();
        let key = StateTransferKey::generate().unwrap();
        let output = fixture
            .environment
            .client_path("transfer", "state.foks")
            .unwrap();
        let preview = prepare_state_export(&fixture.root).unwrap();
        let report = preview.report().unwrap();
        assert!(
            report.exportable,
            "{:?}",
            report
                .profiles
                .iter()
                .map(|p| &p.blockers)
                .collect::<Vec<_>>()
        );
        assert!(crate::ClientCredentials::open(&fixture.root).is_err());
        let exported = preview
            .authorize(&report.digest)
            .unwrap()
            .export(&output, &key)
            .unwrap();
        let mut reader = ArchiveReader::new(File::open(&output).unwrap(), &key).unwrap();
        let manifest = ArchiveManifest::decode(reader.manifest()).unwrap();
        assert_eq!(manifest.source_state_id, fixture.state_id);
        assert_eq!(manifest.entries.len(), exported.files);
        assert!(manifest.profiles["local"]
            .vault
            .iter()
            .any(|key| key == "account.work"));
        assert!(manifest.profiles["local"]
            .vault
            .iter()
            .any(|key| key == "web-admin.work"));
        assert!(manifest
            .entries
            .iter()
            .any(|e| e.path.starts_with("trust/")));
        for entry in &manifest.entries {
            let mut bytes = Zeroizing::new(Vec::new());
            reader.read_entry(entry.size, &mut *bytes).unwrap();
            assert_eq!(
                <[u8; 32]>::from(Sha256::digest(bytes.as_slice())),
                entry.sha256
            );
            assert_eq!(
                bytes.as_slice(),
                std::fs::read(fixture.root.join(&entry.path)).unwrap()
            );
        }
        reader.finish().unwrap();
        assert_eq!(
            crate::ClientCredentials::open(&fixture.root)
                .unwrap()
                .state_id,
            fixture.state_id
        );
        assert!(prepare_state_export(&fixture.root)
            .unwrap()
            .authorize("wrong preview")
            .is_err());
    }
    #[test]
    fn export_detects_changed_inputs_and_never_overwrites_outputs() {
        if !enabled() {
            return;
        }
        let fixture = crate::test_support::AccountFixture::start_native().stop_client();
        let output = fixture
            .environment
            .client_path("transfer", "archive")
            .unwrap();
        let key = StateTransferKey::generate().unwrap();
        let preview = prepare_state_export(&fixture.root).unwrap();
        let digest = preview.report().unwrap().digest;
        std::fs::write(&output, b"existing").unwrap();
        assert!(preview
            .authorize(&digest)
            .unwrap()
            .export(&output, &key)
            .is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"existing");
        std::fs::remove_file(&output).unwrap();
        let preview = prepare_state_export(&fixture.root).unwrap();
        let digest = preview.report().unwrap().digest;
        let config = fixture.root.join("profiles.toml");
        let mut changed = std::fs::read(&config).unwrap();
        changed.push(b'\n');
        std::fs::write(&config, &changed).unwrap();
        assert!(preview
            .authorize(&digest)
            .unwrap()
            .export(&output, &key)
            .is_err());
        assert!(!output.exists());
        assert_eq!(
            std::fs::read_dir(output.parent().unwrap()).unwrap().count(),
            0
        );
    }
}
