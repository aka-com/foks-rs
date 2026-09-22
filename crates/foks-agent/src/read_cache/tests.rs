use super::*;

use foks_agent_proto::chat::ChatAction;
use foks_agent_proto::invitations::InvitationAction;
use foks_agent_proto::{AccountStoreRef, KvStoreRef, TeamStoreRef};
use foks_client_app::{Profile, ProfileRegistry, ProtocolPolicy, TrustRoot};
use foks_proto::{EntityId, Role};

fn entity(byte: u8) -> EntityId {
    let mut bytes = vec![foks_proto::ENTITY_USER];
    bytes.extend_from_slice(&[byte; 32]);
    EntityId::from_bytes(bytes).expect("entity identifier is well formed")
}

fn user_key(profile: &str, binding: u8) -> AuthCacheKey {
    AuthCacheKey {
        state_root: PathBuf::from("/state"),
        profile: profile.to_owned(),
        host_id: entity(0).as_bytes().to_vec(),
        uid: entity(1).as_bytes().to_vec(),
        binding: [binding; 32],
    }
}

fn team_key(profile: &str, team: u8) -> TeamViewCacheKey {
    TeamViewCacheKey::for_user(&user_key(profile, 7), &entity(team))
}

fn grant(generation: u64) -> TeamViewGrant {
    TeamViewGrant::new(Role::OWNER, generation, [generation as u8; 16])
}

/// The caches are process-wide, so the tests that clear them run one at a
/// time rather than wiping each other's entries.
fn shared_cache_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn team_store(profile: &str) -> TeamStoreRef {
    TeamStoreRef {
        profile: profile.to_owned(),
        account_alias: "owner".to_owned(),
        team_alias: "team".to_owned(),
        team_id: "00".to_owned(),
    }
}

#[test]
fn retained_entries_expire_after_their_lifetime() {
    let mut cache = Expiring::new(Duration::from_secs(30), 8);
    let start = Instant::now();
    cache.put_at(user_key("local", 1), 1_u32, start);
    assert_eq!(
        cache.get_at(&user_key("local", 1), start + Duration::from_secs(29)),
        Some(1)
    );
    assert_eq!(
        cache.get_at(&user_key("local", 1), start + Duration::from_secs(30)),
        None
    );
    assert_eq!(cache.len(), 0);
}

#[test]
fn retained_entries_are_bounded_oldest_first() {
    let mut cache = Expiring::new(Duration::from_secs(30), 3);
    let start = Instant::now();
    for index in 0..4 {
        cache.put_at(user_key("local", index), u32::from(index), start);
    }
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.get_at(&user_key("local", 0), start), None);
    assert_eq!(cache.get_at(&user_key("local", 3), start), Some(3));
}

#[test]
fn storing_a_key_again_replaces_its_entry() {
    let mut cache = Expiring::new(Duration::from_secs(30), 3);
    let start = Instant::now();
    cache.put_at(user_key("local", 1), 1_u32, start);
    cache.put_at(user_key("local", 1), 2_u32, start);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get_at(&user_key("local", 1), start), Some(2));
}

#[test]
fn eviction_drops_the_last_reference_to_a_retained_outcome() {
    let mut cache = Expiring::new(Duration::from_secs(30), 1);
    let start = Instant::now();
    let held = Arc::new(7_u32);
    cache.put_at(user_key("local", 1), Arc::clone(&held), start);
    assert_eq!(Arc::strong_count(&held), 2);
    cache.put_at(user_key("local", 2), Arc::new(8), start);
    assert_eq!(Arc::strong_count(&held), 1);
}

