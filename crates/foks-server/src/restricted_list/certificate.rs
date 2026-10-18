//! Bounded extraction only. TLS possession and current SQL admission are separate.
use std::{mem::size_of, sync::Arc};

use x509_parser::{
    certificate::X509CertificateParser,
    extensions::X509Extension,
    nom::Parser as _,
    x509::{AttributeTypeAndValue, RelativeDistinguishedName},
};

use crate::net::memory::Request;

pub(in crate::restricted_storage) const MAX_DER: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::restricted_storage) enum CertificateError {
    Bounds,
    Malformed,
    Key,
    Serial,
    Expiry,
    Admission,
}

/// Fixed parsed facts, never a TLS or authentication capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::restricted_storage) struct CertificateFields {
    serial: [u8; 20],
    serial_len: u8,
    pub(in crate::restricted_storage) public_key: [u8; 32],
    pub(in crate::restricted_storage) not_after_seconds: i64,
}
impl CertificateFields {
    pub(in crate::restricted_storage) fn serial(&self) -> &[u8] {
        &self.serial[..usize::from(self.serial_len)]
    }

    // Native Principal::authenticate performs this conversion after its SQL
    // binding, public-key and credential-kind checks. Import does not call it.
    pub(super) fn native_expiry_micros(&self) -> Result<u64, CertificateError> {
        u64::try_from(self.not_after_seconds)
            .ok()
            .and_then(|seconds| seconds.checked_mul(1_000_000))
            .ok_or(CertificateError::Expiry)
    }
}

/// Source-derived cumulative requested-allocation ceiling for the pinned shallow
/// parser. Includes failed partial vectors, realloc overlap and BigUint shrink.
pub(in crate::restricted_storage) fn allowance(length: usize) -> Option<usize> {
    if length == 0 || length > MAX_DER {
        return None;
    }
    let attributes = length / 6;
    let names = length / 8;
    let extensions = length / 6;
    let rounded = length.checked_add(7)? & !7;
    attributes
        .checked_mul(4)?
        .checked_mul(size_of::<AttributeTypeAndValue<'static>>())?
        .checked_add(
            names
                .checked_add(2)?
                .checked_mul(4)?
                .checked_mul(size_of::<RelativeDistinguishedName<'static>>())?,
        )?
        .checked_add(
            extensions
                .checked_add(1)?
                .checked_mul(4)?
                .checked_mul(size_of::<X509Extension<'static>>())?,
        )?
        .checked_add(length)?
        .checked_add(rounded.checked_mul(2)?)?
        .checked_add(2 * 53)
}

/// One physically retained workspace. `&mut self` prevents overlapping parser
/// trees; parse returns only fixed facts after dropping the complete X509 value.
pub(in crate::restricted_storage) struct CertificateWorkspace {
    owner: Arc<Request>,
    maximum_der: usize,
}
impl CertificateWorkspace {
    pub(in crate::restricted_storage) fn new(
        owner: Arc<Request>,
        maximum_der: usize,
    ) -> foks_metadata_core::Result<Self> {
        let bytes = allowance(maximum_der).ok_or(foks_metadata_core::Error::Bounds)?;
        if owner.failed() || !foks_snowpack::AllocationBudget::reserve(owner.as_ref(), bytes) {
            return Err(foks_metadata_core::Error::Bounds);
        }
        Ok(Self { owner, maximum_der })
    }

    pub(in crate::restricted_storage) fn parse(
        &mut self,
        der: &[u8],
    ) -> Result<CertificateFields, CertificateError> {
        if self.owner.failed() {
            return Err(CertificateError::Admission);
        }
        if der.is_empty() || der.len() > self.maximum_der {
            return Err(CertificateError::Bounds);
        }
        let mut parser = X509CertificateParser::new().with_deep_parse_extensions(false);
        let (remaining, parsed) = parser.parse(der).map_err(|_| CertificateError::Malformed)?;
        if !remaining.is_empty() {
            return Err(CertificateError::Malformed);
        }
        let public = parsed.public_key();
        // This is the same no-bigint iterator used by native to_id_string(),
        // including nonminimal arcs. Do not format attacker-controlled OIDs.
        if !public
            .algorithm
            .algorithm
            .iter()
            .is_some_and(|arcs| arcs.eq([1, 3, 101, 112]))
            || public.subject_public_key.unused_bits != 0
            || public.subject_public_key.data.len() != 32
        {
            return Err(CertificateError::Key);
        }
        let raw = parsed.raw_serial();
        if raw.is_empty() || raw[0] & 0x80 != 0 {
            return Err(CertificateError::Serial);
        }
        let first = raw
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(raw.len() - 1);
        let magnitude = &raw[first..];
        if magnitude.len() > 20 {
            return Err(CertificateError::Serial);
        }
        let mut serial = [0; 20];
        serial[..magnitude.len()].copy_from_slice(magnitude);
        let result = CertificateFields {
            serial,
            serial_len: magnitude.len() as u8,
            public_key: public
                .subject_public_key
                .data
                .as_ref()
                .try_into()
                .map_err(|_| CertificateError::Key)?,
            not_after_seconds: parsed.validity().not_after.timestamp(),
        };
        drop(parsed);
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
