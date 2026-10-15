//! Chat-scope access and verified chat identities.

use super::catalog::catalog_changed_during_read;
use super::{AppState, LockExt};
use crate::agent::AgentError;
use crate::commands::validation::{invalid_request, invalid_response};
use std::sync::atomic::Ordering;

impl AppState {
    pub(in crate::commands) fn selected_chat(
        &self,
        id: &str,
    ) -> Result<(u64, foks_agent_proto::TeamStoreRef), AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        let team = self.selected_active_team_for_mutation(id)?;
        self.require_chat_access(&team.profile)?;
        Ok((self.chat_generation().load(Ordering::Acquire), team))
    }

    fn require_chat_access(&self, profile: &str) -> Result<(), AgentError> {
        let catalog = self.catalog.lock_unpoisoned();
        let catalog = catalog.as_ref().ok_or_else(catalog_changed_during_read)?;
        if catalog.inventory.iter().any(|entry| {
            entry.profile == profile && (!entry.accounts_complete || !entry.teams_complete)
        }) {
            return Err(catalog_changed_during_read());
        }
        if let Some(overview) = catalog
            .profile_overviews
            .iter()
            .find(|overview| overview.profile == profile)
        {
            let foks_agent_proto::ResponseResult::Success { value } = &overview.server_status
            else {
                return Err(catalog_changed_during_read());
            };
            let status: foks_agent_proto::ServerStatusSnapshot =
                serde_json::from_value(value.clone())
                    .map_err(|_| invalid_response("Invalid chat server status."))?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if status.profile != profile
                || status.chat_supported != Some(true)
                || match status.compatibility {
                    foks_agent_proto::CompatibilityStatus::NotRequired => false,
                    foks_agent_proto::CompatibilityStatus::Validated {
                        expires_at,
                        capabilities,
                    } => expires_at <= now || !capabilities.contains("chat"),
                    _ => true,
                }
            {
                return Err(AgentError::new(
                    "capability-unavailable",
                    "Chat access must be revalidated for this profile.",
                    true,
                ));
            }
        }
        Ok(())
    }

    pub(in crate::commands) fn accept_chat_scope(
        &self,
        id: &str,
        generation: u64,
        scope: &foks_agent_proto::chat::ChatScope,
    ) -> Result<(), AgentError> {
        let _coordination = self.catalog_coordination.lock_unpoisoned();
        if self.chat_generation().load(Ordering::Acquire) != generation {
            return Err(AgentError::new(
                "chat-restart",
                "Chat access changed during the request.",
                true,
            ));
        }
        if self.selected_active_team_for_mutation(id)? != scope.store {
            return Err(invalid_response("The selected chat identity changed."));
        }
        self.require_chat_access(&scope.store.profile)?;
        let catalog = self.catalog.lock_unpoisoned();
        if let Some(overview) = catalog.as_ref().and_then(|catalog| {
            catalog
                .profile_overviews
                .iter()
                .find(|overview| overview.profile == scope.store.profile)
        }) {
            if let foks_agent_proto::ResponseResult::Success { value } = &overview.server_status {
                if value
                    .get("host")
                    .and_then(|host| host.get("host_id_hex"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|host| host != scope.host)
                {
                    self.chat_generation()
                        .store(self.next_generation(), Ordering::Release);
                    return Err(invalid_response("The selected chat host changed."));
                }
            }
        }
        let mut identities = self.scope_state.chat_identities.lock_unpoisoned();
        if identities.get(id).is_some_and(|previous| previous != scope) {
            self.chat_generation()
                .store(self.next_generation(), Ordering::Release);
            identities.insert(id.to_owned(), scope.clone());
            return Err(AgentError::new(
                "chat-restart",
                "The verified chat identity changed.",
                true,
            ));
        }
        if !identities.contains_key(id) && identities.len() >= 4096 {
            return Err(invalid_request("Too many verified chat identities."));
        }
        identities.insert(id.to_owned(), scope.clone());
        Ok(())
    }
}
