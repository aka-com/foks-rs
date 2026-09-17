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
