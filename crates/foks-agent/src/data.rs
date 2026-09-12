//! Read adapters share the checked profile/vault lifetime and immutable identity checks.
use super::*;
use foks_agent_proto::data::{DataCatalog, DataCatalogEntry, DataRead, DataScope, DataUsage};

fn submission_id(input: &str) -> Result<[u8; 16], Box<dyn std::error::Error>> {
    if input.len() != 32
        || !input
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(Box::new(AgentRequestError(
            "submission ID must be 32 lowercase hex characters",
        )));
    }
    let mut id = [0; 16];
    for (out, pair) in id.iter_mut().zip(input.as_bytes().chunks_exact(2)) {
        *out = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(id)
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
        ProfileSession::open_with_control(registry, &scope.profile, timeout, cancellation)?;
    with_vault(state_dir, &profile, |session, vault| {
        session.check_data_identity(&scope.account_alias, &scope.host_id, &scope.user_id, vault)?;
        let credentials = ClientCredentials::open(state_dir)?;
        let master = credentials.master_key()?;
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
                    &master,
                )?)?
            }
            Operation::ExecuteDataWrite { submission } => {
                serde_json::to_value(session.execute_data_write(
                    &scope.account_alias,
                    submission_id(&submission.submission_id)?,
                    &mut std::io::empty(),
                    vault,
                    &master,
                )?)?
            }
            Operation::DataWriteStatus { submission } => {
                serde_json::to_value(session.data_write_status(
                    &scope.account_alias,
                    submission_id(&submission.submission_id)?,
                    vault,
                    &master,
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
        ProfileSession::open_with_control(registry, &scope.profile, timeout, cancellation)?;
    with_vault(state_dir, &profile, |session, vault| {
        session.check_data_identity(&scope.account_alias, &scope.host_id, &scope.user_id, vault)?;
        let credentials = ClientCredentials::open(state_dir)?;
        Ok(serde_json::to_value(session.execute_data_write(
            &scope.account_alias,
            submission_id(&submission.submission_id)?,
            reader,
            vault,
            &*credentials.master_key()?,
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
    let session = ProfileSession::open_with_control(registry, &profile, timeout, cancellation)?;
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

pub(super) fn read(
    state_dir: &Path,
    registry: &ProfileRegistry,
    mut scope: DataScope,
    query: DataRead,
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let session =
        ProfileSession::open_with_control(registry, &scope.profile, timeout, cancellation)?;
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
