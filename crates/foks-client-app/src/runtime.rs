use std::fs::{File, OpenOptions};

use foks_client::{
    AuthenticatedTeamOutcome, AuthenticatedUserOutcome, DeviceCredential,
    EncryptedFileMutationStore, FoksScheduler, NoPassphraseConfigured,
    RefreshTeamMemberKeysRequest, ScheduledJobRegistration, SchedulerConfig, TeamMemberKeyRefresh,
    TeamMemberSelector, TeamPtkRotationSeed, UserPukRotation, VerifiedMemberParty,
    VerifiedTeamRecipient, YubiCredential,
};
use foks_client_db::{
    HardStateStore, MutationKind, MutationState, ScheduledJobKind, TeamMutationState,
};
use fs2::FileExt as _;
use serde::Serialize;

use crate::team::{StoredTeamPtkRotation, StoredTeamRekey, StoredTeamRekeyChange, StoredTeamRole};
use crate::{
    derive_mutation_key, hex, now_microseconds, random_array, AccountVault, Capability,
    CheckedProfileSession, Error, ProfilePaths, Result,
};

const USER_REFRESH_JOB_TYPE_ID: u64 = 0xb1a8_c09a_d2b9_4de7;
const TEAM_REFRESH_JOB_TYPE_ID: u64 = 0x8583_9c90_0eb4_b47e;
const TEAM_REFRESH_ALIAS_TYPE_ID: u64 = 0xb5ee_e7c5_3473_ed98;
const DEFAULT_USER_REFRESH_INTERVAL_MICROS: u64 = 15 * 60 * 1_000_000;
const DEFAULT_TEAM_REFRESH_INTERVAL_MICROS: u64 = 17 * 60 * 1_000_000;
const OPERATION_LOCK_FILE: &str = ".profile-operation.lock";
const SCHEDULER_LOCK_FILE: &str = ".scheduler-run.lock";
const DATABASE_LOCK_DIRECTORY: &str = ".database-operation-locks";

pub(crate) struct ProfileLock {
    file: File,
}

pub(crate) struct DatabaseLock {
    file: File,
}

pub(crate) struct NativeManifestLock {
    file: File,
}

impl ProfileLock {
    pub(crate) fn operation(paths: &ProfilePaths) -> Result<Self> {
        Self::acquire(paths, OPERATION_LOCK_FILE)
    }

    pub(crate) fn try_operation(paths: &ProfilePaths) -> Result<Option<Self>> {
        Self::try_acquire(paths, OPERATION_LOCK_FILE)
    }

    pub(crate) fn scheduler(paths: &ProfilePaths) -> Result<Self> {
        Self::acquire(paths, SCHEDULER_LOCK_FILE)
    }

    fn try_scheduler(paths: &ProfilePaths) -> Result<Option<Self>> {
        Self::try_acquire(paths, SCHEDULER_LOCK_FILE)
    }

    fn acquire(paths: &ProfilePaths, name: &str) -> Result<Self> {
        let file = open_lock(paths, name)?;
        file.lock_exclusive()?;
        Ok(Self { file })
    }

    fn try_acquire(paths: &ProfilePaths, name: &str) -> Result<Option<Self>> {
        let file = open_lock(paths, name)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn release(self) -> Result<()> {
        self.file.unlock()?;
        Ok(())
    }
}

impl NativeManifestLock {
    pub(crate) fn acquire(state_id: &str) -> Result<Self> {
        let file = crate::portability::manifest_lock_file(state_id)?;
        file.lock_exclusive()?;
        Ok(Self { file })
    }

    pub(crate) fn try_acquire(state_id: &str) -> Result<Option<Self>> {
        let file = crate::portability::manifest_lock_file(state_id)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn release(self) -> Result<()> {
        self.file.unlock()?;
        Ok(())
    }
}

impl DatabaseLock {
    pub(crate) fn acquire(root: &std::path::Path, database_id: &[u8; 16]) -> Result<Self> {
        let file = open_database_lock(root, database_id)?;
        file.lock_exclusive()?;
        Ok(Self { file })
    }

    pub(crate) fn try_acquire(
        root: &std::path::Path,
        database_id: &[u8; 16],
    ) -> Result<Option<Self>> {
        let file = open_database_lock(root, database_id)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn release(self) -> Result<()> {
        self.file.unlock()?;
        Ok(())
    }
}

fn open_lock(paths: &ProfilePaths, name: &str) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(paths.directory.join(name)).map_err(Into::into)
}

fn open_database_lock(root: &std::path::Path, database_id: &[u8; 16]) -> Result<File> {
    let directory = crate::prepare_private_directory(&root.join(DATABASE_LOCK_DIRECTORY))?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(directory.join(format!("{}.lock", crate::hex(database_id))))
        .map_err(Into::into)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobRunReport {
    pub runs: Vec<JobRun>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobRun {
    pub job_id_hex: String,
    pub completed: bool,
    pub error: Option<String>,
    /// Set when execution is deferred pending manual user intervention, such as
    /// unlocking a hardware security key. The job is neither failed nor backed off;
    /// the description specifies the required user action.
    pub deferred: Option<String>,
    pub next_run_at: u64,
}

pub(super) enum TeamRefreshAttempt {
    Complete,
    Refreshed,
    TryStrongerCredential,
}

pub(super) type TeamRefreshPartyKey = (Vec<u8>, Option<Vec<u8>>);

#[derive(Clone)]
pub(super) enum TeamRefreshParty {
    User(foks_verify::VerifiedUserState),
    Team(foks_client::VerifiedTeamRecipient),
}

impl TeamRefreshParty {
    pub(super) fn verified(&self) -> VerifiedMemberParty<'_> {
        match self {
            Self::User(user) => VerifiedMemberParty::User(user),
            Self::Team(team) => VerifiedMemberParty::Team(team),
        }
    }

    pub(super) fn shared_key(
        &self,
        role: foks_proto::Role,
    ) -> Option<&foks_verify::VerifiedSharedKey> {
        match self {
            Self::User(user) => user.shared_key(role),
            Self::Team(team) => team.verified().shared_key(role),
        }
    }

    fn has_stale_shared_key(&self, role: foks_proto::Role) -> bool {
        match self {
            Self::User(user) => user.stale_shared_key_roles().contains(&role),
            Self::Team(team) => !team.is_recursively_authenticated(),
        }
    }

    fn is_recursively_authenticated(&self) -> bool {
        match self {
            Self::User(_) => true,
            Self::Team(team) => team.is_recursively_authenticated(),
        }
    }

    fn matches_local_root(
        &self,
        local_host: &foks_proto::EntityId,
        root: &foks_proto::TreeRoot,
    ) -> bool {
        match self {
            Self::User(user) => user.host() != local_host || user.tree_root() == *root,
            Self::Team(team) => {
                team.verified().host() != local_host || team.verified().tree_root() == *root
            }
        }
    }
}

fn team_refresh_party_key(
    party: &foks_proto::EntityId,
    scoped_host: Option<&foks_proto::EntityId>,
) -> TeamRefreshPartyKey {
    (
        party.as_bytes().to_vec(),
        scoped_host.map(|host| host.as_bytes().to_vec()),
    )
}

fn transport_can_observe_team_rekey(
    team: &foks_verify::VerifiedTeamState,
    transport: &foks_proto::EntityId,
) -> bool {
    team.members().iter().any(|member| {
        member.party == *transport
            && member.scoped_host.is_none()
            && matches!(
                member.role.kind(),
                foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
            )
    })
}

pub(super) fn local_team_can_observe_team_rekey(
    target: &foks_verify::VerifiedTeamState,
    actor: &AuthenticatedTeamOutcome,
) -> bool {
    target.members().iter().any(|member| {
        member.party == *actor.verified.team()
            && member.scoped_host.is_none()
            && matches!(
                member.role.kind(),
                foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
            )
            && actor.holds_roster_private_key(member)
    })
}

#[derive(Clone, Copy)]
pub(super) enum TeamRefreshCredential<'a, 'device> {
    Software(&'a DeviceCredential),
    Yubi(&'a YubiCredential<'device>),
}

impl TeamRefreshCredential<'_, '_> {
    fn uid(&self) -> &foks_proto::EntityId {
        match self {
            Self::Software(credential) => &credential.uid,
            Self::Yubi(credential) => &credential.uid,
        }
    }

    fn device_id(&self) -> Result<foks_proto::EntityId> {
        match self {
            Self::Software(credential) => Ok(credential.public_material()?.id),
            Self::Yubi(credential) => Ok(credential.parent.entity_id().clone()),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum TeamRefreshActor<'a> {
    User(&'a AuthenticatedUserOutcome),
    LocalTeam {
        transport_user: &'a AuthenticatedUserOutcome,
        authenticated: &'a AuthenticatedTeamOutcome,
        recipient: Option<&'a VerifiedTeamRecipient>,
    },
}

struct LocalTeamRefreshTarget<'a> {
    team_alias: &'a str,
    team_id: &'a foks_proto::EntityId,
    actor: TeamRefreshActor<'a>,
    team: &'a AuthenticatedTeamOutcome,
    supplied_parties: &'a std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>,
}

struct StoredTeamRekeyTarget<'a> {
    actor: TeamRefreshActor<'a>,
    team_id: &'a foks_proto::EntityId,
    team: &'a AuthenticatedTeamOutcome,
    party_states: &'a std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>,
    pending: &'a StoredTeamRekey,
}

impl TeamRefreshActor<'_> {
    fn party(&self) -> &foks_proto::EntityId {
        match self {
            Self::User(user) => user.verified.uid(),
            Self::LocalTeam { authenticated, .. } => authenticated.verified.team(),
        }
    }

    fn party_state(&self) -> Option<TeamRefreshParty> {
        match self {
            Self::User(user) => Some(TeamRefreshParty::User(user.verified.clone())),
            Self::LocalTeam { recipient, .. } => {
                recipient.map(|recipient| TeamRefreshParty::Team(recipient.clone()))
            }
        }
    }

    fn local_team(
        &self,
    ) -> Option<(
        &AuthenticatedUserOutcome,
        &AuthenticatedTeamOutcome,
        Option<&VerifiedTeamRecipient>,
    )> {
        match self {
            Self::User(_) => None,
            Self::LocalTeam {
                transport_user,
                authenticated,
                recipient,
            } => Some((transport_user, authenticated, *recipient)),
        }
    }
}

fn team_refresh_absence_is_deferred(has_pending_rekey: bool, status: u64) -> bool {
    !has_pending_rekey
        && matches!(
            status,
            foks_rpc::STATUS_PERMISSION_ERROR | foks_rpc::STATUS_TEAM_NOT_FOUND_ERROR
        )
}

fn team_refresh_has_no_authorized_work(direct_admin: bool, has_pending_rekey: bool) -> bool {
    !direct_admin && !has_pending_rekey
}

fn team_recipient_staleness_is_deferred(error: &foks_client::Error) -> bool {
    matches!(
        error,
        foks_client::Error::TeamBinding(
            "team recipient roster key is not current"
                | "team recipient has a stale direct or descendant key"
        )
    )
}

/// Expands a blocked set upward through `(child, parent)` membership edges.
/// A team whose caller-durable CLKR another unlocked device must recover
/// cannot be rotated yet, and neither can any team that depends on it, or the
/// parent would be rekeyed against a roster that is about to change again.
fn exclude_team_ancestors(
    mut blocked: std::collections::BTreeSet<Vec<u8>>,
    edges: &[(foks_proto::EntityId, foks_proto::EntityId)],
) -> std::collections::BTreeSet<Vec<u8>> {
    loop {
        let mut changed = false;
        for (child, parent) in edges {
            if blocked.contains(child.as_bytes()) {
                changed |= blocked.insert(parent.as_bytes().to_vec());
            }
        }
        if !changed {
            return blocked;
        }
    }
}

impl CheckedProfileSession<'_> {
    fn team_refresh_alias(
        &self,
        vault: &mut AccountVault<'_>,
        team: &foks_proto::EntityId,
    ) -> Result<String> {
        if let Some((alias, _)) = vault.team_rekey_for_team(team.as_bytes())? {
            return Ok(alias);
        }

        let stored_aliases = vault
            .team_aliases()?
            .into_iter()
            .filter_map(|alias| match vault.team(&alias) {
                Ok(stored) if stored.team_id == team.as_bytes() => Some(Ok(alias)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>>>()?;
        match stored_aliases.as_slice() {
            [alias] => return Ok(alias.clone()),
            [] => {}
            _ => {
                return Err(Error::InvalidAccount(
                    "more than one stored alias names the same team",
                ))
            }
        }
        // Teams created or joined by another implementation do not have a
        // local display alias. Their caller-durable CLKR state still needs a
        // stable vault key across crashes and restarts.
        let mut occupied = vault
            .team_aliases()?
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        occupied.extend(vault.team_rekey_aliases()?);
        for nonce in 0_u32..=u32::MAX {
            let mut input = team.as_bytes().to_vec();
            input.extend_from_slice(&nonce.to_be_bytes());
            let digest = foks_crypto::prefixed_hash(TEAM_REFRESH_ALIAS_TYPE_ID, &input);
            let encoded = hex(&digest);
            let alias = format!(
                "{}{}",
                super::team::BACKGROUND_TEAM_ALIAS_PREFIX,
                &encoded[..60]
            );
            if !occupied.contains(&alias) {
                return Ok(alias);
            }
        }
        Err(Error::InvalidAccount(
            "background team alias namespace is exhausted",
        ))
    }

    fn authenticate_team_refresh_transport(
        &self,
        host: &foks_client::PinnedHost,
        credential: TeamRefreshCredential<'_, '_>,
    ) -> Result<AuthenticatedUserOutcome> {
        match credential {
            TeamRefreshCredential::Software(credential) => {
                Ok(self.client.authenticate_and_pin(host, credential)?)
            }
            TeamRefreshCredential::Yubi(credential) => {
                Ok(self.client.authenticate_yubi_and_pin(host, credential)?)
            }
        }
    }

    fn discover_team_refresh_graph(
        &self,
        host: &foks_client::PinnedHost,
        credential: TeamRefreshCredential<'_, '_>,
        user: &AuthenticatedUserOutcome,
    ) -> Result<foks_client::AuthenticatedLocalTeamGraph> {
        match credential {
            TeamRefreshCredential::Software(credential) => Ok(self
                .client
                .discover_local_team_graph(host, credential, &user.verified, &user.puks)?),
            TeamRefreshCredential::Yubi(credential) => Ok(self
                .client
                .discover_local_team_graph_yubi(host, credential, &user.verified, &user.puks)?),
        }
    }

    pub(super) fn load_team_refresh_user(
        &self,
        host: &foks_client::PinnedHost,
        credential: TeamRefreshCredential<'_, '_>,
        user: &foks_proto::EntityId,
        view_token: &[u8; 16],
    ) -> Result<foks_verify::VerifiedUserState> {
        match credential {
            TeamRefreshCredential::Software(credential) => Ok(self
                .client
                .load_and_pin_user_as_local_team(host, credential, user, view_token)?),
            TeamRefreshCredential::Yubi(credential) => Ok(self
                .client
                .load_and_pin_user_as_local_team_yubi(host, credential, user, view_token)?),
        }
    }

    /// Sweeps the authenticated local membership DAG child-before-parent.
    /// Any mutation advances the Merkle tree, so the whole graph is
    /// rediscovered before another PTK is emitted; this prevents mixing
    /// recipient projections authenticated at different heads.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn refresh_authenticated_team_graph(
        &self,
        host: &foks_client::PinnedHost,
        credential: TeamRefreshCredential<'_, '_>,
        supplied_parties_by_team: &std::collections::BTreeMap<
            Vec<u8>,
            std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>,
        >,
        scope: Option<&std::collections::BTreeSet<Vec<u8>>>,
        excluded: Option<&std::collections::BTreeSet<Vec<u8>>>,
        vault: &mut AccountVault<'_>,
        protected_store: &mut EncryptedFileMutationStore,
    ) -> Result<bool> {
        self.cleanup_inactive_team_rekeys(host, vault, protected_store)?;
        for _ in 0..256 {
            let user = self.authenticate_team_refresh_transport(host, credential)?;
            let graph = self.discover_team_refresh_graph(host, credential, &user)?;
            let blocked =
                exclude_team_ancestors(excluded.cloned().unwrap_or_default(), &graph.edges);
            if graph.child_first.is_empty()
                || scope.is_some_and(|scope| {
                    !graph
                        .child_first
                        .iter()
                        .any(|team| scope.contains(team.as_bytes()))
                })
            {
                // The return value reports whether the authenticated graph
                // covered the caller's team surface. Terminal cleanup is
                // deliberately independent and must not suppress the legacy
                // direct-alias fallback when no graph teams were found.
                return Ok(false);
            }
            let mut teams = graph
                .teams
                .into_iter()
                .map(|team| (team.verified.team().as_bytes().to_vec(), team))
                .collect::<std::collections::BTreeMap<_, _>>();
            let mut recipients =
                std::collections::BTreeMap::<Vec<u8>, VerifiedTeamRecipient>::new();
            let mut restarted = false;

            for team_id in graph.child_first {
                let team_key = team_id.as_bytes().to_vec();
                if blocked.contains(&team_key)
                    || scope.is_some_and(|scope| !scope.contains(&team_key))
                {
                    continue;
                }
                let team = teams
                    .remove(&team_key)
                    .ok_or(foks_client::Error::TeamBinding(
                        "authenticated membership graph lost a target team",
                    ))?;
                let alias = self.team_refresh_alias(vault, &team_id)?;
                let pending = vault.team_rekey(&alias)?;
                if let Some(pending) = pending
                    .as_ref()
                    .filter(|pending| team.verified.chain_seqno() >= pending.expected_seqno)
                {
                    if pending.transport_uid != credential.uid().as_bytes() {
                        return Err(foks_client::Error::CredentialBinding(
                            "caller-durable CLKR transport principal changed",
                        )
                        .into());
                    }
                    let actor_id = foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?;
                    let actor = if actor_id == *user.verified.uid()
                        && transport_can_observe_team_rekey(&team.verified, user.verified.uid())
                    {
                        TeamRefreshActor::User(&user)
                    } else if let Some(actor_team) = teams
                        .get(actor_id.as_bytes())
                        .filter(|actor| local_team_can_observe_team_rekey(&team.verified, actor))
                    {
                        TeamRefreshActor::LocalTeam {
                            transport_user: &user,
                            authenticated: actor_team,
                            // Reconciliation authenticates the exact visible
                            // transition and retained PTKs; it does not mint a
                            // new recipient box or signature.
                            recipient: recipients.get(actor_id.as_bytes()),
                        }
                    } else if transport_can_observe_team_rekey(&team.verified, user.verified.uid())
                    {
                        // The recorded nested actor may have become
                        // unreachable after submission. A current direct
                        // administrator can still authenticate whether that
                        // exact retained transition committed or conflicted;
                        // it cannot replay a pre-sequence request.
                        TeamRefreshActor::User(&user)
                    } else if let Some((actor_id, actor_team)) = team
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
                        })
                        .filter_map(|member| {
                            teams
                                .get(member.party.as_bytes())
                                .filter(|actor| actor.holds_roster_private_key(member))
                                .map(|actor| (member, actor))
                        })
                        .max_by_key(|(member, _)| (member.role, member.source_role))
                        .map(|(member, actor)| (member.party.clone(), actor))
                    {
                        TeamRefreshActor::LocalTeam {
                            transport_user: &user,
                            authenticated: actor_team,
                            recipient: recipients.get(actor_id.as_bytes()),
                        }
                    } else {
                        return Err(foks_client::Error::TeamBinding(
                            "visible CLKR has neither its recorded actor nor a current administrator observer",
                        )
                        .into());
                    };
                    match self.refresh_local_team_member_keys(
                        host,
                        credential,
                        LocalTeamRefreshTarget {
                            team_alias: &alias,
                            team_id: &team_id,
                            actor,
                            team: &team,
                            supplied_parties: &std::collections::BTreeMap::new(),
                        },
                        vault,
                        protected_store,
                    )? {
                        TeamRefreshAttempt::Complete | TeamRefreshAttempt::Refreshed => {
                            restarted = true;
                            break;
                        }
                        TeamRefreshAttempt::TryStrongerCredential => {
                            return Err(foks_client::Error::TeamBinding(
                                "visible CLKR cannot be reconciled by its recorded actor",
                            )
                            .into())
                        }
                    }
                }
                let mut parties = std::collections::BTreeMap::new();
                let mut unavailable = false;
                for member in team.verified.members() {
                    let key = team_refresh_party_key(&member.party, member.scoped_host.as_ref());
                    let party = if member.scoped_host.is_some() {
                        let Some(party) = supplied_parties_by_team
                            .get(&team_key)
                            .and_then(|parties| parties.get(&key))
                            .cloned()
                        else {
                            let paired = vault.contains_team(&alias)?
                                && vault.team(&alias)?.federated_members.iter().any(|binding| {
                                    binding.active
                                        && binding.remote_team_id == member.party.as_bytes()
                                        && binding.remote_host_id
                                            == member
                                                .scoped_host
                                                .as_ref()
                                                .expect("remote roster row has a host")
                                                .as_bytes()
                                });
                            if !paired {
                                return Err(Error::BackgroundRefresh(format!(
                                    "team {} has an authenticated remote roster member without a protected federation binding",
                                    hex(team_id.as_bytes()),
                                )));
                            }
                            // Paired federation jobs supply authoritative
                            // remote projections. The ordinary local sweep
                            // defers rather than minting boxes for an
                            // unauthenticated remote view.
                            unavailable = true;
                            continue;
                        };
                        party
                    } else {
                        match member.party.entity_type() {
                            foks_proto::ENTITY_USER if member.party == *user.verified.uid() => {
                                TeamRefreshParty::User(user.verified.clone())
                            }
                            foks_proto::ENTITY_USER => {
                                TeamRefreshParty::User(self.load_team_refresh_user(
                                    host,
                                    credential,
                                    &member.party,
                                    &team.view_token,
                                )?)
                            }
                            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM => {
                                let recipient = match recipients.get(member.party.as_bytes()) {
                                    Some(recipient) => recipient.clone(),
                                    None => match credential {
                                        TeamRefreshCredential::Software(credential) => {
                                            self.client.load_local_child_team_recipient(
                                                host,
                                                credential,
                                                &team,
                                                &member.party,
                                            )?
                                        }
                                        TeamRefreshCredential::Yubi(credential) => {
                                            self.client.load_local_child_team_recipient_yubi(
                                                host,
                                                credential,
                                                &team,
                                                &member.party,
                                            )?
                                        }
                                    },
                                };
                                recipients
                                    .entry(member.party.as_bytes().to_vec())
                                    .or_insert_with(|| recipient.clone());
                                TeamRefreshParty::Team(recipient)
                            }
                            _ => {
                                return Err(foks_client::Error::TeamBinding(
                                    "authenticated team roster has an unsupported party type",
                                )
                                .into())
                            }
                        }
                    };
                    // One authenticated party projection supplies every
                    // source-role row for the same scoped roster identity.
                    parties.entry(key).or_insert(party);
                }
                if unavailable {
                    // This team cannot become a recursively verified actor,
                    // and no parent depending on it can safely rotate either.
                    teams.insert(team_key, team);
                    continue;
                }

                let selected_actor = if let Some(pending) = pending.as_ref() {
                    if pending.transport_uid != credential.uid().as_bytes() {
                        return Err(foks_client::Error::CredentialBinding(
                            "caller-durable CLKR transport principal changed",
                        )
                        .into());
                    }
                    foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?
                } else if team.verified.members().iter().any(|member| {
                    member.party == *user.verified.uid()
                        && member.scoped_host.is_none()
                        && matches!(
                            member.role.kind(),
                            foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
                        )
                }) {
                    user.verified.uid().clone()
                } else if let Some(member) = team
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
                            && recipients.contains_key(member.party.as_bytes())
                            && teams
                                .get(member.party.as_bytes())
                                .is_some_and(|actor| actor.holds_roster_private_key(member))
                    })
                    .max_by_key(|member| (member.role, member.source_role))
                {
                    member.party.clone()
                } else {
                    let direct = parties
                        .values()
                        .map(TeamRefreshParty::verified)
                        .collect::<Vec<_>>();
                    match team.verified_recipient(host, &direct) {
                        Ok(recipient) => {
                            recipients.insert(team_key.clone(), recipient);
                        }
                        Err(error) if team_recipient_staleness_is_deferred(&error) => {
                            // A stale PUK is an expected deferred state. This
                            // team cannot safely act for a parent yet.
                        }
                        Err(error) => return Err(error.into()),
                    }
                    teams.insert(team_key, team);
                    continue;
                };

                let attempt = if selected_actor == *user.verified.uid() {
                    self.refresh_local_team_member_keys(
                        host,
                        credential,
                        LocalTeamRefreshTarget {
                            team_alias: &alias,
                            team_id: &team_id,
                            actor: TeamRefreshActor::User(&user),
                            team: &team,
                            supplied_parties: &parties,
                        },
                        vault,
                        protected_store,
                    )?
                } else {
                    let actor_team =
                        teams
                            .get(selected_actor.as_bytes())
                            .ok_or(foks_client::Error::TeamBinding(
                            "caller-durable CLKR local-team actor is outside the membership graph",
                        ))?;
                    let actor_recipient = recipients
                        .get(selected_actor.as_bytes())
                        .ok_or(foks_client::Error::TeamBinding(
                        "caller-durable CLKR local-team actor lacks a current recipient witness",
                    ))?;
                    self.refresh_local_team_member_keys(
                        host,
                        credential,
                        LocalTeamRefreshTarget {
                            team_alias: &alias,
                            team_id: &team_id,
                            actor: TeamRefreshActor::LocalTeam {
                                transport_user: &user,
                                authenticated: actor_team,
                                recipient: Some(actor_recipient),
                            },
                            team: &team,
                            supplied_parties: &parties,
                        },
                        vault,
                        protected_store,
                    )?
                };
                match attempt {
                    TeamRefreshAttempt::Refreshed => {
                        restarted = true;
                        break;
                    }
                    TeamRefreshAttempt::TryStrongerCredential => {
                        return Err(foks_client::Error::TeamBinding(
                            "authenticated membership graph selected an unauthorized CLKR actor",
                        )
                        .into())
                    }
                    TeamRefreshAttempt::Complete => {
                        // No chain or Merkle state changed. Retain the
                        // already-authenticated target so a no-op sweep stays
                        // linear in the number of teams.
                        let direct = parties
                            .values()
                            .map(TeamRefreshParty::verified)
                            .collect::<Vec<_>>();
                        match team.verified_recipient(host, &direct) {
                            Ok(recipient) => {
                                recipients.insert(team_key.clone(), recipient);
                            }
                            Err(error) if team_recipient_staleness_is_deferred(&error) => {
                                // Keep parents depending on this team deferred
                                // until its direct PUK responder catches up.
                            }
                            Err(error) => return Err(error.into()),
                        }
                        teams.insert(team_key, team);
                    }
                }
            }
            if !restarted {
                return Ok(true);
            }
        }
        Err(Error::BackgroundRefresh(
            "team-key refresh did not converge after 256 authenticated mutations".to_owned(),
        ))
    }

