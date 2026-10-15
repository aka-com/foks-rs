//! Read adapters share the checked profile/vault lifetime and immutable identity checks.
use super::*;
use foks_agent_proto::data::{DataCatalog, DataCatalogEntry, DataRead, DataScope, DataUsage};

fn submission_id(
    input: &str,
) -> Result<foks_agent_proto::data::SubmissionHandle, Box<dyn std::error::Error>> {
    input.parse().map_err(|_| {
        Box::new(AgentRequestError(
            "submission handle must be v1-<16 lowercase hex seconds>-<32 lowercase random hex>",
        )) as Box<dyn std::error::Error>
    })
}

pub(super) fn write_control(
    state_dir: &Path,
    registry: &ProfileRegistry,
    scope: DataScope,
    operation: Operation,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if scope.team_id.is_some() {
        return Err(Box::new(AgentRequestError(
            "submission control requires account scope",
        )));
    }
    let profile =
        crate::read_cache::open_profile_session(registry, &scope.profile, timeout, cancellation)?;
    // As in `execute`: the session already holds the vault wrapping key, so a
    // second credentials handle here would only repeat the manifest reads
    // behind it from inside the profile and database lock span.
    with_vault_and_master(state_dir, &profile, |session, vault, master| {
        session.check_data_identity(&scope.account_alias, &scope.host_id, &scope.user_id, vault)?;
        Ok(match operation {
            Operation::PrepareDataWrite {
                submission_id: id,
                spec,
                ..
            } => {
                let kind = match spec.kind {
                    foks_agent_proto::data::DataWriteKind::Put => {
                        foks_client_app::DataWriteKind::Put
                    }
                    foks_agent_proto::data::DataWriteKind::Mkdir => {
                        foks_client_app::DataWriteKind::Mkdir
                    }
                    foks_agent_proto::data::DataWriteKind::Remove => {
                        foks_client_app::DataWriteKind::Remove
                    }
                    foks_agent_proto::data::DataWriteKind::Move => {
                        foks_client_app::DataWriteKind::Move
                    }
                };
                serde_json::to_value(session.prepare_data_write(
                    &scope.account_alias,
                    submission_id(&id)?,
                    foks_client_app::DataWriteSpec {
                        kind,
                        path: spec.path,
                        destination: spec.destination,
                        team_selector: spec.team_selector,
                        overwrite: spec.overwrite,
                        mkdir_p: spec.mkdir_p,
                        recursive: spec.recursive,
                        body_length: spec.body_length,
                        body_hash: spec.body_hash,
                    },
                    vault,
                    master,
                )?)?
            }
            Operation::ExecuteDataWrite { submission } => {
                serde_json::to_value(session.execute_data_write(
                    &scope.account_alias,
                    submission_id(&submission.submission_id)?,
                    &mut std::io::empty(),
                    vault,
                    master,
                )?)?
            }
            Operation::DataWriteStatus { submission } => {
                serde_json::to_value(session.data_write_status(
                    &scope.account_alias,
                    submission_id(&submission.submission_id)?,
                    vault,
                    master,
                )?)?
            }
            Operation::PendingDataWrites { .. } => {
                serde_json::to_value(session.pending_data_writes(&scope.account_alias, vault)?)?
            }
            _ => return Err(Box::new(AgentRequestError("invalid submission control"))),
        })
    })
}

pub(super) fn upload<R: std::io::Read>(
    state_dir: &Path,
    registry: &ProfileRegistry,
    submission: foks_agent_proto::data::DataSubmission,
    reader: &mut R,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let scope = submission.scope;
    if scope.team_id.is_some() {
        return Err(Box::new(AgentRequestError(
            "submission upload requires account scope",
        )));
    }
    let profile =
        crate::read_cache::open_profile_session(registry, &scope.profile, timeout, cancellation)?;
    // The vault wrapping key this session already read is the one the write
    // needs, so opening a second credentials handle here would repeat the
    // manifest reads behind it from inside the profile and database lock
    // span, where the manifest lock is taken again.
    with_vault_and_master(state_dir, &profile, |session, vault, master| {
        session.check_data_identity(&scope.account_alias, &scope.host_id, &scope.user_id, vault)?;
        Ok(serde_json::to_value(session.execute_data_write(
            &scope.account_alias,
            submission_id(&submission.submission_id)?,
            reader,
            vault,
            master,
        )?)?)
    })
}

