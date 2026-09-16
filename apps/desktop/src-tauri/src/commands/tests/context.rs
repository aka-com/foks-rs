use crate::agent::AgentHandle;
use crate::commands::accounts::{AccountDto, DeviceDto};
use crate::commands::context::AppState;
use crate::commands::tests::servers::ProfileListTransport;
use crate::commands::tests::support::{account_ref, phase_four_state, team_ref};
use crate::commands::vault::store_id;
use foks_desktop::{CatalogSnapshot, CatalogStoreRef, CatalogStoreSummary};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[test]
fn mutation_gate_refuses_a_second_write_until_the_first_finishes() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let first = state.begin_mutation().unwrap();
    assert_eq!(
        state.begin_mutation().unwrap_err().code,
        "mutation-in-flight"
    );
    assert_eq!(
        state.begin_catalog_load_checked().unwrap_err().code,
        "mutation-in-flight"
    );
    drop(first);
    assert!(state.begin_mutation().is_ok());
}

#[test]
fn ambiguous_mutations_require_a_fresh_catalog_before_another_write() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    state
        .mutation_requires_refresh
        .store(true, Ordering::Release);
    let error = state.begin_mutation().unwrap_err();
    assert_eq!(error.code, "ambiguous");
    assert!(error.ambiguous);
    let initialization = state.begin_initialization().unwrap();
    assert_eq!(
        state.begin_initialization().unwrap_err().code,
        "mutation-in-flight"
    );
    drop(initialization);
    assert!(state.mutation_requires_refresh.load(Ordering::Acquire));
    let (generation, _) = state.begin_catalog_load_checked().unwrap();
    assert!(state.accept_catalog(generation, CatalogSnapshot::default()));
    assert!(state.begin_mutation().is_ok());
}

#[test]
fn renderer_can_only_consume_paths_from_the_latest_native_drop_once() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let first = PathBuf::from("/tmp/first");
    let second = PathBuf::from("/tmp/second");
    assert_eq!(
        state.record_drop_paths(std::slice::from_ref(&first)),
        vec!["/tmp/first"]
    );
    assert_eq!(state.take_drop_path("/tmp/first").unwrap(), first);
    assert_eq!(
        state.take_drop_path("/tmp/first").unwrap_err().code,
        "drop-not-authorized"
    );
    state.record_drop_paths(&[second]);
    assert_eq!(
        state.take_drop_path("/tmp/first").unwrap_err().code,
        "drop-not-authorized"
    );
    let third = PathBuf::from("/tmp/third");
    let fourth = PathBuf::from("/tmp/fourth");
    assert_eq!(
        state.record_drop_paths(&[third, fourth]),
        vec!["/tmp/third", "/tmp/fourth"]
    );
    for path in ["/tmp/third", "/tmp/fourth"] {
        assert_eq!(
            state.take_drop_path(path).unwrap_err().code,
            "drop-not-authorized"
        );
    }
}

#[test]
fn known_store_metadata_never_authorizes_an_account_operation() {
    let state = phase_four_state(vec![]);
    let account = account_ref("work.example", "personal");
    *state.catalog.lock().unwrap() = Some(CatalogSnapshot {
        profiles: vec!["work.example".to_owned()],
        known_stores: vec![CatalogStoreSummary::Account {
            store: account.clone(),
        }],
        ..CatalogSnapshot::default()
    });
    assert_eq!(
        state
            .selected_account(&store_id(&CatalogStoreRef::Account(account)))
            .unwrap_err()
            .code,
        "store-not-found"
    );
}