#[test]
fn a_retained_team_view_is_reused_until_its_profile_is_invalidated() {
    let _serialized = shared_cache_guard();
    invalidate_all();
    let caches = AgentReadCaches;
    TeamViewTokenCache::put(&caches, team_key("local", 2), grant(4));
    TeamViewTokenCache::put(&caches, team_key("other", 2), grant(5));
    assert_eq!(
        TeamViewTokenCache::get(&caches, &team_key("local", 2)).map(|grant| grant.generation()),
        Some(4)
    );

    TeamViewTokenCache::invalidate_profile(&caches, Path::new("/state"), "local");
    assert!(TeamViewTokenCache::get(&caches, &team_key("local", 2)).is_none());
    assert_eq!(
        TeamViewTokenCache::get(&caches, &team_key("other", 2)).map(|grant| grant.generation()),
        Some(5)
    );
    invalidate_all();
    assert!(TeamViewTokenCache::get(&caches, &team_key("other", 2)).is_none());
}

fn cache_key(profile: &str, path: &str, version: u64) -> KvNodeCacheKey {
    KvNodeCacheKey::new(&user_key(profile, 7), &entity(2), path, version)
}

fn cache_entry(node: u8) -> KvNodeCacheEntry {
    let mut node_id = [node; 17];
    node_id[0] = foks_proto::KvNodeType::File as u8;
    KvNodeCacheEntry {
        node_id,
        versions: foks_proto::KvPathVersionVector {
            root_version: 3,
            directories: Vec::new(),
        },
    }
}

/// A remembered path is scoped exactly like a retained view, and is dropped
/// by the same profile invalidation. It is never scoped by path alone: two
/// paths, two versions and two parties are separate entries.
#[test]
fn a_remembered_kv_path_is_scoped_and_dropped_with_its_profile() {
    let _serialized = shared_cache_guard();
    invalidate_all();
    let caches = AgentReadCaches;
    KvNodeCache::put(
        &caches,
        cache_key("local", "/a/large.bin", 4),
        cache_entry(1),
    );
    KvNodeCache::put(
        &caches,
        cache_key("other", "/a/large.bin", 4),
        cache_entry(2),
    );
    assert_eq!(
        KvNodeCache::get(&caches, &cache_key("local", "/a/large.bin", 4)),
        Some(cache_entry(1))
    );
    // A different version of the same path is a different entry, and so is
    // the same path in another profile.
    assert!(KvNodeCache::get(&caches, &cache_key("local", "/a/large.bin", 5)).is_none());
    assert!(KvNodeCache::get(&caches, &cache_key("local", "/a/other.bin", 4)).is_none());

    KvNodeCache::invalidate(&caches, &cache_key("local", "/a/large.bin", 4));
    assert!(KvNodeCache::get(&caches, &cache_key("local", "/a/large.bin", 4)).is_none());

    KvNodeCache::put(
        &caches,
        cache_key("local", "/a/large.bin", 4),
        cache_entry(1),
    );
    KvNodeCache::invalidate_profile(&caches, Path::new("/state"), "local");
    assert!(KvNodeCache::get(&caches, &cache_key("local", "/a/large.bin", 4)).is_none());
    assert_eq!(
        KvNodeCache::get(&caches, &cache_key("other", "/a/large.bin", 4)),
        Some(cache_entry(2))
    );
    invalidate_all();
    assert!(KvNodeCache::get(&caches, &cache_key("other", "/a/large.bin", 4)).is_none());
}

#[test]
fn a_refused_team_view_is_dropped_on_its_own() {
    let _serialized = shared_cache_guard();
    invalidate_all();
    let caches = AgentReadCaches;
    TeamViewTokenCache::put(&caches, team_key("local", 3), grant(1));
    TeamViewTokenCache::invalidate(&caches, &team_key("local", 3));
    assert!(TeamViewTokenCache::get(&caches, &team_key("local", 3)).is_none());
}

/// The three classes an operation can fall into. They are disjoint by
/// construction: an operation is served from the caches, or it leaves retained
/// material in place, or it drops everything.
#[derive(Debug, PartialEq, Eq)]
enum Class {
    Read,
    Leaves,
    Invalidates,
}

