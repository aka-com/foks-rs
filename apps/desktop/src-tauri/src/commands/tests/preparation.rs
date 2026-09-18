use crate::agent::{AgentError, AgentHandle};
use crate::commands::context::AppState;
use crate::commands::execution::{execute_kv_mutation, MutationKind};
use crate::commands::preparation::{
    check_mutation_access, ensure_catalog_for_mutation, prepare_group_facts, GroupMutationFacts,
};
use crate::commands::tests::support::{account_ref, test_profile_value};
use crate::commands::vault::store_id;
use foks_agent_proto::{
    KvEntryMetadata, KvPage, KvRole, Operation, ProfileOverview, ResponseResult, TeamDetailsSummary,
};
use foks_desktop::{AgentTransport, CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

struct PreparationTransport {
    calls: Mutex<Vec<Operation>>,
    fail_catalog: bool,
    fail_items: bool,
    fail_members: bool,
    federated: bool,
    version: u64,
    access: Option<Arc<AtomicU64>>,
    pause_accounts: Option<(
        std::sync::mpsc::Sender<()>,
        Mutex<std::sync::mpsc::Receiver<()>>,
    )>,
    username: &'static str,
}

impl PreparationTransport {
    fn new(version: u64) -> Self {
        Self {
            calls: Mutex::new(vec![]),
            fail_catalog: false,
            fail_items: false,
            fail_members: false,
            federated: false,
            version,
            access: None,
            pause_accounts: None,
            username: "alice",
        }
    }
}

fn success(value: serde_json::Value) -> ResponseResult {
    ResponseResult::Success { value }
}

impl AgentTransport for PreparationTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ListProfiles if self.fail_catalog => Err(foks_desktop::AgentError::Transport("catalog unavailable".into())),
            Operation::ListProfiles => {
                if let Some(access) = &self.access { access.fetch_add(1, Ordering::AcqRel); }
                Ok(serde_json::json!([test_profile_value("work.example")]))
            },
            Operation::ListKnownStores { .. } => Ok(serde_json::json!([])),
            Operation::ListProfileOverview { profile } => Ok(serde_json::to_value(ProfileOverview {
                profile: profile.clone(),
                accounts: success(serde_json::json!([{"profile": profile, "alias":"personal", "username":"alice"}])),
                teams: success(serde_json::json!([])),
                server_status: success(serde_json::json!({
                    "profile": profile, "configured_probe": profile, "host": null,
                    "compatibility": {"status": "not-required"}, "chat_supported": null,
                })),
            }).unwrap()),
            Operation::ListKv { .. } if self.fail_items => Err(foks_desktop::AgentError::Transport("items unavailable".into())),
            Operation::ListKv { .. } => Ok(serde_json::to_value(KvPage {
                snapshot_version: 1,
                entries: vec![KvEntryMetadata {
                    path: "/note".into(), node_type: "small-file".into(), version: self.version,
                    size: Some(1), read_role: KvRole::Owner, write_role: KvRole::Owner,
                }],
                next_cursor: None,
            }).unwrap()),
            Operation::ListTeamDetails { .. } => Ok(serde_json::to_value(TeamDetailsSummary {
                members: if self.fail_members {
                    ResponseResult::Error { code: foks_agent_proto::ErrorCode::Busy, message:"members unavailable".into(), fields: Default::default() }
                } else if self.federated {
                    success(serde_json::json!([{
                        "username":null, "party_id_hex":"03".repeat(33), "party_kind":"named-team", "scoped_host_id_hex":"02".repeat(33),
                        "source_role":"admin", "destination_role":{"member":{"visibility":0}},
                        "generation":1, "locally_manageable":false
                    }]))
                } else {
                    success(serde_json::json!([{
                        "username":"bob", "party_id_hex":"01".repeat(33), "party_kind":"user", "scoped_host_id_hex":null,
                        "source_role":{"member":{"visibility":0}}, "destination_role":{"member":{"visibility":0}},
                        "generation":1, "locally_manageable":true
                    }]))
                },
                federation: success(if self.federated { serde_json::json!([{
                    "local_team_alias":"engineering", "remote_profile":"home.example", "remote_team_alias":"homelab",
                    "remote_host_id_hex":"02".repeat(33), "remote_team_id_hex":"03".repeat(33),
                    "destination":{"member":{"visibility":0}}, "operation_id_hex":"07".repeat(16), "active":true
                }]) } else { serde_json::json!([]) }),
            }).unwrap()),
            Operation::ListAccounts { profile } => {
                if let Some((entered, resume)) = &self.pause_accounts {
                    entered.send(()).unwrap();
                    resume.lock().unwrap().recv().unwrap();
                }
                Ok(serde_json::json!([{"profile":profile,"alias":"personal","username":self.username}]))
            }
            Operation::PutKv { .. } => Ok(serde_json::Value::Null),
            other => panic!("unexpected operation {other:?}"),
        }
    }
}