#[test]
fn catalog_reads_bound_to_a_generation_fail_once_it_is_replaced() {
    let state = phase_four_state(vec![]);
    let (generation, catalog) = state.catalog_at(None).unwrap();
    assert!(catalog.is_some());
    assert!(state.catalog_at(Some(generation)).is_ok());
    // A replacement is private until publication; the accepted token and
    // facts stay valid while a refresh is running or fails.
    let (next, _token) = state.begin_catalog_load_checked().unwrap();
    assert_eq!(next, generation + 1);
    assert!(state.catalog_at(Some(generation)).unwrap().1.is_some());
    assert!(state.catalog_at(Some(next)).is_err());
    assert!(state.accept_catalog(next, CatalogSnapshot::default()));
    let stale = state.catalog_at(Some(generation)).unwrap_err();
    assert_eq!(stale.code, "catalog-required");
    assert!(stale.retryable);
    assert_eq!(state.catalog_at(Some(next)).unwrap().0, next);
}

#[test]
fn a_catalog_load_that_lost_its_generation_is_not_accepted() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let (first, _first_token) = state.begin_catalog_load_checked().unwrap();
    let (second, _second_token) = state.begin_catalog_load_checked().unwrap();
    assert!(!state.accept_catalog(first, CatalogSnapshot::default()));
    assert!(state.catalog.lock().unwrap().is_none());
    assert!(state.accept_catalog(second, CatalogSnapshot::default()));
    assert!(state.catalog.lock().unwrap().is_some());
}

#[test]
fn invitation_history_does_not_retire_catalog_or_compete_with_mutations() {
    use foks_agent_proto::invitations::InvitationAction;
    let state = phase_four_state(vec![]);
    let before = state.catalog_at(None).unwrap().0;
    let writer = state.begin_mutation().unwrap();
    for action in [
        InvitationAction::List,
        InvitationAction::PendingApprovals {
            team_alias: "team".into(),
        },
    ] {
        assert!(state
            .begin_catalog_action(action.changes_catalog())
            .unwrap()
            .is_none());
        assert_eq!(state.catalog_at(None).unwrap().0, before);
        assert!(state.catalog_at(None).unwrap().1.is_some());
    }
    drop(writer);
    let writer = state
        .begin_catalog_action(
            InvitationAction::Create {
                team_alias: "team".into(),
            }
            .changes_catalog(),
        )
        .unwrap();
    assert!(writer.is_some());
    assert!(state.catalog_at(None).unwrap().1.is_none());
}

#[test]
fn failed_refresh_preserves_store_selection_but_mutation_retires_it() {
    let state = phase_four_state(vec![]);
    let before = state.catalog_at(None).unwrap();
    // Use a real fixture target instead of depending on its display label.
    let target = before.1.as_ref().unwrap().stores[0].store_ref();
    let target_id = store_id(&target);
    let (_loading, token) = state.begin_catalog_load_checked().unwrap();
    token.cancel();
    assert!(state.selected_store(&target_id).is_ok());
    assert_eq!(state.catalog_at(None).unwrap(), before);
    state.invalidate_catalog();
    assert_eq!(
        state.selected_store(&target_id).unwrap_err().code,
        "catalog-required"
    );
}

