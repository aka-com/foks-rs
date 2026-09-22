//! Local FOKS adapter submission identities; these are not Go FOKS wire mutation IDs.
use std::{fmt, str::FromStr};

/// A canonical issuance time and 128 random bits. Generic mutation IDs remain
/// separately generated: neither this identity nor its digest may be truncated.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubmissionHandle {
    issued_at: u64,
    random: [u8; 16],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("submission handle must be v1-<16 lowercase hex seconds>-<32 lowercase random hex>")]
pub struct InvalidSubmissionHandle;

impl SubmissionHandle {
    pub const ENCODED_LEN: usize = 52;

    pub fn new(issued_at: u64, random: [u8; 16]) -> Self {
        Self { issued_at, random }
    }
    pub fn issued_at(self) -> u64 {
        self.issued_at
    }
}

impl fmt::Display for SubmissionHandle {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "v1-{:016x}-", self.issued_at)?;
        for byte in self.random {
            write!(out, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for SubmissionHandle {
    type Err = InvalidSubmissionHandle;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let bytes = input.as_bytes();
        if bytes.len() != Self::ENCODED_LEN
            || &bytes[..3] != b"v1-"
            || bytes[19] != b'-'
            || !bytes[3..19]
                .iter()
                .chain(&bytes[20..])
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(InvalidSubmissionHandle);
        }
        // The preceding ASCII check makes these UTF-8 boundaries safe.
        let issued_at =
            u64::from_str_radix(&input[3..19], 16).map_err(|_| InvalidSubmissionHandle)?;
        let mut random = [0; 16];
        for (index, byte) in random.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&input[20 + index * 2..22 + index * 2], 16)
                .map_err(|_| InvalidSubmissionHandle)?;
        }
        Ok(Self { issued_at, random })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_round_trip_covers_full_timestamp_domain() {
        for seconds in [0, u32::MAX as u64 + 1, u64::MAX] {
            let handle = SubmissionHandle::new(seconds, [0xab; 16]);
            assert_eq!(handle.to_string().parse(), Ok(handle));
            assert_eq!(handle.to_string().len(), SubmissionHandle::ENCODED_LEN);
        }
    }
    #[test]
    fn malformed_legacy_and_noncanonical_handles_are_rejected() {
        let valid = SubmissionHandle::new(42, [0xab; 16]).to_string();
        for input in [
            String::new(),
            "ab".repeat(16),
            valid.to_uppercase(),
            valid.replace("v1", "v2"),
            valid[1..].into(),
            format!("{valid}0"),
            valid.replace("ab", "é"),
        ] {
            assert!(input.parse::<SubmissionHandle>().is_err(), "{input}");
        }
    }
}
