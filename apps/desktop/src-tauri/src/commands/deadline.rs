pub(super) use crate::agent::deadline::{clock_error, run};

use super::context::AppState;
use crate::agent::AgentError;
use foks_agent_proto::DeadlineClass;

/// Owned command view: internal calls clone the same original deadline.
#[derive(Clone)]
pub struct CommandState(AppState);
impl std::ops::Deref for CommandState {
    type Target = AppState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl CommandState {
    pub fn inner(&self) -> &AppState {
        &self.0
    }
    fn admitted(state: &AppState, name: &str) -> Result<Self, AgentError> {
        Ok(Self(match command_class(name)? {
            Some(class) => state.begin_operation(class)?,
            None => state.clone(),
        }))
    }
}
impl<'de, R: tauri::Runtime> tauri::ipc::CommandArg<'de, R> for CommandState {
    fn from_command(
        command: tauri::ipc::CommandItem<'de, R>,
    ) -> Result<Self, tauri::ipc::InvokeError> {
        let name = command.name;
        let state =
            <tauri::State<'de, AppState> as tauri::ipc::CommandArg<'de, R>>::from_command(command)?;
        Self::admitted(state.inner(), name).map_err(Into::into)
    }
}

fn command_class(name: &str) -> Result<Option<DeadlineClass>, AgentError> {
    Ok(match name {
        "agent_process_info"
        | "app_info"
        | "auto_recover_agent"
        | "bot_account_request"
        | "cancel_chat_requests"
        | "chat_request"
        | "client_state_maintenance_status"
        | "diagnostic_timings"
        | "download_file"
        | "export_logins"
        | "first_run_operation_status"
        | "maintain_client_state"
        | "pick_and_replace_file"
        | "pick_import_file"
        | "quit_app"
        | "release_import_file"
        | "relocate_client_state"
        | "restart_agent"
        | "retry_agent_connection"
        | "sso_request"
        | "take_agent_connection_loss" => None,
        "commit_password_import"
        | "copy_item_to_store"
        | "empty_trash"
        | "import_dropped_file"
        | "replace_dropped_file"
        | "resume_file_upload" => Some(DeadlineClass::Bulk),
        // A catalog read walks every store of every profile it covers, one
        // ListKv page after another, under this one budget. The agent caps
        // each of those requests by its own class (Ordinary, 15 s), so the
        // longer budget does not lengthen any single request; it lets a
        // vault with many stores on a slow server finish loading.
        "list_catalog" | "list_catalog_progressive" | "list_profile_catalog" => {
            Some(DeadlineClass::Bulk)
        }
        "accept_device_pairing"
        | "accept_go_profile_pairing"
        | "copy_go_profile_device"
        | "finish_device_pairing"
        | "resume_device_pairing_acceptance"
        | "resume_device_pairing_offer"
        | "resume_go_profile_pairing"
        | "run_first_run_account_operation"
        | "start_device_pairing" => Some(DeadlineClass::Pairing),
        "abandon_group_creation"
        | "account_passphrase_status"
        | "adapter_clock_request"
        | "add_federated_team_member"
        | "add_group_member"
        | "agent_status"
        | "cancel_file_upload"
        | "change_account_passphrase"
        | "change_yubi_passphrase"
        | "change_yubi_pin"
        | "change_yubi_puk"
        | "chat_local"
        | "check_and_add_go_profile"
        | "check_and_add_profile"
        | "check_server"
        | "commit_owner_backup"
        | "configure_web_admin"
        | "copy_item_field"
        | "copy_item_value"
        | "create_first_run_account"
        | "create_folder"
        | "create_group"
        | "create_text_item"
        | "create_yubi_account"
        | "demote_group_member"
        | "describe_reset"
        | "describe_server_status"
        | "discover_go_profiles"
        | "discover_groups"
        | "edit_text_item"
        | "expel_federated_group"
        | "initialize_client_state"
        | "invitation_request"
        | "list_account_devices"
        | "list_accounts"
        | "list_backup_enrollments"
        | "list_federation"
        | "list_file_uploads"
        | "list_group_details"
        | "list_parties"
        | "list_pending_operations"
        | "list_servers"
        | "list_stores"
        | "list_yubi_accounts"
        | "list_yubi_cards"
        | "move_item"
        | "open_item_url"
        | "open_sso_browser"
        | "open_web_admin"
        | "prepare_owner_backup"
        | "promote_group_member"
        | "provision_yubi_device"
        | "read_item"
        | "reconcile_server"
        | "recover_owner_account"
        | "recover_yubi_management_key"
        | "recover_yubi_subkey"
        | "remove_account_device"
        | "remove_group_member"
        | "remove_item"
        | "remove_server_and_credentials"
        | "rename_account_request"
        | "rerun_federated_team_member_add"
        | "reset_server"
        | "restore_trash_item"
        | "resume_first_run_account"
        | "resume_group_creation"
        | "resume_group_member_addition"
        | "resume_group_member_edit"
        | "resume_owner_recovery"
        | "resume_yubi_account"
        | "resume_yubi_management_key"
        | "revoke_owner_backup"
        | "revoke_yubi_device"
        | "rotate_yubi_management_key"
        | "set_account_passphrase"
        | "set_first_run_passphrase"
        | "set_local_account_alias"
        | "set_server_label"
        | "set_yubi_passphrase"
        | "store_usage"
        | "sync_yubi_account"
        | "trash_item"
        | "unblock_yubi_pin"
        | "verify_account_passphrase"
        | "verify_recovery_phrase"
        | "verify_yubi_passphrase"
        | "yubi_pin_status" => Some(DeadlineClass::Ordinary),
        _ => {
            return Err(AgentError::new(
                "missing-deadline-policy",
                "This command has no execution deadline policy.",
                false,
            ))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn state() -> AppState {
        AppState::new(Arc::new(crate::agent::AgentHandle::new(
            "/tmp/unused-deadline-agent.sock".into(),
        )))
    }

    #[test]
    fn command_admission_and_nested_scopes_keep_the_original_context() {
        let base = state();
        assert_eq!(base.transport().err().unwrap().code, "missing-deadline");
        let admitted = CommandState::admitted(&base, "read_item").unwrap();
        let expected = admitted.deadline().unwrap().envelope().clone();
        assert_eq!(admitted.clone().deadline().unwrap().envelope(), &expected);
        let scoped = admitted.for_profile("home").unwrap();
        assert_eq!(scoped.deadline().unwrap().envelope(), &expected);
        let nested = CommandState::admitted(&scoped, "describe_server_status").unwrap();
        assert_eq!(nested.deadline().unwrap().envelope(), &expected);
        assert!(base.deadline().is_err());
    }

    #[test]
    fn catalog_reads_take_the_bulk_budget() {
        for name in [
            "list_catalog",
            "list_catalog_progressive",
            "list_profile_catalog",
        ] {
            assert_eq!(
                command_class(name).unwrap(),
                Some(DeadlineClass::Bulk),
                "{name}"
            );
            let admitted = CommandState::admitted(&state(), name).unwrap();
            assert!(
                admitted
                    .deadline()
                    .unwrap()
                    .instant()
                    .saturating_duration_since(std::time::Instant::now())
                    > std::time::Duration::from_secs(290),
                "{name}"
            );
        }
    }

    #[test]
    fn go_device_copy_preserves_the_pairing_budget() {
        assert_eq!(
            command_class("copy_go_profile_device").unwrap(),
            Some(DeadlineClass::Pairing)
        );
    }

    #[test]
    fn dynamic_and_human_dialog_stages_require_explicit_binding() {
        for name in [
            "chat_request",
            "sso_request",
            "download_file",
            "export_logins",
            "pick_and_replace_file",
            "bot_account_request",
        ] {
            let admitted = CommandState::admitted(&state(), name).unwrap();
            assert!(admitted.deadline().is_err(), "{name}");
            assert_eq!(admitted.transport().err().unwrap().code, "missing-deadline");
            let confirmed = admitted.begin_operation(DeadlineClass::Bulk).unwrap();
            assert!(
                confirmed
                    .deadline()
                    .unwrap()
                    .instant()
                    .saturating_duration_since(std::time::Instant::now())
                    > std::time::Duration::from_secs(290)
            );
        }
        assert_eq!(
            CommandState::admitted(&state(), "future_unclassified_command")
                .err()
                .unwrap()
                .code,
            "missing-deadline-policy"
        );
    }
}
