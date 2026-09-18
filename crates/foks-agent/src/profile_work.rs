//! Agent-owned admission. A permit covers the actual worker lifetime, including
//! cancellation cleanup; it never represents permission to replay an operation.
use foks_agent_proto::{KvStoreRef, Operation};
use foks_client_app::CancellationToken;
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::Notify;

const MAX_QUEUED: usize = 256;
const CONTROL_POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Scope {
    None,
    Profiles(Vec<String>),
    LocalMetadata(String),
    SecurityRoot,
    // Snapshot reads share admission with profile work and other readers, but
    // remain ordered against root-wide registry changes and state maintenance.
    // ProfileRegistry::open still takes the cross-process registry lock.
    RegistryRead,
    // Registry changes and federation cascades whose transitive profile set is
    // only known during execution reserve the state root before taking any lock.
    Root,
}
impl Scope {
    pub(super) fn profile(profile: &str) -> Self {
        Self::Profiles(vec![profile.to_owned()])
    }
    fn overlaps(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, _) | (_, Self::None) => false,
            (Self::Root, _) | (_, Self::Root) => true,
            (Self::LocalMetadata(a), Self::LocalMetadata(b)) => a == b,
            (Self::LocalMetadata(_), _) | (_, Self::LocalMetadata(_)) => false,
            (Self::RegistryRead, _) | (_, Self::RegistryRead) => false,
            (Self::SecurityRoot, _) | (_, Self::SecurityRoot) => true,
            (Self::Profiles(a), Self::Profiles(b)) => a.iter().any(|p| b.contains(p)),
        }
    }
}

fn kv_scope(store: &KvStoreRef) -> Scope {
    Scope::profile(match store {
        KvStoreRef::Account(store) => &store.profile,
        KvStoreRef::Team(store) => &store.profile,
    })
}

/// Exhaustive so a new operation cannot silently bypass profile admission.
pub(super) fn operation_scope(operation: &Operation) -> Scope {
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
        | ResetHardState { .. }
        | RefreshLease { .. }
        | ReconcileProfile { .. } => Scope::Root,
        DemoteTeamMember { .. }
        | RemoveTeamMember { .. }
        | ResumeTeamMemberEdit { .. }
        | ExpelFederatedTeam { .. }
        | RefreshFederatedSecurity { .. }
        | RunDueJobs { .. }
        | SyncYubiAccount {
            with_federation: true,
            ..
        } => Scope::SecurityRoot,
        AdmitFederatedTeam {
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

#[derive(Clone)]
struct Work {
    root: PathBuf,
    scope: Scope,
}
impl Work {
    fn conflicts(&self, other: &Self) -> bool {
        self.root == other.root && self.scope.overlaps(&other.scope)
    }
}
#[derive(Default)]
struct State {
    next: u64,
    waiting: VecDeque<(u64, Work)>,
    active: BTreeMap<u64, Work>,
}
#[derive(Default)]
pub(super) struct Coordinator {
    state: Mutex<State>,
    changed: Notify,
    blocking_changed: Condvar,
}
#[derive(Debug, Eq, PartialEq)]
pub(super) enum AdmissionError {
    Full,
    Deadline,
    Cancelled,
}
impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Full => "Profile request queue is full; operation did not start.",
            Self::Deadline => "Profile admission deadline exceeded; operation did not start.",
            Self::Cancelled => "Profile request cancelled before execution.",
        })
    }
}
impl std::error::Error for AdmissionError {}

pub(super) fn coordinator() -> &'static Arc<Coordinator> {
    static COORDINATOR: OnceLock<Arc<Coordinator>> = OnceLock::new();
    COORDINATOR.get_or_init(|| Arc::new(Coordinator::default()))
}

