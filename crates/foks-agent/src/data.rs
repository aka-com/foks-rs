//! Read adapters share the checked profile/vault lifetime and immutable identity checks.
use super::*;
use foks_agent_proto::data::{DataCatalog, DataCatalogEntry, DataRead, DataScope, DataUsage};

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
