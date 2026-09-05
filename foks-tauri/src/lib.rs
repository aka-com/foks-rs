//! The FOKS desktop application.
//!
//! A second Tauri app beside `src-tauri`'s AKA, sharing the workspace and the
//! toolchain and nothing else — not the bundle identifier, the release train,
//! the command surface or the trust boundary. FOKS talks only to a local agent
//! over a private Unix socket and owns no server.
//!
//! Two settings decided here and visible in `tauri.conf.json`:
//! `dragDropEnabled: true`, so file drops arrive in Rust as paths (see
//! [`dragdrop`]), and `withGlobalTauri: false`, so the IPC surface is not
//! published on a global object in a window that renders secrets.

mod agent;
mod applock;
mod clipboard;
mod commands;
mod dragdrop;
mod navigation;
mod startup;
mod window_state;

use std::sync::Arc;

use tauri::Manager as _;
use tauri_plugin_dialog::{DialogExt as _, MessageDialogKind};

use agent::AgentHandle;
use commands::{AppState, MAIN};

pub fn run() {
    // `args_os`, not `args`: the latter panics on an argument that is not valid
    // UTF-8, and this process is handed flags it does not own. Resolve once so
    // only the default managed endpoint grants us crash-marker storage.
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
            tracing::error!(reason = %error.message, "FOKS could not prepare private crash storage");
            std::process::exit(1);
        }
        foks_desktop::install_crash_reporter(directory);
    }

    let Some(endpoint) = endpoint else {
        // No dialog: without a HOME there is no desktop session to show one in.
        tracing::error!(
            "FOKS could not work out where its agent socket is; pass {} or set {}",
            agent::SOCKET_ARG,
            agent::SOCKET_ENV
        );
        std::process::exit(1);
    };
    let socket = endpoint.socket;
    tracing::info!(socket = %socket.display(), "FOKS is using this agent socket");
    let agent = Arc::new(AgentHandle::new(socket));

    tauri::Builder::default()
        // Must be the first plugin registered: a duplicate launch hands off to
        // the running instance and exits here, before it can race for the
        // agent socket inside the setup hook.
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
        // Registered for the Rust-side copy-with-clear of Phase 2. The webview
        // is granted no clipboard permission; see PERMISSIONS.md.
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState::new(Arc::clone(&agent)))
        .manage(Arc::new(applock::AppLock::new()))
        .invoke_handler(tauri::generate_handler![
            commands::agent_status,
            commands::retry_agent_connection,
            commands::initialize_client_state,
            commands::discover_go_profiles,
            commands::check_and_add_profile,
            commands::check_and_add_go_profile,
            commands::add_server,
            commands::forget_server,
            commands::describe_server_status,
            commands::check_server,
            commands::list_pending_operations,
            commands::create_first_run_account,
            commands::resume_first_run_account,
            commands::set_first_run_passphrase,
            commands::prepare_owner_backup,
            commands::commit_owner_backup,
            commands::recover_owner_account,
            commands::resume_owner_recovery,
            commands::discover_groups,
            commands::list_account_devices,
            commands::remove_account_device,
            commands::list_backup_enrollments,
            commands::revoke_owner_backup,
            commands::list_yubi_cards,
            commands::list_yubi_accounts,
            commands::create_yubi_account,
            commands::resume_yubi_account,
            commands::provision_yubi_device,
            commands::sync_yubi_account,
            commands::yubi_pin_status,
            commands::change_yubi_pin,
            commands::set_yubi_passphrase,
            commands::change_yubi_passphrase,
            commands::verify_yubi_passphrase,
            commands::change_yubi_puk,
            commands::unblock_yubi_pin,
            commands::rotate_yubi_management_key,
            commands::resume_yubi_management_key,
            commands::recover_yubi_management_key,
            commands::recover_yubi_subkey,
            commands::revoke_yubi_device,
            commands::start_device_pairing,
            commands::resume_device_pairing_offer,
            commands::finish_device_pairing,
            commands::accept_device_pairing,
            commands::accept_go_profile_pairing,
            commands::resume_device_pairing_acceptance,
            commands::resume_go_profile_pairing,
            commands::copy_go_profile_device,
            commands::set_account_passphrase,
            commands::change_account_passphrase,
            commands::verify_account_passphrase,
            commands::describe_reset,
            commands::reset_server,
            commands::app_info,
            window_state::get_window_state,
            commands::list_stores,
            commands::list_catalog,
            commands::list_servers,
            commands::list_accounts,
            commands::list_group_details,
            commands::list_parties,
            commands::list_federation,
            commands::read_item,
            commands::copy_item_value,
            commands::copy_item_path,
            commands::copy_text,
            commands::download_file,
            commands::create_text_item,
            commands::create_link,
            commands::create_folder,
            commands::edit_text_item,
            commands::remove_item,
            commands::import_dropped_file,
            commands::pick_and_import_file,
            commands::replace_dropped_file,
            commands::pick_and_replace_file,
            commands::create_group,
            commands::resume_group_creation,
            commands::add_group_member,
            commands::resume_group_member_addition,
            commands::demote_group_member,
            commands::remove_group_member,
            commands::resume_group_member_edit,
            commands::admit_group,
            commands::rerun_group_admission,
            commands::expel_federated_group,
            commands::take_agent_connection_loss,
            applock::app_lock_state,
            applock::lock_app,
            applock::unlock_app,
        ])
        .setup(move |app| {
            // Tauri has already created the config-declared webview by the
            // time setup runs, so this does not precede the window — it
            // precedes the first request. An unreachable agent becomes a dialog
            // and a clean exit rather than a shell that fails on every action.
            startup::require_agent(app, &agent);
            if let Some(window) = app.get_webview_window(MAIN) {
                dragdrop::observe(&window);
                window_state::observe(&window);
            } else {
                tracing::error!("the {MAIN} window is missing from tauri.conf.json");
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("the FOKS desktop application could not start")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                agent::terminate_managed_agent();
                clipboard::defer_exit_cleanup(app, code, &api);
            }
        });
}

