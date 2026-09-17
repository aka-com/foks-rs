use crate::agent::AgentHandle;
use crate::commands::accounts::account_sync_response;
use crate::commands::context::AppState;
use crate::commands::execution::{ambiguous_mutation_response, map_mutation_error, MutationKind};
use std::sync::Arc;

#[test]
fn malformed_post_mutation_success_is_ambiguous_fatal_and_requires_refresh() {
    let state = AppState::new(Arc::new(AgentHandle::new(
        "/tmp/unused-foks-agent.sock".into(),
    )));
    let error = account_sync_response(serde_json::json!({
        "username": "",
        "user_chain_sequence": 1,
        "directories": 0,
        "entries": 0
    }))
    .map_err(|error| ambiguous_mutation_response(&state, error.message))
    .unwrap_err();
    assert_eq!(error.code, "response-binding");
    assert!(error.ambiguous);
    assert!(error.fatal);
    assert!(!error.retryable);
    let blocked = state.begin_mutation().unwrap_err();
    assert_eq!(blocked.code, "ambiguous");
}

#[test]
fn guarded_worker_in_one_profile_does_not_block_another_or_local_aliases() {
    use crate::commands::execution::apply_kv_mutation_with_transport;
    use crate::commands::tests::support::{account_ref, phase_four_state};
    use foks_agent_proto::{KvEntryMetadata, KvRole, Operation};
    use foks_desktop::{AgentTransport, CatalogItem, CatalogStoreRef};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Mutex,
    };
    use std::time::Duration;

    struct WorkerTransport {
        calls: AtomicUsize,
        pause: Option<(mpsc::Sender<()>, Mutex<mpsc::Receiver<()>>)>,
    }
    impl AgentTransport for WorkerTransport {
        fn call(
            &self,
            operation: Operation,
        ) -> Result<serde_json::Value, foks_desktop::AgentError> {
            assert!(matches!(operation, Operation::PutKv { .. }));
            self.calls.fetch_add(1, Ordering::AcqRel);
            if let Some((entered, resume)) = &self.pause {
                entered.send(()).unwrap();
                resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
                return Err(foks_desktop::AgentError::Ambiguous(
                    "unknown remote outcome".into(),
                ));
            }
            Ok(serde_json::Value::Null)
        }
    }
    let mutation = |profile| {
        foks_desktop::edit_kv_file_mutation(
            &CatalogItem {
                store: CatalogStoreRef::Account(account_ref(profile, "personal")),
                metadata: KvEntryMetadata {
                    path: "/note".into(),
                    version: 7,
                    node_type: "small-file".into(),
                    size: Some(1),
                    read_role: KvRole::Owner,
                    write_role: KvRole::Owner,
                },
            },
            b"changed".to_vec(),
        )
        .unwrap()
    };
    let state = phase_four_state(vec![]);
    let a = state.for_profile("work.example").unwrap();
    let b = state.for_profile("home.example").unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let transport_a = Arc::new(WorkerTransport {
        calls: AtomicUsize::new(0),
        pause: Some((entered_tx, Mutex::new(resume_rx))),
    });
    let transport_b = Arc::new(WorkerTransport {
        calls: AtomicUsize::new(0),
        pause: None,
    });
    let worker_state = a.clone();
    let worker_transport = transport_a.clone();
    let write_a = mutation("work.example");
    let worker = std::thread::spawn(move || {
        tauri::async_runtime::block_on(async {
            let _permit = worker_state.begin_mutation().unwrap();
            apply_kv_mutation_with_transport(
                &worker_state,
                write_a,
                MutationKind::Guarded,
                worker_transport,
            )
            .await
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(state.begin_mutation().is_err());
    tauri::async_runtime::block_on(async {
        let _permit = b.begin_mutation().unwrap();
        assert!(
            apply_kv_mutation_with_transport(
                &b,
                mutation("home.example"),
                MutationKind::Guarded,
                transport_b.clone()
            )
            .await
            .unwrap()
            .applied
        );
    });
    let aliases = state.for_local_aliases().unwrap();
    let local = aliases.begin_mutation().unwrap();
    let target = crate::commands::vault::store_id(&CatalogStoreRef::Account(account_ref(
        "work.example",
        "personal",
    )));
    assert!(aliases.local_account(&target).is_ok());
    drop(local);
    resume_tx.send(()).unwrap();
    let error = worker.join().unwrap().unwrap_err();
    assert!(error.ambiguous);
    assert!(!error.retryable);
    assert_eq!(a.begin_mutation().unwrap_err().code, "ambiguous");
    assert!(b.begin_mutation().is_ok());
    assert!(aliases.begin_mutation().is_ok());
    assert!(a.mutation_requires_refresh.load(Ordering::Acquire));
    assert_eq!(transport_a.calls.load(Ordering::Acquire), 1);
    assert_eq!(transport_b.calls.load(Ordering::Acquire), 1);
}

#[test]
fn mutation_failures_have_stable_ui_codes() {
    let conflict = || foks_desktop::AgentError::Protocol {
        code: foks_agent_proto::ErrorCode::Conflict,
        message: "precondition failed".to_owned(),
        fields: foks_agent_proto::ErrorFields::default(),
    };
    assert_eq!(
        map_mutation_error(conflict(), MutationKind::Create).code,
        "already-exists"
    );
    assert_eq!(
        map_mutation_error(conflict(), MutationKind::Guarded).code,
        "conflict"
    );
    let lease = map_mutation_error(
        foks_desktop::AgentError::Protocol {
            code: foks_agent_proto::ErrorCode::CapabilityDenied,
            message: "denied".to_owned(),
            fields: foks_agent_proto::ErrorFields {
                capability: Some("kv".to_owned()),
                ..Default::default()
            },
        },
        MutationKind::Guarded,
    );
    assert_eq!(lease.code, "capability-unavailable");
    assert_eq!(
        map_mutation_error(
            foks_desktop::AgentError::Transport("gone".to_owned()),
            MutationKind::Guarded,
        )
        .code,
        "agent-lost"
    );
    let ambiguous = map_mutation_error(
        foks_desktop::AgentError::Ambiguous("unknown outcome".to_owned()),
        MutationKind::Guarded,
    );
    assert_eq!(ambiguous.code, "ambiguous");
    assert!(ambiguous.ambiguous);
    assert!(!ambiguous.retryable);
}