    pub(super) fn cleanup_inactive_team_rekeys(
        &self,
        host: &foks_client::PinnedHost,
        vault: &mut AccountVault<'_>,
        protected_store: &mut EncryptedFileMutationStore,
    ) -> Result<()> {
        // Terminal and provably unsubmitted records are self-authenticating
        // cleanup work. Sweep them independently of current membership-graph
        // reachability or credential availability so a later removal,
        // demotion, or hardware lock cannot retain obsolete PTK seeds.
        for alias in vault.team_rekey_aliases()? {
            let Some(pending) = vault.team_rekey(&alias)? else {
                continue;
            };
            let team_id = foks_proto::EntityId::from_bytes(pending.team_id.clone())?;
            self.cleanup_inactive_team_rekey(
                host,
                &alias,
                &team_id,
                &pending,
                vault,
                protected_store,
            )?;
        }
        Ok(())
    }

    pub(super) fn register_default_refresh_jobs_for(
        &self,
        uid: &foks_proto::EntityId,
        now: u64,
    ) -> Result<()> {
        // Default registration is opportunistic. Probe-only profiles and
        // expired capability canaries must not abort an unrelated scheduler
        // batch (or the signup finalization path that calls this helper).
        match self.profile.require(Capability::UserSync) {
            Ok(()) => {}
            Err(Error::CapabilityDenied(Capability::UserSync)) => return Ok(()),
            Err(error) => return Err(error),
        }
        let host = self.pinned_host()?;
        let scheduler = FoksScheduler::new(&self.paths.hard_database, SchedulerConfig::default())?;
        register_default_refresh_job(
            &scheduler,
            USER_REFRESH_JOB_TYPE_ID,
            ScheduledJobKind::UserRefresh,
            host.host_id(),
            uid,
            DEFAULT_USER_REFRESH_INTERVAL_MICROS,
            now,
        )?;
        if self.profile.require(Capability::Teams).is_ok() {
            register_default_refresh_job(
                &scheduler,
                TEAM_REFRESH_JOB_TYPE_ID,
                ScheduledJobKind::TeamRefresh,
                host.host_id(),
                uid,
                DEFAULT_TEAM_REFRESH_INTERVAL_MICROS,
                now,
            )?;
        }
        Ok(())
    }

    fn ensure_default_refresh_jobs(&self, vault: &mut AccountVault<'_>, now: u64) -> Result<()> {
        let aliases = vault.aliases()?;
        let mut users = std::collections::BTreeSet::new();
        for alias in aliases {
            if vault.bot_selection(&alias)?.is_some() {
                continue;
            }
            users.insert(vault.account(&alias)?.credential.uid.into_bytes());
        }
        for alias in vault.yubi_aliases()? {
            if let Ok(account) = vault.yubi_account(&alias) {
                users.insert(account.uid.into_bytes());
            }
        }
        for uid in users.into_iter().map(foks_proto::EntityId::from_bytes) {
            let uid = uid?;
            self.register_default_refresh_jobs_for(&uid, now)?;
        }
        Ok(())
    }