#[cfg(test)]
mod tests {
    /// The window-label gate is only meaningful while there is exactly one
    /// window to gate on. If a second window is ever added to
    /// `tauri.conf.json`, this fails and the posture in PERMISSIONS.md has to
    /// be rewritten rather than silently widened.
    #[test]
    fn the_configuration_declares_exactly_one_window_and_it_is_main() {
        let configuration: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json"))
                .expect("tauri.conf.json is valid JSON");
        let windows = configuration["app"]["windows"]
            .as_array()
            .expect("tauri.conf.json declares windows");
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0]["label"], super::MAIN);
        assert_eq!(windows[0]["dragDropEnabled"], true);
        assert_eq!(configuration["app"]["withGlobalTauri"], false);
    }

    /// The exact renderer capability, in the order the file lists it.
    ///
    /// The webview needs three things and is granted three things: the two
    /// halves of the drag-drop event subscription (`dragdrop.rs` emits, the
    /// renderer listens), and the start-dragging command Tauri's own
    /// `data-tauri-drag-region` script calls to move an overlay-title-bar
    /// window. Everything else — image, menu, tray, path, app, webview, the
    /// rest of window, `emit`/`emit_to`, and every plugin API — is absent.
    const RENDERER_PERMISSIONS: [&str; 3] = [
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "core:window:allow-start-dragging",
    ];

    /// The command identifiers the resolved capability grants, as
    /// `<manifest>|<command>`. This is the whole IPC surface a script running
    /// in the FOKS window can reach that FOKS did not write itself.
    const RENDERER_COMMANDS: [&str; 3] = [
        "core:event|listen",
        "core:event|unlisten",
        "core:window|start_dragging",
    ];

    /// The capability file is the audited surface, and it is audited exactly:
    /// an added entry, a dropped entry, a duplicate, a reordering or a second
    /// window all fail here. A future Tauri bump that makes a broad set
    /// convenient again has to change this list in the same commit, in front
    /// of a reviewer, rather than widen the grant quietly.
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
            "a duplicated permission hides what the capability really grants"
        );

        // Named so the diff that reintroduces one of them fails on the name a
        // reviewer would recognise, not only on the set comparison above.
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
                "{forbidden} is back in the capability"
            );
        }
        for permission in &permissions {
            assert!(
                permission.starts_with("core:"),
                "{permission} is a plugin permission; FOKS drives its plugins from Rust"
            );
        }
    }

    /// What the capability *resolves to*, which is the thing that matters.
    ///
    /// `core:default` looked like three entries in the file and was 92
    /// commands after expansion. This walks `tauri-build`'s own generated
    /// projection — the capability as ingested, resolved through the ACL
    /// manifests — so the assertion is about command identifiers a webview can
    /// invoke rather than about how the file happens to be written. Command
    /// identifiers, not descriptions: a Tauri documentation change must not
    /// make this fail, and a Tauri permission-set change must.
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

        // The specific commands the old `core:default` grant carried, by the
        // identifier the ACL uses for each. `core:image|from_path` read any
        // file the webview named; `core:webview|internal_toggle_devtools`
        // opened an inspector over a window that renders secrets.
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
                "{forbidden} is reachable from the FOKS webview again"
            );
        }
    }

    /// `tauri-build` writes these next to the crate on every build, and
    /// `.gitignore`s them: they are a projection of `tauri`'s own ACL, not
    /// source. Read them from disk rather than `include_str!` so the Bazel
    /// library build — which stages a different manifest directory and does
    /// not carry `gen/` — is unaffected either way.
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
