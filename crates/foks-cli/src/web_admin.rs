use clap::{Args, Subcommand};
use foks_agent_proto::{
    admin::{AdminAction, AdminNavigation},
    Operation, ResponseResult,
};
use std::path::{Path, PathBuf};
#[derive(Args)]
pub struct Scope {
    #[arg(long)]
    profile: String,
    #[arg(long)]
    account_alias: String,
}
#[derive(Subcommand)]
pub enum AdminCommand {
    /// Configure the HTTPS origin supplied by your host operator; never a session URL.
    Configure {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        destination: String,
    },
    /// Verify hosted administration without printing a session URL. Open it in desktop settings.
    Check {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        pin_file: Option<PathBuf>,
    },
}
pub fn run(state: &Path, command: AdminCommand) -> Result<(), Box<dyn std::error::Error>> {
    let (scope, action) = match command {
        AdminCommand::Configure { scope, destination } => {
            (scope, AdminAction::Configure { destination })
        }
        AdminCommand::Check { scope, pin_file } => (
            scope,
            AdminAction::Prepare {
                pin: pin_file
                    .as_deref()
                    .map(super::read_pin)
                    .transpose()?
                    .map(|p| foks_agent_proto::SecretString::new(p.expose())),
            },
        ),
    };
    if !action.validate() {
        return Err("invalid admin request".into());
    }
    super::mcp::ensure_agent(state)?;
    let checking = matches!(&action, AdminAction::Prepare { .. });
    let response = foks_agent_client::AgentClient::new(state.join("foks-rs.sock")).call(
        Operation::WebAdmin {
            profile: scope.profile.clone(),
            account_alias: scope.account_alias.clone(),
            action,
        },
    )?;
    match response.result {
        ResponseResult::Error { message, .. } => Err(message.into()),
        ResponseResult::Success { value } => {
            if checking {
                let result: AdminNavigation = serde_json::from_value(value)?;
                if result.profile != scope.profile || result.account_alias != scope.account_alias {
                    return Err("admin handoff changed account".into());
                }
            }
            println!(
                "{}",
                serde_json::json!({"configured":!checking,"verified":checking})
            );
            Ok(())
        }
    }
}
