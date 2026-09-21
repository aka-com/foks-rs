//! Shared transport preflight, mutation execution, and ambiguous-outcome handling.

use crate::agent::AgentError;
use crate::commands::context::AppState;
use crate::commands::enrollment::{pending_operations, validated_pending_dtos};
use crate::commands::servers::{require_transport_profile, transport_profile, ProfileSummary};
use crate::commands::types::MutationDto;
use crate::commands::validation::invalid_response;
use foks_agent_proto::{KvStoreRef, Operation, PendingOperationKind, PendingOperationSummary};
use foks_desktop::{CatalogStoreRef, KvAccountMutation};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MutationKind {
    Create,
    Guarded,
    Resume,
}

pub(super) fn ambiguous_mutation_response(
    state: &AppState,
    message: impl Into<String>,
) -> AgentError {
    state
        .mutation_requires_refresh
        .store(true, Ordering::Release);
    let mut error = AgentError::new("response-binding", message, false);
    error.ambiguous = true;
    error.fatal = true;
    error
}

pub(super) fn ambiguous_worker_failure(state: &AppState, message: impl Into<String>) -> AgentError {
    state
        .mutation_requires_refresh
        .store(true, Ordering::Release);
    let mut error = AgentError::new("ambiguous", message, false);
    error.ambiguous = true;
    error
}

pub(super) fn observe_mutation_error(state: &AppState, error: AgentError) -> AgentError {
    if error.code == "invalid-response" {
        return ambiguous_mutation_response(state, error.message);
    }
    if error.ambiguous {
        state
            .mutation_requires_refresh
            .store(true, Ordering::Release);
    }
    error
}

pub(super) fn map_mutation_error(
    error: foks_desktop::AgentError,
    kind: MutationKind,
) -> AgentError {
    let mut error = AgentError::from_desktop(error);
    if error.ambiguous {
        if matches!(error.code.as_str(), "cancelled" | "deadline-exceeded") {
            error.code = "ambiguous".to_owned();
        }
        error.retryable = false;
        return error;
    }
    if error.code == "capability-denied"
        && error
            .details
            .as_deref()
            .and_then(|details| details.capability.as_deref())
            == Some("kv")
    {
        // The protocol indicates KV is unavailable without distinguishing
        // between an expired lease or an ungranted capability.
        error.message =
            "This server is not granting vault access. Check its compatibility status before reading or writing."
                .to_owned();
        error.retryable = false;
        return error;
    }
    if error.code == "conflict" {
        error.code = match kind {
            MutationKind::Create => "already-exists",
            MutationKind::Guarded | MutationKind::Resume => "conflict",
        }
        .to_owned();
        error.retryable = false;
    }
    error
}

pub(super) fn execute_kv_mutation(
    transport: &dyn foks_desktop::AgentTransport,
    mutation: KvAccountMutation,
    kind: MutationKind,
) -> Result<(), AgentError> {
    match mutation {
        KvAccountMutation::Inline(operation) => {
            transport
                .call(operation)
                .map_err(|error| map_mutation_error(error, kind))?;
        }
        KvAccountMutation::Stream {
            header,
            mut content,
        } => {
            let mut reader = std::io::Cursor::new(content.as_mut_slice());
            transport
                .put_kv_stream(header, &mut reader)
                .map_err(|error| map_mutation_error(error, kind))?;
        }
    }
    Ok(())
}

pub(super) async fn apply_kv_mutation(
    state: &AppState,
    mutation: KvAccountMutation,
    kind: MutationKind,
) -> Result<MutationDto, AgentError> {
    apply_kv_mutation_with_transport(state, mutation, kind, state.agent.transport()).await
}

/// The store a KV write lands in. Every KV mutation names one; an operation
/// that names none is not a KV write and has no store's items to discard.
pub(super) fn kv_mutation_store(mutation: &KvAccountMutation) -> Option<CatalogStoreRef> {
    let store = match mutation {
        KvAccountMutation::Inline(
            Operation::PutKv { store, .. }
            | Operation::PutKvSymlink { store, .. }
            | Operation::MkdirKv { store, .. }
            | Operation::RemoveKv { store, .. },
        ) => store,
        KvAccountMutation::Inline(_) => return None,
        KvAccountMutation::Stream { header, .. } => &header.store,
    };
    Some(catalog_store(store))
}

pub(super) fn catalog_store(store: &KvStoreRef) -> CatalogStoreRef {
    match store {
        KvStoreRef::Account(store) => CatalogStoreRef::Account(store.clone()),
        KvStoreRef::Team(store) => CatalogStoreRef::Team(store.clone()),
    }
}

