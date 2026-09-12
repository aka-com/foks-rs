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
    let mutation = action.is_mutation();
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
    require_catalog_generation(
        generation,
        state.catalog_generation.load(Ordering::Acquire),
        mutation,
    )?;
    if state.selected_store(&store_id)?.0 != CatalogStoreRef::Team(expected) {
        let mut error = invalid_response("The selected chat account changed.");
        error.fatal = true;
        return Err(error);
    }
    super::chat_local::remember(webview.app_handle(), &store_id, &reply.scope, generation);
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

fn require_catalog_generation(
    expected: u64,
    current: u64,
    mutation: bool,
) -> Result<(), AgentError> {
    if expected == current {
        return Ok(());
    }
    // Never publish across catalog generations, including a reused alias.
    let mut error = AgentError::new(
        "chat-restart",
        "Chat must revalidate after the catalog changed.",
        true,
    );
    error.ambiguous = mutation;
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channel_integrity_is_a_distinct_fatal_local_outcome() {
        let error = AgentError::from_agent(
            foks_agent_proto::ErrorCode::ChatChannelIntegrity,
            "bad message".into(),
        );
        assert_eq!(error.code, "chat-channel-integrity");
        assert!(error.fatal);
        assert!(!error.retryable);
    }

    #[test]
    fn catalog_reload_restarts_reads_but_preserves_mutation_ambiguity() {
        assert!(require_catalog_generation(7, 7, true).is_ok());
        let read = require_catalog_generation(7, 8, false).unwrap_err();
        assert_eq!(read.code, "chat-restart");
        assert!(read.retryable);
        assert!(!read.fatal);
        assert!(!read.ambiguous);
        let mutation = require_catalog_generation(7, 8, true).unwrap_err();
        assert!(mutation.ambiguous);
        assert!(!mutation.fatal);
    }
}

/// Explicit user navigation only. Never navigate the privileged webview remotely.
#[tauri::command]
pub async fn open_chat_link(
    webview: tauri::Webview,
    url: String,
) -> Result<serde_json::Value, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(webview.app_handle())?;
    let parsed = safe_external_url(&url).ok_or_else(|| invalid_request("Invalid chat link."))?;
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        let command = "open";
        #[cfg(all(unix, not(target_os = "macos")))]
        let command = "xdg-open";
        #[cfg(not(unix))]
        return Err(invalid_request(
            "Opening links is unavailable on this platform.",
        ));
        #[cfg(unix)]
        {
            let status = std::process::Command::new(command)
                .arg(parsed.as_str())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|_| invalid_request("Could not open the link."))?;
            if !status.success() {
                return Err(invalid_request("Could not open the link."));
            }
            Ok(serde_json::json!({"ok": true}))
        }
    })
    .await
    .map_err(|_| invalid_request("Link opening was interrupted."))?
}

fn safe_external_url(input: &str) -> Option<url::Url> {
    if input.len() > 2048
        || input
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }
    let parsed = url::Url::parse(input).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    Some(parsed)
}

#[cfg(test)]
mod link_tests {
    #[test]
    fn rejects_executable_obfuscated_and_credential_urls() {
        for bad in [
            "javascript:alert(1)",
            "java\nscript:alert(1)",
            "data:text/html,a",
            "file:///tmp/a",
            "https://user@example.com",
            " https://example.com",
            "https:\\example.com",
            "javascript%3Aalert(1)",
        ] {
            assert!(super::safe_external_url(bad).is_none(), "{bad}");
        }
        assert!(super::safe_external_url("https://example.com/a?q=b#c").is_some());
    }
}
