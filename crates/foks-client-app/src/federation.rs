use super::*;
use crate::runtime::{
    local_team_can_observe_team_rekey, TeamRefreshCredential, TeamRefreshParty, TeamRefreshPartyKey,
};
use foks_client::{
    FederationCredential, FoksScheduler, ScheduledJobRegistration, SchedulerConfig, YubiCredential,
};

const FEDERATION_JOB_TYPE_ID: u64 = 0xc426_0c8c_25bb_912d;
const FEDERATION_REFRESH_INTERVAL_MICROS: u64 = 17 * 60 * 1_000_000;
const FEDERATION_FIRST_RETRY_MICROS: u64 = 5 * 60 * 1_000_000;

/// The durable identities of the ordinary refresh jobs a profile registers for
/// each of its users, read here to decide whether that profile's own scheduler
/// has already swept. Taken from the registration side rather than restated,
/// so a job identity cannot drift between the two.
use crate::runtime::{refresh_job_id, TEAM_REFRESH_JOB_TYPE_ID, USER_REFRESH_JOB_TYPE_ID};

/// Counts of the network work one federation refresh performed, kept per
/// thread because a refresh runs to completion on the thread that started it.
///
/// These exist so the cost of a refresh can be asserted on rather than
/// described: the reuse rules below are only worth their complexity if they
/// actually remove loads, and only safe if the post-commit selection still
/// shows up as a load. Recording them unconditionally keeps the instrumented
/// path and the shipped path the same code.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FederationReadCounters {
    /// Authenticated local team graphs discovered for a federated selection.
    graph_loads: u64,
    /// Selections served from a graph a previous load had already produced.
    reused_selections: u64,
    /// Remote profile sessions opened, one connection pool each.
    remote_sessions: u64,
    /// Remote responder sweeps this cascade ran.
    ran_responders: u64,
    /// Remote responder sweeps skipped because the remote profile's own
    /// scheduler had already run them within their interval.
    skipped_responders: u64,
    /// Skipped sweeps a stale recipient forced this cascade to run after all.
    unskipped_responders: u64,
    /// `graph_loads` as it stood when the last commit was recorded. A refresh
    /// whose final value is higher performed a fresh authenticated read after
    /// everything it committed.
    graph_loads_at_last_commit: u64,
}

thread_local! {
    static FEDERATION_READ_COUNTERS: std::cell::Cell<FederationReadCounters> =
        const { std::cell::Cell::new(FederationReadCounters {
            graph_loads: 0,
            reused_selections: 0,
            remote_sessions: 0,
            ran_responders: 0,
            skipped_responders: 0,
            unskipped_responders: 0,
            graph_loads_at_last_commit: 0,
        }) };
}

fn count_federation_read(field: impl FnOnce(&mut FederationReadCounters)) {
    FEDERATION_READ_COUNTERS.with(|counters| {
        let mut current = counters.get();
        field(&mut current);
        counters.set(current);
    });
}

#[cfg(test)]
fn federation_read_counters() -> FederationReadCounters {
    FEDERATION_READ_COUNTERS.with(std::cell::Cell::get)
}

/// One already-unlocked hardware credential offered to a federation refresh.
/// Only the caller's stack holds it; no PIN and no hardware handle is stored,
/// journaled, or written to any vault by this module. `profile` binds the
/// credential to the profile that enrolled it, so a credential offered for one
/// side of a federation can never be used as the other side's transport.
#[derive(Clone, Copy)]
pub struct UnlockedYubiActor<'a, 'device> {
    pub profile: &'a str,
    pub alias: &'a str,
    pub credential: &'a YubiCredential<'device>,
}

/// What one federation security refresh actually did. A refresh that could not
/// proceed without hardware reports `deferred` instead of failing, so an
/// unattended scheduler never treats an action-required state as an error and
/// never applies retry backoff to that action-required state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FederationRefreshReport {
    pub local_profile: String,
    pub local_team_alias: String,
    pub refreshed: bool,
    pub deferred: Option<String>,
}

/// A federation actor's acting credential. Software credentials are read from
/// the encrypted vault and owned here; hardware credentials are only ever
/// borrowed from a caller that already unlocked the device.
enum FederatedActorCredential<'a, 'device> {
    Software(foks_client::DeviceCredential),
    Yubi {
        alias: &'a str,
        credential: &'a YubiCredential<'device>,
    },
}

impl<'device> FederatedActorCredential<'_, 'device> {
    fn borrowed(&self) -> FederationCredential<'_, 'device> {
        match self {
            Self::Software(credential) => FederationCredential::Software(credential),
            Self::Yubi { credential, .. } => FederationCredential::Yubi(credential),
        }
    }

    fn team_refresh(&self) -> TeamRefreshCredential<'_, 'device> {
        match self {
            Self::Software(credential) => TeamRefreshCredential::Software(credential),
            Self::Yubi { credential, .. } => TeamRefreshCredential::Yubi(credential),
        }
    }

    fn uid(&self) -> &EntityId {
        match self {
            Self::Software(credential) => &credential.uid,
            Self::Yubi { credential, .. } => &credential.uid,
        }
    }
}

/// One authenticated credential together with the entire local team graph
/// that credential opened.
///
/// Discovery is the expensive part of selecting a federated administrator: it
/// walks and authenticates every membership chain the credential can reach.
/// Selecting an administrator for a second team of the same profile is a pure
/// computation over this snapshot, so it is kept behind an `Rc` and shared by
/// every actor derived from it rather than rediscovered per team.
///
/// The contents are immutable for the lifetime of the `Rc`. They are a read of
/// one host at one instant, so reuse is only sound while nothing this cascade
/// ran can have moved that host's chains; see [`FederationCascade::commits`].
struct AuthenticatedFederationGraph<'a, 'device> {
    credential: FederatedActorCredential<'a, 'device>,
    user: foks_client::AuthenticatedUserOutcome,
    teams: Vec<foks_client::AuthenticatedTeamOutcome>,
    child_first: Vec<EntityId>,
    edges: Vec<(EntityId, EntityId)>,
}

impl AuthenticatedFederationGraph<'_, '_> {
    fn index_of(&self, team: &EntityId) -> Option<usize> {
        self.teams
            .iter()
            .position(|candidate| candidate.verified.team() == team)
    }
}

/// An administrator for one federated local team, together with the
/// authenticated membership graph it was selected from. Keeping the graph
/// here rather than rediscovering it per binding matters: a cascade visits
/// the same profile several times per convergence pass, and a redundant walk
/// is a full round of chain loads against that host.
///
/// `target` and `actor_team` are positions in `graph.teams`. They are checked
/// against that vector when the actor is built and the vector is immutable
/// behind the `Rc`, so both stay in range for the actor's whole lifetime.
#[derive(Clone)]
struct LocalFederatedActor<'a, 'device> {
    graph: std::rc::Rc<AuthenticatedFederationGraph<'a, 'device>>,
    target: usize,
    actor_team: Option<usize>,
}

type FederatedGraphRecipients<'a, 'device> = (
    LocalFederatedActor<'a, 'device>,
    std::collections::BTreeMap<
        Vec<u8>,
        std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>,
    >,
    std::collections::BTreeSet<Vec<u8>>,
);

impl<'a, 'device> LocalFederatedActor<'a, 'device> {
    /// Binds an already-authenticated graph to one target team and one acting
    /// party. Returns `None` if either is outside the graph, which is the
    /// caller's signal that this graph cannot serve the requested team.
    fn bind(
        graph: &std::rc::Rc<AuthenticatedFederationGraph<'a, 'device>>,
        target: &EntityId,
        actor_team: Option<&EntityId>,
    ) -> Option<Self> {
        let target = graph.index_of(target)?;
        let actor_team = match actor_team {
            Some(team) => Some(graph.index_of(team)?),
            None => None,
        };
        Some(Self {
            graph: std::rc::Rc::clone(graph),
            target,
            actor_team,
        })
    }

    fn credential(&self) -> &FederatedActorCredential<'a, 'device> {
        &self.graph.credential
    }

    fn user(&self) -> &foks_client::AuthenticatedUserOutcome {
        &self.graph.user
    }

    fn target(&self) -> &foks_client::AuthenticatedTeamOutcome {
        self.graph
            .teams
            .get(self.target)
            .expect("federated actor target index is checked when the actor is built")
    }

    fn actor_team(&self) -> Option<&foks_client::AuthenticatedTeamOutcome> {
        self.actor_team.map(|index| {
            self.graph
                .teams
                .get(index)
                .expect("federated actor team index is checked when the actor is built")
        })
    }

    fn child_first(&self) -> &[EntityId] {
        &self.graph.child_first
    }

    fn edges(&self) -> &[(EntityId, EntityId)] {
        &self.graph.edges
    }

    fn team(&self, team: &EntityId) -> Option<&foks_client::AuthenticatedTeamOutcome> {
        self.graph
            .index_of(team)
            .and_then(|index| self.graph.teams.get(index))
    }
}

/// Per-cascade bookkeeping for one top-level federated security refresh.
///
/// A cascade re-reads every hop on each convergence pass, so without this a
/// three-profile graph re-runs the same security responders and the same
/// chain loads often enough to trip a host's request rate limit.
#[derive(Default)]
struct FederationCascade {
    /// `(profile, team alias)` hops currently being resolved on this thread.
    visited: std::collections::BTreeSet<(String, String)>,
    /// Software `(profile, uid)` whose user and team chains this cascade has
    /// already brought current.
    software_refreshed: std::collections::BTreeSet<(String, Vec<u8>)>,
    /// Hardware `(profile, alias)` whose unlocked responders have already run.
    yubi_refreshed: std::collections::BTreeSet<(String, String)>,
    /// How many times this cascade has run something on each profile that can
    /// move a chain, rotate a key, or write a caller-durable team member-key refresh journal.
    ///
    /// An authenticated graph is a read of one host at one instant. Reusing
    /// one in place of a fresh selection is sound exactly while this counter
    /// is unchanged for the profile the graph was read from: every commit in
    /// this module increments it before the next selection can observe it, so
    /// a selection that follows a commit on the same profile always reloads.
    /// See [`CheckedProfileSession::record_federation_commit`].
    commits: std::collections::BTreeMap<String, u64>,
    /// Sessions already opened for a remote hop, one per remote profile.
    ///
    /// A session owns its connection pool, so reopening one per binding and
    /// per convergence pass pays a fresh TLS handshake for every remote call.
    /// Every session in a cascade carries the same operation controls, which
    /// [`ProfileSession::related_profile`] copies from the caller, so the
    /// profile name identifies the session completely.
    sessions: std::collections::BTreeMap<String, std::rc::Rc<ProfileSession>>,
    /// `(profile, uid)` whose ordinary user and team responders this cascade
    /// skipped because the profile's own scheduler had already run them within
    /// their interval. A recipient that then proves stale un-skips them; see
    /// [`CheckedProfileSession::load_federated_team_recipients`].
    skipped_responders: std::collections::BTreeSet<(String, Vec<u8>)>,
}

impl FederationCascade {
    /// The number of commits recorded for `profile` so far. A caller snapshots
    /// this beside a graph it intends to reuse and compares before each reuse.
    fn commits(&self, profile: &str) -> u64 {
        self.commits.get(profile).copied().unwrap_or_default()
    }

    fn record_commit(&mut self, profile: &str) {
        *self.commits.entry(profile.to_owned()).or_default() += 1;
    }

    fn rooted_at(profile: &str, team_alias: &str) -> Self {
        Self {
            visited: std::collections::BTreeSet::from([(
                profile.to_owned(),
                team_alias.to_owned(),
            )]),
            ..Self::default()
        }
    }
}

fn federated_expulsion_parties<'a>(
    context: &'a super::team::LocalTeamContext,
    target_team: &EntityId,
    target_host: &EntityId,
    supplied: &'a std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>,
) -> Result<(
    &'a foks_verify::VerifiedTeamMemberState,
    Vec<foks_client::VerifiedMemberParty<'a>>,
)> {
    let mut targets = context.team.verified.members().iter().filter(|member| {
        member.party == *target_team && member.scoped_host.as_ref() == Some(target_host)
    });
    let target = targets.next().ok_or(foks_client::Error::TeamRequest(
        "exact federated team is not in the authenticated roster",
    ))?;
    if targets.next().is_some() {
        return Err(
            foks_client::Error::TeamBinding("exact federated team selector is ambiguous").into(),
        );
    }
    let mut remaining = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for member in context.team.verified.members() {
        if (member.party == context.account.credential.uid && member.scoped_host.is_none())
            || (member.party == *target_team && member.scoped_host.as_ref() == Some(target_host))
        {
            continue;
        }
        let key = (
            member.party.as_bytes().to_vec(),
            member
                .scoped_host
                .as_ref()
                .map(|host| host.as_bytes().to_vec()),
        );
        if !seen.insert((key.clone(), member.source_role)) {
            return Err(foks_client::Error::TeamBinding(
                "authenticated remaining roster party is ambiguous",
            )
            .into());
        }
        if member.scoped_host.is_none() && member.party.entity_type() == foks_proto::ENTITY_USER {
            remaining.push(foks_client::VerifiedMemberParty::User(
                context.users.get(member.party.as_bytes()).ok_or(
                    foks_client::Error::TeamBinding("remaining local user state is unavailable"),
                )?,
            ));
        } else {
            remaining.push(
                supplied
                    .get(&key)
                    .ok_or(foks_client::Error::TeamBinding(
                        "remaining federated roster party lacks an authenticated recipient",
                    ))?
                    .verified(),
            );
        }
    }
    Ok((target, remaining))
}

