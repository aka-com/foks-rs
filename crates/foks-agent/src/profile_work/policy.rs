//! Operation admission policy, separate from queueing and worker lifetimes.
//!
//! Policy dimensions deliberately have different owners and defaults:
//!
//! | Decision | Owner | Guard when adding an operation |
//! | --- | --- | --- |
//! | Profiles/root reserved | `operation_scope` | Exhaustive operation match below |
//! | Shared profile locks | `read_cache::operation_shares_profile` | Explicit allow-list; otherwise exclusive |
//! | Cached authentication | `read_cache::operation_serves_reads` / `operation_leaves_retained_material` | Explicit allow-lists; otherwise invalidating |
//! | Disconnect abandonment | `read_cache::operation_is_abandonable` | Intent writes and trust updates remain owned |
//! | IPC mutation uncertainty | `Operation::is_mutation` | Unlisted operations remain mutations |
//! | Worker lane | `ConnectionCapacity::worker_pool` and chat-poll admission | Bounded worker capacity |
//! | Credential interaction | `operation_is_noninteractive` | Explicit noninteractive allow-list |
//!
//! A cache read can write verified local projections and need exclusive locks.
//! A local intent write can preserve cached keys while still requiring a worker
//! to finish after disconnect. Do not derive these policies from one read/write
//! flag. Tests here exercise those combinations; coordinator tests in the parent
//! module cover waiting, barriers, cancellation and actual permit lifetimes.

use super::Scope;
use foks_agent_proto::{KvStoreRef, Operation};

fn kv_scope(store: &KvStoreRef) -> Scope {
    Scope::profile(match store {
        KvStoreRef::Account(store) => &store.profile,
        KvStoreRef::Team(store) => &store.profile,
    })
}

