use crate::MemberAccessArgument;
use clap::Subcommand;
use foks_agent_proto::{bot::BotAction, KvRole, Operation, ResponseResult, SecretString};
use std::path::{Path, PathBuf};
use zeroize::{Zeroize as _, Zeroizing};
#[derive(clap::Args)]
pub struct Scope {
    #[arg(long)]
    profile: String,
    #[arg(long)]
    account_alias: String,
}
#[derive(clap::Args)]
pub struct Flow {
    #[command(flatten)]
    scope: Scope,
    #[arg(long)]
    operation: String,
}
#[derive(Subcommand)]
pub enum BotCommand {
    /// Load the original token from a private file, or bounded stdin when omitted.
    Load {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        input: Option<PathBuf>,
    },
    Revoke {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        device_id: String,
        #[arg(long)]
        pin_file: Option<PathBuf>,
    },
    Unload(Scope),
    List(Scope),
    Prepare {
        #[command(flatten)]
        scope: Scope,
        #[arg(long,value_parser=["owner","admin","member"])]
        role: String,
        #[arg(long, value_enum, default_value_t = MemberAccessArgument::Standard)]
        member_access: MemberAccessArgument,
        #[arg(long)]
        pin_file: Option<PathBuf>,
    },
    Attempt {
        #[command(flatten)]
        flow: Flow,
        #[arg(long)]
        pin_file: Option<PathBuf>,
    },
    Status {
        #[command(flatten)]
        flow: Flow,
        #[arg(long)]
        pin_file: Option<PathBuf>,
    },
    Cancel(Flow),
    /// Export once to a newly-created private file; never prints the token.
    Export {
        #[command(flatten)]
        flow: Flow,
        #[arg(long)]
        output: PathBuf,
    },
}
fn token_input(path: Option<&Path>) -> Result<SecretString, Box<dyn std::error::Error>> {
    Ok(match path {
        Some(p) => foks_agent_client::secret_file::import_token(p)?,
        None => foks_agent_client::secret_file::read_token(std::io::stdin())?,
    })
}

pub fn run(state: &Path, command: BotCommand) -> Result<(), Box<dyn std::error::Error>> {
    let pin = |p: Option<PathBuf>| -> Result<Option<SecretString>, Box<dyn std::error::Error>> {
        p.as_deref()
            .map(super::read_pin)
            .transpose()
            .map(|p| p.map(|v| SecretString::new(v.expose())))
    };
    let mut output = None;
    let (scope, action) = match command {
        BotCommand::Load { scope, input } => (
            scope,
            BotAction::Load {
                token: token_input(input.as_deref())?,
            },
        ),
        BotCommand::Revoke {
            scope,
            device_id,
            pin_file,
        } => (
            scope,
            BotAction::Revoke {
                device_id,
                pin: pin(pin_file)?,
            },
        ),
        BotCommand::Unload(scope) => (scope, BotAction::Unload),
        BotCommand::List(scope) => (scope, BotAction::List),
        BotCommand::Prepare {
            scope,
            role,
            member_access,
            pin_file,
        } => {
            let role = match role.as_str() {
                "owner" if matches!(member_access, MemberAccessArgument::Standard) => KvRole::Owner,
                "admin" if matches!(member_access, MemberAccessArgument::Standard) => KvRole::Admin,
                "member" => KvRole::Member {
                    visibility: member_access.visibility(),
                },
                _ => return Err("--member-access applies only to member roles".into()),
            };
            (
                scope,
                BotAction::Prepare {
                    role,
                    pin: pin(pin_file)?,
                },
            )
        }
        BotCommand::Attempt { flow, pin_file } => (
            flow.scope,
            BotAction::Attempt {
                operation_id: flow.operation,
                pin: pin(pin_file)?,
            },
        ),
        BotCommand::Status { flow, pin_file } => (
            flow.scope,
            BotAction::Status {
                operation_id: flow.operation,
                pin: pin(pin_file)?,
            },
        ),
        BotCommand::Cancel(flow) => (
            flow.scope,
            BotAction::Cancel {
                operation_id: flow.operation,
            },
        ),
        BotCommand::Export { flow, output: path } => {
            if !(BotAction::Export {
                operation_id: flow.operation.clone(),
            })
            .validate()
            {
                return Err("invalid bot handle".into());
            }
            output = Some(foks_agent_client::secret_file::reserve_export(&path)?);
            (
                flow.scope,
                BotAction::Export {
                    operation_id: flow.operation,
                },
            )
        }
    };
    if !action.validate() {
        return Err("invalid bot action".into());
    }
    super::mcp::ensure_agent(state)?;
    let response = foks_agent_client::AgentClient::new(state.join("foks-rs.sock")).call(
        Operation::BotAccount {
            profile: scope.profile,
            account_alias: scope.account_alias,
            action,
        },
    )?;
    match response.result {
        ResponseResult::Success { mut value } => {
            if let Some(mut file) = output {
                let mut secret = value
                    .get_mut("token")
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .map(Zeroizing::new)
                    .ok_or("missing one-time token export")?;
                if let Some(serde_json::Value::String(s)) = value.get_mut("token") {
                    s.zeroize();
                }
                foks_agent_client::secret_file::write_export(&mut file, &secret)?;
                secret.zeroize();
                println!("{}", serde_json::to_string_pretty(&value["report"])?);
            } else {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            Ok(())
        }
        ResponseResult::Error { message, .. } => Err(message.into()),
    }
}
