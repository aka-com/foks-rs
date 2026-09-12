//! Team loading, PTK recovery, ad-hoc creation, and reconciliation.

mod bearer;
mod invitation_operations;
mod invitations;
pub use invitation_operations::*;
pub use invitations::*;
mod membership;
mod metadata;
mod named;
mod rotation;

pub use membership::*;
pub use metadata::*;
pub use named::*;
pub use rotation::*;

use std::time::Duration;

use foks_client_db::{
    Acceptance, AdHocTeamOperation, AdHocTeamOperationState, HardStateStore,
    VerifiedUserGenericChainSnapshot,
};
use foks_crypto::{
    adhoc_team_id_from_admin_seed, derive_subkey_id, hepk_fingerprint,
    make_single_owner_adhoc_team, make_single_owner_adhoc_team_yubi, open_shared_key_parcel_with,
    prefixed_hash, seal_shared_key_boxes, sign_shared_key_typed, AdHocTeamInput, AdHocTeamMaterial,
    PukBoxRandomness, SharedKeyBoxInput, SharedKeyDecapsulator,
};
use foks_proto::{
    ActivatedTeamView, AdHocTeamCreateArgument, ChangeMetadata, EntityId, GenericChain,
    GenericLinkPayload, HostConfig, PassphraseInfo, Role, SecretSeed, TeamChain,
    TeamMembershipPayload, TeamMembershipState, TeamViewChallenge, TeamViewRequest,
    UnsignedUserLink, UserLink, UserSettingsLinkPublic, ViewershipMode, CHAIN_TYPE_TEAM_MEMBERSHIP,
    CHAIN_TYPE_USER_SETTINGS, ENTITY_PTK_VERIFY, ENTITY_PUK_VERIFY, LINK_OUTER_TYPE_ID,
    LINK_OUTER_V1_TYPE_ID, MERKLE_ROOT_TYPE_ID, TEAM_VIEW_CHALLENGE_TYPE_ID, TREE_LOCATION_TYPE_ID,
};
use foks_rpc::{
    encode_activate_team_view_request, encode_create_adhoc_team_request,
    encode_get_host_config_request, encode_get_team_list_server_trust_request,
    encode_load_generic_chain_request, encode_load_team_chain_for_local_parent_request,
    encode_load_team_chain_request_from, encode_load_team_membership_chain_request,
    encode_post_generic_link_request, encode_post_team_membership_link_request,
    encode_team_view_challenge_request, TeamChainLoadOptions, STATUS_TX_RETRY_ERROR,
};
use foks_verify::{
    verify_merkle_path, verify_team_chain, verify_team_chain_increment, AuthenticatedMerkleRoots,
    UserDeviceSigningBookends, VerifiedTeamState, VerifiedUserState,
};

use crate::{
    current_owner_puk, now_microseconds, now_milliseconds, random_bytes,
    require_nonstale_shared_key, user_key_history_for_seed, AuthenticatedUserOutcome,
    DeviceCredential, Error, FederationCredential, FoksClient, PinnedHost, Result, UserPrivateKey,
    YubiCredential, ADHOC_TEAM_OPERATION_ID_TYPE_ID, ADHOC_TEAM_REQUEST_HASH_TYPE_ID,
};

pub struct TeamPrivateKey {
    pub role: Role,
    pub generation: u64,
    pub seed: SecretSeed,
}

pub struct AuthenticatedTeamOutcome {
    pub merkle_acceptance: Acceptance,
    pub acceptance: Acceptance,
    pub verified: VerifiedTeamState,
    pub ptks: Vec<TeamPrivateKey>,
    pub view_token: [u8; 16],
}

impl AuthenticatedTeamOutcome {
    /// Converts this locally loaded current-head team into a recursively
    /// freshness-checked PTK recipient witness.
    pub fn verified_recipient(
        &self,
        authoritative_host: &PinnedHost,
        direct_parties: &[VerifiedMemberParty<'_>],
    ) -> Result<VerifiedTeamRecipient> {
        VerifiedTeamRecipient::new(
            self.verified.clone(),
            self.verified.tree_root(),
            authoritative_host.clone(),
            direct_parties,
        )
    }

    /// Returns whether this authenticated team holds the exact current PTK
    /// named by a parent team's roster row. A loaded public team projection
    /// alone is not acting authority: the transport must also be able to
    /// decrypt the row's source-role key.
    pub fn holds_roster_private_key(&self, member: &foks_verify::VerifiedTeamMemberState) -> bool {
        if member.party != *self.verified.team() || member.scoped_host.is_some() {
            return false;
        }
        let Ok(history) = verified_team_private_history(self) else {
            return false;
        };
        self.ptks.iter().any(|private| {
            team_key_for_seed(&history, private).is_ok_and(|public| {
                public.role == member.source_role
                    && public.generation == member.generation
                    && public.verify_key == member.verify_key
                    && foks_crypto::hepk_fingerprint(&public.hepk).ok()
                        == Some(member.hepk_fingerprint)
            })
        })
    }
}

struct TeamViewActor<'a> {
    party: &'a EntityId,
    host: &'a EntityId,
    source: &'a foks_verify::VerifiedSharedKey,
    history: &'a [foks_verify::VerifiedSharedKey],
    seed: &'a SecretSeed,
}

/// Caller-durable, role-keyed PTK seeds for a single-owner ad-hoc team.
pub struct AdHocTeamSecrets {
    pub member_min: SecretSeed,
    pub member: SecretSeed,
    pub admin: SecretSeed,
    pub owner: SecretSeed,
}

impl AdHocTeamSecrets {
    fn ordered(&self) -> [&SecretSeed; 4] {
        [&self.member_min, &self.member, &self.admin, &self.owner]
    }

    /// Returns the stable TeamID before any network mutation is attempted.
    pub fn team_id(&self) -> Result<EntityId> {
        Ok(adhoc_team_id_from_admin_seed(&self.admin)?)
    }

    /// Stable public journal identifier recoverable from retained PTKs even
    /// when the process loses the create call's return value.
    pub fn operation_id(&self) -> Result<[u8; 16]> {
        let hash = prefixed_hash(ADHOC_TEAM_OPERATION_ID_TYPE_ID, self.team_id()?.as_bytes());
        Ok(hash[..16].try_into().expect("hash prefix has fixed length"))
    }
}

pub struct CreatedAdHocTeam {
    pub operation_id: [u8; 16],
    pub team: EntityId,
    pub authenticated: AuthenticatedTeamOutcome,
}

#[derive(Clone, Copy)]
pub(super) struct MembershipChainTail {
    pub sequence: u64,
    pub previous: Option<[u8; 32]>,
}

struct HistoricalMembershipSigner {
    generic_root: foks_proto::TreeRoot,
    generic_key: [u8; 32],
    generic_value: [u8; 32],
    bookends: UserDeviceSigningBookends,
}

struct VerifiedUserGenericChain {
    tail: MembershipChainTail,
    payloads: Vec<GenericLinkPayload>,
    link_hashes: Vec<[u8; 32]>,
    chain_bytes: Vec<u8>,
    cited_roots: Vec<foks_proto::TreeRoot>,
    historical_signers: Vec<HistoricalMembershipSigner>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedTeamMembershipEvent {
    pub sequence: u64,
    pub membership: TeamMembershipPayload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedTeamMembershipChain {
    pub owner: EntityId,
    pub host: EntityId,
    pub next_sequence: u64,
    pub events: Vec<AuthenticatedTeamMembershipEvent>,
    pub current: Vec<AuthenticatedTeamMembershipEvent>,
    server_trust: Vec<foks_proto::LocalTeamListEntry>,
}

impl AuthenticatedTeamMembershipChain {
    pub fn active(&self) -> impl Iterator<Item = &AuthenticatedTeamMembershipEvent> {
        self.current.iter().filter(|event| {
            matches!(
                event.membership.state,
                TeamMembershipState::Approved { .. } | TeamMembershipState::ApprovedAdHoc { .. }
            )
        })
    }
}

pub struct AuthenticatedLocalTeamGraph {
    pub teams: Vec<AuthenticatedTeamOutcome>,
    /// Team IDs ordered so a member team is refreshed before every parent
    /// team that depends on its PTKs.
    pub child_first: Vec<EntityId>,
    /// Directed `(member_team, parent_team)` edges.
    pub edges: Vec<(EntityId, EntityId)>,
}

impl AuthenticatedLocalTeamGraph {
    pub fn team(&self, team: &EntityId) -> Option<&AuthenticatedTeamOutcome> {
        self.teams
            .iter()
            .find(|candidate| candidate.verified.team() == team)
    }
}

enum LocalTeamGraphClaim {
    Membership(AuthenticatedTeamMembershipEvent),
    ServerTrust(foks_proto::LocalTeamListEntry),
}

impl LocalTeamGraphClaim {
    fn team(&self) -> &EntityId {
        match self {
            Self::Membership(event) => &event.membership.team,
            Self::ServerTrust(entry) => &entry.team,
        }
    }

    fn host<'a>(&'a self, local_host: &'a EntityId) -> &'a EntityId {
        match self {
            Self::Membership(event) => &event.membership.team_host,
            Self::ServerTrust(_) => local_host,
        }
    }
}

fn local_team_graph_path_is_unavailable(error: &Error) -> bool {
    matches!(
        error,
        Error::KeyBinding(_)
            | Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_PERMISSION_ERROR | foks_rpc::STATUS_TEAM_NOT_FOUND_ERROR,
                ..
            })
    )
}

type AuthenticatedUserSettings = (
    MembershipChainTail,
    Option<PassphraseInfo>,
    Vec<(u64, [u8; 32], PassphraseInfo)>,
);

impl FoksClient {
    pub fn load_generic_chain(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        chain_type: u64,
        start: u64,
    ) -> Result<GenericChain> {
        let response = self.call(
            host,
            &host.user,
            &encode_load_generic_chain_request(&credential.uid, chain_type, start)?,
            Some(credential),
        )?;
        GenericChain::decode(&response).map_err(Into::into)
    }

    /// Loads and verifies the transparent UserSettings chain at the same
    /// authenticated Merkle head as `user`.
    pub(crate) fn authenticated_user_settings(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
    ) -> Result<AuthenticatedUserSettings> {
        let response = self.call(
            host,
            &host.user,
            &encode_load_generic_chain_request(&credential.uid, CHAIN_TYPE_USER_SETTINGS, 1)?,
            Some(credential),
        )?;
        let verified = verify_user_generic_chain(
            &response,
            &credential.uid,
            host.host_id(),
            user,
            CHAIN_TYPE_USER_SETTINGS,
        )?;
        self.verify_generic_chain_roots(host, &user.tree_root(), &verified)?;
        self.pin_user_generic_chain(host, user, CHAIN_TYPE_USER_SETTINGS, &verified)?;
        let history = user_settings_history(&verified)?;
        Ok((
            verified.tail,
            history.last().map(|(_, _, info)| info.clone()),
            history,
        ))
    }

