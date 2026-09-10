//! Chat outcomes are scoped to their durable operations, without invalidating the vault.
use super::{
    context::AppState,
    validation::{invalid_request, invalid_response, require_main_window},
};
use crate::agent::AgentError;
use foks_agent_proto::chat::{ChatAction, ChatReply, CHAT_OPEN_VIEWS};
use foks_desktop::CatalogStoreRef;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Manager as _, State};

#[tauri::command]
pub async fn chat_request(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    action: ChatAction,
    view_id: String,
) -> Result<ChatReply, AgentError> {
    require_main_window(&webview)?;
    let unlocked = crate::applock::unlocked_generation(webview.app_handle())?;
    if !action.validate() || !foks_agent_proto::chat::valid_chat_id(&view_id) {
        return Err(invalid_request("Invalid chat request."));
    }
    let generation = state.catalog_generation.load(Ordering::Acquire);
    let (CatalogStoreRef::Team(store), Some(true)) = state.selected_store(&store_id)? else {
        return Err(invalid_request("Chat requires an active named team."));
    };
    let expected = store.clone();
    let transport = state.agent.transport();
    let token = {
        let mut views = state
            .chat_views
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if views.len() >= CHAT_OPEN_VIEWS && !views.contains_key(&view_id) {
            return Err(invalid_request("Too many open chat views."));
        }
        views
            .entry(view_id)
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    };
    let app = webview.app_handle().clone();
    let reply = tauri::async_runtime::spawn_blocking(move || {
        foks_desktop::chat_request_cancellable(transport.as_ref(), store, action, &|| {
            token.load(Ordering::Acquire)
                || crate::applock::require_unlocked_generation(&app, unlocked).is_err()
        })
        .map_err(AgentError::from_desktop)
    })
    .await
    .map_err(|_| {
        let mut error = AgentError::new(
            "chat-interrupted",
            "Chat was interrupted. Check pending operations before submitting again.",
            true,
        );
        error.ambiguous = true;
        error
    })??;
    require_main_window(&webview)?;
    crate::applock::require_unlocked_generation(webview.app_handle(), unlocked)?;
    if state.catalog_generation.load(Ordering::Acquire) != generation
        || state.selected_store(&store_id)?.0 != CatalogStoreRef::Team(expected)
    {
        return Err(invalid_response("The selected chat account changed."));
    }
    Ok(reply)
}

/// Close foreground requests for one view without changing durable operations.
#[tauri::command]
pub fn cancel_chat_requests(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    view_id: String,
) -> Result<(), AgentError> {
    require_main_window(&webview)?;
    if let Some(token) = state
        .chat_views
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&view_id)
    {
        token.store(true, Ordering::Release);
    }
    Ok(())
}
