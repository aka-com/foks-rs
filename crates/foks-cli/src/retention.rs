use foks_agent_proto::{data::AdapterClockRepairAction, Operation, ResponseResult};
use std::path::Path;

#[derive(clap::Subcommand)]
pub enum RetentionCommand {
    /// Read redacted resident maintenance counters.
    Status,
    /// Report whether the account's adapter clock admits new MCP writes.
    ClockStatus {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        account_alias: String,
    },
    /// Preview or explicitly re-anchor local adapter time at this computer's
    /// current time, without pruning or writing remotely.
    Reanchor {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        account_alias: String,
        /// The time to confirm; the agent refuses one more than five minutes
        /// from its own clock. Defaults to now for a preview.
        #[arg(long)]
        unix_seconds: Option<u64>,
        /// Exact digest printed by the preview, with the preview's
        /// `proposed_wall` as `--unix-seconds`.
        #[arg(long, requires = "unix_seconds")]
        confirm_digest: Option<String>,
    },
}

/// Every retention command runs in the resident agent, which samples the same
/// adapter clock that later admits writes.
pub fn run(state: &Path, command: RetentionCommand) -> Result<(), Box<dyn std::error::Error>> {
    let operation = match command {
        RetentionCommand::Status => Operation::RetentionStatus,
        RetentionCommand::ClockStatus {
            profile,
            account_alias,
        } => {
            super::mcp::ensure_agent(state)?;
            Operation::AdapterClockStatus {
                profile,
                account_alias,
            }
        }
        RetentionCommand::Reanchor {
            profile,
            account_alias,
            unix_seconds,
            confirm_digest,
        } => {
            let unix_seconds = match unix_seconds {
                Some(seconds) => seconds,
                None => std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
            };
            super::mcp::ensure_agent(state)?;
            Operation::RepairAdapterClock {
                profile,
                account_alias,
                action: match confirm_digest {
                    Some(confirmation_digest) => AdapterClockRepairAction::Confirm {
                        unix_seconds,
                        confirmation_digest,
                    },
                    None => AdapterClockRepairAction::Preview { unix_seconds },
                },
            }
        }
    };
    let reply = foks_agent_client::AgentClient::new(state.join("foks-rs.sock")).call(operation)?;
    match reply.result {
        ResponseResult::Success { value } => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        ResponseResult::Error { code, message, .. } => Err(format!("{code:?}: {message}").into()),
    }
}