fn federated_roster_matches(
    team: &foks_verify::VerifiedTeamState,
    parties: &std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>,
) -> bool {
    parties.iter().all(|((party_id, host_id), party)| {
        let matching = team
            .members()
            .iter()
            .filter(|member| {
                member.party.as_bytes() == party_id
                    && member.scoped_host.as_ref().map(EntityId::as_bytes) == host_id.as_deref()
            })
            .collect::<Vec<_>>();
        let [member] = matching.as_slice() else {
            return false;
        };
        party.shared_key(member.source_role).is_some_and(|key| {
            key.generation == member.generation
                && key.verify_key == member.verify_key
                && foks_crypto::hepk_fingerprint(&key.hepk).ok() == Some(member.hepk_fingerprint)
        })
    })
}

/// A remote team projection that failed because the remote team's own
/// federated roster is behind. Only the remote profile can fix that, so this
/// is the signal to run its responder before retrying rather than to fail the
/// whole cascade.
fn federated_recipient_requires_nested_refresh(error: &Error) -> bool {
    matches!(
        error,
        Error::Client(foks_client::Error::TeamBinding(
            "team recipient roster key is not current"
                | "team recipient has a stale direct or descendant key"
        ))
    )
}

/// Chooses the acting party for one federated team from a graph that has
/// already been authenticated, performing no network read of any kind.
///
/// This is the whole of the selection decision: the candidate walk in
/// [`CheckedProfileSession::select_local_federated_admin`] exists only to
/// produce a graph to run this against. Keeping the decision here is what lets
/// a caller that already holds a graph select a second team out of it.
///
/// `Ok(Err(reason))` means this graph cannot serve the team and another
/// candidate's graph may still do so; `reason` is the text the candidate walk
/// reports. `Err` is a hard failure no other candidate can repair.
fn federated_actor_in_graph<'a, 'device>(
    graph: &std::rc::Rc<AuthenticatedFederationGraph<'a, 'device>>,
    team_id: &EntityId,
    pending: Option<&super::team::StoredTeamRekey>,
) -> Result<std::result::Result<LocalFederatedActor<'a, 'device>, String>> {
    let credential = &graph.credential;
    let Some(target) = graph
        .teams
        .iter()
        .find(|candidate| candidate.verified.team() == team_id)
    else {
        return Ok(Err(format!(
            "{} cannot reach the federated target through its membership graph",
            hex(credential.uid().as_bytes())
        )));
    };
    let team = |party: &EntityId| {
        graph
            .index_of(party)
            .and_then(|index| graph.teams.get(index))
    };
    let direct_admin = target.verified.members().iter().any(|member| {
        member.party == *credential.uid()
            && member.scoped_host.is_none()
            && matches!(
                member.role.kind(),
                foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
            )
    });
    let selected_actor = if let Some(pending) = pending {
        if pending.transport_uid != credential.uid().as_bytes() {
            return Ok(Err(format!(
                "{} is not the caller-durable team member-key refresh transport",
                hex(credential.uid().as_bytes())
            )));
        }
        let recorded = EntityId::from_bytes(pending.actor_uid.clone())?;
        let recorded_team_is_current = team(&recorded)
            .is_some_and(|actor| local_team_can_observe_team_rekey(&target.verified, actor));
        if (recorded == *credential.uid() && direct_admin) || recorded_team_is_current {
            recorded
        } else if target.verified.chain_seqno() >= pending.expected_seqno && direct_admin {
            credential.uid().clone()
        } else if target.verified.chain_seqno() >= pending.expected_seqno {
            let Some(member) = target
                .verified
                .members()
                .iter()
                .filter(|member| {
                    member.scoped_host.is_none()
                        && matches!(
                            member.party.entity_type(),
                            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
                        )
                        && matches!(
                            member.role.kind(),
                            foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
                        )
                        && team(&member.party)
                            .is_some_and(|actor| actor.holds_roster_private_key(member))
                })
                .max_by_key(|member| (member.role, member.source_role))
            else {
                return Ok(Err(format!(
                    "{} cannot recover the visible caller-durable team member-key refresh actor",
                    hex(credential.uid().as_bytes())
                )));
            };
            member.party.clone()
        } else {
            return Ok(Err(format!(
                "{} cannot recover the caller-durable team member-key refresh actor",
                hex(credential.uid().as_bytes())
            )));
        }
    } else if direct_admin {
        credential.uid().clone()
    } else if let Some(member) = target
        .verified
        .members()
        .iter()
        .filter(|member| {
            member.scoped_host.is_none()
                && matches!(
                    member.party.entity_type(),
                    foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
                )
                && matches!(
                    member.role.kind(),
                    foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
                )
                && team(&member.party).is_some_and(|actor| actor.holds_roster_private_key(member))
        })
        .max_by_key(|member| (member.role, member.source_role))
    {
        member.party.clone()
    } else {
        return Ok(Err(format!(
            "{} has no direct or local-team administrator path",
            hex(credential.uid().as_bytes())
        )));
    };
    let actor_team = (selected_actor != *credential.uid()).then_some(&selected_actor);
    LocalFederatedActor::bind(graph, team_id, actor_team)
        .map(Ok)
        .ok_or(Error::InvalidAccount(
            "membership graph lost the federated local-team actor",
        ))
}

fn connected_local_team_component(
    root: &EntityId,
    edges: &[(EntityId, EntityId)],
) -> std::collections::BTreeSet<Vec<u8>> {
    let mut connected = std::collections::BTreeSet::from([root.as_bytes().to_vec()]);
    loop {
        let mut changed = false;
        for (child, parent) in edges {
            if connected.contains(child.as_bytes()) || connected.contains(parent.as_bytes()) {
                changed |= connected.insert(child.as_bytes().to_vec());
                changed |= connected.insert(parent.as_bytes().to_vec());
            }
        }
        if !changed {
            return connected;
        }
    }
}

