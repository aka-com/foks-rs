use crate::MemberAccessArgument;
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
    account_alias: String,
    #[arg(long)]
    pin_file: Option<PathBuf>,
}
#[derive(clap::Subcommand)]
pub enum InvitationCommand {
    AcceptTeam {
        #[command(flatten)]
        scope: Scope,
        invite: String,
        source_team: String,
        #[arg(long, default_value="admin", value_parser=["member","admin","owner"])]
        source_role: String,
        #[arg(long, value_enum, default_value_t = MemberAccessArgument::Standard)]
        member_access: MemberAccessArgument,
        #[arg(long)]
        remote_profile: Option<String>,
    },
    Range {
        #[command(flatten)]
        scope: Scope,
        team: String,
        #[arg(long)]
        raise: bool,
    },
    PendingApprovals {
        #[command(flatten)]
        scope: Scope,
        team: String,
    },

    PreviewRemote {
        #[command(flatten)]
        scope: Scope,
        remote_profile: String,
        invite: String,
    },
    AcceptRemote {
        #[command(flatten)]
        scope: Scope,
        remote_profile: String,
        invite: String,
    },
    AttemptRemote {
        #[command(flatten)]
        scope: Scope,
        remote_profile: String,
        operation: String,
    },
    StatusRemote {
        #[command(flatten)]
        scope: Scope,
        remote_profile: String,
        operation: String,
    },
    InspectRemote {
        #[command(flatten)]
        scope: Scope,
        remote_profile: String,
        team: String,
        request: String,
    },
    ApproveRemote {
        #[command(flatten)]
        scope: Scope,
        remote_profile: String,
        team: String,
        request: String,
        #[arg(long, value_enum, default_value_t = MemberAccessArgument::Standard)]
        member_access: MemberAccessArgument,
    },
    SyncRemote {
        #[command(flatten)]
        scope: Scope,
        remote_profile: String,
        team_id: String,
        #[arg(long)]
        source_team: Option<String>,
        #[arg(long, default_value="admin",value_parser=["member","admin","owner"])]
        source_role: String,
        #[arg(long, value_enum, default_value_t = MemberAccessArgument::Standard)]
        member_access: MemberAccessArgument,
    },
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
        #[arg(long, value_enum, default_value_t = MemberAccessArgument::Standard)]
        member_access: MemberAccessArgument,
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
        InvitationCommand::AcceptTeam {
            scope,
            invite,
            source_team,
            source_role,
            member_access,
            remote_profile,
        } => {
            let role = invitation_role(&source_role, member_access)?;
            let action = match remote_profile {
                Some(remote_profile) => InvitationAction::AcceptTeamRemote {
                    remote_profile,
                    invite,
                    source_team_alias: source_team,
                    source_role: role,
                },
                None => InvitationAction::AcceptTeam {
                    invite,
                    source_team_alias: source_team,
                    source_role: role,
                },
            };
            (scope, action)
        }
        InvitationCommand::Range { scope, team, raise } => (
            scope,
            InvitationAction::Range {
                team_alias: team,
                raise,
            },
        ),
        InvitationCommand::PendingApprovals { scope, team } => (
            scope,
            InvitationAction::PendingApprovals { team_alias: team },
        ),

        InvitationCommand::PreviewRemote {
            scope,
            remote_profile,
            invite,
        } => (
            scope,
            InvitationAction::PreviewRemote {
                remote_profile,
                invite,
            },
        ),
        InvitationCommand::AcceptRemote {
            scope,
            remote_profile,
            invite,
        } => (
            scope,
            InvitationAction::AcceptRemote {
                remote_profile,
                invite,
            },
        ),
        InvitationCommand::AttemptRemote {
            scope,
            remote_profile,
            operation,
        } => (
            scope,
            InvitationAction::AttemptRemote {
                remote_profile,
                operation_id: operation,
            },
        ),
        InvitationCommand::StatusRemote {
            scope,
            remote_profile,
            operation,
        } => (
            scope,
            InvitationAction::StatusRemote {
                remote_profile,
                operation_id: operation,
            },
        ),
        InvitationCommand::InspectRemote {
            scope,
            remote_profile,
            team,
            request,
        } => (
            scope,
            InvitationAction::InspectRemote {
                remote_profile,
                team_alias: team,
                request_id: request,
            },
        ),
        InvitationCommand::ApproveRemote {
            scope,
            remote_profile,
            team,
            request,
            member_access,
        } => (
            scope,
            InvitationAction::ApproveRemote {
                remote_profile,
                team_alias: team,
                request_id: request,
                role: InvitationRole::Member {
                    visibility: member_access.visibility(),
                },
            },
        ),
        InvitationCommand::SyncRemote {
            scope,
            remote_profile,
            team_id,
            source_team,
            source_role,
            member_access,
        } => (
            scope,
            InvitationAction::SyncRemote {
                remote_profile,
                team_id,
                source_team_alias: source_team,
                source_role: Some(invitation_role(&source_role, member_access)?),
            },
        ),
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
            member_access,
        } => (
            scope,
            InvitationAction::Approve {
                team_alias: team,
                request_id: request,
                role: invitation_role(&role, member_access)?,
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
            account_alias: scope.account_alias,
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

fn invitation_role(
    role: &str,
    member_access: MemberAccessArgument,
) -> Result<InvitationRole, Box<dyn std::error::Error>> {
    match role {
        "owner" if matches!(member_access, MemberAccessArgument::Standard) => {
            Ok(InvitationRole::Owner)
        }
        "admin" if matches!(member_access, MemberAccessArgument::Standard) => {
            Ok(InvitationRole::Admin)
        }
        "member" => Ok(InvitationRole::Member {
            visibility: member_access.visibility(),
        }),
        _ => Err("--member-access applies only to member roles".into()),
    }
}
