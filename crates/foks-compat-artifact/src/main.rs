#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::path::PathBuf;

use clap::Parser as _;
use foks_compat_artifact::{CanaryArtifact, Outcome, SignedCanaryArtifact, SCHEMA_VERSION};
use zeroize::{Zeroize as _, Zeroizing};

#[derive(clap::Parser)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    Sign(Sign),
    Verify(Verify),
}

#[derive(clap::Args)]
struct Sign {
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
                serde_json::from_slice(&fs::read(arguments.artifact)?)?;
            signed.verify(&foks_compat_artifact::decode_public_key(
                &arguments.public_key_hex,
            )?)?;
            println!("{}", signed.artifact.digest_hex()?);
        }
    }
    Ok(())
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