/// Exhaustive so a new operation cannot silently bypass profile admission.
pub(crate) fn operation_scope(operation: &Operation) -> Scope {
    use Operation::*;
    match operation {
        Ping | AgentStatus | RetentionStatus | DiscoverGoProfiles => Scope::None,
        ListProfiles => Scope::RegistryRead,
        InitializeState { .. }
        | AddProfile { .. }
        | CheckAndAddProfile { .. }
        | CheckAndAddGoProfile { .. }
        | RemoveProfile { .. }
        | SetProfileLabel { .. }
        | ResetHardState { .. } => Scope::Root,
        RefreshLease { profile } | ReconcileProfile { profile } => Scope::profile(profile),
        PromoteTeamMember { .. }
        | DemoteTeamMember { .. }
        | RemoveTeamMember { .. }
        | ResumeTeamMemberEdit { .. }
        | ExpelFederatedTeam { .. }
        | RefreshFederatedSecurity { .. }
        | RunDueJobs { .. }
        | SyncYubiAccount {
            with_federation: true,
            ..
        } => Scope::SecurityRoot,
        AddFederatedTeamMember {
            local_profile,
            remote_profile,
            ..
        } => {
            let mut profiles = vec![local_profile.clone(), remote_profile.clone()];
            profiles.sort();
            profiles.dedup();
            Scope::Profiles(profiles)
        }
        Invitations {
            profile, action, ..
        } => {
            let mut profiles = vec![profile.clone()];
            if let Some(remote) = action.remote_profile() {
                profiles.push(remote.to_owned());
            }
            profiles.sort();
            profiles.dedup();
            Scope::Profiles(profiles)
        }
        PrepareDataWrite { scope, .. } | PendingDataWrites { scope } | ReadData { scope, .. } => {
            Scope::profile(&scope.profile)
        }
        ExecuteDataWrite { submission } | DataWriteStatus { submission } => {
            Scope::profile(&submission.scope.profile)
        }
        Chat { store, .. } | ListTeamKv { store, .. } => Scope::profile(&store.profile),
        ListKv { store, .. } => Scope::profile(&store.profile),
        ReadKv { store, .. }
        | ReadKvChunk { store, .. }
        | PutKv { store, .. }
        | PutKvSymlink { store, .. }
        | MkdirKv { store, .. }
        | MoveKv { store, .. }
        | RemoveKv { store, .. } => kv_scope(store),
        PutKvStream { header } => match &header.adapter {
            Some(submission) => Scope::profile(&submission.scope.profile),
            None => kv_scope(&header.store),
        },
        SetLocalAccountAlias { profile, .. } => Scope::LocalMetadata(profile.clone()),
        WebAdmin { profile, .. }
        | BotAccount { profile, .. }
        | ListAccountRenames { profile, .. }
        | RenameAccount { profile, .. }
        | Sso { profile, .. }
        | BindDataAccount { profile, .. }
        | DescribeResetHardState { profile }
        | Probe { profile }
        | ListKnownStores { profile }
        | ListProfileOverview { profile }
        | ListAccounts { profile }
        | ListPendingOperations { profile }
        | CreateAccount { profile, .. }
        | ResumeAccount { profile, .. }
        | ListDevices { profile, .. }
        | ListBackupEnrollments { profile, .. }
        | DescribeServerStatus { profile }
        | RemoveDevice { profile, .. }
        | ProvisionOwnerDevice { profile, .. }
        | ResumeOwnerDeviceProvision { profile, .. }
        | PrepareOwnerBackup { profile, .. }
        | CommitOwnerBackup { profile, .. }
        | RevokeOwnerBackup { profile, .. }
        | RecoverOwnerAccount { profile, .. }
        | ResumeOwnerRecovery { profile, .. }
        | PassphraseStatus { profile, .. }
        | SetPassphrase { profile, .. }
        | ChangePassphrase { profile, .. }
        | VerifyPassphrase { profile, .. }
        | SetYubiPassphrase { profile, .. }
        | ChangeYubiPassphrase { profile, .. }
        | VerifyYubiPassphrase { profile, .. }
        | SyncAccount { profile, .. }
        | StartDevicePairing { profile, .. }
        | RepublishDevicePairing { profile, .. }
        | FinishDevicePairing { profile, .. }
        | AcceptDevicePairing { profile, .. }
        | AcceptGoProfilePairing { profile, .. }
        | ResumeDevicePairingAcceptance { profile, .. }
        | ResumeGoProfilePairing { profile, .. }
        | CopyGoProfileDevice { profile, .. }
        | ListYubiCards { profile }
        | ListYubiAccounts { profile }
        | CreateYubiAccount { profile, .. }
        | ResumeYubiAccount { profile, .. }
        | ProvisionYubiDevice { profile, .. }
        | SyncYubiAccount { profile, .. }
        | YubiPinStatus { profile, .. }
        | ChangeYubiPin { profile, .. }
        | ChangeYubiPuk { profile, .. }
        | UnblockYubiPin { profile, .. }
        | RotateYubiManagementKey { profile, .. }
        | ResumeYubiManagementKey { profile, .. }
        | RecoverYubiManagementKey { profile, .. }
        | RecoverYubiSubkey { profile, .. }
        | RevokeYubiDevice { profile, .. }
        | CreateTeam { profile, .. }
        | ResumeTeamCreation { profile, .. }
        | AbandonTeamCreation { profile, .. }
        | ListTeams { profile }
        | DiscoverTeams { profile, .. }
        | SyncTeam { profile, .. }
        | ListTeamDetails { profile, .. }
        | ListTeamMembers { profile, .. }
        | AddTeamMember { profile, .. }
        | ResumeTeamMemberAddition { profile, .. }
        | ListFederatedTeams { profile, .. } => Scope::profile(profile),
    }
}

/// The scope a request is admitted under: its operation scope, in shared
/// mode for the reads that may share their profile.
pub(crate) fn admission_scope(operation: &Operation) -> Scope {
    match operation_scope(operation) {
        Scope::Profiles(mut profiles)
            if profiles.len() == 1 && crate::read_cache::operation_shares_profile(operation) =>
        {
            Scope::SharedProfile(profiles.remove(0))
        }
        scope => scope,
    }
}

#[cfg(test)]
mod tests;