#[test]
fn accepted_catalog_facts_survive_refresh_until_a_replacement_is_published() {
    let state = phase_four_state(vec![]);
    let (generation, catalog) = state.catalog_at(None).unwrap();
    let catalog = catalog.unwrap();
    let account = AccountDto {
        local_alias: None,
        store: store_id(&CatalogStoreRef::Account(account_ref(
            "work.example",
            "personal",
        ))),
        profile: "work.example".into(),
        alias: "personal".into(),
        username: "alice".into(),
    };
    let device = DeviceDto {
        id: format!("04{}", "11".repeat(32)),
        name: Some("Other Mac".into()),
        role: "owner",
        current: false,
    };
    let group = store_id(&CatalogStoreRef::Team(team_ref(
        "work.example",
        "personal",
        "engineering",
    )));
    let retain_facts = |generation| {
        state.retain_accounts(generation, std::slice::from_ref(&account))?;
        state.retain_devices(
            generation,
            account.store.clone(),
            std::slice::from_ref(&device),
        )?;
        // An empty response is still a retained fact. A missing map entry
        // requires a read before an operation can use this group.
        state.retain_group_details(generation, group.clone(), Some(&[]), Some(&[]))
    };
    let assert_retained_facts = || {
        assert_eq!(state.selected_account_dto(&account.store).unwrap(), account);
        assert!(state
            .selected_device_target(&account.store, &device.id)
            .is_ok());
        assert_eq!(state.rosters.lock().unwrap().get(&group), Some(&vec![]));
        assert_eq!(state.federations.lock().unwrap().get(&group), Some(&vec![]));
    };
    let assert_no_facts = || {
        assert!(state.accounts.lock().unwrap().is_empty());
        assert!(state.devices.lock().unwrap().is_empty());
        assert!(state.rosters.lock().unwrap().is_empty());
        assert!(state.federations.lock().unwrap().is_empty());
    };
    retain_facts(generation).unwrap();
    let (failed, failed_token) = state.begin_catalog_load_checked().unwrap();
    assert_retained_facts();
    failed_token.cancel();
    assert_retained_facts();
    assert_eq!(state.catalog_at(None).unwrap().0, generation);

    let (replacement, _token) = state.begin_catalog_load_checked().unwrap();
    assert!(!state.accept_catalog(failed, CatalogSnapshot::default()));
    assert_retained_facts();
    assert!(state.accept_catalog(replacement, catalog));
    assert_no_facts();
    assert_eq!(
        state.selected_account_dto(&account.store).unwrap_err().code,
        "accounts-required"
    );
    assert_eq!(
        state
            .selected_device_target(&account.store, &device.id)
            .unwrap_err()
            .code,
        "devices-required"
    );

    // Replies from before publication cannot reintroduce retired facts,
    // whether the caller fetched individual sections or combined details.
    assert_eq!(
        retain_facts(generation).unwrap_err().code,
        "catalog-required"
    );
    assert!(state
        .retain_devices(
            generation,
            account.store.clone(),
            std::slice::from_ref(&device)
        )
        .is_err());
    assert!(state.retain_roster(generation, group.clone(), &[]).is_err());
    assert!(state
        .retain_federation(generation, group.clone(), &[])
        .is_err());
    assert!(state
        .retain_group_details(generation, group.clone(), Some(&[]), Some(&[]))
        .is_err());
    assert_no_facts();
    retain_facts(replacement).unwrap();
    assert_retained_facts();
}

#[test]
fn mutation_cancels_a_private_catalog_load_and_rejects_its_late_reply() {
    let state = phase_four_state(vec![]);
    let (published, catalog) = state.catalog_at(None).unwrap();
    let (loading, token) = state.begin_catalog_load_checked().unwrap();
    assert!(loading > published);
    let transport = Arc::new(ProfileListTransport {
        value: serde_json::json!([]),
    });
    assert!(foks_desktop::load_catalog_cancellable(transport.clone(), token.clone()).is_ok());
    let mutation = state.begin_catalog_action(true).unwrap().unwrap();
    // The same transport returned a valid response before invalidation.
    assert!(matches!(
        foks_desktop::load_catalog_cancellable(transport, token),
        Err(foks_desktop::AgentError::Cancelled)
    ));
    let (invalidated, catalog_after_mutation) = state.catalog_at(None).unwrap();
    assert!(invalidated > loading);
    assert!(catalog_after_mutation.is_none());
    assert!(!state.accept_catalog(loading, catalog.clone().unwrap()));
    assert!(state.catalog_at(None).unwrap().1.is_none());
    assert!(state.begin_catalog_load_checked().is_err());
    drop(mutation);

    let (replacement, _token) = state.begin_catalog_load_checked().unwrap();
    assert!(state.accept_catalog(replacement, catalog.unwrap()));
    assert_eq!(state.catalog_at(None).unwrap().0, replacement);
    assert!(state.catalog_at(None).unwrap().1.is_some());
}
