use foks_agent_proto::{
    invitations::{InvitationAction, InvitationRole},
    Operation, ResponseResult, SecretString,
};
use std::path::{Path, PathBuf};
#[derive(clap::Args)]
pub struct Scope {
    #[arg(long)]
    profile: String,
    #[arg(long)]
    account: String,
    #[arg(long)]
    pin_file: Option<PathBuf>,
}
#[derive(clap::Subcommand)]
pub enum InvitationCommand {
    Preview {
        #[command(flatten)]
        scope: Scope,
        invite: String,
    },
    /// Prepare a certificate upload. Attempt the returned operation to publish it.
    Create {
        #[command(flatten)]
        scope: Scope,
        team: String,
    },
    /// Prepare a join request. Attempt the returned operation to deliver it once.
    Accept {
        #[command(flatten)]
        scope: Scope,
        invite: String,
    },
    Attempt {
        #[command(flatten)]
        scope: Scope,
        operation: String,
    },
    Status {
        #[command(flatten)]
        scope: Scope,
        operation: String,
    },
    Cancel {
        #[command(flatten)]
        scope: Scope,
        operation: String,
    },
    List {
        #[command(flatten)]
        scope: Scope,
    },
    Inbox {
        #[command(flatten)]
        scope: Scope,
        team: String,
    },
    Approve {
        #[command(flatten)]
        scope: Scope,
        team: String,
        request: String,
        #[arg(long,default_value="member",value_parser=["member","admin","owner"])]
        role: String,
        #[arg(long, default_value_t = 0)]
        visibility: i16,
    },
    /// Prepare a rejection; attempt its returned operation to submit it once.
    Reject {
        #[command(flatten)]
        scope: Scope,
        team: String,
        request: String,
    },
}
pub fn run(state: &Path, command: InvitationCommand) -> Result<(), Box<dyn std::error::Error>> {
    let (scope, action) = match command {
        InvitationCommand::Preview { scope, invite } => {
            (scope, InvitationAction::Preview { invite })
        }
        InvitationCommand::Create { scope, team } => {
            (scope, InvitationAction::Create { team_alias: team })
        }
        InvitationCommand::Accept { scope, invite } => (scope, InvitationAction::Accept { invite }),
        InvitationCommand::Attempt { scope, operation } => (
            scope,
            InvitationAction::Attempt {
                operation_id: operation,
            },
        ),
        InvitationCommand::Status { scope, operation } => (
            scope,
            InvitationAction::Status {
                operation_id: operation,
            },
        ),
        InvitationCommand::Cancel { scope, operation } => (
            scope,
            InvitationAction::Cancel {
                operation_id: operation,
            },
        ),
        InvitationCommand::List { scope } => (scope, InvitationAction::List),
        InvitationCommand::Inbox { scope, team } => {
            (scope, InvitationAction::Inbox { team_alias: team })
        }
        InvitationCommand::Approve {
            scope,
            team,
            request,
            role,
            visibility,
        } => (
            scope,
            InvitationAction::Approve {
                team_alias: team,
                request_id: request,
                role: match role.as_str() {
                    "admin" => InvitationRole::Admin,
                    "owner" => InvitationRole::Owner,
                    _ => InvitationRole::Member { visibility },
                },
            },
        ),
        InvitationCommand::Reject {
            scope,
            team,
            request,
        } => (
            scope,
            InvitationAction::Reject {
                team_alias: team,
                request_id: request,
            },
        ),
    };
    if !action.validate() {
        return Err("invalid invitation input".into());
    }
    let pin = scope
        .pin_file
        .as_deref()
        .map(super::read_pin)
        .transpose()?
        .map(|p| SecretString::new(p.expose()));
    super::mcp::ensure_agent(state)?;
    let response = foks_agent_client::AgentClient::new(state.join("foks-rs.sock")).call(
        Operation::Invitations {
            profile: scope.profile,
            account_alias: scope.account,
            action,
            pin,
        },
    )?;
    match response.result {
        ResponseResult::Success { value } => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        ResponseResult::Error { message, .. } => Err(message.into()),
    }
}
