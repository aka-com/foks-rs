//! Native maintenance controls: destinations and approval stay in native dialogs.
use crate::{
    agent::{
        AgentError, MaintenanceCompletion, MaintenanceKind, MaintenancePhase, MaintenanceSnapshot,
        MaintenanceWorker, MAINTENANCE_EVENT,
    },
    commands::{require_main_window, AppState},
};
use tauri::{Emitter as _, Manager as _, State};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons};

#[tauri::command]
pub async fn relocate_client_state(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<MaintenanceSnapshot, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle().clone();
    let generation = crate::applock::unlocked_generation(&app)?;
    let agent = state.agent.clone();
    let worker_app = app.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        let app = worker_app;
        let state = app.state::<AppState>();
        crate::applock::require_unlocked_generation(&app, generation)?;
        let _mutation = state.begin_mutation()?;
        let snapshot = agent.run_maintenance(
            MaintenanceKind::Relocate,
            &|snapshot| {
                if matches!(snapshot, MaintenanceSnapshot::Complete { .. }) {
                    state.invalidate_catalog();
                }
                let _ = app.emit(MAINTENANCE_EVENT, snapshot);
            },
            |worker| relocate_operation(&app, generation, worker),
        )?;
        if matches!(
            snapshot,
            MaintenanceSnapshot::Complete {
                disposition: crate::agent::MaintenanceDisposition::RestartSelectedRoot { .. },
                ..
            }
        ) {
            app.restart();
        }
        Ok::<_, AgentError>(snapshot)
    })
    .await
    .map_err(|_| AgentError::unknown("State move worker interrupted."))??;
    Ok(snapshot)
}

fn relocate_operation(
    app: &tauri::AppHandle,
    generation: u64,
    worker: &mut MaintenanceWorker<'_>,
) -> MaintenanceCompletion {
    let source = match worker.source_root() {
        Ok(root) => root,
        Err(error) => return maintenance_failed(error, Vec::new()),
    };
    let dialogs = NativeDialogs { app, generation };
    let parent = match dialogs.pick("Choose the new parent folder for FOKS state", false, true) {
        Ok(Some(parent)) => parent,
        Ok(None) => return MaintenanceCompletion::Cancelled,
        Err(error) => return maintenance_failed(error, vec![source]),
    };
    let destination = parent.join("foks-rs");
    worker.phase(MaintenancePhase::Confirming);
    let prompt=format!("Move all FOKS profiles, credentials and local data from {} to {}? The destination must not exist. FOKS will close administration windows, stop its background service, verify the move and restart.",source.display(),destination.display());
    match dialogs.confirm("Move FOKS state", prompt) {
        Ok(true) => {}
        Ok(false) => return MaintenanceCompletion::Cancelled,
        Err(error) => return maintenance_failed(error, vec![source, destination]),
    }
    if let Err(error) = crate::applock::require_unlocked_generation(app, generation) {
        return maintenance_failed(error, vec![source, destination]);
    }
    super::web_admin::close_all(app);
    if let Err(error) = worker.stop_owned_agent() {
        return maintenance_failed(error, vec![source, destination]);
    }
    match foks_client_app::portability::relocate_state(&source, &destination) {
        Ok(_) => MaintenanceCompletion::RestartSelected(destination),
        Err(error) => maintenance_failed(portability_error(error), vec![source, destination]),
    }
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StateArchiveAction {
    Export,
    Import,
    Verify,
}

trait MaintenanceDialogs {
    fn pick(
        &self,
        title: &str,
        save: bool,
        folder: bool,
    ) -> Result<Option<std::path::PathBuf>, AgentError>;
    fn confirm(&self, title: &str, message: String) -> Result<bool, AgentError>;
}

struct NativeDialogs<'a> {
    app: &'a tauri::AppHandle,
    generation: u64,
}

impl MaintenanceDialogs for NativeDialogs<'_> {
    fn pick(
        &self,
        title: &str,
        save: bool,
        folder: bool,
    ) -> Result<Option<std::path::PathBuf>, AgentError> {
        crate::applock::require_unlocked_generation(self.app, self.generation)?;
        selected_path(self.app, title, save, folder).map_err(portability_error)
    }

    fn confirm(&self, title: &str, message: String) -> Result<bool, AgentError> {
        crate::applock::require_unlocked_generation(self.app, self.generation)?;
        Ok(confirm(self.app, title, message))
    }
}

#[derive(Debug)]
struct ImportSelection {
    archive: std::path::PathBuf,
    key: std::path::PathBuf,
    parent: std::path::PathBuf,
}

fn collect_import_selection(
    dialogs: &dyn MaintenanceDialogs,
) -> Result<Option<ImportSelection>, AgentError> {
    let Some(archive) = dialogs.pick("Open encrypted FOKS archive", false, false)? else {
        return Ok(None);
    };
    let Some(key) = dialogs.pick("Open its private transfer key", false, false)? else {
        return Ok(None);
    };
    let Some(parent) = dialogs.pick(
        "Choose the parent folder for imported FOKS state",
        false,
        true,
    )?
    else {
        return Ok(None);
    };
    Ok(Some(ImportSelection {
        archive,
        key,
        parent,
    }))
}