fn class(operation: &Operation) -> Class {
    let reads = operation_serves_reads(operation);
    let leaves = operation_leaves_retained_material(operation);
    assert!(
        !(reads && leaves),
        "two classes claim the same operation: {operation:?}"
    );
    match (reads, leaves) {
        (true, _) => Class::Read,
        (_, true) => Class::Leaves,
        _ => Class::Invalidates,
    }
}

#[test]
fn only_the_enumerated_operations_serve_reads() {
    let account = AccountStoreRef {
        profile: "local".to_owned(),
        account_alias: "owner".to_owned(),
    };
    for operation in [
        Operation::ListKv {
            store: account.clone(),
            cursor: None,
            limit: 1,
            fresh: false,
        },
        Operation::ReadKv {
            store: KvStoreRef::Account(account.clone()),
            path: "/a".to_owned(),
            version: 1,
        },
        Operation::ListTeamMembers {
            profile: "local".to_owned(),
            team_alias: "team".to_owned(),
        },
        Operation::DiscoverTeams {
            profile: "local".to_owned(),
            account_alias: "owner".to_owned(),
        },
        Operation::Chat {
            store: team_store("local"),
            action: ChatAction::Inbox,
        },
        Operation::Invitations {
            profile: "local".to_owned(),
            account_alias: "owner".to_owned(),
            action: InvitationAction::List,
            pin: None,
        },
    ] {
        assert_eq!(
            class(&operation),
            Class::Read,
            "expected a read: {operation:?}"
        );
    }
}

#[test]
fn mutating_operations_never_serve_reads() {
    for operation in [
        Operation::PutKv {
            store: KvStoreRef::Account(AccountStoreRef {
                profile: "local".to_owned(),
                account_alias: "owner".to_owned(),
            }),
            path: "/a".to_owned(),
            content: b"x".to_vec(),
            precondition: foks_agent_proto::KvPrecondition::Create,
            read_role: foks_agent_proto::KvRole::Owner,
            write_role: foks_agent_proto::KvRole::Owner,
            mkdir_p: false,
        },
        Operation::Chat {
            store: team_store("local"),
            action: ChatAction::MarkRead {
                channel: "00".to_owned(),
                sequence: "1".to_owned(),
            },
        },
        Operation::Chat {
            store: team_store("local"),
            action: ChatAction::PollInbox {
                since: "1".to_owned(),
                timeout_milliseconds: 1,
            },
        },
        Operation::Invitations {
            profile: "local".to_owned(),
            account_alias: "owner".to_owned(),
            action: InvitationAction::Accept {
                invite: "x".to_owned(),
            },
            pin: None,
        },
        Operation::RemoveProfile {
            name: "local".to_owned(),
        },
    ] {
        assert_eq!(
            class(&operation),
            Class::Invalidates,
            "expected a mutation: {operation:?}"
        );
    }
}

#[test]
fn reads_of_devices_keys_and_rosters_leave_retained_material_in_place() {
    for operation in [
        Operation::ListProfiles,
        Operation::ListKnownStores {
            profile: "local".to_owned(),
        },
        Operation::ListProfileOverview {
            profile: "local".to_owned(),
        },
        Operation::ListAccounts {
            profile: "local".to_owned(),
        },
        Operation::ListPendingOperations {
            profile: "local".to_owned(),
        },
        Operation::ListAccountRenames {
            profile: "local".to_owned(),
            account_alias: "owner".to_owned(),
        },
        Operation::ListDevices {
            profile: "local".to_owned(),
            alias: "owner".to_owned(),
        },
        Operation::ListBackupEnrollments {
            profile: "local".to_owned(),
            account_alias: "owner".to_owned(),
        },
        Operation::ListTeams {
            profile: "local".to_owned(),
        },
        Operation::ListYubiCards {
            profile: "local".to_owned(),
        },
        Operation::ListYubiAccounts {
            profile: "local".to_owned(),
        },
        Operation::YubiPinStatus {
            profile: "local".to_owned(),
            alias: "owner".to_owned(),
        },
        Operation::DescribeServerStatus {
            profile: "local".to_owned(),
        },
        Operation::DescribeResetHardState {
            profile: "local".to_owned(),
        },
        Operation::Probe {
            profile: "local".to_owned(),
        },
        Operation::ReconcileProfile {
            profile: "local".to_owned(),
        },
        Operation::RefreshLease {
            profile: "local".to_owned(),
        },
        Operation::Chat {
            store: team_store("local"),
            action: ChatAction::Status {
                operation: "1".to_owned(),
            },
        },
    ] {
        assert_eq!(
            class(&operation),
            Class::Leaves,
            "expected retained material to survive: {operation:?}"
        );
    }
}