fn descendant_team_component(
    root: &EntityId,
    edges: &[(EntityId, EntityId)],
) -> std::collections::BTreeSet<Vec<u8>> {
    let mut descendants = std::collections::BTreeSet::from([root.as_bytes().to_vec()]);
    loop {
        let mut changed = false;
        for (child, parent) in edges {
            if descendants.contains(parent.as_bytes()) {
                changed |= descendants.insert(child.as_bytes().to_vec());
            }
        }
        if !changed {
            return descendants;
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FederationDestinationRole {
    Member { visibility: i16 },
    Admin,
    Owner,
}

impl FederationDestinationRole {
    pub(crate) fn role(self) -> Role {
        match self {
            Self::Member { visibility } => Role::member(visibility),
            Self::Admin => Role::ADMIN,
            Self::Owner => Role::OWNER,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AddFederatedTeamMemberReport {
    pub operation_id_hex: String,
    pub local_profile: String,
    pub local_team_alias: String,
    pub remote_profile: String,
    pub remote_team_alias: String,
    pub remote_team_id_hex: String,
    pub destination: FederationDestinationRole,
    pub scheduled_job_id_hex: String,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FederatedMembershipSummary {
    pub local_team_alias: String,
    pub remote_profile: String,
    pub remote_team_alias: String,
    pub remote_host_id_hex: String,
    pub remote_team_id_hex: String,
    pub destination: FederationDestinationRole,
    pub operation_id_hex: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FederationExpulsionReport {
    pub local_profile: String,
    pub local_team_alias: String,
    pub remote_profile: String,
    pub remote_team_alias: String,
    pub remote_host_id_hex: String,
    pub remote_team_id_hex: String,
    pub operation_id_hex: String,
    pub team_chain_sequence: u64,
    pub active: bool,
}

impl CheckedProfileSession<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn add_federated_team_member(
        &self,
        remote: &CheckedProfileSession<'_>,
        local_team_alias: &str,
        remote_team_alias: &str,
        destination: FederationDestinationRole,
        local_vault: &mut AccountVault<'_>,
        remote_vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<AddFederatedTeamMemberReport> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Federation)?;
        remote.profile.require(Capability::Teams)?;
        remote.profile.require(Capability::Federation)?;
        if self.profile.name == remote.profile.name {
            return Err(Error::InvalidConfig(
                "federation requires distinct local and remote profiles",
            ));
        }
        if !matches!(destination, FederationDestinationRole::Member { .. }) {
            return Err(Error::InvalidAccount(
                "federated parties cannot be team administrators or owners",
            ));
        }
        let local_host = self.pinned_host()?;
        let remote_host = remote.pinned_host()?;
        if local_host.host_id() == remote_host.host_id() {
            return Err(Error::InvalidConfig(
                "federation profiles resolve to the same host",
            ));
        }

        let mut local_team = local_vault.team(local_team_alias)?;
        let remote_team = remote_vault.team(remote_team_alias)?;
        if local_team.kind != super::team::StoredTeamKind::Named
            || !local_team.active
            || !remote_team.active
        {
            return Err(Error::InvalidAccount(
                "federation requires active teams and a named local team",
            ));
        }
        let local_team_id = EntityId::from_bytes(local_team.team_id.clone())?;
        let remote_team_id = EntityId::from_bytes(remote_team.team_id.clone())?;
        let local_account = local_vault.account(&local_team.account_alias)?;
        let remote_account = remote_vault.account(&remote_team.account_alias)?;

        let binding = local_team.federated_members.iter().position(|member| {
            member.remote_host_id == remote_host.host_id().as_bytes()
                && member.remote_team_id == remote_team_id.as_bytes()
        });
        let removal_key = match binding {
            Some(index) => {
                let stored = &local_team.federated_members[index];
                if stored.remote_profile != remote.profile.name
                    || stored.remote_team_alias != remote_team_alias
                    || stored.destination != destination
                {
                    return Err(Error::InvalidAccount(
                        "federated membership binding cannot be repurposed",
                    ));
                }
                stored.removal_key
            }
            None => {
                let removal_key = random_array()?;
                local_team
                    .federated_members
                    .push(super::team::StoredFederatedMembership {
                        remote_profile: remote.profile.name.clone(),
                        remote_team_alias: remote_team_alias.to_owned(),
                        remote_host_id: remote_host.host_id().as_bytes().to_vec(),
                        remote_team_id: remote_team_id.as_bytes().to_vec(),
                        destination,
                        removal_key,
                        operation_id: None,
                        active: false,
                    });
                local_vault.put_team(&local_team)?;
                removal_key
            }
        };

        let scheduled_job_id = federation_job_id(
            local_host.host_id(),
            &local_team_id,
            remote_host.host_id(),
            &remote_team_id,
            &removal_key,
        )?;
        let mut local_mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut remote_mutations = EncryptedFileMutationStore::open(
            &remote.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let removal_key = SecretSeed::new(removal_key);
        let request = foks_client::AddFederatedTeamMemberRequest {
            remote_host: &remote_host,
            remote_credential: &remote_account.credential,
            remote_team: &remote_team_id,
            local_host: &local_host,
            local_credential: &local_account.credential,
            local_team: &local_team_id,
            destination_role: destination.role(),
            removal_key: &removal_key,
        };
        // Index ranges are authenticated chain state, not application hints.
        // Narrow the child before raising the parent, and do not grant/create
        // the scoped membership until both current heads prove disjoint.
        self.client.allocate_federated_team_index_ranges(
            &request,
            &mut remote_mutations,
            &mut local_mutations,
        )?;
        let outcome = self
            .client
            .add_remote_team_member(&request, &mut local_mutations)?;

        let stored = local_team
            .federated_members
            .iter_mut()
            .find(|member| {
                member.remote_host_id == remote_host.host_id().as_bytes()
                    && member.remote_team_id == remote_team_id.as_bytes()
            })
            .ok_or(Error::InvalidAccount(
                "federated membership disappeared from protected storage",
            ))?;
        stored.operation_id = Some(outcome.operation_id);
        stored.active = true;
        local_vault.put_team(&local_team)?;

        let now = now_microseconds()?;
        FoksScheduler::new(&self.paths.hard_database, SchedulerConfig::default())?.register(
            ScheduledJobRegistration {
                job_id: scheduled_job_id,
                kind: ScheduledJobKind::FederationRefresh,
                host_id: local_host.host_id().as_bytes().to_vec(),
                // The public job row identifies the wake-up only. Profile and
                // team aliases remain authoritative in the encrypted record.
                scope_id: Vec::new(),
                interval_micros: FEDERATION_REFRESH_INTERVAL_MICROS,
                first_run_at: now
                    .checked_add(FEDERATION_FIRST_RETRY_MICROS)
                    .ok_or(Error::InvalidConfig("federation refresh time overflow"))?,
                registered_at: now,
            },
        )?;

        Ok(AddFederatedTeamMemberReport {
            operation_id_hex: hex(&outcome.operation_id),
            local_profile: self.profile.name.clone(),
            local_team_alias: local_team_alias.to_owned(),
            remote_profile: remote.profile.name.clone(),
            remote_team_alias: remote_team_alias.to_owned(),
            remote_team_id_hex: hex(remote_team_id.as_bytes()),
            destination,
            scheduled_job_id_hex: hex(&scheduled_job_id),
            active: true,
        })
    }

    pub fn list_federated_memberships(
        &self,
        local_team_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<FederatedMembershipSummary>> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Federation)?;
        let team = vault.team(local_team_alias)?;
        Ok(team
            .federated_members
            .iter()
            .map(|member| FederatedMembershipSummary {
                local_team_alias: local_team_alias.to_owned(),
                remote_profile: member.remote_profile.clone(),
                remote_team_alias: member.remote_team_alias.clone(),
                remote_host_id_hex: hex(&member.remote_host_id),
                remote_team_id_hex: hex(&member.remote_team_id),
                destination: member.destination,
                operation_id_hex: member.operation_id.map(|operation| hex(&operation)),
                active: member.active,
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn expel_federated_team(
        &self,
        local_team_alias: &str,
        remote_host_id_hex: &str,
        remote_team_id_hex: &str,
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<FederationExpulsionReport> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Federation)?;
        let _scheduler_lock = super::runtime::ProfileLock::scheduler(&self.paths)?;
        let requested_host =
            entity_id_from_hex(remote_host_id_hex)?.require_type(foks_proto::ENTITY_HOST)?;
        let requested_team = entity_id_from_hex(remote_team_id_hex)?;
        if !matches!(
            requested_team.entity_type(),
            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
        ) {
            return Err(Error::InvalidAccount(
                "federation expulsion target is not a team",
            ));
        }
        let stored_team = local_vault.team(local_team_alias)?;
        if stored_team.kind != super::team::StoredTeamKind::Named || !stored_team.active {
            return Err(Error::InvalidAccount(
                "federation expulsion requires an active named local team",
            ));
        }
        if local_vault.team_member_edit(local_team_alias)?.is_some()
            || local_vault.team_rekey(local_team_alias)?.is_some()
        {
            return Err(Error::InvalidAccount(
                "team has another pending membership mutation; resume it first",
            ));
        }
        let existing_intent = local_vault.federation_expulsion(local_team_alias)?;
        if existing_intent.as_ref().is_some_and(|intent| {
            intent.remote_host_id != requested_host.as_bytes()
                || intent.remote_team_id != requested_team.as_bytes()
        }) {
            return Err(Error::InvalidAccount(
                "another exact federation expulsion is already pending for this team",
            ));
        }
        let mut context = self.load_local_team_context(local_team_alias, local_vault)?;
        // A committed expulsion is reconciled entirely from local evidence.
        // Before submission, only survivors need authenticated recipient keys:
        // do not require keys from the expelled host.
        let parties = if existing_intent
            .as_ref()
            .is_some_and(|intent| context.team.verified.chain_seqno() >= intent.expected_seqno)
        {
            std::collections::BTreeMap::new()
        } else {
            let bindings = stored_team
                .federated_members
                .iter()
                .filter(|member| {
                    member.active
                        && (member.remote_host_id != requested_host.as_bytes()
                            || member.remote_team_id != requested_team.as_bytes())
                })
                .map(|member| ScheduledFederationBinding {
                    local_team_alias: local_team_alias.to_owned(),
                    remote_profile: member.remote_profile.clone(),
                    remote_team_alias: member.remote_team_alias.clone(),
                    destination: member.destination,
                })
                .collect::<Vec<_>>();
            if bindings.is_empty() {
                std::collections::BTreeMap::new()
            } else {
                let actor =
                    self.select_local_federated_admin(local_team_alias, &[], local_vault)?;
                let parties = self.load_federated_team_recipients(
                    &bindings,
                    &actor,
                    &[],
                    local_vault,
                    registry,
                    credentials,
                    master_key,
                    &mut FederationCascade::rooted_at(&self.profile.name, local_team_alias),
                )?;
                context = self.load_local_team_context(local_team_alias, local_vault)?;
                parties
            }
        };
        if context.team_id.as_bytes() != stored_team.team_id
            || context.host.host_id() == &requested_host
        {
            return Err(foks_client::Error::TeamBinding(
                "federation expulsion local team or remote host binding changed",
            )
            .into());
        }
        let actor_members = context
            .team
            .verified
            .members()
            .iter()
            .filter(|member| {
                member.party == context.account.credential.uid
                    && member.scoped_host.is_none()
                    && matches!(
                        member.role.kind(),
                        foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
                    )
            })
            .collect::<Vec<_>>();
        let [actor_member] = actor_members.as_slice() else {
            return Err(foks_client::Error::TeamRequest(
                "federated teams can be expelled only by one exact authenticated admin or owner",
            )
            .into());
        };

        let mut protected = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let pending = match existing_intent {
            Some(pending) => pending,
            None => {
                let (target, remaining) = federated_expulsion_parties(
                    &context,
                    &requested_team,
                    &requested_host,
                    &parties,
                )?;
                let binding = stored_team
                    .federated_members
                    .iter()
                    .filter(|member| {
                        member.active
                            && member.remote_host_id == requested_host.as_bytes()
                            && member.remote_team_id == requested_team.as_bytes()
                    })
                    .collect::<Vec<_>>();
                let [binding] = binding.as_slice() else {
                    return Err(Error::InvalidAccount(
                        "exact active federation binding is missing or ambiguous",
                    ));
                };
                if binding.destination.role() != target.role {
                    return Err(foks_client::Error::TeamBinding(
                        "protected federation role differs from the authenticated roster",
                    )
                    .into());
                }
                let removal_key = SecretSeed::new(binding.removal_key);
                let selector = foks_client::TeamMemberSelector {
                    party: &requested_team,
                    host: Some(&requested_host),
                    source_role: target.source_role,
                };
                let roles = self.client.team_member_rotation_roles(
                    &context.team,
                    selector,
                    Role::NONE,
                    None,
                )?;
                let seeds = roles
                    .iter()
                    .map(|_| random_array().map(SecretSeed::new))
                    .collect::<Result<Vec<_>>>()?;
                let rotations = roles
                    .iter()
                    .zip(&seeds)
                    .map(|(role, seed)| foks_client::TeamPtkRotationSeed { role: *role, seed })
                    .collect::<Vec<_>>();
                let request = foks_client::RetainedTeamMemberRemovalRequest {
                    target: selector,
                    removal_key: &removal_key,
                    rotations: &rotations,
                    remaining_parties: &remaining,
                };
                let operation_id = self.client.retained_team_member_removal_operation_id(
                    &context.account.credential.uid,
                    &context.team_id,
                    &context.team,
                    &request,
                )?;
                let pending = super::team::StoredFederatedExpulsion {
                    version: CREDENTIAL_VERSION,
                    local_team_alias: local_team_alias.to_owned(),
                    local_team_id: context.team_id.as_bytes().to_vec(),
                    local_host_id: context.host.host_id().as_bytes().to_vec(),
                    remote_profile: binding.remote_profile.clone(),
                    remote_team_alias: binding.remote_team_alias.clone(),
                    remote_team_id: requested_team.as_bytes().to_vec(),
                    remote_host_id: requested_host.as_bytes().to_vec(),
                    source_role: super::team::StoredTeamRole::from_role(target.source_role),
                    destination_role: super::team::StoredTeamRole::from_role(target.role),
                    removal_key_commitment: foks_crypto::team_removal_key_commitment(&removal_key)?,
                    actor_uid: context.account.credential.uid.as_bytes().to_vec(),
                    actor_device_id: foks_crypto::derive_device_public(
                        &context.account.credential.seed,
                    )?
                    .id
                    .as_bytes()
                    .to_vec(),
                    actor_source_role: super::team::StoredTeamRole::from_role(
                        actor_member.source_role,
                    ),
                    actor_generation: actor_member.generation,
                    expected_seqno: context
                        .team
                        .verified
                        .chain_seqno()
                        .checked_add(1)
                        .ok_or(foks_client::Error::TeamRequest("team sequence overflow"))?,
                    operation_id,
                    scheduler_job_id: federation_job_id(
                        context.host.host_id(),
                        &context.team_id,
                        &requested_host,
                        &requested_team,
                        &binding.removal_key,
                    )?,
                    rotations: roles
                        .iter()
                        .zip(&seeds)
                        .map(|(role, seed)| super::team::StoredTeamPtkRotation {
                            role: super::team::StoredTeamRole::from_role(*role),
                            seed: *seed.as_bytes(),
                        })
                        .collect(),
                };
                local_vault.put_federation_expulsion(&pending)?;
                pending
            }
        };
        if pending.local_team_id != context.team_id.as_bytes()
            || pending.local_host_id != context.host.host_id().as_bytes()
            || pending.actor_uid != context.account.credential.uid.as_bytes()
            || pending.actor_device_id
                != context.account.credential.public_material()?.id.as_bytes()
            || pending.actor_source_role.role()? != actor_member.source_role
            || pending.actor_generation != actor_member.generation
            || pending.remote_team_id != requested_team.as_bytes()
            || pending.remote_host_id != requested_host.as_bytes()
        {
            return Err(foks_client::Error::OperationBinding(
                "pending federation expulsion identity changed",
            )
            .into());
        }
        let seeds = pending
            .rotations
            .iter()
            .map(|rotation| SecretSeed::new(rotation.seed))
            .collect::<Vec<_>>();
        let rotations = pending
            .rotations
            .iter()
            .zip(&seeds)
            .map(|(rotation, seed)| {
                Ok(foks_client::TeamPtkRotationSeed {
                    role: rotation.role.role()?,
                    seed,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let hard_recorded = HardStateStore::open(&self.paths.hard_database)?
            .team_mutation(&pending.operation_id)?
            .is_some();
        if context.team.verified.chain_seqno() >= pending.expected_seqno && !hard_recorded {
            local_vault.remove_federation_expulsion(local_team_alias)?;
            return Err(foks_client::Error::OperationBinding(
                "another transition consumed the pending federation expulsion sequence",
            )
            .into());
        }
        let reserved_sequence_is_observable =
            context.team.verified.chain_seqno() >= pending.expected_seqno;
        let attempt = if reserved_sequence_is_observable {
            self.client.finish_recorded_team_member_change(
                &context.host,
                &context.account.credential,
                foks_client::TeamMutationRecovery {
                    team: &context.team_id,
                    expected_seqno: pending.expected_seqno,
                    expected_operation_id: &pending.operation_id,
                },
                pending.removal_key_commitment,
                &rotations,
                &mut protected,
            )
        } else {
            let (target, remaining) =
                federated_expulsion_parties(&context, &requested_team, &requested_host, &parties)?;
            if target.source_role != pending.source_role.role()?
                || target.role != pending.destination_role.role()?
                || target.removal_key_commitment != Some(pending.removal_key_commitment)
            {
                return Err(foks_client::Error::OperationBinding(
                    "pending federation expulsion no longer matches the authenticated roster",
                )
                .into());
            }
            let binding = stored_team
                .federated_members
                .iter()
                .find(|member| {
                    member.active
                        && member.remote_host_id == requested_host.as_bytes()
                        && member.remote_team_id == requested_team.as_bytes()
                        && member.remote_profile == pending.remote_profile
                        && member.remote_team_alias == pending.remote_team_alias
                })
                .ok_or(Error::InvalidAccount(
                    "pending federation expulsion lost its protected removal key",
                ))?;
            let removal_key = SecretSeed::new(binding.removal_key);
            if foks_crypto::team_removal_key_commitment(&removal_key)?
                != pending.removal_key_commitment
            {
                return Err(foks_client::Error::OperationBinding(
                    "pending federation expulsion removal key changed",
                )
                .into());
            }
            let request = foks_client::RetainedTeamMemberRemovalRequest {
                target: foks_client::TeamMemberSelector {
                    party: &requested_team,
                    host: Some(&requested_host),
                    source_role: target.source_role,
                },
                removal_key: &removal_key,
                rotations: &rotations,
                remaining_parties: &remaining,
            };
            if hard_recorded {
                self.client.resume_retained_team_member_removal(
                    &context.host,
                    &context.account.credential,
                    foks_client::TeamMutationRecovery {
                        team: &context.team_id,
                        expected_seqno: pending.expected_seqno,
                        expected_operation_id: &pending.operation_id,
                    },
                    &request,
                    &mut protected,
                )
            } else {
                self.client.remove_retained_team_member_and_rotate_ptks(
                    &context.host,
                    &context.account.credential,
                    &context.team_id,
                    &request,
                    &mut protected,
                )
            }
        };
        let result = match attempt {
            Ok(result) => result,
            Err(error @ foks_client::Error::OperationBinding(_))
                if reserved_sequence_is_observable =>
            {
                self.client.supersede_recorded_team_member_change(
                    &context.host,
                    &context.account.credential,
                    &context.team_id,
                    pending.expected_seqno,
                    &pending.operation_id,
                    &mut protected,
                )?;
                local_vault.remove_federation_expulsion(local_team_alias)?;
                return Err(error.into());
            }
            Err(error) => return Err(error.into()),
        };
        if result
            .authenticated
            .verified
            .members()
            .iter()
            .any(|member| {
                member.party == requested_team
                    && member.scoped_host.as_ref() == Some(&requested_host)
            })
        {
            return Err(foks_client::Error::OperationBinding(
                "authenticated expulsion left the exact federated target in the roster",
            )
            .into());
        }
        // The durable intent owns the scheduler identity after the chain edit.
        // Unregister first: if protected binding cleanup then fails, a retry can
        // idempotently unregister the same persisted ID instead of leaving a
        // refresh job after the removal key has disappeared.
        FoksScheduler::new(&self.paths.hard_database, SchedulerConfig::default())?
            .unregister(&pending.scheduler_job_id)?;
        let mut finished_team = local_vault.team(local_team_alias)?;
        finished_team.federated_members.retain(|member| {
            member.remote_host_id != requested_host.as_bytes()
                || member.remote_team_id != requested_team.as_bytes()
        });
        local_vault.put_team(&finished_team)?;
        local_vault.remove_federation_expulsion(local_team_alias)?;
        Ok(FederationExpulsionReport {
            local_profile: self.profile.name.clone(),
            local_team_alias: local_team_alias.to_owned(),
            remote_profile: pending.remote_profile.clone(),
            remote_team_alias: pending.remote_team_alias.clone(),
            remote_host_id_hex: hex(requested_host.as_bytes()),
            remote_team_id_hex: hex(requested_team.as_bytes()),
            operation_id_hex: hex(&pending.operation_id),
            team_chain_sequence: result.authenticated.verified.chain_seqno(),
            active: false,
        })
    }

    fn refresh_federated_team(
        &self,
        remote: &CheckedProfileSession<'_>,
        binding: &ScheduledFederationBinding,
        local_actor: &LocalFederatedActor<'_, '_>,
        remote_actor: &LocalFederatedActor<'_, '_>,
        local_vault: &mut AccountVault<'_>,
        remote_vault: &mut AccountVault<'_>,
    ) -> Result<foks_client::RemoteTeamOutcome> {
        self.profile.require(Capability::Federation)?;
        self.profile.require(Capability::Teams)?;
        remote.profile.require(Capability::Federation)?;
        remote.profile.require(Capability::Teams)?;
        let local_host = self.pinned_host()?;
        let remote_host = remote.pinned_host()?;
        let local_team = local_vault.team(&binding.local_team_alias)?;
        let remote_team = remote_vault.team(&binding.remote_team_alias)?;
        let local_team_id = EntityId::from_bytes(local_team.team_id.clone())?;
        let remote_team_id = EntityId::from_bytes(remote_team.team_id.clone())?;
        if remote_actor.target().verified.team() != &remote_team_id {
            return Err(Error::InvalidAccount(
                "remote federation actor loaded a different team",
            ));
        }
        let member = local_team
            .federated_members
            .iter()
            .find(|member| {
                member.remote_profile == remote.profile.name
                    && member.remote_team_alias == binding.remote_team_alias
                    && member.remote_host_id == remote_host.host_id().as_bytes()
                    && member.remote_team_id == remote_team_id.as_bytes()
            })
            .ok_or(Error::InvalidAccount(
                "scheduled federation membership disappeared",
            ))?;
        if !local_team.active
            || !remote_team.active
            || !member.active
            || member.operation_id.is_none()
            || member.destination != binding.destination
        {
            return Err(Error::InvalidAccount(
                "scheduled federation membership is not active",
            ));
        }
        let request = foks_client::FederatedTeamRefreshRequest {
            remote_host: &remote_host,
            remote_credential: remote_actor.credential().borrowed(),
            remote_team: &remote_team_id,
            local_host: &local_host,
            local_credential: local_actor.credential().borrowed(),
            local_team: &local_team_id,
        };
        self.client
            .refresh_federated_team_capability_with_actors(
                &request,
                &remote_actor.user().verified,
                remote_actor.actor_team(),
                &local_actor.user().verified,
                local_actor.actor_team(),
            )
            .map_err(Into::into)
    }

    /// Loads one recipient projection per binding, bringing each remote
    /// profile's own security state current first.
    ///
    /// `visited` tracks `(profile, team alias)` ancestor hops during recursive
    /// federation resolution. A binding that points back to an ancestor hop
    /// indicates a dependency cycle that cannot be resolved.
    #[allow(clippy::too_many_arguments)]
    fn load_federated_team_recipients(
        &self,
        bindings: &[ScheduledFederationBinding],
        local_actor: &LocalFederatedActor<'_, '_>,
        unlocked: &[UnlockedYubiActor<'_, '_>],
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
        cascade: &mut FederationCascade,
    ) -> Result<std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>> {
        let mut parties = std::collections::BTreeMap::new();
        for binding in bindings {
            let visit = (
                binding.remote_profile.clone(),
                binding.remote_team_alias.clone(),
            );
            if !cascade.visited.insert(visit.clone()) {
                return Err(Error::InvalidAccount(
                    "federation team graph contains a protected cycle",
                ));
            }
            let local_vault = &mut *local_vault;
            let cascade = &mut *cascade;
            let outcome = (|| {
                let remote_session =
                    self.federation_session(registry, &binding.remote_profile, cascade)?;
                // A nested profile can run its own user, team, and hardware
                // responders before the final bearer refresh. Gate that work
                // here, before any protected state or network operation is
                // touched, rather than relying on the later refresh call.
                remote_session.profile.require(Capability::Teams)?;
                remote_session.profile.require(Capability::Federation)?;
                let attempted =
                    credentials.try_with_checked_session(&remote_session, |remote| {
                        let mut remote_store = foks_keystore::EncryptedFileSecretStore::open(
                            &remote.paths.credential_store,
                            derive_vault_key(master_key),
                        )?;
                        let mut remote_vault = AccountVault::new(&mut remote_store);
                        let mut remote_actor = remote.select_local_federated_admin(
                            &binding.remote_team_alias,
                            unlocked,
                            &mut remote_vault,
                        )?;
                        // Bring the remote side's own security state current
                        // before reading its roster, exactly as the local side
                        // does, unless that profile's own scheduler already did.
                        if remote.run_federation_responders(
                            &remote_actor,
                            &mut remote_vault,
                            master_key,
                            cascade,
                            false,
                        )? {
                            // Only reselect when the responders could actually
                            // have moved the chain this actor was derived from.
                            remote_actor = remote.select_local_federated_admin(
                                &binding.remote_team_alias,
                                unlocked,
                                &mut remote_vault,
                            )?;
                        }
                        let mut recipient = self.load_federated_team_recipient(
                            remote,
                            binding,
                            local_actor,
                            &remote_actor,
                            unlocked,
                            local_vault,
                            &mut remote_vault,
                            registry,
                            credentials,
                            master_key,
                            cascade,
                        );
                        // A stale recipient is the evidence that a skipped
                        // responder sweep had work after all. Run it now and
                        // retry before deciding this is the remote profile's
                        // own federation to repair.
                        if recipient
                            .as_ref()
                            .is_err_and(federated_recipient_requires_nested_refresh)
                            && remote.unskip_federation_responders(
                                &remote_actor,
                                &mut remote_vault,
                                master_key,
                                cascade,
                            )?
                        {
                            remote_actor = remote.select_local_federated_admin(
                                &binding.remote_team_alias,
                                unlocked,
                                &mut remote_vault,
                            )?;
                            recipient = self.load_federated_team_recipient(
                                remote,
                                binding,
                                local_actor,
                                &remote_actor,
                                unlocked,
                                local_vault,
                                &mut remote_vault,
                                registry,
                                credentials,
                                master_key,
                                cascade,
                            );
                        }
                        // A stale roster key on the remote side is not this
                        // profile's to fix. Run the remote's own responder here,
                        // under the lock already held, and retry once.
                        if recipient
                            .as_ref()
                            .is_err_and(federated_recipient_requires_nested_refresh)
                            && remote_vault
                                .team(&binding.remote_team_alias)?
                                .federated_members
                                .iter()
                                .any(|member| member.active)
                        {
                            remote.refresh_federated_team_security_inner(
                                &binding.remote_team_alias,
                                unlocked,
                                &mut remote_vault,
                                registry,
                                credentials,
                                master_key,
                                cascade,
                            )?;
                            remote_actor = remote.select_local_federated_admin(
                                &binding.remote_team_alias,
                                unlocked,
                                &mut remote_vault,
                            )?;
                            recipient = self.load_federated_team_recipient(
                                remote,
                                binding,
                                local_actor,
                                &remote_actor,
                                unlocked,
                                local_vault,
                                &mut remote_vault,
                                registry,
                                credentials,
                                master_key,
                                cascade,
                            );
                        }
                        recipient
                    })?;
                attempted.ok_or(Error::InvalidConfig(
                    "remote federation profile is busy; retry later",
                ))
            })();
            cascade.visited.remove(&visit);
            let (key, party) = outcome?;
            parties.entry(key).or_insert(party);
        }
        Ok(parties)
    }

    /// Renews one binding's remote bearer and projects the remote team, plus
    /// the child teams it recursively depends on, into a recipient the local
    /// team member-key refresh can rekey against.
    #[allow(clippy::too_many_arguments)]
    fn load_federated_team_recipient(
        &self,
        remote: &CheckedProfileSession<'_>,
        binding: &ScheduledFederationBinding,
        local_actor: &LocalFederatedActor<'_, '_>,
        remote_actor: &LocalFederatedActor<'_, '_>,
        unlocked: &[UnlockedYubiActor<'_, '_>],
        local_vault: &mut AccountVault<'_>,
        remote_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
        cascade: &mut FederationCascade,
    ) -> Result<(TeamRefreshPartyKey, TeamRefreshParty)> {
        // `remote_actor` is current on entry: the caller reselects it after
        // anything it ran that could have moved the remote's chains. Taking
        // the snapshot before any call below keeps that true without relying
        // on what those calls do.
        let remote_commits = cascade.commits(&remote.profile.name);
        let public = self.refresh_federated_team(
            remote,
            binding,
            local_actor,
            remote_actor,
            local_vault,
            remote_vault,
        )?;
        let remote_host = remote.pinned_host()?;
        // A profile can administer many unrelated teams. Only the target and
        // the child teams it recursively depends on are recipients for this
        // federation binding; an unrelated team's unsupported remote member
        // must not abort the bound team's refresh.
        //
        // The graph comes from the actor rather than a fresh walk: a cascade
        // reaches this binding several times per convergence pass, and a
        // redundant discovery is a full round of chain loads against the
        // remote host each time.
        let relevant = descendant_team_component(public.verified.team(), remote_actor.edges());
        let mut recipients =
            std::collections::BTreeMap::<Vec<u8>, foks_client::VerifiedTeamRecipient>::new();
        for team_id in remote_actor.child_first() {
            let team_key = team_id.as_bytes().to_vec();
            if !relevant.contains(&team_key) {
                continue;
            }
            let team = remote_actor.team(team_id).ok_or(Error::InvalidAccount(
                "remote membership graph lost a team recipient",
            ))?;
            // The durable record for this remote team, needed to translate a
            // third-host roster member back into the binding that produced
            // it. Only looked up when such a member is actually present.
            let mut stored_alias = None;
            let mut direct = std::collections::BTreeMap::new();
            for member in team.verified.members() {
                let party_key = (
                    member.party.as_bytes().to_vec(),
                    member
                        .scoped_host
                        .as_ref()
                        .map(|host| host.as_bytes().to_vec()),
                );
                if direct.contains_key(&party_key) {
                    continue;
                }
                let party = if let Some(scoped_host) = member.scoped_host.as_ref() {
                    if stored_alias.is_none() {
                        stored_alias = remote_vault.team_aliases()?.into_iter().find_map(|alias| {
                            remote_vault
                                .team(&alias)
                                .ok()
                                .filter(|stored| stored.active && stored.team_id == team_key)
                                .map(|stored| (alias, stored))
                        });
                    }
                    let (nested_alias, stored) =
                        stored_alias.as_ref().ok_or(Error::InvalidAccount(
                            "remote protected team has no durable local binding record",
                        ))?;
                    let nested = stored
                        .federated_members
                        .iter()
                        .find(|nested| {
                            nested.active
                                && nested.remote_host_id == scoped_host.as_bytes()
                                && nested.remote_team_id == member.party.as_bytes()
                        })
                        .ok_or(Error::InvalidAccount(
                            "remote protected roster member has no matching federation binding",
                        ))?;
                    let nested_binding = ScheduledFederationBinding {
                        local_team_alias: nested_alias.clone(),
                        remote_profile: nested.remote_profile.clone(),
                        remote_team_alias: nested.remote_team_alias.clone(),
                        destination: nested.destination,
                    };
                    // The remote's graph is already authenticated and covers
                    // every team of that profile the credential can reach, so
                    // the descendant walk selects out of it instead of paying
                    // a discovery per scoped roster row.
                    let nested_actor = remote.select_federated_admin_reusing(
                        nested_alias,
                        remote_actor,
                        remote_commits,
                        unlocked,
                        remote_vault,
                        cascade,
                    )?;
                    // Resolve the next hop from the remote's own point of
                    // view: it is the local side of that binding.
                    let nested_parties = remote.load_federated_team_recipients(
                        std::slice::from_ref(&nested_binding),
                        &nested_actor,
                        unlocked,
                        remote_vault,
                        registry,
                        credentials,
                        master_key,
                        cascade,
                    )?;
                    nested_parties
                        .get(&party_key)
                        .cloned()
                        .ok_or(Error::InvalidAccount(
                            "nested federation binding returned a different protected party",
                        ))?
                } else {
                    match member.party.entity_type() {
                        foks_proto::ENTITY_USER => {
                            TeamRefreshParty::User(remote.load_team_refresh_user(
                                &remote_host,
                                remote_actor.credential().team_refresh(),
                                &member.party,
                                &team.view_token,
                            )?)
                        }
                        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM => {
                            TeamRefreshParty::Team(
                                recipients.get(member.party.as_bytes()).cloned().ok_or(
                                    Error::InvalidAccount(
                                        "remote membership graph is not child-first",
                                    ),
                                )?,
                            )
                        }
                        _ => {
                            return Err(Error::InvalidAccount(
                                "paired remote team has an unsupported roster party",
                            ))
                        }
                    }
                };
                direct.insert(party_key, party);
            }
            let witnesses = direct
                .values()
                .map(TeamRefreshParty::verified)
                .collect::<Vec<_>>();
            let recipient = if team.verified.team() == public.verified.team() {
                if team.verified.chain_tail_hash() != public.verified.chain_tail_hash()
                    || team.verified.tree_root() != public.verified.tree_root()
                {
                    return Err(Error::InvalidAccount(
                        "local and exported remote team projections disagree",
                    ));
                }
                public.verified_recipient(&witnesses)?
            } else {
                team.verified_recipient(&remote_host, &witnesses)?
            };
            recipients.insert(team_key, recipient);
        }
        let recipient =
            recipients
                .remove(public.verified.team().as_bytes())
                .ok_or(Error::InvalidAccount(
                    "paired remote team is outside the authenticated membership graph",
                ))?;
        let key = (
            recipient.verified().team().as_bytes().to_vec(),
            Some(recipient.verified().host().as_bytes().to_vec()),
        );
        Ok((key, TeamRefreshParty::Team(recipient)))
    }

    /// Picks an administrator for one federated local team from the software
    /// accounts in the vault plus any hardware credential the caller has
    /// already unlocked. Hardware is preferred only through the same
    /// caller-durable team member-key refresh preference software uses; authority is always
    /// re-derived from the authenticated membership graph.
    fn select_local_federated_admin<'a, 'device>(
        &self,
        local_team_alias: &str,
        unlocked: &'a [UnlockedYubiActor<'a, 'device>],
        local_vault: &mut AccountVault<'_>,
    ) -> Result<LocalFederatedActor<'a, 'device>> {
        let host = self.pinned_host()?;
        let team_id = self.federated_team_id(local_team_alias, local_vault)?;
        let pending = local_vault
            .team_rekey_for_team(team_id.as_bytes())?
            .map(|(_, pending)| pending);
        let mut candidates = local_vault
            .aliases()?
            .into_iter()
            .map(|alias| {
                local_vault
                    .account(&alias)
                    .map(|account| FederatedActorCredential::Software(account.credential))
            })
            .collect::<Result<Vec<_>>>()?;
        candidates.extend(
            unlocked
                .iter()
                .filter(|actor| actor.profile == self.profile.name)
                .map(|actor| FederatedActorCredential::Yubi {
                    alias: actor.alias,
                    credential: actor.credential,
                }),
        );
        // A profile that still holds locked hardware must defer rather than
        // fail: the same YubiKey may be the only remaining administrator.
        let locked_yubi = self.locked_yubi_aliases(unlocked, local_vault)?;
        candidates.sort_by_key(|candidate| {
            let device = candidate.borrowed().device_id().ok();
            !pending.as_ref().is_some_and(|pending| {
                pending.transport_uid == candidate.uid().as_bytes()
                    && device
                        .as_ref()
                        .is_some_and(|device| pending.actor_device_id == device.as_bytes())
            })
        });
        let mut errors = Vec::new();
        for credential in candidates {
            let actor = match self
                .client
                .authenticate_credential_and_pin(&host, credential.borrowed())
            {
                Ok(actor) => actor,
                Err(error) => {
                    // A rate-limited host rejects every credential the same
                    // way. Walking the rest of the candidate list only spends
                    // the budget further and buries the real reason.
                    if federation_client_rate_limited(&error) {
                        return Err(error.into());
                    }
                    errors.push(format!(
                        "{} authentication failed: {error}",
                        hex(credential.uid().as_bytes())
                    ));
                    continue;
                }
            };
            let graph = match self.client.discover_local_team_graph_with_credential(
                &host,
                credential.borrowed(),
                &actor.verified,
                &actor.puks,
            ) {
                Ok(graph) => graph,
                Err(error) => {
                    if federation_client_rate_limited(&error) {
                        return Err(error.into());
                    }
                    errors.push(format!(
                        "{} membership graph load failed: {error}",
                        hex(credential.uid().as_bytes())
                    ));
                    continue;
                }
            };
            count_federation_read(|counters| counters.graph_loads += 1);
            let graph = std::rc::Rc::new(AuthenticatedFederationGraph {
                credential,
                user: actor,
                teams: graph.teams,
                child_first: graph.child_first,
                edges: graph.edges,
            });
            match federated_actor_in_graph(&graph, &team_id, pending.as_ref())? {
                Ok(actor) => return Ok(actor),
                Err(reason) => {
                    errors.push(reason);
                    continue;
                }
            }
        }
        if !locked_yubi.is_empty() {
            return Err(Error::YubiUnlockRequired(format!(
                "federated team {local_team_alias} has no usable unlocked administrator; unlock Yubi credential(s) {} and rerun the federation security responder{}",
                locked_yubi.join(", "),
                if errors.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", errors.join("; "))
                }
            )));
        }
        Err(Error::BackgroundRefresh(if errors.is_empty() {
            "no software credential is available for federated team refresh".to_owned()
        } else {
            errors.join("; ")
        }))
    }

    /// Whether this profile's own scheduler has already brought the ordinary
    /// user and team security state current for `uid`, so running those
    /// responders again inside a federation cascade would repeat work the
    /// profile itself considers done.
    ///
    /// The job rows alone are not enough, because they only say when the
    /// profile last swept, not what has happened since. Two conditions that
    /// are current by construction are checked first:
    ///
    /// * A stale user shared key is exactly what the user responder repairs,
    ///   and the selection that produced `actor` authenticated the user a
    ///   moment ago, so the answer is read from that authentication.
    /// * A caller-durable team member-key refresh journal is work only the team responder
    ///   resumes. Nothing later in the refresh would surface it, so an
    ///   outstanding journal always runs the responders.
    ///
    /// Everything else the responders would repair shows up as a stale
    /// recipient when the roster is projected, and
    /// [`Self::unskip_federation_responders`] runs them then. The skip is
    /// therefore never the last word on whether the work was needed.
    fn federation_responders_are_current(
        &self,
        uid: &EntityId,
        actor: &LocalFederatedActor<'_, '_>,
        vault: &mut AccountVault<'_>,
    ) -> Result<bool> {
        if !actor.user().verified.stale_shared_key_roles().is_empty() {
            return Ok(false);
        }
        if !vault.team_rekey_aliases()?.is_empty() {
            return Ok(false);
        }
        let host = self.pinned_host()?;
        let now = now_microseconds()?;
        let store = HardStateStore::open(&self.paths.hard_database)?;
        for (type_id, kind) in [
            (USER_REFRESH_JOB_TYPE_ID, ScheduledJobKind::UserRefresh),
            (TEAM_REFRESH_JOB_TYPE_ID, ScheduledJobKind::TeamRefresh),
        ] {
            let Some(job) = store.scheduled_job(&refresh_job_id(type_id, host.host_id(), uid))?
            else {
                return Ok(false);
            };
            // A failing job never counts as current however recently it ran:
            // its backoff moves its next run into the future, which is the one
            // way a job row can look settled while nothing is being repaired.
            let settled = job.kind == kind
                && job.host_id == host.host_id().as_bytes()
                && job.scope_id == uid.as_bytes()
                && job.failure_count == 0
                && job.last_completed_at.is_some_and(|completed| {
                    completed <= now && now - completed < job.interval_micros
                });
            if !settled {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Brings this profile's own security state current before its roster is
    /// read, exactly as the side that started the refresh does for itself.
    ///
    /// Returns whether anything ran, which is the caller's signal that the
    /// actor it holds was derived from a chain that may have moved.
    ///
    /// The software sweep is unattended and PIN-free; a hardware actor runs
    /// the equivalent already-unlocked responders, which the software sweep
    /// cannot drive because it must never prompt.
    fn run_federation_responders(
        &self,
        actor: &LocalFederatedActor<'_, '_>,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
        cascade: &mut FederationCascade,
        force: bool,
    ) -> Result<bool> {
        match actor.credential() {
            FederatedActorCredential::Software(credential) => {
                let key = (
                    self.profile.name.clone(),
                    credential.uid.as_bytes().to_vec(),
                );
                if cascade.software_refreshed.contains(&key) {
                    return Ok(false);
                }
                if !force
                    && self.federation_responders_are_current(&credential.uid, actor, vault)?
                {
                    count_federation_read(|counters| counters.skipped_responders += 1);
                    cascade.skipped_responders.insert(key);
                    return Ok(false);
                }
                cascade.skipped_responders.remove(&key);
                cascade.software_refreshed.insert(key);
                count_federation_read(|counters| counters.ran_responders += 1);
                self.refresh_user_security(credential.uid.as_bytes(), vault, master_key)
                    .map_err(Error::BackgroundRefresh)?;
                self.refresh_team_chains(credential.uid.as_bytes(), vault, master_key)
                    .map_err(Error::BackgroundRefresh)?;
                self.record_federation_commit(cascade);
                Ok(true)
            }
            FederatedActorCredential::Yubi { alias, credential } => {
                let key = (self.profile.name.clone(), (*alias).to_owned());
                if !cascade.yubi_refreshed.insert(key) {
                    return Ok(false);
                }
                count_federation_read(|counters| counters.ran_responders += 1);
                let host = self.pinned_host()?;
                let authenticated = self.client.authenticate_yubi_and_pin(&host, credential)?;
                self.run_unlocked_yubi_security_responders(
                    alias,
                    &host,
                    credential,
                    authenticated,
                    vault,
                    master_key,
                )?;
                self.record_federation_commit(cascade);
                Ok(true)
            }
        }
    }

    /// Runs a responder sweep this cascade skipped, after a roster projection
    /// proved the remote side was not current after all.
    ///
    /// A skip is always provisional. The only work the sweep does that can
    /// change this refresh's outcome is repairing a key some roster row still
    /// depends on, and that is precisely what makes a recipient projection
    /// fail, so the one error that follows a wrong skip is the one that undoes
    /// it. Returns whether the sweep ran, so the caller retries only then.
    fn unskip_federation_responders(
        &self,
        actor: &LocalFederatedActor<'_, '_>,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
        cascade: &mut FederationCascade,
    ) -> Result<bool> {
        let FederatedActorCredential::Software(credential) = actor.credential() else {
            return Ok(false);
        };
        let key = (
            self.profile.name.clone(),
            credential.uid.as_bytes().to_vec(),
        );
        if !cascade.skipped_responders.contains(&key) {
            return Ok(false);
        }
        count_federation_read(|counters| counters.unskipped_responders += 1);
        self.run_federation_responders(actor, vault, master_key, cascade, true)
    }

    fn federated_team_id(
        &self,
        local_team_alias: &str,
        local_vault: &mut AccountVault<'_>,
    ) -> Result<EntityId> {
        Ok(EntityId::from_bytes(
            local_vault.team(local_team_alias)?.team_id.clone(),
        )?)
    }

    /// Selects an administrator for `local_team_alias` out of a graph this
    /// cascade already authenticated, falling back to a full candidate walk
    /// when that graph cannot serve the team.
    ///
    /// Two conditions gate the reuse, and both are necessary:
    ///
    /// * Nothing this cascade ran may have moved the profile the graph was
    ///   read from since `held` was selected. `held_commits` is the caller's
    ///   snapshot of [`FederationCascade::commits`] taken when `held` was
    ///   known current, and every commit increments that counter before
    ///   control can return here, so a selection that follows a commit on the
    ///   same profile always reloads. This is what keeps the selection after
    ///   the local team member-key refresh commit a fresh cross-host read rather than a replay of
    ///   the pre-commit graph.
    /// * Reuse for a *different* team must not change which credential acts.
    ///   The candidate walk tries software before already-unlocked hardware,
    ///   so a hardware graph is reused only for the team it was selected for,
    ///   or on a profile that holds no software account at all. Reuse for the
    ///   same team cannot change the credential: the walk is deterministic in
    ///   the vault and the host state, and the host state is unmoved.
    fn select_federated_admin_reusing<'a, 'device>(
        &self,
        local_team_alias: &str,
        held: &LocalFederatedActor<'a, 'device>,
        held_commits: u64,
        unlocked: &'a [UnlockedYubiActor<'a, 'device>],
        local_vault: &mut AccountVault<'_>,
        cascade: &FederationCascade,
    ) -> Result<LocalFederatedActor<'a, 'device>> {
        if cascade.commits(&self.profile.name) == held_commits {
            let team_id = self.federated_team_id(local_team_alias, local_vault)?;
            let credential_is_unchanged = held.target().verified.team() == &team_id
                || matches!(held.credential(), FederatedActorCredential::Software(_))
                || local_vault.aliases()?.is_empty();
            if credential_is_unchanged {
                let pending = local_vault
                    .team_rekey_for_team(team_id.as_bytes())?
                    .map(|(_, pending)| pending);
                if let Ok(actor) =
                    federated_actor_in_graph(&held.graph, &team_id, pending.as_ref())?
                {
                    count_federation_read(|counters| counters.reused_selections += 1);
                    return Ok(actor);
                }
            }
        }
        self.select_local_federated_admin(local_team_alias, unlocked, local_vault)
    }

    /// Records that this profile's durable state may have moved, so no graph
    /// read before this point is reused for it again.
    ///
    /// Called immediately after every operation in this module that can commit
    /// a chain link, rotate a key, or write a caller-durable team member-key refresh journal.
    fn record_federation_commit(&self, cascade: &mut FederationCascade) {
        cascade.record_commit(&self.profile.name);
        count_federation_read(|counters| {
            counters.graph_loads_at_last_commit = counters.graph_loads;
        });
    }

    /// The session for one remote hop, opened once per cascade.
    ///
    /// Opening it per binding and per convergence pass builds a fresh
    /// connection pool each time, so every remote call in the refresh pays its
    /// own TLS handshake. The cached session is equivalent to a freshly opened
    /// one: [`ProfileSession::related_profile`] derives it from the registry
    /// snapshot this call already holds, and copies operation controls that are
    /// identical for every session in one cascade.
    fn federation_session(
        &self,
        registry: &ProfileRegistry,
        name: &str,
        cascade: &mut FederationCascade,
    ) -> Result<std::rc::Rc<ProfileSession>> {
        if let Some(session) = cascade.sessions.get(name) {
            return Ok(std::rc::Rc::clone(session));
        }
        let session = std::rc::Rc::new(self.related_profile(registry, name)?);
        count_federation_read(|counters| counters.remote_sessions += 1);
        cascade
            .sessions
            .insert(name.to_owned(), std::rc::Rc::clone(&session));
        Ok(session)
    }

    /// Aliases of Yubi accounts this profile holds that the caller has not
    /// unlocked. An empty list means every hardware credential on the profile
    /// is already available to the current operation.
    fn locked_yubi_aliases(
        &self,
        unlocked: &[UnlockedYubiActor<'_, '_>],
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<String>> {
        let mut locked = Vec::new();
        for alias in vault.yubi_aliases()? {
            if unlocked
                .iter()
                .any(|actor| actor.profile == self.profile.name && actor.alias == alias)
            {
                continue;
            }
            // A pending enrollment has no usable credential yet, so it can
            // never be the administrator this refresh is waiting for.
            if vault.yubi_account(&alias).is_ok() {
                locked.push(alias);
            }
        }
        Ok(locked)
    }

    #[allow(clippy::too_many_arguments)]
    fn load_federated_graph_recipients<'a, 'device>(
        &self,
        root_team_alias: &str,
        unlocked: &'a [UnlockedYubiActor<'a, 'device>],
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
        cascade: &mut FederationCascade,
    ) -> Result<FederatedGraphRecipients<'a, 'device>> {
        let root = self.select_local_federated_admin(root_team_alias, unlocked, local_vault)?;
        // The graph `root` was read from is current as of this snapshot; every
        // reuse below re-checks it against the cascade's commit counter.
        let root_commits = cascade.commits(&self.profile.name);
        // The selection authenticated this credential and walked its whole
        // membership graph a moment ago. Rediscovering it here would issue the
        // identical call and discard the identical answer.
        let reachable = connected_local_team_component(root.target().verified.team(), root.edges());
        let mut scheduled = Vec::new();
        for alias in local_vault.team_aliases()? {
            let stored = local_vault.team(&alias)?;
            if !stored.active || !reachable.contains(&stored.team_id) {
                continue;
            }
            let bindings = stored
                .federated_members
                .iter()
                .filter(|member| member.active)
                .map(|member| ScheduledFederationBinding {
                    local_team_alias: alias.clone(),
                    remote_profile: member.remote_profile.clone(),
                    remote_team_alias: member.remote_team_alias.clone(),
                    destination: member.destination,
                })
                .collect::<Vec<_>>();
            if !bindings.is_empty() {
                scheduled.push((alias, stored.team_id.clone(), bindings));
            }
        }
        let mut supplied = std::collections::BTreeMap::new();
        for (alias, team_id, bindings) in scheduled {
            let actor = self.select_federated_admin_reusing(
                &alias,
                &root,
                root_commits,
                unlocked,
                local_vault,
                cascade,
            )?;
            let parties = self.load_federated_team_recipients(
                &bindings,
                &actor,
                unlocked,
                local_vault,
                registry,
                credentials,
                master_key,
                cascade,
            )?;
            if supplied.insert(team_id, parties).is_some() {
                return Err(Error::InvalidAccount(
                    "federation graph contains a duplicate local team",
                ));
            }
        }
        // Only the credential of the returned actor is used downstream, and a
        // remote hop cannot move this profile unless the cascade came back
        // around to it, which the commit counter records.
        Ok((
            self.select_federated_admin_reusing(
                root_team_alias,
                &root,
                root_commits,
                unlocked,
                local_vault,
                cascade,
            )?,
            supplied,
            reachable,
        ))
    }

    /// Runs the federated post-revocation team member-key refresh responder for one local team.
    /// `unlocked` carries any hardware credentials the caller has already
    /// opened, for either side; an empty slice is the unattended software
    /// case and behaves exactly as before this abstraction existed.
    #[allow(clippy::too_many_arguments)]
    fn refresh_federated_team_security(
        &self,
        local_team_alias: &str,
        unlocked: &[UnlockedYubiActor<'_, '_>],
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<()> {
        self.refresh_federated_team_security_inner(
            local_team_alias,
            unlocked,
            local_vault,
            registry,
            credentials,
            master_key,
            &mut FederationCascade::rooted_at(&self.profile.name, local_team_alias),
        )
    }

    /// The body of [`Self::refresh_federated_team_security`], carrying the
    /// chain of federation hops already being resolved on this thread so the
    /// cascade cannot re-enter a team it is already computing.
    #[allow(clippy::too_many_arguments)]
    fn refresh_federated_team_security_inner(
        &self,
        local_team_alias: &str,
        unlocked: &[UnlockedYubiActor<'_, '_>],
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
        cascade: &mut FederationCascade,
    ) -> Result<()> {
        let stored = local_vault.team(local_team_alias)?;
        let bindings = stored
            .federated_members
            .iter()
            .filter(|member| member.active)
            .map(|member| ScheduledFederationBinding {
                local_team_alias: local_team_alias.to_owned(),
                remote_profile: member.remote_profile.clone(),
                remote_team_alias: member.remote_team_alias.clone(),
                destination: member.destination,
            })
            .collect::<Vec<_>>();
        if bindings.is_empty() {
            return Ok(());
        }

        let host = self.pinned_host()?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        self.cleanup_inactive_team_rekeys(&host, local_vault, &mut mutations)?;
        self.record_federation_commit(cascade);

        // First run the local graph without remote projections. Its visible
        // reconciliation path uses only authenticated local evidence, so an
        // unreachable remote profile cannot strand a committed team member-key refresh intent.
        let local_actor =
            self.select_local_federated_admin(local_team_alias, unlocked, local_vault)?;
        // The selection already walked this credential's membership graph.
        let local_scope = connected_local_team_component(
            local_actor.target().verified.team(),
            local_actor.edges(),
        );
        self.refresh_authenticated_team_graph(
            &host,
            local_actor.credential().team_refresh(),
            &std::collections::BTreeMap::new(),
            Some(&local_scope),
            None,
            local_vault,
            &mut mutations,
        )?;
        self.record_federation_commit(cascade);

        for convergence in 0..2 {
            let (local_actor, supplied, local_scope) = self.load_federated_graph_recipients(
                local_team_alias,
                unlocked,
                local_vault,
                registry,
                credentials,
                master_key,
                cascade,
            )?;
            self.refresh_authenticated_team_graph(
                &host,
                local_actor.credential().team_refresh(),
                &supplied,
                Some(&local_scope),
                None,
                local_vault,
                &mut mutations,
            )?;
            self.record_federation_commit(cascade);

            // Cross-host freshness cannot be committed atomically by the
            // v0.1.9 protocol. Re-read both sides after the local commit and
            // immediately converge once more if a remote PTK advanced in the
            // race window. This selection is deliberately the uncached one:
            // reuse is what the commit just recorded above forbids, and the
            // whole point of the re-read is to observe what the commit and any
            // concurrent remote writer did.
            let current =
                self.select_local_federated_admin(local_team_alias, unlocked, local_vault)?;
            let confirmed = self.load_federated_team_recipients(
                &bindings,
                &current,
                unlocked,
                local_vault,
                registry,
                credentials,
                master_key,
                cascade,
            )?;
            if federated_roster_matches(&current.target().verified, &confirmed) {
                return Ok(());
            }
            if convergence == 1 {
                return Err(Error::BackgroundRefresh(
                    "federated recipient advanced during both immediate team member-key refresh convergence attempts"
                        .to_owned(),
                ));
            }
        }
        unreachable!("bounded federated team member-key refresh convergence loop always returns")
    }

    fn authenticated_member_edit_parties(
        &self,
        team_alias: &str,
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>> {
        let stored = local_vault.team(team_alias)?;
        if !stored.federated_members.iter().any(|member| member.active) {
            return Ok(std::collections::BTreeMap::new());
        }

        // A roster edit rotates PTKs. First converge the existing federation
        // bindings, then re-authenticate every scoped recipient against the
        // resulting local head. An inactive or unbound scoped row is never
        // treated as a usable key recipient.
        self.refresh_federated_team_security(
            team_alias,
            &[],
            local_vault,
            registry,
            credentials,
            master_key,
        )?;
        let mut cascade = FederationCascade::rooted_at(&self.profile.name, team_alias);
        let (_, mut supplied, _) = self.load_federated_graph_recipients(
            team_alias,
            &[],
            local_vault,
            registry,
            credentials,
            master_key,
            &mut cascade,
        )?;
        supplied
            .remove(&stored.team_id)
            .ok_or(Error::InvalidAccount(
                "active federation bindings did not authenticate the edited team roster",
            ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn demote_local_team_member_in_authenticated_roster(
        &self,
        team_alias: &str,
        party_id_hex: &str,
        destination: super::team::TeamMemberRole,
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<super::team::TeamMemberMutationReport> {
        self.profile.require(Capability::Teams)?;
        let _scheduler_lock = super::runtime::ProfileLock::scheduler(&self.paths)?;
        let parties = self.authenticated_member_edit_parties(
            team_alias,
            local_vault,
            registry,
            credentials,
            master_key,
        )?;
        self.demote_local_team_member_with_parties(
            team_alias,
            party_id_hex,
            destination,
            &parties,
            local_vault,
            master_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn remove_local_team_member_in_authenticated_roster(
        &self,
        team_alias: &str,
        party_id_hex: &str,
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<super::team::TeamMemberMutationReport> {
        self.profile.require(Capability::Teams)?;
        let _scheduler_lock = super::runtime::ProfileLock::scheduler(&self.paths)?;
        let parties = self.authenticated_member_edit_parties(
            team_alias,
            local_vault,
            registry,
            credentials,
            master_key,
        )?;
        self.remove_local_team_member_with_parties(
            team_alias,
            party_id_hex,
            &parties,
            local_vault,
            master_key,
        )
    }

    pub fn resume_local_team_member_edit_in_authenticated_roster(
        &self,
        team_alias: &str,
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<super::team::TeamMemberMutationReport> {
        self.profile.require(Capability::Teams)?;
        let _scheduler_lock = super::runtime::ProfileLock::scheduler(&self.paths)?;
        // A remotely accepted edit can be waiting only for local
        // finalization. Complete that path without making the already
        // committed operation depend on every federation profile still being
        // reachable. A pre-submit mixed-roster edit fails before mutation
        // with this exact binding error, at which point we authenticate the
        // external recipients and retry.
        match self.resume_local_team_member_edit(team_alias, local_vault, master_key) {
            Ok(report) => return Ok(report),
            Err(Error::Client(foks_client::Error::TeamBinding(
                "remaining non-local roster party lacks an authenticated recipient",
            ))) => {}
            Err(error) => return Err(error),
        }
        let parties = self.authenticated_member_edit_parties(
            team_alias,
            local_vault,
            registry,
            credentials,
            master_key,
        )?;
        self.resume_local_team_member_edit_with_authenticated_parties(
            team_alias,
            &parties,
            local_vault,
            master_key,
        )
    }

    /// Runs ordinary local jobs plus cross-profile federation refreshes. The
    /// caller already holds this profile's checked operation lock; the remote
    /// lock is attempted without waiting so inverse profile jobs cannot
    /// deadlock each other.
    ///
    /// This path is unattended: it never prompts for a PIN and never opens a
    /// hardware device. A binding whose only remaining administrator is a
    /// locked YubiKey reports a deferred, action-required run instead of
    /// failing, so the job remains scheduled without increasing its backoff.
    pub fn run_due_jobs_with_federation(
        &self,
        now: u64,
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<JobRunReport> {
        self.run_due_jobs_with_federation_unlocked(
            now,
            &[],
            local_vault,
            registry,
            credentials,
            master_key,
        )
    }

    /// Scheduler entry point that may additionally use hardware credentials
    /// the caller has already unlocked, for either federation side. With an
    /// empty `unlocked` this is exactly [`Self::run_due_jobs_with_federation`].
    #[allow(clippy::too_many_arguments)]
    pub fn run_due_jobs_with_federation_unlocked(
        &self,
        now: u64,
        unlocked: &[UnlockedYubiActor<'_, '_>],
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<JobRunReport> {
        self.run_due_jobs_with_federation_config(
            now,
            unlocked,
            local_vault,
            registry,
            credentials,
            master_key,
            SchedulerConfig::default(),
        )
    }

    /// Claims at most one job, leaving all other jobs unleased. The caller must
    /// finish its checked session and release admission before calling again.
    /// Keep `now` fixed across a pass so a rescheduled job cannot run twice.
    pub fn run_next_due_job_with_federation(
        &self,
        now: u64,
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<JobRunReport> {
        self.run_due_jobs_with_federation_config(
            now,
            &[],
            local_vault,
            registry,
            credentials,
            master_key,
            SchedulerConfig {
                claim_limit: 1,
                ..SchedulerConfig::default()
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_due_jobs_with_federation_config(
        &self,
        now: u64,
        unlocked: &[UnlockedYubiActor<'_, '_>],
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
        config: SchedulerConfig,
    ) -> Result<JobRunReport> {
        self.profile.require(Capability::UserSync)?;
        let _scheduler_lock = super::runtime::ProfileLock::scheduler(&self.paths)?;
        // A federation job is registered per link, but the refresh it runs is
        // alias-wide: it collects every active binding of the local team and
        // converges all of them. Running it once per link would repeat the
        // identical alias-wide pass F times for a team with F federated
        // members. This map records each alias already refreshed in this
        // batch, together with the outcome that refresh produced, so a later
        // job naming the same alias reports that outcome and advances its own
        // schedule without re-running the pass. It lives only for the duration
        // of the `run_due_jobs_locked_with` call below, which is exactly one
        // scheduler batch; nothing is carried across calls. Errors are
        // deliberately not recorded, so a failing alias still gives every job
        // its own attempt and its own backoff.
        let mut refreshed_aliases = std::collections::BTreeMap::<String, Option<String>>::new();
        self.run_due_jobs_locked_with(now, local_vault, master_key, config, |job, local_vault| {
            let binding = self
                .protected_job_binding(job, local_vault)
                .map_err(|error| error.to_string())?;
            if let Some(outcome) = refreshed_aliases.get(&binding.local_team_alias) {
                return Ok(outcome.clone());
            }
            match self.refresh_federated_team_security(
                &binding.local_team_alias,
                unlocked,
                local_vault,
                registry,
                credentials,
                master_key,
            ) {
                Ok(()) => {
                    refreshed_aliases.insert(binding.local_team_alias, None);
                    Ok(None)
                }
                Err(Error::YubiUnlockRequired(reason)) => {
                    refreshed_aliases.insert(binding.local_team_alias, Some(reason.clone()));
                    Ok(Some(reason))
                }
                Err(error) => Err(error.to_string()),
            }
        })
    }

    /// Runs each supplied hardware credential's own user, PPE, and local-team
    /// responders before any federation work begins.
    ///
    /// A team's caller-durable team member-key refresh names the exact device that must recover
    /// it, and one device's sweep skips the teams another device owns (plus
    /// everything depending on them, see `exclude_team_ancestors`). Running
    /// the credentials round-robin until no further pending hardware journal
    /// clears makes the outcome independent of the order the keys were
    /// presented in, instead of leaving whatever the first key could not
    /// reach for a later run.
    fn prepare_unlocked_yubi_responders(
        &self,
        unlocked: &[UnlockedYubiActor<'_, '_>],
        local_vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
        cascade: &mut FederationCascade,
    ) -> Result<()> {
        let local = unlocked
            .iter()
            .filter(|actor| actor.profile == self.profile.name)
            .collect::<Vec<_>>();
        if local.len() < 2 {
            // One credential cannot be waiting on another, and the ordinary
            // selection path already runs the single credential's responders.
            return Ok(());
        }
        let host = self.pinned_host()?;
        let mut before = self.pending_hardware_team_rekeys(local_vault)?;
        for _ in 0..=local.len() {
            for actor in &local {
                let authenticated = self
                    .client
                    .authenticate_yubi_and_pin(&host, actor.credential)?;
                self.run_unlocked_yubi_security_responders(
                    actor.alias,
                    &host,
                    actor.credential,
                    authenticated,
                    local_vault,
                    master_key,
                )?;
                // The cascade can reach this profile again as some other
                // hop's remote; it must not repeat the operation executed locally.
                cascade
                    .yubi_refreshed
                    .insert((self.profile.name.clone(), actor.alias.to_owned()));
                self.record_federation_commit(cascade);
            }
            let after = self.pending_hardware_team_rekeys(local_vault)?;
            if before == 0 || after == 0 || after >= before {
                break;
            }
            before = after;
        }
        Ok(())
    }

    /// How many caller-durable team member-key refresh journals are still waiting on a hardware
    /// device. A round of responders that does not reduce this has nothing
    /// further to unblock.
    fn pending_hardware_team_rekeys(&self, local_vault: &mut AccountVault<'_>) -> Result<usize> {
        let mut count = 0;
        for alias in local_vault.team_rekey_aliases()? {
            let Some(pending) = local_vault.team_rekey(&alias)? else {
                continue;
            };
            if EntityId::from_bytes(pending.actor_device_id.clone())?.entity_type()
                == foks_proto::ENTITY_YUBI
            {
                count += 1;
            }
        }
        Ok(count)
    }

    /// Explicit federated security responder for a caller that has already
    /// unlocked one or more YubiKeys. This is the hardware-backed equivalent
    /// of the unattended post-revocation team member-key refresh responder: it renews the remote
    /// bearer, reproves the local capability, and converges the local team
    /// graph so a revoked or rotated member key stops appearing in future
    /// federated team material.
    ///
    /// A YubiKey is never opened here. Callers pass already-unlocked devices,
    /// and both sides may be hardware-backed as long as both were unlocked
    /// before the call. No PIN and no transient hardware secret is retained.
    #[allow(clippy::too_many_arguments)]
    pub fn refresh_federated_security_with_unlocked_yubi(
        &self,
        local_team_alias: &str,
        unlocked: &[UnlockedYubiActor<'_, '_>],
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<FederationRefreshReport> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Federation)?;
        let _scheduler_lock = super::runtime::ProfileLock::scheduler(&self.paths)?;
        let mut cascade = FederationCascade::rooted_at(&self.profile.name, local_team_alias);
        self.prepare_unlocked_yubi_responders(unlocked, local_vault, master_key, &mut cascade)?;
        match self.refresh_federated_team_security_inner(
            local_team_alias,
            unlocked,
            local_vault,
            registry,
            credentials,
            master_key,
            &mut cascade,
        ) {
            Ok(()) => Ok(FederationRefreshReport {
                local_profile: self.profile.name.clone(),
                local_team_alias: local_team_alias.to_owned(),
                refreshed: true,
                deferred: None,
            }),
            Err(Error::YubiUnlockRequired(reason)) => Ok(FederationRefreshReport {
                local_profile: self.profile.name.clone(),
                local_team_alias: local_team_alias.to_owned(),
                refreshed: false,
                deferred: Some(reason),
            }),
            Err(error) => Err(error),
        }
    }

    /// Runs [`Self::refresh_federated_security_with_unlocked_yubi`] for every
    /// active federated binding this profile holds. Used by the Yubi
    /// security-sync flow, where the caller has one unlocked device and can use
    /// every responder that device can drive to run.
    pub(super) fn refresh_all_federated_security_with_unlocked_yubi(
        &self,
        unlocked: &[UnlockedYubiActor<'_, '_>],
        local_vault: &mut AccountVault<'_>,
        registry: &ProfileRegistry,
        credentials: &ClientCredentials,
        master_key: &[u8; 32],
    ) -> Result<Vec<FederationRefreshReport>> {
        if self.profile.require(Capability::Federation).is_err()
            || self.profile.require(Capability::Teams).is_err()
        {
            return Ok(Vec::new());
        }
        let mut aliases = Vec::new();
        for alias in local_vault.team_aliases()? {
            let stored = local_vault.team(&alias)?;
            if stored.active && stored.federated_members.iter().any(|member| member.active) {
                aliases.push(alias);
            }
        }
        let mut reports = Vec::new();
        for alias in aliases {
            reports.push(self.refresh_federated_security_with_unlocked_yubi(
                &alias,
                unlocked,
                local_vault,
                registry,
                credentials,
                master_key,
            )?);
        }
        Ok(reports)
    }

    fn protected_job_binding(
        &self,
        job: &foks_client_db::ScheduledJob,
        vault: &mut AccountVault<'_>,
    ) -> Result<ScheduledFederationBinding> {
        if !job.scope_id.is_empty() {
            return Err(Error::InvalidAccount(
                "federation job contains an untrusted public scope",
            ));
        }
        let local_host = self.pinned_host()?;
        if job.host_id != local_host.host_id().as_bytes() {
            return Err(Error::InvalidAccount(
                "federation job is bound to another local host",
            ));
        }
        let mut found = None;
        for local_team_alias in vault.team_aliases()? {
            let team = vault.team(&local_team_alias)?;
            let local_team = EntityId::from_bytes(team.team_id.clone())?;
            for member in &team.federated_members {
                let remote_host = EntityId::from_bytes(member.remote_host_id.clone())?;
                let remote_team = EntityId::from_bytes(member.remote_team_id.clone())?;
                let candidate = federation_job_id(
                    local_host.host_id(),
                    &local_team,
                    &remote_host,
                    &remote_team,
                    &member.removal_key,
                )?;
                if candidate != job.job_id {
                    continue;
                }
                if found.is_some() {
                    return Err(Error::InvalidAccount(
                        "federation job matches multiple protected bindings",
                    ));
                }
                found = Some(ScheduledFederationBinding {
                    local_team_alias: local_team_alias.clone(),
                    remote_profile: member.remote_profile.clone(),
                    remote_team_alias: member.remote_team_alias.clone(),
                    destination: member.destination,
                });
            }
        }
        found.ok_or(Error::InvalidAccount(
            "federation job has no protected binding",
        ))
    }
}

struct ScheduledFederationBinding {
    local_team_alias: String,
    remote_profile: String,
    remote_team_alias: String,
    destination: FederationDestinationRole,
}

/// A rate-limited host returns the same error for every credential, so
/// candidate enumeration must stop rather than spend the remaining budget
/// and report a misleading "no credential" error.
fn federation_client_rate_limited(error: &foks_client::Error) -> bool {
    matches!(
        error,
        foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code: 1012, .. })
    )
}

fn federation_job_id(
    local_host: &EntityId,
    local_team: &EntityId,
    remote_host: &EntityId,
    remote_team: &EntityId,
    removal_key: &[u8; 32],
) -> Result<[u8; 16]> {
    let removal_commitment =
        foks_crypto::team_removal_key_commitment(&SecretSeed::new(*removal_key))?;
    let mut binding = Vec::with_capacity(33 * 4 + 32);
    binding.extend_from_slice(local_host.as_bytes());
    binding.extend_from_slice(local_team.as_bytes());
    binding.extend_from_slice(remote_host.as_bytes());
    binding.extend_from_slice(remote_team.as_bytes());
    binding.extend_from_slice(&removal_commitment);
    Ok(prefixed_hash(FEDERATION_JOB_TYPE_ID, &binding)[..16]
        .try_into()
        .expect("hash prefix has fixed length"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team(fill: u8) -> EntityId {
        let mut bytes = vec![fill; 33];
        bytes[0] = foks_proto::ENTITY_NAMED_TEAM;
        EntityId::from_bytes(bytes).unwrap()
    }

    #[test]
    fn federation_scope_is_only_the_roots_connected_team_component() {
        let child = team(0x11);
        let root = team(0x22);
        let parent = team(0x33);
        let unrelated_child = team(0x44);
        let unrelated_parent = team(0x55);
        let connected = connected_local_team_component(
            &root,
            &[
                (child.clone(), root.clone()),
                (root.clone(), parent.clone()),
                (unrelated_child.clone(), unrelated_parent.clone()),
            ],
        );
        assert_eq!(connected.len(), 3);
        for included in [&child, &root, &parent] {
            assert!(connected.contains(included.as_bytes()));
        }
        for excluded in [&unrelated_child, &unrelated_parent] {
            assert!(!connected.contains(excluded.as_bytes()));
        }
    }

    #[test]
    fn remote_recipient_scope_is_descendant_only() {
        let grandchild = team(0x66);
        let child = team(0x67);
        let root = team(0x68);
        let parent = team(0x69);
        let unrelated_child = team(0x6a);
        let unrelated_parent = team(0x6b);
        let descendants = descendant_team_component(
            &root,
            &[
                (grandchild.clone(), child.clone()),
                (child.clone(), root.clone()),
                (root.clone(), parent.clone()),
                (unrelated_child.clone(), unrelated_parent.clone()),
            ],
        );
        assert_eq!(descendants.len(), 3);
        for included in [&grandchild, &child, &root] {
            assert!(descendants.contains(included.as_bytes()));
        }
        for excluded in [&parent, &unrelated_child, &unrelated_parent] {
            assert!(!descendants.contains(excluded.as_bytes()));
        }
    }

    /// Two profiles on two hosts, each with a named team, joined by one active
    /// federated membership. This is the smallest shape that exercises a
    /// convergence pass: one local graph, one remote hop, one local commit and
    /// one post-commit re-read.
    struct FederationFixture {
        _environments: Vec<foks_server_testkit::TestEnvironment>,
        _servers: Vec<foks_server_testkit::InProcessServer>,
        _temporary: tempfile::TempDir,
        credentials: ClientCredentials,
        registry: ProfileRegistry,
        master: [u8; 32],
    }

    impl FederationFixture {
        fn start() -> Self {
            let environments = (0..2)
                .map(|_| foks_server_testkit::TestEnvironment::new().unwrap())
                .collect::<Vec<_>>();
            let servers = environments
                .iter()
                .map(|environment| environment.start_server().unwrap())
                .collect::<Vec<_>>();
            let temporary = tempfile::tempdir().unwrap();
            let state = temporary.path().join("state");
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
            let mut registry = ProfileRegistry::open(&state).unwrap();
            for (index, name) in ["local", "remote"].into_iter().enumerate() {
                let root = temporary.path().join(format!("{name}-root.der"));
                environments[index].write_probe_root(&root).unwrap();
                registry
                    .add(Profile {
                        name: name.to_owned(),
                        label: None,
                        probe: format!(
                            "localhost:{}",
                            environments[index].addresses().unwrap().probe.port()
                        ),
                        protocol: ProtocolPolicy::V019,
                        trust: TrustRoot::CertificateDer { path: root },
                    })
                    .unwrap();
            }
            let credentials = ClientCredentials::open(&state).unwrap();
            let master = *credentials.master_key().unwrap();
            let fixture = Self {
                _environments: environments,
                _servers: servers,
                _temporary: temporary,
                credentials,
                registry,
                master,
            };
            for name in ["local", "remote"] {
                fixture.run(name, |session, _, _| session.probe_and_pin());
            }
            fixture.run("local", |local, vault, master| {
                local.create_account(
                    "owner",
                    "localowner",
                    "local owner",
                    "local@example.test",
                    "",
                    None,
                    vault,
                    master,
                )?;
                local.create_named_team("owner", "team", "localteam", vault, master)?;
                Ok(())
            });
            fixture.run("remote", |remote, vault, master| {
                remote.create_account(
                    "owner",
                    "remoteowner",
                    "remote owner",
                    "remote@example.test",
                    "",
                    None,
                    vault,
                    master,
                )?;
                remote.create_named_team("owner", "team", "remoteteam", vault, master)?;
                remote.create_account(
                    "member",
                    "remotemember",
                    "remote member",
                    "remote-member@example.test",
                    "",
                    None,
                    vault,
                    master,
                )?;
                remote.add_local_team_member(
                    "team",
                    "remotemember",
                    super::super::team::TeamMemberRole::Admin,
                    vault,
                    master,
                )?;
                Ok(())
            });
            let local = ProfileSession::open(&fixture.registry, "local").unwrap();
            let remote = ProfileSession::open(&fixture.registry, "remote").unwrap();
            fixture
                .credentials
                .with_checked_sessions(&local, &remote, |local, remote| {
                    let mut local_store = foks_keystore::EncryptedFileSecretStore::open(
                        &local.paths.credential_store,
                        derive_vault_key(&fixture.master),
                    )?;
                    let mut remote_store = foks_keystore::EncryptedFileSecretStore::open(
                        &remote.paths.credential_store,
                        derive_vault_key(&fixture.master),
                    )?;
                    local.add_federated_team_member(
                        remote,
                        "team",
                        "team",
                        FederationDestinationRole::Member { visibility: 0 },
                        &mut AccountVault::new(&mut local_store),
                        &mut AccountVault::new(&mut remote_store),
                        &fixture.master,
                    )
                })
                .unwrap();
            fixture
        }

        fn run<T>(
            &self,
            profile: &str,
            operation: impl FnOnce(
                &CheckedProfileSession<'_>,
                &mut AccountVault<'_>,
                &[u8; 32],
            ) -> Result<T>,
        ) -> T {
            self.try_run(profile, operation).unwrap()
        }

        fn try_run<T>(
            &self,
            profile: &str,
            operation: impl FnOnce(
                &CheckedProfileSession<'_>,
                &mut AccountVault<'_>,
                &[u8; 32],
            ) -> Result<T>,
        ) -> Result<T> {
            let session = ProfileSession::open(&self.registry, profile).unwrap();
            self.credentials.with_checked_session(&session, |session| {
                let mut store = foks_keystore::EncryptedFileSecretStore::open(
                    &session.paths.credential_store,
                    derive_vault_key(&self.master),
                )?;
                operation(session, &mut AccountVault::new(&mut store), &self.master)
            })
        }

        fn refresh(&self) -> Result<()> {
            self.try_run("local", |local, vault, master| {
                local.refresh_federated_team_security(
                    "team",
                    &[],
                    vault,
                    &self.registry,
                    &self.credentials,
                    master,
                )
            })
        }

        /// The local team's authenticated state together with the recipient
        /// projection of every federated binding it holds, exactly as one
        /// convergence pass computes them.
        fn converged(&self) -> bool {
            self.run("local", |local, vault, master| {
                let mut cascade = FederationCascade::rooted_at(&local.profile.name, "team");
                let (actor, supplied, _) = local.load_federated_graph_recipients(
                    "team",
                    &[],
                    vault,
                    &self.registry,
                    &self.credentials,
                    master,
                    &mut cascade,
                )?;
                let parties = supplied
                    .get(actor.target().verified.team().as_bytes())
                    .ok_or(Error::InvalidAccount("test binding produced no parties"))?;
                Ok(federated_roster_matches(&actor.target().verified, parties))
            })
        }

        /// Rotates the remote team's PTKs without telling the local side, by
        /// removing a member of that team.
        fn rotate_remote_team(&self) {
            self.run("remote", |remote, vault, master| {
                let party = remote
                    .list_team_members("team", vault)?
                    .into_iter()
                    .find(|member| member.username.as_deref() == Some("remotemember"))
                    .map(|member| member.party_id_hex)
                    .ok_or(Error::InvalidAccount("remote member is missing"))?;
                remote.remove_local_team_member("team", &party, vault, master)?;
                Ok(())
            });
        }

        /// Backdates the remote profile's own user and team refresh jobs to a
        /// clean completion a moment ago, which is what makes a cascade treat
        /// that profile's ordinary responders as already run.
        fn settle_remote_refresh_jobs(&self) {
            let database = self
                .registry
                .prepare_profile_directory("remote")
                .unwrap()
                .hard_database;
            let now = now_microseconds().unwrap();
            let connection = rusqlite::Connection::open(database).unwrap();
            let updated = connection
                .execute(
                    "UPDATE scheduled_jobs
                     SET last_completed_at = ?1, failure_count = 0, last_error = NULL,
                         lease_until = NULL, next_run_at = ?1 + interval_micros
                     WHERE job_kind IN (?2, ?3)",
                    rusqlite::params![
                        now as i64,
                        ScheduledJobKind::UserRefresh as u8,
                        ScheduledJobKind::TeamRefresh as u8,
                    ],
                )
                .unwrap();
            assert!(
                updated >= 2,
                "the remote profile must have both ordinary refresh jobs registered"
            );
        }
    }

    /// The reuse rules have to pay for themselves: a refresh must not walk the
    /// same credential's membership graph again for every team and every pass.
    ///
    /// Before the graph behind a selection was shared, this shape performed ten
    /// authenticated graph discoveries per refresh: seven on the local side,
    /// two of them verbatim rediscoveries of a graph the selection immediately
    /// before had just produced, and three on the remote side. It now performs
    /// six, or five once the remote profile's own scheduler has swept, and the
    /// only ones left are the loads a commit boundary makes mandatory.
    #[test]
    fn a_federation_refresh_reuses_authenticated_graphs_instead_of_rediscovering_them() {
        let fixture = FederationFixture::start();
        let before = federation_read_counters();
        fixture.refresh().unwrap();
        let after = federation_read_counters();
        let loads = after.graph_loads - before.graph_loads;
        let reused = after.reused_selections - before.reused_selections;
        let sessions = after.remote_sessions - before.remote_sessions;
        assert!(
            reused >= 2,
            "the selections a commit did not invalidate must reuse a loaded graph: {after:?}"
        );
        assert!(
            loads <= 6,
            "a two-profile refresh must not load more than six authenticated graphs: {loads}"
        );
        assert_eq!(
            sessions, 1,
            "the one remote profile must be opened once, so its connection pool is reused"
        );

        assert!(
            (after.ran_responders - before.ran_responders) >= 1,
            "a remote profile with no recorded sweep must have one run for it"
        );

        // A second refresh skips the remote profile's ordinary responders once
        // that profile's own scheduler has run them within their interval.
        fixture.settle_remote_refresh_jobs();
        let before = federation_read_counters();
        fixture.refresh().unwrap();
        let after = federation_read_counters();
        assert_eq!(
            after.ran_responders - before.ran_responders,
            0,
            "a settled remote profile must not have its user and team sweep repeated"
        );
        assert!(
            (after.skipped_responders - before.skipped_responders) >= 1,
            "the sweep must be recorded as skipped rather than silently omitted"
        );
        assert_eq!(
            after.unskipped_responders - before.unskipped_responders,
            0,
            "nothing was stale, so no skipped sweep should have been forced"
        );
    }

    /// The re-read after the local team member-key refresh commit is the whole cross-host
    /// freshness guarantee: it is what notices a remote PTK that advanced
    /// inside the race window the protocol cannot close. It must therefore
    /// never be served from a graph read before that commit.
    #[test]
    fn the_federated_selection_after_the_local_commit_is_a_fresh_read() {
        let fixture = FederationFixture::start();
        let before = federation_read_counters();
        fixture.refresh().unwrap();
        let after = federation_read_counters();
        assert!(
            after.graph_loads > after.graph_loads_at_last_commit,
            "a refresh must authenticate a graph again after the last state it committed: {after:?}"
        );
        assert!(after.graph_loads_at_last_commit > before.graph_loads);

        // The gate itself: a recorded commit forces the very next selection
        // for the same profile back onto a fresh candidate walk, and without
        // one the identical call is served from the held graph.
        fixture.run("local", |local, vault, master| {
            let _ = master;
            let mut cascade = FederationCascade::rooted_at(&local.profile.name, "team");
            let held = local.select_local_federated_admin("team", &[], vault)?;
            let commits = cascade.commits(&local.profile.name);

            let before = federation_read_counters();
            local.select_federated_admin_reusing("team", &held, commits, &[], vault, &cascade)?;
            let after = federation_read_counters();
            assert_eq!(
                after.graph_loads, before.graph_loads,
                "an uncommitted selection must not reload"
            );
            assert_eq!(after.reused_selections, before.reused_selections + 1);

            local.record_federation_commit(&mut cascade);
            let before = federation_read_counters();
            local.select_federated_admin_reusing("team", &held, commits, &[], vault, &cascade)?;
            let after = federation_read_counters();
            assert_eq!(
                after.graph_loads,
                before.graph_loads + 1,
                "a selection across a commit must authenticate the graph again"
            );
            assert_eq!(
                after.reused_selections, before.reused_selections,
                "a selection across a commit must not be served from the held graph"
            );
            Ok(())
        });
    }

    /// Convergence is declared only when every authenticated roster row names
    /// the key the remote side currently holds. A remote team that rotated
    /// behind this profile's back must fail that test, including when the
    /// remote profile's own responders were skipped as already current.
    #[test]
    fn a_stale_remote_roster_fails_convergence_rather_than_matching() {
        let fixture = FederationFixture::start();
        fixture.refresh().unwrap();
        assert!(
            fixture.converged(),
            "the first refresh must leave the binding converged"
        );

        fixture.rotate_remote_team();
        fixture.settle_remote_refresh_jobs();
        assert!(
            !fixture.converged(),
            "a roster row naming a superseded remote key must not be reported as converged"
        );

        let before = federation_read_counters();
        fixture.refresh().unwrap();
        let after = federation_read_counters();
        assert!(
            after.skipped_responders > before.skipped_responders,
            "the settled remote profile's ordinary responders must have been skipped"
        );
        assert!(
            fixture.converged(),
            "the refresh must repair the roster rather than leave it stale"
        );
    }

    /// The rows this module looks up are the ones a profile actually
    /// registers. The identities are now shared rather than restated, so this
    /// no longer guards against two copies drifting; what it still pins is
    /// that the derivation, the kind and the scope agree end to end with the
    /// registration, which a shared constant alone does not establish.
    #[test]
    fn default_refresh_job_ids_match_the_registered_jobs() {
        let fixture = crate::test_support::AccountFixture::start();
        let uid = fixture.run(|session, vault, master| {
            session.create_account(
                "owner",
                "jobidowner",
                "job id owner",
                "jobid@example.test",
                "",
                None,
                vault,
                master,
            )?;
            let uid = vault.account("owner")?.credential.uid.clone();
            session.register_default_refresh_jobs_for(&uid, now_microseconds()?)?;
            Ok(uid)
        });
        fixture.run(|session, _, _| {
            let host = session.pinned_host()?;
            let store = HardStateStore::open(&session.paths.hard_database)?;
            for (type_id, kind) in [
                (USER_REFRESH_JOB_TYPE_ID, ScheduledJobKind::UserRefresh),
                (TEAM_REFRESH_JOB_TYPE_ID, ScheduledJobKind::TeamRefresh),
            ] {
                let job = store
                    .scheduled_job(&refresh_job_id(type_id, host.host_id(), &uid))?
                    .unwrap_or_else(|| panic!("{kind:?} job is registered under another identity"));
                assert_eq!(job.kind, kind);
                assert_eq!(job.scope_id, uid.as_bytes());
            }
            Ok(())
        });
    }
}