fn empty_state() -> AppState {
    AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )))
}

fn prepared_edit(
    state: &AppState,
    transport: Arc<PreparationTransport>,
    version: u64,
) -> Result<(), AgentError> {
    tauri::async_runtime::block_on(async {
        let unlocked = transport
            .access
            .as_ref()
            .map_or(0, |access| access.load(Ordering::Acquire));
        let permit = state.begin_mutation()?;
        ensure_catalog_for_mutation(state, &permit, transport.clone()).await?;
        let store = store_id(&CatalogStoreRef::Account(account_ref(
            "work.example",
            "personal",
        )));
        let item = state.selected_mutation_item(&store, "/note", version)?;
        let mutation = foks_desktop::edit_kv_file_mutation(&item, b"changed".to_vec()).unwrap();
        check_mutation_access(
            unlocked,
            Ok(transport
                .access
                .as_ref()
                .map_or(0, |access| access.load(Ordering::Acquire))),
        )?;
        state.invalidate_catalog();
        execute_kv_mutation(transport.as_ref(), mutation, MutationKind::Guarded)
    })
}

#[test]
fn ambiguity_reconciliation_requires_full_item_reads_for_root_and_profile() {
    for profile_only in [false, true] {
        let root = empty_state();
        let profile = root.for_profile("work.example").unwrap();
        let state = if profile_only {
            profile.clone()
        } else {
            root.clone()
        };
        profile
            .mutation_requires_refresh
            .store(true, Ordering::Release);
        state
            .mutation_requires_refresh
            .store(true, Ordering::Release);
        let stores = Arc::new(PreparationTransport::new(7));
        let catalog = foks_desktop::load_stores(stores.clone()).unwrap();
        assert!(catalog
            .inventory
            .iter()
            .all(|inventory| inventory.accounts_complete && inventory.teams_complete));
        assert!(catalog.failures.is_empty());
        assert!(!stores
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, Operation::ListKv { .. })));
        let (generation, _) = state.begin_catalog_load_checked().unwrap();
        assert!(state.accept_catalog(generation, catalog));
        assert!(state.mutation_requires_refresh.load(Ordering::Acquire));
        assert!(profile.mutation_requires_refresh.load(Ordering::Acquire));
        for fail_items in [true, false] {
            let mut transport = PreparationTransport::new(7);
            transport.fail_items = fail_items;
            let transport = Arc::new(transport);
            let (generation, token) = state.begin_catalog_load_checked().unwrap();
            let catalog = if profile_only {
                foks_desktop::load_profile_catalog_cancellable(
                    transport.clone(),
                    "work.example".into(),
                    token,
                )
            } else {
                foks_desktop::load_catalog_cancellable(transport.clone(), token)
            }
            .unwrap();
            assert!(transport
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|call| matches!(call, Operation::ListKv { .. })));
            assert_eq!(catalog.failures.is_empty(), !fail_items);
            assert!(state.accept_catalog(generation, catalog));
            assert_eq!(
                state.mutation_requires_refresh.load(Ordering::Acquire),
                fail_items
            );
            assert_eq!(
                profile.mutation_requires_refresh.load(Ordering::Acquire),
                fail_items
            );
        }
        assert!(root.begin_mutation().is_ok());
    }
}

#[test]
fn absent_catalog_is_loaded_before_exactly_one_version_bound_write() {
    let state = empty_state();
    let transport = Arc::new(PreparationTransport::new(7));
    prepared_edit(&state, transport.clone(), 7).unwrap();
    let calls = transport.calls.lock().unwrap();
    assert!(matches!(calls.first(), Some(Operation::ListProfiles)));
    assert!(matches!(calls.last(), Some(Operation::PutKv { .. })));
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, Operation::PutKv { .. }))
            .count(),
        1
    );
    assert!(state.begin_mutation().is_ok());
}

