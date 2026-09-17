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
