use foks_agent_proto::{
    data::{AdapterClockRepairAction, ReceiptClockDomain},
    Operation, ResponseResult,
};
use foks_client_app::{ProfileRegistry, ProfileSession};
use std::path::Path;

#[derive(clap::Subcommand)]
pub enum RetentionCommand {
    /// Read redacted resident maintenance counters.
    Status,
    /// Report whether the account's receipt clock admits new handles: MCP
    /// writes for the adapter clock, file uploads for the upload clock.
    ClockStatus {
        #[arg(long)]
        profile: String,
        /// Receipt clock to report. Upload and adapter clocks are independent.
        #[arg(long, value_enum, default_value = "adapter")]
        domain: ReceiptDomain,
        #[arg(long)]
        account_alias: String,
    },
    /// Drain local completed-upload cleanup without contacting a server.
    Maintain {
        #[arg(long)]
        profile: String,
        /// Cooperative budget; an in-progress filesystem operation may overrun.
        #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=60))]
        max_seconds: u64,
        #[arg(long, default_value_t = 256, value_parser = clap::value_parser!(u32).range(1..=1024))]
        max_turns: u32,
    },
    /// Preview or explicitly re-anchor a local receipt clock at this
    /// computer's current time, without pruning or writing remotely.
    Reanchor {
        #[arg(long)]
        profile: String,
        /// Receipt clock to repair. Upload and adapter clocks are independent.
        #[arg(long, value_enum, default_value = "adapter")]
        domain: ReceiptDomain,
        #[arg(long)]
        account_alias: String,
        /// The time to confirm. The agent refuses a preview more than five
        /// minutes from its own clock and a confirmation more than one minute
        /// from it. Defaults to now for a preview.
        #[arg(long)]
        unix_seconds: Option<u64>,
        /// Exact digest printed by the preview, with the preview's
        /// `proposed_wall` as `--unix-seconds`. Confirm within one minute of
        /// the preview; after that, preview again.
        #[arg(long, requires = "unix_seconds")]
        confirm_digest: Option<String>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum ReceiptDomain {
    Adapter,
    Upload,
}

impl From<ReceiptDomain> for ReceiptClockDomain {
    fn from(domain: ReceiptDomain) -> Self {
        match domain {
            ReceiptDomain::Adapter => Self::Adapter,
            ReceiptDomain::Upload => Self::Upload,
        }
    }
}

/// Clock status and repair run in the resident agent, which samples the same
/// process clock that later admits writes and uploads. `maintain` drains
/// local cleanup in this process under the profile lock, as the agent's
/// resident maintenance does, and reports contention as incomplete.
pub fn run(state: &Path, command: RetentionCommand) -> Result<(), Box<dyn std::error::Error>> {
    let operation = match command {
        RetentionCommand::Status => Operation::RetentionStatus,
        RetentionCommand::Maintain {
            profile,
            max_seconds,
            max_turns,
        } => return maintain(state, &profile, max_seconds, max_turns),
        RetentionCommand::ClockStatus {
            profile,
            domain,
            account_alias,
        } => {
            super::mcp::ensure_agent(state)?;
            Operation::AdapterClockStatus {
                profile,
                account_alias,
                domain: domain.into(),
            }
        }
        RetentionCommand::Reanchor {
            profile,
            domain,
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
                domain: domain.into(),
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
    let reply = super::deadline::agent_client(state)?.call(operation)?;
    match reply.result {
        ResponseResult::Success { value } => super::deadline::print_agent_value(value),
        ResponseResult::Error { code, message, .. } => Err(format!("{code:?}: {message}").into()),
    }
}

fn maintain(
    state: &Path,
    profile: &str,
    seconds: u64,
    turns: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let control = super::deadline::current()?
        .shorten(std::time::Instant::now() + std::time::Duration::from_secs(seconds))?;
    let deadline = control.instant();
    let registry = ProfileRegistry::open(state)?;
    let session = ProfileSession::open(&registry, profile)?;
    let credentials = foks_client_app::ClientCredentials::open(state)?;
    let mut cursor = foks_client_app::AdapterMaintenanceCursor::default();
    let mut last = None;
    let mut completed_turns = 0;
    while completed_turns < turns && std::time::Instant::now() < deadline {
        control.check()?;
        let turn_deadline =
            deadline.min(std::time::Instant::now() + std::time::Duration::from_millis(50));
        let result = credentials.try_with_checked_session(&session, |checked| {
            let master = credentials.master_key()?;
            let report =
                checked.maintain_adapter_state_until(&mut cursor, &master, turn_deadline)?;
            Ok::<_, foks_client_app::Error>((report, checked.upload_cleanup_debt()?))
        })?;
        completed_turns += 1;
        if let Some((report, debt)) = result {
            let complete = debt.incomplete == 0 && debt.reserved_bytes == 0;
            let blocked = debt.actionable == 0 && debt.incomplete > 0;
            last = Some(debt);
            if complete || blocked {
                break;
            }
            if !report.made_progress {
                std::thread::sleep(
                    std::time::Duration::from_millis(10)
                        .min(deadline.saturating_duration_since(std::time::Instant::now())),
                );
            }
        } else {
            std::thread::sleep(
                std::time::Duration::from_millis(10)
                    .min(deadline.saturating_duration_since(std::time::Instant::now())),
            );
        }
        // No profile lock survives this point; foreground users can enter.
    }
    let complete = last
        .as_ref()
        .is_some_and(|debt| debt.incomplete == 0 && debt.reserved_bytes == 0);
    println!(
        "{}",
        serde_json::json!({"complete": complete, "turns":completed_turns,
        "reserved_bytes":last.as_ref().map(|d|d.reserved_bytes),
        "incomplete_owners":last.as_ref().map(|d|d.incomplete),
        "actionable_owners":last.as_ref().map(|d|d.actionable)})
    );
    if complete {
        Ok(())
    } else {
        Err("Local cleanup is incomplete; pending uploads may need resume/cancel, or retry maintenance after resolving the reported condition.".into())
    }
}

pub(super) fn preflight(state: &Path, session: &ProfileSession) {
    // One local turn only, outside the source-owning upload operation. Failure
    // does not authorize re-reading a source or mask normal admission errors.
    if let Ok(credentials) = foks_client_app::ClientCredentials::open(state) {
        let _ = credentials.try_with_checked_session(session, |checked| {
            let master = credentials.master_key()?;
            checked.maintain_adapter_state(&mut Default::default(), &master)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn maintenance_cli_has_explicit_bounded_policy() {
        let parse = |extra: &[&str]| {
            let mut args = vec![
                "foks-rs",
                "--state-dir",
                "/tmp/test-state",
                "retention",
                "maintain",
                "--profile",
                "local",
            ];
            args.extend_from_slice(extra);
            crate::Arguments::try_parse_from(args)
        };
        assert!(matches!(
            parse(&[]).unwrap().command,
            crate::Command::Retention(RetentionCommand::Maintain {
                max_seconds: 15,
                max_turns: 256,
                ..
            })
        ));
        assert!(parse(&["--max-seconds", "61"]).is_err());
        assert!(parse(&["--max-turns", "1025"]).is_err());
        assert!(parse(&["--max-seconds", "0"]).is_err());
    }
    #[test]
    fn local_drain_reports_completion_and_contention_without_remote_access() {
        let _scope =
            super::super::deadline::CommandScope::enter(std::time::Duration::from_secs(15))
                .unwrap();
        use foks_client_app::*;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("state");
        ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(Profile {
                name: "local".into(),
                label: None,
                probe: "unreachable.invalid".into(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        assert!(maintain(&root, "local", 1, 4).is_ok());
        let (entered, received) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let held_root = root.clone();
        let holder = std::thread::spawn(move || {
            let credentials = ClientCredentials::open(&held_root).unwrap();
            let registry = ProfileRegistry::open(&held_root).unwrap();
            let session = ProfileSession::open(&registry, "local").unwrap();
            credentials
                .with_checked_session(&session, |_| {
                    entered.send(()).unwrap();
                    wait.recv().unwrap();
                    Ok::<_, foks_client_app::Error>(())
                })
                .unwrap();
        });
        received.recv().unwrap();
        let result = maintain(&root, "local", 1, 1);
        release.send(()).unwrap();
        holder.join().unwrap();
        assert!(result.unwrap_err().to_string().contains("incomplete"));
        assert!(maintain(&root, "local", 1, 4).is_ok());
    }
}
