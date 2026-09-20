//! The only reader of the retired desktop credential namespace. No source
//! lock is held across agent IPC, and only acknowledged records are removed.
use super::{chat_local, context::AppState, vault::store_id};
use crate::agent::AgentError;
use foks_agent_proto::{
    chat::{ChatAction, ChatScope},
    SecretString,
};
use foks_client_app::LocalChatIntentStore;
use foks_desktop::{CatalogStoreRef, CatalogStoreSummary};
use std::sync::atomic::Ordering;

fn unavailable() -> AgentError {
    AgentError::new("chat-migration", "Saved messages still need recovery. Retry saved message recovery before sending. Existing messages have been kept.", true)
}
pub(super) fn require_recovered(app: &tauri::AppHandle) -> Result<(), AgentError> {
    let path = chat_local::intent_path(app)?;
    if LocalChatIntentStore::has_existing(&path).map_err(|_| unavailable())? {
        return Err(unavailable());
    }
    LocalChatIntentStore::retire_empty(&path).map_err(|_| unavailable())?;
    Ok(())
}

pub(super) fn recover(
    app: &tauri::AppHandle,
    state: &AppState,
    unlocked: u64,
) -> Result<usize, AgentError> {
    crate::applock::require_unlocked_generation(app, unlocked)?;
    let path = chat_local::intent_path(app)?;
    if !LocalChatIntentStore::has_existing(&path).map_err(|_| unavailable())? {
        LocalChatIntentStore::retire_empty(&path).map_err(|_| unavailable())?;
        return Ok(0);
    }
    let mut legacy = LocalChatIntentStore::open_existing(&path).map_err(|_| unavailable())?;
    let records = legacy.existing_records().map_err(|_| unavailable())?;
    legacy.release_lock();
    crate::applock::require_unlocked_generation(app, unlocked)?;
    let candidates = {
        let catalog = state.catalog.lock().map_err(|_| unavailable())?;
        let catalog = catalog.as_ref().ok_or_else(unavailable)?;
        catalog
            .stores
            .iter()
            .filter_map(|entry| match entry {
                CatalogStoreSummary::Team {
                    store,
                    active: true,
                    kind,
                    ..
                } if kind == "named" && !catalog.profile_blocked(&store.profile) => {
                    Some(store.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    if candidates.len() > 4096 {
        return Err(unavailable());
    }
    let mut remaining = 0;
    for record in records {
        crate::applock::require_unlocked_generation(app, unlocked)?;
        let (old, channel): (ChatScope, String) =
            serde_json::from_slice(&record.binding).map_err(|_| unavailable())?;
        if old.store.profile != record.profile {
            return Err(unavailable());
        }
        let probe = ChatAction::LoadIntent {
            host: old.host.clone(),
            actor: old.actor.clone(),
            channel: channel.clone(),
        };
        if !probe.validate() {
            return Err(unavailable());
        }
        // Aliases route the request, but cannot authorize it. Probe the pinned
        // local identities first, and refuse ambiguous destinations.
        let mut matches = Vec::new();
        for candidate in candidates.iter().filter(|s| s.team_id == old.store.team_id) {
            let scoped = state.for_profile(&candidate.profile)?;
            let id = store_id(&CatalogStoreRef::Team(candidate.clone()));
            let Ok((generation, selected)) = scoped.selected_chat(&id) else {
                continue;
            };
            if selected != *candidate {
                continue;
            }
            let transport = scoped
                .agent
                .transport_for("chat_migration", Some(&candidate.profile));
            let cancelled = || {
                crate::applock::require_unlocked_generation(app, unlocked).is_err()
                    || scoped.chat_generation.load(Ordering::Acquire) != generation
            };
            if cancelled() {
                return Err(unavailable());
            }
            if let Ok(reply) = foks_desktop::chat_request_cancellable(
                transport.as_ref(),
                candidate.clone(),
                probe.clone(),
                &cancelled,
            ) {
                if !cancelled()
                    && scoped
                        .accept_chat_scope(&id, generation, &reply.scope)
                        .is_ok()
                {
                    matches.push((scoped, id, generation, candidate.clone()));
                }
            }
        }
        if matches.len() != 1 {
            remaining += 1;
            continue;
        }
        let (scoped, id, generation, selected) = matches.pop().unwrap();
        let transport = scoped
            .agent
            .transport_for("chat_migration", Some(&selected.profile));
        let cancelled = || {
            crate::applock::require_unlocked_generation(app, unlocked).is_err()
                || scoped.chat_generation.load(Ordering::Acquire) != generation
        };
        if cancelled() {
            return Err(unavailable());
        }
        let imported = foks_desktop::chat_request_cancellable(
            transport.as_ref(),
            selected,
            ChatAction::ImportIntent {
                host: old.host,
                actor: old.actor,
                channel,
                submission: record.intent.submission.clone(),
                text: SecretString::new(&record.intent.text),
                source: record.source.clone(),
            },
            &cancelled,
        );
        match imported {
            Ok(reply)
                if !cancelled()
                    && scoped
                        .accept_chat_scope(&id, generation, &reply.scope)
                        .is_ok() =>
            {
                let cleared = legacy.clear_imported(&record);
                legacy.release_lock();
                cleared.map_err(|_| unavailable())?;
            }
            _ => remaining += 1,
        }
    }
    if remaining == 0 {
        LocalChatIntentStore::retire_empty(&path).map_err(|_| unavailable())?;
    }
    Ok(remaining)
}