pub(super) fn bind_account(
    state_dir: &Path,
    registry: &ProfileRegistry,
    profile: String,
    account_alias: String,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let session =
        crate::read_cache::open_profile_session(registry, &profile, timeout, cancellation)?;
    with_vault(state_dir, &session, |session, vault| {
        let (host_id, user_id) = session.data_identity(&account_alias, vault)?;
        Ok(serde_json::to_value(DataScope {
            profile,
            account_alias,
            host_id,
            user_id,
            team_id: None,
        })?)
    })
}

/// Refuses a repair time that would leave writes paused after a reported
/// success. A preview only displays the time and may be up to the admission
/// skew away; a confirmation must be fresh. This runs before the profile lock
/// and the identity check; the repair repeats the confirmation bound against
/// the clock sample it writes.
fn check_repair_time(
    action: &foks_agent_proto::data::AdapterClockRepairAction,
    now: u64,
) -> Result<(), AgentRequestError> {
    use foks_agent_proto::data::AdapterClockRepairAction;
    match action {
        AdapterClockRepairAction::Preview { unix_seconds }
            if unix_seconds.abs_diff(now) > foks_client_db::ADAPTER_CLOCK_SKEW_SECONDS =>
        {
            Err(AgentRequestError(
                "the proposed time must be within five minutes of this computer's clock; \
                 correct the system clock, then confirm its current time",
            ))
        }
        AdapterClockRepairAction::Confirm { unix_seconds, .. }
            if unix_seconds.abs_diff(now) > foks_client_app::ADAPTER_CLOCK_CONFIRM_SKEW_SECONDS =>
        {
            Err(AgentRequestError(
                "the confirmed time must be within one minute of this computer's clock; \
                 preview the current time again, then confirm it",
            ))
        }
        _ => Ok(()),
    }
}

/// Status reads the clock state without anchoring it. A repair runs only when
/// the CLI or the desktop sends it; the MCP adapter has no tool that does, but
/// the agent socket does not tell the callers apart. The repair re-anchors at
/// this process's own monotonic clock, so it can only accept the computer's
/// current time (see [`check_repair_time`]).
pub(super) fn adapter_clock(
    state_dir: &Path,
    registry: &ProfileRegistry,
    profile: String,
    account_alias: String,
    repair: Option<foks_agent_proto::data::AdapterClockRepairAction>,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use foks_agent_proto::data::AdapterClockRepairAction;
    if let Some(action) = &repair {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Box::new(AgentRequestError("this computer's clock is before 1970")))?
            .as_secs();
        check_repair_time(action, now).map_err(Box::new)?;
    }
    let session =
        crate::read_cache::open_profile_session(registry, &profile, timeout, cancellation)?;
    with_vault(state_dir, &session, |session, vault| {
        Ok(match repair {
            None => serde_json::to_value(session.adapter_clock_status(&account_alias, vault)?)?,
            Some(AdapterClockRepairAction::Preview { unix_seconds }) => serde_json::to_value(
                session.adapter_clock_preview(&account_alias, unix_seconds, vault)?,
            )?,
            Some(AdapterClockRepairAction::Confirm {
                unix_seconds,
                confirmation_digest,
            }) => serde_json::to_value(session.reanchor_adapter_clock(
                &account_alias,
                unix_seconds,
                &confirmation_digest,
                vault,
            )?)?,
        })
    })
}

