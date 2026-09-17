//! Native maintenance controls: destinations and approval stay in native dialogs.
use crate::{
    agent::AgentError,
    commands::{require_main_window, AppState},
};
use tauri::{Manager as _, State};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons};

#[tauri::command]
pub async fn relocate_client_state(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle().clone();
    let generation = crate::applock::unlocked_generation(&app)?;
    let _mutation = state.begin_mutation()?;
    let picker = app.clone();
    let parent = tauri::async_runtime::spawn_blocking(move || {
        picker
            .dialog()
            .file()
            .set_title("Choose the new parent folder for FOKS state")
            .blocking_pick_folder()
    })
    .await
    .map_err(|_| AgentError::unknown("Folder selection interrupted."))?;
    let Some(parent) = parent else {
        return Ok(serde_json::json!({"ok":true}));
    };
    let parent = parent
        .into_path()
        .map_err(|_| AgentError::unknown("Invalid destination folder."))?;
    let source = state
        .agent
        .socket()
        .parent()
        .ok_or_else(|| AgentError::unknown("Missing client state root."))?
        .to_owned();
    let destination = parent.join("foks-rs");
    let prompt=format!("Move all FOKS profiles, credentials and local data from {} to {}? The destination must not exist. FOKS will close administration windows, stop its background service, verify the move and restart.",source.display(),destination.display());
    let confirmation = app.clone();
    let approved = tauri::async_runtime::spawn_blocking(move || {
        confirmation
            .dialog()
            .message(prompt)
            .title("Move FOKS state")
            .buttons(MessageDialogButtons::OkCancel)
            .blocking_show()
    })
    .await
    .map_err(|_| AgentError::unknown("Move confirmation interrupted."))?;
    if !approved {
        return Ok(serde_json::json!({"ok":true}));
    }
    crate::applock::require_unlocked_generation(&app, generation)?;
    super::web_admin::close_all(&app);
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || agent.relocate_managed_state(&destination))
        .await
        .map_err(|_| AgentError::unknown("State move interrupted; recover using the CLI."))??;
    app.restart();
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StateArchiveAction {
    Export,
    Import,
    Verify,
}
fn selected_path(
    app: &tauri::AppHandle,
    title: &str,
    save: bool,
    folder: bool,
) -> foks_client_app::Result<Option<std::path::PathBuf>> {
    let picker = app.dialog().file().set_title(title);
    let path = if folder {
        picker.blocking_pick_folder()
    } else if save {
        picker.blocking_save_file()
    } else {
        picker.blocking_pick_file()
    };
    path.map(|p| {
        p.into_path().map_err(|_| {
            foks_client_app::Error::InvalidConfig("native picker returned a nonlocal path")
        })
    })
    .transpose()
}
fn confirm(app: &tauri::AppHandle, title: &str, message: String) -> bool {
    app.dialog()
        .message(message)
        .title(title)
        .buttons(MessageDialogButtons::OkCancel)
        .blocking_show()
}
fn require_unlocked(app: &tauri::AppHandle, generation: u64) -> foks_client_app::Result<()> {
    crate::applock::require_unlocked_generation(app, generation)
        .map_err(|_| foks_client_app::Error::InvalidConfig("application locked during maintenance"))
}
fn export_archive(
    app: &tauri::AppHandle,
    generation: u64,
    root: &std::path::Path,
) -> foks_client_app::Result<bool> {
    use foks_client_app::portability as p;
    let Some(archive) = selected_path(app, "Save encrypted FOKS archive", true, false)? else {
        return Ok(false);
    };
    let Some(key) = selected_path(app, "Save private transfer key separately", true, false)? else {
        return Ok(false);
    };
    let preview = p::prepare_state_export(root)?;
    let report = preview.report()?;
    let message=format!("{}\n\nThis copies existing device credentials. Copies share server-side revocation. Provision a new device for independent revocation. Keep the archive and its separate key as sensitive backups.",serde_json::to_string_pretty(&report)?);
    if !confirm(app, "Export this exact FOKS snapshot?", message) {
        return Ok(false);
    }
    require_unlocked(app, generation)?;
    preview
        .authorize(&report.digest)?
        .export_with_key_file(archive, key)?;
    Ok(false)
}
fn import_archive(
    app: &tauri::AppHandle,
    generation: u64,
    agent: &crate::agent::AgentHandle,
) -> Result<bool, AgentError> {
    use foks_client_app::portability as p;
    let choose = |title, folder| {
        selected_path(app, title, false, folder).map_err(|e| AgentError::unknown(e.to_string()))
    };
    let Some(archive) = choose("Open encrypted FOKS archive", false)? else {
        return Ok(false);
    };
    let Some(key) = choose("Open its private transfer key", false)? else {
        return Ok(false);
    };
    let Some(parent) = choose("Choose the parent folder for imported FOKS state", true)? else {
        return Ok(false);
    };
    let destination = parent.join("foks-imported");
    let message=format!("Install an independent local copy in {} and restart? Existing state is preserved. Imported accounts remain blocked until online verification. This copies device credentials and their revocation identity. The archive and key remain in place.",destination.display());
    if !confirm(app, "Import FOKS state?", message) {
        return Ok(false);
    }
    crate::applock::require_unlocked_generation(app, generation)?;
    agent.with_managed_state(true, |_| {
        p::import_state_for_desktop(archive, &p::read_transfer_key(key)?, destination)?;
        Ok(true)
    })
}
fn verify_accounts(
    app: &tauri::AppHandle,
    generation: u64,
    root: &std::path::Path,
) -> foks_client_app::Result<bool> {
    use foks_client_app::portability as p;
    use std::collections::BTreeMap;
    let registry = foks_client_app::ProfileRegistry::open(root)?;
    let credentials = foks_client_app::ClientCredentials::open(root)?;
    let provider = foks_yubi::HardwareYubiProvider::new();
    let mut reports = Vec::new();
    for profile in registry.profiles() {
        require_unlocked(app, generation)?;
        let session = foks_client_app::ProfileSession::open(&registry, &profile.name)?;
        if !credentials.requires_import_verification(&session)? {
            continue;
        }
        let first = p::verify_imported_profile(&credentials, &session, &BTreeMap::new())?;
        let mut tokens = BTreeMap::new();
        let mut pins = BTreeMap::new();
        for account in &first.accounts {
            let label = match account.status {
                "hardware-required" => "PIN",
                "token-required" => "original bot token",
                _ => continue,
            };
            let title = format!(
                "Choose private {label} file for {}/{}",
                profile.name, account.alias
            );
            if let Some(path) = selected_path(app, &title, false, false)? {
                require_unlocked(app, generation)?;
                let secret = p::read_private_secret_file(path, 128)?;
                if account.status == "hardware-required" {
                    pins.insert(account.alias.clone(), foks_yubi::Pin::new(secret.as_str())?);
                } else {
                    tokens.insert(account.alias.clone(), secret);
                }
            }
        }
        let mut secrets = BTreeMap::new();
        for (alias, token) in &tokens {
            secrets.insert(alias.clone(), p::VerificationSecret::BotToken(token));
        }
        for (alias, pin) in &pins {
            secrets.insert(
                alias.clone(),
                p::VerificationSecret::Yubi {
                    pin,
                    provider: &provider,
                },
            );
        }
        require_unlocked(app, generation)?;
        let report = if secrets.is_empty() {
            first
        } else {
            p::verify_imported_profile(&credentials, &session, &secrets)?
        };
        reports.push(report);
    }
    require_unlocked(app, generation)?;
    let message=format!("{}\n\nAccounts requiring sign-in remain blocked. Complete their existing-account sign-in, then verify again. No queued writes were submitted.",serde_json::to_string_pretty(&reports)?);
    app.dialog()
        .message(message)
        .title("Import verification")
        .blocking_show();
    Ok(false)
}
#[tauri::command]
pub async fn maintain_client_state(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    action: StateArchiveAction,
) -> Result<serde_json::Value, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle().clone();
    let generation = crate::applock::unlocked_generation(&app)?;
    let _mutation = state.begin_mutation()?;
    super::web_admin::close_all(&app);
    let agent = state.agent.clone();
    let worker_app = app.clone();
    let restart = tauri::async_runtime::spawn_blocking(move || {
        let app = worker_app;
        crate::applock::require_unlocked_generation(&app, generation)?;
        match action {
            StateArchiveAction::Export => {
                agent.with_managed_state(false, |root| export_archive(&app, generation, root))
            }
            StateArchiveAction::Import => import_archive(&app, generation, &agent),
            StateArchiveAction::Verify => {
                agent.with_managed_state(false, |root| verify_accounts(&app, generation, root))
            }
        }
    })
    .await
    .map_err(|_| {
        AgentError::unknown("State maintenance interrupted; use state status and state recover.")
    })??;
    if restart {
        app.restart();
    }
    Ok(serde_json::json!({"ok":true}))
}
