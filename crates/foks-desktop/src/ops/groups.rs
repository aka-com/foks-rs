//! Team creation and membership operations.

use foks_agent_proto::{AccountStoreRef, Operation, TeamKind, TeamRole, TeamStoreRef};

use super::{required_field, typed_entity_id_hex, InvalidInput};

const USER_ID_PREFIX: &str = "01";

/// The local selector the agent resolves a team membership operation against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalTeam {
    pub profile: String,
    pub team_alias: String,
}

impl From<TeamStoreRef> for LocalTeam {
    fn from(team: TeamStoreRef) -> Self {
        Self {
            profile: team.profile,
            team_alias: team.team_alias,
        }
    }
}

/// Builds `CreateTeam` for a new team owned by `account`.
///
/// A named team requires a display name. An ad-hoc team has no server-side
/// name; its local alias is its user-facing name, so a supplied name is
/// ignored rather than sent.
pub fn create_team_operation(
    account: AccountStoreRef,
    team_alias: &str,
    name: &str,
    kind: TeamKind,
) -> Result<Operation, InvalidInput> {
    let team_alias = required_field(team_alias, "Group alias is required.")?;
    let name = match kind {
        TeamKind::Named => required_field(name, "Group name is required.")?,
        TeamKind::AdHoc => String::new(),
    };
    Ok(Operation::CreateTeam {
        profile: account.profile,
        account_alias: account.account_alias,
        team_alias,
        name,
        kind,
    })
}

/// Builds `AddTeamMember` for a local user named by `username`.
pub fn add_team_member_operation(
    team: LocalTeam,
    username: &str,
    role: TeamRole,
    visibility: i16,
) -> Result<Operation, InvalidInput> {
    let username = required_field(username, "Username is required.")?;
    member_visibility(role, visibility)?;
    Ok(Operation::AddTeamMember {
        profile: team.profile,
        team_alias: team.team_alias,
        username,
        role,
        visibility,
    })
}

/// Builds `PromoteTeamMember`. Whether the destination is above the member's
/// current role is checked by callers that hold the authenticated roster.
pub fn promote_team_member_operation(
    team: LocalTeam,
    party_id_hex: &str,
    role: TeamRole,
    visibility: i16,
) -> Result<Operation, InvalidInput> {
    let party_id_hex = user_party_id(party_id_hex)?;
    member_visibility(role, visibility)?;
    Ok(Operation::PromoteTeamMember {
        profile: team.profile,
        team_alias: team.team_alias,
        party_id_hex,
        role,
        visibility,
    })
}

/// Builds `DemoteTeamMember`. Whether the destination is below the member's
/// current role is checked by callers that hold the authenticated roster.
pub fn demote_team_member_operation(
    team: LocalTeam,
    party_id_hex: &str,
    role: TeamRole,
    visibility: i16,
) -> Result<Operation, InvalidInput> {
    let party_id_hex = user_party_id(party_id_hex)?;
    member_visibility(role, visibility)?;
    Ok(Operation::DemoteTeamMember {
        profile: team.profile,
        team_alias: team.team_alias,
        party_id_hex,
        role,
        visibility,
    })
}

pub fn remove_team_member_operation(
    team: LocalTeam,
    party_id_hex: &str,
) -> Result<Operation, InvalidInput> {
    Ok(Operation::RemoveTeamMember {
        profile: team.profile,
        team_alias: team.team_alias,
        party_id_hex: user_party_id(party_id_hex)?,
    })
}

/// Membership edits name the member by its authenticated user party id, never
/// by a username the agent would have to resolve again.
fn user_party_id(party_id_hex: &str) -> Result<String, InvalidInput> {
    typed_entity_id_hex(party_id_hex, USER_ID_PREFIX, "Select a valid user.")
}

/// Only the member role carries a visibility level; the agent rejects one on
/// the admin and owner roles.
fn member_visibility(role: TeamRole, visibility: i16) -> Result<(), InvalidInput> {
    if role != TeamRole::Member && visibility != 0 {
        return Err(InvalidInput("Visibility applies only to the member role."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn account() -> AccountStoreRef {
        AccountStoreRef {
            profile: "work".to_owned(),
            account_alias: "personal".to_owned(),
        }
    }

    fn team() -> LocalTeam {
        LocalTeam {
            profile: "work".to_owned(),
            team_alias: "engineering".to_owned(),
        }
    }

    #[test]
    fn named_teams_require_a_trimmed_name_and_alias() {
        assert_eq!(
            create_team_operation(account(), " engineering ", " Engineering ", TeamKind::Named)
                .unwrap(),
            Operation::CreateTeam {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "engineering".to_owned(),
                name: "Engineering".to_owned(),
                kind: TeamKind::Named,
            }
        );
        assert_eq!(
            create_team_operation(account(), "engineering", "  ", TeamKind::Named)
                .unwrap_err()
                .message(),
            "Group name is required."
        );
        assert_eq!(
            create_team_operation(account(), " ", "Engineering", TeamKind::Named)
                .unwrap_err()
                .message(),
            "Group alias is required."
        );
    }

    #[test]
    fn ad_hoc_teams_ignore_a_supplied_name() {
        assert_eq!(
            create_team_operation(account(), "weekend", "Weekend project", TeamKind::AdHoc)
                .unwrap(),
            Operation::CreateTeam {
                profile: "work".to_owned(),
                account_alias: "personal".to_owned(),
                team_alias: "weekend".to_owned(),
                name: String::new(),
                kind: TeamKind::AdHoc,
            }
        );
    }

    #[test]
    fn membership_edits_require_an_authenticated_user_party_id() {
        for party_id_hex in [
            "satoshi",
            // A named-team id has the right length but the wrong type prefix.
            "03aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "01AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert_eq!(
                remove_team_member_operation(team(), party_id_hex).unwrap_err(),
                InvalidInput("Select a valid user.")
            );
            assert!(
                promote_team_member_operation(team(), party_id_hex, TeamRole::Admin, 0).is_err()
            );
            assert!(
                demote_team_member_operation(team(), party_id_hex, TeamRole::Member, 0).is_err()
            );
        }
        assert_eq!(
            remove_team_member_operation(team(), USER).unwrap(),
            Operation::RemoveTeamMember {
                profile: "work".to_owned(),
                team_alias: "engineering".to_owned(),
                party_id_hex: USER.to_owned(),
            }
        );
        assert_eq!(
            demote_team_member_operation(team(), USER, TeamRole::Member, -2).unwrap(),
            Operation::DemoteTeamMember {
                profile: "work".to_owned(),
                team_alias: "engineering".to_owned(),
                party_id_hex: USER.to_owned(),
                role: TeamRole::Member,
                visibility: -2,
            }
        );
    }

    #[test]
    fn visibility_applies_only_to_the_member_role() {
        let error = InvalidInput("Visibility applies only to the member role.");
        assert_eq!(
            add_team_member_operation(team(), "jules", TeamRole::Admin, 3).unwrap_err(),
            error
        );
        assert_eq!(
            promote_team_member_operation(team(), USER, TeamRole::Owner, -1).unwrap_err(),
            error
        );
        assert_eq!(
            add_team_member_operation(team(), " jules ", TeamRole::Member, -3).unwrap(),
            Operation::AddTeamMember {
                profile: "work".to_owned(),
                team_alias: "engineering".to_owned(),
                username: "jules".to_owned(),
                role: TeamRole::Member,
                visibility: -3,
            }
        );
        assert_eq!(
            add_team_member_operation(team(), " ", TeamRole::Member, 0)
                .unwrap_err()
                .message(),
            "Username is required."
        );
    }
}
