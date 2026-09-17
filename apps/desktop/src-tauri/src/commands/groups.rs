//! Group discovery, membership, federation, and authorization projections.

use crate::agent::AgentError;
use crate::commands::context::AppState;
use crate::commands::execution::{
    ambiguous_mutation_response, apply_operation, apply_profile_operation_value, MutationKind,
};
use crate::commands::preparation::{
    check_mutation_access, prepare_catalog_mutation, prepare_group_mutation, GroupMutationFacts,
};
use crate::commands::types::{MutationDto, RoleDto};
use crate::commands::validation::{
    bounded_local_name, invalid_request, invalid_response, require_main_window,
    require_nested_response_row_cap, require_response_row_cap, required_field, valid_local_name,
    valid_operation_id_hex, valid_response_text, valid_typed_entity_id_hex, AD_HOC_TEAM_ID_PREFIX,
    HOST_ID_PREFIX, MAXIMUM_FIRST_RUN_ROWS, NAMED_TEAM_ID_PREFIX, USER_ID_PREFIX,
};
use foks_agent_proto::{
    FederationRole, KvRole, Operation, ResponseResult, TeamDetailsSummary, TeamKind, TeamRole,
};
use foks_desktop::{AgentError as DesktopAgentError, CatalogStoreRef};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use tauri::State;

pub(super) fn member_not_actionable() -> AgentError {
    AgentError::new(
        "member-not-actionable",
        "This member cannot be modified from this account.",
        false,
    )
}

pub(super) fn member_role_from_dto(role: &RoleDto) -> Result<MemberRole, AgentError> {
    match (role.role, role.visibility) {
        ("Member", Some(visibility)) => Ok(MemberRole::Member { visibility }),
        ("Admin", None) => Ok(MemberRole::Admin),
        ("Owner", None) => Ok(MemberRole::Owner),
        _ => Err(invalid_response("The group member role is invalid.")),
    }
}

