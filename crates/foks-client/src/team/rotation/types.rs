//! Public request and result types for team membership rotation.

use foks_proto::{EntityId, Role, SecretSeed};
use foks_verify::{VerifiedSharedKey, VerifiedTeamState, VerifiedUserState};

use super::super::AuthenticatedTeamOutcome;

/// Caller-durable new PTK material. Every role listed by the authenticated
/// removal schedule must appear exactly once.
pub struct TeamPtkRotationSeed<'a> {
    pub role: Role,
    pub seed: &'a SecretSeed,
}

/// Inputs retained by the caller's encrypted secret store before submission.
///
/// `remaining_users` supplies authenticated PUK material for every remaining
/// local user except the actor, which is already available from authentication.
pub struct RemoveLocalTeamMemberRequest<'a> {
    pub target_user: &'a EntityId,
    pub removal_key: &'a SecretSeed,
    pub rotations: &'a [TeamPtkRotationSeed<'a>],
    pub remaining_users: &'a [&'a VerifiedUserState],
}

/// Removal inputs when TeamAdmin should retrieve the committed removal key.
pub struct RemoveTeamMemberRequest<'a> {
    pub target: &'a EntityId,
    pub rotations: &'a [TeamPtkRotationSeed<'a>],
    pub remaining_users: &'a [&'a VerifiedUserState],
}

#[derive(Clone, Copy)]
pub struct TeamMemberSelector<'a> {
    pub party: &'a EntityId,
    /// `None` selects a local roster row; federated rows require their exact
    /// authenticated remote HostID.
    pub host: Option<&'a EntityId>,
    pub source_role: Role,
}

#[derive(Clone, Copy)]
pub enum VerifiedMemberParty<'a> {
    User(&'a VerifiedUserState),
    Team(&'a VerifiedTeamState),
}

impl<'a> VerifiedMemberParty<'a> {
    pub(super) fn party(self) -> &'a EntityId {
        match self {
            Self::User(user) => user.uid(),
            Self::Team(team) => team.team(),
        }
    }

    pub(super) fn host(self) -> &'a EntityId {
        match self {
            Self::User(user) => user.host(),
            Self::Team(team) => team.host(),
        }
    }

    pub(super) fn shared_key(self, role: Role) -> Option<&'a VerifiedSharedKey> {
        match self {
            Self::User(user) => user.shared_key(role),
            Self::Team(team) => team.shared_key(role),
        }
    }
}

/// A removal, role demotion, or member credential-generation advance.
/// `replacement` is absent only for removal and otherwise supplies the
/// independently verified current PUK/PTK named by the replacement roster row.
pub struct ChangeTeamMemberRequest<'a> {
    pub target: TeamMemberSelector<'a>,
    pub destination_role: Role,
    pub replacement: Option<VerifiedMemberParty<'a>>,
    pub rotations: &'a [TeamPtkRotationSeed<'a>],
    /// Complete post-transition roster material except the acting user and
    /// changed party, both of which are supplied by the authenticated session
    /// and `replacement` respectively.
    pub remaining_parties: &'a [VerifiedMemberParty<'a>],
}

pub struct RotatedTeamPtks {
    pub operation_id: [u8; 16],
    pub expected_seqno: u64,
    pub authenticated: AuthenticatedTeamOutcome,
}