fn collect_export_selection(
    dialogs: &dyn MaintenanceDialogs,
) -> Result<Option<(std::path::PathBuf, std::path::PathBuf)>, AgentError> {
    let Some(archive) = dialogs.pick("Save encrypted FOKS archive", true, false)? else {
        return Ok(None);
    };
    let Some(key) = dialogs.pick("Save private transfer key separately", true, false)? else {
        return Ok(None);
    };
    Ok(Some((archive, key)))
}

#[tauri::command]
pub fn client_state_maintenance_status(
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<MaintenanceSnapshot, AgentError> {
    require_main_window(&webview)?;
    Ok(state.agent.maintenance_snapshot())
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

fn portability_error(error: foks_client_app::Error) -> AgentError {
    AgentError::new("state-maintenance", error.to_string(), false)
}

fn maintenance_failed(
    error: AgentError,
    affected_roots: Vec<std::path::PathBuf>,
) -> MaintenanceCompletion {
    MaintenanceCompletion::Failed {
        error,
        affected_roots,
    }
}
fn export_archive(
    app: &tauri::AppHandle,
    generation: u64,
    root: &std::path::Path,
    archive: std::path::PathBuf,
    key: std::path::PathBuf,
) -> Result<bool, AgentError> {
    use foks_client_app::portability as p;
    let preview = p::prepare_state_export(root).map_err(portability_error)?;
    let report = preview.report().map_err(portability_error)?;
    let report_json = serde_json::to_string_pretty(&report)
        .map_err(|error| AgentError::unknown(error.to_string()))?;
    let message=format!("{}\n\nThis copies existing device credentials. Copies share server-side revocation. Provision a new device for independent revocation. Keep the archive and its separate key as sensitive backups.",report_json);
    let dialogs = NativeDialogs { app, generation };
    match dialogs.confirm("Export this exact FOKS snapshot?", message) {
        Ok(true) => {}
        Ok(false) => return Ok(false),
        Err(error) => return Err(error),
    }
    crate::applock::require_unlocked_generation(app, generation)?;
    preview
        .authorize(&report.digest)
        .map_err(portability_error)?
        .export_with_key_file(archive, key)
        .map_err(portability_error)?;
    Ok(true)
}
fn import_archive(
    app: &tauri::AppHandle,
    generation: u64,
    worker: &mut MaintenanceWorker<'_>,
) -> MaintenanceCompletion {
    use foks_client_app::portability as p;
    let dialogs = NativeDialogs { app, generation };
    let selection = match collect_import_selection(&dialogs) {
        Ok(Some(selection)) => selection,
        Ok(None) => return MaintenanceCompletion::Cancelled,
        Err(error) => return maintenance_failed(error, Vec::new()),
    };
    let destination = selection.parent.join("foks-imported");
    let message=format!("Install an independent local copy in {} and restart? Existing state is preserved. Imported accounts remain blocked until online verification. This copies device credentials and their revocation identity. The archive and key remain in place.",destination.display());
    match dialogs.confirm("Import FOKS state?", message) {
        Ok(true) => {}
        Ok(false) => return MaintenanceCompletion::Cancelled,
        Err(error) => return maintenance_failed(error, vec![destination]),
    }
    if let Err(error) = crate::applock::require_unlocked_generation(app, generation) {
        return maintenance_failed(error, vec![destination]);
    }
    super::web_admin::close_all(app);
    if let Err(error) = worker.stop_owned_agent() {
        return maintenance_failed(error, vec![destination]);
    }
    let result = p::read_transfer_key(selection.key)
        .and_then(|key| p::import_state_for_desktop(selection.archive, &key, &destination));
    match result {
        Ok(_) => MaintenanceCompletion::RestartSelected(destination),
        Err(error) => maintenance_failed(portability_error(error), vec![destination]),
    }
}
fn verify_accounts(
    app: &tauri::AppHandle,
    generation: u64,
    root: &std::path::Path,
) -> foks_client_app::Result<()> {
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
    Ok(())
}

fn archive_operation(
    app: &tauri::AppHandle,
    generation: u64,
    action: StateArchiveAction,
    worker: &mut MaintenanceWorker<'_>,
) -> MaintenanceCompletion {
    if matches!(action, StateArchiveAction::Import) {
        return import_archive(app, generation, worker);
    }
    let root = match worker.source_root() {
        Ok(root) => root,
        Err(error) => return maintenance_failed(error, Vec::new()),
    };
    let export_paths = if matches!(action, StateArchiveAction::Export) {
        match collect_export_selection(&NativeDialogs { app, generation }) {
            Ok(Some(paths)) => Some(paths),
            Ok(None) => return MaintenanceCompletion::Cancelled,
            Err(error) => {
                return maintenance_failed(error, vec![root.clone()]);
            }
        }
    } else {
        None
    };
    if let Err(error) = crate::applock::require_unlocked_generation(app, generation) {
        return maintenance_failed(error, vec![root]);
    }
    super::web_admin::close_all(app);
    if let Err(error) = worker.stop_owned_agent() {
        return maintenance_failed(error, vec![root]);
    }
    match (action, export_paths) {
        (StateArchiveAction::Export, Some((archive, key))) => {
            match export_archive(app, generation, &root, archive, key) {
                Ok(true) => MaintenanceCompletion::Continue,
                Ok(false) => MaintenanceCompletion::Cancelled,
                Err(error) => maintenance_failed(error, vec![root]),
            }
        }
        (StateArchiveAction::Verify, None) => match verify_accounts(app, generation, &root) {
            Ok(()) => MaintenanceCompletion::Continue,
            Err(error) => maintenance_failed(portability_error(error), vec![root]),
        },
        _ => unreachable!("archive action and selected inputs remain correlated"),
    }
}
#[tauri::command]
pub async fn maintain_client_state(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    action: StateArchiveAction,
) -> Result<MaintenanceSnapshot, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle().clone();
    let generation = crate::applock::unlocked_generation(&app)?;
    let agent = state.agent.clone();
    let worker_app = app.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        let app = worker_app;
        let state = app.state::<AppState>();
        crate::applock::require_unlocked_generation(&app, generation)?;
        let _mutation = state.begin_mutation()?;
        let kind = match action {
            StateArchiveAction::Export => MaintenanceKind::Export,
            StateArchiveAction::Import => MaintenanceKind::Import,
            StateArchiveAction::Verify => MaintenanceKind::Verify,
        };
        let snapshot = agent.run_maintenance(
            kind,
            &|snapshot| {
                if matches!(snapshot, MaintenanceSnapshot::Complete { .. }) {
                    state.invalidate_catalog();
                }
                let _ = app.emit(MAINTENANCE_EVENT, snapshot);
            },
            |worker| archive_operation(&app, generation, action, worker),
        )?;
        if matches!(
            snapshot,
            MaintenanceSnapshot::Complete {
                disposition: crate::agent::MaintenanceDisposition::RestartSelectedRoot { .. },
                ..
            }
        ) {
            app.restart();
        }
        Ok::<_, AgentError>(snapshot)
    })
    .await
    .map_err(|_| AgentError::unknown("State maintenance worker interrupted."))??;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeDialogs {
        paths: Mutex<VecDeque<Option<std::path::PathBuf>>>,
        approved: bool,
        fail_at: Option<usize>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl FakeDialogs {
        fn with_paths(paths: impl IntoIterator<Item = Option<&'static str>>) -> Self {
            Self {
                paths: Mutex::new(
                    paths
                        .into_iter()
                        .map(|path| path.map(std::path::PathBuf::from))
                        .collect(),
                ),
                approved: false,
                fail_at: None,
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn locked_during(
            paths: impl IntoIterator<Item = Option<&'static str>>,
            call: usize,
        ) -> Self {
            Self {
                fail_at: Some(call),
                ..Self::with_paths(paths)
            }
        }

        fn require_unlocked(&self) -> Result<(), AgentError> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            if self.fail_at == Some(call) {
                return Err(AgentError::new(
                    "app-locked",
                    "application locked during maintenance dialog",
                    true,
                ));
            }
            Ok(())
        }
    }

    impl MaintenanceDialogs for FakeDialogs {
        fn pick(
            &self,
            _title: &str,
            _save: bool,
            _folder: bool,
        ) -> Result<Option<std::path::PathBuf>, AgentError> {
            self.require_unlocked()?;
            Ok(self.paths.lock().unwrap().pop_front().flatten())
        }

        fn confirm(&self, _title: &str, _message: String) -> Result<bool, AgentError> {
            self.require_unlocked()?;
            Ok(self.approved)
        }
    }

    #[test]
    fn export_picker_cancellation_at_each_step_returns_no_selection() {
        for paths in [[None, Some("key")], [Some("archive"), None]] {
            assert!(collect_export_selection(&FakeDialogs::with_paths(paths))
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn import_picker_cancellation_at_each_step_returns_no_selection() {
        for paths in [
            [None, Some("key"), Some("parent")],
            [Some("archive"), None, Some("parent")],
            [Some("archive"), Some("key"), None],
        ] {
            assert!(collect_import_selection(&FakeDialogs::with_paths(paths))
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn confirmation_adapter_keeps_cancellation_typed() {
        let dialogs = FakeDialogs::with_paths([]);
        assert!(!dialogs.confirm("title", "message".into()).unwrap());
    }

    #[test]
    fn lock_changes_between_native_dialogs_stop_further_selection() {
        let dialogs = FakeDialogs::locked_during([Some("archive"), Some("key"), Some("parent")], 1);
        let error = collect_import_selection(&dialogs).unwrap_err();
        assert_eq!(error.code, "app-locked");
        assert_eq!(dialogs.calls.load(std::sync::atomic::Ordering::Acquire), 2);
    }
}
