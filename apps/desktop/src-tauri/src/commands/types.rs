//! Wire types shared by more than one command domain.

use foks_agent_proto::KvRole;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDto {
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<i16>,
}

impl From<KvRole> for RoleDto {
    fn from(role: KvRole) -> Self {
        match role {
            KvRole::Member { visibility } => Self {
                role: "Member",
                visibility: Some(visibility),
            },
            KvRole::Admin => Self {
                role: "Admin",
                visibility: None,
            },
            KvRole::Owner => Self {
                role: "Owner",
                visibility: None,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CommandAck {
    pub ok: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MutationDto {
    pub applied: bool,
}