pub(super) async fn apply_kv_mutation_with_transport(
    state: &AppState,
    mutation: KvAccountMutation,
    kind: MutationKind,
    transport: std::sync::Arc<dyn foks_desktop::AgentTransport>,
) -> Result<MutationDto, AgentError> {
    match kv_mutation_store(&mutation) {
        Some(store) => state.invalidate_catalog_items(&store),
        // Not a KV write, so which items it changed is unknown. Retire the
        // whole catalog rather than leave any of it stale.
        None => state.invalidate_catalog(),
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
        execute_kv_mutation(transport.as_ref(), mutation, kind)
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The background operation stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(()) => Ok(MutationDto { applied: true }),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

pub(super) async fn apply_operation(
    state: &AppState,
    operation: Operation,
    kind: MutationKind,
) -> Result<MutationDto, AgentError> {
    apply_operation_value(state, operation, kind)
        .await
        .map(|_| MutationDto { applied: true })
}

pub(super) async fn apply_operation_value(
    state: &AppState,
    operation: Operation,
    kind: MutationKind,
) -> Result<serde_json::Value, AgentError> {
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        transport
            .call(operation)
            .map_err(|error| map_mutation_error(error, kind))
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The background operation stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

fn require_one_pending_operation(
    pending: &[PendingOperationSummary],
    kind: PendingOperationKind,
    alias: &str,
    target: Option<&str>,
) -> Result<(), AgentError> {
    let matches = pending
        .iter()
        .filter(|operation| {
            operation.kind == kind
                && operation.alias == alias
                && operation.target.as_deref() == target
        })
        .count();
    if matches == 1 {
        Ok(())
    } else if matches == 0 {
        Err(AgentError::new(
            "pending-operation-not-found",
            "Refresh the setup status before resuming this operation.",
            false,
        ))
    } else {
        Err(invalid_response(
            "The agent returned a duplicate pending-operation identity.",
        ))
    }
}

pub(super) fn execute_pending_operation(
    transport: &dyn foks_desktop::AgentTransport,
    profile: &str,
    pending_kind: PendingOperationKind,
    pending_alias: &str,
    pending_target: Option<&str>,
    operation: Operation,
) -> Result<serde_json::Value, AgentError> {
    let pending = pending_operations(transport, profile)?;
    validated_pending_dtos(&pending)?;
    require_one_pending_operation(&pending, pending_kind, pending_alias, pending_target)?;
    transport
        .call(operation)
        .map_err(|error| map_mutation_error(error, MutationKind::Resume))
}

pub(super) async fn apply_pending_operation_value(
    state: &AppState,
    profile: String,
    pending_kind: PendingOperationKind,
    pending_alias: String,
    pending_target: Option<String>,
    operation: Operation,
) -> Result<serde_json::Value, AgentError> {
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        execute_pending_operation(
            transport.as_ref(),
            &profile,
            pending_kind,
            &pending_alias,
            pending_target.as_deref(),
            operation,
        )
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The resume operation stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

pub(super) async fn read_profile_operation_value(
    state: &AppState,
    profile: String,
    operation: Operation,
) -> Result<serde_json::Value, AgentError> {
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        execute_read_profile_operation(transport.as_ref(), &profile, operation)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Failed to read setup details: {error}")))?
}

pub(super) fn execute_read_profile_operation(
    transport: &dyn foks_desktop::AgentTransport,
    profile: &str,
    operation: Operation,
) -> Result<serde_json::Value, AgentError> {
    require_transport_profile(transport, profile)?;
    transport.call(operation).map_err(AgentError::from_desktop)
}

pub(super) async fn apply_profile_operation_value(
    state: &AppState,
    profile: String,
    operation: Operation,
    kind: MutationKind,
) -> Result<serde_json::Value, AgentError> {
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        execute_profile_operation(transport.as_ref(), &profile, operation, kind)
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The setup operation stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

/// Runs a profile operation whose catalog effect is only known from its
/// reply, and retires the catalog only when the reply says a local record
/// changed. An operation whose outcome the reply cannot describe, including
/// every failure, retires the catalog as an unconditional mutation does.
///
/// Retiring the catalog for an operation that changed nothing costs every
/// reader that depends on it: the store identifiers a renderer holds are
/// rejected until it reloads, and long-running readers restart.
pub(super) async fn apply_surveying_profile_operation_value(
    state: &AppState,
    profile: String,
    operation: Operation,
    kind: MutationKind,
    changed: fn(&serde_json::Value) -> bool,
) -> Result<serde_json::Value, AgentError> {
    apply_surveying_profile_operation_with_transport(
        state,
        profile,
        operation,
        kind,
        changed,
        state.agent.transport(),
    )
    .await
}

pub(super) async fn apply_surveying_profile_operation_with_transport(
    state: &AppState,
    profile: String,
    operation: Operation,
    kind: MutationKind,
    changed: fn(&serde_json::Value) -> bool,
    transport: std::sync::Arc<dyn foks_desktop::AgentTransport>,
) -> Result<serde_json::Value, AgentError> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        execute_profile_operation(transport.as_ref(), &profile, operation, kind)
    })
    .await
    .map_err(|error| {
        state.invalidate_catalog();
        ambiguous_worker_failure(
            state,
            format!("The setup operation stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(value) => {
            if changed(&value) {
                state.invalidate_catalog();
            }
            Ok(value)
        }
        Err(error) => {
            // A failure reports no effect, and a partially applied operation
            // leaves records the catalog does not hold.
            state.invalidate_catalog();
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

pub(super) async fn apply_profile_operation_with_profile(
    state: &AppState,
    profile: String,
    operation: Operation,
    kind: MutationKind,
) -> Result<(serde_json::Value, ProfileSummary), AgentError> {
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<_, AgentError> {
        let configured = transport_profile(transport.as_ref(), &profile)?;
        let value = transport
            .call(operation)
            .map_err(|error| map_mutation_error(error, kind))?;
        Ok((value, configured))
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The server check stopped before reporting its outcome: {error}"),
        )
    })?;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

fn execute_profile_operation(
    transport: &dyn foks_desktop::AgentTransport,
    profile: &str,
    operation: Operation,
    kind: MutationKind,
) -> Result<serde_json::Value, AgentError> {
    require_transport_profile(transport, profile)?;
    transport
        .call(operation)
        .map_err(|error| map_mutation_error(error, kind))
}
