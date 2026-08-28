//! Signed, expiring outcomes from destructive hosted FOKS compatibility canaries.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;
const SIGNATURE_DOMAIN: &[u8] = b"foks-hosted-compat-canary-v1\0";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Compatible,
    Drift,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanaryArtifact {
    pub schema_version: u32,
    pub target: String,
    pub run_id: String,
    pub generated_at: u64,
    pub expires_at: u64,
    pub protocol_metadata_sha256: String,
    pub mutation_digest: String,
    pub read_digest: String,
    pub outcome: Outcome,
    pub capabilities: BTreeSet<String>,
    pub drift_reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCanaryArtifact {
    pub artifact: CanaryArtifact,
    pub key_id: String,
    pub signature: String,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid canary artifact: {0}")]
    Invalid(&'static str),
    #[error("invalid hexadecimal value")]
    Hex,
    #[error("invalid Ed25519 key or signature")]
    Signature,
    #[error("canary artifact signature did not verify")]
    Verification,
    #[error("canary artifact JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl CanaryArtifact {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION
            || self.target.is_empty()
            || self.target.len() > 512
            || self.run_id.is_empty()
            || self.run_id.len() > 128
            || self.generated_at >= self.expires_at
            || self.expires_at - self.generated_at > 7 * 24 * 60 * 60
            || !is_digest(&self.protocol_metadata_sha256)
            || !is_digest(&self.mutation_digest)
            || !is_digest(&self.read_digest)
        {
            return Err(Error::Invalid("identity, lifetime, or digest is malformed"));
        }
        if self.capabilities.iter().any(|capability| {
            capability.is_empty()
                || capability.len() > 64
                || !capability
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
        }) {
            return Err(Error::Invalid("capability is malformed"));
        }
        match self.outcome {
            Outcome::Compatible
                if self.capabilities.is_empty() || !self.drift_reason.is_empty() =>
            {
                Err(Error::Invalid("compatible outcome is incomplete"))
            }
            Outcome::Drift
                if !self.capabilities.is_empty()
                    || self.drift_reason.is_empty()
                    || self.drift_reason.len() > 512 =>
            {
                Err(Error::Invalid("drift outcome must revoke every capability"))
            }
            _ => Ok(()),
        }
    }

    pub fn digest_hex(&self) -> Result<String> {
        Ok(hex(&Sha256::digest(signing_bytes(self)?)))
    }
}

impl SignedCanaryArtifact {
    pub fn sign(artifact: CanaryArtifact, signing_key: &[u8; 32]) -> Result<Self> {
        artifact.validate()?;
        let signing_key = SigningKey::from_bytes(signing_key);
        let signature = signing_key.sign(&signing_bytes(&artifact)?);
        let verifying_key = signing_key.verifying_key();
        Ok(Self {
            artifact,
            key_id: hex(&Sha256::digest(verifying_key.as_bytes()))[..16].to_owned(),
            signature: hex(&signature.to_bytes()),
        })
    }

    pub fn verify(&self, public_key: &[u8; 32]) -> Result<()> {
        self.artifact.validate()?;
        let verifying_key = VerifyingKey::from_bytes(public_key).map_err(|_| Error::Signature)?;
        let expected_key_id = &hex(&Sha256::digest(public_key))[..16];
        if self.key_id != expected_key_id {
            return Err(Error::Verification);
        }
        let signature_bytes: [u8; 64] = decode_hex(&self.signature)?
            .try_into()
            .map_err(|_| Error::Signature)?;
        let signature = Signature::from_bytes(&signature_bytes);
        verifying_key
            .verify(&signing_bytes(&self.artifact)?, &signature)
            .map_err(|_| Error::Verification)
    }
}

pub fn decode_public_key(input: &str) -> Result<[u8; 32]> {
    decode_hex(input.trim())?
        .try_into()
        .map_err(|_| Error::Signature)
}

fn signing_bytes(artifact: &CanaryArtifact) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(artifact)?;
    let mut bytes = Vec::with_capacity(SIGNATURE_DOMAIN.len() + json.len());
    bytes.extend_from_slice(SIGNATURE_DOMAIN);
    bytes.extend_from_slice(&json);
    Ok(bytes)
}

fn is_digest(input: &str) -> bool {
    input.len() == 64 && input.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn decode_hex(input: &str) -> Result<Vec<u8>> {
    let pairs = input.as_bytes().chunks_exact(2);
    if !pairs.remainder().is_empty() {
        return Err(Error::Hex);
    }
    pairs
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| Error::Hex)?;
            u8::from_str_radix(text, 16).map_err(|_| Error::Hex)
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
        output
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(outcome: Outcome) -> CanaryArtifact {
        CanaryArtifact {
            schema_version: SCHEMA_VERSION,
            target: "foks.pub:443".to_owned(),
            run_id: "run-42".to_owned(),
            generated_at: 100,
            expires_at: 200,
            protocol_metadata_sha256: "11".repeat(32),
            mutation_digest: "22".repeat(32),
            read_digest: "33".repeat(32),
            outcome,
            capabilities: if outcome == Outcome::Compatible {
                BTreeSet::from(["kv".to_owned(), "user-sync".to_owned()])
            } else {
                BTreeSet::new()
            },
            drift_reason: if outcome == Outcome::Drift {
                "read-back mismatch".to_owned()
            } else {
                String::new()
            },
        }
    }

    #[test]
    fn signature_binds_every_capability_and_outcome() {
        let key = [7; 32];
        let signed = SignedCanaryArtifact::sign(artifact(Outcome::Compatible), &key).unwrap();
        signed
            .verify(SigningKey::from_bytes(&key).verifying_key().as_bytes())
            .unwrap();
        let mut tampered = signed;
        tampered.artifact.capabilities.insert("teams".to_owned());
        assert!(matches!(
            tampered.verify(SigningKey::from_bytes(&key).verifying_key().as_bytes()),
            Err(Error::Verification)
        ));
    }

    #[test]
    fn drift_artifacts_cannot_grant_capabilities() {
        let mut drift = artifact(Outcome::Drift);
        drift.capabilities.insert("kv".to_owned());
        assert!(drift.validate().is_err());
    }
}