#[test]
fn missing_target_profile_catalog_never_joins_an_in_flight_or_ambiguous_profile() {
    struct IsolatedTransport {
        inner: PreparationTransport,
    }
    impl AgentTransport for IsolatedTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            if matches!(operation, Operation::ListProfiles) {
                self.inner.calls.lock().unwrap().push(operation);
                return Ok(serde_json::json!([
                    test_profile_value("work.example"),
                    test_profile_value("blocked.example")
                ]));
            }
            match &operation {
                Operation::ListKnownStores { profile }
                | Operation::ListProfileOverview { profile } => {
                    assert_eq!(profile, "work.example", "must not load the blocked profile");
                }
                _ => {}
            }
            self.inner.call(operation)
        }
    }
    let state = empty_state();
    let a = state.for_profile("blocked.example").unwrap();
    let b = state
        .for_store(&store_id(&CatalogStoreRef::Account(account_ref(
            "work.example",
            "personal",
        ))))
        .unwrap();
    let a_permit = a.begin_mutation().unwrap();
    let transport = Arc::new(IsolatedTransport {
        inner: PreparationTransport::new(7),
    });
    for ambiguous in [false, true] {
        if ambiguous {
            crate::commands::execution::ambiguous_worker_failure(&a, "unknown remote outcome");
        }
        tauri::async_runtime::block_on(async {
            let permit = b.begin_mutation().unwrap();
            ensure_catalog_for_mutation(&b, &permit, transport.clone())
                .await
                .unwrap();
            let target = store_id(&CatalogStoreRef::Account(account_ref(
                "work.example",
                "personal",
            )));
            let item = b.selected_mutation_item(&target, "/note", 7).unwrap();
            let mutation = foks_desktop::edit_kv_file_mutation(&item, b"changed".to_vec()).unwrap();
            b.invalidate_catalog();
            execute_kv_mutation(transport.as_ref(), mutation, MutationKind::Guarded).unwrap();
        });
    }
    drop(a_permit);
    assert_eq!(a.begin_mutation().unwrap_err().code, "ambiguous");
    assert_eq!(
        transport
            .inner
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, Operation::PutKv { .. }))
            .count(),
        2
    );
}

#[test]
fn scoped_preparation_rejects_a_permit_from_another_profile() {
    let state = empty_state();
    let a = state.for_profile("work.example").unwrap();
    let b = state.for_profile("home.example").unwrap();
    let permit = a.begin_mutation().unwrap();
    let error = tauri::async_runtime::block_on(ensure_catalog_for_mutation(
        &b,
        &permit,
        Arc::new(PreparationTransport::new(7)),
    ))
    .unwrap_err();
    assert_eq!(error.code, "invalid-request");
}

#[test]
fn failed_catalog_or_changed_version_never_dispatches_a_write() {
    for fail_catalog in [false, true] {
        let state = empty_state();
        let mut transport = PreparationTransport::new(8);
        transport.fail_catalog = fail_catalog;
        let transport = Arc::new(transport);
        let error = prepared_edit(&state, transport.clone(), 7).unwrap_err();
        if !fail_catalog {
            assert_eq!(error.code, "conflict");
        }
        assert!(!transport
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| matches!(call, Operation::PutKv { .. })));
        assert!(state.begin_mutation().is_ok());
    }
}

