//! The FOKS desktop application.
//!
//! FOKS communicates with a local agent over a private Unix socket and runs
//! without a remote backend server.
//!
//! Configures native file drop routing via `dragDropEnabled` (see [`dragdrop`])
//! and disables `withGlobalTauri` to prevent exposing global IPC objects in webviews.

mod agent;
mod applock;
mod clipboard;
mod close_guard;
mod commands;
mod diagnostics;
mod dragdrop;
mod navigation;
mod startup;
mod window_state;

use std::sync::Arc;

use tauri::Manager as _;
use tauri_plugin_dialog::{DialogExt as _, MessageDialogKind};

use agent::AgentHandle;
use close_guard::CloseGuard;
use commands::{AppState, MAIN};

#[cfg(target_os = "macos")]
const NEW_WINDOW_MENU_ID: &str = "new-main-window";
#[cfg(target_os = "macos")]
const NEW_PASSWORD_MENU_ID: &str = "new-password";
#[cfg(target_os = "macos")]
const NEW_DOCUMENT_MENU_ID: &str = "new-document";
#[cfg(target_os = "macos")]
const SETTINGS_MENU_ID: &str = "open-settings";
#[cfg(target_os = "macos")]
const OPEN_SETTINGS_EVENT: &str = "foks://open-settings";

#[cfg(target_os = "macos")]
fn open_main_window(app: &tauri::AppHandle, settings: bool) -> Result<(), String> {
    use tauri::Emitter as _;

    if let Some(window) = app.get_webview_window(MAIN) {
        window.unminimize().map_err(|error| error.to_string())?;
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        if settings {
            window
                .emit(OPEN_SETTINGS_EVENT, ())
                .map_err(|error| error.to_string())?;
        }
        return Ok(());
    }

    let mut config = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == MAIN)
        .cloned()
        .ok_or_else(|| format!("{MAIN} window configuration not found"))?;
    if settings {
        config.url = tauri::WebviewUrl::App("index.html?state=settings".into());
    }
    let window = tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?;
    dragdrop::observe(&window);
    window_state::observe(&window);
    close_guard::observe(&window, Arc::clone(&app.state::<Arc<CloseGuard>>()));
    window.set_focus().map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn install_macos_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItemBuilder, PredefinedMenuItem};

    let menu = Menu::default(app.handle())?;
    let settings = MenuItemBuilder::with_id(SETTINGS_MENU_ID, "Settings…")
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    let new_window = MenuItemBuilder::with_id(NEW_WINDOW_MENU_ID, "New Window").build(app)?;
    let new_password = MenuItemBuilder::with_id(NEW_PASSWORD_MENU_ID, "New Password")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let new_document = MenuItemBuilder::with_id(NEW_DOCUMENT_MENU_ID, "Add Document")
        .accelerator("CmdOrCtrl+Shift+N")
        .build(app)?;
    let items = menu.items()?;
    if let Some(application) = items.first().and_then(|item| item.as_submenu()) {
        application.insert(&settings, 2)?;
        application.insert(&PredefinedMenuItem::separator(app)?, 3)?;
    }
    if let Some(file) = items
        .iter()
        .filter_map(|item| item.as_submenu())
        .find(|submenu| submenu.text().is_ok_and(|text| text == "File"))
    {
        file.prepend(&PredefinedMenuItem::separator(app)?)?;
        file.prepend(&new_window)?;
        file.prepend(&PredefinedMenuItem::separator(app)?)?;
        file.prepend(&new_document)?;
        file.prepend(&new_password)?;
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let settings = event.id() == SETTINGS_MENU_ID;
        if settings || event.id() == NEW_WINDOW_MENU_ID {
            if let Err(error) = open_main_window(app, settings) {
                tracing::error!(%error, "Failed to open main window from application menu");
            }
            return;
        }
        let new_item_event = if event.id() == NEW_PASSWORD_MENU_ID {
            Some("foks://new-password")
        } else if event.id() == NEW_DOCUMENT_MENU_ID {
            Some("foks://new-document")
        } else {
            None
        };
        if let Some(new_item_event) = new_item_event {
            use tauri::Emitter as _;
            let result = open_main_window(app, false).and_then(|()| {
                app.get_webview_window(MAIN)
                    .ok_or_else(|| format!("{MAIN} window is unavailable"))?
                    .emit(new_item_event, ())
                    .map_err(|error| error.to_string())
            });
            if let Err(error) = result {
                tracing::error!(%error, "Failed to create an item from the application menu");
            }
        }
    });
    Ok(())
}

