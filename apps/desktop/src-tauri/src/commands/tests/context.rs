use crate::agent::AgentHandle;
use crate::commands::context::AppState;
use crate::commands::tests::support::{account_ref, phase_four_state};
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
    state.accept_catalog(0, CatalogSnapshot::default());
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
    // A later load clears the snapshot and moves the generation on.
    let (next, _token) = state.begin_catalog_load_checked().unwrap();
    assert_eq!(next, generation + 1);
    let stale = state.catalog_at(Some(generation)).unwrap_err();
    assert_eq!(stale.code, "catalog-required");
    assert!(stale.retryable);
    let (current, catalog) = state.catalog_at(Some(next)).unwrap();
    assert_eq!(current, next);
    assert!(catalog.is_none());
    // Reads that do not name a generation keep answering from whatever is current.
    assert!(state.catalog_at(None).is_ok());
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