#[test]
fn mutation_reservation_fences_old_refresh_without_retiring_accepted_facts() {
    let state = empty_state();
    assert!(state.accept_catalog(0, CatalogSnapshot::default()));
    let accepted = state.catalog_at(None).unwrap().0;
    let (loading, _) = state.begin_catalog_load_checked().unwrap();
    let permit = state.begin_mutation().unwrap();
    assert!(!state.accept_catalog(loading, CatalogSnapshot::default()));
    assert_eq!(state.catalog_at(None).unwrap().0, accepted);
    assert!(state.catalog_at(None).unwrap().1.is_some());
    assert_eq!(
        state.begin_catalog_load_checked().unwrap_err().code,
        "mutation-in-flight"
    );
    let transport = Arc::new(PreparationTransport::new(7));
    tauri::async_runtime::block_on(ensure_catalog_for_mutation(
        &state,
        &permit,
        transport.clone(),
    ))
    .unwrap();
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[test]
fn foreign_permits_and_prior_ambiguous_writes_cannot_enter_preparation() {
    let state = empty_state();
    let foreign = empty_state();
    let permit = foreign.begin_mutation().unwrap();
    let transport = Arc::new(PreparationTransport::new(7));
    let error = tauri::async_runtime::block_on(ensure_catalog_for_mutation(
        &state,
        &permit,
        transport.clone(),
    ))
    .unwrap_err();
    assert_eq!(error.code, "invalid-request");
    state
        .mutation_requires_refresh
        .store(true, Ordering::Release);
    assert_eq!(
        prepared_edit(&state, transport.clone(), 7)
            .unwrap_err()
            .code,
        "ambiguous"
    );
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[test]
fn member_preparation_loads_native_facts_and_preserves_other_profile_accounts() {
    let state = empty_state();
    let team = foks_agent_proto::TeamStoreRef {
        profile: "work.example".into(),
        account_alias: "personal".into(),
        team_alias: "engineering".into(),
        team_id: "03".repeat(33),
    };
    let store = store_id(&CatalogStoreRef::Team(team.clone()));
    assert!(state.accept_catalog(
        0,
        CatalogSnapshot {
            profiles: vec!["work.example".into(), "unavailable.example".into()],
            stores: vec![
                CatalogStoreSummary::Account {
                    store: account_ref("work.example", "personal")
                },
                CatalogStoreSummary::Team {
                    store: team,
                    kind: "named".into(),
                    name: Some("Engineering".into()),
                    active: true,
                    creation_phase: None
                }
            ],
            ..Default::default()
        }
    ));
    let unrelated = crate::commands::accounts::AccountDto {
        local_alias: None,
        store: "unrelated".into(),
        profile: "unavailable.example".into(),
        alias: "personal".into(),
        username: "casey".into(),
    };
    state
        .retain_accounts(0, std::slice::from_ref(&unrelated))
        .unwrap();
    let permit = state.begin_mutation().unwrap();
    let transport = Arc::new(PreparationTransport::new(7));
    let prepared = tauri::async_runtime::block_on(prepare_group_facts(
        &state,
        &store,
        GroupMutationFacts::Members,
        transport.clone(),
    ))
    .unwrap();
    state
        .with_prepared_group_facts(&permit, prepared, || {
            state.selected_member_target(&store, "bob")
        })
        .unwrap();
    assert_eq!(
        state.accounts.lock().unwrap().get("unrelated"),
        Some(&unrelated)
    );
    assert!(transport.calls.lock().unwrap().iter().all(|call| matches!(call, Operation::ListTeamDetails {profile,..} | Operation::ListAccounts {profile} if profile == "work.example")));
}

#[test]
fn failed_required_group_facts_do_not_fall_back_to_cached_authorization() {
    let state = crate::commands::tests::support::phase_four_state(vec![]);
    let store = store_id(&CatalogStoreRef::Team(
        crate::commands::tests::support::team_ref("work.example", "personal", "engineering"),
    ));
    let cached = crate::commands::groups::PartyDto {
        store: store.clone(),
        username: Some("bob".into()),
        party_kind: "user".into(),
        generation: 0,
        locally_manageable: true,
        party_id_hex: "01".repeat(33),
        scoped_host_id_hex: None,
        source_role: KvRole::Member { visibility: 0 }.into(),
        destination_role: KvRole::Member { visibility: 0 }.into(),
    };
    state.retain_roster(0, store.clone(), &[cached]).unwrap();
    state
        .retain_accounts(
            0,
            &[crate::commands::accounts::AccountDto {
                local_alias: None,
                store: store_id(&CatalogStoreRef::Account(account_ref(
                    "work.example",
                    "personal",
                ))),
                profile: "work.example".into(),
                alias: "personal".into(),
                username: "alice".into(),
            }],
        )
        .unwrap();
    assert!(state.selected_member_target(&store, "bob").is_ok());
    let _permit = state.begin_mutation().unwrap();
    let mut transport = PreparationTransport::new(7);
    transport.fail_members = true;
    let transport = Arc::new(transport);
    let error = tauri::async_runtime::block_on(prepare_group_facts(
        &state,
        &store,
        GroupMutationFacts::Members,
        transport.clone(),
    ))
    .unwrap_err();
    assert_eq!(error.code, "busy");
    let error = tauri::async_runtime::block_on(prepare_group_facts(
        &state,
        &store,
        GroupMutationFacts::FederationRemoval,
        transport.clone(),
    ))
    .unwrap_err();
    assert_eq!(error.code, "busy");
    // A failed unrelated member read does not block a federation-only request.
    tauri::async_runtime::block_on(prepare_group_facts(
        &state,
        &store,
        GroupMutationFacts::Federation,
        transport.clone(),
    ))
    .unwrap();
    assert!(transport
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|call| matches!(call, Operation::ListTeamDetails { .. })));
}

#[test]
fn lock_generation_change_during_catalog_preparation_prevents_dispatch() {
    let state = empty_state();
    let mut transport = PreparationTransport::new(7);
    transport.access = Some(Arc::new(AtomicU64::new(1)));
    let transport = Arc::new(transport);
    let error = prepared_edit(&state, transport.clone(), 7).unwrap_err();
    assert_eq!(error.code, "app-locked");
    assert!(!transport
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| matches!(call, Operation::PutKv { .. })));
}