pub(super) fn read(
    state_dir: &Path,
    registry: &ProfileRegistry,
    mut scope: DataScope,
    query: DataRead,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let session =
        crate::read_cache::open_profile_session(registry, &scope.profile, timeout, cancellation)?;
    with_vault(state_dir, &session, |session, vault| {
        session.check_data_identity(&scope.account_alias, &scope.host_id, &scope.user_id, vault)?;
        let value = match query {
            DataRead::ResolveTeam { selector } => {
                scope.team_id =
                    Some(session.resolve_data_team(&scope.account_alias, &selector, vault)?);
                serde_json::to_value(&scope)?
            }
            DataRead::Stat { path, version } => serde_json::to_value(session.data_stat(
                &scope.account_alias,
                scope.team_id.as_deref(),
                &path,
                version,
                vault,
            )?)?,
            DataRead::Catalog => {
                let report =
                    session.data_catalog(&scope.account_alias, scope.team_id.as_deref(), vault)?;
                serde_json::to_value(DataCatalog {
                    user_chain_sequence: report.user_chain_sequence,
                    team_chain_sequence: report.team_chain_sequence,
                    root_id: report.root_id,
                    snapshot_version: report.snapshot_version,
                    entries: report
                        .entries
                        .into_iter()
                        .map(|row| DataCatalogEntry {
                            path: row.metadata.path,
                            node_type: row.metadata.node_type,
                            node_id: row.node_id,
                            modified_microseconds: row.modified_microseconds,
                            version: row.metadata.version,
                            size: row.metadata.size,
                            read_role: app_role_to_wire(row.metadata.read_role),
                            write_role: app_role_to_wire(row.metadata.write_role),
                        })
                        .collect(),
                })?
            }
            DataRead::Entry { path, version } => serde_json::to_value(session.data_entry(
                &scope.account_alias,
                scope.team_id.as_deref(),
                &path,
                version,
                vault,
            )?)?,
            DataRead::Chunk {
                path,
                version,
                offset,
                length,
            } => serde_json::to_value(session.data_chunk(
                &scope.account_alias,
                scope.team_id.as_deref(),
                &path,
                version,
                offset,
                length as usize,
                vault,
            )?)?,
            DataRead::Members => serde_json::to_value(
                session.data_members(
                    &scope.account_alias,
                    scope
                        .team_id
                        .as_deref()
                        .ok_or(AgentRequestError("team scope required"))?,
                    vault,
                )?,
            )?,
            DataRead::Memberships => {
                if scope.team_id.is_some() {
                    return Err(Box::new(AgentRequestError(
                        "memberships requires account scope",
                    )));
                }
                serde_json::to_value(session.data_memberships(&scope.account_alias, vault)?)?
            }
            DataRead::Usage => {
                let (files, bytes) =
                    session.data_usage(&scope.account_alias, scope.team_id.as_deref(), vault)?;
                serde_json::to_value(DataUsage { files, bytes })?
            }
        };
        // Existing profile checkpoints guard local replacement; keep the explicit
        // binding check adjacent to publication as well as before network work.
        session.check_data_identity(&scope.account_alias, &scope.host_id, &scope.user_id, vault)?;
        Ok(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_agent_proto::data::AdapterClockRepairAction;

    #[test]
    fn a_confirmed_repair_time_must_be_fresher_than_a_previewed_one() {
        let now = 1_700_000_000;
        let preview = |unix_seconds| AdapterClockRepairAction::Preview { unix_seconds };
        let confirm = |unix_seconds| AdapterClockRepairAction::Confirm {
            unix_seconds,
            confirmation_digest: "ab".repeat(32),
        };
        for seconds in [now - 300, now + 300] {
            assert!(check_repair_time(&preview(seconds), now).is_ok());
        }
        for seconds in [now - 301, now + 301] {
            assert!(check_repair_time(&preview(seconds), now).is_err());
        }
        for seconds in [now - 60, now + 60] {
            assert!(check_repair_time(&confirm(seconds), now).is_ok());
        }
        // A preview confirmed four minutes later would have anchored the clock
        // four minutes behind, leaving one minute of admission skew.
        for seconds in [now - 61, now - 240, now + 61] {
            assert!(check_repair_time(&confirm(seconds), now).is_err());
        }
    }
}