/// The renderer's send service issues both of these every two seconds for
/// every team, whether or not anything is pending. Each one reads the local
/// pending-operation store and nothing else, so neither may be classified as
/// a mutation: that would drop every retained outcome and view twice per
/// tick and leave the caches serving almost nothing.
#[test]
fn idle_pending_chat_reads_leave_retained_material_in_place() {
    for action in [ChatAction::Pending, ChatAction::CleanupPending] {
        let operation = Operation::Chat {
            store: team_store("local"),
            action: action.clone(),
        };
        assert_eq!(
            class(&operation),
            Class::Leaves,
            "expected retained material to survive: {}",
            action.operation_name()
        );
    }
}

#[test]
fn an_operation_that_writes_durable_state_is_never_in_the_leaves_list() {
    // Each of these reads too, but each one also writes: a local account label
    // and a passphrase change land in the credential store, and a sync posts
    // to the server. They stay invalidating.
    for operation in [
        Operation::SetLocalAccountAlias {
            profile: "local".to_owned(),
            account_alias: "owner".to_owned(),
            label: "work".to_owned(),
        },
        Operation::SyncAccount {
            profile: "local".to_owned(),
            alias: "owner".to_owned(),
        },
        Operation::SetProfileLabel {
            profile: "local".to_owned(),
            label: Some("work".to_owned()),
        },
    ] {
        assert_eq!(
            class(&operation),
            Class::Invalidates,
            "expected a write: {operation:?}"
        );
    }
}

/// End to end through `dispatch_result`: only an operation outside both lists
/// drops what the caches hold. Each operation fails at the registry lookup for
/// an unknown profile, so nothing here touches the network.
///
/// The team view stands in for both caches: an authenticated outcome can only
/// be produced by verification, so it cannot be seeded here, and both caches
/// live in one `CacheState` that `invalidate_all` clears in a single call.
#[test]
fn only_an_unlisted_operation_drops_what_the_caches_hold() {
    let _serialized = shared_cache_guard();
    let directory = tempfile::tempdir().expect("temporary state directory");
    let state = directory.path().join("state");
    ProfileRegistry::open(&state).expect("registry opens");
    let caches = AgentReadCaches;
    let seed = || {
        invalidate_all();
        TeamViewTokenCache::put(&caches, team_key("local", 2), grant(4));
        assert!(TeamViewTokenCache::get(&caches, &team_key("local", 2)).is_some());
    };
    let dispatch = |operation| {
        crate::dispatch_result(
            &state,
            operation,
            Duration::from_secs(5),
            CancellationToken::new(),
            true,
        )
    };

    seed();
    assert!(dispatch(Operation::ListTeamMembers {
        profile: "missing".to_owned(),
        team_alias: "team".to_owned(),
    })
    .is_err());
    assert!(
        TeamViewTokenCache::get(&caches, &team_key("local", 2)).is_some(),
        "a read must not drop retained material"
    );

    seed();
    assert!(dispatch(Operation::ListDevices {
        profile: "missing".to_owned(),
        alias: "owner".to_owned(),
    })
    .is_err());
    assert!(
        TeamViewTokenCache::get(&caches, &team_key("local", 2)).is_some(),
        "a listed non-read must not drop retained material"
    );

    for action in [ChatAction::Pending, ChatAction::CleanupPending] {
        seed();
        let name = action.operation_name();
        assert!(dispatch(Operation::Chat {
            store: team_store("missing"),
            action,
        })
        .is_err());
        assert!(
            TeamViewTokenCache::get(&caches, &team_key("local", 2)).is_some(),
            "{name} runs on every idle tick and must not drop retained material"
        );
    }

    seed();
    assert!(dispatch(Operation::RemoveDevice {
        profile: "missing".to_owned(),
        signer_alias: "owner".to_owned(),
        device_id: "00".to_owned(),
    })
    .is_err());
    assert!(
        TeamViewTokenCache::get(&caches, &team_key("local", 2)).is_none(),
        "an unlisted operation must drop retained material"
    );
    invalidate_all();
}