fn group_state() -> (AppState, String) {
    let state = empty_state();
    let team = foks_agent_proto::TeamStoreRef {
        profile: "work.example".into(),
        account_alias: "personal".into(),
        team_alias: "engineering".into(),
        team_id: "03".repeat(33),
    };
    let store = store_id(&CatalogStoreRef::Team(team.clone()));
    assert!(state.accept_catalog(0, CatalogSnapshot {
        profiles: vec!["work.example".into()],
        stores: vec![
            CatalogStoreSummary::Account { store: account_ref("work.example", "personal") },
            CatalogStoreSummary::Team { store: team, kind: "named".into(), name: Some("Engineering".into()), active:true, creation_phase:None },
        ],
        profile_overviews: vec![ProfileOverview {
            profile: "work.example".into(),
            accounts: success(serde_json::json!([{"profile":"work.example","alias":"personal","username":"cached-alice"}])),
            teams: success(serde_json::json!([])), server_status: success(serde_json::Value::Null),
        }],
        ..Default::default()
    }));
    (state, store)
}

#[test]
fn native_account_identity_supersedes_cached_overview_before_self_member_check() {
    let (state, store) = group_state();
    let permit = state.begin_mutation().unwrap();
    let mut transport = PreparationTransport::new(7);
    transport.username = "bob";
    let transport = Arc::new(transport);
    let prepared = tauri::async_runtime::block_on(prepare_group_facts(
        &state,
        &store,
        GroupMutationFacts::Members,
        transport.clone(),
    ))
    .unwrap();
    let error = state
        .with_prepared_group_facts(&permit, prepared, || {
            state.selected_member_target(&store, "bob")
        })
        .unwrap_err();
    assert_eq!(error.code, "member-not-actionable");
    assert!(transport
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|call| matches!(call, Operation::ListAccounts { .. })));
}

#[test]
fn older_roster_completion_cannot_replace_prepared_member_target() {
    let (state, store) = group_state();
    let state = Arc::new(state);
    let (entered, waiting) = std::sync::mpsc::channel();
    let (resume, released) = std::sync::mpsc::channel();
    let mut transport = PreparationTransport::new(7);
    transport.pause_accounts = Some((entered, Mutex::new(released)));
    let transport = Arc::new(transport);
    let worker_state = Arc::clone(&state);
    let worker_store = store.clone();
    let worker = std::thread::spawn(move || {
        let permit = worker_state.begin_mutation().unwrap();
        let prepared = tauri::async_runtime::block_on(prepare_group_facts(
            &worker_state,
            &worker_store,
            GroupMutationFacts::Members,
            transport,
        ))
        .unwrap();
        worker_state
            .with_prepared_group_facts(&permit, prepared, || {
                worker_state.selected_member_target(&worker_store, "bob")
            })
            .unwrap()
            .1
    });
    waiting
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    // A previously started metadata read returns while the final preflight
    // account read is blocked. This formerly overwrote the fresh roster.
    let old = crate::commands::groups::PartyDto {
        store: store.clone(),
        username: Some("bob".into()),
        party_kind: "user".into(),
        generation: 0,
        locally_manageable: true,
        party_id_hex: format!("01{}", "ab".repeat(32)),
        scoped_host_id_hex: None,
        source_role: KvRole::Owner.into(),
        destination_role: KvRole::Owner.into(),
    };
    state.retain_roster(0, store, &[old]).unwrap();
    resume.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), "01".repeat(33));
}

#[test]
fn federation_removal_prepares_both_facts_without_a_cached_roster() {
    let (state, store) = group_state();
    // Catalog replacement retires these facts; removal must rebuild both.
    assert!(state.rosters.lock().unwrap().is_empty());
    assert!(state.federations.lock().unwrap().is_empty());
    let permit = state.begin_mutation().unwrap();
    let mut transport = PreparationTransport::new(7);
    transport.federated = true;
    let prepared = tauri::async_runtime::block_on(prepare_group_facts(
        &state,
        &store,
        GroupMutationFacts::FederationRemoval,
        Arc::new(transport),
    ))
    .unwrap();
    assert!(prepared.parties.is_some());
    assert!(prepared.federation.is_some());
    // A late display read must not replace the roster owned by preparation.
    state.retain_roster(0, store.clone(), &[]).unwrap();
    let (_, entry) = state
        .with_prepared_group_facts(&permit, prepared, || {
            state.selected_active_federation_target(&store, &"02".repeat(33), &"03".repeat(33))
        })
        .unwrap();
    assert_eq!(entry.remote_team_alias, "homelab");
}