pub fn run() {
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--smoke-test-packaged-startup"))
    {
        match agent::smoke_test_packaged_startup() {
            Ok(()) => println!("Packaged production agent startup passed"),
            Err(error) => {
                eprintln!("Packaged startup failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    // Use `args_os` to preserve non-UTF-8 arguments safely when inspecting flags.
    let endpoint = agent::resolve_endpoint(
        std::env::args_os().skip(1),
        std::env::var_os(agent::SOCKET_ENV).map(std::path::PathBuf::from),
    );
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "foks_desktop_app=info".into()),
        )
        .init();

    if let Some(directory) = endpoint
        .as_ref()
        .and_then(|endpoint| endpoint.managed_crash_directory.clone())
    {
        if let Err(error) = agent::prepare_managed_crash_directory(&directory) {
            tracing::error!(reason = %error.message, "Failed to initialize crash storage directory");
            std::process::exit(1);
        }
        foks_desktop::install_crash_reporter(directory);
    }

    let Some(endpoint) = endpoint else {
        // Exit if agent socket path cannot be determined.
        tracing::error!(
            "Could not determine agent socket path; pass {} or set {}",
            agent::SOCKET_ARG,
            agent::SOCKET_ENV
        );
        std::process::exit(1);
    };
    let socket = endpoint.socket;
    tracing::info!(socket = %socket.display(), "Using agent socket");
    let agent = Arc::new(AgentHandle::new(socket));
    // Shared with the run loop below, which asks the same question for a quit
    // that never passes through the window's own close.
    let closing = Arc::new(CloseGuard::default());
    let exiting = Arc::clone(&closing);
    let exiting_agent = Arc::clone(&agent);

    tauri::Builder::default()
        // Register single-instance plugin first so duplicate processes hand off
        // and exit before competing for the agent socket.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window(MAIN) {
                let _ = window.set_focus();
                return;
            }
            app.dialog()
                .message("FOKS is already running.")
                .kind(MessageDialogKind::Info)
                .title("FOKS")
                .show(|_| {});
        }))
        // Before every other plugin's hook and before the config-declared
        // webview loads its first page: the window may only ever be on FOKS's
        // own origin. See navigation.rs.
        .plugin(navigation::policy())
        .plugin(tauri_plugin_dialog::init())
        // Registered for Rust-side clipboard access with auto-clearing.
        // The webview has no direct clipboard permissions.
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState::new(Arc::clone(&agent)))
        .manage(commands::chat_local::LocalState::default())
        .manage(Arc::new(applock::AppLock::new()))
        .manage(Arc::clone(&closing))
        .invoke_handler(tauri::generate_handler![
            commands::sso::sso_request,
            commands::account_conveniences::rename_account_request,
            commands::account_conveniences::set_local_account_alias,
            commands::invitations::invitation_request,
            commands::bot::bot_account_request,
            commands::web_admin::configure_web_admin,
            commands::web_admin::open_web_admin,
            commands::portability::relocate_client_state,
            commands::portability::maintain_client_state,
            commands::portability::client_state_maintenance_status,
            commands::sso::open_sso_browser,
            commands::application::agent_status,
            commands::application::auto_recover_agent,
            commands::application::retry_agent_connection,
            commands::enrollment::initialize_client_state,
            commands::enrollment::discover_go_profiles,
            commands::servers::check_and_add_profile,
            commands::enrollment::check_and_add_go_profile,
            commands::servers::set_server_label,
            commands::servers::remove_server_and_credentials,
            commands::servers::describe_server_status,
            commands::servers::reconcile_server,
            commands::servers::check_server,
            commands::enrollment::list_pending_operations,
            commands::enrollment::create_first_run_account,
            commands::first_run::run_first_run_account_operation,
            commands::first_run::first_run_operation_status,
            commands::enrollment::resume_first_run_account,
            commands::enrollment::set_first_run_passphrase,
            commands::accounts::prepare_owner_backup,
            commands::accounts::commit_owner_backup,
            commands::enrollment::recover_owner_account,
            commands::enrollment::resume_owner_recovery,
            commands::groups::discover_groups,
            commands::accounts::list_account_devices,
            commands::accounts::remove_account_device,
            commands::accounts::list_backup_enrollments,
            commands::accounts::revoke_owner_backup,
            commands::yubikey::list_yubi_cards,
            commands::yubikey::list_yubi_accounts,
            commands::yubikey::create_yubi_account,
            commands::yubikey::resume_yubi_account,
            commands::yubikey::provision_yubi_device,
            commands::yubikey::sync_yubi_account,
            commands::yubikey::yubi_pin_status,
            commands::yubikey::change_yubi_pin,
            commands::yubikey::set_yubi_passphrase,
            commands::yubikey::change_yubi_passphrase,
            commands::yubikey::verify_yubi_passphrase,
            commands::yubikey::change_yubi_puk,
            commands::yubikey::unblock_yubi_pin,
            commands::yubikey::rotate_yubi_management_key,
            commands::yubikey::resume_yubi_management_key,
            commands::yubikey::recover_yubi_management_key,
            commands::yubikey::recover_yubi_subkey,
            commands::yubikey::revoke_yubi_device,
            commands::enrollment::start_device_pairing,
            commands::enrollment::resume_device_pairing_offer,
            commands::enrollment::finish_device_pairing,
            commands::enrollment::accept_device_pairing,
            commands::enrollment::accept_go_profile_pairing,
            commands::enrollment::resume_device_pairing_acceptance,
            commands::enrollment::resume_go_profile_pairing,
            commands::enrollment::copy_go_profile_device,
            commands::accounts::set_account_passphrase,
            commands::accounts::change_account_passphrase,
            commands::accounts::verify_account_passphrase,
            commands::accounts::account_passphrase_status,
            commands::servers::describe_reset,
            commands::servers::reset_server,
            commands::application::app_info,
            commands::application::diagnostic_timings,
            commands::application::restart_app,
            commands::application::quit_app,
            window_state::get_window_state,
            window_state::set_traffic_lights_visible,
            close_guard::set_unsent_messages,
            close_guard::exit_state,
            close_guard::handle_exit_action,
            commands::vault::list_stores,
            commands::vault::list_catalog,
            commands::vault::list_profile_catalog,
            commands::vault::list_catalog_progressive,
            commands::servers::list_servers,
            commands::accounts::list_accounts,
            commands::groups::list_group_details,
            commands::chat::chat_request,
            commands::chat_local::chat_local,
            commands::chat_local::open_notification_settings,
            commands::chat::open_chat_link,
            commands::chat::cancel_chat_requests,
            commands::groups::list_parties,
            commands::groups::list_federation,
            commands::vault::read_item,
            commands::vault::copy_item_value,
            commands::vault::copy_item_path,
            commands::vault::copy_text,
            commands::vault::download_file,
            commands::vault::create_text_item,
            commands::vault::create_link,
            commands::vault::create_folder,
            commands::vault::edit_text_item,
            commands::vault::remove_item,
            commands::vault::import_dropped_file,
            commands::vault::pick_import_file,
            commands::vault::release_import_file,
            commands::vault::replace_dropped_file,
            commands::vault::pick_and_replace_file,
            commands::groups::create_group,
            commands::groups::resume_group_creation,
            commands::groups::abandon_group_creation,
            commands::groups::add_group_member,
            commands::groups::resume_group_member_addition,
            commands::groups::demote_group_member,
            commands::groups::remove_group_member,
            commands::groups::resume_group_member_edit,
            commands::groups::add_federated_team_member,
            commands::groups::rerun_federated_team_member_add,
            commands::groups::expel_federated_group,
            commands::application::take_agent_connection_loss,
            commands::application::agent_process_info,
            commands::application::restart_agent,
            applock::app_lock_state,
            applock::lock_app,
            applock::unlock_app,
        ])
        .setup(move |app| {
            use tauri::Emitter as _;

            #[cfg(target_os = "macos")]
            install_macos_menu(app)?;
            commands::chat_local::platform::install(app.handle());
            // Forward connection loss events to the main webview window rather
            // than relying on periodic polling. If emission fails, the frontend
            // will still read the error via `take_agent_connection_loss`.
            let losses = app.handle().clone();
            agent.set_connection_loss_notifier(move || {
                let _ = losses.emit_to(MAIN, agent::CONNECTION_LOSS_EVENT, ());
            });
            // Verify agent reachability before handling agent requests; exit
            // with a dialog if unreachable. This runs off the main thread so
            // the window paints its loading state instead of staying blank
            // while the agent starts. Ordinary commands block on the gate
            // until the check finishes, so the frontend's boot sequence
            // resumes by itself.
            agent.hold_commands_for_startup();
            let startup_app = app.handle().clone();
            let startup_agent = Arc::clone(&agent);
            std::thread::Builder::new()
                .name("foks-startup".into())
                .spawn(move || {
                    startup::require_agent(&startup_app, &startup_agent);
                    startup_agent.release_startup();
                })
                .expect("failed to spawn the startup thread");
            if let Some(window) = app.get_webview_window(MAIN) {
                dragdrop::observe(&window);
                window_state::observe(&window);
                close_guard::observe(&window, Arc::clone(&closing));
            } else {
                tracing::error!("{MAIN} window configuration not found in tauri.conf.json");
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start application")
        .run(move |app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                // User quits are held until both the volatile chat queue and
                // the agent process owned by this launch have been decided.
                if close_guard::intercept_exit_request(app, &exiting, &exiting_agent, code) {
                    api.prevent_exit();
                    return;
                }
                clipboard::defer_exit_cleanup(app, code, &api);
            }
        });
}

