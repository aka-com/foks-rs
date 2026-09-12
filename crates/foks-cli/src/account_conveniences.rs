use foks_agent_client::AgentClient;
use foks_agent_proto::{account::RenameAction, Operation, ResponseResult, SecretString};
use std::path::{Path, PathBuf};
#[derive(clap::Subcommand)]
pub enum RenameCommand {
    /// Recover pending handles and inspect recent receipts.
    List(Scope),
    /// Prepare an immutable rename; returns the operation handle for confirmation.
    Prepare {
        #[command(flatten)]
        scope: Scope,
        username: String,
        #[arg(long)]
        pin_file: Option<PathBuf>,
    },
    /// Submit once, or reconcile the original request if already attempted.
    Attempt {
        #[command(flatten)]
        flow: Flow,
        #[arg(long)]
        pin_file: Option<PathBuf>,
    },
    /// Reconcile without sending a rename request.
    Status {
        #[command(flatten)]
        flow: Flow,
        #[arg(long)]
        pin_file: Option<PathBuf>,
    },
    /// Cancel an unsubmitted rename.
    Cancel(Flow),
}
#[derive(clap::Args)]
pub struct Scope {
    #[arg(long)]
    profile: String,
    #[arg(long)]
    account: String,
}
#[derive(clap::Args)]
pub struct Flow {
    #[command(flatten)]
    scope: Scope,
    #[arg(long)]
    operation: String,
}
pub fn run(state: &Path, command: RenameCommand) -> Result<(), Box<dyn std::error::Error>> {
    let pin = |path: Option<PathBuf>| -> Result<Option<SecretString>, Box<dyn std::error::Error>> {
        path.as_deref()
            .map(super::read_pin)
            .transpose()
            .map(|p| p.map(|p| SecretString::new(p.expose())))
    };
    if let RenameCommand::List(scope) = &command {
        super::mcp::ensure_agent(state)?;
        let response =
            AgentClient::new(state.join("foks-rs.sock")).call(Operation::ListAccountRenames {
                profile: scope.profile.clone(),
                account_alias: scope.account.clone(),
            })?;
        return match response.result {
            ResponseResult::Success { value } => {
                println!("{}", serde_json::to_string_pretty(&value)?);
                Ok(())
            }
            ResponseResult::Error { message, .. } => Err(message.into()),
        };
    }
    let (scope, action) = match command {
        RenameCommand::List(_) => unreachable!(),
        RenameCommand::Prepare {
            scope,
            username,
            pin_file,
        } => (
            scope,
            RenameAction::Prepare {
                username,
                pin: pin(pin_file)?,
            },
        ),
        RenameCommand::Attempt { flow, pin_file } => (
            flow.scope,
            RenameAction::Attempt {
                operation_id: flow.operation,
                pin: pin(pin_file)?,
            },
        ),
        RenameCommand::Status { flow, pin_file } => (
            flow.scope,
            RenameAction::Status {
                operation_id: flow.operation,
                pin: pin(pin_file)?,
            },
        ),
        RenameCommand::Cancel(flow) => (
            flow.scope,
            RenameAction::Cancel {
                operation_id: flow.operation,
            },
        ),
    };
    if !action.validate() {
        return Err("invalid rename action or operation handle".into());
    }
    super::mcp::ensure_agent(state)?;
    let response = AgentClient::new(state.join("foks-rs.sock")).call(Operation::RenameAccount {
        profile: scope.profile,
        account_alias: scope.account,
        action,
    })?;
    match response.result {
        ResponseResult::Success { value } => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        ResponseResult::Error { message, .. } => Err(message.into()),
    }
}
