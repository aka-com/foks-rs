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

/// An administrator for one federated local team, together with the
/// authenticated membership graph it was selected from. Keeping the graph
/// here rather than rediscovering it per binding matters: a cascade visits
/// the same profile several times per convergence pass, and a redundant walk
/// is a full round of chain loads against that host.
struct LocalFederatedActor<'a, 'device> {
    credential: FederatedActorCredential<'a, 'device>,
    user: foks_client::AuthenticatedUserOutcome,
    target: foks_client::AuthenticatedTeamOutcome,
    actor_team: Option<foks_client::AuthenticatedTeamOutcome>,
    other_teams: Vec<foks_client::AuthenticatedTeamOutcome>,
    child_first: Vec<EntityId>,
    edges: Vec<(EntityId, EntityId)>,
}

type FederatedGraphRecipients<'a, 'device> = (
    LocalFederatedActor<'a, 'device>,
    std::collections::BTreeMap<
        Vec<u8>,
        std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>,
    >,
    std::collections::BTreeSet<Vec<u8>>,
);

impl LocalFederatedActor<'_, '_> {
    fn team(&self, team: &EntityId) -> Option<&foks_client::AuthenticatedTeamOutcome> {
        if self.target.verified.team() == team {
            return Some(&self.target);
        }
        self.actor_team
            .as_ref()
            .filter(|candidate| candidate.verified.team() == team)
            .or_else(|| {
                self.other_teams
                    .iter()
                    .find(|candidate| candidate.verified.team() == team)
            })
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
pub struct FederationAdmissionReport {
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
    pub fn admit_federated_team(
        &self,
        remote: &CheckedProfileSession<'_>,
        local_team_alias: &str,
        remote_team_alias: &str,
        destination: FederationDestinationRole,
        local_vault: &mut AccountVault<'_>,
        remote_vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<FederationAdmissionReport> {
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
        let request = foks_client::FederatedTeamAdmissionRequest {
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
            .admit_remote_team_to_named_team(&request, &mut local_mutations)?;

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

        Ok(FederationAdmissionReport {
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
                    &mut FederationCascade {
                        visited: std::collections::BTreeSet::from([(
                            self.profile.name.clone(),
                            local_team_alias.to_owned(),
                        )]),
                        ..FederationCascade::default()
                    },
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
                    "pending federation expulsion lost its protected admission key",
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
        if remote_actor.target.verified.team() != &remote_team_id {
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
            remote_credential: remote_actor.credential.borrowed(),
            remote_team: &remote_team_id,
            local_host: &local_host,
            local_credential: local_actor.credential.borrowed(),
            local_team: &local_team_id,
        };
        self.client
            .refresh_federated_team_capability_with_actors(
                &request,
                &remote_actor.user.verified,
                remote_actor.actor_team.as_ref(),
                &local_actor.user.verified,
                local_actor.actor_team.as_ref(),
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
                let remote_session = self.related_profile(registry, &binding.remote_profile)?;
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
                        // does. The software sweep is unattended and PIN-free; a
                        // hardware actor runs the equivalent already-unlocked
                        // responders, which the software sweep cannot drive
                        // because it must never prompt.
                        let ran_responders = match &remote_actor.credential {
                            FederatedActorCredential::Software(credential) => {
                                let key = (
                                    remote.profile.name.clone(),
                                    credential.uid.as_bytes().to_vec(),
                                );
                                cascade.software_refreshed.insert(key) && {
                                    remote
                                        .refresh_user_security(
                                            credential.uid.as_bytes(),
                                            &mut remote_vault,
                                            master_key,
                                        )
                                        .map_err(Error::BackgroundRefresh)?;
                                    remote
                                        .refresh_team_chains(
                                            credential.uid.as_bytes(),
                                            &mut remote_vault,
                                            master_key,
                                        )
                                        .map_err(Error::BackgroundRefresh)?;
                                    true
                                }
                            }
                            FederatedActorCredential::Yubi { alias, credential } => {
                                let key = (remote.profile.name.clone(), (*alias).to_owned());
                                cascade.yubi_refreshed.insert(key) && {
                                    let remote_host = remote.pinned_host()?;
                                    let authenticated = remote
                                        .client
                                        .authenticate_yubi_and_pin(&remote_host, credential)?;
                                    remote.run_unlocked_yubi_security_responders(
                                        alias,
                                        &remote_host,
                                        credential,
                                        authenticated,
                                        &mut remote_vault,
                                        master_key,
                                    )?;
                                    true
                                }
                            }
                        };
                        if ran_responders {
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
    /// CLKR can rekey against.
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
        let relevant = descendant_team_component(public.verified.team(), &remote_actor.edges);
        let mut recipients =
            std::collections::BTreeMap::<Vec<u8>, foks_client::VerifiedTeamRecipient>::new();
        for team_id in &remote_actor.child_first {
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
                    let nested_actor = remote.select_local_federated_admin(
                        nested_alias,
                        unlocked,
                        remote_vault,
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
                                remote_actor.credential.team_refresh(),
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
    /// caller-durable CLKR preference software uses; authority is always
    /// re-derived from the authenticated membership graph.
    fn select_local_federated_admin<'a, 'device>(
        &self,
        local_team_alias: &str,
        unlocked: &'a [UnlockedYubiActor<'a, 'device>],
        local_vault: &mut AccountVault<'_>,
    ) -> Result<LocalFederatedActor<'a, 'device>> {
        let host = self.pinned_host()?;
        let stored = local_vault.team(local_team_alias)?;
        let team_id = EntityId::from_bytes(stored.team_id.clone())?;
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
            let Some(target) = graph.team(&team_id) else {
                errors.push(format!(
                    "{} cannot reach the federated target through its membership graph",
                    hex(credential.uid().as_bytes())
                ));
                continue;
            };
            let direct_admin = target.verified.members().iter().any(|member| {
                member.party == *credential.uid()
                    && member.scoped_host.is_none()
                    && matches!(
                        member.role.kind(),
                        foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
                    )
            });
            let selected_actor = if let Some(pending) = pending.as_ref() {
                if pending.transport_uid != credential.uid().as_bytes() {
                    errors.push(format!(
                        "{} is not the caller-durable CLKR transport",
                        hex(credential.uid().as_bytes())
                    ));
                    continue;
                }
                let recorded = EntityId::from_bytes(pending.actor_uid.clone())?;
                let recorded_team_is_current = graph.team(&recorded).is_some_and(|actor| {
                    local_team_can_observe_team_rekey(&target.verified, actor)
                });
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
                                && graph
                                    .team(&member.party)
                                    .is_some_and(|actor| actor.holds_roster_private_key(member))
                        })
                        .max_by_key(|member| (member.role, member.source_role))
                    else {
                        errors.push(format!(
                            "{} cannot recover the visible caller-durable CLKR actor",
                            hex(credential.uid().as_bytes())
                        ));
                        continue;
                    };
                    member.party.clone()
                } else {
                    errors.push(format!(
                        "{} cannot recover the caller-durable CLKR actor",
                        hex(credential.uid().as_bytes())
                    ));
                    continue;
                }
            } else {
                if direct_admin {
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
                            && graph.team(&member.party).is_some()
                            && graph
                                .team(&member.party)
                                .is_some_and(|actor| actor.holds_roster_private_key(member))
                    })
                    .max_by_key(|member| (member.role, member.source_role))
                {
                    member.party.clone()
                } else {
                    errors.push(format!(
                        "{} has no direct or local-team administrator path",
                        hex(credential.uid().as_bytes())
                    ));
                    continue;
                }
            };
            let mut teams = graph.teams;
            let child_first = graph.child_first;
            let edges = graph.edges;
            let target_index = teams
                .iter()
                .position(|team| team.verified.team() == &team_id)
                .ok_or(Error::InvalidAccount(
                    "membership graph lost the federated target",
                ))?;
            let target = teams.remove(target_index);
            let actor_team = if selected_actor == *credential.uid() {
                None
            } else {
                let index = teams
                    .iter()
                    .position(|team| team.verified.team() == &selected_actor)
                    .ok_or(Error::InvalidAccount(
                        "membership graph lost the federated local-team actor",
                    ))?;
                Some(teams.remove(index))
            };
            return Ok(LocalFederatedActor {
                credential,
                user: actor,
                target,
                actor_team,
                other_teams: teams,
                child_first,
                edges,
            });
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
        let host = self.pinned_host()?;
        let root = self.select_local_federated_admin(root_team_alias, unlocked, local_vault)?;
        let graph = self.client.discover_local_team_graph_with_credential(
            &host,
            root.credential.borrowed(),
            &root.user.verified,
            &root.user.puks,
        )?;
        let reachable = connected_local_team_component(root.target.verified.team(), &graph.edges);
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
            let actor = self.select_local_federated_admin(&alias, unlocked, local_vault)?;
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
        Ok((
            self.select_local_federated_admin(root_team_alias, unlocked, local_vault)?,
            supplied,
            reachable,
        ))
    }

    /// Runs the federated post-revocation CLKR responder for one local team.
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
            &mut FederationCascade {
                visited: std::collections::BTreeSet::from([(
                    self.profile.name.clone(),
                    local_team_alias.to_owned(),
                )]),
                ..FederationCascade::default()
            },
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

        // First run the local graph without remote projections. Its visible
        // reconciliation path uses only authenticated local evidence, so an
        // unreachable remote profile cannot strand a committed CLKR intent.
        let local_actor =
            self.select_local_federated_admin(local_team_alias, unlocked, local_vault)?;
        let local_graph = self.client.discover_local_team_graph_with_credential(
            &host,
            local_actor.credential.borrowed(),
            &local_actor.user.verified,
            &local_actor.user.puks,
        )?;
        let local_scope =
            connected_local_team_component(local_actor.target.verified.team(), &local_graph.edges);
        self.refresh_authenticated_team_graph(
            &host,
            local_actor.credential.team_refresh(),
            &std::collections::BTreeMap::new(),
            Some(&local_scope),
            None,
            local_vault,
            &mut mutations,
        )?;

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
                local_actor.credential.team_refresh(),
                &supplied,
                Some(&local_scope),
                None,
                local_vault,
                &mut mutations,
            )?;

            // Cross-host freshness cannot be committed atomically by the
            // v0.1.9 protocol. Re-read both sides after the local commit and
            // immediately converge once more if a remote PTK advanced in the
            // race window.
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
            if federated_roster_matches(&current.target.verified, &confirmed) {
                return Ok(());
            }
            if convergence == 1 {
                return Err(Error::BackgroundRefresh(
                    "federated recipient advanced during both immediate CLKR convergence attempts"
                        .to_owned(),
                ));
            }
        }
        unreachable!("bounded federated CLKR convergence loop always returns")
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
        let mut cascade = FederationCascade {
            visited: std::collections::BTreeSet::from([(
                self.profile.name.clone(),
                team_alias.to_owned(),
            )]),
            ..FederationCascade::default()
        };
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
        self.profile.require(Capability::UserSync)?;
        let _scheduler_lock = super::runtime::ProfileLock::scheduler(&self.paths)?;
        self.run_due_jobs_locked_with(now, local_vault, master_key, |job, local_vault| {
            let binding = self
                .protected_job_binding(job, local_vault)
                .map_err(|error| error.to_string())?;
            match self.refresh_federated_team_security(
                &binding.local_team_alias,
                unlocked,
                local_vault,
                registry,
                credentials,
                master_key,
            ) {
                Ok(()) => Ok(None),
                Err(Error::YubiUnlockRequired(reason)) => Ok(Some(reason)),
                Err(error) => Err(error.to_string()),
            }
        })
    }

    /// Runs each supplied hardware credential's own user, PPE, and local-team
    /// responders before any federation work begins.
    ///
    /// A team's caller-durable CLKR names the exact device that must recover
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
            }
            let after = self.pending_hardware_team_rekeys(local_vault)?;
            if before == 0 || after == 0 || after >= before {
                break;
            }
            before = after;
        }
        Ok(())
    }

    /// How many caller-durable CLKR journals are still waiting on a hardware
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
    /// of the unattended post-revocation CLKR responder: it renews the remote
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
        let mut cascade = FederationCascade {
            visited: std::collections::BTreeSet::from([(
                self.profile.name.clone(),
                local_team_alias.to_owned(),
            )]),
            ..FederationCascade::default()
        };
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
}
