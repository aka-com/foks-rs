//! Root-owned trust artifacts retain exact custom CA bytes across path changes.
use crate::{Error, Result, TrustRoot};
use sha2::{Digest as _, Sha256};
use std::path::Path;

pub(crate) fn validate_digest(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|b| matches!(b,b'0'..=b'9'|b'a'..=b'f')) {
        return Err(Error::TrustRoot);
    }
    Ok(())
}
pub(crate) fn read_certificate(root: &Path, trust: &TrustRoot) -> Result<Option<Vec<u8>>> {
    let (path, expected) = match trust {
        TrustRoot::WebPki => return Ok(None),
        TrustRoot::CertificateDer { path } => (path.clone(), None),
        TrustRoot::CertificateArtifact { sha256 } => {
            validate_digest(sha256)?;
            super::files::private_directory(&root.join("trust"))?;
            (
                root.join("trust").join(format!("{sha256}.der")),
                Some(sha256),
            )
        }
    };
    let certificate = crate::read_bounded_regular_file(&path, crate::MAX_CERTIFICATE_BYTES as u64)?;
    validate_certificate(&certificate)?;
    if expected.is_some_and(|digest| *digest != crate::hex(&Sha256::digest(&certificate))) {
        return Err(Error::TrustRoot);
    }
    Ok(Some(certificate))
}
pub(super) fn validate_certificate(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > crate::MAX_CERTIFICATE_BYTES {
        return Err(Error::TrustRoot);
    }
    rustls::RootCertStore::empty()
        .add(rustls_pki_types::CertificateDer::from(bytes))
        .map_err(|_| Error::TrustRoot)
}

impl super::inventory::StateSnapshot {
    /// Uses the exact DER captured in the reviewed snapshot, never an external
    /// path that may have changed since preview. The caller retains all leases.
    pub(super) fn normalize_trust(
        self,
        guard: &mut super::ClientStateMaintenanceGuard,
    ) -> Result<Self> {
        guard.require_path(&self.root)?;
        for expected in &self.artifacts {
            if super::files::artifact(&self.root, &self.root.join(&expected.path), expected.soft)?
                != *expected
            {
                return Err(Error::StatePathChanged);
            }
        }
        guard.with_native_manifest(|m| {
            if m.generation != self.native.generation {
                return Err(Error::StatePathChanged);
            }
            Ok(())
        })?;
        let mut profiles = std::collections::BTreeMap::new();
        let mut changed = false;
        for (name, p) in &self.profiles {
            let mut profile = p.profile.clone();
            if let TrustRoot::CertificateDer { .. } = &profile.trust {
                // Snapshot associates each profile with its captured certificate.
                let digest = p.trust_digest.as_ref().ok_or(Error::TrustRoot)?;
                let bytes = self.trust.get(digest).ok_or(Error::TrustRoot)?;
                let directory = self.root.join("trust");
                if directory.exists() {
                    super::files::private_directory(&directory)?;
                } else {
                    crate::prepare_private_directory(&directory)?;
                    std::fs::File::open(&self.root)?.sync_all()?;
                }
                let path = directory.join(format!("{digest}.der"));
                if path.exists() {
                    if crate::read_bounded_regular_file(&path, crate::MAX_CERTIFICATE_BYTES as u64)?
                        != *bytes
                    {
                        return Err(Error::TrustRoot);
                    }
                } else {
                    crate::create_private_config(&path, bytes)?;
                }
                profile.trust = TrustRoot::CertificateArtifact {
                    sha256: digest.clone(),
                };
                changed = true;
            }
            profiles.insert(name.clone(), profile);
        }
        if changed {
            crate::registry::save_registry(&self.root, &profiles)?;
        }
        guard.inspect(&self.root)
    }
}
