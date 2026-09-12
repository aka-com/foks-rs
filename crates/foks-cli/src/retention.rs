use foks_client_app::{ProfileRegistry, ProfileSession};
use std::path::Path;

#[derive(clap::Subcommand)]
pub enum RetentionCommand {
    /// Read redacted resident maintenance counters.
    Status,
    /// Preview or explicitly re-anchor local adapter time without pruning or writing remotely.
    Reanchor {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        account: String,
        #[arg(long)]
        unix_seconds: u64,
        /// Exact digest printed by the preview for these time values.
        #[arg(long)]
        confirm_digest: Option<String>,
    },
}

pub fn run(state: &Path, command: RetentionCommand) -> Result<(), Box<dyn std::error::Error>> {
    let RetentionCommand::Reanchor {
        profile,
        account,
        unix_seconds,
        confirm_digest,
    } = command
    else {
        let reply = foks_agent_client::AgentClient::new(state.join("foks-rs.sock"))
            .call(foks_agent_proto::Operation::RetentionStatus)?;
        return match reply.result {
            foks_agent_proto::ResponseResult::Success { value } => {
                println!("{}", serde_json::to_string_pretty(&value)?);
                Ok(())
            }
            foks_agent_proto::ResponseResult::Error { code, message, .. } => {
                Err(format!("{code:?}: {message}").into())
            }
        };
    };
    let registry = ProfileRegistry::open(state)?;
    let session = ProfileSession::open(&registry, &profile)?;
    // Output follows checkpoint publication, including error paths.
    let result = super::with_vault(state, &session, |checked, vault, _| {
        Ok(match confirm_digest {
            Some(ref digest) => serde_json::to_value(checked.reanchor_adapter_clock(
                &account,
                unix_seconds,
                digest,
                vault,
            )?)?,
            None => serde_json::to_value(checked.adapter_clock_preview(
                &account,
                unix_seconds,
                vault,
            )?)?,
        })
    })?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