#[cfg(test)]
mod tests {
    /// Ensures that only a single window is configured, matching the expected security boundary.
    #[test]
    fn the_configuration_declares_exactly_one_window_and_it_is_main() {
        let configuration: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json"))
                .expect("tauri.conf.json is valid JSON");
        let windows = configuration["app"]["windows"]
            .as_array()
            .expect("tauri.conf.json declares windows");
        assert_eq!(windows.len(), 1);
        assert_eq!(
            windows[0]["label"],
            super::MAIN,
            "primary window label must match MAIN constant"
        );
        assert_eq!(windows[0]["dragDropEnabled"], true);
        assert_eq!(configuration["app"]["withGlobalTauri"], false);
        assert_eq!(
            configuration["identifier"], "com.aka.foks.desktop",
            "the bundle identifier is also the desktop persistence boundary"
        );
        assert_eq!(
            configuration["bundle"]["externalBin"],
            serde_json::json!(["binaries/foks-agent"]),
            "every application bundle must include the managed agent"
        );
    }

    /// The exact renderer capability, in the order the file lists it.
    ///
    /// The webview capability is limited to three permissions: the drag-drop
    /// event listener and unlistener (`dragdrop.rs`), and the window drag command
    /// used by Tauri's `data-tauri-drag-region` for custom title bar movement.
    /// All other Tauri core, window, and plugin APIs are omitted.
    const RENDERER_PERMISSIONS: [&str; 3] = [
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "core:window:allow-start-dragging",
    ];

