//! Actual TLS/native restricted List qualification. Ordinary routes stay unchanged.
mod bootstrap;
pub(in crate::restricted_storage) mod certificate;
mod connection;
mod operation;
mod output;
mod profile;
#[cfg(test)]
mod tests;

use crate::net::memory::{Pools, Request};
#[cfg(feature = "celld-proof")]
use certificate::{CertificateError, CertificateWorkspace};
use std::sync::Arc;

pub use output::start_native_personal_list_qualification;
#[cfg(feature = "celld-proof")]
pub use output::{NativeListQualification, NativeListQualificationReport};

/// Pure parser allocation qualification, not a certificate-authentication proof.
/// Construct outside the measured parser interval; it retains one admitted
/// workspace and exposes neither TLS nor SQL admission constructors.
#[cfg(feature = "celld-proof")]
pub struct CertificateParserQualification {
    workspace: CertificateWorkspace,
}
#[cfg(feature = "celld-proof")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CertificateQualification {
    pub public_key: [u8; 32],
    pub serial: [u8; 20],
    pub serial_length: u8,
    pub not_after_seconds: i64,
}
#[cfg(feature = "celld-proof")]
impl CertificateParserQualification {
    pub fn new(maximum_der: usize) -> Result<Self, &'static str> {
        let owner = Pools::new(crate::SessionLimits::default())
            .admitted_request(None)
            .map_err(|_| "certificate allocation admission")?;
        let workspace = CertificateWorkspace::new(owner, maximum_der)
            .map_err(|_| "certificate allocation admission")?;
        Ok(Self { workspace })
    }
    pub fn allowance(maximum_der: usize) -> Option<usize> {
        certificate::allowance(maximum_der)
    }
    pub fn parse(&mut self, der: &[u8]) -> Result<CertificateQualification, &'static str> {
        let fields = self.workspace.parse(der).map_err(|error| match error {
            CertificateError::Bounds => "certificate bounds",
            CertificateError::Malformed => "certificate malformed",
            CertificateError::Key => "certificate key",
            CertificateError::Serial => "certificate serial",
            CertificateError::Expiry => "certificate expiry",
            CertificateError::Admission => "certificate allocation admission",
        })?;
        let mut serial = [0; 20];
        serial[..fields.serial().len()].copy_from_slice(fields.serial());
        Ok(CertificateQualification {
            public_key: fields.public_key,
            serial,
            serial_length: fields.serial().len() as u8,
            not_after_seconds: fields.not_after_seconds,
        })
    }
}

/// One retained original copy, created only from a negotiated TLS stream.
/// Per-operation workers borrow this immutable DER; transient SQL bind copies
/// have separate current-request admission.
struct SessionPeer {
    der: Vec<u8>,
    _memory: Arc<Request>,
}
impl SessionPeer {
    fn from_tls(
        stream: &tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
        pools: &Arc<Pools>,
    ) -> foks_metadata_core::Result<Arc<Self>> {
        let certificates = stream
            .get_ref()
            .1
            .peer_certificates()
            .ok_or(foks_metadata_core::Error::Permission)?;
        let der = certificates
            .first()
            .ok_or(foks_metadata_core::Error::Permission)?
            .as_ref();
        if der.is_empty() || der.len() > certificate::MAX_DER {
            return Err(foks_metadata_core::Error::Bounds);
        }
        let memory = pools.admitted_request(None)?;
        let charge = der
            .len()
            .checked_add(operation::arc_bytes::<Self>())
            .ok_or(foks_metadata_core::Error::Bounds)?;
        if !foks_snowpack::AllocationBudget::reserve(memory.as_ref(), charge) {
            return Err(foks_metadata_core::Error::Bounds);
        }
        let mut copy = Vec::new();
        copy.try_reserve_exact(der.len())
            .map_err(|_| foks_metadata_core::Error::Bounds)?;
        copy.extend_from_slice(der);
        Ok(Arc::new(Self {
            der: copy,
            _memory: memory,
        }))
    }
}
