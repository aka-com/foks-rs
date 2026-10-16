use foks_client_app::{ProfileRegistry, ProfileSession};
use std::path::Path;

#[derive(clap::Subcommand)]
pub enum RetentionCommand {
    /// Read redacted resident maintenance counters.
    Status,
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
    /// Preview or explicitly re-anchor local receipt time without pruning or writing remotely.
    Reanchor {
        #[arg(long)]
        profile: String,
        /// Receipt clock to repair. Upload and adapter clocks are independent.
        #[arg(long, value_enum, default_value = "adapter")]
        domain: ReceiptDomain,
        #[arg(long)]
        account_alias: String,
        #[arg(long)]
        unix_seconds: u64,
        /// Exact digest printed by the preview for these time values.
        #[arg(long)]
        confirm_digest: Option<String>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum ReceiptDomain {
    Adapter,
    Upload,
}

pub fn run(state: &Path, command: RetentionCommand) -> Result<(), Box<dyn std::error::Error>> {
    if let RetentionCommand::Maintain {
        profile,
        max_seconds,
        max_turns,
    } = command
    {
        return maintain(state, &profile, max_seconds, max_turns);
    }
    let RetentionCommand::Reanchor {
        profile,
        domain,
        account_alias,
        unix_seconds,
        confirm_digest,
    } = command
    else {
        let reply = super::deadline::agent_client(state)?
            .call(foks_agent_proto::Operation::RetentionStatus)?;
        return match reply.result {
            foks_agent_proto::ResponseResult::Success { value } => {
                super::deadline::print_agent_value(value)?;
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
    let result =
        super::with_vault(state, &session, |checked, vault, _| {
            Ok(match (domain, confirm_digest) {
                (ReceiptDomain::Adapter, Some(ref digest)) => serde_json::to_value(
                    checked.reanchor_adapter_clock(&account_alias, unix_seconds, digest, vault)?,
                )?,
                (ReceiptDomain::Adapter, None) => serde_json::to_value(
                    checked.adapter_clock_preview(&account_alias, unix_seconds, vault)?,
                )?,
                (ReceiptDomain::Upload, Some(ref digest)) => serde_json::to_value(
                    checked.reanchor_upload_clock(&account_alias, unix_seconds, digest, vault)?,
                )?,
                (ReceiptDomain::Upload, None) => serde_json::to_value(
                    checked.upload_clock_preview(&account_alias, unix_seconds, vault)?,
                )?,
            })
        })?;
    super::deadline::print_agent_value(result)?;
    Ok(())
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