struct Pending {
    coordinator: Arc<Coordinator>,
    id: Option<u64>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.coordinator
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .waiting
                .retain(|(candidate, _)| *candidate != id);
            self.coordinator.wake();
        }
    }
}
pub(super) struct Permit {
    coordinator: Arc<Coordinator>,
    id: u64,
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.coordinator
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .remove(&self.id);
        self.coordinator.wake();
    }
}
impl Coordinator {
    fn wake(&self) {
        self.changed.notify_waiters();
        self.blocking_changed.notify_all();
    }
    fn enqueue(self: &Arc<Self>, root: &Path, scope: Scope) -> Result<Pending, AdmissionError> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_owned());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if scope != Scope::None
            && state
                .waiting
                .iter()
                .filter(|(_, work)| work.root == root)
                .count()
                >= MAX_QUEUED
        {
            return Err(AdmissionError::Full);
        }
        let id = state.next;
        state.next = state.next.wrapping_add(1);
        state.waiting.push_back((id, Work { root, scope }));
        Ok(Pending {
            coordinator: Arc::clone(self),
            id: Some(id),
        })
    }
    // FIFO among conflicting requests, irrespective of foreground/background.
    // Disjoint profiles may pass a blocked waiter; no partial multi-profile hold.
    fn admit(state: &mut State, id: u64) -> bool {
        let Some(index) = state
            .waiting
            .iter()
            .position(|(candidate, _)| *candidate == id)
        else {
            return false;
        };
        let work = &state.waiting[index].1;
        if state.active.values().any(|active| work.conflicts(active))
            || state
                .waiting
                .iter()
                .take(index)
                .any(|(_, earlier)| work.conflicts(earlier))
        {
            return false;
        }
        let (_, work) = state.waiting.remove(index).expect("located waiter exists");
        state.active.insert(id, work);
        true
    }
    #[cfg(test)]
    pub(super) fn try_acquire(
        self: &Arc<Self>,
        root: &Path,
        scope: Scope,
    ) -> Result<Option<Permit>, AdmissionError> {
        let mut pending = self.enqueue(root, scope)?;
        let id = pending.id.expect("new waiter");
        if Self::admit(
            &mut self.state.lock().unwrap_or_else(|e| e.into_inner()),
            id,
        ) {
            pending.id = None;
            Ok(Some(Permit {
                coordinator: Arc::clone(self),
                id,
            }))
        } else {
            Ok(None)
        }
    }
    pub(super) async fn acquire(
        self: &Arc<Self>,
        root: &Path,
        scope: Scope,
        timeout: Duration,
    ) -> Result<Permit, AdmissionError> {
        let mut pending = self.enqueue(root, scope)?;
        let id = pending.id.expect("new waiter");
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if tokio::time::Instant::now() >= deadline {
                return Err(AdmissionError::Deadline);
            }
            if Self::admit(
                &mut self.state.lock().unwrap_or_else(|e| e.into_inner()),
                id,
            ) {
                pending.id = None;
                return Ok(Permit {
                    coordinator: Arc::clone(self),
                    id,
                });
            }
            tokio::time::timeout_at(deadline, notified)
                .await
                .map_err(|_| AdmissionError::Deadline)?;
        }
    }
    pub(super) fn acquire_blocking(
        self: &Arc<Self>,
        root: &Path,
        scope: Scope,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<Permit, AdmissionError> {
        let mut pending = self.enqueue(root, scope)?;
        let id = pending.id.expect("new waiter");
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            // Release the mutex before Pending's destructor removes the waiter.
            if cancellation.is_cancelled() {
                drop(state);
                return Err(AdmissionError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                drop(state);
                return Err(AdmissionError::Deadline);
            }
            if Self::admit(&mut state, id) {
                pending.id = None;
                return Ok(Permit {
                    coordinator: Arc::clone(self),
                    id,
                });
            }
            (state, _) = self
                .blocking_changed
                .wait_timeout(state, remaining.min(CONTROL_POLL))
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

// Controls for external CLI file-lock contention. Only acquisition is retried;
// the checked-session callback remains FnOnce and errors from it are returned.
thread_local! {
    static CONTROL: RefCell<Option<(Instant, CancellationToken)>> = const { RefCell::new(None) };
}
pub(super) fn with_control<T>(
    timeout: Duration,
    cancellation: CancellationToken,
    work: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<(Instant, CancellationToken)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CONTROL.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore =
        Restore(CONTROL.with(|slot| slot.replace(Some((Instant::now() + timeout, cancellation)))));
    work()
}
pub(super) fn check_control() -> Result<(), AdmissionError> {
    CONTROL.with(|slot| {
        if let Some((deadline, cancellation)) = slot.borrow().as_ref() {
            if cancellation.is_cancelled() {
                return Err(AdmissionError::Cancelled);
            }
            if Instant::now() >= *deadline {
                return Err(AdmissionError::Deadline);
            }
        }
        Ok(())
    })
}
pub(super) fn wait_for_external_lock() -> Result<(), AdmissionError> {
    CONTROL.with(|slot| {
        let control = slot.borrow();
        let Some((deadline, cancellation)) = control.as_ref() else {
            return Err(AdmissionError::Full);
        };
        if cancellation.is_cancelled() {
            return Err(AdmissionError::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AdmissionError::Deadline);
        }
        std::thread::sleep(remaining.min(CONTROL_POLL));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, pin::Pin, task::Poll};

    const BUDGET: Duration = Duration::from_secs(5);
    async fn assert_pending<F: Future>(mut future: Pin<&mut F>) {
        std::future::poll_fn(|cx| {
            assert!(future.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }

    #[tokio::test]
    async fn same_profile_is_fifo_while_other_profiles_and_roots_proceed() {
        let coordinator = Arc::new(Coordinator::default());
        let root = Path::new("/admission-test");
        let first = coordinator
            .acquire(root, Scope::profile("a"), BUDGET)
            .await
            .unwrap();
        let mut second = Box::pin(coordinator.acquire(root, Scope::profile("a"), BUDGET));
        let mut third = Box::pin(coordinator.acquire(root, Scope::profile("a"), BUDGET));
        assert_pending(second.as_mut()).await;
        assert_pending(third.as_mut()).await;
        let other_profile = coordinator
            .acquire(root, Scope::profile("b"), BUDGET)
            .await
            .unwrap();
        let other_root = coordinator
            .acquire(Path::new("/another-root"), Scope::profile("a"), BUDGET)
            .await
            .unwrap();
        drop(first);
        let second = second.await.unwrap();
        assert_pending(third.as_mut()).await;
        drop(second);
        drop(third.await.unwrap());
        drop((other_profile, other_root));
        assert!(coordinator.state.lock().unwrap().active.is_empty());
    }

    #[test]
    fn profile_listing_does_not_turn_a_long_profile_operation_into_a_root_barrier() {
        let coordinator = Arc::new(Coordinator::default());
        let root = Path::new("/admission-test");
        // Represent an in-progress pairing on A without a timer or network wait.
        let pairing = coordinator
            .try_acquire(root, Scope::profile("a"))
            .unwrap()
            .unwrap();
        let listing = coordinator
            .try_acquire(root, operation_scope(&Operation::ListProfiles))
            .unwrap()
            .expect("registry listing must not wait for pairing");
        let second_listing = coordinator
            .try_acquire(root, operation_scope(&Operation::ListProfiles))
            .unwrap()
            .expect("readers can share the registry snapshot scope");
        let other = coordinator
            .try_acquire(root, Scope::profile("b"))
            .unwrap()
            .expect("listing must not block an independent profile");
        assert!(coordinator
            .try_acquire(root, Scope::profile("a"))
            .unwrap()
            .is_none());
        drop((pairing, listing, second_listing, other));
        assert!(coordinator.state.lock().unwrap().active.is_empty());
    }

    #[tokio::test]
    async fn registry_changes_remain_exclusive_and_readers_cannot_overtake_them() {
        let coordinator = Arc::new(Coordinator::default());
        let root = Path::new("/admission-test");
        for change in [
            Operation::SetProfileLabel {
                profile: "a".into(),
                label: Some("Work".into()),
            },
            Operation::RemoveProfile { name: "a".into() },
        ] {
            let first_read = coordinator
                .acquire(root, operation_scope(&Operation::ListProfiles), BUDGET)
                .await
                .unwrap();
            let mut writer = Box::pin(coordinator.acquire(root, operation_scope(&change), BUDGET));
            assert_pending(writer.as_mut()).await;
            let mut later_read = Box::pin(coordinator.acquire(
                root,
                operation_scope(&Operation::ListProfiles),
                BUDGET,
            ));
            assert_pending(later_read.as_mut()).await;
            drop(first_read);
            let writer = writer.await.unwrap();
            assert_pending(later_read.as_mut()).await;
            drop(writer);
            drop(later_read.await.unwrap());
        }
        assert!(coordinator.state.lock().unwrap().active.is_empty());
    }

    #[tokio::test]
    async fn multi_profile_reservation_is_atomic_and_cannot_be_overtaken() {
        let coordinator = Arc::new(Coordinator::default());
        let root = Path::new("/admission-test");
        let a = coordinator
            .acquire(root, Scope::profile("a"), BUDGET)
            .await
            .unwrap();
        let mut ab = Box::pin(coordinator.acquire(
            root,
            Scope::Profiles(vec!["a".into(), "b".into()]),
            BUDGET,
        ));
        assert_pending(ab.as_mut()).await;
        assert_eq!(
            coordinator.state.lock().unwrap().active.len(),
            1,
            "waiting pair holds neither profile"
        );
        let mut ba = Box::pin(coordinator.acquire(
            root,
            Scope::Profiles(vec!["b".into(), "a".into()]),
            BUDGET,
        ));
        assert_pending(ba.as_mut()).await;
        let unrelated = coordinator
            .acquire(root, Scope::profile("c"), BUDGET)
            .await
            .unwrap();
        drop(a);
        let ab = ab.await.unwrap();
        assert_pending(ba.as_mut()).await;
        drop(ab);
        drop(ba.await.unwrap());
        drop(unrelated);
    }

    #[tokio::test]
    async fn root_work_is_fair_with_foreground_and_releases_all_profiles() {
        let coordinator = Arc::new(Coordinator::default());
        let root = Path::new("/admission-test");
        let active = coordinator
            .acquire(root, Scope::profile("a"), BUDGET)
            .await
            .unwrap();
        let mut maintenance = Box::pin(coordinator.acquire(root, Scope::Root, BUDGET));
        assert_pending(maintenance.as_mut()).await;
        let mut foreground = Box::pin(coordinator.acquire(root, Scope::profile("b"), BUDGET));
        assert_pending(foreground.as_mut()).await;
        drop(active);
        let maintenance = maintenance.await.unwrap();
        assert_pending(foreground.as_mut()).await;
        // Non-locking health/recovery diagnostics remain admissible.
        drop(
            coordinator
                .acquire(root, Scope::None, BUDGET)
                .await
                .unwrap(),
        );
        drop(maintenance);
        drop(foreground.await.unwrap());
    }

    #[tokio::test]
    async fn cancelling_waiter_removes_its_reservation_without_running_it() {
        let coordinator = Arc::new(Coordinator::default());
        let root = Path::new("/admission-test");
        let active = coordinator
            .acquire(root, Scope::profile("a"), BUDGET)
            .await
            .unwrap();
        let mut abandoned = Box::pin(coordinator.acquire(root, Scope::Root, BUDGET));
        assert_pending(abandoned.as_mut()).await;
        drop(abandoned);
        drop(
            coordinator
                .acquire(root, Scope::profile("b"), BUDGET)
                .await
                .unwrap(),
        );
        assert!(coordinator.state.lock().unwrap().waiting.is_empty());
        drop(active);
    }

    #[tokio::test]
    async fn admission_deadline_and_queue_bounds_leave_no_partial_holds() {
        let coordinator = Arc::new(Coordinator::default());
        let root = Path::new("/admission-test");
        assert!(matches!(
            coordinator.acquire(root, Scope::Root, Duration::ZERO).await,
            Err(AdmissionError::Deadline)
        ));
        let waiters = (0..MAX_QUEUED)
            .map(|_| coordinator.enqueue(root, Scope::Root).unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            coordinator.enqueue(root, Scope::Root),
            Err(AdmissionError::Full)
        ));
        drop(
            coordinator
                .acquire(root, Scope::None, BUDGET)
                .await
                .unwrap(),
        );
        drop(waiters);
        let state = coordinator.state.lock().unwrap();
        assert!(state.waiting.is_empty());
        assert!(state.active.is_empty());
    }

    #[test]
    fn cancelled_background_wait_never_acquires_a_profile() {
        let coordinator = Arc::new(Coordinator::default());
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            coordinator.acquire_blocking(Path::new("/root"), Scope::Root, BUDGET, &cancellation),
            Err(AdmissionError::Cancelled)
        ));
        assert!(coordinator.state.lock().unwrap().waiting.is_empty());
        assert!(coordinator.state.lock().unwrap().active.is_empty());
    }

    #[test]
    fn adapter_upload_reserves_the_profile_its_execution_actually_uses() {
        let header = foks_agent_proto::KvUploadHeader {
            adapter: Some(foks_agent_proto::data::DataSubmission {
                scope: foks_agent_proto::data::DataScope {
                    profile: "adapter".into(),
                    account_alias: "account".into(),
                    host_id: "host".into(),
                    user_id: "user".into(),
                    team_id: None,
                },
                submission_id: "saved-operation".into(),
            }),
            store: KvStoreRef::Account(foks_agent_proto::AccountStoreRef {
                profile: "placeholder".into(),
                account_alias: "account".into(),
            }),
            path: "file".into(),
            total_length: 0,
            read_role: foks_agent_proto::KvRole::Owner,
            write_role: foks_agent_proto::KvRole::Owner,
            precondition: foks_agent_proto::KvPrecondition::Create,
            mkdir_p: false,
        };
        assert_eq!(
            operation_scope(&Operation::PutKvStream { header }),
            Scope::profile("adapter")
        );
    }

    #[test]
    fn local_metadata_proceeds_during_remote_work_but_serializes_with_itself_and_reset() {
        let coordinator = Arc::new(Coordinator::default());
        let root = Path::new("/local-metadata-test");
        let alias = Operation::SetLocalAccountAlias {
            profile: "a".into(),
            account_alias: "owner".into(),
            label: "Personal".into(),
        };
        for operation in [
            Operation::SyncAccount {
                profile: "a".into(),
                alias: "owner".into(),
            },
            Operation::RunDueJobs {
                profile: "a".into(),
            },
        ] {
            let remote = coordinator
                .try_acquire(root, operation_scope(&operation))
                .unwrap()
                .unwrap();
            let local = coordinator
                .try_acquire(root, operation_scope(&alias))
                .unwrap()
                .expect("local aliases must not wait for server work");
            assert!(coordinator
                .try_acquire(root, operation_scope(&alias))
                .unwrap()
                .is_none());
            assert!(coordinator
                .try_acquire(
                    root,
                    operation_scope(&Operation::RemoveProfile { name: "a".into() })
                )
                .unwrap()
                .is_none());
            drop((local, remote));
        }
        let reset = coordinator.try_acquire(root, Scope::Root).unwrap().unwrap();
        assert!(coordinator
            .try_acquire(root, operation_scope(&alias))
            .unwrap()
            .is_none());
        drop(reset);
    }

    #[test]
    fn operation_scopes_cover_pairs_cascades_and_metadata_reads() {
        assert_eq!(
            operation_scope(&Operation::ListDevices {
                profile: "a".into(),
                alias: "account".into()
            }),
            Scope::profile("a")
        );
        assert_eq!(
            operation_scope(&Operation::AdmitFederatedTeam {
                local_profile: "b".into(),
                local_team_alias: "team".into(),
                remote_profile: "a".into(),
                remote_team_alias: "team".into(),
                role: foks_agent_proto::FederationRole::Member,
                visibility: 0,
            }),
            Scope::Profiles(vec!["a".into(), "b".into()])
        );
        assert_eq!(
            operation_scope(&Operation::RunDueJobs {
                profile: "a".into()
            }),
            Scope::SecurityRoot
        );
    }
}