pub(super) fn admission_not_resumable() -> AgentError {
    AgentError::new(
        "admission-not-resumable",
        "This inactive admission cannot be resumed.",
        false,
    )
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredGroupDto {
    pub alias: String,
    pub account_alias: String,
    pub team_id_hex: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDiscoveryDto {
    pub account_alias: String,
    pub groups: Vec<DiscoveredGroupDto>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GroupDiscoveryResponse {
    pub(super) account_alias: String,
    pub(super) teams: Vec<DiscoveredGroupResponse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DiscoveredGroupResponse {
    pub(super) alias: String,
    pub(super) account_alias: String,
    pub(super) team_id_hex: String,
    pub(super) kind: String,
    pub(super) name: Option<String>,
    pub(super) active: bool,
    // Creation lifecycle applies to locally created teams; discovered records
    // carry no creation intent. Accept this optional summary field explicitly.
    #[serde(default, rename = "creation_phase")]
    pub(super) _creation_phase: Option<String>,
}

impl GroupDiscoveryDto {
    pub(super) fn from_response(
        expected_account: &str,
        response: GroupDiscoveryResponse,
    ) -> Result<Self, AgentError> {
        if response.account_alias != expected_account
            || !valid_local_name(&response.account_alias)
            || response.teams.len() > MAXIMUM_FIRST_RUN_ROWS
        {
            return Err(invalid_response(
                "Discovered group results do not match the selected account.",
            ));
        }
        let mut aliases = std::collections::HashSet::new();
        let mut team_ids = std::collections::HashSet::new();
        let groups = response
            .teams
            .into_iter()
            .map(|team| {
                let kind = match team.kind.as_str() {
                    "named" => "named",
                    "ad-hoc" => "adhoc",
                    _ => {
                        return Err(invalid_response("The returned group type is unsupported."));
                    }
                };
                let team_id_is_valid = match kind {
                    "named" => valid_typed_entity_id_hex(&team.team_id_hex, NAMED_TEAM_ID_PREFIX),
                    "adhoc" => valid_typed_entity_id_hex(&team.team_id_hex, AD_HOC_TEAM_ID_PREFIX),
                    _ => unreachable!("discovered group kinds are normalized above"),
                };
                if team.account_alias != expected_account
                    || !valid_local_name(&team.alias)
                    || !team_id_is_valid
                    || team
                        .name
                        .as_ref()
                        .is_some_and(|name| !valid_response_text(name, 512))
                    || (kind == "named" && team.name.is_none())
                    || (kind == "adhoc" && team.name.is_some())
                    || !aliases.insert(team.alias.clone())
                    || !team_ids.insert(team.team_id_hex.clone())
                {
                    return Err(invalid_response("Discovered group details are invalid."));
                }
                Ok(DiscoveredGroupDto {
                    alias: team.alias,
                    account_alias: team.account_alias,
                    team_id_hex: team.team_id_hex,
                    kind,
                    name: team.name,
                    active: team.active,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            account_alias: response.account_alias,
            groups,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GoProfileCandidateResponse {
    pub(super) candidate_id: String,
    pub(super) username: Option<String>,
    pub(super) server_hint: Option<String>,
    pub(super) host_id_hex: String,
    pub(super) user_id_hex: String,
    pub(super) device_id_hex: String,
    pub(super) role: String,
    pub(super) storage_kind: String,
    pub(super) hidden: bool,
    pub(super) provisional: bool,
    pub(super) pairable: bool,
    pub(super) copyable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MemberRole {
    Member { visibility: i16 },
    Admin,
    Owner,
}

impl<'de> Deserialize<'de> for MemberRole {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case")]
        enum WireRole {
            Member(MemberFields),
            Admin,
            Owner,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct MemberFields {
            visibility: i16,
        }

        Ok(match WireRole::deserialize(deserializer)? {
            WireRole::Member(fields) => Self::Member {
                visibility: fields.visibility,
            },
            WireRole::Admin => Self::Admin,
            WireRole::Owner => Self::Owner,
        })
    }
}

impl MemberRole {
    pub(super) fn is_strictly_lower_than(self, current: Self) -> bool {
        match (self, current) {
            (
                Self::Member {
                    visibility: destination,
                },
                Self::Member {
                    visibility: current,
                },
            ) => destination < current,
            (Self::Member { .. }, Self::Admin | Self::Owner) | (Self::Admin, Self::Owner) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoleInput {
    Member { visibility: i16 },
    Admin,
    Owner,
}

impl<'de> Deserialize<'de> for RoleInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum RoleName {
            Member,
            Admin,
            Owner,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireRole {
            role: RoleName,
            visibility: Option<i16>,
        }

        let wire = WireRole::deserialize(deserializer)?;
        match (wire.role, wire.visibility) {
            (RoleName::Member, Some(visibility)) => Ok(Self::Member { visibility }),
            (RoleName::Admin, None) => Ok(Self::Admin),
            (RoleName::Owner, None) => Ok(Self::Owner),
            (RoleName::Member, None) => Err(serde::de::Error::missing_field("visibility")),
            (RoleName::Admin | RoleName::Owner, Some(_)) => Err(serde::de::Error::custom(
                "Visibility can only be specified for the member role.",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum GroupKindInput {
    Named,
    Adhoc,
}

impl RoleInput {
    pub(super) fn parts(self) -> (TeamRole, i16) {
        match self {
            Self::Member { visibility } => (TeamRole::Member, visibility),
            Self::Admin => (TeamRole::Admin, 0),
            Self::Owner => (TeamRole::Owner, 0),
        }
    }
}

impl From<RoleInput> for MemberRole {
    fn from(role: RoleInput) -> Self {
        match role {
            RoleInput::Member { visibility } => Self::Member { visibility },
            RoleInput::Admin => Self::Admin,
            RoleInput::Owner => Self::Owner,
        }
    }
}

impl From<MemberRole> for RoleDto {
    fn from(role: MemberRole) -> Self {
        match role {
            MemberRole::Member { visibility } => KvRole::Member { visibility }.into(),
            MemberRole::Admin => KvRole::Admin.into(),
            MemberRole::Owner => KvRole::Owner.into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MemberResponse {
    pub(super) username: Option<String>,
    pub(super) party_id_hex: String,
    pub(super) scoped_host_id_hex: Option<String>,
    pub(super) party_kind: String,
    pub(super) source_role: MemberRole,
    pub(super) destination_role: MemberRole,
    pub(super) generation: u64,
    pub(super) locally_manageable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PartyDto {
    pub store: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub party_kind: String,
    pub generation: u64,
    pub locally_manageable: bool,
    pub party_id_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoped_host_id_hex: Option<String>,
    pub source_role: RoleDto,
    pub destination_role: RoleDto,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FederationResponse {
    pub(super) local_team_alias: String,
    pub(super) remote_profile: String,
    pub(super) remote_team_alias: String,
    pub(super) remote_host_id_hex: String,
    pub(super) remote_team_id_hex: String,
    pub(super) destination: MemberRole,
    pub(super) operation_id_hex: Option<String>,
    pub(super) active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FederationEntryDto {
    pub store: String,
    pub remote_profile: String,
    pub remote_team_alias: String,
    pub remote_host_id_hex: String,
    pub remote_team_id_hex: String,
    pub destination: RoleDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id_hex: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum GroupDetailResultDto<T> {
    Success { value: T },
    Error { error: AgentError },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GroupDetailsDto {
    pub parties: GroupDetailResultDto<Vec<PartyDto>>,
    pub federation: GroupDetailResultDto<Vec<FederationEntryDto>>,
}

pub(super) fn group_detail_result<T>(
    result: ResponseResult,
    decode: impl FnOnce(serde_json::Value) -> Result<T, AgentError>,
) -> Result<GroupDetailResultDto<T>, AgentError> {
    match result {
        ResponseResult::Success { value } => Ok(GroupDetailResultDto::Success {
            value: decode(value)?,
        }),
        ResponseResult::Error {
            code,
            message,
            fields,
        } => Ok(GroupDetailResultDto::Error {
            error: AgentError::from_desktop(DesktopAgentError::Protocol {
                code,
                message,
                fields,
            }),
        }),
    }
}

pub(super) fn party_dtos(
    store: &str,
    members: Vec<MemberResponse>,
) -> Result<Vec<PartyDto>, AgentError> {
    if members.len() > MAXIMUM_FIRST_RUN_ROWS {
        return Err(invalid_response(
            "The agent returned too many group members.",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    members
        .into_iter()
        .map(|member| {
            let (party_shape_is_valid, party_id_is_valid) = match member.party_kind.as_str() {
                "user" => (
                    !member.locally_manageable
                        || (member.username.is_some() && member.scoped_host_id_hex.is_none()),
                    valid_typed_entity_id_hex(&member.party_id_hex, USER_ID_PREFIX),
                ),
                "named-team" => (
                    member.username.is_none() && !member.locally_manageable,
                    valid_typed_entity_id_hex(&member.party_id_hex, NAMED_TEAM_ID_PREFIX),
                ),
                "ad-hoc-team" => (
                    member.username.is_none() && !member.locally_manageable,
                    valid_typed_entity_id_hex(&member.party_id_hex, AD_HOC_TEAM_ID_PREFIX),
                ),
                _ => (false, false),
            };
            if !party_shape_is_valid
                || !party_id_is_valid
                || member
                    .username
                    .as_ref()
                    .is_some_and(|name| !valid_response_text(name, 256))
                || member
                    .scoped_host_id_hex
                    .as_ref()
                    .is_some_and(|host| !valid_typed_entity_id_hex(host, HOST_ID_PREFIX))
                || !ids.insert(member.party_id_hex.clone())
            {
                return Err(invalid_response(
                    "The agent returned an unsupported or inconsistent group member.",
                ));
            }
            Ok(PartyDto {
                store: store.to_owned(),
                username: member.username,
                party_kind: member.party_kind,
                generation: member.generation,
                locally_manageable: member.locally_manageable,
                party_id_hex: member.party_id_hex,
                scoped_host_id_hex: member.scoped_host_id_hex,
                source_role: member.source_role.into(),
                destination_role: member.destination_role.into(),
            })
        })
        .collect()
}

pub(super) fn federation_dtos(
    store: &str,
    local_profile: &str,
    local_team_alias: &str,
    memberships: Vec<FederationResponse>,
) -> Result<Vec<FederationEntryDto>, AgentError> {
    if memberships.len() > MAXIMUM_FIRST_RUN_ROWS {
        return Err(invalid_response(
            "The response contains too many federation entries.",
        ));
    }
    let mut remote_aliases = std::collections::HashSet::new();
    let mut remote_ids = std::collections::HashSet::new();
    let mut operation_ids = std::collections::HashSet::new();
    memberships
        .into_iter()
        .map(|membership| {
            if membership.local_team_alias != local_team_alias
                || membership.remote_profile == local_profile
                || !valid_local_name(&membership.local_team_alias)
                || !valid_response_text(&membership.remote_profile, 64)
                || !valid_local_name(&membership.remote_team_alias)
                || !valid_typed_entity_id_hex(&membership.remote_host_id_hex, HOST_ID_PREFIX)
                || !(valid_typed_entity_id_hex(
                    &membership.remote_team_id_hex,
                    NAMED_TEAM_ID_PREFIX,
                ) || valid_typed_entity_id_hex(
                    &membership.remote_team_id_hex,
                    AD_HOC_TEAM_ID_PREFIX,
                ))
                || membership
                    .operation_id_hex
                    .as_ref()
                    .is_some_and(|operation| !valid_operation_id_hex(operation))
                || (membership.active && membership.operation_id_hex.is_none())
                || !matches!(membership.destination, MemberRole::Member { .. })
                || !remote_aliases.insert((
                    membership.remote_profile.clone(),
                    membership.remote_team_alias.clone(),
                ))
                || !remote_ids.insert((
                    membership.remote_host_id_hex.clone(),
                    membership.remote_team_id_hex.clone(),
                ))
                || membership
                    .operation_id_hex
                    .as_ref()
                    .is_some_and(|operation| !operation_ids.insert(operation.clone()))
            {
                return Err(invalid_response(
                    "The returned federation entry is unsupported or inconsistent.",
                ));
            }
            Ok(FederationEntryDto {
                store: store.to_owned(),
                remote_profile: membership.remote_profile,
                remote_team_alias: membership.remote_team_alias,
                remote_host_id_hex: membership.remote_host_id_hex,
                remote_team_id_hex: membership.remote_team_id_hex,
                destination: membership.destination.into(),
                operation_id_hex: membership.operation_id_hex,
                active: membership.active,
            })
        })
        .collect()
}

pub(super) fn create_group_operation(
    account: foks_agent_proto::AccountStoreRef,
    team_alias: &str,
    name: &str,
    kind: GroupKindInput,
) -> Result<Operation, AgentError> {
    let team_alias = required_field(team_alias, "Group alias is required.")?;
    let (name, kind) = match kind {
        GroupKindInput::Named => (
            required_field(name, "Group name is required.")?,
            TeamKind::Named,
        ),
        // An ad-hoc group's user-facing local name is represented by the
        // caller-derived local alias. The protocol requires no server name.
        GroupKindInput::Adhoc => (String::new(), TeamKind::AdHoc),
    };
    Ok(Operation::CreateTeam {
        profile: account.profile,
        account_alias: account.account_alias,
        team_alias,
        name,
        kind,
    })
}

pub(super) fn add_group_member_operation(
    team: foks_agent_proto::TeamStoreRef,
    username: &str,
    destination: RoleInput,
) -> Result<Operation, AgentError> {
    let username = required_field(username, "Username is required.")?;
    let (role, visibility) = destination.parts();
    Ok(Operation::AddTeamMember {
        profile: team.profile,
        team_alias: team.team_alias,
        username,
        role,
        visibility,
    })
}

pub(super) fn demote_group_member_operation(
    team: foks_agent_proto::TeamStoreRef,
    party_id_hex: &str,
    current: MemberRole,
    destination: RoleInput,
) -> Result<Operation, AgentError> {
    let destination_role = MemberRole::from(destination);
    if !destination_role.is_strictly_lower_than(current) {
        return Err(AgentError::new(
            "not-a-demotion",
            "Select a role lower than the member's current role.",
            false,
        ));
    }
    if !valid_typed_entity_id_hex(party_id_hex, USER_ID_PREFIX) {
        return Err(invalid_request("Select a valid user."));
    }
    let party_id_hex = party_id_hex.to_owned();
    let (role, visibility) = destination.parts();
    Ok(Operation::DemoteTeamMember {
        profile: team.profile,
        team_alias: team.team_alias,
        party_id_hex,
        role,
        visibility,
    })
}

pub(super) fn remove_group_member_operation(
    team: foks_agent_proto::TeamStoreRef,
    party_id_hex: &str,
) -> Result<Operation, AgentError> {
    if !valid_typed_entity_id_hex(party_id_hex, USER_ID_PREFIX) {
        return Err(invalid_request("Select a valid user."));
    }
    Ok(Operation::RemoveTeamMember {
        profile: team.profile,
        team_alias: team.team_alias,
        party_id_hex: party_id_hex.to_owned(),
    })
}

pub(super) fn admit_group_operation(
    local: foks_agent_proto::TeamStoreRef,
    remote: foks_agent_proto::TeamStoreRef,
    visibility: i16,
) -> Operation {
    Operation::AdmitFederatedTeam {
        local_profile: local.profile,
        local_team_alias: local.team_alias,
        remote_profile: remote.profile,
        remote_team_alias: remote.team_alias,
        role: FederationRole::Member,
        visibility,
    }
}

pub(super) fn rerun_group_admission_operation(
    local: foks_agent_proto::TeamStoreRef,
    entry: FederationEntryDto,
) -> Result<Operation, AgentError> {
    let visibility = match (entry.destination.role, entry.destination.visibility) {
        ("Member", Some(visibility)) => visibility,
        _ => {
            return Err(invalid_response(
                "The federation destination role is unsupported.",
            ));
        }
    };
    Ok(Operation::AdmitFederatedTeam {
        local_profile: local.profile,
        local_team_alias: local.team_alias,
        remote_profile: entry.remote_profile,
        remote_team_alias: entry.remote_team_alias,
        role: FederationRole::Member,
        visibility,
    })
}

#[tauri::command]
pub async fn discover_groups(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
) -> Result<GroupDiscoveryDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let account_alias = bounded_local_name(&account_alias, "Choose a valid account alias.")?;
    let expected = account_alias.clone();
    let operation = Operation::DiscoverTeams {
        profile: profile.clone(),
        account_alias,
    };
    let value =
        apply_profile_operation_value(&state, profile, operation, MutationKind::Resume).await?;
    require_nested_response_row_cap(&value, "teams", "discovered groups")
        .map_err(|error| ambiguous_mutation_response(&state, error.message))?;
    let response: GroupDiscoveryResponse = serde_json::from_value(value).map_err(|error| {
        ambiguous_mutation_response(&state, format!("Invalid group discovery response: {error}"))
    })?;
    GroupDiscoveryDto::from_response(&expected, response)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn list_group_details(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
) -> Result<GroupDetailsDto, AgentError> {
    require_main_window(&webview)?;
    load_group_details(&state, store_id, state.agent.transport(), true).await
}

pub(super) async fn load_group_details(
    state: &AppState,
    store_id: String,
    transport: std::sync::Arc<dyn foks_desktop::AgentTransport>,
    retain: bool,
) -> Result<GroupDetailsDto, AgentError> {
    let generation = state.catalog_generation.load(Ordering::Acquire);
    let (profile, team_alias) = state.selected_team(&store_id)?;
    let expected_profile = profile.clone();
    let expected_team_alias = team_alias.clone();
    let result_store = store_id.clone();
    let details = tauri::async_runtime::spawn_blocking(move || {
        let value = transport
            .call(Operation::ListTeamDetails {
                profile,
                team_alias,
            })
            .map_err(AgentError::from_desktop)?;
        let details: TeamDetailsSummary =
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
        let parties = group_detail_result(details.members, |value| {
            require_response_row_cap(&value, "group roster entries")?;
            let members: Vec<MemberResponse> = serde_json::from_value(value)
                .map_err(|error| invalid_response(error.to_string()))?;
            party_dtos(&result_store, members)
        })?;
        let federation = group_detail_result(details.federation, |value| {
            require_response_row_cap(&value, "federation entries")?;
            let memberships: Vec<FederationResponse> = serde_json::from_value(value)
                .map_err(|error| invalid_response(error.to_string()))?;
            federation_dtos(
                &result_store,
                &expected_profile,
                &expected_team_alias,
                memberships,
            )
        })?;
        Ok::<_, AgentError>(GroupDetailsDto {
            parties,
            federation,
        })
    })
    .await
    .map_err(|error| AgentError::unknown(format!("failed to load group details: {error}")))??;
    let parties = match &details.parties {
        GroupDetailResultDto::Success { value } => Some(value.as_slice()),
        GroupDetailResultDto::Error { .. } => None,
    };
    let federation = match &details.federation {
        GroupDetailResultDto::Success { value } => Some(value.as_slice()),
        GroupDetailResultDto::Error { .. } => None,
    };
    if retain {
        state.retain_group_details(generation, store_id, parties, federation)?;
    }
    Ok(details)
}

#[tauri::command]
pub async fn list_parties(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
) -> Result<Vec<PartyDto>, AgentError> {
    require_main_window(&webview)?;
    let generation = state.catalog_generation.load(Ordering::Acquire);
    let (profile, team_alias) = state.selected_team(&store_id)?;
    let transport = state.agent.transport();
    let cache_key = store_id.clone();
    let parties = tauri::async_runtime::spawn_blocking(move || {
        let value = transport
            .call(Operation::ListTeamMembers {
                profile,
                team_alias,
            })
            .map_err(AgentError::from_desktop)?;
        require_response_row_cap(&value, "group roster entries")?;
        let members: Vec<MemberResponse> =
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
        party_dtos(&store_id, members)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("failed to load group roster: {error}")))??;
    state.retain_roster(generation, cache_key, &parties)?;
    Ok(parties)
}

#[tauri::command]
pub async fn list_federation(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
) -> Result<Vec<FederationEntryDto>, AgentError> {
    require_main_window(&webview)?;
    let generation = state.catalog_generation.load(Ordering::Acquire);
    let (profile, team_alias) = state.selected_team(&store_id)?;
    let transport = state.agent.transport();
    let cache_key = store_id.clone();
    let expected_profile = profile.clone();
    let expected_team_alias = team_alias.clone();
    let entries = tauri::async_runtime::spawn_blocking(move || {
        let value = transport
            .call(Operation::ListFederatedTeams {
                profile,
                team_alias,
            })
            .map_err(AgentError::from_desktop)?;
        require_response_row_cap(&value, "federation entries")?;
        let memberships: Vec<FederationResponse> =
            serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
        federation_dtos(
            &store_id,
            &expected_profile,
            &expected_team_alias,
            memberships,
        )
    })
    .await
    .map_err(|error| {
        AgentError::unknown(format!("failed to load federation entries: {error}"))
    })??;
    state.retain_federation(generation, cache_key, &entries)?;
    Ok(entries)
}

#[tauri::command]
pub async fn create_group(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    account_store_id: String,
    team_alias: String,
    name: String,
    kind: GroupKindInput,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let account = state.selected_account(&account_store_id)?;
    let operation = create_group_operation(account, &team_alias, &name, kind)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(&state, operation, MutationKind::Create).await
}

#[tauri::command]
pub async fn add_group_member(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    username: String,
    destination: RoleInput,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let team = state.selected_active_team_for_mutation(&store_id)?;
    let operation = add_group_member_operation(team, &username, destination)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(&state, operation, MutationKind::Create).await
}

#[tauri::command]
pub async fn resume_group_member_addition(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    username: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let team = state.selected_active_team_for_mutation(&store_id)?;
    let operation = Operation::ResumeTeamMemberAddition {
        profile: team.profile,
        team_alias: team.team_alias,
        username: required_field(&username, "Enter a member username.")?,
    };
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(&state, operation, MutationKind::Resume).await
}

#[tauri::command]
pub async fn demote_group_member(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    username: String,
    destination: RoleInput,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let (_permit, operation) =
        prepare_group_mutation(&state, &store_id, GroupMutationFacts::Members, || {
            let (team, party_id_hex, current) =
                state.selected_member_target(&store_id, &username)?;
            demote_group_member_operation(team, &party_id_hex, current, destination)
        })
        .await?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(&state, operation, MutationKind::Guarded).await
}

#[tauri::command]
pub async fn remove_group_member(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    username: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let (_permit, operation) =
        prepare_group_mutation(&state, &store_id, GroupMutationFacts::Members, || {
            let (team, party_id_hex, _) = state.selected_member_target(&store_id, &username)?;
            remove_group_member_operation(team, &party_id_hex)
        })
        .await?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(&state, operation, MutationKind::Guarded).await
}

#[tauri::command]
pub async fn resume_group_member_edit(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let team = state.selected_active_team_for_mutation(&store_id)?;
    let operation = Operation::ResumeTeamMemberEdit {
        profile: team.profile,
        team_alias: team.team_alias,
    };
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(&state, operation, MutationKind::Resume).await
}

#[tauri::command]
pub async fn admit_group(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    remote_store_id: String,
    visibility: i16,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let local = state.selected_active_team_for_mutation(&store_id)?;
    let remote = state.selected_remote_named_team(&local.profile, &remote_store_id)?;
    let operation = admit_group_operation(local, remote, visibility);
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(&state, operation, MutationKind::Create).await
}

#[tauri::command]
pub async fn rerun_group_admission(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    operation_id: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let (_permit, operation) =
        prepare_group_mutation(&state, &store_id, GroupMutationFacts::Federation, || {
            let (local, entry) = state.selected_inactive_admission(&store_id, &operation_id)?;
            rerun_group_admission_operation(local, entry)
        })
        .await?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(&state, operation, MutationKind::Resume).await
}

#[tauri::command]
pub async fn expel_federated_group(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    remote_host_id_hex: String,
    remote_team_id_hex: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let (_permit, (team, entry)) = prepare_group_mutation(
        &state,
        &store_id,
        GroupMutationFacts::FederationRemoval,
        || {
            state.selected_active_federation_target(
                &store_id,
                &remote_host_id_hex,
                &remote_team_id_hex,
            )
        },
    )
    .await?;
    if entry.remote_host_id_hex != remote_host_id_hex
        || entry.remote_team_id_hex != remote_team_id_hex
    {
        return Err(invalid_request(
            "The federated group changed before removal. Refresh and try again.",
        ));
    }
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(
        &state,
        Operation::ExpelFederatedTeam {
            profile: team.profile,
            team_alias: team.team_alias,
            remote_host_id_hex,
            remote_team_id_hex,
        },
        MutationKind::Guarded,
    )
    .await
}

#[tauri::command]
pub async fn resume_group_creation(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let (store, active) = state.selected_store(&store_id)?;
    let CatalogStoreRef::Team(store) = store else {
        return Err(invalid_request("Only pending group stores can be resumed."));
    };
    if active != Some(false) {
        return Err(invalid_request(
            "This group has no incomplete creation to resume.",
        ));
    }
    state.ensure_profile_available(&store.profile)?;
    let operation = Operation::ResumeTeamCreation {
        profile: store.profile,
        team_alias: store.team_alias,
    };
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(&state, operation, MutationKind::Resume).await
}

/// Forgets a group whose creation never completed. Only a store the catalog
/// reports as inactive qualifies, which is the gate resuming uses too: a group
/// that finished creating is left to its own server-side removal paths.
#[tauri::command]
pub async fn abandon_group_creation(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let (store, active) = state.selected_store(&store_id)?;
    let CatalogStoreRef::Team(store) = store else {
        return Err(invalid_request(
            "Only pending group stores can be removed this way.",
        ));
    };
    if active != Some(false) {
        return Err(invalid_request(
            "This group has no incomplete creation to remove.",
        ));
    }
    state.ensure_profile_available(&store.profile)?;
    let operation = Operation::AbandonTeamCreation {
        profile: store.profile,
        team_alias: store.team_alias,
    };
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_operation(&state, operation, MutationKind::Guarded).await
}
