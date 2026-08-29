#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::path::PathBuf;

use clap::Parser as _;
use ed25519_dalek::SigningKey;
use foks_compat_artifact::{CanaryArtifact, Outcome, SignedCanaryArtifact, SCHEMA_VERSION};
use zeroize::{Zeroize as _, Zeroizing};

const MAXIMUM_ARTIFACT_BYTES: u64 = 1024 * 1024;

#[derive(clap::Parser)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    Sign(Sign),
    Verify(Verify),
    /// Allocates a generation above the authenticated stable artifact.
    AllocateGeneration(AllocateGeneration),
}

#[derive(clap::Args)]
struct Sign {
    #[arg(long)]
    generation: u64,
    #[arg(long)]
    target: String,
    #[arg(long)]
    run_id: String,
    #[arg(long)]
    generated_at: u64,
    #[arg(long)]
    expires_at: u64,
    #[arg(long)]
    protocol_digest: String,
    #[arg(long)]
    mutation_digest: String,
    #[arg(long)]
    read_digest: String,
    #[arg(long, value_enum)]
    outcome: OutcomeArgument,
    #[arg(long)]
    capability: Vec<String>,
    #[arg(long, default_value = "")]
    drift_reason: String,
    #[arg(long)]
    signing_key_file: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum OutcomeArgument {
    Compatible,
    Drift,
}

#[derive(clap::Args)]
struct Verify {
    #[arg(long)]
    artifact: PathBuf,
    #[arg(long)]
    public_key_hex: String,
}

#[derive(clap::Args)]
struct AllocateGeneration {
    /// Previously published artifact. Omit only for the first publication.
    #[arg(long)]
    previous_artifact: Option<PathBuf>,
    #[arg(long)]
    target: String,
    #[arg(long)]
    run_number: u64,
    #[arg(long)]
    run_attempt: u64,
    #[arg(long)]
    signing_key_file: PathBuf,
}

fn main() {
    if let Err(error) = run(Arguments::parse()) {
        eprintln!("foks-compat-artifact: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> Result<(), Box<dyn std::error::Error>> {
    match arguments.command {
        Command::Sign(arguments) => {
            let mut key = Zeroizing::new(read_private_key(&arguments.signing_key_file)?);
            if key.len() != 32 {
                return Err("signing key file must contain exactly 32 raw bytes".into());
            }
            let mut seed = Zeroizing::new([0u8; 32]);
            seed.copy_from_slice(&key);
            key.zeroize();
            let artifact = CanaryArtifact {
                schema_version: SCHEMA_VERSION,
                generation: arguments.generation,
                target: arguments.target,
                run_id: arguments.run_id,
                generated_at: arguments.generated_at,
                expires_at: arguments.expires_at,
                protocol_metadata_sha256: arguments.protocol_digest,
                mutation_digest: arguments.mutation_digest,
                read_digest: arguments.read_digest,
                outcome: match arguments.outcome {
                    OutcomeArgument::Compatible => Outcome::Compatible,
                    OutcomeArgument::Drift => Outcome::Drift,
                },
                capabilities: arguments.capability.into_iter().collect::<BTreeSet<_>>(),
                drift_reason: arguments.drift_reason,
            };
            let signed = SignedCanaryArtifact::sign(artifact, &seed)?;
            write_new(&arguments.output, &serde_json::to_vec_pretty(&signed)?)?;
        }
        Command::Verify(arguments) => {
            let signed: SignedCanaryArtifact =
                serde_json::from_slice(&read_bounded_artifact(&arguments.artifact)?)?;
            signed.verify(&foks_compat_artifact::decode_public_key(
                &arguments.public_key_hex,
            )?)?;
            println!("{}", signed.artifact.digest_hex()?);
        }
        Command::AllocateGeneration(arguments) => {
            let mut key = Zeroizing::new(read_private_key(&arguments.signing_key_file)?);
            if key.len() != 32 {
                return Err("signing key file must contain exactly 32 raw bytes".into());
            }
            let mut seed = Zeroizing::new([0u8; 32]);
            seed.copy_from_slice(&key);
            key.zeroize();
            let previous = arguments
                .previous_artifact
                .as_deref()
                .map(read_bounded_artifact)
                .transpose()?
                .map(|bytes| serde_json::from_slice::<SignedCanaryArtifact>(&bytes))
                .transpose()?;
            println!(
                "{}",
                allocate_generation(
                    previous.as_ref(),
                    &seed,
                    &arguments.target,
                    arguments.run_number,
                    arguments.run_attempt,
                )?
            );
        }
    }
    Ok(())
}

fn allocate_generation(
    previous: Option<&SignedCanaryArtifact>,
    signing_seed: &[u8; 32],
    target: &str,
    run_number: u64,
    run_attempt: u64,
) -> Result<u64, Box<dyn std::error::Error>> {
    if run_number == 0 || run_attempt == 0 || run_attempt >= 1000 {
        return Err("run number and attempt must use GitHub's positive attempt allocation".into());
    }
    let current = match previous {
        Some(previous) => {
            previous.verify(
                SigningKey::from_bytes(signing_seed)
                    .verifying_key()
                    .as_bytes(),
            )?;
            if previous.artifact.target != target {
                return Err("previous artifact targets a different profile".into());
            }
            previous.artifact.generation
        }
        None => 0,
    };
    let allocation = run_number
        .checked_mul(1000)
        .and_then(|value| value.checked_add(run_attempt))
        .ok_or("workflow generation allocation overflowed")?;
    current
        .checked_add(allocation)
        .ok_or_else(|| "canary generation overflowed".into())
}

fn read_bounded_artifact(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAXIMUM_ARTIFACT_BYTES {
        return Err("artifact is not a bounded regular file".into());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(MAXIMUM_ARTIFACT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len())? > MAXIMUM_ARTIFACT_BYTES {
        return Err("artifact changed size while being read".into());
    }
    Ok(bytes)
}

fn read_private_key(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != 32 {
        return Err("signing key must be a 32-byte regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("signing key permissions allow group or other access".into());
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut bytes = Vec::with_capacity(32);
    options.open(path)?.take(33).read_to_end(&mut bytes)?;
    if bytes.len() != 32 {
        return Err("signing key changed size while being read".into());
    }
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn previous(signing_seed: &[u8; 32], generation: u64) -> SignedCanaryArtifact {
        SignedCanaryArtifact::sign(
            CanaryArtifact {
                schema_version: SCHEMA_VERSION,
                generation,
                target: "foks.pub:443".to_owned(),
                run_id: "prior-run".to_owned(),
                generated_at: 100,
                expires_at: 200,
                protocol_metadata_sha256: "11".repeat(32),
                mutation_digest: "22".repeat(32),
                read_digest: "33".repeat(32),
                outcome: Outcome::Compatible,
                capabilities: BTreeSet::from(["kv".to_owned()]),
                drift_reason: String::new(),
            },
            signing_seed,
        )
        .unwrap()
    }

    #[test]
    fn generation_is_above_the_authenticated_stable_artifact() {
        let seed = [7; 32];
        let previous = previous(&seed, 50_000);
        assert_eq!(
            allocate_generation(Some(&previous), &seed, "foks.pub:443", 17, 2).unwrap(),
            67_002
        );
        assert_eq!(
            allocate_generation(Some(&previous), &seed, "foks.pub:443", 17, 3).unwrap(),
            67_003
        );
        assert_eq!(
            allocate_generation(None, &seed, "foks.pub:443", 17, 2).unwrap(),
            17_002
        );
    }

    #[test]
    fn generation_rejects_untrusted_history_and_unsafe_allocations() {
        let seed = [7; 32];
        let previous = previous(&seed, 50_000);
        assert!(allocate_generation(Some(&previous), &[8; 32], "foks.pub:443", 17, 2).is_err());
        assert!(allocate_generation(Some(&previous), &seed, "other:443", 17, 2).is_err());
        assert!(allocate_generation(Some(&previous), &seed, "foks.pub:443", 17, 0).is_err());
        assert!(allocate_generation(Some(&previous), &seed, "foks.pub:443", 17, 1000).is_err());
        assert!(allocate_generation(Some(&previous), &seed, "foks.pub:443", u64::MAX, 1,).is_err());
    }
}
