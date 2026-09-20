//! Chat outcomes are scoped to their durable operations, without invalidating the vault.
use super::{
    context::AppState,
    validation::{invalid_request, invalid_response, require_main_window},
};
use crate::agent::AgentError;
use foks_agent_proto::chat::{ChatAction, ChatReply, CHAT_OPEN_VIEWS};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
};
use tauri::{Manager as _, State};

type ChatViews = HashMap<String, Weak<AtomicBool>>;

fn open_chat_view(views: &mut ChatViews, view_id: &str) -> Option<Arc<AtomicBool>> {
    views.retain(|_, token| token.strong_count() > 0);
    if let Some(token) = views.get(view_id).and_then(Weak::upgrade) {
        return Some(token);
    }
    views.remove(view_id);
    if views.len() >= CHAT_OPEN_VIEWS {
        return None;
    }
    let token = Arc::new(AtomicBool::new(false));
    views.insert(view_id.to_owned(), Arc::downgrade(&token));
    Some(token)
}

fn cancel_chat_view(views: &mut ChatViews, view_id: &str) {
    if let Some(token) = views.remove(view_id).and_then(|token| token.upgrade()) {
        token.store(true, Ordering::Release);
    }
}

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
    // Passive only: never open the old Keychain namespace from a chat view.
    if matches!(&action, ChatAction::ImportIntent { .. }) {
        return Err(invalid_request(
            "Legacy imports are owned by saved message recovery.",
        ));
    }
    if matches!(
        &action,
        ChatAction::LoadIntent { .. }
            | ChatAction::SaveIntent { .. }
            | ChatAction::PrepareMessage { .. }
            | ChatAction::SubmitMessage { .. }
            | ChatAction::Attempt { .. }
            | ChatAction::Reconcile { .. }
    ) {
        super::chat_migration::require_recovered(webview.app_handle())?;
    }
    let state = state.for_store(&store_id)?;
    let mutation = action.is_mutation();
    let (generation, store) = state.selected_chat(&store_id)?;
    let expected = store.clone();
    let transport = state
        .agent
        .transport_for("chat_request", Some(&store.profile));
    let token = open_chat_view(
        &mut state
            .chat_views
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        &view_id,
    )
    .ok_or_else(|| invalid_request("Too many open chat views."))?;
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
        state.chat_generation.load(Ordering::Acquire),
        mutation,
    )?;
    if reply.scope.store != expected {
        let mut error = invalid_response("The selected chat account changed.");
        error.fatal = true;
        return Err(error);
    }
    state
        .accept_chat_scope(&store_id, generation, &reply.scope)
        .map_err(|mut error| {
            error.ambiguous = mutation;
            error
        })?;
    super::chat_local::remember(
        webview.app_handle(),
        &store_id,
        &reply.scope,
        generation,
        unlocked,
    );
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
    cancel_chat_view(
        &mut state
            .chat_views
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        &view_id,
    );
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
    fn recovery_native_contract_preserves_local_mutation_classification() {
        let reconcile: ChatAction = serde_json::from_value(serde_json::json!({
            "action": "reconcile", "operation": "ab".repeat(16)
        }))
        .unwrap();
        assert!(reconcile.validate());
        assert!(reconcile.is_mutation());
        assert!(
            require_catalog_generation(1, 2, reconcile.is_mutation())
                .unwrap_err()
                .ambiguous
        );
        let cleanup: ChatAction = serde_json::from_value(serde_json::json!({
            "action": "cleanup-pending"
        }))
        .unwrap();
        assert!(cleanup.validate());
        assert!(!cleanup.is_mutation());
        assert!(
            !require_catalog_generation(1, 2, cleanup.is_mutation())
                .unwrap_err()
                .ambiguous
        );
    }

    fn view_id(index: usize) -> String {
        format!("{:032x}", index + 1)
    }

    #[test]
    fn completed_views_do_not_exhaust_open_view_capacity() {
        let mut views = ChatViews::new();
        for index in 0..CHAT_OPEN_VIEWS * 2 {
            drop(open_chat_view(&mut views, &view_id(index)).unwrap());
        }
        assert!(open_chat_view(&mut views, &view_id(CHAT_OPEN_VIEWS * 2)).is_some());
    }

    #[test]
    fn live_views_exhaust_capacity_until_one_finishes() {
        let mut views = ChatViews::new();
        let mut live = (0..CHAT_OPEN_VIEWS)
            .map(|index| open_chat_view(&mut views, &view_id(index)).unwrap())
            .collect::<Vec<_>>();
        let next = view_id(CHAT_OPEN_VIEWS);
        assert!(open_chat_view(&mut views, &next).is_none());
        live.pop();
        assert!(open_chat_view(&mut views, &next).is_some());
    }

    #[test]
    fn one_view_shares_and_cancels_its_live_requests() {
        let mut views = ChatViews::new();
        let id = view_id(0);
        let first = open_chat_view(&mut views, &id).unwrap();
        let second = open_chat_view(&mut views, &id).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(views.len(), 1);
        cancel_chat_view(&mut views, &id);
        assert!(first.load(Ordering::Acquire));
        assert!(second.load(Ordering::Acquire));
        assert!(views.is_empty());
    }

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
    fn unrelated_profile_updates_do_not_retire_chat_but_own_profile_and_root_changes_do() {
        let state = AppState::new(Arc::new(crate::agent::AgentHandle::new(
            "/tmp/unused-chat-agent.sock".into(),
        )));
        let store = serde_json::json!({"kind":"team", "profile":"chat", "accountAlias":"owner", "teamAlias":"team", "teamId":"team"}).to_string();
        let chat = state.for_store(&store).unwrap();
        let generation = chat.chat_generation.load(Ordering::Acquire);
        state.for_profile("other").unwrap().invalidate_catalog();
        assert!(require_catalog_generation(
            generation,
            chat.chat_generation.load(Ordering::Acquire),
            true
        )
        .is_ok());
        chat.invalidate_catalog();
        assert!(
            require_catalog_generation(
                generation,
                chat.chat_generation.load(Ordering::Acquire),
                true
            )
            .unwrap_err()
            .ambiguous
        );
        let generation = chat.chat_generation.load(Ordering::Acquire);
        state.invalidate_catalog();
        assert!(require_catalog_generation(
            generation,
            chat.chat_generation.load(Ordering::Acquire),
            false
        )
        .is_err());
    }

    #[test]
    fn progressive_sibling_catalog_keeps_the_chat_profile_generation() {
        let state = AppState::new(Arc::new(crate::agent::AgentHandle::new(
            "/tmp/unused-progress-agent.sock".into(),
        )));
        let chat = state.for_profile("chat").unwrap();
        let (load, _) = state.begin_catalog_load_checked().unwrap();
        let mut snapshot = foks_desktop::CatalogSnapshot {
            profiles: vec!["chat".into(), "other".into()],
            inventory: vec![foks_desktop::CatalogInventoryState {
                profile: "chat".into(),
                accounts_complete: true,
                teams_complete: true,
            }],
            full_item_reads: Some(vec!["chat".into()]),
            ..Default::default()
        };
        assert!(state.publish_catalog(load, snapshot.clone(), |_| {}));
        let generation = chat.chat_generation.load(Ordering::Acquire);
        snapshot
            .inventory
            .push(foks_desktop::CatalogInventoryState {
                profile: "other".into(),
                accounts_complete: true,
                teams_complete: true,
            });
        snapshot
            .full_item_reads
            .as_mut()
            .unwrap()
            .push("other".into());
        assert!(state.publish_catalog(load, snapshot, |_| {}));
        assert!(
            require_catalog_generation(
                generation,
                chat.chat_generation.load(Ordering::Acquire),
                true
            )
            .is_ok(),
            "an unrelated profile's progress must not turn chat delivery ambiguous"
        );
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
