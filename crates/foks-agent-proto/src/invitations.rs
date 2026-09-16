//! Bounded native invitation commands; request IDs are opaque local handles.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvitationRole {
    Member { visibility: i16 },
    Admin,
    Owner,
}
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum InvitationAction {
    AcceptTeam {
        invite: String,
        source_team_alias: String,
        source_role: InvitationRole,
    },
    AcceptTeamRemote {
        remote_profile: String,
        invite: String,
        source_team_alias: String,
        source_role: InvitationRole,
    },
    Range {
        team_alias: String,
        raise: bool,
    },
    PendingApprovals {
        team_alias: String,
    },

    PreviewRemote {
        remote_profile: String,
        invite: String,
    },
    AcceptRemote {
        remote_profile: String,
        invite: String,
    },
    AttemptRemote {
        remote_profile: String,
        operation_id: String,
    },
    StatusRemote {
        remote_profile: String,
        operation_id: String,
    },
    InspectRemote {
        remote_profile: String,
        team_alias: String,
        request_id: String,
    },
    ApproveRemote {
        remote_profile: String,
        team_alias: String,
        request_id: String,
        role: InvitationRole,
    },
    SyncRemote {
        remote_profile: String,
        team_id: String,
        #[serde(default)]
        source_team_alias: Option<String>,
        #[serde(default)]
        source_role: Option<InvitationRole>,
    },

    Preview {
        invite: String,
    },
    Create {
        team_alias: String,
    },
    Accept {
        invite: String,
    },
    Attempt {
        operation_id: String,
    },
    Status {
        operation_id: String,
    },
    Cancel {
        operation_id: String,
    },
    List,
    Inbox {
        team_alias: String,
    },
    Approve {
        team_alias: String,
        request_id: String,
        role: InvitationRole,
    },
    Reject {
        team_alias: String,
        request_id: String,
    },
}
impl std::fmt::Debug for InvitationAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InvitationAction { [REDACTED] }")
    }
}
impl InvitationAction {
    /// Whether this action can change account/team inventory or an operation's
    /// durable outcome. Inspection still requires the agent's profile lock.
    pub fn changes_catalog(&self) -> bool {
        match self {
            Self::List
            | Self::PendingApprovals { .. }
            | Self::Preview { .. }
            | Self::PreviewRemote { .. }
            | Self::InspectRemote { .. }
            | Self::Inbox { .. }
            | Self::Range { raise: false, .. } => false,
            Self::AcceptTeam { .. }
            | Self::AcceptTeamRemote { .. }
            | Self::Range { raise: true, .. }
            | Self::AcceptRemote { .. }
            | Self::AttemptRemote { .. }
            | Self::StatusRemote { .. }
            | Self::ApproveRemote { .. }
            | Self::SyncRemote { .. }
            | Self::Create { .. }
            | Self::Accept { .. }
            | Self::Attempt { .. }
            | Self::Status { .. }
            | Self::Cancel { .. }
            | Self::Approve { .. }
            | Self::Reject { .. } => true,
        }
    }

    pub fn remote_profile(&self) -> Option<&str> {
        match self {
            Self::AcceptTeamRemote { remote_profile, .. }
            | Self::PreviewRemote { remote_profile, .. }
            | Self::AcceptRemote { remote_profile, .. }
            | Self::AttemptRemote { remote_profile, .. }
            | Self::StatusRemote { remote_profile, .. }
            | Self::InspectRemote { remote_profile, .. }
            | Self::ApproveRemote { remote_profile, .. }
            | Self::SyncRemote { remote_profile, .. } => Some(remote_profile),
            _ => None,
        }
    }

    pub fn validate(&self) -> bool {
        let id = |s: &str| {
            s.len() == 32
                && s.bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        };
        let name = |s: &str| !s.is_empty() && s.len() <= 128;
        if self.remote_profile().is_some_and(|p| !name(p)) {
            return false;
        }
        match self {
            Self::AcceptTeam {
                invite,
                source_team_alias,
                ..
            }
            | Self::AcceptTeamRemote {
                invite,
                source_team_alias,
                ..
            } => {
                name(source_team_alias)
                    && !invite.is_empty()
                    && invite.len() <= 256
                    && invite.bytes().all(|c| c.is_ascii_alphanumeric())
            }
            Self::Range { team_alias, .. } | Self::PendingApprovals { team_alias } => {
                name(team_alias)
            }
            Self::PreviewRemote { invite, .. } | Self::AcceptRemote { invite, .. } => {
                !invite.is_empty()
                    && invite.len() <= 256
                    && invite.bytes().all(|c| c.is_ascii_alphanumeric())
            }
            Self::AttemptRemote { operation_id, .. } | Self::StatusRemote { operation_id, .. } => {
                id(operation_id)
            }
            Self::InspectRemote {
                team_alias,
                request_id,
                ..
            }
            | Self::ApproveRemote {
                team_alias,
                request_id,
                ..
            } => name(team_alias) && id(request_id),
            Self::SyncRemote { team_id, .. } => {
                team_id.len() == 66 && team_id.bytes().all(|c| c.is_ascii_hexdigit())
            }
            Self::Preview { invite } | Self::Accept { invite } => {
                !invite.is_empty()
                    && invite.len() <= 256
                    && invite.bytes().all(|c| c.is_ascii_alphanumeric())
            }
            Self::Create { team_alias } | Self::Inbox { team_alias } => name(team_alias),
            Self::Attempt { operation_id }
            | Self::Status { operation_id }
            | Self::Cancel { operation_id } => id(operation_id),
            Self::Approve {
                team_alias,
                request_id,
                ..
            }
            | Self::Reject {
                team_alias,
                request_id,
            } => name(team_alias) && id(request_id),
            Self::List => true,
        }
    }
}