    /// The command identifiers the resolved capability grants, as
    /// `<manifest>|<command>`. This represents the entire default Tauri IPC
    /// surface accessible to scripts outside application-defined commands.
    const RENDERER_COMMANDS: [&str; 3] = [
        "core:event|listen",
        "core:event|unlisten",
        "core:window|start_dragging",
    ];

    /// Asserts that capabilities match the explicitly permitted set without additions or reordering.
    #[test]
    fn the_capability_grants_exactly_the_three_renderer_permissions() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json"))
                .expect("capabilities/default.json is valid JSON");
        assert_eq!(capability["identifier"], "default");
        assert_eq!(capability["windows"].as_array().map(Vec::len), Some(1));
        assert_eq!(capability["windows"][0], super::MAIN);

        let permissions: Vec<&str> = capability["permissions"]
            .as_array()
            .expect("the capability lists permissions")
            .iter()
            .map(|permission| permission.as_str().expect("permissions are strings"))
            .collect();
        assert_eq!(permissions, RENDERER_PERMISSIONS);
        assert_eq!(
            permissions
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            permissions.len(),
            "duplicate permission found in capability"
        );

        // Explicitly check for disallowed sensitive permissions.
        for forbidden in [
            "core:default",
            "core:event:default",
            "core:image:default",
            "core:image:allow-from-path",
            "core:menu:default",
            "core:tray:default",
            "core:path:default",
            "core:webview:default",
            "core:app:default",
            "core:window:default",
            "core:event:allow-emit",
            "core:event:allow-emit-to",
        ] {
            assert!(
                !permissions.contains(&forbidden),
                "forbidden permission present in capability: {forbidden}"
            );
        }
        for permission in &permissions {
            assert!(
                permission.starts_with("core:"),
                "{permission} is a plugin permission; FOKS drives its plugins from Rust"
            );
        }
    }

    /// Verifies the expanded command set resolved from the renderer capability.
    ///
    /// Resolves capability permissions through generated ACL manifests to verify the
    /// concrete command set exposed to the webview.
    #[test]
    fn the_resolved_capability_reaches_only_those_three_commands() {
        let manifests = acl_manifests();
        let capabilities: serde_json::Value =
            serde_json::from_str(&read_generated("capabilities.json"))
                .expect("gen/schemas/capabilities.json is valid JSON");
        let capability = &capabilities["default"];
        assert_eq!(capability["windows"][0], super::MAIN);
        assert_eq!(capability["local"], true);

        let mut commands = std::collections::BTreeSet::new();
        for permission in capability["permissions"]
            .as_array()
            .expect("the generated capability lists permissions")
        {
            resolve(
                &manifests,
                permission.as_str().expect("permissions are strings"),
                "core",
                &mut std::collections::BTreeSet::new(),
                &mut commands,
            );
        }

        assert_eq!(
            commands,
            RENDERER_COMMANDS
                .iter()
                .map(|command| (*command).to_owned())
                .collect::<std::collections::BTreeSet<_>>()
        );

        // Verify that sensitive commands remain inaccessible to the webview.
        for forbidden in [
            "core:image|from_path",
            "core:image|rgba",
            "core:image|new",
            "core:menu|new",
            "core:tray|new",
            "core:path|resolve",
            "core:path|resolve_directory",
            "core:webview|internal_toggle_devtools",
            "core:window|internal_toggle_maximize",
            "core:event|emit",
            "core:event|emit_to",
        ] {
            assert!(
                !commands.contains(forbidden),
                "forbidden command reachable from webview: {forbidden}"
            );
        }
    }

    /// `tauri-build` generates these files during build (excluded via
    /// `.gitignore`). They project Tauri's internal ACL rather than crate
    /// source. Read them from disk so the desktop build need not stage `gen/`.
    fn read_generated(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("gen/schemas")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("{} is generated by tauri-build: {error}", path.display())
        })
    }

    fn acl_manifests() -> serde_json::Value {
        serde_json::from_str(&read_generated("acl-manifests.json"))
            .expect("gen/schemas/acl-manifests.json is valid JSON")
    }

    /// Split `core:event:allow-listen` into the manifest it lives in and the
    /// permission inside it. A bare `allow-listen` inside a set belongs to the
    /// manifest that listed it, which is what `context` carries.
    fn manifest_of<'a>(permission: &'a str, context: &str) -> (String, &'a str) {
        if let Some(rest) = permission.strip_prefix("core:") {
            return match rest.split_once(':') {
                Some((module, identifier)) => (format!("core:{module}"), identifier),
                None => ("core".to_owned(), rest),
            };
        }
        match permission.split_once(':') {
            Some((plugin, identifier)) => (plugin.to_owned(), identifier),
            None => (context.to_owned(), permission),
        }
    }

    /// Expand one capability entry into the commands it allows.
    ///
    /// Sets and defaults nest, so this recurses and remembers what it has
    /// already expanded. Only `allow` is collected: this asks what a webview
    /// can reach, and a `deny` entry cannot widen that.
    fn resolve(
        manifests: &serde_json::Value,
        permission: &str,
        context: &str,
        seen: &mut std::collections::BTreeSet<String>,
        commands: &mut std::collections::BTreeSet<String>,
    ) {
        let (manifest_name, identifier) = manifest_of(permission, context);
        if !seen.insert(format!("{manifest_name}:{identifier}")) {
            return;
        }
        let manifest = manifests
            .get(&manifest_name)
            .unwrap_or_else(|| panic!("{manifest_name} is a known ACL manifest"));

        let nested = if identifier == "default" {
            Some(&manifest["default_permission"]["permissions"])
        } else {
            manifest["permission_sets"]
                .get(identifier)
                .map(|set| &set["permissions"])
        };
        if let Some(nested) = nested {
            for entry in nested
                .as_array()
                .unwrap_or_else(|| panic!("{manifest_name}:{identifier} lists permissions"))
            {
                resolve(
                    manifests,
                    entry.as_str().expect("permissions are strings"),
                    &manifest_name,
                    seen,
                    commands,
                );
            }
            return;
        }

        let permission = manifest["permissions"]
            .get(identifier)
            .unwrap_or_else(|| panic!("{manifest_name}:{identifier} is a known permission"));
        for command in permission["commands"]["allow"]
            .as_array()
            .expect("a permission lists the commands it allows")
        {
            commands.insert(format!(
                "{manifest_name}|{}",
                command.as_str().expect("command names are strings")
            ));
        }
    }
}
