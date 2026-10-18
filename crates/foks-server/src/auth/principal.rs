use rustls_pki_types::CertificateDer;
use x509_parser::prelude::parse_x509_certificate;

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredentialKind {
    SoftwareDevice,
    DelegatedSubkey,
    BackupKey,
    BotToken,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Principal {
    uid: Vec<u8>,
    credential_id: Vec<u8>,
    kind: CredentialKind,
    certificate_expires_at: u64,
}

impl Principal {
    pub(crate) fn authenticate(
        certificate: &CertificateDer<'_>,
        database: &foks_server_db::ReadDatabase,
        now: u64,
    ) -> Result<Self> {
        let public_key = certificate_public_key(certificate)?;
        let binding = database
            .credential_for_certificate(certificate.as_ref(), now)?
            .ok_or(Error::Config(
                "authenticated peer certificate is not bound to an active credential",
            ))?;
        let entity = foks_proto::EntityId::from_bytes(binding.credential_id.clone())?;
        if entity.ed25519_key()? != public_key {
            return Err(Error::Config(
                "authenticated peer certificate public key does not match its credential",
            ));
        }
        let kind = match entity.entity_type() {
            foks_proto::ENTITY_DEVICE => CredentialKind::SoftwareDevice,
            foks_proto::ENTITY_SUBKEY => CredentialKind::DelegatedSubkey,
            foks_proto::ENTITY_BACKUP_KEY => CredentialKind::BackupKey,
            foks_proto::ENTITY_BOT_TOKEN_KEY => CredentialKind::BotToken,
            _ => {
                return Err(Error::Config(
                    "authenticated peer certificate has an unsupported credential kind",
                ));
            }
        };
        let (_, parsed) = parse_x509_certificate(certificate.as_ref())
            .map_err(|_| Error::Config("invalid certificate"))?;
        let certificate_expires_at = u64::try_from(parsed.validity().not_after.timestamp())
            .ok()
            .and_then(|seconds| seconds.checked_mul(1_000_000))
            .ok_or(Error::Config("certificate expiry out of range"))?;
        Ok(Self {
            certificate_expires_at,
            uid: binding.uid,
            credential_id: binding.credential_id,
            kind,
        })
    }

    pub(crate) fn certificate_expires_at(&self) -> u64 {
        self.certificate_expires_at
    }

    pub(crate) fn uid(&self) -> &[u8] {
        &self.uid
    }

    pub(crate) fn device_id(&self) -> &[u8] {
        &self.credential_id
    }

    pub(crate) fn require_interactive_device(
        &self,
    ) -> std::result::Result<(), foks_rpc::RpcStatus> {
        if self.kind == CredentialKind::BotToken {
            return Err(foks_rpc::RpcStatus::PermissionDenied(
                "interactive device authentication required".into(),
            ));
        }
        self.require_ordinary_device()
    }

    pub(crate) fn require_ordinary_device(&self) -> std::result::Result<(), foks_rpc::RpcStatus> {
        if matches!(
            self.kind,
            CredentialKind::SoftwareDevice
                | CredentialKind::DelegatedSubkey
                | CredentialKind::BotToken
        ) {
            Ok(())
        } else {
            Err(foks_rpc::RpcStatus::PermissionDenied(
                "credential is recovery-only".to_owned(),
            ))
        }
    }
}

fn certificate_public_key(certificate: &CertificateDer<'_>) -> Result<[u8; 32]> {
    let (_, parsed) = parse_x509_certificate(certificate.as_ref())
        .map_err(|_| Error::Config("authenticated peer certificate is malformed"))?;
    let public = parsed.public_key();
    if public.algorithm.algorithm.to_id_string() != "1.3.101.112"
        || public.subject_public_key.unused_bits != 0
        || public.subject_public_key.data.len() != 32
    {
        return Err(Error::Config(
            "authenticated peer certificate is not an Ed25519 device",
        ));
    }
    public
        .subject_public_key
        .data
        .as_ref()
        .try_into()
        .map_err(|_| Error::Config("authenticated peer certificate public key has the wrong width"))
}

/// Parse the bounded hosted profile's certificate context after the frontend's
/// TLS verifier has established possession. This helper does not authenticate
/// the certificate against current database state or verify its signature.
#[cfg(any(test, feature = "celld-proof"))]
pub(crate) fn parse_hosted_peer_certificate(
    certificate: &CertificateDer<'_>,
) -> Result<foks_metadata_core::personal::PeerCertificate> {
    use foks_metadata_core::personal::{PeerCertificate, MAX_PEER_CERTIFICATE_BYTES};

    let der = certificate.as_ref();
    if der.is_empty() || der.len() > MAX_PEER_CERTIFICATE_BYTES {
        return Err(Error::Config(
            "hosted peer certificate exceeds profile bounds",
        ));
    }
    let (remaining, parsed) = parse_x509_certificate(der)
        .map_err(|_| Error::Config("hosted peer certificate is malformed"))?;
    if !remaining.is_empty() {
        return Err(Error::Config("hosted peer certificate has trailing bytes"));
    }
    let public = parsed.public_key();
    if public.algorithm.algorithm.to_id_string() != "1.3.101.112"
        || public.subject_public_key.unused_bits != 0
        || public.subject_public_key.data.len() != 32
    {
        return Err(Error::Config(
            "hosted peer certificate is not an Ed25519 device",
        ));
    }
    let key = public
        .subject_public_key
        .data
        .as_ref()
        .try_into()
        .map_err(|_| Error::Config("hosted peer certificate public key width"))?;
    let serial = hosted_serial_magnitude(parsed.raw_serial())?;
    let expiry = u64::try_from(parsed.validity().not_after.timestamp())
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000_000))
        .ok_or(Error::Config("hosted peer certificate expiry out of range"))?;
    PeerCertificate::from_frontend_parts(der.to_vec(), serial.to_vec(), key, expiry)
        .map_err(|_| Error::Config("hosted peer certificate context exceeds profile bounds"))
}

#[cfg(any(test, feature = "celld-proof"))]
fn hosted_serial_magnitude(serial: &[u8]) -> Result<&[u8]> {
    // x509-parser deliberately accepts negative ASN.1 integers as unsigned.
    // The hosted profile accepts positive magnitudes only, including DER's
    // leading sign zero. Preserve the single-byte representation of zero.
    if serial.is_empty() || serial[0] & 0x80 != 0 {
        return Err(Error::Config(
            "hosted peer certificate serial is not unsigned",
        ));
    }
    let first = serial
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(serial.len() - 1);
    let magnitude = &serial[first..];
    if magnitude.len() > 20 {
        return Err(Error::Config(
            "hosted peer certificate serial exceeds profile bounds",
        ));
    }
    Ok(magnitude)
}

#[cfg(test)]
#[path = "principal/hosted_tests.rs"]
mod hosted_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_certificate_is_rejected() {
        assert!(certificate_public_key(&CertificateDer::from(vec![1, 2, 3])).is_err());
    }
}