#[test]
fn only_refusals_of_retained_material_request_a_retry() {
    for error in [
        foks_client::Error::KeyBinding("stale"),
        foks_client::Error::CredentialBinding("stale"),
        foks_client::Error::UserBinding("stale"),
        foks_client::Error::TeamBinding("stale"),
        // A chain anchored past the retained outcome's own Merkle position.
        // `foks_client::retryable_chain_load_error` already treats this as
        // recoverable staleness; the two lists answer different questions but
        // must not disagree about what staleness means.
        foks_client::Error::GenericChainRootChanged,
    ] {
        let error = foks_client_app::Error::Client(error);
        assert!(error_rejects_retained_material(&error), "{error}");
    }

    for error in [
        foks_client_app::Error::Client(foks_client::Error::DeadlineExceeded),
        foks_client_app::Error::Client(foks_client::Error::Cancelled),
        foks_client_app::Error::Client(foks_client::Error::KvResponse("too large")),
        foks_client_app::Error::KvConflict,
    ] {
        assert!(!error_rejects_retained_material(&error), "{error}");
    }
}

#[test]
fn only_refusals_of_the_credential_or_the_view_are_server_side_retries() {
    for code in [
        foks_rpc::STATUS_PERMISSION_ERROR,
        foks_rpc::STATUS_KEY_NOT_FOUND_ERROR,
        foks_rpc::STATUS_EXPIRED_ERROR,
        foks_rpc::STATUS_WRONG_USER_ERROR,
        foks_rpc::STATUS_TEAM_BEARER_TOKEN_STALE_ERROR,
        foks_rpc::STATUS_TEAM_KEY_ERROR,
        foks_rpc::STATUS_TEAM_NO_SRC_ROLE_ERROR,
    ] {
        assert!(status_rejects_material(code), "{code}");
    }
    for code in [
        foks_rpc::STATUS_RATE_LIMIT_ERROR,
        foks_rpc::STATUS_TIMEOUT_ERROR,
        foks_rpc::STATUS_KV_NOENT_ERROR,
        foks_rpc::STATUS_TEAM_RACE_ERROR,
        foks_rpc::STATUS_OK,
    ] {
        assert!(!status_rejects_material(code), "{code}");
    }
}

#[test]
fn only_a_read_that_was_served_retained_material_earns_a_retry() {
    let refusal = foks_client_app::Error::Client(foks_client::Error::KeyBinding("stale"));
    let other = foks_client_app::Error::KvConflict;
    let served = OperationTrace {
        served_from_cache: true,
        profiles: Vec::new(),
    };
    let authenticated = OperationTrace {
        served_from_cache: false,
        profiles: Vec::new(),
    };
    assert!(read_earns_a_retry(&served, &refusal));
    assert!(!read_earns_a_retry(&served, &other));
    assert!(!read_earns_a_retry(&authenticated, &refusal));
}