    pub fn schedule_user_refresh(
        &self,
        alias: &str,
        interval_micros: u64,
        first_run_at: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<[u8; 16]> {
        self.profile.require(Capability::UserSync)?;
        if interval_micros == 0 {
            return Err(Error::InvalidConfig("job interval is zero"));
        }
        if vault.bot_selection(alias)?.is_some() {
            return Err(Error::InvalidAccount(
                "background security jobs require a permanent account credential",
            ));
        }
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let mut binding = Vec::with_capacity(66);
        binding.extend_from_slice(host.host_id().as_bytes());
        binding.extend_from_slice(loaded.credential.uid.as_bytes());
        let digest = foks_crypto::prefixed_hash(USER_REFRESH_JOB_TYPE_ID, &binding);
        let job_id = digest[..16]
            .try_into()
            .expect("hash prefix has fixed length");
        FoksScheduler::new(&self.paths.hard_database, SchedulerConfig::default())?.register(
            ScheduledJobRegistration {
                job_id,
                kind: ScheduledJobKind::UserRefresh,
                host_id: host.host_id().as_bytes().to_vec(),
                scope_id: loaded.credential.uid.as_bytes().to_vec(),
                interval_micros,
                first_run_at,
                registered_at: now_microseconds()?,
            },
        )?;
        Ok(job_id)
    }

    pub fn run_due_jobs(
        &self,
        now: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<JobRunReport> {
        self.profile.require(Capability::UserSync)?;
        // The scheduler lock prevents overlapping manual and periodic runs.
        // The rollback/operation lock remains a separate checked-session
        // concern on a separate inode.
        let _scheduler_lock = ProfileLock::scheduler(&self.paths)?;
        self.run_due_jobs_locked(now, vault, master_key)
    }

    /// Runs due work only if another process is not already scheduling this
    /// profile. Periodic schedulers use this to avoid blocking a worker.
    pub fn try_run_due_jobs(
        &self,
        now: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<Option<JobRunReport>> {
        self.profile.require(Capability::UserSync)?;
        let Some(_scheduler_lock) = ProfileLock::try_scheduler(&self.paths)? else {
            return Ok(None);
        };
        self.run_due_jobs_locked(now, vault, master_key).map(Some)
    }

    fn run_due_jobs_locked(
        &self,
        now: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<JobRunReport> {
        self.run_due_jobs_locked_with(now, vault, master_key, |_, _| {
            Err("federation reconciliation requires access to the remote profile".to_owned())
        })
    }

    pub(super) fn run_due_jobs_locked_with(
        &self,
        now: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
        mut federation: impl FnMut(
            &foks_client_db::ScheduledJob,
            &mut AccountVault<'_>,
        ) -> std::result::Result<Option<String>, String>,
    ) -> Result<JobRunReport> {
        self.ensure_default_refresh_jobs(vault, now)?;
        let scheduler = FoksScheduler::new(&self.paths.hard_database, SchedulerConfig::default())?;
        // Deferrals are reported alongside the scheduler's own outcome rather
        // than through it: a deferred run is a successful, non-backed-off run
        // that could not run because the required credential was unavailable.
        let mut deferrals = std::collections::BTreeMap::<[u8; 16], String>::new();
        let report = scheduler.run_due(now, |job| match job.kind {
            ScheduledJobKind::UserRefresh => {
                self.refresh_user_security(&job.scope_id, vault, master_key)
            }
            ScheduledJobKind::TeamRefresh => {
                self.refresh_team_chains(&job.scope_id, vault, master_key)
            }
            ScheduledJobKind::YubiManagementRefresh => {
                self.profile
                    .require(Capability::DeviceAdministration)
                    .map_err(|error| error.to_string())?;
                let scope: super::yubi::YubiRefreshScope = serde_json::from_slice(&job.scope_id)
                    .map_err(|_| "scheduled Yubi refresh scope is invalid".to_owned())?;
                self.refresh_yubi_management_envelope(
                    &scope.yubi_alias,
                    &scope.software_alias,
                    vault,
                )
                .map_err(|error| error.to_string())
            }
            ScheduledJobKind::MutationReconcile => {
                Err("mutation reconciliation is driven by explicit resume flows".to_owned())
            }
            ScheduledJobKind::FederationRefresh => match federation(job, vault) {
                Ok(None) => Ok(()),
                Ok(Some(reason)) => {
                    deferrals.insert(job.job_id, reason);
                    Ok(())
                }
                Err(error) => Err(error),
            },
        })?;
        Ok(JobRunReport {
            runs: report
                .runs
                .into_iter()
                .map(|run| {
                    let (completed, error) = match run.status {
                        foks_client::ScheduledRunStatus::Completed => (true, None),
                        foks_client::ScheduledRunStatus::Failed { error } => (false, Some(error)),
                    };
                    JobRun {
                        job_id_hex: hex(&run.job_id),
                        completed,
                        error,
                        deferred: deferrals.remove(&run.job_id),
                        next_run_at: run.next_run_at,
                    }
                })
                .collect(),
        })
    }

    pub(super) fn refresh_user_security(
        &self,
        uid: &[u8],
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> std::result::Result<(), String> {
        let host = self.pinned_host().map_err(|error| error.to_string())?;
        let preferred_signer = HardStateStore::open(&self.paths.hard_database)
            .and_then(|store| store.pending_mutations(host.host_id().as_bytes()))
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|operation| {
                operation.kind == MutationKind::PukRotation && operation.scope_id == uid
            })
            .map(|operation| operation.subject_id);
        let mut aliases = vault.aliases().map_err(|error| error.to_string())?;
        prefer_pending_signer_alias(&mut aliases, vault, preferred_signer.as_deref());
        let mut authenticated_any = false;
        let mut stale_without_owner = false;
        let mut owner_attempted = false;
        let mut last_error = None;
        for alias in aliases {
            let account = match vault.account(&alias) {
                Ok(account) => account,
                Err(error) => {
                    last_error = Some(error.to_string());
                    continue;
                }
            };
            if account.credential.key_kind == foks_client::SoftwareKeyKind::BotToken {
                continue;
            }
            if account.credential.uid.as_bytes() != uid {
                continue;
            }
            match self.client.authenticate_and_pin(&host, &account.credential) {
                Ok(authenticated) => {
                    authenticated_any = true;
                    let signer = account
                        .credential
                        .public_material()
                        .map_err(|error| error.to_string())?;
                    let is_owner = authenticated.verified.devices().iter().any(|device| {
                        device.id == signer.id && device.role == foks_proto::Role::OWNER
                    });
                    if !is_owner {
                        stale_without_owner |=
                            !authenticated.verified.stale_shared_key_roles().is_empty();
                        continue;
                    }
                    owner_attempted = true;
                    match self.refresh_user_security_as_owner(
                        &host,
                        &account.credential,
                        authenticated,
                        master_key,
                    ) {
                        Ok(()) => return Ok(()),
                        Err(error) => last_error = Some(error),
                    }
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        if authenticated_any && !stale_without_owner && !owner_attempted {
            return Ok(());
        }
        if !owner_attempted {
            for alias in vault.yubi_aliases().map_err(|error| error.to_string())? {
                if vault
                    .yubi_account(&alias)
                    .is_ok_and(|account| account.uid.as_bytes() == uid)
                {
                    // Resident jobs never prompt for or retain a PIN. Treat
                    // hardware-presence work as deferred without growing the
                    // failure backoff, even if a lower-role software device
                    // authenticated but cannot perform the stale-key repair.
                    // The next explicit hardware unlock runs it.
                    return Ok(());
                }
            }
        }
        Err(if authenticated_any {
            last_error.unwrap_or_else(|| {
                "stale PUK rotation requires a locally available owner credential".to_owned()
            })
        } else if last_error.is_some() {
            last_error.unwrap_or_else(|| "all scheduled user credentials failed".to_owned())
        } else {
            "scheduled account credential is unavailable".to_owned()
        })
    }

    fn refresh_user_security_as_owner(
        &self,
        host: &foks_client::PinnedHost,
        credential: &DeviceCredential,
        authenticated: AuthenticatedUserOutcome,
        master_key: &[u8; 32],
    ) -> std::result::Result<(), String> {
        let pending = HardStateStore::open(&self.paths.hard_database)
            .and_then(|store| store.pending_mutations(host.host_id().as_bytes()))
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|operation| {
                operation.kind == MutationKind::PukRotation
                    && operation.scope_id == credential.uid.as_bytes()
            });
        let mut refreshed = authenticated;
        if let Some(operation) = pending {
            let operation_id = operation.operation_id;
            self.profile
                .require(Capability::DeviceAdministration)
                .map_err(|error| error.to_string())?;
            let mut mutations = EncryptedFileMutationStore::open(
                &self.paths.protected_mutations,
                derive_mutation_key(master_key),
            )
            .map_err(|error| error.to_string())?;
            if self
                .client
                .journaled_puk_rotation_requires_passphrase_capability(
                    host,
                    credential,
                    operation_id,
                    &mut mutations,
                )
                .map_err(|error| error.to_string())?
            {
                self.profile
                    .require(Capability::Passphrases)
                    .map_err(|error| error.to_string())?;
            }
            let signer = credential
                .public_material()
                .map_err(|error| error.to_string())?;
            refreshed = if operation.subject_id == signer.id.as_bytes() {
                self.client.resume_software_puk_rotation_from_journal(
                    host,
                    credential,
                    operation_id,
                    &mut mutations,
                )
            } else {
                self.client.reconcile_software_puk_rotation_from_journal(
                    host,
                    credential,
                    operation_id,
                    &mut mutations,
                )
            }
            .map_err(|error| error.to_string())?;
            if HardStateStore::open(&self.paths.hard_database)
                .and_then(|store| store.mutation(&operation_id))
                .map_err(|error| error.to_string())?
                .is_some_and(|operation| {
                    matches!(
                        operation.state,
                        MutationState::Prepared
                            | MutationState::Submitting
                            | MutationState::SubmissionUnknown
                            | MutationState::RemoteVerified
                    )
                })
            {
                return Err(
                    "journaled PUK rotation remains ambiguous; deferring a competing rotation"
                        .to_owned(),
                );
            }
        }
        if let Some(upper) = refreshed
            .verified
            .stale_shared_key_roles()
            .iter()
            .next_back()
            .copied()
        {
            let expected_version = refreshed
                .verified
                .chain_seqno()
                .checked_add(1)
                .ok_or_else(|| "user chain sequence overflow".to_owned())?;
            let chain_position_reserved = HardStateStore::open(&self.paths.hard_database)
                .and_then(|store| store.pending_mutations(host.host_id().as_bytes()))
                .map_err(|error| error.to_string())?
                .into_iter()
                .any(|operation| {
                    operation.scope_id == credential.uid.as_bytes()
                        && operation.expected_version == Some(expected_version)
                        && matches!(
                            operation.kind,
                            MutationKind::DeviceProvision
                                | MutationKind::DeviceRevoke
                                | MutationKind::PukRotation
                        )
                });
            if chain_position_reserved {
                return Err(
                    "an active user-chain mutation reserves the stale-PUK rotation position"
                        .to_owned(),
                );
            }
            self.profile
                .require(Capability::DeviceAdministration)
                .map_err(|error| error.to_string())?;
            let rotates_owner = refreshed
                .verified
                .shared_keys()
                .iter()
                .any(|key| key.role <= upper && key.role == foks_proto::Role::OWNER);
            let passphrase_settings = if rotates_owner {
                self.profile
                    .require(Capability::Passphrases)
                    .map_err(|error| error.to_string())?;
                let settings = self
                    .client
                    .authenticated_passphrase_settings(host, credential, &refreshed)
                    .map_err(|error| error.to_string())?;
                Some(settings)
            } else {
                None
            };
            let rotations = refreshed
                .verified
                .shared_keys()
                .iter()
                .filter(|key| key.role <= upper)
                .map(|key| {
                    let role_history = self
                        .client
                        .load_puks_for_role(host, credential, &refreshed.verified, key.role)
                        .map_err(|error| error.to_string())?;
                    let previous = role_history
                        .into_iter()
                        .find(|private| {
                            private.role == key.role && private.generation == key.generation
                        })
                        .ok_or_else(|| "current stale PUK material is unavailable".to_owned())?;
                    Ok(UserPukRotation {
                        role: key.role,
                        previous_generation: key.generation,
                        previous_seed: previous.seed,
                        new_seed: foks_proto::SecretSeed::new(
                            random_array().map_err(|error| error.to_string())?,
                        ),
                    })
                })
                .collect::<std::result::Result<Vec<_>, String>>()?;
            let no_passphrase = if rotates_owner {
                match passphrase_settings.flatten() {
                    Some(_) => None,
                    None => {
                        let attested = HardStateStore::open(&self.paths.hard_database)
                            .and_then(|store| {
                                store.user_has_no_passphrase_attestation(
                                    host.host_id().as_bytes(),
                                    credential.uid.as_bytes(),
                                )
                            })
                            .map_err(|error| error.to_string())?;
                        if !attested {
                            return Err(
                                "owner PUK rotation requires passphrase verification for an unlinked legacy account"
                                    .to_owned(),
                            );
                        }
                        Some(NoPassphraseConfigured)
                    }
                }
            } else {
                None
            };
            let mut mutations = EncryptedFileMutationStore::open(
                &self.paths.protected_mutations,
                derive_mutation_key(master_key),
            )
            .map_err(|error| error.to_string())?;
            refreshed = self
                .client
                .rotate_software_puks(host, credential, &rotations, no_passphrase, &mut mutations)
                .map_err(|error| error.to_string())?;
        }
        if self.profile.require(Capability::Passphrases).is_ok() {
            self.client
                .refresh_passphrase_for_current_puk(host, credential, &refreshed)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(super) fn refresh_team_chains(
        &self,
        uid: &[u8],
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> std::result::Result<(), String> {
        self.profile
            .require(Capability::Teams)
            .map_err(|error| error.to_string())?;
        let host = self.pinned_host().map_err(|error| error.to_string())?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )
        .map_err(|error| error.to_string())?;
        self.cleanup_inactive_team_rekeys(&host, vault, &mut mutations)
            .map_err(|error| error.to_string())?;
        let aliases = vault.aliases().map_err(|error| error.to_string())?;
        let mut credentials = Vec::new();
        for alias in aliases {
            let account = vault.account(&alias).map_err(|error| error.to_string())?;
            if account.credential.uid.as_bytes() == uid {
                credentials.push(account.credential);
            }
        }
        if credentials.is_empty() {
            let has_yubi = vault
                .yubi_aliases()
                .map_err(|error| error.to_string())?
                .into_iter()
                .any(|alias| {
                    vault
                        .yubi_account(&alias)
                        .is_ok_and(|account| account.uid.as_bytes() == uid)
                });
            if has_yubi {
                // Team CLKR for hardware-only accounts is driven by the next
                // explicit unlock; unattended scheduling remains PIN-free.
                return Ok(());
            }
            return Err("scheduled team credential is unavailable".to_owned());
        }
        let mut graph_errors = Vec::new();
        let mut graph_was_empty = true;
        for credential in &credentials {
            match self.refresh_authenticated_team_graph(
                &host,
                TeamRefreshCredential::Software(credential),
                &std::collections::BTreeMap::new(),
                None,
                None,
                vault,
                &mut mutations,
            ) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(error) => {
                    graph_was_empty = false;
                    graph_errors.push(error.to_string());
                }
            }
        }
        if !graph_was_empty {
            return Err(graph_errors.join("; "));
        }
        // Creation intents from predating clients can lack a membership
        // sidechain until their next successful edit. Preserve the direct
        // alias sweep only for that empty-graph recovery case.
        let team_aliases = vault.team_aliases().map_err(|error| error.to_string())?;
        let mut sweep_errors = Vec::new();
        for team_alias in team_aliases {
            let team_result = (|| -> std::result::Result<(), String> {
                let stored = vault.team(&team_alias).map_err(|error| error.to_string())?;
                if !stored.active {
                    return Ok(());
                }
                let team_id = foks_proto::EntityId::from_bytes(stored.team_id.clone())
                    .map_err(|error| error.to_string())?;
                let pending_signer = vault
                    .team_rekey(&team_alias)
                    .map_err(|error| error.to_string())?
                    .map(|pending| pending.actor_device_id.clone());
                let mut candidates = credentials.iter().collect::<Vec<_>>();
                candidates.sort_by_key(|credential| {
                    let matches = pending_signer.as_ref().is_some_and(|preferred| {
                        credential
                            .public_material()
                            .is_ok_and(|device| device.id.as_bytes() == preferred)
                    });
                    !matches
                });
                let mut load_errors = Vec::new();
                let mut absent_from_team = false;
                let mut insufficient_authority = false;
                'credentials: for credential in candidates {
                    let mut user = match self.client.authenticate_and_pin(&host, credential) {
                        Ok(user) => user,
                        Err(error) => {
                            load_errors
                                .push(format!("team refresh user authentication failed: {error}"));
                            continue;
                        }
                    };
                    let mut team = match self.client.load_and_pin_team(
                        &host,
                        credential,
                        &user.verified,
                        &user.puks,
                        &team_id,
                    ) {
                        Ok(team) => team,
                        Err(error) => {
                            if matches!(
                                &error,
                                foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
                                    code,
                                    ..
                                }) if team_refresh_absence_is_deferred(
                                    pending_signer.is_some(),
                                    *code,
                                )
                            ) {
                                absent_from_team = true;
                                continue;
                            }
                            load_errors.push(format!("team refresh chain load failed: {error}"));
                            continue;
                        }
                    };
                    let direct_admin = team.verified.members().iter().any(|member| {
                        member.party.as_bytes() == uid
                            && member.scoped_host.is_none()
                            && matches!(
                                member.role.kind(),
                                foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
                            )
                    });
                    if team_refresh_has_no_authorized_work(direct_admin, pending_signer.is_some()) {
                        return Ok(());
                    }
                    // Once a credential has loaded an authenticated team,
                    // planner/journal failures are team-wide rather than
                    // credential-specific. Do not mint another view token by
                    // repeating them for every local device.
                    for attempt in 0..3 {
                        match self.refresh_local_team_member_keys(
                            &host,
                            TeamRefreshCredential::Software(credential),
                            LocalTeamRefreshTarget {
                                team_alias: &team_alias,
                                team_id: &team_id,
                                actor: TeamRefreshActor::User(&user),
                                team: &team,
                                supplied_parties: &std::collections::BTreeMap::new(),
                            },
                            vault,
                            &mut mutations,
                        ) {
                            Ok(TeamRefreshAttempt::Complete | TeamRefreshAttempt::Refreshed) => {
                                return Ok(())
                            }
                            Ok(TeamRefreshAttempt::TryStrongerCredential) => {
                                insufficient_authority = true;
                                continue 'credentials;
                            }
                            Err(Error::Client(foks_client::Error::TeamRequest(
                                "CLKR snapshot does not match the latest authenticated Merkle root",
                            ))) if attempt < 2 => {
                                user = self
                                    .client
                                    .authenticate_and_pin(&host, credential)
                                    .map_err(|error| {
                                        format!("team refresh race reload failed: {error}")
                                    })?;
                                team = self
                                    .client
                                    .load_and_pin_team(
                                        &host,
                                        credential,
                                        &user.verified,
                                        &user.puks,
                                        &team_id,
                                    )
                                    .map_err(|error| {
                                        format!("team refresh race reload failed: {error}")
                                    })?;
                                std::thread::sleep(std::time::Duration::from_millis(25));
                            }
                            Err(error) => {
                                return Err(format!("team refresh CLKR failed: {error}"));
                            }
                        }
                    }
                    unreachable!("bounded team refresh loop always returns")
                }
                if load_errors.is_empty() && absent_from_team {
                    return Ok(());
                }
                if insufficient_authority {
                    return Err(
                        "no local credential can perform the required team-key refresh".to_owned(),
                    );
                }
                Err(if load_errors.is_empty() {
                    "all scheduled team credentials failed".to_owned()
                } else {
                    load_errors.join("; ")
                })
            })();
            match team_result {
                Ok(_) => {}
                Err(error) => sweep_errors.push(format!("team {team_alias}: {error}")),
            }
        }
        if sweep_errors.is_empty() {
            Ok(())
        } else {
            Err(sweep_errors.join("; "))
        }
    }

    /// Runs the same local-team CLKR planner and exact crash-recovery path as
    /// the unattended software scheduler while a Yubi credential is already
    /// unlocked. No PIN or hardware handle crosses this call.
    pub(super) fn refresh_yubi_team_chains(
        &self,
        host: &foks_client::PinnedHost,
        credential: &YubiCredential<'_>,
        mut actor: AuthenticatedUserOutcome,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<AuthenticatedUserOutcome> {
        if self.profile.require(Capability::Teams).is_err() {
            return Ok(actor);
        }
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        // A pending caller-durable CLKR that names a different device must be
        // recovered by exactly that device. Skipping such a team here (and,
        // through `exclude_team_ancestors`, everything that depends on it)
        // makes a multi-key sweep independent of the order the keys are
        // presented in, instead of wedging on whichever runs first.
        let device_id = credential.parent.entity_id();
        let mut excluded = std::collections::BTreeSet::new();
        for team_alias in vault.team_aliases()? {
            let stored = vault.team(&team_alias)?;
            if !stored.active {
                continue;
            }
            if vault
                .team_rekey(&team_alias)?
                .as_ref()
                .is_some_and(|pending| {
                    pending.transport_uid != credential.uid.as_bytes()
                        || pending.actor_device_id != device_id.as_bytes()
                })
            {
                excluded.insert(stored.team_id.clone());
            }
        }
        if self.refresh_authenticated_team_graph(
            host,
            TeamRefreshCredential::Yubi(credential),
            &std::collections::BTreeMap::new(),
            None,
            Some(&excluded),
            vault,
            &mut mutations,
        )? {
            return self
                .client
                .authenticate_yubi_and_pin(host, credential)
                .map_err(Into::into);
        }
        let mut sweep_errors = Vec::new();
        for team_alias in vault.team_aliases()? {
            let team_result = (|| -> Result<()> {
                let stored = vault.team(&team_alias)?;
                if !stored.active {
                    return Ok(());
                }
                let team_id = foks_proto::EntityId::from_bytes(stored.team_id.clone())?;
                let pending_rekey = vault.team_rekey(&team_alias)?;
                if pending_rekey.as_ref().is_some_and(|pending| {
                    pending.transport_uid != credential.uid.as_bytes()
                        || pending.actor_device_id != device_id.as_bytes()
                }) {
                    // Another unlocked device owns exact recovery here.
                    return Ok(());
                }
                let mut team = match self.client.load_and_pin_team_yubi(
                    host,
                    credential,
                    &actor.verified,
                    &actor.puks,
                    &team_id,
                ) {
                    Ok(team) => team,
                    Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
                        code, ..
                    })) if team_refresh_absence_is_deferred(pending_rekey.is_some(), code) => {
                        return Ok(())
                    }
                    Err(error) => return Err(error.into()),
                };
                let direct_admin = team.verified.members().iter().any(|member| {
                    member.party == credential.uid
                        && member.scoped_host.is_none()
                        && matches!(
                            member.role.kind(),
                            foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
                        )
                });
                if team_refresh_has_no_authorized_work(direct_admin, pending_rekey.is_some()) {
                    return Ok(());
                }
                for attempt in 0..3 {
                    match self.refresh_local_team_member_keys(
                        host,
                        TeamRefreshCredential::Yubi(credential),
                        LocalTeamRefreshTarget {
                            team_alias: &team_alias,
                            team_id: &team_id,
                            actor: TeamRefreshActor::User(&actor),
                            team: &team,
                            supplied_parties: &std::collections::BTreeMap::new(),
                        },
                        vault,
                        &mut mutations,
                    ) {
                        Ok(TeamRefreshAttempt::Complete | TeamRefreshAttempt::Refreshed) => {
                            return Ok(())
                        }
                        Ok(TeamRefreshAttempt::TryStrongerCredential) => {
                            return Err(Error::InvalidAccount(
                                "Yubi credential lacks authority for team-key refresh",
                            ))
                        }
                        Err(Error::Client(foks_client::Error::TeamRequest(
                            "CLKR snapshot does not match the latest authenticated Merkle root",
                        ))) if attempt < 2 => {
                            actor = self.client.authenticate_yubi_and_pin(host, credential)?;
                            team = self.client.load_and_pin_team_yubi(
                                host,
                                credential,
                                &actor.verified,
                                &actor.puks,
                                &team_id,
                            )?;
                            std::thread::sleep(std::time::Duration::from_millis(25));
                        }
                        Err(error) => return Err(error),
                    }
                }
                unreachable!("bounded Yubi team refresh loop always returns")
            })();
            if let Err(error) = team_result {
                sweep_errors.push(format!("team {team_alias}: {error}"));
            }
        }
        if !sweep_errors.is_empty() {
            return Err(Error::BackgroundRefresh(sweep_errors.join("; ")));
        }
        Ok(actor)
    }

    fn refresh_local_team_member_keys(
        &self,
        host: &foks_client::PinnedHost,
        credential: TeamRefreshCredential<'_, '_>,
        target: LocalTeamRefreshTarget<'_>,
        vault: &mut AccountVault<'_>,
        protected_store: &mut EncryptedFileMutationStore,
    ) -> Result<TeamRefreshAttempt> {
        let LocalTeamRefreshTarget {
            team_alias,
            team_id,
            actor,
            team,
            supplied_parties,
        } = target;
        if team_id.entity_type() == foks_proto::ENTITY_AD_HOC_TEAM {
            return Ok(TeamRefreshAttempt::Complete);
        }
        let actor_member = team
            .verified
            .members()
            .iter()
            .filter(|member| {
                member.party == *actor.party()
                    && member.scoped_host.is_none()
                    && matches!(
                        member.role.kind(),
                        foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
                    )
            })
            .max_by_key(|member| (member.role, member.source_role));

        let mut parties = supplied_parties.clone();
        if let Some(party) = actor.party_state() {
            parties.insert(team_refresh_party_key(actor.party(), None), party);
        }

        // Reconcile a visible pending sequence before applying the local-only
        // planning scope. A later transition may have added a remote or nested
        // member, but that must not strand an already committed CLKR intent.
        if let Some(pending) = vault.team_rekey(team_alias)? {
            if pending.team_id != team_id.as_bytes() {
                return Err(foks_client::Error::CredentialBinding(
                    "caller-durable CLKR intent belongs to another team",
                )
                .into());
            }
            let visible_current_observer = team.verified.chain_seqno() >= pending.expected_seqno
                && actor_member.is_some_and(|member| match actor {
                    TeamRefreshActor::User(_) => true,
                    TeamRefreshActor::LocalTeam { authenticated, .. } => {
                        authenticated.holds_roster_private_key(member)
                    }
                });
            if pending.transport_uid != credential.uid().as_bytes()
                || (pending.actor_uid != actor.party().as_bytes() && !visible_current_observer)
            {
                // Durable material is recoverable only with the same
                // authenticated transport principal and cryptographic actor.
                return Ok(TeamRefreshAttempt::TryStrongerCredential);
            }
            if self.cleanup_inactive_team_rekey(
                host,
                team_alias,
                team_id,
                &pending,
                vault,
                protected_store,
            )? {
                return Ok(TeamRefreshAttempt::Complete);
            }
            if team.verified.chain_seqno() >= pending.expected_seqno {
                let empty = std::collections::BTreeMap::new();
                match self.resume_stored_team_rekey(
                    host,
                    credential,
                    StoredTeamRekeyTarget {
                        actor,
                        team_id,
                        team,
                        party_states: &empty,
                        pending: &pending,
                    },
                    protected_store,
                ) {
                    Ok(outcome) => {
                        vault.remove_team_rekey(team_alias)?;
                        let _ = outcome;
                        return Ok(TeamRefreshAttempt::Refreshed);
                    }
                    Err(error) => {
                        let journal = HardStateStore::open(&self.paths.hard_database)?
                            .team_mutation(&pending.operation_id)?;
                        if let Some(operation) = journal.as_ref() {
                            if !team_mutation_matches_pending(
                                operation,
                                &pending,
                                host.host_id().as_bytes(),
                            ) {
                                return Err(foks_client::Error::OperationBinding(
                                    "caller-durable CLKR journal binding changed",
                                )
                                .into());
                            }
                            if matches!(
                                operation.state,
                                TeamMutationState::Rejected | TeamMutationState::Superseded
                            ) {
                                self.client.reject_recorded_team_rekey(
                                    host,
                                    team_id,
                                    pending.expected_seqno,
                                    &pending.operation_id,
                                    &foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?,
                                    protected_store,
                                )?;
                                vault.remove_team_rekey(team_alias)?;
                                return Ok(TeamRefreshAttempt::Complete);
                            }
                        } else {
                            self.client.discard_unjournaled_team_rekey_material(
                                host,
                                &pending.operation_id,
                                protected_store,
                            )?;
                            vault.remove_team_rekey(team_alias)?;
                            return Ok(TeamRefreshAttempt::Complete);
                        }
                        if actor_member.is_none() {
                            return Ok(TeamRefreshAttempt::TryStrongerCredential);
                        }
                        return Err(error);
                    }
                }
            }
        }
        if actor_member.is_none() {
            return Ok(TeamRefreshAttempt::TryStrongerCredential);
        }
        for member in team.verified.members() {
            let key = team_refresh_party_key(&member.party, member.scoped_host.as_ref());
            if parties.contains_key(&key) {
                continue;
            }
            if member.scoped_host.is_some() || member.party.entity_type() != foks_proto::ENTITY_USER
            {
                // Nested and remote parties are loaded by the graph/federation
                // orchestrator. Never rotate while a roster recipient lacks
                // an authenticated current-head projection.
                return Err(foks_client::Error::TeamRequest(
                    "CLKR requires an unmanaged nested or federated roster party",
                )
                .into());
            }
            {
                let verified = match credential {
                    TeamRefreshCredential::Software(credential) => {
                        self.client.load_and_pin_user_as_local_team(
                            host,
                            credential,
                            &member.party,
                            &team.view_token,
                        )?
                    }
                    TeamRefreshCredential::Yubi(credential) => {
                        self.client.load_and_pin_user_as_local_team_yubi(
                            host,
                            credential,
                            &member.party,
                            &team.view_token,
                        )?
                    }
                };
                parties.insert(key, TeamRefreshParty::User(verified));
            }
        }

        if let Some(mut pending) = vault.team_rekey(team_alias)? {
            if pending.team_id != team_id.as_bytes() {
                return Err(foks_client::Error::CredentialBinding(
                    "caller-durable CLKR intent belongs to another team",
                )
                .into());
            }
            if team.verified.chain_seqno().checked_add(1) != Some(pending.expected_seqno) {
                return Err(foks_client::Error::OperationBinding(
                    "caller-durable CLKR intent is not adjacent to the team head",
                )
                .into());
            }
            let hard_store = HardStateStore::open(&self.paths.hard_database)?;
            let operation = hard_store.team_mutation(&pending.operation_id)?;
            match operation {
                None => {
                    let occupant = hard_store.team_mutation_at(
                        host.host_id().as_bytes(),
                        team_id.as_bytes(),
                        pending.expected_seqno,
                    )?;
                    if occupant.as_ref().is_some_and(|operation| {
                        matches!(
                            operation.state,
                            TeamMutationState::Prepared
                                | TeamMutationState::Submitting
                                | TeamMutationState::SubmissionUnknown
                                | TeamMutationState::Submitted
                        )
                    }) {
                        return Err(foks_client::Error::OperationBinding(
                            "another active mutation occupies the pending CLKR sequence",
                        )
                        .into());
                    }
                    if occupant.is_some() {
                        drop(hard_store);
                        self.client.discard_unjournaled_team_rekey_material(
                            host,
                            &pending.operation_id,
                            protected_store,
                        )?;
                        vault.remove_team_rekey(team_alias)?;
                        return Ok(TeamRefreshAttempt::Complete);
                    }
                    if pending.transport_uid != credential.uid().as_bytes()
                        || pending.actor_uid != actor.party().as_bytes()
                    {
                        drop(hard_store);
                        return Ok(TeamRefreshAttempt::TryStrongerCredential);
                    }
                    if actor.party_state().is_none() {
                        // The request has protected material but no public
                        // journal row, so submission provably never began.
                        // Discarding it is safe and avoids stranding cleanup
                        // on a temporarily unavailable recursive witness.
                        drop(hard_store);
                        self.client.discard_unjournaled_team_rekey_material(
                            host,
                            &pending.operation_id,
                            protected_store,
                        )?;
                        vault.remove_team_rekey(team_alias)?;
                        return Ok(TeamRefreshAttempt::Complete);
                    }
                    // Protected request material is durably written before
                    // its public journal row. Resume that exact pre-journal
                    // request instead of abandoning the caller intent.
                    drop(hard_store);
                    let current_device = credential.device_id()?.into_bytes();
                    if pending.actor_device_id != current_device {
                        pending.actor_device_id = current_device;
                        vault.put_team_rekey(&pending)?;
                    }
                    let identity_matches = self
                        .stored_team_rekey_operation_id(actor, team_id, team, &parties, &pending)
                        .is_ok_and(|operation_id| operation_id == pending.operation_id);
                    if !identity_matches
                        || !stored_team_rekey_matches_current_plan(
                            team,
                            &parties,
                            actor_member,
                            &pending,
                        )
                    {
                        self.discard_stored_team_rekey(
                            host,
                            actor,
                            team_id,
                            team,
                            &pending,
                            protected_store,
                        )?;
                        vault.remove_team_rekey(team_alias)?;
                        // No journal row means submission could not have
                        // begun, so it is safe to build a fresh plan below.
                    } else {
                        let result = self.submit_stored_team_rekey(
                            host,
                            credential,
                            StoredTeamRekeyTarget {
                                actor,
                                team_id,
                                team,
                                party_states: &parties,
                                pending: &pending,
                            },
                            protected_store,
                        );
                        if result.is_ok() {
                            vault.remove_team_rekey(team_alias)?;
                        }
                        return result.map(|_| TeamRefreshAttempt::Refreshed);
                    }
                }
                Some(operation)
                    if !team_mutation_matches_pending(
                        &operation,
                        &pending,
                        host.host_id().as_bytes(),
                    ) =>
                {
                    return Err(foks_client::Error::OperationBinding(
                        "caller-durable CLKR journal binding changed",
                    )
                    .into());
                }
                Some(operation) if operation.state == TeamMutationState::Prepared => {
                    drop(hard_store);
                    self.client.reject_recorded_team_rekey(
                        host,
                        team_id,
                        pending.expected_seqno,
                        &pending.operation_id,
                        &foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?,
                        protected_store,
                    )?;
                    vault.remove_team_rekey(team_alias)?;
                }
                Some(operation) if operation.state == TeamMutationState::Verified => {
                    return Err(foks_client::Error::OperationBinding(
                        "verified CLKR journal is ahead of the authenticated team head",
                    )
                    .into());
                }
                Some(operation)
                    if matches!(
                        operation.state,
                        TeamMutationState::Rejected | TeamMutationState::Superseded
                    ) =>
                {
                    drop(hard_store);
                    self.client.reject_recorded_team_rekey(
                        host,
                        team_id,
                        pending.expected_seqno,
                        &pending.operation_id,
                        &foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?,
                        protected_store,
                    )?;
                    vault.remove_team_rekey(team_alias)?;
                }
                Some(_) => {
                    drop(hard_store);
                    if actor.party_state().is_none() {
                        // Ambiguous or submitted requests require the exact
                        // current actor-recipient witness before replay.
                        return Ok(TeamRefreshAttempt::TryStrongerCredential);
                    }
                    let snapshot_root = team.verified.tree_root();
                    let replay_safe = parties
                        .values()
                        .all(|party| party.matches_local_root(host.host_id(), &snapshot_root))
                        && team.verified.members().iter().all(|member| {
                            let changed = pending.changes.iter().find(|change| {
                                change.party == member.party.as_bytes()
                                    && change.host.as_deref()
                                        == member.scoped_host.as_ref().map(|host| host.as_bytes())
                                    && change.source_role.role().ok() == Some(member.source_role)
                            });
                            let receives_rotated_key = pending.rotations.iter().any(|rotation| {
                                rotation.role.role().is_ok_and(|role| role <= member.role)
                            });
                            parties
                                .get(&team_refresh_party_key(
                                    &member.party,
                                    member.scoped_host.as_ref(),
                                ))
                                .is_some_and(|party| {
                                    (!receives_rotated_key
                                        || !party.has_stale_shared_key(member.source_role))
                                        && party.shared_key(member.source_role).is_some_and(|key| {
                                            if let Some(change) = changed {
                                                change.generation == key.generation
                                                    && change.verify_key
                                                        == key.verify_key.as_bytes()
                                                    && foks_crypto::hepk_fingerprint(&key.hepk).ok()
                                                        == Some(change.hepk_fingerprint)
                                            } else {
                                                !receives_rotated_key
                                                    || (key.generation == member.generation
                                                        && key.verify_key == member.verify_key
                                                        && foks_crypto::hepk_fingerprint(&key.hepk)
                                                            .ok()
                                                            == Some(member.hepk_fingerprint))
                                            }
                                        })
                                })
                        });
                    let replay_error = if replay_safe
                        && pending.actor_uid == actor.party().as_bytes()
                    {
                        let current_parties = parties
                            .values()
                            .map(TeamRefreshParty::verified)
                            .collect::<Vec<_>>();
                        match (credential, actor.local_team()) {
                            (TeamRefreshCredential::Software(credential), None) => {
                                self.client.replay_recorded_team_rekey(
                                    host,
                                    credential,
                                    foks_client::TeamMutationRecovery {
                                        team: team_id,
                                        expected_seqno: pending.expected_seqno,
                                        expected_operation_id: &pending.operation_id,
                                    },
                                    team,
                                    &current_parties,
                                    protected_store,
                                )
                            }
                            (TeamRefreshCredential::Yubi(credential), None) => {
                                self.client.replay_recorded_team_rekey_yubi(
                                    host,
                                    credential,
                                    foks_client::TeamMutationRecovery {
                                        team: team_id,
                                        expected_seqno: pending.expected_seqno,
                                        expected_operation_id: &pending.operation_id,
                                    },
                                    team,
                                    &current_parties,
                                    protected_store,
                                )
                            }
                            (
                                TeamRefreshCredential::Software(credential),
                                Some((_, actor_team, _)),
                            ) => self.client.replay_recorded_team_rekey_as_local_team(
                                host,
                                credential,
                                actor_team,
                                team_id,
                                pending.expected_seqno,
                                &pending.operation_id,
                                team,
                                &current_parties,
                                protected_store,
                            ),
                            (TeamRefreshCredential::Yubi(credential), Some((_, actor_team, _))) => {
                                self.client.replay_recorded_team_rekey_as_local_team_yubi(
                                    host,
                                    credential,
                                    actor_team,
                                    team_id,
                                    pending.expected_seqno,
                                    &pending.operation_id,
                                    team,
                                    &current_parties,
                                    protected_store,
                                )
                            }
                        }
                        .err()
                    } else {
                        None
                    };
                    let user = match credential {
                        TeamRefreshCredential::Software(credential) => {
                            self.client.authenticate_and_pin(host, credential)?
                        }
                        TeamRefreshCredential::Yubi(credential) => {
                            self.client.authenticate_yubi_and_pin(host, credential)?
                        }
                    };
                    let observed = match (credential, actor.local_team()) {
                        (TeamRefreshCredential::Software(credential), None) => {
                            self.client.load_and_pin_team(
                                host,
                                credential,
                                &user.verified,
                                &user.puks,
                                team_id,
                            )?
                        }
                        (TeamRefreshCredential::Yubi(credential), None) => {
                            self.client.load_and_pin_team_yubi(
                                host,
                                credential,
                                &user.verified,
                                &user.puks,
                                team_id,
                            )?
                        }
                        (TeamRefreshCredential::Software(credential), Some((_, actor_team, _))) => {
                            self.client.load_and_pin_team_as_local_team(
                                host,
                                credential,
                                &user.verified,
                                actor_team,
                                team_id,
                            )?
                        }
                        (TeamRefreshCredential::Yubi(credential), Some((_, actor_team, _))) => {
                            self.client.load_and_pin_team_as_local_team_yubi(
                                host,
                                credential,
                                &user.verified,
                                actor_team,
                                team_id,
                            )?
                        }
                    };
                    if observed.verified.chain_seqno() >= pending.expected_seqno {
                        let empty = std::collections::BTreeMap::new();
                        let outcome = self.resume_stored_team_rekey(
                            host,
                            credential,
                            StoredTeamRekeyTarget {
                                actor,
                                team_id,
                                team: &observed,
                                party_states: &empty,
                                pending: &pending,
                            },
                            protected_store,
                        )?;
                        vault.remove_team_rekey(team_alias)?;
                        let _ = outcome;
                        return Ok(TeamRefreshAttempt::Refreshed);
                    }
                    if let Some(error) = replay_error {
                        let journal = HardStateStore::open(&self.paths.hard_database)?
                            .team_mutation(&pending.operation_id)?;
                        if journal.as_ref().is_some_and(|operation| {
                            team_mutation_matches_pending(
                                operation,
                                &pending,
                                host.host_id().as_bytes(),
                            ) && matches!(
                                operation.state,
                                TeamMutationState::Rejected | TeamMutationState::Superseded
                            )
                        }) {
                            vault.remove_team_rekey(team_alias)?;
                        }
                        return Err(error.into());
                    }
                    return Err(foks_client::Error::TransitionNotObserved(if replay_safe {
                        "replayed CLKR has not reached the authenticated team head"
                    } else {
                        "CLKR replay is deferred until its ambiguous sequence is visible"
                    })
                    .into());
                }
            }
        }

        let Some(actor_member) = actor_member else {
            // TeamRefresh also keeps non-admin team projections current. Any
            // caller-durable intent was reconciled or released above first.
            return Ok(TeamRefreshAttempt::TryStrongerCredential);
        };
        if actor.party_state().is_none() {
            // A current recursive recipient witness is mandatory before a
            // fresh parent CLKR can mint boxes or a signature.
            return Ok(TeamRefreshAttempt::TryStrongerCredential);
        }

        let mut stale = Vec::new();
        let mut stale_puk_recipients = Vec::new();
        let mut has_public_only_team_recipient = false;
        for member in team.verified.members() {
            let party = parties
                .get(&team_refresh_party_key(
                    &member.party,
                    member.scoped_host.as_ref(),
                ))
                .ok_or(foks_client::Error::TeamBinding(
                    "scheduled roster party was not authenticated",
                ))?;
            if !party.is_recursively_authenticated() {
                // A LocalParentTeam load authenticates the child's current
                // public PTK well enough to prove that a no-op parent is
                // current, but not to mint a new parent box. Defer only if
                // this pass actually finds CLKR work below.
                has_public_only_team_recipient = true;
            }
            let key =
                party
                    .shared_key(member.source_role)
                    .ok_or(foks_client::Error::TeamBinding(
                        "scheduled roster party lacks its source-role shared key",
                    ))?;
            if party.has_stale_shared_key(member.source_role) {
                stale_puk_recipients.push(member);
            }
            let fingerprint = foks_crypto::hepk_fingerprint(&key.hepk)?;
            if key.generation < member.generation
                || (key.generation == member.generation
                    && (key.verify_key != member.verify_key
                        || fingerprint != member.hepk_fingerprint))
            {
                return Err(foks_client::Error::TeamBinding(
                    "scheduled roster key conflicts with authenticated user state",
                )
                .into());
            }
            if key.generation > member.generation && member.role <= actor_member.role {
                stale.push((member, key));
            }
        }
        if has_public_only_team_recipient && !stale.is_empty() {
            // The public child might itself be stale or another row might
            // need repair. Either case rotates parent PTKs and therefore
            // requires the child's recursively authenticated recipient graph.
            return Ok(TeamRefreshAttempt::Complete);
        }
        if stale.is_empty() {
            // Higher-role stale rows are intentionally left for a suitably
            // privileged responder, matching Go CLKR behavior.
            return Ok(TeamRefreshAttempt::Complete);
        }
        let maximum_role = stale
            .iter()
            .map(|(member, _)| member.role)
            .max()
            .expect("stale set is nonempty");
        if stale_puk_recipients.iter().any(|member| {
            team.verified
                .shared_keys()
                .iter()
                .any(|key| key.role <= maximum_role && key.role <= member.role)
        }) {
            // A revoked device can still open this PUK. Wait for that user's
            // refresh responder to rotate it before generating any new PTKs.
            return Ok(TeamRefreshAttempt::Complete);
        }
        let expected_seqno = team
            .verified
            .chain_seqno()
            .checked_add(1)
            .ok_or(foks_client::Error::TeamRequest("team sequence overflow"))?;
        if HardStateStore::open(&self.paths.hard_database)?
            .team_mutation_at(
                host.host_id().as_bytes(),
                team_id.as_bytes(),
                expected_seqno,
            )?
            .is_some_and(|operation| {
                matches!(
                    operation.state,
                    TeamMutationState::Prepared
                        | TeamMutationState::Submitting
                        | TeamMutationState::SubmissionUnknown
                        | TeamMutationState::Submitted
                        | TeamMutationState::Verified
                )
            })
        {
            // Another caller already owns this chain position. Do not persist
            // a second secret plan that could later be confused with it.
            return Ok(TeamRefreshAttempt::Complete);
        }
        let mut pending = StoredTeamRekey {
            version: super::CREDENTIAL_VERSION,
            team_alias: team_alias.to_owned(),
            team_id: team_id.as_bytes().to_vec(),
            transport_uid: credential.uid().as_bytes().to_vec(),
            actor_uid: actor.party().as_bytes().to_vec(),
            actor_source_role: StoredTeamRole::from_role(actor_member.source_role),
            actor_generation: actor_member.generation,
            actor_device_id: credential.device_id()?.into_bytes(),
            operation_id: [0; 16],
            expected_seqno,
            changes: stale
                .iter()
                .map(|(member, replacement)| StoredTeamRekeyChange {
                    party: member.party.as_bytes().to_vec(),
                    host: member
                        .scoped_host
                        .as_ref()
                        .map(|host| host.as_bytes().to_vec()),
                    source_role: StoredTeamRole::from_role(member.source_role),
                    destination_role: StoredTeamRole::from_role(member.role),
                    generation: replacement.generation,
                    verify_key: replacement.verify_key.as_bytes().to_vec(),
                    hepk_fingerprint: foks_crypto::hepk_fingerprint(&replacement.hepk)
                        .expect("authenticated HEPK was already validated"),
                })
                .collect(),
            rotations: team
                .verified
                .shared_keys()
                .iter()
                .filter(|key| key.role <= maximum_role)
                .map(|key| {
                    Ok(StoredTeamPtkRotation {
                        role: StoredTeamRole::from_role(key.role),
                        seed: random_array()?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        };
        pending.operation_id =
            self.stored_team_rekey_operation_id(actor, team_id, team, &parties, &pending)?;
        vault.put_team_rekey(&pending)?;
        let result = self.submit_stored_team_rekey(
            host,
            credential,
            StoredTeamRekeyTarget {
                actor,
                team_id,
                team,
                party_states: &parties,
                pending: &pending,
            },
            protected_store,
        );
        if result.is_ok() {
            vault.remove_team_rekey(team_alias)?;
        }
        result.map(|_| TeamRefreshAttempt::Refreshed)
    }

    fn cleanup_inactive_team_rekey(
        &self,
        host: &foks_client::PinnedHost,
        team_alias: &str,
        team_id: &foks_proto::EntityId,
        pending: &StoredTeamRekey,
        vault: &mut AccountVault<'_>,
        protected_store: &mut EncryptedFileMutationStore,
    ) -> Result<bool> {
        if pending.team_id != team_id.as_bytes() {
            return Err(foks_client::Error::CredentialBinding(
                "caller-durable CLKR intent belongs to another team",
            )
            .into());
        }
        let hard_store = HardStateStore::open(&self.paths.hard_database)?;
        let operation = hard_store.team_mutation(&pending.operation_id)?;
        match operation {
            Some(operation) => {
                if !team_mutation_matches_pending(&operation, pending, host.host_id().as_bytes()) {
                    return Err(foks_client::Error::OperationBinding(
                        "caller-durable CLKR journal binding changed",
                    )
                    .into());
                }
                if operation.state == TeamMutationState::Verified {
                    drop(hard_store);
                    self.client.cleanup_verified_team_rekey_material(
                        host,
                        team_id,
                        pending.expected_seqno,
                        &pending.operation_id,
                        &foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?,
                        protected_store,
                    )?;
                    vault.remove_team_rekey(team_alias)?;
                    return Ok(true);
                }
                if !matches!(
                    operation.state,
                    TeamMutationState::Prepared
                        | TeamMutationState::Rejected
                        | TeamMutationState::Superseded
                ) {
                    return Ok(false);
                }
                drop(hard_store);
                self.client.reject_recorded_team_rekey(
                    host,
                    team_id,
                    pending.expected_seqno,
                    &pending.operation_id,
                    &foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?,
                    protected_store,
                )?;
            }
            None => {
                drop(hard_store);
                self.client.discard_unjournaled_team_rekey_material(
                    host,
                    &pending.operation_id,
                    protected_store,
                )?;
            }
        }
        vault.remove_team_rekey(team_alias)?;
        Ok(true)
    }

    fn submit_stored_team_rekey(
        &self,
        host: &foks_client::PinnedHost,
        credential: TeamRefreshCredential<'_, '_>,
        target: StoredTeamRekeyTarget<'_>,
        protected_store: &mut EncryptedFileMutationStore,
    ) -> Result<AuthenticatedTeamOutcome> {
        self.execute_stored_team_rekey(
            host,
            credential,
            target.actor,
            target.team_id,
            Some(target.team),
            target.party_states,
            target.pending,
            false,
            false,
            protected_store,
        )
    }

    fn stored_team_rekey_operation_id(
        &self,
        actor: TeamRefreshActor<'_>,
        team_id: &foks_proto::EntityId,
        team: &AuthenticatedTeamOutcome,
        party_states: &std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>,
        pending: &StoredTeamRekey,
    ) -> Result<[u8; 16]> {
        let party_ids = pending
            .changes
            .iter()
            .map(|change| foks_proto::EntityId::from_bytes(change.party.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let verify_keys = pending
            .changes
            .iter()
            .map(|change| foks_proto::EntityId::from_bytes(change.verify_key.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let hosts = pending
            .changes
            .iter()
            .map(|change| {
                change
                    .host
                    .as_deref()
                    .map(|bytes| foks_proto::EntityId::from_bytes(bytes.to_vec()))
                    .transpose()
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let seeds = pending
            .rotations
            .iter()
            .map(|rotation| foks_proto::SecretSeed::new(rotation.seed))
            .collect::<Vec<_>>();
        let rotations = pending
            .rotations
            .iter()
            .zip(&seeds)
            .map(|(rotation, seed)| {
                Ok(TeamPtkRotationSeed {
                    role: rotation.role.role()?,
                    seed,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let changes = pending
            .changes
            .iter()
            .zip(&party_ids)
            .zip(&verify_keys)
            .zip(&hosts)
            .map(|(((change, party), verify_key), scoped_host)| {
                Ok(TeamMemberKeyRefresh {
                    target: TeamMemberSelector {
                        party,
                        host: scoped_host.as_ref(),
                        source_role: change.source_role.role()?,
                    },
                    destination_role: change.destination_role.role()?,
                    replacement: Some(
                        party_states
                            .get(&(change.party.clone(), change.host.clone()))
                            .ok_or(foks_client::Error::TeamBinding(
                                "caller-durable CLKR target party is unavailable",
                            ))?
                            .verified(),
                    ),
                    replacement_generation: change.generation,
                    replacement_verify_key: verify_key,
                    replacement_hepk_fingerprint: change.hepk_fingerprint,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let request = RefreshTeamMemberKeysRequest {
            expected_seqno: pending.expected_seqno,
            changes: &changes,
            rotations: &rotations,
            remaining_parties: &[],
        };
        Ok(self
            .client
            .refresh_team_member_keys_operation_id_for_actor(
                actor.party(),
                team_id,
                team,
                &request,
            )?)
    }

    fn discard_stored_team_rekey(
        &self,
        host: &foks_client::PinnedHost,
        actor: TeamRefreshActor<'_>,
        team_id: &foks_proto::EntityId,
        team: &AuthenticatedTeamOutcome,
        pending: &StoredTeamRekey,
        protected_store: &mut EncryptedFileMutationStore,
    ) -> Result<()> {
        let party_ids = pending
            .changes
            .iter()
            .map(|change| foks_proto::EntityId::from_bytes(change.party.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let verify_keys = pending
            .changes
            .iter()
            .map(|change| foks_proto::EntityId::from_bytes(change.verify_key.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let hosts = pending
            .changes
            .iter()
            .map(|change| {
                change
                    .host
                    .as_deref()
                    .map(|bytes| foks_proto::EntityId::from_bytes(bytes.to_vec()))
                    .transpose()
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let seeds = pending
            .rotations
            .iter()
            .map(|rotation| foks_proto::SecretSeed::new(rotation.seed))
            .collect::<Vec<_>>();
        let rotations = pending
            .rotations
            .iter()
            .zip(&seeds)
            .map(|(rotation, seed)| {
                Ok(TeamPtkRotationSeed {
                    role: rotation.role.role()?,
                    seed,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let changes = pending
            .changes
            .iter()
            .zip(&party_ids)
            .zip(&verify_keys)
            .zip(&hosts)
            .map(|(((change, party), verify_key), scoped_host)| {
                Ok(TeamMemberKeyRefresh {
                    target: TeamMemberSelector {
                        party,
                        host: scoped_host.as_ref(),
                        source_role: change.source_role.role()?,
                    },
                    destination_role: change.destination_role.role()?,
                    replacement: None,
                    replacement_generation: change.generation,
                    replacement_verify_key: verify_key,
                    replacement_hepk_fingerprint: change.hepk_fingerprint,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let request = RefreshTeamMemberKeysRequest {
            expected_seqno: pending.expected_seqno,
            changes: &changes,
            rotations: &rotations,
            remaining_parties: &[],
        };
        self.client
            .discard_unrecorded_team_rekey_for_local_team_actor(
                host,
                actor.party(),
                team_id,
                team,
                &request,
                protected_store,
            )?;
        Ok(())
    }

    fn resume_stored_team_rekey(
        &self,
        host: &foks_client::PinnedHost,
        credential: TeamRefreshCredential<'_, '_>,
        target: StoredTeamRekeyTarget<'_>,
        protected_store: &mut EncryptedFileMutationStore,
    ) -> Result<AuthenticatedTeamOutcome> {
        let expected_actor = foks_proto::EntityId::from_bytes(target.pending.actor_uid.clone())?;
        let observe_as_transport = expected_actor != *credential.uid()
            && transport_can_observe_team_rekey(&target.team.verified, credential.uid());
        self.execute_stored_team_rekey(
            host,
            credential,
            target.actor,
            target.team_id,
            None,
            target.party_states,
            target.pending,
            true,
            observe_as_transport,
            protected_store,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_stored_team_rekey(
        &self,
        host: &foks_client::PinnedHost,
        credential: TeamRefreshCredential<'_, '_>,
        actor: TeamRefreshActor<'_>,
        team_id: &foks_proto::EntityId,
        team: Option<&AuthenticatedTeamOutcome>,
        party_states: &std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>,
        pending: &StoredTeamRekey,
        resume: bool,
        observe_as_transport: bool,
        protected_store: &mut EncryptedFileMutationStore,
    ) -> Result<AuthenticatedTeamOutcome> {
        let party_ids = pending
            .changes
            .iter()
            .map(|change| foks_proto::EntityId::from_bytes(change.party.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let verify_keys = pending
            .changes
            .iter()
            .map(|change| foks_proto::EntityId::from_bytes(change.verify_key.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let hosts = pending
            .changes
            .iter()
            .map(|change| {
                change
                    .host
                    .as_deref()
                    .map(|bytes| foks_proto::EntityId::from_bytes(bytes.to_vec()))
                    .transpose()
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let seeds = pending
            .rotations
            .iter()
            .map(|rotation| foks_proto::SecretSeed::new(rotation.seed))
            .collect::<Vec<_>>();
        let rotations = pending
            .rotations
            .iter()
            .zip(&seeds)
            .map(|(rotation, seed)| {
                Ok(TeamPtkRotationSeed {
                    role: rotation.role.role()?,
                    seed,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let changes = pending
            .changes
            .iter()
            .zip(&party_ids)
            .zip(&verify_keys)
            .zip(&hosts)
            .map(|(((change, party), verify_key), scoped_host)| {
                let replacement = if resume {
                    None
                } else {
                    Some(
                        party_states
                            .get(&(change.party.clone(), change.host.clone()))
                            .ok_or(foks_client::Error::TeamBinding(
                                "caller-durable CLKR target party is unavailable",
                            ))?
                            .verified(),
                    )
                };
                Ok(TeamMemberKeyRefresh {
                    target: TeamMemberSelector {
                        party,
                        host: scoped_host.as_ref(),
                        source_role: change.source_role.role()?,
                    },
                    destination_role: change.destination_role.role()?,
                    replacement,
                    replacement_generation: change.generation,
                    replacement_verify_key: verify_key,
                    replacement_hepk_fingerprint: change.hepk_fingerprint,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let changed = pending
            .changes
            .iter()
            .map(|change| (change.party.clone(), change.host.clone()))
            .collect::<std::collections::BTreeSet<_>>();
        let remaining = team
            .into_iter()
            .flat_map(|team| team.verified.members())
            .filter(|member| {
                member.party != *actor.party()
                    && !changed.contains(&team_refresh_party_key(
                        &member.party,
                        member.scoped_host.as_ref(),
                    ))
            })
            .map(|member| {
                party_states
                    .get(&team_refresh_party_key(
                        &member.party,
                        member.scoped_host.as_ref(),
                    ))
                    .map(TeamRefreshParty::verified)
                    .ok_or(foks_client::Error::TeamBinding(
                        "caller-durable CLKR remaining party is unavailable",
                    ))
            })
            .collect::<foks_client::Result<Vec<_>>>()?;
        let request = RefreshTeamMemberKeysRequest {
            expected_seqno: pending.expected_seqno,
            changes: &changes,
            rotations: &rotations,
            remaining_parties: &remaining,
        };
        let rotated = match (credential, actor.local_team(), resume, observe_as_transport) {
            (TeamRefreshCredential::Software(credential), _, true, true)
            | (TeamRefreshCredential::Software(credential), None, true, false) => {
                let expected_actor = foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?;
                self.client
                    .resume_refresh_team_member_keys_and_rotate_ptks(
                        host,
                        credential,
                        foks_client::TeamMutationRecovery {
                            team: team_id,
                            expected_seqno: pending.expected_seqno,
                            expected_operation_id: &pending.operation_id,
                        },
                        &request,
                        &expected_actor,
                        protected_store,
                    )?
            }
            (TeamRefreshCredential::Yubi(credential), _, true, true)
            | (TeamRefreshCredential::Yubi(credential), None, true, false) => {
                let expected_actor = foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?;
                self.client
                    .resume_refresh_team_member_keys_and_rotate_ptks_yubi(
                        host,
                        credential,
                        foks_client::TeamMutationRecovery {
                            team: team_id,
                            expected_seqno: pending.expected_seqno,
                            expected_operation_id: &pending.operation_id,
                        },
                        &request,
                        &expected_actor,
                        protected_store,
                    )?
            }
            (TeamRefreshCredential::Software(credential), None, false, _) => {
                self.client.refresh_team_member_keys_and_rotate_ptks(
                    host,
                    credential,
                    team_id,
                    &request,
                    protected_store,
                )?
            }
            (TeamRefreshCredential::Yubi(credential), None, false, _) => {
                self.client.refresh_team_member_keys_and_rotate_ptks_yubi(
                    host,
                    credential,
                    team_id,
                    &request,
                    protected_store,
                )?
            }
            (
                TeamRefreshCredential::Software(credential),
                Some((transport_user, actor_team, actor_recipient)),
                false,
                _,
            ) => self
                .client
                .refresh_team_member_keys_and_rotate_ptks_as_local_team(
                    host,
                    credential,
                    transport_user,
                    actor_team,
                    actor_recipient.ok_or(foks_client::Error::TeamBinding(
                        "fresh nested CLKR lacks a current actor recipient witness",
                    ))?,
                    team.expect("fresh nested CLKR has an authenticated target"),
                    &request,
                    protected_store,
                )?,
            (
                TeamRefreshCredential::Yubi(credential),
                Some((transport_user, actor_team, actor_recipient)),
                false,
                _,
            ) => self
                .client
                .refresh_team_member_keys_and_rotate_ptks_as_local_team_yubi(
                    host,
                    credential,
                    transport_user,
                    actor_team,
                    actor_recipient.ok_or(foks_client::Error::TeamBinding(
                        "fresh nested CLKR lacks a current actor recipient witness",
                    ))?,
                    team.expect("fresh nested CLKR has an authenticated target"),
                    &request,
                    protected_store,
                )?,
            (
                TeamRefreshCredential::Software(credential),
                Some((transport_user, actor_team, _)),
                true,
                false,
            ) => self
                .client
                .resume_refresh_team_member_keys_and_rotate_ptks_as_local_team(
                    host,
                    credential,
                    &transport_user.verified,
                    actor_team,
                    team_id,
                    &request,
                    &pending.operation_id,
                    &foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?,
                    protected_store,
                )?,
            (
                TeamRefreshCredential::Yubi(credential),
                Some((transport_user, actor_team, _)),
                true,
                false,
            ) => self
                .client
                .resume_refresh_team_member_keys_and_rotate_ptks_as_local_team_yubi(
                    host,
                    credential,
                    &transport_user.verified,
                    actor_team,
                    team_id,
                    &request,
                    &pending.operation_id,
                    &foks_proto::EntityId::from_bytes(pending.actor_uid.clone())?,
                    protected_store,
                )?,
        };
        Ok(rotated.authenticated)
    }
}

fn stored_team_rekey_matches_current_plan(
    team: &AuthenticatedTeamOutcome,
    parties: &std::collections::BTreeMap<TeamRefreshPartyKey, TeamRefreshParty>,
    actor_member: Option<&foks_verify::VerifiedTeamMemberState>,
    pending: &StoredTeamRekey,
) -> bool {
    let Some(actor_member) = actor_member else {
        return false;
    };
    let Ok(pending_actor_source_role) = pending.actor_source_role.role() else {
        return false;
    };
    if actor_member.party.as_bytes() != pending.actor_uid
        || actor_member.scoped_host.is_some()
        || actor_member.source_role != pending_actor_source_role
        || actor_member.generation != pending.actor_generation
        || !matches!(
            actor_member.role.kind(),
            foks_proto::RoleType::Admin | foks_proto::RoleType::Owner
        )
    {
        return false;
    }
    let root = team.verified.tree_root();
    if parties
        .values()
        .any(|party| !party.matches_local_root(team.verified.host(), &root))
    {
        return false;
    }
    let mut matched = 0usize;
    let members_match = team.verified.members().iter().all(|member| {
        let Some(party) = parties.get(&team_refresh_party_key(
            &member.party,
            member.scoped_host.as_ref(),
        )) else {
            return false;
        };
        let Some(key) = party.shared_key(member.source_role) else {
            return false;
        };
        let expected = pending.changes.iter().find(|change| {
            change.party == member.party.as_bytes()
                && change.host.as_deref() == member.scoped_host.as_ref().map(|host| host.as_bytes())
                && change.source_role.role().ok() == Some(member.source_role)
        });
        let roster_key_is_current = key.generation == member.generation
            && key.verify_key == member.verify_key
            && foks_crypto::hepk_fingerprint(&key.hepk).ok() == Some(member.hepk_fingerprint);
        let receives_rotated_key = pending
            .rotations
            .iter()
            .any(|rotation| rotation.role.role().is_ok_and(|role| role <= member.role));
        if receives_rotated_key && party.has_stale_shared_key(member.source_role) {
            return false;
        }
        if key.generation > member.generation && member.role <= actor_member.role {
            let matches = expected.is_some_and(|change| {
                change.destination_role.role().ok() == Some(member.role)
                    && change.generation == key.generation
                    && change.verify_key == key.verify_key.as_bytes()
                    && foks_crypto::hepk_fingerprint(&key.hepk).ok()
                        == Some(change.hepk_fingerprint)
            });
            if matches {
                matched += 1;
            }
            matches
        } else {
            expected.is_none() && (!receives_rotated_key || roster_key_is_current)
        }
    });
    members_match && matched == pending.changes.len()
}

fn team_mutation_matches_pending(
    operation: &foks_client_db::TeamMutationOperation,
    pending: &StoredTeamRekey,
    host_id: &[u8],
) -> bool {
    operation.operation_id == pending.operation_id
        && operation.kind == foks_client_db::TeamMutationKind::PtkRotation
        && operation.host_id == host_id
        && operation.actor_id == pending.actor_uid
        && operation.device_id == pending.actor_device_id
        && operation.team_id == pending.team_id
        && operation.expected_seqno == pending.expected_seqno
}

fn prefer_pending_signer_alias(
    aliases: &mut [String],
    vault: &mut AccountVault<'_>,
    preferred: Option<&[u8]>,
) {
    aliases.sort_by_key(|alias| {
        let matches_pending_signer = preferred.is_some_and(|preferred| {
            vault.account(alias).ok().is_some_and(|account| {
                account
                    .credential
                    .public_material()
                    .is_ok_and(|device| device.id.as_bytes() == preferred)
            })
        });
        !matches_pending_signer
    });
}

fn register_default_refresh_job(
    scheduler: &FoksScheduler,
    type_id: u64,
    kind: ScheduledJobKind,
    host: &foks_proto::EntityId,
    uid: &foks_proto::EntityId,
    interval_micros: u64,
    now: u64,
) -> Result<()> {
    let job_id = refresh_job_id(type_id, host, uid);
    scheduler.register_if_missing(ScheduledJobRegistration {
        job_id,
        kind,
        host_id: host.as_bytes().to_vec(),
        scope_id: uid.as_bytes().to_vec(),
        interval_micros,
        first_run_at: now
            .checked_add(interval_micros)
            .ok_or(Error::InvalidConfig("default refresh time overflow"))?,
        registered_at: now,
    })?;
    Ok(())
}

fn refresh_job_id(
    type_id: u64,
    host: &foks_proto::EntityId,
    uid: &foks_proto::EntityId,
) -> [u8; 16] {
    let mut binding = Vec::with_capacity(66);
    binding.extend_from_slice(host.as_bytes());
    binding.extend_from_slice(uid.as_bytes());
    let digest = foks_crypto::prefixed_hash(type_id, &binding);
    digest[..16]
        .try_into()
        .expect("hash prefix has fixed length")
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_client_db::HardStateStore;
    use foks_keystore::{EncryptedFileSecretStore, MemorySecretStore};
    use foks_server_testkit::TestEnvironment;
    use foks_yubi::{MockYubiProvider, Pin, SlotId, YubiProvider as _};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    fn paths(root: &std::path::Path, name: &str) -> ProfilePaths {
        let directory = root.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        ProfilePaths {
            hard_database: directory.join("hard.sqlite3"),
            soft_database: directory.join("soft.sqlite3"),
            protected_mutations: directory.join("mutations"),
            credential_store: directory.join("credentials"),
            directory,
        }
    }

    fn excluded_team_id(fill: u8) -> foks_proto::EntityId {
        let mut bytes = vec![fill; 33];
        bytes[0] = foks_proto::ENTITY_NAMED_TEAM;
        foks_proto::EntityId::from_bytes(bytes).unwrap()
    }

    /// A team whose caller-durable CLKR belongs to a device this sweep does
    /// not hold blocks every team that depends on it, so presenting two keys
    /// in either order converges. Unrelated teams stay rotatable.
    #[test]
    fn a_yubi_journal_blocks_every_dependent_team_until_its_device_runs() {
        let pending = excluded_team_id(0x31);
        let parent = excluded_team_id(0x32);
        let root = excluded_team_id(0x33);
        let unrelated = excluded_team_id(0x34);
        let blocked = exclude_team_ancestors(
            std::collections::BTreeSet::from([pending.as_bytes().to_vec()]),
            &[
                (pending.clone(), parent.clone()),
                (parent.clone(), root.clone()),
            ],
        );
        for team in [&pending, &parent, &root] {
            assert!(blocked.contains(team.as_bytes()));
        }
        assert!(!blocked.contains(unrelated.as_bytes()));
    }

    #[test]
    fn pending_mutation_signer_is_tried_before_other_owner_aliases() {
        let mut uid = vec![0x21; 33];
        uid[0] = foks_proto::ENTITY_USER;
        let uid = foks_proto::EntityId::from_bytes(uid).unwrap();
        let credential = |seed| foks_client::DeviceCredential {
            key_kind: foks_client::SoftwareKeyKind::Device,
            uid: uid.clone(),
            seed: foks_proto::SecretSeed::new([seed; 32]),
            certificate_chain: vec![vec![seed]],
        };
        let first = credential(0x31);
        let preferred = credential(0x41);
        let preferred_id = preferred.public_material().unwrap().id;
        let mut secrets = MemorySecretStore::default();
        let mut vault = AccountVault::new(&mut secrets);
        vault.commit_created("a-first", "alice", &first).unwrap();
        vault
            .commit_created("z-original-signer", "alice", &preferred)
            .unwrap();
        let mut aliases = vault.aliases().unwrap();

        prefer_pending_signer_alias(&mut aliases, &mut vault, Some(preferred_id.as_bytes()));

        assert_eq!(aliases[0], "z-original-signer");
    }

    #[test]
    fn obsolete_team_projection_defers_unless_exact_recovery_is_pending() {
        for status in [
            foks_rpc::STATUS_PERMISSION_ERROR,
            foks_rpc::STATUS_TEAM_NOT_FOUND_ERROR,
        ] {
            assert!(team_refresh_absence_is_deferred(false, status));
            assert!(!team_refresh_absence_is_deferred(true, status));
        }
        assert!(team_refresh_has_no_authorized_work(false, false));
        assert!(!team_refresh_has_no_authorized_work(false, true));
        assert!(!team_refresh_has_no_authorized_work(true, false));
    }

    #[test]
    fn scheduled_refresh_rotates_stale_puks_and_repairs_lagging_ppe() {
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let state = environment
            .client_path("background-security", "state")
            .unwrap();
        let root = environment
            .client_path("background-security", "probe-root.der")
            .unwrap();
        environment.write_probe_root(&root).unwrap();
        crate::ClientCredentials::initialize(&state, crate::CredentialBackend::PrivateFile)
            .unwrap();
        let addresses = environment.addresses().unwrap();
        let mut registry = crate::ProfileRegistry::open(&state).unwrap();
        registry
            .add(crate::Profile {
                name: "local".to_owned(),
                probe: format!("localhost:{}", addresses.probe.port()),
                protocol: crate::ProtocolPolicy::V019,
                trust: crate::TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let session = crate::ProfileSession::open(&registry, "local").unwrap();
        let credentials = crate::ClientCredentials::open(&state).unwrap();
        credentials
            .with_checked_session(&session, |session| {
                session.probe_and_pin()?;
                Ok::<_, crate::Error>(())
            })
            .unwrap();
        let master = credentials.master_key().unwrap();
        credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.create_account(
                    "primary",
                    "backgroundowner",
                    "primary owner",
                    "background@example.test",
                    "",
                    Some(foks_client::Passphrase::new(
                        "background security passphrase",
                    )?),
                    &mut vault,
                    &master,
                )?;
                session.provision_owner_device(
                    "primary",
                    "survivor",
                    "surviving owner",
                    2,
                    &mut vault,
                    &master,
                )?;
                session.create_account(
                    "member",
                    "backgroundmember",
                    "team member",
                    "member@example.test",
                    "",
                    None,
                    &mut vault,
                    &master,
                )?;
                session.create_named_team(
                    "primary",
                    "security-team",
                    "background-security",
                    &mut vault,
                    &master,
                )?;
                let primary = vault.account("primary")?;
                let survivor = vault.account("survivor")?;
                let member = vault.account("member")?;
                let team_id =
                    foks_proto::EntityId::from_bytes(vault.team("security-team")?.team_id.clone())?;
                let target = survivor.credential.public_material()?.id;
                let host = session.pinned_host()?;
                session
                    .client
                    .authenticate_and_pin(&host, &survivor.credential)
                    .expect("provisioned survivor authenticates before self-revocation");
                let mut mutations = EncryptedFileMutationStore::open(
                    &session.paths().protected_mutations,
                    crate::derive_mutation_key(&master),
                )?;
                let member_state = session
                    .client
                    .authenticate_and_pin(&host, &member.credential)?;
                let added = session.client.add_local_user_to_named_team(
                    &host,
                    &primary.credential,
                    &team_id,
                    &foks_client::AddLocalTeamMemberRequest {
                        target_user: &member_state.verified,
                        destination_role: foks_proto::Role::ADMIN,
                        removal_key: &foks_proto::SecretSeed::new([0xb1; 32]),
                    },
                )?;
                session.client.load_and_pin_user_as_local_team(
                    &host,
                    &primary.credential,
                    &member.credential.uid,
                    &added.authenticated.view_token,
                )?;
                let member_owner = member_state
                    .puks
                    .iter()
                    .find(|key| key.role == foks_proto::Role::OWNER && key.generation == 1)
                    .ok_or(crate::Error::InvalidAccount(
                        "member owner PUK is unavailable",
                    ))?;
                session
                    .client
                    .rotate_software_puks_without_passphrase_annex_for_test(
                        &host,
                        &member.credential,
                        &[foks_client::UserPukRotation {
                            role: foks_proto::Role::OWNER,
                            previous_generation: 1,
                            previous_seed: foks_proto::SecretSeed::new(
                                *member_owner.seed.as_bytes(),
                            ),
                            new_seed: foks_proto::SecretSeed::new([0xc1; 32]),
                        }],
                        &mut mutations,
                    )?;
                let stale = session
                    .client
                    .revoke_user_credential_without_puk_rotation_for_test(
                        &host,
                        &survivor.credential,
                        &primary.credential,
                        &target,
                        &mut mutations,
                    )?;
                assert!(stale
                    .verified
                    .stale_shared_key_roles()
                    .contains(&foks_proto::Role::OWNER));
                assert_eq!(
                    stale
                        .verified
                        .shared_key(foks_proto::Role::OWNER)
                        .unwrap()
                        .generation,
                    1
                );
                // Model a KEX-provisioned installation (or a client whose
                // local PPE pin predates another owner's passphrase change).
                // The stale-owner repair must establish the pin from the
                // authenticated settings parcel instead of deadlocking on it.
                rusqlite::Connection::open(&session.paths().hard_database)
                    .unwrap()
                    .execute(
                        "DELETE FROM user_local_security WHERE host_id = ?1 AND uid = ?2",
                        rusqlite::params![
                            host.host_id().as_bytes(),
                            primary.credential.uid.as_bytes()
                        ],
                    )
                    .unwrap();
                assert_eq!(
                    HardStateStore::open(&session.paths().hard_database)?
                        .trusted_user_passphrase_parcel_hash(
                            host.host_id().as_bytes(),
                            primary.credential.uid.as_bytes(),
                        )?,
                    None
                );
                session
                    .refresh_team_chains(primary.credential.uid.as_bytes(), &mut vault, &master)
                    .expect("CLKR sweep defers recipients with a stale PUK");
                let still_stale = session
                    .client
                    .authenticate_and_pin(&host, &primary.credential)?;
                let unchanged_team = session.client.load_and_pin_team(
                    &host,
                    &primary.credential,
                    &still_stale.verified,
                    &still_stale.puks,
                    &team_id,
                )?;
                assert_eq!(
                    unchanged_team.verified.chain_seqno(),
                    2,
                    "CLKR must not box fresh PTKs to an unrotated revoked-device PUK"
                );
                Ok::<_, crate::Error>(())
            })
            .unwrap();

        let first_due = test_micros_now().saturating_add(20 * 60 * 1_000_000);
        let first = credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.run_due_jobs(first_due, &mut vault, &master)
            })
            .unwrap();
        assert_eq!(first.runs.len(), 4);
        assert!(
            first.runs.iter().all(|run| run.completed),
            "scheduled runs failed: {:?}",
            first.runs
        );

        credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                let active = vault.account("primary")?;
                let host = session.pinned_host()?;
                let rotated = session
                    .client
                    .authenticate_and_pin(&host, &active.credential)?;
                assert!(rotated.verified.stale_shared_key_roles().is_empty());
                assert_eq!(rotated.verified.devices().len(), 1);
                assert_eq!(
                    rotated
                        .verified
                        .shared_key(foks_proto::Role::OWNER)
                        .unwrap()
                        .generation,
                    2
                );
                assert_eq!(
                    session
                        .client
                        .passphrase_metadata(&host, &active.credential)?
                        .generation,
                    2
                );
                let member = vault.account("member")?;
                let member_state = session
                    .client
                    .authenticate_and_pin(&host, &member.credential)?;
                let team_id =
                    foks_proto::EntityId::from_bytes(vault.team("security-team")?.team_id.clone())?;
                let team = session.client.load_and_pin_team(
                    &host,
                    &active.credential,
                    &rotated.verified,
                    &rotated.puks,
                    &team_id,
                )?;
                assert_eq!(team.verified.chain_seqno(), 3);
                assert_eq!(
                    team.verified.group_change_at(3)?.changes.len(),
                    2,
                    "one atomic CLKR link must refresh both stale roster rows"
                );
                for uid in [&active.credential.uid, &member.credential.uid] {
                    assert_eq!(
                        team.verified
                            .members()
                            .iter()
                            .find(|roster| roster.party == *uid)
                            .unwrap()
                            .generation,
                        2
                    );
                }
                assert_eq!(
                    member_state
                        .verified
                        .shared_key(foks_proto::Role::OWNER)
                        .unwrap()
                        .generation,
                    2
                );
                assert!(team
                    .verified
                    .shared_keys()
                    .iter()
                    .all(|key| key.generation >= 2));
                let member_team = session.client.load_and_pin_team(
                    &host,
                    &member.credential,
                    &member_state.verified,
                    &member_state.puks,
                    &team_id,
                )?;
                assert_eq!(member_team.verified.chain_seqno(), 3);

                let previous = rotated
                    .puks
                    .iter()
                    .find(|key| key.role == foks_proto::Role::OWNER && key.generation == 2)
                    .unwrap();
                let mut mutations = EncryptedFileMutationStore::open(
                    &session.paths().protected_mutations,
                    crate::derive_mutation_key(&master),
                )?;
                session
                    .client
                    .rotate_software_puks_without_passphrase_annex_for_test(
                        &host,
                        &active.credential,
                        &[foks_client::UserPukRotation {
                            role: foks_proto::Role::OWNER,
                            previous_generation: 2,
                            previous_seed: foks_proto::SecretSeed::new(*previous.seed.as_bytes()),
                            new_seed: foks_proto::SecretSeed::new([0xd1; 32]),
                        }],
                        &mut mutations,
                    )?;
                assert_eq!(
                    session
                        .client
                        .passphrase_metadata(&host, &active.credential)?
                        .generation,
                    2
                );
                Ok::<_, crate::Error>(())
            })
            .unwrap();

        let second_due = first_due.saturating_add(21 * 60 * 1_000_000);
        let second = credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.run_due_jobs(second_due, &mut vault, &master)
            })
            .unwrap();
        assert_eq!(second.runs.len(), 4);
        assert!(
            second.runs.iter().all(|run| run.completed),
            "scheduled runs failed: {:?}",
            second.runs
        );
        credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                let active = vault.account("primary")?;
                let host = session.pinned_host()?;
                assert_eq!(
                    session
                        .client
                        .passphrase_metadata(&host, &active.credential)?
                        .generation,
                    3
                );
                let authenticated = session
                    .client
                    .authenticate_and_pin(&host, &active.credential)?;
                let team_id =
                    foks_proto::EntityId::from_bytes(vault.team("security-team")?.team_id.clone())?;
                let team = session.client.load_and_pin_team(
                    &host,
                    &active.credential,
                    &authenticated.verified,
                    &authenticated.puks,
                    &team_id,
                )?;
                assert_eq!(team.verified.chain_seqno(), 4);
                let owner_generations = team
                    .ptks
                    .iter()
                    .filter(|key| key.role == foks_proto::Role::OWNER)
                    .map(|key| key.generation)
                    .collect::<Vec<_>>();
                assert_eq!(owner_generations, vec![1, 2, 3]);
                assert_eq!(
                    team.verified
                        .members()
                        .iter()
                        .find(|member| member.party == active.credential.uid)
                        .unwrap()
                        .generation,
                    3
                );
                assert_eq!(
                    session
                        .client
                        .load_generic_chain(
                            &host,
                            &active.credential,
                            foks_proto::CHAIN_TYPE_USER_SETTINGS,
                            1,
                        )?
                        .links
                        .len(),
                    3
                );
                Ok::<_, crate::Error>(())
            })
            .unwrap();
    }

    #[test]
    fn unlocked_yubi_runs_mixed_roster_user_and_team_security_responders() {
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let state = environment.client_path("yubi-security", "state").unwrap();
        let root = environment
            .client_path("yubi-security", "probe-root.der")
            .unwrap();
        environment.write_probe_root(&root).unwrap();
        crate::ClientCredentials::initialize(&state, crate::CredentialBackend::PrivateFile)
            .unwrap();
        let addresses = environment.addresses().unwrap();
        let mut registry = crate::ProfileRegistry::open(&state).unwrap();
        registry
            .add(crate::Profile {
                name: "local".to_owned(),
                probe: format!("localhost:{}", addresses.probe.port()),
                protocol: crate::ProtocolPolicy::V019,
                trust: crate::TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let session = crate::ProfileSession::open(&registry, "local").unwrap();
        let credentials = crate::ClientCredentials::open(&state).unwrap();
        credentials
            .with_checked_session(&session, |session| {
                session.probe_and_pin()?;
                Ok::<_, crate::Error>(())
            })
            .unwrap();
        let master = credentials.master_key().unwrap();
        let pin = Pin::new("123456").unwrap();
        let provider = MockYubiProvider::with_card("security-yubikey", 73001, &pin).unwrap();
        let card = provider.cards().unwrap().remove(0);

        credentials
            .with_checked_session(&session, |session| {
                let mut store = EncryptedFileSecretStore::open(
                    &session.paths().credential_store,
                    crate::derive_vault_key(&master),
                )?;
                let mut vault = AccountVault::new(&mut store);
                session.create_account(
                    "primary",
                    "yubisecurityowner",
                    "primary owner",
                    "yubi-security@example.test",
                    "",
                    Some(foks_client::Passphrase::new("Yubi security passphrase")?),
                    &mut vault,
                    &master,
                )?;
                session.provision_owner_device(
                    "primary",
                    "retired",
                    "device to revoke",
                    2,
                    &mut vault,
                    &master,
                )?;
                session.create_account(
                    "member",
                    "yubisecuritymember",
                    "team member",
                    "yubi-member@example.test",
                    "",
                    None,
                    &mut vault,
                    &master,
                )?;
                session.create_named_team(
                    "primary",
                    "security-team",
                    "yubi-security-team",
                    &mut vault,
                    &master,
                )?;
                session.provision_yubi_device(
                    crate::YubiProvisionInput {
                        source_alias: "primary".to_owned(),
                        target_alias: "hardware".to_owned(),
                        device_name: "security YubiKey".to_owned(),
                        serial: 3,
                        card,
                        signing_slot: SlotId::new(0x82)?,
                        pq_slot: SlotId::new(0x83)?,
                        retry_configuration: None,
                    },
                    Pin::new("123456")?,
                    &provider,
                    &mut vault,
                    &master,
                )?;

                let host = session.pinned_host()?;
                let primary = vault.account("primary")?;
                let retired = vault.account("retired")?;
                let member = vault.account("member")?;
                let team_id =
                    foks_proto::EntityId::from_bytes(vault.team("security-team")?.team_id.clone())?;
                let member_state = session
                    .client
                    .authenticate_and_pin(&host, &member.credential)?;
                let added = session.client.add_local_user_to_named_team(
                    &host,
                    &primary.credential,
                    &team_id,
                    &foks_client::AddLocalTeamMemberRequest {
                        target_user: &member_state.verified,
                        destination_role: foks_proto::Role::ADMIN,
                        removal_key: &foks_proto::SecretSeed::new([0xa1; 32]),
                    },
                )?;
                session.client.load_and_pin_user_as_local_team(
                    &host,
                    &primary.credential,
                    &member.credential.uid,
                    &added.authenticated.view_token,
                )?;
                let member_owner = member_state
                    .puks
                    .iter()
                    .find(|key| key.role == foks_proto::Role::OWNER && key.generation == 1)
                    .ok_or(crate::Error::InvalidAccount(
                        "member owner PUK is unavailable",
                    ))?;
                let mut mutations = EncryptedFileMutationStore::open(
                    &session.paths().protected_mutations,
                    crate::derive_mutation_key(&master),
                )?;
                session
                    .client
                    .rotate_software_puks_without_passphrase_annex_for_test(
                        &host,
                        &member.credential,
                        &[foks_client::UserPukRotation {
                            role: foks_proto::Role::OWNER,
                            previous_generation: 1,
                            previous_seed: foks_proto::SecretSeed::new(
                                *member_owner.seed.as_bytes(),
                            ),
                            new_seed: foks_proto::SecretSeed::new([0xc1; 32]),
                        }],
                        &mut mutations,
                    )?;
                let retired_id = retired.credential.public_material()?.id;
                let stale = session
                    .client
                    .revoke_user_credential_without_puk_rotation_for_test(
                        &host,
                        &retired.credential,
                        &primary.credential,
                        &retired_id,
                        &mut mutations,
                    )?;
                assert_eq!(
                    stale.verified.stale_shared_key_roles(),
                    &std::collections::BTreeSet::from([foks_proto::Role::OWNER])
                );
                assert_eq!(
                    session
                        .client
                        .passphrase_metadata(&host, &primary.credential)?
                        .generation,
                    1
                );
                assert_eq!(vault.yubi_management_generation("hardware")?, Some(1));

                let first = session.sync_yubi_account(
                    "hardware",
                    Pin::new("123456")?,
                    &provider,
                    &mut vault,
                    &master,
                )?;
                let refreshed = session
                    .client
                    .authenticate_and_pin(&host, &primary.credential)?;
                assert!(refreshed.verified.stale_shared_key_roles().is_empty());
                assert_eq!(
                    refreshed
                        .verified
                        .shared_key(foks_proto::Role::OWNER)
                        .unwrap()
                        .generation,
                    2
                );
                assert_eq!(
                    session
                        .client
                        .passphrase_metadata(&host, &primary.credential)?
                        .generation,
                    2
                );
                assert_eq!(vault.yubi_management_generation("hardware")?, Some(2));
                let member_refreshed = session
                    .client
                    .authenticate_and_pin(&host, &member.credential)?;
                let team = session.client.load_and_pin_team(
                    &host,
                    &primary.credential,
                    &refreshed.verified,
                    &refreshed.puks,
                    &team_id,
                )?;
                assert_eq!(team.verified.chain_seqno(), 3);
                assert_eq!(team.verified.group_change_at(3)?.changes.len(), 2);
                for (uid, generation) in [(&primary.credential.uid, 2), (&member.credential.uid, 2)]
                {
                    assert_eq!(
                        team.verified
                            .members()
                            .iter()
                            .find(|roster| roster.party == *uid)
                            .unwrap()
                            .generation,
                        generation
                    );
                }
                let member_team = session.client.load_and_pin_team(
                    &host,
                    &member.credential,
                    &member_refreshed.verified,
                    &member_refreshed.puks,
                    &team_id,
                )?;
                assert_eq!(member_team.verified.chain_seqno(), 3);
                assert!(member_team
                    .verified
                    .shared_keys()
                    .iter()
                    .all(|key| key.generation >= 2));

                let second = session.sync_yubi_account(
                    "hardware",
                    Pin::new("123456")?,
                    &provider,
                    &mut vault,
                    &master,
                )?;
                assert_eq!(second.user_chain_sequence, first.user_chain_sequence);
                let unchanged = session.client.load_and_pin_team(
                    &host,
                    &primary.credential,
                    &refreshed.verified,
                    &refreshed.puks,
                    &team_id,
                )?;
                assert_eq!(unchanged.verified.chain_seqno(), 3);
                Ok::<_, crate::Error>(())
            })
            .unwrap();
    }

    fn test_micros_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros()
            .try_into()
            .unwrap()
    }

    #[test]
    fn first_unlocked_account_registers_default_chain_refreshes_once() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = crate::ProfileRegistry::open(&root).unwrap();
        registry
            .add(crate::Profile {
                name: "local".to_owned(),
                probe: "foks.app".to_owned(),
                protocol: crate::ProtocolPolicy::V019,
                trust: crate::TrustRoot::WebPki,
            })
            .unwrap();
        let session = crate::ProfileSession::open(&registry, "local").unwrap();
        let verified = foks_verify::verify_public_host(
            "foks.app",
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            ),
        )
        .unwrap();
        HardStateStore::open(&session.paths.hard_database)
            .unwrap()
            .accept_verified_host(&verified.snapshot)
            .unwrap();
        let mut uid = vec![7; 33];
        uid[0] = foks_proto::ENTITY_USER;
        let uid = foks_proto::EntityId::from_bytes(uid).unwrap();
        let credential = foks_client::DeviceCredential {
            key_kind: foks_client::SoftwareKeyKind::Device,
            uid: uid.clone(),
            seed: foks_proto::SecretSeed::new([8; 32]),
            certificate_chain: vec![vec![9]],
        };
        let mut secrets = MemorySecretStore::default();
        let mut vault = AccountVault::new(&mut secrets);
        vault
            .commit_created("primary", "alice", &credential)
            .unwrap();
        vault
            .commit_created("backup", "alice", &credential)
            .unwrap();
        let checked = crate::CheckedProfileSession { session: &session };
        checked
            .ensure_default_refresh_jobs(&mut vault, 100)
            .unwrap();
        checked
            .ensure_default_refresh_jobs(&mut vault, 200)
            .unwrap();

        let host = foks_proto::EntityId::from_bytes(verified.snapshot.host_id().to_vec()).unwrap();
        let store = HardStateStore::open(&session.paths.hard_database).unwrap();
        let user = store
            .scheduled_job(&refresh_job_id(USER_REFRESH_JOB_TYPE_ID, &host, &uid))
            .unwrap()
            .unwrap();
        let team = store
            .scheduled_job(&refresh_job_id(TEAM_REFRESH_JOB_TYPE_ID, &host, &uid))
            .unwrap()
            .unwrap();
        assert_eq!(user.kind, ScheduledJobKind::UserRefresh);
        assert_eq!(user.next_run_at, 100 + DEFAULT_USER_REFRESH_INTERVAL_MICROS);
        assert_eq!(team.kind, ScheduledJobKind::TeamRefresh);
        assert_eq!(team.next_run_at, 100 + DEFAULT_TEAM_REFRESH_INTERVAL_MICROS);
    }

    #[test]
    fn probe_only_profile_skips_default_job_registration() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = crate::ProfileRegistry::open(&root).unwrap();
        registry
            .add(crate::Profile {
                name: "hosted".to_owned(),
                probe: "foks.app".to_owned(),
                protocol: crate::ProtocolPolicy::CurrentProbeOnly {
                    canary_public_key:
                        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
                            .to_owned(),
                    lease_url: "https://updates.example.test/foks/canary.json".to_owned(),
                    last_artifact: None,
                },
                trust: crate::TrustRoot::WebPki,
            })
            .unwrap();
        let session = crate::ProfileSession::open(&registry, "hosted").unwrap();
        let checked = crate::CheckedProfileSession { session: &session };
        let mut uid = vec![7; 33];
        uid[0] = foks_proto::ENTITY_USER;
        let uid = foks_proto::EntityId::from_bytes(uid).unwrap();

        checked
            .register_default_refresh_jobs_for(&uid, 100)
            .unwrap();

        let store = HardStateStore::open(&session.paths.hard_database).unwrap();
        assert_eq!(store.next_scheduled_run().unwrap(), None);
    }

    #[test]
    fn native_manifest_lock_serializes_namespace_updates() {
        let id = crate::hex(&crate::random_array::<16>().unwrap());
        let first = NativeManifestLock::acquire(&id).unwrap();
        assert!(NativeManifestLock::try_acquire(&id).unwrap().is_none());
        first.release().unwrap();
        let second = NativeManifestLock::try_acquire(&id).unwrap().unwrap();
        second.release().unwrap();
        #[cfg(unix)]
        assert_eq!(
            crate::portability::manifest_lock_file(&id)
                .unwrap()
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn native_manifest_locks_contend_across_processes() {
        const ID_ENV: &str = "FOKS_NATIVE_MANIFEST_LOCK_TEST_ID";
        if let Ok(id) = std::env::var(ID_ENV) {
            assert!(NativeManifestLock::try_acquire(&id).unwrap().is_none());
            return;
        }
        let id = crate::hex(&crate::random_array::<16>().unwrap());
        let lock = NativeManifestLock::acquire(&id).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::tests::native_manifest_locks_contend_across_processes",
                "--nocapture",
            ])
            .env(ID_ENV, &id)
            .status()
            .unwrap();
        assert!(status.success());
        lock.release().unwrap();
    }

    #[test]
    fn operation_locks_are_per_profile_and_scheduler_locks_are_independent() {
        let temporary = tempfile::tempdir().unwrap();
        let first = paths(temporary.path(), "first");
        let second = paths(temporary.path(), "second");
        let operation = ProfileLock::operation(&first).unwrap();
        assert!(ProfileLock::try_acquire(&first, OPERATION_LOCK_FILE)
            .unwrap()
            .is_none());
        let other_profile = ProfileLock::try_acquire(&second, OPERATION_LOCK_FILE)
            .unwrap()
            .unwrap();
        let scheduler = ProfileLock::try_acquire(&first, SCHEDULER_LOCK_FILE)
            .unwrap()
            .unwrap();
        assert!(ProfileLock::try_acquire(&first, SCHEDULER_LOCK_FILE)
            .unwrap()
            .is_none());
        assert!(first.directory.join(OPERATION_LOCK_FILE).is_file());
        assert!(first.directory.join(SCHEDULER_LOCK_FILE).is_file());
        assert!(!temporary.path().join(".rollback-checkpoint.lock").exists());
        scheduler.release().unwrap();
        other_profile.release().unwrap();
        operation.release().unwrap();
    }

    #[test]
    fn database_locks_are_keyed_by_identity_within_one_client_root() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        std::fs::create_dir(&root).unwrap();
        let first = DatabaseLock::acquire(&root, &[1; 16]).unwrap();
        assert!(DatabaseLock::try_acquire(&root, &[1; 16])
            .unwrap()
            .is_none());
        let other = DatabaseLock::try_acquire(&root, &[2; 16]).unwrap().unwrap();
        assert!(root
            .join(DATABASE_LOCK_DIRECTORY)
            .join(format!("{}.lock", crate::hex(&[1; 16])))
            .is_file());
        other.release().unwrap();
        first.release().unwrap();
    }

    #[test]
    fn database_locks_contend_across_processes() {
        const ROOT_ENV: &str = "FOKS_DATABASE_LOCK_TEST_ROOT";
        const CHILD_ENV: &str = "FOKS_DATABASE_LOCK_TEST_CHILD";

        if std::env::var_os(CHILD_ENV).is_some() {
            let root = std::path::PathBuf::from(std::env::var_os(ROOT_ENV).unwrap());
            assert!(DatabaseLock::try_acquire(&root, &[3; 16])
                .unwrap()
                .is_none());
            return;
        }

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        std::fs::create_dir(&root).unwrap();
        let lock = DatabaseLock::acquire(&root, &[3; 16]).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::tests::database_locks_contend_across_processes",
                "--nocapture",
            ])
            .env(ROOT_ENV, &root)
            .env(CHILD_ENV, "1")
            .status()
            .unwrap();
        assert!(status.success());
        lock.release().unwrap();
    }

    #[test]
    fn profile_locks_contend_across_processes() {
        const ROOT_ENV: &str = "FOKS_PROFILE_LOCK_TEST_ROOT";
        const CHILD_ENV: &str = "FOKS_PROFILE_LOCK_TEST_CHILD";

        if std::env::var_os(CHILD_ENV).is_some() {
            let root = std::path::PathBuf::from(std::env::var_os(ROOT_ENV).unwrap());
            let profile = paths(&root, "profile");
            assert!(ProfileLock::try_operation(&profile).unwrap().is_none());
            assert!(ProfileLock::try_scheduler(&profile).unwrap().is_none());
            return;
        }

        let temporary = tempfile::tempdir().unwrap();
        let profile = paths(temporary.path(), "profile");
        let operation = ProfileLock::operation(&profile).unwrap();
        let scheduler = ProfileLock::scheduler(&profile).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime::tests::profile_locks_contend_across_processes",
                "--nocapture",
            ])
            .env(ROOT_ENV, temporary.path())
            .env(CHILD_ENV, "1")
            .status()
            .unwrap();
        assert!(status.success());
        scheduler.release().unwrap();
        operation.release().unwrap();
    }
}
