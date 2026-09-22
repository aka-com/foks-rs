//! Public request and result types for team membership rotation.

use foks_proto::{EntityId, Role, SecretSeed, TreeRoot};
use foks_verify::{VerifiedSharedKey, VerifiedTeamState, VerifiedUserState};

use super::super::AuthenticatedTeamOutcome;
use crate::{Error, FoksClient, PinnedHost, Result};

/// Caller-durable new PTK seeds. Every role listed by the authenticated
/// removal schedule must appear exactly once.
pub struct TeamPtkRotationSeed<'a> {
    pub role: Role,
    pub seed: &'a SecretSeed,
}

pub struct TeamMutationRecovery<'a> {
    pub team: &'a EntityId,
    pub expected_seqno: u64,
    pub expected_operation_id: &'a [u8; 16],
}

/// Inputs retained by the caller's encrypted secret store before submission.
///
/// `remaining_users` supplies authenticated PUK keys for every remaining
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

/// Exact federated-member expulsion using the admission-time removal key
/// retained by the local encrypted vault. The host is mandatory in `target`;
/// callers cannot collapse a fully-qualified federation row to a bare TeamID.
pub struct RetainedTeamMemberRemovalRequest<'a> {
    pub target: TeamMemberSelector<'a>,
    pub removal_key: &'a SecretSeed,
    pub rotations: &'a [TeamPtkRotationSeed<'a>],
    pub remaining_parties: &'a [VerifiedMemberParty<'a>],
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
    RemoteUser(&'a VerifiedRemoteUserRecipient),
    Team(&'a VerifiedTeamRecipient),
}

/// A remote user projection bound to the independently authenticated current
/// head returned by its host's remote-chain loader.
#[derive(Clone, Debug)]
pub struct VerifiedRemoteUserRecipient {
    user: VerifiedUserState,
    authenticated_root: TreeRoot,
    authenticated_heads: Vec<(PinnedHost, TreeRoot)>,
}

impl VerifiedRemoteUserRecipient {
    pub(crate) fn new(
        user: VerifiedUserState,
        authenticated_root: TreeRoot,
        authoritative_host: PinnedHost,
    ) -> Result<Self> {
        if user.tree_root() != authenticated_root || user.host() != authoritative_host.host_id() {
            return Err(Error::UserBinding(
                "remote user recipient is not at its authenticated Merkle head",
            ));
        }
        Ok(Self {
            user,
            authenticated_root: authenticated_root.clone(),
            authenticated_heads: vec![(authoritative_host, authenticated_root)],
        })
    }

    pub fn verified(&self) -> &VerifiedUserState {
        &self.user
    }
}

/// A team recipient whose complete direct roster was independently checked
/// against current authenticated user/team projections. Recursively requiring
/// this witness for child teams prevents a parent team member-key refresh from boxing new PTKs to
/// a team key that is still exposed through a stale descendant PUK/PTK.
#[derive(Clone, Debug)]
pub struct VerifiedTeamRecipient {
    team: VerifiedTeamState,
    authenticated_root: TreeRoot,
    authenticated_heads: Vec<(PinnedHost, TreeRoot)>,
    recursively_authenticated: bool,
}

impl VerifiedTeamRecipient {
    pub(crate) fn new(
        team: VerifiedTeamState,
        authenticated_root: TreeRoot,
        authoritative_host: PinnedHost,
        direct_parties: &[VerifiedMemberParty<'_>],
    ) -> Result<Self> {
        if team.tree_root() != authenticated_root || team.host() != authoritative_host.host_id() {
            return Err(Error::TeamBinding(
                "team recipient is not at its authenticated Merkle head",
            ));
        }
        let mut authenticated_heads = std::collections::BTreeMap::new();
        authenticated_heads.insert(
            authoritative_host.host_id().as_bytes().to_vec(),
            (authoritative_host, authenticated_root.clone()),
        );
        let mut supplied = std::collections::BTreeSet::new();
        for party in direct_parties {
            if !supplied.insert((
                party.party().as_bytes().to_vec(),
                party.host().as_bytes().to_vec(),
            )) {
                return Err(Error::TeamBinding(
                    "team recipient direct party is duplicated",
                ));
            }
            for (host, root) in party.authenticated_heads() {
                match authenticated_heads.get(host.host_id().as_bytes()) {
                    Some((_, known)) if known != root => {
                        return Err(Error::TeamBinding(
                            "team recipient projections disagree on an authoritative Merkle head",
                        ));
                    }
                    Some(_) => {}
                    None => {
                        authenticated_heads.insert(
                            host.host_id().as_bytes().to_vec(),
                            (host.clone(), root.clone()),
                        );
                    }
                }
            }
        }
        let mut used = std::collections::BTreeSet::new();
        for member in team.members() {
            let matching = direct_parties
                .iter()
                .copied()
                .filter(|party| {
                    party.party() == &member.party
                        && member
                            .scoped_host
                            .as_ref()
                            .is_none_or(|host| party.host() == host)
                        && (member.scoped_host.is_some() || party.host() == team.host())
                })
                .collect::<Vec<_>>();
            let [party] = matching.as_slice() else {
                return Err(Error::TeamBinding(
                    "team recipient roster is not exactly authenticated",
                ));
            };
            if !party.matches_authoritative_root(team.host(), &authenticated_root)
                || party.has_stale_shared_key(member.source_role)
            {
                return Err(Error::TeamBinding(
                    "team recipient has a stale direct or descendant key",
                ));
            }
            party
                .shared_key(member.source_role)
                .filter(|key| {
                    key.generation == member.generation
                        && key.verify_key == member.verify_key
                        && foks_crypto::hepk_fingerprint(&key.hepk).ok()
                            == Some(member.hepk_fingerprint)
                })
                .ok_or(Error::TeamBinding(
                    "team recipient roster key is not current",
                ))?;
            used.insert((
                party.party().as_bytes().to_vec(),
                party.host().as_bytes().to_vec(),
            ));
        }
        if used != supplied {
            return Err(Error::TeamBinding(
                "team recipient includes a party outside its current roster",
            ));
        }
        Ok(Self {
            team,
            authenticated_root,
            authenticated_heads: authenticated_heads.into_values().collect(),
            recursively_authenticated: true,
        })
    }

    /// Builds a recipient from a latest-head public team load authorized by a
    /// local parent. The parent needs the child team's current public PTK to
    /// detect and propagate a child rotation, but this witness deliberately
    /// carries no child private-key or acting authority. Descendant response
    /// remains the responsibility of that child's own administrators.
    pub(crate) fn from_latest_public_team(
        team: VerifiedTeamState,
        authenticated_root: TreeRoot,
        authoritative_host: PinnedHost,
    ) -> Result<Self> {
        if team.tree_root() != authenticated_root || team.host() != authoritative_host.host_id() {
            return Err(Error::TeamBinding(
                "public team recipient is not at its authenticated Merkle head",
            ));
        }
        Ok(Self {
            team,
            authenticated_root: authenticated_root.clone(),
            authenticated_heads: vec![(authoritative_host, authenticated_root)],
            recursively_authenticated: false,
        })
    }

    pub fn verified(&self) -> &VerifiedTeamState {
        &self.team
    }

    pub fn is_recursively_authenticated(&self) -> bool {
        self.recursively_authenticated
    }
}

impl<'a> VerifiedMemberParty<'a> {
    fn authenticated_heads(self) -> &'a [(PinnedHost, TreeRoot)] {
        match self {
            Self::User(_) => &[],
            Self::RemoteUser(user) => &user.authenticated_heads,
            Self::Team(team) => &team.authenticated_heads,
        }
    }

    pub(super) fn ensure_current_heads(self, client: &FoksClient) -> Result<()> {
        for (host, expected) in self.authenticated_heads() {
            let (_, latest) = client.advance_merkle_root(host)?;
            let current = TreeRoot {
                epoch: latest.root().epoch,
                hash: latest
                    .authenticated_roots()
                    .root_hash(latest.root().epoch)
                    .ok_or(Error::TeamBinding(
                        "recipient host Merkle head is not authenticated",
                    ))?,
            };
            if current != *expected {
                return Err(Error::TeamBinding(
                    "recipient projection is no longer at its host's current Merkle head",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn party(self) -> &'a EntityId {
        match self {
            Self::User(user) => user.uid(),
            Self::RemoteUser(user) => user.user.uid(),
            Self::Team(team) => team.team.team(),
        }
    }

    pub(super) fn host(self) -> &'a EntityId {
        match self {
            Self::User(user) => user.host(),
            Self::RemoteUser(user) => user.user.host(),
            Self::Team(team) => team.team.host(),
        }
    }

    pub(super) fn shared_key(self, role: Role) -> Option<&'a VerifiedSharedKey> {
        match self {
            Self::User(user) => user.shared_key(role),
            Self::RemoteUser(user) => user.user.shared_key(role),
            Self::Team(team) => team.team.shared_key(role),
        }
    }

    pub(super) fn index_range(self) -> Option<&'a foks_proto::RationalRange> {
        match self {
            Self::Team(team) => Some(team.team.index_range()),
            Self::User(_) | Self::RemoteUser(_) => None,
        }
    }

    pub(super) fn matches_authoritative_root(
        self,
        local_host: &EntityId,
        root: &foks_proto::TreeRoot,
    ) -> bool {
        match self {
            Self::User(user) => user.host() == local_host && user.tree_root() == *root,
            Self::RemoteUser(user) => {
                user.user.tree_root() == user.authenticated_root
                    && (user.user.host() != local_host || user.authenticated_root == *root)
            }
            Self::Team(team) => {
                team.team.tree_root() == team.authenticated_root
                    && (team.team.host() != local_host || team.authenticated_root == *root)
            }
        }
    }

    pub(super) fn has_stale_shared_key(self, role: Role) -> bool {
        match self {
            Self::User(user) => user.stale_shared_key_roles().contains(&role),
            Self::RemoteUser(user) => user.user.stale_shared_key_roles().contains(&role),
            Self::Team(team) => !team.recursively_authenticated,
        }
    }

    pub(super) fn key_at(
        self,
        role: Role,
        generation: u64,
        verify_key: &EntityId,
        hepk_fingerprint: [u8; 32],
    ) -> Option<&'a VerifiedSharedKey> {
        match self {
            Self::User(user) => user.shared_key_history().iter().find(|key| {
                key.role == role
                    && key.generation == generation
                    && key.verify_key == *verify_key
                    && foks_crypto::hepk_fingerprint(&key.hepk).ok() == Some(hepk_fingerprint)
            }),
            Self::RemoteUser(user) => user.user.shared_key_history().iter().find(|key| {
                key.role == role
                    && key.generation == generation
                    && key.verify_key == *verify_key
                    && foks_crypto::hepk_fingerprint(&key.hepk).ok() == Some(hepk_fingerprint)
            }),
            Self::Team(team) => team.team.shared_key(role).filter(|key| {
                key.generation == generation
                    && key.verify_key == *verify_key
                    && foks_crypto::hepk_fingerprint(&key.hepk).ok() == Some(hepk_fingerprint)
            }),
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
    /// Complete post-transition roster entries except the acting user and
    /// changed party, both of which are supplied by the authenticated session
    /// and `replacement` respectively.
    pub remaining_parties: &'a [VerifiedMemberParty<'a>],
}

#[derive(Clone, Copy)]
pub struct TeamMemberKeyRefresh<'a> {
    pub target: TeamMemberSelector<'a>,
    pub destination_role: Role,
    /// Required for a fresh submission. Crash reconciliation binds directly
    /// to the stored public key fields and does not require the party to remain
    /// in the current roster.
    pub replacement: Option<VerifiedMemberParty<'a>>,
    pub replacement_generation: u64,
    pub replacement_verify_key: &'a EntityId,
    pub replacement_hepk_fingerprint: [u8; 32],
}

/// One atomic team member-key refresh transition for every stale roster row observed in the same
/// authenticated team view. The PTK role list is the union required by all
/// replacements, and no newly rotated PTK is boxed to a retired roster key.
pub struct RefreshTeamMemberKeysRequest<'a> {
    pub expected_seqno: u64,
    pub changes: &'a [TeamMemberKeyRefresh<'a>],
    pub rotations: &'a [TeamPtkRotationSeed<'a>],
    /// Complete pre-transition roster entries except the acting user and all
    /// changed parties. Each changed party is supplied by its replacement.
    pub remaining_parties: &'a [VerifiedMemberParty<'a>],
}

pub struct RotatedTeamPtks {
    pub operation_id: [u8; 16],
    pub expected_seqno: u64,
    pub authenticated: AuthenticatedTeamOutcome,
}