#[test]
fn a_chat_reuse_failure_is_classified_by_its_inner_error() {
    let error = foks_client_app::Error::Client(foks_client::Error::ReadReuse(Box::new(
        foks_client_app::Error::Client(foks_client::Error::KeyBinding("stale")),
    )));
    assert!(error_rejects_retained_material(&error));
    let error = foks_client_app::Error::Client(foks_client::Error::ReadReuse(Box::new(
        foks_client_app::Error::KvConflict,
    )));
    assert!(!error_rejects_retained_material(&error));
}

#[test]
fn an_operation_scope_reports_cache_hits_and_the_profiles_it_opened() {
    let (value, trace) = with_operation_scope(true, || {
        assert!(scope_serves_reads());
        note_profile(Path::new("/state"), "local");
        note_profile(Path::new("/state"), "local");
        note_profile(Path::new("/state"), "remote");
        note_cache_hit();
        7
    });
    assert_eq!(value, 7);
    assert!(trace.served_from_cache);
    assert_eq!(
        trace.profiles,
        vec![
            (PathBuf::from("/state"), "local".to_owned()),
            (PathBuf::from("/state"), "remote".to_owned()),
        ]
    );
}

#[test]
fn work_outside_an_operation_is_never_served_from_a_cache() {
    assert!(!scope_serves_reads());
    let (_, trace) = with_operation_scope(false, || {
        assert!(!scope_serves_reads());
        note_cache_hit();
    });
    // The hit is still recorded, but a non-read scope never attaches a cache
    // for one to come from, and its epilogue invalidates rather than retries.
    assert!(trace.served_from_cache);
    assert!(!scope_serves_reads());
}

#[test]
fn one_base_transport_serves_a_profile_until_it_is_dropped() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path().join("state");
    let mut registry = ProfileRegistry::open(&root).expect("registry opens");
    registry
        .add(Profile {
            name: "local".to_owned(),
            label: None,
            probe: "127.0.0.1:1".to_owned(),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::WebPki,
        })
        .expect("profile is added");

    let first = base_client(&registry, "local").expect("base client");
    let second = base_client(&registry, "local").expect("base client");
    assert!(first.shares_connection_pool_with(&second));

    invalidate_base_clients();
    let third = base_client(&registry, "local").expect("base client");
    assert!(!first.shares_connection_pool_with(&third));
}

#[test]
fn a_session_carries_read_caches_only_inside_a_read_scope() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path().join("state");
    let mut registry = ProfileRegistry::open(&root).expect("registry opens");
    registry
        .add(Profile {
            name: "local".to_owned(),
            label: None,
            probe: "127.0.0.1:1".to_owned(),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::WebPki,
        })
        .expect("profile is added");

    let open = || {
        open_profile_session(
            &registry,
            "local",
            Duration::from_secs(5),
            CancellationToken::new(),
        )
        .expect("session opens")
    };
    let (read, _) = with_operation_scope(true, open);
    assert!(read.serves_reads_from_cache());
    let (mutation, _) = with_operation_scope(false, open);
    assert!(!mutation.serves_reads_from_cache());
    assert!(!open().serves_reads_from_cache());
}

#[test]
fn network_chat_state_keeps_exclusive_profile_admission() {
    let store = TeamStoreRef {
        profile: "local".into(),
        account_alias: "owner".into(),
        team_alias: "team".into(),
        team_id: format!("03{}", "ab".repeat(32)),
    };
    for action in [
        ChatAction::Channels,
        ChatAction::Inbox,
        ChatAction::SyncInbox {
            blocked_channels: vec![],
        },
        ChatAction::History {
            after: None,
            channel: "12".repeat(16),
            before: None,
        },
        ChatAction::NotificationHistory {
            channel: "12".repeat(16),
            before: None,
        },
    ] {
        assert!(!operation_shares_profile(&Operation::Chat {
            store: store.clone(),
            action
        }));
    }
    for action in [ChatAction::Pending, ChatAction::CleanupPending] {
        assert!(operation_shares_profile(&Operation::Chat {
            store: store.clone(),
            action
        }));
    }
}