    pub(crate) fn authenticated_user_settings_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        user: &VerifiedUserState,
    ) -> Result<AuthenticatedUserSettings> {
        let response = self.call_with_material(
            host,
            &host.user,
            &encode_load_generic_chain_request(&credential.uid, CHAIN_TYPE_USER_SETTINGS, 1)?,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        let verified = verify_user_generic_chain(
            &response,
            &credential.uid,
            host.host_id(),
            user,
            CHAIN_TYPE_USER_SETTINGS,
        )?;
        self.verify_generic_chain_roots(host, &user.tree_root(), &verified)?;
        self.pin_user_generic_chain(host, user, CHAIN_TYPE_USER_SETTINGS, &verified)?;
        let history = user_settings_history(&verified)?;
        Ok((
            verified.tail,
            history.last().map(|(_, _, info)| info.clone()),
            history,
        ))
    }

    pub(crate) fn make_user_settings_link(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        tail: MembershipChainTail,
        passphrase: &PassphraseInfo,
    ) -> Result<foks_proto::PostGenericLinkArgument> {
        let signer = credential.public_material()?;
        let next_tree_location = random_bytes()?;
        let next_wire =
            foks_snowpack::encode(&foks_snowpack::Value::Binary(next_tree_location.to_vec()))?;
        let unsigned = UnsignedUserLink::user_settings(&UserSettingsLinkPublic {
            user: user.uid(),
            host: host.host_id(),
            signer: &signer.id,
            sequence: tail.sequence,
            previous: tail.previous,
            root: &user.tree_root(),
            time: now_milliseconds()?,
            next_location_commitment: foks_crypto::prefixed_hash_signable(
                TREE_LOCATION_TYPE_ID,
                &next_wire,
            )?,
            passphrase,
        })?;
        let signature = foks_crypto::sign_shared_key_typed(
            &credential.seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&[])?,
        )?;
        Ok(foks_proto::PostGenericLinkArgument {
            link: unsigned.finish(vec![signature])?,
            next_tree_location,
        })
    }

    pub(crate) fn make_user_settings_link_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        user: &VerifiedUserState,
        tail: MembershipChainTail,
        passphrase: &PassphraseInfo,
    ) -> Result<foks_proto::PostGenericLinkArgument> {
        let next_tree_location = random_bytes()?;
        let next_wire =
            foks_snowpack::encode(&foks_snowpack::Value::Binary(next_tree_location.to_vec()))?;
        let unsigned = UnsignedUserLink::user_settings(&UserSettingsLinkPublic {
            user: user.uid(),
            host: host.host_id(),
            signer: credential.parent.entity_id(),
            sequence: tail.sequence,
            previous: tail.previous,
            root: &user.tree_root(),
            time: now_milliseconds()?,
            next_location_commitment: foks_crypto::prefixed_hash_signable(
                TREE_LOCATION_TYPE_ID,
                &next_wire,
            )?,
            passphrase,
        })?;
        let signature = foks_crypto::sign_yubi_typed(
            credential.parent,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&[])?,
        )?;
        Ok(foks_proto::PostGenericLinkArgument {
            link: unsigned.finish(vec![signature])?,
            next_tree_location,
        })
    }

    pub fn post_generic_link(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        argument: &foks_proto::PostGenericLinkArgument,
    ) -> Result<()> {
        self.call_void_with_material(
            host,
            &host.user,
            &encode_post_generic_link_request(argument)?,
            &credential.seed,
            &credential.certificate_chain,
        )
    }

    pub fn load_team_membership_chain(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &AuthenticatedTeamOutcome,
        start: u64,
    ) -> Result<GenericChain> {
        let response = self.call(
            host,
            &host.user,
            &encode_load_team_membership_chain_request(
                team.verified.team(),
                host.host_id(),
                &team.view_token,
                start,
            )?,
            Some(credential),
        )?;
        GenericChain::decode(&response).map_err(Into::into)
    }

    /// Authenticates the logged-in user's complete team-membership side
    /// chain and checks that every server-trust membership is represented by
    /// a current approved chain entry.
    pub fn authenticated_user_team_memberships(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
    ) -> Result<AuthenticatedTeamMembershipChain> {
        self.authenticated_user_team_memberships_with_material(
            host,
            &credential.uid,
            &credential.seed,
            &credential.certificate_chain,
            user,
        )
    }

    pub fn authenticated_user_team_memberships_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        user: &VerifiedUserState,
    ) -> Result<AuthenticatedTeamMembershipChain> {
        // The chain is caller-supplied here, so the UID and host match
        // `_with_material` performs is not enough on its own: a chain that
        // merely shares the UID would otherwise stand in for the hardware
        // identity. Require the enrolled parent HEPK and delegated subkey too.
        FederationCredential::Yubi(credential).require_enrolled(host.host_id(), user)?;
        self.authenticated_user_team_memberships_with_material(
            host,
            &credential.uid,
            &credential.subkey_seed,
            &credential.certificate_chain,
            user,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn authenticated_user_team_memberships_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        user: &VerifiedUserState,
    ) -> Result<AuthenticatedTeamMembershipChain> {
        if user.uid() != uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "membership-chain user does not match the transport principal",
            ));
        }
        let response = self.call_with_material(
            host,
            &host.user,
            &encode_load_generic_chain_request(uid, CHAIN_TYPE_TEAM_MEMBERSHIP, 1)?,
            auth_seed,
            certificate_chain,
        )?;
        let verified = verify_user_generic_chain(
            &response,
            uid,
            host.host_id(),
            user,
            CHAIN_TYPE_TEAM_MEMBERSHIP,
        )?;
        self.verify_generic_chain_roots(host, &user.tree_root(), &verified)?;
        self.pin_user_generic_chain(host, user, CHAIN_TYPE_TEAM_MEMBERSHIP, &verified)?;
        let mut projection = membership_projection(uid, host.host_id(), &verified)?;
        let server_trust = foks_proto::decode_local_team_list(&self.call_with_material(
            host,
            &host.user,
            &encode_get_team_list_server_trust_request()?,
            auth_seed,
            certificate_chain,
        )?)?;
        reconcile_user_membership_server_trust(host.host_id(), &projection, &server_trust)?;
        projection.server_trust = server_trust;
        Ok(projection)
    }

    /// Authenticates a local team's complete membership side chain using the
    /// already activated team-view capability.
    pub fn authenticated_team_memberships(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &AuthenticatedTeamOutcome,
    ) -> Result<AuthenticatedTeamMembershipChain> {
        self.authenticated_team_memberships_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
            team,
        )
    }

    pub fn authenticated_team_memberships_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &AuthenticatedTeamOutcome,
    ) -> Result<AuthenticatedTeamMembershipChain> {
        self.authenticated_team_memberships_with_material(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
            team,
        )
    }

    fn authenticated_team_memberships_with_material(
        &self,
        host: &PinnedHost,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        team: &AuthenticatedTeamOutcome,
    ) -> Result<AuthenticatedTeamMembershipChain> {
        if team.verified.host() != host.host_id() {
            return Err(Error::TeamBinding(
                "membership-chain team is not local to the pinned host",
            ));
        }
        let response = self.call_with_material(
            host,
            &host.user,
            &encode_load_team_membership_chain_request(
                team.verified.team(),
                host.host_id(),
                &team.view_token,
                1,
            )?,
            auth_seed,
            certificate_chain,
        )?;
        let verified =
            verify_team_generic_chain(&response, &team.verified, CHAIN_TYPE_TEAM_MEMBERSHIP)?;
        self.verify_generic_chain_roots(host, &team.verified.tree_root(), &verified)?;
        membership_projection(team.verified.team(), host.host_id(), &verified)
    }

    /// Recursively discovers the authenticated local team-membership graph
    /// reachable from the logged-in user. Every membership-chain assertion is
    /// reconciled against the target team's current roster before it becomes
    /// an edge in the returned child-before-parent order.
    pub fn discover_local_team_graph(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
    ) -> Result<AuthenticatedLocalTeamGraph> {
        self.discover_local_team_graph_with_credential(
            host,
            FederationCredential::Software(credential),
            user,
            puks,
        )
    }

    pub fn discover_local_team_graph_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
    ) -> Result<AuthenticatedLocalTeamGraph> {
        self.discover_local_team_graph_with_credential(
            host,
            FederationCredential::Yubi(credential),
            user,
            puks,
        )
    }

    /// Credential-agnostic membership-graph discovery. Federation drives this
    /// directly so a hardware-only administrator walks the same DAG.
    pub fn discover_local_team_graph_with_credential(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
    ) -> Result<AuthenticatedLocalTeamGraph> {
        let user_memberships = match credential {
            FederationCredential::Software(credential) => {
                self.authenticated_user_team_memberships(host, credential, user)?
            }
            FederationCredential::Yubi(credential) => {
                self.authenticated_user_team_memberships_yubi(host, credential, user)?
            }
        };
        let mut queue =
            std::collections::VecDeque::<(Option<EntityId>, LocalTeamGraphClaim)>::new();
        for membership in user_memberships.active() {
            queue.push_back((None, LocalTeamGraphClaim::Membership(membership.clone())));
        }
        for entry in &user_memberships.server_trust {
            let represented = user_memberships.active().any(|event| {
                event.membership.team == entry.team
                    && event.membership.team_host == *host.host_id()
                    && event.membership.source_role == entry.source_role
                    && match event.membership.state {
                        TeamMembershipState::Approved {
                            destination_role,
                            team_sequence,
                            ..
                        }
                        | TeamMembershipState::ApprovedAdHoc {
                            destination_role,
                            team_sequence,
                        } => {
                            destination_role == entry.destination_role
                                && team_sequence <= entry.team_sequence
                        }
                        _ => false,
                    }
            });
            if !represented {
                queue.push_back((None, LocalTeamGraphClaim::ServerTrust(entry.clone())));
            }
        }
        let mut teams = Vec::<AuthenticatedTeamOutcome>::new();
        let mut indexes = std::collections::BTreeMap::<Vec<u8>, usize>::new();
        let mut edges = std::collections::BTreeSet::<(Vec<u8>, Vec<u8>)>::new();
        let mut explored = std::collections::BTreeSet::<Vec<u8>>::new();

        while !queue.is_empty() {
            let round_len = queue.len();
            let mut round_progress = false;
            let mut deferred_error = None;
            for _ in 0..round_len {
                let (actor_id, claim) = queue
                    .pop_front()
                    .expect("membership discovery round length is exact");
                if claim.host(host.host_id()) != host.host_id() {
                    // This API deliberately returns only the graph whose teams
                    // are authoritative on `host`. Paired federation profiles
                    // authenticate and refresh remote components independently.
                    round_progress = true;
                    continue;
                }
                let target_id = claim.team().clone();
                let target_key = target_id.as_bytes().to_vec();
                let existing = indexes.get(&target_key).copied();
                // A target can be reachable through several roster paths with
                // different destination roles. Load each team-actor path and
                // retain the strongest decrypted PTK view instead of freezing
                // whichever (possibly low-role) claim happened to be queued
                // first.
                let candidate_result = if existing.is_none() || actor_id.is_some() {
                    Some(match actor_id.as_ref() {
                        None => match credential {
                            FederationCredential::Software(credential) => {
                                self.load_and_pin_team(host, credential, user, puks, &target_id)
                            }
                            FederationCredential::Yubi(credential) => self
                                .load_and_pin_team_yubi(host, credential, user, puks, &target_id),
                        },
                        Some(actor_id) => {
                            let actor_index = indexes.get(actor_id.as_bytes()).copied().ok_or(
                                Error::TeamBinding("membership graph lost its local team actor"),
                            )?;
                            let actor = &teams[actor_index];
                            match credential {
                                FederationCredential::Software(credential) => self
                                    .load_and_pin_team_as_local_team(
                                        host, credential, user, actor, &target_id,
                                    ),
                                FederationCredential::Yubi(credential) => self
                                    .load_and_pin_team_as_local_team_yubi(
                                        host, credential, user, actor, &target_id,
                                    ),
                            }
                        }
                    })
                } else {
                    None
                };
                let mut candidate = match candidate_result.transpose() {
                    Ok(candidate) => candidate,
                    Err(_) if actor_id.is_some() && existing.is_some() => {
                        // The target's already-authenticated public state is
                        // sufficient to validate this membership edge. A
                        // weaker actor path need not be able to reopen the
                        // target merely to contribute graph ordering.
                        None
                    }
                    Err(error) if actor_id.is_some() => {
                        // Another queued path can reveal a stronger PTK view
                        // of this actor. Defer the dependent claim until every
                        // currently available claim has had a chance to
                        // strengthen it; if no claim makes progress, return
                        // the original authenticated-load failure.
                        deferred_error.get_or_insert(error);
                        queue.push_back((actor_id, claim));
                        continue;
                    }
                    Err(error)
                        if actor_id.is_none() && local_team_graph_path_is_unavailable(&error) =>
                    {
                        // A direct membership sidechain and server-trust row
                        // are discovery hints, not authority. If this
                        // transport cannot authenticate the target through
                        // that path, omit the claim and keep exploring: a
                        // queued local-team actor may still authenticate the
                        // same target, and a stale direct hint must not strand
                        // unrelated or caller-durable nested-team work.
                        round_progress = true;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let (target_index, newly_loaded) = if let Some(index) = existing {
                    (index, false)
                } else {
                    let index = teams.len();
                    indexes.insert(target_key.clone(), index);
                    teams.push(candidate.take().expect("new target has a loaded view"));
                    (index, true)
                };
                let owner = actor_id.as_ref().unwrap_or(user.uid());
                let claim_target = candidate.as_ref().unwrap_or(&teams[target_index]);
                let active = match &claim {
                    LocalTeamGraphClaim::Membership(membership) => {
                        validate_membership_against_team(owner, membership, &claim_target.verified)?
                    }
                    LocalTeamGraphClaim::ServerTrust(entry) => {
                        validate_server_trust_against_team(
                            user.uid(),
                            entry,
                            &claim_target.verified,
                        )?;
                        true
                    }
                };
                if !active {
                    if newly_loaded {
                        indexes.remove(&target_key);
                        let removed = teams
                            .pop()
                            .ok_or(Error::TeamBinding("membership graph lost a stale target"))?;
                        if removed.verified.team() != &target_id {
                            return Err(Error::TeamBinding(
                                "membership graph stale-target rollback changed order",
                            ));
                        }
                    }
                    round_progress = true;
                    continue;
                }
                if let Some(candidate) = candidate {
                    let private_scope = |team: &AuthenticatedTeamOutcome| {
                        (team.ptks.iter().map(|key| key.role).max(), team.ptks.len())
                    };
                    if private_scope(&candidate) > private_scope(&teams[target_index]) {
                        teams[target_index] = candidate;
                    }
                }
                if let Some(actor_id) = actor_id {
                    edges.insert((actor_id.as_bytes().to_vec(), target_key.clone()));
                }
                if explored.insert(target_key) {
                    let nested = match credential {
                        FederationCredential::Software(credential) => self
                            .authenticated_team_memberships(
                                host,
                                credential,
                                &teams[target_index],
                            )?,
                        FederationCredential::Yubi(credential) => self
                            .authenticated_team_memberships_yubi(
                                host,
                                credential,
                                &teams[target_index],
                            )?,
                    };
                    for child_membership in nested.active() {
                        queue.push_back((
                            Some(target_id.clone()),
                            LocalTeamGraphClaim::Membership(child_membership.clone()),
                        ));
                    }
                }
                round_progress = true;
            }
            if !round_progress {
                return Err(deferred_error.unwrap_or(Error::TeamBinding(
                    "membership graph made no progress resolving local team actors",
                )));
            }
        }

        let child_first = child_first_team_order(&indexes, &edges)?;
        let edges = edges
            .into_iter()
            .map(|(child, parent)| {
                Ok((EntityId::from_bytes(child)?, EntityId::from_bytes(parent)?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(AuthenticatedLocalTeamGraph {
            teams,
            child_first,
            edges,
        })
    }

    /// Authenticates a local child team's current public PTKs using a parent
    /// team's active view token. This grants no child private keys or acting
    /// authority; it is the Go-compatible roster load used to notice that a
    /// child rotated and the parent must update its recorded member key.
    pub fn load_local_child_team_recipient(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        parent: &AuthenticatedTeamOutcome,
        child: &EntityId,
    ) -> Result<VerifiedTeamRecipient> {
        self.load_local_child_team_recipient_with_material(
            host,
            &credential.uid,
            &credential.seed,
            &credential.certificate_chain,
            parent,
            child,
        )
    }

    /// Hardware-transport variant of
    /// [`Self::load_local_child_team_recipient`].
    pub fn load_local_child_team_recipient_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        parent: &AuthenticatedTeamOutcome,
        child: &EntityId,
    ) -> Result<VerifiedTeamRecipient> {
        self.load_local_child_team_recipient_with_material(
            host,
            &credential.uid,
            &credential.subkey_seed,
            &credential.certificate_chain,
            parent,
            child,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn load_local_child_team_recipient_with_material(
        &self,
        host: &PinnedHost,
        _uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        parent: &AuthenticatedTeamOutcome,
        child: &EntityId,
    ) -> Result<VerifiedTeamRecipient> {
        if parent.verified.host() != host.host_id()
            || parent.verified.team() == child
            || !matches!(
                child.entity_type(),
                foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
            )
        {
            return Err(Error::TeamBinding(
                "local parent and child team binding is invalid",
            ));
        }
        let matches = parent
            .verified
            .members()
            .iter()
            .filter(|member| member.party == *child && member.scoped_host.is_none())
            .count();
        if matches != 1 {
            return Err(Error::TeamBinding(
                "local child team is not unique in the parent roster",
            ));
        }

        let verified = self.retry_chain_load(host, |current| {
            let (_, merkle) = self.advance_merkle_root(current)?;
            let prior = match self.pinned_team(current, child) {
                Ok(prior) => prior,
                Err(Error::Verify(
                    foks_verify::Error::PersistedMerkleEvidence
                    | foks_verify::Error::TeamChainContinuity,
                )) => None,
                Err(error) => return Err(error),
            };
            let (start, current_name) = match prior.as_ref() {
                Some(prior) => (
                    prior
                        .chain_seqno()
                        .checked_add(1)
                        .ok_or(Error::TeamBinding("team chain sequence overflow"))?,
                    Some((
                        prior.team_name(),
                        prior
                            .team_name_sequence()
                            .checked_add(1)
                            .ok_or(Error::TeamBinding("team name sequence overflow"))?,
                    )),
                ),
                None => (1, None),
            };
            let chain_bytes = self.call_with_material(
                current,
                &current.user,
                &encode_load_team_chain_for_local_parent_request(
                    child,
                    current.host_id(),
                    &parent.view_token,
                    start,
                    TeamChainLoadOptions {
                        current_name,
                        ..TeamChainLoadOptions::default()
                    },
                )?,
                auth_seed,
                certificate_chain,
            )?;
            let chain = TeamChain::decode(&chain_bytes)?;
            if !chain.boxes.is_empty()
                || chain.removal_key.is_some()
                || !chain.remote_view_tokens.is_empty()
            {
                return Err(Error::TeamBinding(
                    "local-parent public load returned private team material",
                ));
            }
            let authenticated_roots =
                self.authenticate_team_chain_roots(current, &merkle, &chain_bytes)?;
            let verified = match prior.as_ref() {
                Some(prior) => verify_team_chain_increment(
                    &chain_bytes,
                    prior,
                    child,
                    current.host_id(),
                    &authenticated_roots,
                    &merkle,
                )?,
                None => verify_team_chain(
                    &chain_bytes,
                    child,
                    current.host_id(),
                    &authenticated_roots,
                    &merkle,
                )?,
            };
            Ok(verified)
        })?;
        HardStateStore::open(&host.database_path)?
            .accept_verified_team(&verified.hard_state_snapshot()?)?;
        VerifiedTeamRecipient::from_latest_public_team(
            verified.clone(),
            verified.tree_root(),
            host.clone(),
        )
    }

    pub fn post_team_membership_link(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &AuthenticatedTeamOutcome,
        argument: &foks_proto::PostGenericLinkArgument,
    ) -> Result<()> {
        let token = self.activate_team_admin_bearer(
            host,
            &credential.uid,
            &credential.seed,
            &credential.certificate_chain,
            team,
        )?;
        self.call_void_with_material(
            host,
            &host.user,
            &encode_post_team_membership_link_request(&token, argument)?,
            &credential.seed,
            &credential.certificate_chain,
        )
    }

    pub fn local_team_list(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<Vec<foks_proto::LocalTeamListEntry>> {
        let response = self.call(
            host,
            &host.user,
            &encode_get_team_list_server_trust_request()?,
            Some(credential),
        )?;
        foks_proto::decode_local_team_list(&response).map_err(Into::into)
    }

    pub(super) fn load_membership_chain_tail(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        user: &VerifiedUserState,
    ) -> Result<MembershipChainTail> {
        let response = self.call_with_material(
            host,
            &host.user,
            &encode_load_generic_chain_request(uid, CHAIN_TYPE_TEAM_MEMBERSHIP, 1)?,
            auth_seed,
            certificate_chain,
        )?;
        let verified = verify_user_generic_chain(
            &response,
            uid,
            host.host_id(),
            user,
            CHAIN_TYPE_TEAM_MEMBERSHIP,
        )?;
        if verified
            .payloads
            .iter()
            .any(|payload| !matches!(payload, GenericLinkPayload::TeamMembership(_)))
        {
            return Err(Error::TeamBinding(
                "membership chain contains a settings payload",
            ));
        }
        self.verify_generic_chain_roots(host, &user.tree_root(), &verified)?;
        self.pin_user_generic_chain(host, user, CHAIN_TYPE_TEAM_MEMBERSHIP, &verified)?;
        Ok(verified.tail)
    }

    fn pin_user_generic_chain(
        &self,
        host: &PinnedHost,
        user: &VerifiedUserState,
        chain_type: u64,
        chain: &VerifiedUserGenericChain,
    ) -> Result<()> {
        let root = user.tree_root();
        HardStateStore::open(&host.database_path)?.accept_verified_user_generic_chain(
            &VerifiedUserGenericChainSnapshot {
                host_id: host.host_id().as_bytes(),
                uid: user.uid().as_bytes(),
                chain_type,
                sequence: chain.tail.sequence.saturating_sub(1),
                tail_hash: chain.tail.previous,
                chain_bytes: &chain.chain_bytes,
                merkle_epoch: root.epoch,
                merkle_root_hash: root.hash,
            },
        )?;
        Ok(())
    }

    fn verify_generic_chain_roots(
        &self,
        host: &PinnedHost,
        owner_root: &foks_proto::TreeRoot,
        chain: &VerifiedUserGenericChain,
    ) -> Result<()> {
        let (_, latest) = self.advance_merkle_root(host)?;
        let latest_bytes = latest.root().encoded()?;
        let latest_tree = foks_proto::TreeRoot {
            epoch: latest.root().epoch,
            hash: foks_crypto::prefixed_hash_signable(MERKLE_ROOT_TYPE_ID, &latest_bytes)?,
        };
        if latest_tree != *owner_root {
            return Err(Error::TeamBinding(
                "chain response is not anchored at the latest Merkle root",
            ));
        }
        let mut targets = std::collections::BTreeSet::new();
        for root in &chain.cited_roots {
            if root.epoch == latest_tree.epoch {
                if root.hash != latest_tree.hash {
                    return Err(Error::UserBinding(
                        "generic link cites the wrong current Merkle root",
                    ));
                }
            } else {
                targets.insert(root.epoch);
            }
        }
        for signer in &chain.historical_signers {
            targets.insert(signer.generic_root.epoch);
            if let Some(revoke_root) = &signer.bookends.revoke_root {
                targets.insert(revoke_root.epoch);
            }
        }
        targets.retain(|epoch| !latest.authenticated_roots().contains_epoch(*epoch));
        let authenticated = self.authenticate_chain_roots(
            host,
            &latest,
            targets.into_iter().collect(),
            Error::TeamBinding,
        )?;
        for root in &chain.cited_roots {
            if authenticated.root_hash(root.epoch) != Some(root.hash) {
                return Err(Error::UserBinding(
                    "generic link cites an unauthenticated Merkle root",
                ));
            }
        }
        for signer in &chain.historical_signers {
            if let Some(revoke_root) = &signer.bookends.revoke_root {
                self.verify_historical_leaf(
                    host,
                    &authenticated,
                    revoke_root,
                    signer.generic_key,
                    signer.generic_value,
                )?;
            }
            self.verify_historical_leaf(
                host,
                &authenticated,
                &signer.generic_root,
                signer.bookends.provision.key,
                signer.bookends.provision.value,
            )?;
        }
        Ok(())
    }

    fn verify_historical_leaf(
        &self,
        host: &PinnedHost,
        authenticated: &AuthenticatedMerkleRoots,
        root: &foks_proto::TreeRoot,
        key: [u8; 32],
        value: [u8; 32],
    ) -> Result<()> {
        if authenticated.root_hash(root.epoch) != Some(root.hash) {
            return Err(Error::TeamBinding(
                "generic signer bookend root is unauthenticated",
            ));
        }
        let lookup = self.merkle_lookup(host, key, false, Some(root.epoch))?;
        let encoded_root = lookup.root.encoded()?;
        if lookup.root.epoch != root.epoch
            || foks_crypto::prefixed_hash_signable(foks_proto::MERKLE_ROOT_TYPE_ID, &encoded_root)?
                != root.hash
        {
            return Err(Error::TeamBinding(
                "generic signer bookend lookup returned the wrong root",
            ));
        }
        verify_merkle_path(&lookup.path, &key, Some(&value), &lookup.root.root_node)?;
        Ok(())
    }
}

enum GenericChainOwner<'a> {
    User(&'a VerifiedUserState),
    Team(&'a VerifiedTeamState),
}

impl GenericChainOwner<'_> {
    fn entity(&self) -> &EntityId {
        match self {
            Self::User(user) => user.uid(),
            Self::Team(team) => team.team(),
        }
    }

    fn host(&self) -> &EntityId {
        match self {
            Self::User(user) => user.host(),
            Self::Team(team) => team.host(),
        }
    }

    fn tree_root(&self) -> foks_proto::TreeRoot {
        match self {
            Self::User(user) => user.tree_root(),
            Self::Team(team) => team.tree_root(),
        }
    }

    fn subchain_location_commitment(&self) -> Result<[u8; 32]> {
        let metadata = match self {
            Self::User(user) => {
                let snapshot = user.hard_state_snapshot()?;
                let foks_snowpack::Value::Array(user_links) =
                    foks_snowpack::decode(snapshot.parts().chain_bytes)?
                else {
                    return Err(Error::TeamBinding("authenticated user chain is malformed"));
                };
                let eldest = user_links
                    .first()
                    .ok_or(Error::TeamBinding("authenticated user chain has no eldest"))?;
                UserLink::decode(&foks_snowpack::encode(eldest)?)?
                    .decode_eldest()?
                    .metadata
            }
            Self::Team(team) => team.group_change_at(1)?.metadata,
        };
        let mut commitments = metadata.iter().filter_map(|metadata| match metadata {
            ChangeMetadata::Eldest {
                subchain_location_commitment,
            } => Some(*subchain_location_commitment),
            _ => None,
        });
        let commitment = commitments.next().ok_or(Error::TeamBinding(
            "generic chain owner has no subchain location commitment",
        ))?;
        if commitments.next().is_some() {
            return Err(Error::TeamBinding(
                "generic chain owner has ambiguous subchain commitments",
            ));
        }
        Ok(commitment)
    }

    fn signing_bookends(&self, signer: &EntityId, epoch: u64) -> Result<UserDeviceSigningBookends> {
        match self {
            Self::User(user) => user.device_signing_bookends(signer, epoch),
            Self::Team(team) => team.shared_key_signing_bookends(signer, epoch),
        }?
        .ok_or(Error::TeamBinding(
            "generic signer was not active at its cited Merkle root",
        ))
    }
}

fn verify_user_generic_chain(
    response: &[u8],
    uid: &EntityId,
    host: &EntityId,
    user: &VerifiedUserState,
    chain_type: u64,
) -> Result<VerifiedUserGenericChain> {
    if user.uid() != uid || user.host() != host {
        return Err(Error::UserBinding(
            "generic-chain user does not match its authenticated owner",
        ));
    }
    verify_generic_chain(response, GenericChainOwner::User(user), chain_type)
}

fn verify_team_generic_chain(
    response: &[u8],
    team: &VerifiedTeamState,
    chain_type: u64,
) -> Result<VerifiedUserGenericChain> {
    verify_generic_chain(response, GenericChainOwner::Team(team), chain_type)
}

fn verify_generic_chain(
    response: &[u8],
    owner: GenericChainOwner<'_>,
    chain_type: u64,
) -> Result<VerifiedUserGenericChain> {
    let chain = GenericChain::decode(response)?;
    if chain.locations.len() != chain.links.len()
        || chain.merkle.paths().len() != chain.links.len().saturating_add(1)
    {
        return Err(Error::TeamBinding(
            "generic chain proof and location counts are invalid",
        ));
    }
    let mut persisted_links = Vec::with_capacity(chain.links.len().saturating_add(1));
    persisted_links.push(foks_snowpack::Value::Unsigned(chain_type));
    persisted_links.extend(
        chain
            .links
            .iter()
            .map(|link| Ok(foks_snowpack::decode(&link.encoded()?)?))
            .collect::<Result<Vec<_>>>()?,
    );
    let chain_bytes = foks_snowpack::encode(&foks_snowpack::Value::Array(persisted_links))?;
    let seed = chain.location_seed.ok_or(Error::TeamBinding(
        "membership chain omitted its location seed",
    ))?;
    let root_bytes = chain.merkle.encoded_root()?;
    let response_root = foks_proto::TreeRoot {
        epoch: chain.merkle.root().epoch,
        hash: foks_crypto::prefixed_hash_signable(MERKLE_ROOT_TYPE_ID, &root_bytes)?,
    };
    if response_root != owner.tree_root() {
        return Err(Error::TeamBinding(
            "membership chain is not bound to its authenticated owner root",
        ));
    }
    let seed_wire = foks_snowpack::encode(&foks_snowpack::Value::Binary(seed.to_vec()))?;
    let seed_commitment = foks_crypto::prefixed_hash_signable(TREE_LOCATION_TYPE_ID, &seed_wire)?;
    if owner.subchain_location_commitment()? != seed_commitment {
        return Err(Error::TeamBinding(
            "membership chain seed is not committed by its owner eldest",
        ));
    }
    let owner_entity = owner.entity();
    let owner_host = owner.host();
    let mut location = foks_crypto::subchain_tree_location(&seed, chain_type)?;
    let mut previous = None;
    let mut historical_signers = Vec::new();
    let mut payloads = Vec::with_capacity(chain.links.len());
    let mut link_hashes = Vec::with_capacity(chain.links.len());
    let mut cited_roots = Vec::with_capacity(chain.links.len());
    let mut previous_settings = None::<PassphraseInfo>;
    for (index, ((link, next_location), path)) in chain
        .links
        .iter()
        .zip(&chain.locations)
        .zip(&chain.merkle.paths()[..chain.links.len()])
        .enumerate()
    {
        let sequence = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(Error::TeamBinding("membership chain sequence overflow"))?;
        let decoded = link.decode_generic()?;
        if decoded.entity != *owner_entity
            || decoded.host != *owner_host
            || decoded.sequence != sequence
            || decoded.previous != previous
            || !matches!(
                (chain_type, &decoded.payload),
                (
                    CHAIN_TYPE_TEAM_MEMBERSHIP,
                    GenericLinkPayload::TeamMembership(_)
                ) | (
                    CHAIN_TYPE_USER_SETTINGS,
                    GenericLinkPayload::UserSettings(_)
                )
            )
            || link.signatures().len() != 1
            || foks_crypto::verify_typed(
                &decoded.signer,
                &link.signatures()[0],
                LINK_OUTER_V1_TYPE_ID,
                &link.signing_bytes(0)?,
            )
            .is_err()
        {
            return Err(Error::TeamBinding(
                "generic chain continuity, payload, or signer is invalid",
            ));
        }
        let next_wire =
            foks_snowpack::encode(&foks_snowpack::Value::Binary(next_location.to_vec()))?;
        if foks_crypto::prefixed_hash_signable(TREE_LOCATION_TYPE_ID, &next_wire)?
            != decoded.next_location_commitment
        {
            return Err(Error::TeamBinding(
                "generic chain location disclosure is invalid",
            ));
        }
        let exact = link.encoded()?;
        let link_hash = foks_crypto::prefixed_hash_signable(LINK_OUTER_TYPE_ID, &exact)?;
        let key = foks_merkle_store::chain_key(chain_type, owner_entity, sequence, Some(&location))
            .map_err(|_| Error::TeamBinding("generic Merkle key is invalid"))?;
        verify_merkle_path(path, &key, Some(&link_hash), &chain.merkle.root().root_node)?;
        let bookends = owner.signing_bookends(&decoded.signer, decoded.root.epoch)?;
        historical_signers.push(HistoricalMembershipSigner {
            generic_root: decoded.root.clone(),
            generic_key: key,
            generic_value: link_hash,
            bookends,
        });
        cited_roots.push(decoded.root);
        let payload = match decoded.payload {
            GenericLinkPayload::UserSettings(mut info) => {
                if previous_settings
                    .as_ref()
                    .is_some_and(|previous| info.generation < previous.generation)
                {
                    return Err(Error::UserBinding(
                        "user-settings passphrase generation moved backwards",
                    ));
                }
                if info.salt.is_none() {
                    info.salt = previous_settings
                        .as_ref()
                        .and_then(|previous| previous.salt);
                }
                previous_settings = Some(info.clone());
                GenericLinkPayload::UserSettings(info)
            }
            GenericLinkPayload::TeamMembership(membership) => {
                GenericLinkPayload::TeamMembership(membership)
            }
        };
        payloads.push(payload);
        link_hashes.push(link_hash);
        previous = Some(link_hash);
        location = *next_location;
    }
    let sequence = u64::try_from(chain.links.len())
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(Error::TeamBinding("membership chain sequence overflow"))?;
    let key = foks_merkle_store::chain_key(chain_type, owner_entity, sequence, Some(&location))
        .map_err(|_| Error::TeamBinding("generic bookend key is invalid"))?;
    verify_merkle_path(
        chain
            .merkle
            .paths()
            .last()
            .ok_or(Error::TeamBinding("membership chain omitted its bookend"))?,
        &key,
        None,
        &chain.merkle.root().root_node,
    )?;
    Ok(VerifiedUserGenericChain {
        tail: MembershipChainTail { sequence, previous },
        payloads,
        link_hashes,
        chain_bytes,
        cited_roots,
        historical_signers,
    })
}

fn membership_projection(
    owner: &EntityId,
    host: &EntityId,
    chain: &VerifiedUserGenericChain,
) -> Result<AuthenticatedTeamMembershipChain> {
    let mut events = Vec::with_capacity(chain.payloads.len());
    let mut current = std::collections::BTreeMap::new();
    for (index, payload) in chain.payloads.iter().enumerate() {
        let GenericLinkPayload::TeamMembership(membership) = payload else {
            return Err(Error::TeamBinding(
                "team-membership chain contains another payload type",
            ));
        };
        let sequence = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(Error::TeamBinding("membership chain sequence overflow"))?;
        let event = AuthenticatedTeamMembershipEvent {
            sequence,
            membership: membership.clone(),
        };
        current.insert(
            (
                membership.team.as_bytes().to_vec(),
                membership.team_host.as_bytes().to_vec(),
                membership.source_role.protocol_value(),
                membership.source_role.visibility().unwrap_or_default(),
            ),
            event.clone(),
        );
        events.push(event);
    }
    Ok(AuthenticatedTeamMembershipChain {
        owner: owner.clone(),
        host: host.clone(),
        next_sequence: chain.tail.sequence,
        events,
        current: current.into_values().collect(),
        server_trust: Vec::new(),
    })
}

fn reconcile_user_membership_server_trust(
    host: &EntityId,
    chain: &AuthenticatedTeamMembershipChain,
    server: &[foks_proto::LocalTeamListEntry],
) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for entry in server {
        let key = (
            entry.team.as_bytes().to_vec(),
            entry.source_role.protocol_value(),
            entry.source_role.visibility().unwrap_or_default(),
        );
        if !seen.insert(key) {
            return Err(Error::TeamBinding(
                "server-trust team list contains a duplicate membership",
            ));
        }
        let matches = chain
            .active()
            .filter(|event| {
                event.membership.team == entry.team
                    && event.membership.team_host == *host
                    && event.membership.source_role == entry.source_role
                    && match &event.membership.state {
                        TeamMembershipState::Approved {
                            destination_role,
                            team_sequence,
                            ..
                        }
                        | TeamMembershipState::ApprovedAdHoc {
                            destination_role,
                            team_sequence,
                        } => {
                            *destination_role == entry.destination_role
                                // The signed membership event cites the team
                                // transition that admitted this party. The
                                // server-trust projection reports the team's
                                // current head, which advances after unrelated
                                // roster edits and CLKR rotations.
                                && *team_sequence <= entry.team_sequence
                        }
                        _ => false,
                    }
            })
            .count();
        if matches > 1 {
            return Err(Error::TeamBinding(
                "server-trust membership is ambiguous in the authenticated chain",
            ));
        }
    }
    Ok(())
}

fn validate_membership_against_team(
    owner: &EntityId,
    event: &AuthenticatedTeamMembershipEvent,
    team: &VerifiedTeamState,
) -> Result<bool> {
    if event.membership.team != *team.team() || event.membership.team_host != *team.host() {
        return Err(Error::TeamBinding(
            "membership chain points at a different authenticated team",
        ));
    }
    let (destination_role, team_sequence, removal_key_commitment) = match &event.membership.state {
        TeamMembershipState::Approved {
            destination_role,
            team_sequence,
            removal_key_commitment,
        } => (
            *destination_role,
            *team_sequence,
            Some(*removal_key_commitment),
        ),
        TeamMembershipState::ApprovedAdHoc {
            destination_role,
            team_sequence,
        } => (*destination_role, *team_sequence, None),
        _ => {
            return Err(Error::TeamBinding(
                "inactive membership was used to traverse the team graph",
            ));
        }
    };
    let change = team.group_change_at(team_sequence)?;
    let matching_changes = change
        .changes
        .iter()
        .filter(|change| {
            change.party == *owner
                && change.scoped_host.is_none()
                && change.source_role == event.membership.source_role
                && change.role == destination_role
                && change
                    .keys
                    .as_ref()
                    .is_some_and(|keys| keys.removal_key_commitment == removal_key_commitment)
        })
        .count();
    if matching_changes != 1 {
        return Err(Error::TeamBinding(
            "membership approval does not match its cited team transition",
        ));
    }
    let mut roster_matches = team.members().iter().filter(|member| {
        member.party == *owner
            && member.scoped_host.is_none()
            && member.source_role == event.membership.source_role
    });
    let member = roster_matches.next();
    if roster_matches.next().is_some() {
        return Err(Error::TeamBinding(
            "membership chain is ambiguous in the target roster",
        ));
    }
    Ok(member.is_some_and(|member| {
        member.role == destination_role && member.removal_key_commitment == removal_key_commitment
    }))
}

fn validate_server_trust_against_team(
    user: &EntityId,
    entry: &foks_proto::LocalTeamListEntry,
    team: &VerifiedTeamState,
) -> Result<()> {
    if entry.team != *team.team() || entry.team_sequence > team.chain_seqno() {
        return Err(Error::TeamBinding(
            "server-trust membership does not match the authenticated team head",
        ));
    }
    let current = team
        .members()
        .iter()
        .filter(|member| {
            member.party == *user
                && member.scoped_host.is_none()
                && member.source_role == entry.source_role
                && member.role == entry.destination_role
                && member.generation == entry.key_generation
        })
        .count();
    // Go reports the row's admission/last-change sequence; the standalone
    // Rust server historically reported the current head. Authenticate the
    // cited historical change when one is available, while retaining the
    // exact current-roster check for the head-form projection.
    let introduced = if entry.team_sequence == team.chain_seqno() {
        1
    } else {
        team.group_change_at(entry.team_sequence)?
            .changes
            .iter()
            .filter(|change| {
                change.party == *user
                    && change.scoped_host.is_none()
                    && change.source_role == entry.source_role
                    && change.role == entry.destination_role
                    && change
                        .keys
                        .as_ref()
                        .is_some_and(|keys| keys.generation == entry.key_generation)
            })
            .count()
    };
    if current != 1 || introduced != 1 {
        return Err(Error::TeamBinding(
            "server-trust membership does not match its authenticated team transition",
        ));
    }
    Ok(())
}

fn child_first_team_order(
    indexes: &std::collections::BTreeMap<Vec<u8>, usize>,
    edges: &std::collections::BTreeSet<(Vec<u8>, Vec<u8>)>,
) -> Result<Vec<EntityId>> {
    let mut indegree = indexes
        .keys()
        .cloned()
        .map(|team| (team, 0_usize))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut parents = std::collections::BTreeMap::<Vec<u8>, Vec<Vec<u8>>>::new();
    for (child, parent) in edges {
        if !indexes.contains_key(child) || !indexes.contains_key(parent) {
            return Err(Error::TeamBinding(
                "membership graph edge references an unloaded team",
            ));
        }
        *indegree
            .get_mut(parent)
            .ok_or(Error::TeamBinding("membership graph parent is missing"))? += 1;
        parents
            .entry(child.clone())
            .or_default()
            .push(parent.clone());
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(team, count)| (*count == 0).then_some(team.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(indexes.len());
    while let Some(team) = ready.pop_first() {
        ordered.push(EntityId::from_bytes(team.clone())?);
        for parent in parents.get(&team).into_iter().flatten() {
            let count = indegree
                .get_mut(parent)
                .ok_or(Error::TeamBinding("membership graph parent is missing"))?;
            *count = count
                .checked_sub(1)
                .ok_or(Error::TeamBinding("membership graph indegree underflow"))?;
            if *count == 0 {
                ready.insert(parent.clone());
            }
        }
    }
    if ordered.len() != indexes.len() {
        return Err(Error::TeamBinding("team membership graph contains a cycle"));
    }
    Ok(ordered)
}

fn user_settings_history(
    chain: &VerifiedUserGenericChain,
) -> Result<Vec<(u64, [u8; 32], PassphraseInfo)>> {
    chain
        .payloads
        .iter()
        .zip(&chain.link_hashes)
        .enumerate()
        .map(|(index, (payload, hash))| {
            let GenericLinkPayload::UserSettings(info) = payload else {
                return Err(Error::UserBinding(
                    "settings chain contains a membership payload",
                ));
            };
            Ok((
                u64::try_from(index)
                    .ok()
                    .and_then(|index| index.checked_add(1))
                    .ok_or(Error::UserBinding("settings chain sequence overflow"))?,
                *hash,
                info.clone(),
            ))
        })
        .collect()
}

impl FoksClient {
    /// Loads the authenticated host capability policy advertised by
    /// `User.getHostConfig`.
    pub fn host_config(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<HostConfig> {
        self.host_config_with_material(host, &credential.seed, &credential.certificate_chain)
    }

    fn host_config_with_material(
        &self,
        host: &PinnedHost,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
    ) -> Result<HostConfig> {
        HostConfig::decode(&self.call_with_material(
            host,
            &host.user,
            &encode_get_host_config_request()?,
            auth_seed,
            certificate_chain,
        )?)
        .map_err(Into::into)
    }

    // Prior state is only an optimization for recipient ordering/incremental
    // loads. Use the same full, authenticated reload fallback in both places;
    // persistence still enforces the existing monotonic team pin.
    fn prior_team_for_reload(
        &self,
        host: &PinnedHost,
        team: &EntityId,
    ) -> Result<Option<VerifiedTeamState>> {
        match self.pinned_team(host, team) {
            Ok(prior) => Ok(prior),
            Err(Error::Verify(
                foks_verify::Error::PersistedMerkleEvidence
                | foks_verify::Error::TeamChainContinuity,
            )) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Acquires a short-lived view token with the current device-role PUK,
    /// loads and verifies a team chain, unboxes exactly the PTKs visible to
    /// this member, and atomically seals the public team projection in SQLite.
    pub fn load_and_pin_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        let mut last_error = None;
        let prior = self.prior_team_for_reload(host, team)?;
        let mut candidates = puks.iter().rev().collect::<Vec<_>>();
        candidates.sort_by_key(|puk| {
            !prior
                .as_ref()
                .is_some_and(|prior| user_puk_matches_team_roster(user, puk, prior, host.host_id()))
        });
        for puk in candidates {
            if user_key_history_for_seed(user, &puk.seed).is_err() {
                continue;
            }
            match self.load_and_pin_team_with_material(
                host,
                &credential.uid,
                &credential.seed,
                &credential.certificate_chain,
                user,
                &puk.seed,
                team,
            ) {
                Ok(authenticated) => return Ok(authenticated),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or(Error::KeyBinding(
            "no authenticated user private key can open the team",
        )))
    }

    /// Yubi variant of [`Self::load_and_pin_team`]. The delegated software
    /// subkey is used only for mTLS; the already-unboxed PUK signs the team
    /// view challenge and decrypts returned PTKs.
    pub fn load_and_pin_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        if !user.devices().iter().any(|device| {
            device.id == *credential.parent.entity_id()
                && device.hepk == *credential.parent.hepk()
                && device.subkey.as_ref() == Some(&subkey)
        }) {
            return Err(Error::CredentialBinding(
                "Yubi credential is not enrolled in the supplied user state",
            ));
        }
        let mut last_error = None;
        let prior = self.prior_team_for_reload(host, team)?;
        let mut candidates = puks.iter().rev().collect::<Vec<_>>();
        candidates.sort_by_key(|puk| {
            !prior
                .as_ref()
                .is_some_and(|prior| user_puk_matches_team_roster(user, puk, prior, host.host_id()))
        });
        for puk in candidates {
            if user_key_history_for_seed(user, &puk.seed).is_err() {
                continue;
            }
            match self.load_and_pin_team_with_material(
                host,
                &credential.uid,
                &credential.subkey_seed,
                &credential.certificate_chain,
                user,
                &puk.seed,
                team,
            ) {
                Ok(authenticated) => return Ok(authenticated),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or(Error::KeyBinding(
            "no authenticated user private key can open the team",
        )))
    }

    /// Loads a local parent team through a current or historical PTK of one
    /// of its local member teams. The user credential remains the transport
    /// principal; possession of the member-team PTK is the view authority.
    pub fn load_and_pin_team_as_local_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        transport_user: &VerifiedUserState,
        actor: &AuthenticatedTeamOutcome,
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        // The transport user is caller-supplied, so bind it to the credential
        // itself rather than to its UID alone: the chain must be this
        // credential's own and must enroll exactly this device.
        FederationCredential::Software(credential)
            .require_enrolled(host.host_id(), transport_user)?;
        self.load_and_pin_team_as_local_team_with_material(
            host,
            &credential.uid,
            &credential.seed,
            &credential.certificate_chain,
            actor,
            team,
        )
    }

    /// Hardware-backed transport variant of
    /// [`Self::load_and_pin_team_as_local_team`].
    pub fn load_and_pin_team_as_local_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        transport_user: &VerifiedUserState,
        actor: &AuthenticatedTeamOutcome,
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        // Full enrollment, not just the UID and host: the hardware parent's
        // authority must not be reachable through any other chain that
        // happens to share the UID.
        FederationCredential::Yubi(credential).require_enrolled(host.host_id(), transport_user)?;
        self.load_and_pin_team_as_local_team_with_material(
            host,
            &credential.uid,
            &credential.subkey_seed,
            &credential.certificate_chain,
            actor,
            team,
        )
    }

    /// Credential-agnostic form of [`Self::load_and_pin_team`].
    pub fn load_and_pin_team_with_credential(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        match credential {
            FederationCredential::Software(credential) => {
                self.load_and_pin_team(host, credential, user, puks, team)
            }
            FederationCredential::Yubi(credential) => {
                self.load_and_pin_team_yubi(host, credential, user, puks, team)
            }
        }
    }

    /// Credential-agnostic form of [`Self::load_and_pin_team_as_local_team`].
    /// Both arms re-bind the transport user to the credential themselves, so
    /// an actor cannot borrow a chain that merely shares its UID.
    pub fn load_and_pin_team_as_local_team_with_credential(
        &self,
        host: &PinnedHost,
        credential: FederationCredential<'_, '_>,
        transport_user: &VerifiedUserState,
        actor: &AuthenticatedTeamOutcome,
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        match credential {
            FederationCredential::Software(credential) => {
                self.load_and_pin_team_as_local_team(host, credential, transport_user, actor, team)
            }
            FederationCredential::Yubi(credential) => self.load_and_pin_team_as_local_team_yubi(
                host,
                credential,
                transport_user,
                actor,
                team,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn load_and_pin_team_as_local_team_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor: &AuthenticatedTeamOutcome,
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        if actor.verified.host() != host.host_id() || actor.verified.team() == team {
            return Err(Error::TeamBinding(
                "local team actor does not match the target host",
            ));
        }
        let history = verified_team_private_history(actor)?;
        let prior = self.prior_team_for_reload(host, team)?;
        let mut candidates = actor.ptks.iter().rev().collect::<Vec<_>>();
        candidates.sort_by_key(|private| {
            !prior.as_ref().is_some_and(|prior| {
                team_key_for_seed(&history, private).is_ok_and(|public| {
                    prior.members().iter().any(|member| {
                        member.party == *actor.verified.team()
                            && member.scoped_host.is_none()
                            && member.source_role == public.role
                            && member.generation == public.generation
                            && member.verify_key == public.verify_key
                    })
                })
            })
        });
        let mut last_error = None;
        for private in candidates {
            let Ok(source) = team_key_for_seed(&history, private) else {
                continue;
            };
            let view_actor = TeamViewActor {
                party: actor.verified.team(),
                host: actor.verified.host(),
                source,
                history: &history,
                seed: &private.seed,
            };
            match self.load_and_pin_team_for_actor_with_material(
                host,
                uid,
                auth_seed,
                certificate_chain,
                view_actor,
                team,
            ) {
                Ok(authenticated) => return Ok(authenticated),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or(Error::KeyBinding(
            "no authenticated member-team private key can open the parent team",
        )))
    }

    /// Creates a FOKS v0.1.9 single-owner ad-hoc team and then reconciles the
    /// predicted TeamID through the ordinary verified team-loading path.
    ///
    /// `secrets` must already be committed to the caller's encrypted
    /// credential store. They remain caller-owned if posting or reconciliation
    /// fails, so the operation can be resumed by loading the TeamID derived
    /// from the admin PTK.
    pub fn create_single_owner_adhoc_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        let (authenticated_user, membership) = self.retry_chain_load(host, |current| {
            let authenticated = self.authenticate_and_pin(current, credential)?;
            let membership = self.load_membership_chain_tail(
                current,
                &credential.uid,
                &credential.seed,
                &credential.certificate_chain,
                &authenticated.verified,
            )?;
            Ok((authenticated, membership))
        })?;
        self.require_open_user_viewership(host, &credential.seed, &credential.certificate_chain)?;
        require_nonstale_shared_key(&authenticated_user.verified, Role::OWNER)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = credential.public_material()?.id;
        let device = authenticated_user
            .verified
            .devices()
            .iter()
            .find(|device| device.id == device_id)
            .ok_or(Error::CredentialBinding(
                "team creator device is not enrolled",
            ))?;
        if device.role != Role::OWNER {
            return Err(Error::CredentialBinding(
                "team creator device is not an owner",
            ));
        }
        let ordered_seeds = secrets.ordered();
        let material = make_single_owner_adhoc_team(
            &AdHocTeamInput {
                user: &credential.uid,
                host: host.host_id(),
                root: &authenticated_user.verified.tree_root(),
                time: now_milliseconds()?,
                owner_puk_generation: owner.generation,
                membership_sequence: membership.sequence,
                membership_previous: membership.previous,
                next_tree_location: random_bytes()?,
                subchain_tree_location: random_bytes()?,
                membership_next_tree_location: random_bytes()?,
            },
            &credential.seed,
            &owner.seed,
            ordered_seeds,
        )?;
        self.finish_single_owner_adhoc_team_creation(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            material,
            secrets,
        )
    }

    /// Hardware-backed variant of [`Self::create_single_owner_adhoc_team`].
    /// The Yubi parent signs the membership chain while the delegated software
    /// subkey is used only for mTLS.
    pub fn create_single_owner_adhoc_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        let (authenticated_user, membership) = self.retry_chain_load(host, |current| {
            let authenticated = self.authenticate_yubi_and_pin(current, credential)?;
            let membership = self.load_membership_chain_tail(
                current,
                &credential.uid,
                &credential.subkey_seed,
                &credential.certificate_chain,
                &authenticated.verified,
            )?;
            Ok((authenticated, membership))
        })?;
        self.require_open_user_viewership(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        require_nonstale_shared_key(&authenticated_user.verified, Role::OWNER)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        let device = authenticated_user
            .verified
            .devices()
            .iter()
            .find(|device| {
                device.id == *credential.parent.entity_id()
                    && device.hepk == *credential.parent.hepk()
                    && device.subkey.as_ref() == Some(&subkey)
            })
            .ok_or(Error::CredentialBinding(
                "Yubi team creator is not enrolled",
            ))?;
        if device.role != Role::OWNER {
            return Err(Error::CredentialBinding(
                "Yubi team creator is not an owner",
            ));
        }
        let ordered_seeds = secrets.ordered();
        let material = make_single_owner_adhoc_team_yubi(
            &AdHocTeamInput {
                user: &credential.uid,
                host: host.host_id(),
                root: &authenticated_user.verified.tree_root(),
                time: now_milliseconds()?,
                owner_puk_generation: owner.generation,
                membership_sequence: membership.sequence,
                membership_previous: membership.previous,
                next_tree_location: random_bytes()?,
                subchain_tree_location: random_bytes()?,
                membership_next_tree_location: random_bytes()?,
            },
            credential.parent,
            &owner.seed,
            ordered_seeds,
        )?;
        self.finish_single_owner_adhoc_team_creation(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            material,
            secrets,
        )
    }

    fn require_open_user_viewership(
        &self,
        host: &PinnedHost,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
    ) -> Result<()> {
        let host_config = self.host_config_with_material(host, auth_seed, certificate_chain)?;
        if host_config.user_viewership != ViewershipMode::Open {
            return Err(Error::TeamRequest(
                "host does not allow open user viewership",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_single_owner_adhoc_team_creation(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        authenticated_user: &AuthenticatedUserOutcome,
        owner: &UserPrivateKey,
        material: AdHocTeamMaterial,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        let ordered_seeds = secrets.ordered();
        let owner_public = foks_crypto::derive_shared_public(&owner.seed, ENTITY_PUK_VERIFY)?;
        let box_inputs = ordered_seeds
            .into_iter()
            .zip([
                Role::member(-0x4000),
                Role::member(0),
                Role::ADMIN,
                Role::OWNER,
            ])
            .map(|(seed, role)| SharedKeyBoxInput {
                seed,
                generation: 1,
                role,
                receiver_id: uid,
                receiver_host: None,
                receiver_hepk: &owner_public.hepk,
                receiver_role: Role::OWNER,
                receiver_generation: owner.generation,
            })
            .collect::<Vec<_>>();
        let randomness = (0..box_inputs.len())
            .map(|_| {
                Ok(PukBoxRandomness {
                    kem_message: random_bytes()?,
                    nonce: random_bytes()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let boxes = seal_shared_key_boxes(
            host.host_id(),
            &owner.seed,
            &owner_public.hepk,
            random_bytes()?,
            &box_inputs,
            &randomness,
        )?;
        let hepks = material
            .ptks
            .iter()
            .map(|ptk| ptk.hepk.clone())
            .collect::<Vec<_>>();
        let request = encode_create_adhoc_team_request(&AdHocTeamCreateArgument {
            link: &material.link,
            next_tree_location: material.next_tree_location,
            ptk_boxes: &boxes,
            hepks: &hepks,
            subchain_tree_location: material.subchain_tree_location,
            membership_link: &material.membership_link,
            membership_next_tree_location: material.membership_next_tree_location,
        })?;
        let operation_id = secrets.operation_id()?;
        let created_at = now_microseconds()?;
        let operation = AdHocTeamOperation {
            operation_id,
            host_id: host.host_id().as_bytes().to_vec(),
            uid: uid.as_bytes().to_vec(),
            device_id: device_id.as_bytes().to_vec(),
            team_id: material.team.as_bytes().to_vec(),
            request_hash: prefixed_hash(ADHOC_TEAM_REQUEST_HASH_TYPE_ID, &request),
            state: AdHocTeamOperationState::Prepared,
            created_at,
            updated_at: created_at,
        };
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        hard_store.record_adhoc_team_operation(&operation)?;
        let post = || {
            self.call_void_with_material(host, &host.user, &request, auth_seed, certificate_chain)
        };
        let post_error = match post() {
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: STATUS_TX_RETRY_ERROR,
                ..
            })) => post().err(),
            Err(error) => Some(error),
            Ok(()) => None,
        };
        if post_error.is_none() {
            hard_store.advance_adhoc_team_operation(
                &operation_id,
                AdHocTeamOperationState::Submitted,
                now_microseconds()?,
            )?;
        }
        if matches!(
            post_error.as_ref(),
            Some(Error::Rpc(foks_rpc::Error::RemoteStatus { .. }))
        ) {
            return Err(post_error.expect("matched above"));
        }
        let authenticated = match self.wait_for_adhoc_team_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            authenticated_user,
            &owner.seed,
            &material.team,
            secrets,
        ) {
            Ok(authenticated) => authenticated,
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        let persisted =
            hard_store
                .adhoc_team_operation(&operation_id)?
                .ok_or(Error::OperationBinding(
                    "ad-hoc team operation record was not found after submission",
                ))?;
        if persisted.state == AdHocTeamOperationState::Prepared {
            hard_store.advance_adhoc_team_operation(
                &operation_id,
                AdHocTeamOperationState::Submitted,
                now_microseconds()?,
            )?;
        }
        hard_store.advance_adhoc_team_operation(
            &operation_id,
            AdHocTeamOperationState::Verified,
            now_microseconds()?,
        )?;
        Ok(CreatedAdHocTeam {
            operation_id,
            team: material.team,
            authenticated,
        })
    }

    /// Reconciles a journaled ad-hoc creation without replaying the mutation.
    pub fn resume_single_owner_adhoc_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        let device_id = credential.public_material()?.id;
        self.ensure_adhoc_team_operation_binding(host, &credential.uid, &device_id, secrets)?;
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        self.resume_single_owner_adhoc_team_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            &owner.seed,
            secrets,
        )
    }

    /// Hardware-backed variant of [`Self::resume_single_owner_adhoc_team`].
    /// It performs verified reconciliation only and never reposts the mutation.
    pub fn resume_single_owner_adhoc_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        self.ensure_adhoc_team_operation_binding(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            secrets,
        )?;
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        self.resume_single_owner_adhoc_team_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            &owner.seed,
            secrets,
        )
    }

    fn ensure_adhoc_team_operation_binding(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        secrets: &AdHocTeamSecrets,
    ) -> Result<()> {
        let operation_id = secrets.operation_id()?;
        let operation = HardStateStore::open(&host.database_path)?
            .adhoc_team_operation(&operation_id)?
            .ok_or(Error::AccountRequest(
                "ad-hoc team operation is not recorded",
            ))?;
        let team = secrets.team_id()?;
        if operation.host_id != host.host_id().as_bytes()
            || operation.uid != uid.as_bytes()
            || operation.device_id != device_id.as_bytes()
            || operation.team_id != team.as_bytes()
        {
            return Err(Error::OperationBinding(
                "ad-hoc team operation does not match supplied identities",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn resume_single_owner_adhoc_team_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        authenticated_user: &AuthenticatedUserOutcome,
        owner_seed: &SecretSeed,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        let operation_id = secrets.operation_id()?;
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation =
            hard_store
                .adhoc_team_operation(&operation_id)?
                .ok_or(Error::AccountRequest(
                    "ad-hoc team operation is not recorded",
                ))?;
        let team = secrets.team_id()?;
        if operation.host_id != host.host_id().as_bytes()
            || operation.uid != uid.as_bytes()
            || operation.device_id != device_id.as_bytes()
            || operation.team_id != team.as_bytes()
        {
            return Err(Error::OperationBinding(
                "ad-hoc team operation does not match supplied identities",
            ));
        }
        let authenticated = self.wait_for_adhoc_team_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            authenticated_user,
            owner_seed,
            &team,
            secrets,
        )?;
        if operation.state == AdHocTeamOperationState::Prepared {
            hard_store.advance_adhoc_team_operation(
                &operation_id,
                AdHocTeamOperationState::Submitted,
                now_microseconds()?,
            )?;
        }
        if operation.state != AdHocTeamOperationState::Verified {
            hard_store.advance_adhoc_team_operation(
                &operation_id,
                AdHocTeamOperationState::Verified,
                now_microseconds()?,
            )?;
        }
        Ok(CreatedAdHocTeam {
            operation_id,
            team,
            authenticated,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_for_adhoc_team_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        user: &AuthenticatedUserOutcome,
        owner_seed: &SecretSeed,
        team: &EntityId,
        secrets: &AdHocTeamSecrets,
    ) -> Result<AuthenticatedTeamOutcome> {
        let roles_and_seeds = [
            (Role::member(-0x4000), &secrets.member_min),
            (Role::member(0), &secrets.member),
            (Role::ADMIN, &secrets.admin),
            (Role::OWNER, &secrets.owner),
        ];
        let mut last_error = None;
        for attempt in 0..40 {
            match self.load_and_pin_team_with_material(
                host,
                uid,
                auth_seed,
                certificate_chain,
                &user.verified,
                owner_seed,
                team,
            ) {
                Ok(authenticated)
                    if authenticated.ptks.len() == roles_and_seeds.len()
                        && roles_and_seeds.iter().all(|(role, seed)| {
                            authenticated.ptks.iter().any(|key| {
                                key.role == *role && key.generation == 1 && key.seed == **seed
                            })
                        }) =>
                {
                    return Ok(authenticated);
                }
                Ok(_) => {
                    last_error = Some(Error::TransitionNotObserved(
                        "ad-hoc team keys do not match prepared secrets",
                    ));
                }
                Err(error) => last_error = Some(error),
            }
            if attempt != 39 {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        Err(last_error.expect("team transition loop executes at least once"))
    }

    #[allow(clippy::too_many_arguments)]
    fn load_and_pin_team_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        user: &VerifiedUserState,
        puk_seed: &SecretSeed,
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        if user.uid() != uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "user state does not match the credential and pinned host",
            ));
        }
        let source = user_key_history_for_seed(user, puk_seed)?;
        self.load_and_pin_team_for_actor_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            TeamViewActor {
                party: uid,
                host: host.host_id(),
                source,
                history: user.shared_key_history(),
                seed: puk_seed,
            },
            team,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn load_and_pin_team_for_actor_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor: TeamViewActor<'_>,
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        let token = self.activate_team_view_for_actor_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            &actor,
            team,
        )?;
        self.load_and_pin_team_for_actor_with_view_token(
            host,
            uid,
            auth_seed,
            certificate_chain,
            actor,
            team,
            &token,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn activate_team_view_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        user: &VerifiedUserState,
        puk_seed: &SecretSeed,
        team: &EntityId,
    ) -> Result<[u8; 16]> {
        if user.uid() != uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "user state does not match the credential and pinned host",
            ));
        }
        let source = user_key_history_for_seed(user, puk_seed)?;
        self.activate_team_view_for_actor_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            &TeamViewActor {
                party: uid,
                host: host.host_id(),
                source,
                history: user.shared_key_history(),
                seed: puk_seed,
            },
            team,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn activate_team_view_for_actor_with_material(
        &self,
        host: &PinnedHost,
        _uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor: &TeamViewActor<'_>,
        team: &EntityId,
    ) -> Result<[u8; 16]> {
        if actor.host != host.host_id() {
            return Err(Error::TeamBinding(
                "team view actor is not local to the target host",
            ));
        }
        let view_request = TeamViewRequest {
            team: team.clone(),
            host: host.host_id.clone(),
            member: actor.party.clone(),
            member_host: actor.host.clone(),
            source_role: actor.source.role,
            generation: actor.source.generation,
        };
        let challenge_bytes = self.call_with_material(
            host,
            &host.user,
            &encode_team_view_challenge_request(&view_request)?,
            auth_seed,
            certificate_chain,
        )?;
        let mut challenge = TeamViewChallenge::decode(&challenge_bytes)?;
        // Never sign server-selected authorization fields. FOKS v0.1.9's Go
        // client likewise overwrites the echoed request before signing.
        challenge.request = view_request;
        let signature = sign_shared_key_typed(
            actor.seed,
            TEAM_VIEW_CHALLENGE_TYPE_ID,
            &challenge.encoded()?,
        )?;
        let activated_bytes = self.call_with_material(
            host,
            &host.user,
            &encode_activate_team_view_request(&challenge, &signature)?,
            auth_seed,
            certificate_chain,
        )?;
        let activated = ActivatedTeamView::decode(&activated_bytes)?;
        if activated.team != *team || activated.token != challenge.token {
            return Err(Error::TeamBinding(
                "activated view does not match its challenge",
            ));
        }

        Ok(activated.token)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn load_and_pin_team_with_view_token(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        user: &VerifiedUserState,
        puk_seed: &SecretSeed,
        team: &EntityId,
        view_token: &[u8; 16],
    ) -> Result<AuthenticatedTeamOutcome> {
        if user.uid() != uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "user state does not match the credential and pinned host",
            ));
        }
        let source = user_key_history_for_seed(user, puk_seed)?;
        self.load_and_pin_team_for_actor_with_view_token(
            host,
            uid,
            auth_seed,
            certificate_chain,
            TeamViewActor {
                party: uid,
                host: host.host_id(),
                source,
                history: user.shared_key_history(),
                seed: puk_seed,
            },
            team,
            view_token,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn load_and_pin_team_for_actor_with_view_token(
        &self,
        host: &PinnedHost,
        _uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        actor: TeamViewActor<'_>,
        team: &EntityId,
        view_token: &[u8; 16],
    ) -> Result<AuthenticatedTeamOutcome> {
        if actor.host != host.host_id() {
            return Err(Error::TeamBinding(
                "team view actor is not local to the target host",
            ));
        }
        let (merkle_acceptance, chain_bytes, verified) = self.retry_chain_load(host, |host| {
            let (merkle_acceptance, merkle) = self.advance_merkle_root(host)?;
            let prior = self.prior_team_for_reload(host, team)?;
            let (start, name) = match prior.as_ref() {
                Some(prior) => (
                    prior
                        .chain_seqno()
                        .checked_add(1)
                        .ok_or(Error::TeamBinding("team chain sequence overflow"))?,
                    Some((
                        prior.team_name(),
                        prior
                            .team_name_sequence()
                            .checked_add(1)
                            .ok_or(Error::TeamBinding("team name sequence overflow"))?,
                    )),
                ),
                None => (1, None),
            };
            let chain_bytes = self.call_with_material(
                host,
                &host.user,
                &encode_load_team_chain_request_from(
                    team,
                    host.host_id(),
                    view_token,
                    start,
                    name,
                )?,
                auth_seed,
                certificate_chain,
            )?;
            let authenticated_roots =
                self.authenticate_team_chain_roots(host, &merkle, &chain_bytes)?;
            let verified = match prior.as_ref() {
                Some(prior) => verify_team_chain_increment(
                    &chain_bytes,
                    prior,
                    team,
                    host.host_id(),
                    &authenticated_roots,
                    &merkle,
                )?,
                None => verify_team_chain(
                    &chain_bytes,
                    team,
                    host.host_id(),
                    &authenticated_roots,
                    &merkle,
                )?,
            };
            Ok((merkle_acceptance, chain_bytes, verified))
        })?;
        let member = verified
            .members()
            .iter()
            .find(|member| {
                member.party == *actor.party
                    && member.source_role == actor.source.role
                    && member
                        .scoped_host
                        .as_ref()
                        .is_none_or(|scope| scope == host.host_id())
            })
            .ok_or(Error::TeamBinding(
                "requesting user is not in the verified team roster",
            ))?;
        if member.verify_key.as_bytes()[1..] != actor.source.verify_key.as_bytes()[1..]
            || member.generation != actor.source.generation
        {
            return Err(Error::TeamBinding(
                "team membership key does not match the requesting PUK",
            ));
        }
        let chain = TeamChain::decode(&chain_bytes)?;
        let receiver = SharedKeyDecapsulator::new(actor.seed, actor.party.clone())?;
        let expected_keys = verified
            .shared_keys()
            .iter()
            .filter(|key| key.role <= member.role)
            .collect::<Vec<_>>();
        if chain.boxes.len() != expected_keys.len() {
            return Err(Error::TeamBinding(
                "team key boxes do not match visible PTK roles",
            ));
        }
        let mut ptks = Vec::with_capacity(expected_keys.len());
        for key in expected_keys {
            let parcels = chain
                .boxes
                .iter()
                .filter(|parcel| parcel.role == key.role)
                .collect::<Vec<_>>();
            let [parcel] = parcels.as_slice() else {
                return Err(Error::TeamBinding(
                    "team PTK role has a missing or duplicate parcel",
                ));
            };
            let sender_hepk = team_parcel_sender_hepk(
                parcel,
                actor.source,
                actor.history,
                host.host_id(),
                verified.members(),
                &chain.hepks,
            )?;
            let clear = open_shared_key_parcel_with(
                parcel,
                &receiver,
                sender_hepk,
                &key.verify_key,
                &key.hepk,
                key.generation,
                host.host_id(),
                actor.source.role,
                actor.source.generation,
                key.role,
                ENTITY_PTK_VERIFY,
            )?;
            ptks.extend({
                let history = verified.shared_key_history()?;
                let opened =
                    foks_crypto::open_team_shared_key_seed_chain(clear, parcel, host.host_id())?;
                let expected_generations = history
                    .iter()
                    .filter(|key| key.role == parcel.role && key.generation <= parcel.generation)
                    .map(|key| key.generation)
                    .collect::<std::collections::BTreeSet<_>>();
                let opened_generations = opened
                    .iter()
                    .map(|key| key.generation)
                    .collect::<std::collections::BTreeSet<_>>();
                if opened_generations != expected_generations {
                    return Err(Error::KeyBinding("team PTK seed chain is incomplete"));
                }
                opened
                    .into_iter()
                    .map(|key| {
                        let public = foks_crypto::derive_shared_public(
                            &key.seed,
                            foks_proto::ENTITY_PTK_VERIFY,
                        )?;
                        let fingerprint = foks_crypto::hepk_fingerprint(&public.hepk)?;
                        if !history.iter().any(|authenticated| {
                            authenticated.role == key.role
                                && authenticated.generation == key.generation
                                && authenticated.verify_key == public.verify_key
                                && authenticated.hepk_fingerprint == fingerprint
                        }) {
                            return Err(Error::KeyBinding(
                                "team PTK seed chain is absent from authenticated history",
                            ));
                        }
                        Ok(TeamPrivateKey {
                            role: key.role,
                            generation: key.generation,
                            seed: key.into_seed(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            });
        }
        let mut store = HardStateStore::open(&host.database_path)?;
        let acceptance = store.accept_verified_team(&verified.hard_state_snapshot()?)?;
        Ok(AuthenticatedTeamOutcome {
            merkle_acceptance,
            acceptance,
            verified,
            ptks,
            view_token: *view_token,
        })
    }
}

fn user_puk_matches_team_roster(
    user: &VerifiedUserState,
    private: &UserPrivateKey,
    team: &VerifiedTeamState,
    host: &EntityId,
) -> bool {
    user_key_history_for_seed(user, &private.seed).is_ok_and(|public| {
        private.role == public.role
            && private.generation == public.generation
            && team.members().iter().any(|member| {
                member.party == *user.uid()
                    && member
                        .scoped_host
                        .as_ref()
                        .is_none_or(|scope| scope == host)
                    && member.source_role == public.role
                    && member.generation == public.generation
                    && member.verify_key.as_bytes()[1..] == public.verify_key.as_bytes()[1..]
            })
    })
}

fn team_key_for_seed<'a>(
    history: &'a [foks_verify::VerifiedSharedKey],
    private: &TeamPrivateKey,
) -> Result<&'a foks_verify::VerifiedSharedKey> {
    let public = foks_crypto::derive_shared_public(&private.seed, ENTITY_PTK_VERIFY)?;
    let fingerprint = foks_crypto::hepk_fingerprint(&public.hepk)?;
    let mut matches = history.iter().filter(|key| {
        key.role == private.role
            && key.generation == private.generation
            && key.verify_key == public.verify_key
            && foks_crypto::hepk_fingerprint(&key.hepk).ok() == Some(fingerprint)
    });
    let key = matches.next().ok_or(Error::KeyBinding(
        "team PTK is absent from verified history",
    ))?;
    if matches.next().is_some() {
        return Err(Error::KeyBinding(
            "team PTK is ambiguous in verified history",
        ));
    }
    Ok(key)
}

fn verified_team_private_history(
    team: &AuthenticatedTeamOutcome,
) -> Result<Vec<foks_verify::VerifiedSharedKey>> {
    let public_history = team.verified.shared_key_history()?;
    let mut verified = Vec::with_capacity(team.ptks.len());
    for private in &team.ptks {
        let public = foks_crypto::derive_shared_public(&private.seed, ENTITY_PTK_VERIFY)?;
        let fingerprint = foks_crypto::hepk_fingerprint(&public.hepk)?;
        if !public_history.iter().any(|key| {
            key.role == private.role
                && key.generation == private.generation
                && key.verify_key == public.verify_key
                && key.hepk_fingerprint == fingerprint
        }) {
            return Err(Error::KeyBinding(
                "team PTK private history is not authenticated by the team chain",
            ));
        }
        if verified.iter().any(|key: &foks_verify::VerifiedSharedKey| {
            key.role == private.role && key.generation == private.generation
        }) {
            return Err(Error::KeyBinding(
                "team PTK private history contains a duplicate role generation",
            ));
        }
        verified.push(foks_verify::VerifiedSharedKey {
            role: private.role,
            generation: private.generation,
            verify_key: public.verify_key,
            hepk: public.hepk,
        });
    }
    Ok(verified)
}

fn team_parcel_sender_hepk<'a>(
    parcel: &foks_proto::PukParcel,
    receiver: &'a foks_verify::VerifiedSharedKey,
    user_history: &'a [foks_verify::VerifiedSharedKey],
    expected_host: &foks_proto::EntityId,
    members: &'a [foks_verify::VerifiedTeamMemberState],
    hepks: &'a [foks_proto::Hepk],
) -> Result<&'a foks_proto::Hepk> {
    let sender = parcel.sender.to_rolling_entity_id();
    if sender == receiver.verify_key {
        return Ok(&receiver.hepk);
    }
    let mut historical = user_history.iter().filter(|key| key.verify_key == sender);
    if let Some(sender) = historical.next() {
        if historical.next().is_some() {
            return Err(Error::TeamBinding(
                "team PTK parcel sender is ambiguous in user history",
            ));
        }
        return Ok(&sender.hepk);
    }
    let mut senders = members.iter().filter(|member| {
        member.verify_key == sender
            && member
                .scoped_host
                .as_ref()
                .is_none_or(|scope| scope == expected_host)
    });
    let sender = senders.next().ok_or(Error::TeamBinding(
        "team PTK parcel sender is not in the verified roster",
    ))?;
    if senders.next().is_some() {
        return Err(Error::TeamBinding(
            "team PTK parcel sender is ambiguous in the verified roster",
        ));
    }
    let mut matched = None;
    for candidate in hepks {
        if hepk_fingerprint(candidate)? != sender.hepk_fingerprint {
            continue;
        }
        if matched.replace(candidate).is_some() {
            return Err(Error::TeamBinding(
                "team PTK parcel sender HEPK is ambiguous",
            ));
        }
    }
    matched.ok_or(Error::TeamBinding(
        "team PTK parcel sender HEPK is unavailable",
    ))
}

#[cfg(test)]
mod membership_graph_tests {
    use std::collections::{BTreeMap, BTreeSet};

    use foks_proto::{EntityId, ENTITY_NAMED_TEAM};

    use super::{child_first_team_order, local_team_graph_path_is_unavailable};

    fn team(fill: u8) -> EntityId {
        let mut bytes = vec![fill; 33];
        bytes[0] = ENTITY_NAMED_TEAM;
        EntityId::from_bytes(bytes).unwrap()
    }

    #[test]
    fn nested_membership_order_is_child_first_and_deduplicated() {
        let a = team(0x11);
        let b = team(0x22);
        let c = team(0x33);
        let d = team(0x44);
        let indexes = [&a, &b, &c, &d]
            .into_iter()
            .enumerate()
            .map(|(index, team)| (team.as_bytes().to_vec(), index))
            .collect::<BTreeMap<_, _>>();
        let edges = BTreeSet::from([
            (a.as_bytes().to_vec(), b.as_bytes().to_vec()),
            (a.as_bytes().to_vec(), c.as_bytes().to_vec()),
            (b.as_bytes().to_vec(), d.as_bytes().to_vec()),
            (c.as_bytes().to_vec(), d.as_bytes().to_vec()),
        ]);
        let order = child_first_team_order(&indexes, &edges).unwrap();
        let position = |team: &EntityId| order.iter().position(|item| item == team).unwrap();
        assert!(position(&a) < position(&b));
        assert!(position(&a) < position(&c));
        assert!(position(&b) < position(&d));
        assert!(position(&c) < position(&d));
        assert_eq!(order.len(), 4);
    }

    #[test]
    fn nested_membership_cycle_is_rejected() {
        let a = team(0x51);
        let b = team(0x52);
        let indexes = [&a, &b]
            .into_iter()
            .enumerate()
            .map(|(index, team)| (team.as_bytes().to_vec(), index))
            .collect::<BTreeMap<_, _>>();
        let edges = BTreeSet::from([
            (a.as_bytes().to_vec(), b.as_bytes().to_vec()),
            (b.as_bytes().to_vec(), a.as_bytes().to_vec()),
        ]);
        assert!(child_first_team_order(&indexes, &edges).is_err());
    }

    #[test]
    fn unavailable_direct_hints_do_not_hide_authenticated_binding_failures() {
        assert!(local_team_graph_path_is_unavailable(
            &crate::Error::KeyBinding("private source-role key is unavailable")
        ));
        assert!(!local_team_graph_path_is_unavailable(
            &crate::Error::TeamBinding("authenticated roster is inconsistent")
        ));
    }
}

#[cfg(test)]
mod parcel_sender_tests {
    use foks_crypto::{derive_shared_public, hepk_fingerprint};
    use foks_proto::{
        EntityId, PukParcel, Role, SecretSeed, SharedKeyBoxSet, UserLink, ENTITY_HOST, ENTITY_USER,
    };
    use foks_snowpack::{decode, Value};
    use foks_verify::{VerifiedSharedKey, VerifiedTeamMemberState};

    use super::team_parcel_sender_hepk;

    const MUTATION_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";
    const USER_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user";

    fn fixture(directory: &str, name: &str) -> Vec<u8> {
        std::fs::read(format!("{directory}/{name}")).unwrap()
    }

    fn entity_fixture(name: &str) -> EntityId {
        match decode(&fixture(MUTATION_DIR, name)).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            other => panic!("expected entity fixture, got {other:?}"),
        }
    }

    #[test]
    fn cross_user_ptk_sender_is_resolved_from_authenticated_roster() {
        let eldest = UserLink::decode(&fixture(MUTATION_DIR, "named-team-link.snowp")).unwrap();
        let owner_change = eldest.decode_team_group_change().unwrap().changes.remove(0);
        let actor_seed = SecretSeed::new(fixture(USER_DIR, "puk-seed.bin").try_into().unwrap());
        let actor = derive_shared_public(&actor_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let target_seed = SecretSeed::new(
            fixture(MUTATION_DIR, "add-member-target-puk-seed.bin")
                .try_into()
                .unwrap(),
        );
        let target = derive_shared_public(&target_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let member = VerifiedTeamMemberState {
            party: owner_change.party,
            scoped_host: None,
            source_role: Role::OWNER,
            role: Role::OWNER,
            generation: owner_change.keys.as_ref().unwrap().generation,
            verify_key: actor.verify_key.clone(),
            hepk_fingerprint: hepk_fingerprint(&actor.hepk).unwrap(),
            removal_key_commitment: owner_change.keys.as_ref().unwrap().removal_key_commitment,
            index_range: owner_change.keys.as_ref().unwrap().index_range.clone(),
        };
        let receiver = VerifiedSharedKey {
            role: Role::OWNER,
            generation: 3,
            verify_key: target.verify_key,
            hepk: target.hepk,
        };
        let boxes =
            SharedKeyBoxSet::decode(&fixture(MUTATION_DIR, "add-member-ptk-boxes.snowp")).unwrap();
        let boxed = boxes.boxes.first().unwrap();
        let mut persistent_sender = actor.verify_key.as_bytes().to_vec();
        persistent_sender[0] = ENTITY_USER;
        let persistent_sender = EntityId::from_bytes(persistent_sender).unwrap();
        let parcel = PukParcel {
            generation: boxed.generation,
            role: boxed.role,
            hybrid: boxed.hybrid.clone(),
            target: entity_fixture("add-member-target-uid.snowp"),
            target_host: None,
            target_role: boxed.target.role,
            target_generation: boxed.target.generation,
            sender: persistent_sender,
            box_id: boxes.box_id,
            temp_dh_key: boxes.temp_dh_key,
            seed_chain: Vec::new(),
        };
        let host = EntityId::from_bytes([vec![ENTITY_HOST], vec![0x44; 32]].concat()).unwrap();
        let remote_host =
            EntityId::from_bytes([vec![ENTITY_HOST], vec![0x55; 32]].concat()).unwrap();
        let actor_receiver = VerifiedSharedKey {
            role: Role::OWNER,
            generation: member.generation,
            verify_key: actor.verify_key.clone(),
            hepk: actor.hepk.clone(),
        };
        assert_eq!(
            team_parcel_sender_hepk(&parcel, &actor_receiver, &[], &host, &[], &[]).unwrap(),
            &actor.hepk
        );
        assert_eq!(
            team_parcel_sender_hepk(
                &parcel,
                &receiver,
                &[],
                &host,
                std::slice::from_ref(&member),
                std::slice::from_ref(&actor.hepk),
            )
            .unwrap(),
            &actor.hepk
        );
        let mut remote_member = member.clone();
        remote_member.scoped_host = Some(remote_host);
        assert!(team_parcel_sender_hepk(
            &parcel,
            &receiver,
            &[],
            &host,
            std::slice::from_ref(&remote_member),
            std::slice::from_ref(&actor.hepk),
        )
        .is_err());
        assert!(team_parcel_sender_hepk(
            &parcel,
            &receiver,
            &[],
            &host,
            std::slice::from_ref(&member),
            &[],
        )
        .is_err());
        let historical = VerifiedSharedKey {
            role: Role::OWNER,
            generation: member.generation,
            verify_key: actor.verify_key,
            hepk: actor.hepk,
        };
        assert_eq!(
            team_parcel_sender_hepk(
                &parcel,
                &receiver,
                std::slice::from_ref(&historical),
                &host,
                &[],
                &[],
            )
            .unwrap(),
            &historical.hepk
        );
    }
}
