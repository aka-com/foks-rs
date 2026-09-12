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
    pub fn validate(&self) -> bool {
        let id = |s: &str| {
            s.len() == 32
                && s.bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        };
        let name = |s: &str| !s.is_empty() && s.len() <= 128;
        match self {
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
