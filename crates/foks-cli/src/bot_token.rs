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
    /// Save to a new private file, then confirm the durable export; never prints the token.
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
    super::deadline::check()?;
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
    let export_id = match &action {
        BotAction::Export { operation_id } => Some(operation_id.clone()),
        _ => None,
    };
    super::deadline::check()?;
    super::mcp::ensure_agent(state)?;
    let client = super::deadline::agent_client(state)?;
    let response = client.call(Operation::BotAccount {
        profile: scope.profile.clone(),
        account_alias: scope.account_alias.clone(),
        action,
    })?;
    match response.result {
        ResponseResult::Success { value } => {
            let mut value = super::deadline::AgentValue::new(value);
            if let Some(mut file) = output {
                let mut secret = value
                    .get_mut("token")
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .map(Zeroizing::new)
                    .ok_or("missing one-time token export")?;
                if let Some(serde_json::Value::String(s)) = value.get_mut("token") {
                    s.zeroize();
                }
                super::deadline::check()?;
                foks_agent_client::secret_file::prepare_export(&mut file, &secret)?;
                secret.zeroize();
                super::deadline::check()?;
                foks_agent_client::secret_file::publish_export(&mut file)?;
                foks_agent_client::secret_file::finish_export(&mut file).map_err(|_| {
                    "The file was published, but directory durability could not be confirmed. The agent retains the token for recovery."
                })?;
                super::deadline::check().map_err(|_| {
                    "Token saved. Local export confirmation timed out; the agent retains the token for recovery."
                })?;
                let completed = client.call(Operation::BotAccount {
                    profile: scope.profile,
                    account_alias: scope.account_alias,
                    action: BotAction::CompleteExport { operation_id: export_id.expect("export reservation has operation ID") },
                }).map_err(|_| "Token saved. Local export confirmation was not received; retain this file and check enrollment status.")?;
                match completed.result {
                    ResponseResult::Success { value } => super::deadline::print_agent_value(value).map_err(|_| "Token saved and local export confirmed. The status response could not be displayed; retain the saved file.")?,
                    ResponseResult::Error { .. } => return Err("Token saved, but local export confirmation failed; retain this file and check enrollment status.".into()),
                }
            } else {
                value.print()?;
            }
            Ok(())
        }
        ResponseResult::Error { message, .. } => Err(message.into()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        io::{Read as _, Write as _},
        os::unix::{fs::PermissionsExt as _, net::UnixListener},
        time::Duration,
    };
    fn request(stream: &mut std::os::unix::net::UnixStream) -> foks_agent_proto::Request {
        // Darwin accepts may inherit the listener's nonblocking flag.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut prefix = [0; 4];
        stream.read_exact(&mut prefix).unwrap();
        let mut frame = vec![0; 4 + u32::from_be_bytes(prefix) as usize];
        frame[..4].copy_from_slice(&prefix);
        stream.read_exact(&mut frame[4..]).unwrap();
        foks_agent_proto::decode_request(&frame).unwrap()
    }
    #[test]
    fn export_confirms_only_after_file_publication_with_original_scope_and_deadline() {
        for lose_offer in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let socket = temp.path().join("foks-rs.sock");
            let destination = temp.path().join("saved-token");
            let listener = UnixListener::bind(&socket).unwrap();
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
            let _scope =
                super::super::deadline::CommandScope::enter(Duration::from_secs(15)).unwrap();
            let original = super::super::deadline::current().unwrap();
            let path = destination.clone();
            let id = "12".repeat(16);
            let expected_id = id.clone();
            let (done, completed) = std::sync::mpsc::channel();
            listener.set_nonblocking(true).unwrap();
            let server = std::thread::spawn(move || {
                let until = std::time::Instant::now() + Duration::from_secs(5);
                let mut calls = 0;
                for stage in 0..if lose_offer { 3 } else { 4 } {
                    let (mut stream, _) = loop {
                        match listener.accept() {
                            Ok(connection) => break connection,
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(
                                    std::time::Instant::now() < until,
                                    "expected bot request did not arrive"
                                );
                                std::thread::yield_now();
                            }
                            Err(e) => panic!("bot accept failed: {e}"),
                        }
                    };
                    if stage == 0 {
                        // ensure_agent starts with the version-independent Hello.
                        stream.set_nonblocking(false).unwrap();
                        let mut prefix = [0; 4];
                        stream.read_exact(&mut prefix).unwrap();
                        let mut frame = vec![0; 4 + u32::from_be_bytes(prefix) as usize];
                        frame[..4].copy_from_slice(&prefix);
                        stream.read_exact(&mut frame[4..]).unwrap();
                        let hello = foks_agent_proto::decode_hello(&frame).unwrap().unwrap();
                        let reply = foks_agent_proto::HelloReply::new(
                            foks_agent_proto::Peer::new("foks-agent", "test"),
                            &[],
                        );
                        stream
                            .write_all(
                                &foks_agent_proto::encode(&foks_agent_proto::Response::success(
                                    hello.id,
                                    serde_json::to_value(reply).unwrap(),
                                ))
                                .unwrap(),
                            )
                            .unwrap();
                        continue;
                    }
                    let req = request(&mut stream);
                    assert_eq!(req.deadline.domain(), original.envelope().domain());
                    assert!(req.deadline.expires_ns() <= original.envelope().expires_ns());
                    let value = match (stage, req.operation) {
                        (1, Operation::AgentStatus) => {
                            serde_json::to_value(foks_agent_proto::AgentStatus::ready()).unwrap()
                        }
                        (
                            2,
                            Operation::BotAccount {
                                profile,
                                account_alias,
                                action: BotAction::Export { operation_id },
                            },
                        ) => {
                            assert_eq!(
                                (
                                    profile.as_str(),
                                    account_alias.as_str(),
                                    operation_id.as_str()
                                ),
                                ("local", "owner", expected_id.as_str())
                            );
                            assert!(!path.exists());
                            if lose_offer {
                                break;
                            }
                            serde_json::json!({"token":"a".repeat(37),"report":{}})
                        }
                        (
                            3,
                            Operation::BotAccount {
                                profile,
                                account_alias,
                                action: BotAction::CompleteExport { operation_id },
                            },
                        ) => {
                            assert_eq!(
                                (
                                    profile.as_str(),
                                    account_alias.as_str(),
                                    operation_id.as_str()
                                ),
                                ("local", "owner", expected_id.as_str())
                            );
                            assert_eq!(
                                foks_agent_client::secret_file::import_token(&path)
                                    .unwrap()
                                    .expose(),
                                "a".repeat(37)
                            );
                            calls += 1;
                            serde_json::json!({"export_available":false})
                        }
                        _ => panic!("unexpected bot request"),
                    };
                    stream
                        .write_all(
                            &foks_agent_proto::encode(&foks_agent_proto::Response::success(
                                req.id, value,
                            ))
                            .unwrap(),
                        )
                        .unwrap();
                }
                completed.recv_timeout(Duration::from_secs(3)).unwrap();
                listener.set_nonblocking(true).unwrap();
                assert!(
                    listener.accept().is_err(),
                    "unexpected completion after failed publication"
                );
                calls
            });
            let result = run(
                temp.path(),
                BotCommand::Export {
                    flow: Flow {
                        scope: Scope {
                            profile: "local".into(),
                            account_alias: "owner".into(),
                        },
                        operation: id,
                    },
                    output: destination.clone(),
                },
            );
            done.send(()).unwrap();
            assert_eq!(result.is_ok(), !lose_offer);
            assert_eq!(server.join().unwrap(), usize::from(!lose_offer));
            assert_eq!(destination.exists(), !lose_offer);
            assert!(!std::fs::read_dir(temp.path()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".foks-export-")));
        }
    }
}
