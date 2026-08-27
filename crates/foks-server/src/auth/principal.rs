use rustls_pki_types::CertificateDer;
use x509_parser::prelude::parse_x509_certificate;

use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Principal {
    device_id: [u8; 33],
}

impl Principal {
    pub(crate) fn from_certificate(certificate: &CertificateDer<'_>) -> Result<Self> {
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
        let mut device_id = [0; 33];
        device_id[0] = foks_proto::ENTITY_DEVICE;
        device_id[1..].copy_from_slice(public.subject_public_key.data.as_ref());
        Ok(Self { device_id })
    }

    pub(crate) fn device_id(&self) -> &[u8; 33] {
        &self.device_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_certificate_is_rejected() {
        assert!(Principal::from_certificate(&CertificateDer::from(vec![1, 2, 3])).is_err());
    }
}
