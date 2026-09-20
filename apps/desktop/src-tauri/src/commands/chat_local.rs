pub mod platform;
// Device-local notification preferences and unlocked session boundary.
use crate::{
    agent::AgentError,
    commands::{require_main_window, AppState},
};
use foks_agent_proto::chat::ChatScope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    sync::Mutex,
};
use tauri::{Emitter, Manager, State};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub previews: bool,
    pub overrides: BTreeMap<String, bool>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    RecoverIntents {},
    Begin,
    End {
        epoch: String,
    },
    Configure {
        enabled: Option<bool>,
        previews: Option<bool>,
        #[serde(rename = "storeId")]
        store_id: Option<String>,
        scope: Option<ChatScope>,
        channel: Option<String>,
        mode: Option<bool>,
    },
    TakeActivation,
    Clear {
        epoch: String,
    },
    Display {
        epoch: String,
        #[serde(rename = "storeId")]
        store_id: String,
        scope: ChatScope,
        channel: String,
        body: Option<String>,
        count: u32,
        incomplete: bool,
    },
}
#[derive(Serialize)]
pub struct Session {
    pub epoch: String,
    pub available: bool,
    pub settings: Settings,
    pub activation: Option<Route>,
    #[serde(rename = "migrationRemaining", skip_serializing_if = "Option::is_none")]
    pub migration_remaining: Option<usize>,
}
#[derive(Default)]
pub struct LocalState(pub Mutex<Inner>);
#[derive(Default)]
pub struct Inner {
    pub settings: Settings,
    pub loaded: bool,
    pub epoch: String,
    pub generation: Option<u64>,
    routes: BTreeMap<String, Route>,
    pending: Option<Route>,
    pub scopes: BTreeMap<String, (ChatScope, u64)>,
}
fn error(message: &str) -> AgentError {
    AgentError::new("chat-notification", message, false)
}
fn digest(parts: &[&str]) -> String {
    Sha256::digest(serde_json::to_vec(parts).expect("string array"))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn key(scope: &ChatScope, channel: &str) -> String {
    format!(
        "{}/{}",
        digest(&[&scope.store.profile]),
        digest(&[&scope.host, &scope.actor, &scope.store.team_id, channel])
    )
}
fn epoch() -> Result<String, AgentError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| error("Could not start local alerts."))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AgentError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| error("Local settings path unavailable."))?
        .join("chat-local");
    if std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(error(
            "Local settings directory must not be a symbolic link.",
        ));
    }
    std::fs::create_dir_all(&dir).map_err(|_| error("Could not create local settings."))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| error("Could not protect local settings."))?;
    }
    Ok(dir.join("notifications.json"))
}
pub(super) fn intent_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AgentError> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|_| error("Local chat storage path unavailable."))?
        .join("chat-intents"))
}
fn load(path: &std::path::Path) -> Result<Settings, AgentError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = match options.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(_) => return Err(error("Could not read local settings.")),
    };
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("Could not read local settings."))?;
    if bytes.len() > 1024 * 1024 {
        return Err(error("Local settings exceed the size limit."));
    }
    let settings: Settings =
        serde_json::from_slice(&bytes).map_err(|_| error("Local settings are invalid."))?;
    if settings.overrides.len() > 4096
        || settings.overrides.keys().any(|k| {
            k.len() != 129
                || k.as_bytes()[64] != b'/'
                || !k
                    .bytes()
                    .enumerate()
                    .all(|(i, c)| i == 64 || c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
    {
        return Err(error("Local settings are invalid."));
    }
    Ok(settings)
}
fn save(path: &std::path::Path, settings: &Settings) -> Result<(), AgentError> {
    let mut temp = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| error("Local settings path unavailable."))?,
    )
    .map_err(|_| error("Could not save local settings."))?;
    temp.write_all(&serde_json::to_vec(settings).map_err(|_| error("Invalid local settings."))?)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|_| error("Could not save local settings."))?;
    temp.persist(path)
        .map_err(|_| error("Could not save local settings."))?;
    std::fs::File::open(path.parent().unwrap())
        .and_then(|f| f.sync_all())
        .map_err(|_| error("Could not finish saving local settings."))?;
    Ok(())
}
impl Inner {
    fn end(&mut self, epoch: &str) -> bool {
        if epoch != self.epoch {
            return false;
        }
        self.generation = None;
        self.routes.clear();
        true
    }
    fn session(&self) -> Session {
        Session {
            epoch: self.epoch.clone(),
            available: platform::available(),
            activation: None,
            migration_remaining: None,
            settings: self.settings.clone(),
        }
    }
}
/// Opens the system pane where desktop alerts are permitted. The destination
/// is fixed, so nothing the webview says can redirect it.
#[tauri::command]
pub async fn open_notification_settings(
    webview: tauri::Webview,
) -> Result<serde_json::Value, AgentError> {
    require_main_window(&webview)?;
    tauri::async_runtime::spawn_blocking(platform::open_settings)
        .await
        .map_err(|_| error("Opening notification settings was interrupted."))??;
    Ok(serde_json::json!({"ok": true}))
}

#[tauri::command]
pub async fn chat_local(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    local: State<'_, LocalState>,
    action: Action,
) -> Result<Session, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle();
    if matches!(
        &action,
        Action::Configure {
            enabled: Some(true),
            ..
        }
    ) {
        let generation = crate::applock::unlocked_generation(app)?;
        if !platform::permission().await {
            // Its own code: the shell offers the settings pane for this one
            // refusal, and states the rest as plain failures.
            return Err(AgentError::new(
                "chat-notification-permission",
                "Desktop alerts were not permitted. Check macOS notification settings.",
                false,
            ));
        }
        crate::applock::require_unlocked_generation(app, generation)?;
    }
    if matches!(&action, Action::RecoverIntents {}) {
        let generation = crate::applock::unlocked_generation(app)?;
        let owned_app = app.clone();
        let state = state.inner().clone();
        let remaining = tauri::async_runtime::spawn_blocking(move || {
            super::chat_migration::recover(&owned_app, &state, generation)
        })
        .await
        .map_err(|_| error("Saved message recovery was interrupted."))??;
        crate::applock::require_unlocked_generation(app, generation)?;
        let mut inner = local
            .0
            .lock()
            .map_err(|_| error("Local settings unavailable."))?;
        if inner.epoch.is_empty() {
            inner.epoch = epoch()?;
        }
        let mut session = inner.session();
        session.migration_remaining = Some(remaining);
        return Ok(session);
    }
    let mut inner = local
        .0
        .lock()
        .map_err(|_| error("Local alerts unavailable."))?;
    if let Action::End { epoch } = &action {
        if inner.end(epoch) {
            platform::clear();
        }
        return Ok(inner.session());
    }
    let generation = crate::applock::unlocked_generation(app)?;
    if !inner.loaded {
        inner.settings = load(&path(app)?)?;
        inner.loaded = true;
    }
    if inner.epoch.is_empty() {
        inner.epoch = epoch()?;
    }
    match action {
        Action::Clear { epoch } => {
            if epoch == inner.epoch {
                inner.routes.clear();
                platform::clear();
            }
        }
        Action::TakeActivation => {
            let mut result = inner.session();
            result.activation = inner.pending.take();
            return Ok(result);
        }
        Action::Begin => {
            inner.routes.clear();
            platform::clear();
            inner.epoch = epoch()?;
            inner.generation = Some(generation);
        }
        Action::Configure {
            enabled,
            previews,
            store_id,
            scope,
            channel,
            mode,
        } => {
            inner.generation = None;
            inner.routes.clear();
            platform::clear();
            let mut settings = inner.settings.clone();
            if let Some(value) = enabled {
                settings.enabled = value;
            }
            if let Some(value) = previews {
                settings.previews = value;
            }
            if let Some(scope) = scope {
                let ch = channel.ok_or_else(|| error("A channel is required."))?;
                if !foks_agent_proto::chat::valid_chat_id(&ch) {
                    return Err(error("Invalid channel."));
                }
                // Revalidate the catalog binding rather than accepting an alias-only key.
                let id = store_id.ok_or_else(|| error("A verified store is required."))?;
                verify_scope(&state, &inner, &id, &scope)?;
                let key = key(&scope, &ch);
                if let Some(value) = mode {
                    if !settings.overrides.contains_key(&key) && settings.overrides.len() >= 4096 {
                        return Err(error("Local channel setting limit reached."));
                    }
                    settings.overrides.insert(key, value);
                } else {
                    settings.overrides.remove(&key);
                }
            }
            save(&path(app)?, &settings)?;
            inner.settings = settings;
            inner.epoch = epoch()?;
            inner.generation = Some(generation);
        }
        Action::Display {
            epoch: expected,
            store_id,
            scope,
            channel,
            body,
            count,
            incomplete,
        } => {
            if !platform::available()
                || inner.epoch != expected
                || inner.generation != Some(generation)
                || !inner.settings.enabled
            {
                return Err(error("Local alert session expired."));
            }
            verify_scope(&state, &inner, &store_id, &scope)?;
            if !foks_agent_proto::chat::valid_chat_id(&channel)
                || count > 100
                || body.as_ref().is_some_and(|b| b.chars().count() > 256)
            {
                return Err(error("Invalid local alert."));
            }
            if inner.settings.overrides.get(&key(&scope, &channel)) == Some(&false) {
                return Err(error("Channel alerts are disabled."));
            }
            if inner.routes.len() >= 64 {
                inner.routes.clear();
                platform::clear();
            }
            let token = epoch()?;
            let route = Route {
                store_id,
                scope,
                channel,
                generation,
            };
            let text = if inner.settings.previews {
                body.unwrap_or_else(|| alert_text(count, incomplete))
            } else {
                alert_text(count, incomplete)
            };
            inner.routes.insert(token.clone(), route);
            crate::applock::require_unlocked_generation(app, generation)?;
            platform::display(app, &token, &text)?;
        }
        Action::End { .. } | Action::RecoverIntents {} => unreachable!(),
    }
    Ok(inner.session())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_command_rejects_intent_io_and_arbitrary_migration_paths() {
        assert!(
            serde_json::from_value::<Action>(serde_json::json!({"action":"load-intent"})).is_err()
        );
        assert!(
            serde_json::from_value::<Action>(serde_json::json!({"action":"recover-intents"}))
                .is_ok()
        );
        assert!(serde_json::from_value::<Action>(
            serde_json::json!({"action":"recover-intents", "path":"/arbitrary"})
        )
        .is_err());
    }

    #[test]
    fn stale_cleanup_cannot_close_replacement_session() {
        let mut state = Inner {
            epoch: "new".into(),
            generation: Some(7),
            ..Default::default()
        };
        assert!(!state.end("old"));
        assert_eq!(state.generation, Some(7));
        assert!(state.end("new"));
        assert_eq!(state.generation, None);
        assert_eq!(alert_text(1, false), "New chat message.");
        assert!(alert_text(5, true).contains("could not be checked"));
    }
    #[test]
    fn notification_scope_uses_its_profile_generation() {
        let state = AppState::new(std::sync::Arc::new(crate::agent::AgentHandle::new(
            "/tmp/unused-notification-agent.sock".into(),
        )));
        let scope = ChatScope {
            host: "02".to_owned() + &"ab".repeat(32),
            actor: "01".to_owned() + &"ab".repeat(32),
            store: foks_agent_proto::TeamStoreRef {
                profile: "chat".into(),
                account_alias: "owner".into(),
                team_alias: "team".into(),
                team_id: "03".to_owned() + &"ab".repeat(32),
            },
        };
        let id = super::super::vault::store_id(&foks_desktop::CatalogStoreRef::Team(
            scope.store.clone(),
        ));
        *state.catalog.lock().unwrap() = Some(foks_desktop::CatalogSnapshot {
            profiles: vec!["chat".into(), "other".into()],
            stores: vec![foks_desktop::CatalogStoreSummary::Team {
                store: scope.store.clone(),
                kind: "named".into(),
                name: Some("team".into()),
                active: true,
                creation_phase: None,
                chain_seqno: None,
            }],
            ..Default::default()
        });
        let chat = state.for_profile("chat").unwrap();
        let generation = chat
            .chat_generation
            .load(std::sync::atomic::Ordering::Acquire);
        let mut inner = Inner::default();
        inner.scopes.insert(id.clone(), (scope.clone(), generation));
        assert!(verify_scope(&state, &inner, &id, &scope).is_ok());
        state.for_profile("other").unwrap().invalidate_catalog();
        assert!(verify_scope(&state, &inner, &id, &scope).is_ok());
        chat.invalidate_catalog();
        assert!(verify_scope(&state, &inner, &id, &scope).is_err());
    }

    #[test]
    fn settings_are_bounded_private_and_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s");
        let mut s = Settings {
            enabled: true,
            ..Default::default()
        };
        s.overrides
            .insert(format!("{}/{}", "a".repeat(64), "b".repeat(64)), false);
        save(&p, &s).unwrap();
        assert!(load(&p).unwrap().enabled);
        assert_eq!(load(&p).unwrap().overrides, s.overrides);
        std::fs::write(
            &p,
            b"{\"enabled\":true,\"previews\":false,\"overrides\":{\"alias\":true}}",
        )
        .unwrap();
        assert!(load(&p).is_err());
    }
}

pub fn remember(
    app: &tauri::AppHandle,
    id: &str,
    scope: &ChatScope,
    generation: u64,
    unlocked: u64,
) {
    if crate::applock::require_unlocked_generation(app, unlocked).is_err() {
        return;
    }
    if let Some(local) = app.try_state::<LocalState>() {
        if let Ok(mut inner) = local.0.lock() {
            if inner.scopes.len() >= 4096 && !inner.scopes.contains_key(id) {
                inner.scopes.clear();
            }
            inner
                .scopes
                .insert(id.to_owned(), (scope.clone(), generation));
        }
    }
}
fn verify_scope(
    state: &AppState,
    inner: &Inner,
    id: &str,
    scope: &ChatScope,
) -> Result<(), AgentError> {
    let (generation, selected) = state.for_profile(&scope.store.profile)?.selected_chat(id)?;
    if inner.scopes.get(id) != Some(&(scope.clone(), generation)) {
        return Err(error("Refresh this chat before changing local alerts."));
    }
    if selected != scope.store {
        return Err(error("The selected account changed."));
    }
    Ok(())
}

pub fn forget_profile(app: &tauri::AppHandle, profile: &str) -> Result<(), AgentError> {
    let local = app.state::<LocalState>();
    let mut inner = local
        .0
        .lock()
        .map_err(|_| error("Local settings unavailable."))?;
    let path = path(app)?;
    let mut settings = if inner.loaded {
        inner.settings.clone()
    } else {
        load(&path)?
    };
    let prefix = format!("{}/", digest(&[profile]));
    settings
        .overrides
        .retain(|key, _| !key.starts_with(&prefix));
    save(&path, &settings)?;
    inner.settings = settings;
    inner.loaded = true;
    inner.generation = None;
    inner.routes.clear();
    platform::clear();
    inner
        .scopes
        .retain(|_, (scope, _)| scope.store.profile != profile);
    Ok(())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub store_id: String,
    pub scope: ChatScope,
    pub channel: String,
    #[serde(skip)]
    generation: u64,
}
fn alert_text(count: u32, incomplete: bool) -> String {
    if incomplete {
        "New chat activity. Some messages could not be checked.".into()
    } else if count == 1 {
        "New chat message.".into()
    } else {
        format!("{count} additional chat messages.")
    }
}
pub fn route_current(app: &tauri::AppHandle, token: &str) -> bool {
    let local = app.state::<LocalState>();
    let Ok(inner) = local.0.lock() else {
        return false;
    };
    inner.routes.get(token).is_some_and(|r| {
        verify_scope(&app.state::<AppState>(), &inner, &r.store_id, &r.scope).is_ok()
            && inner.generation == Some(r.generation)
            && crate::applock::require_unlocked_generation(app, r.generation).is_ok()
    })
}
pub fn activate(app: &tauri::AppHandle, token: &str) {
    let local = app.state::<LocalState>();
    if let Ok(mut inner) = local.0.lock() {
        if let Some(route) = inner.routes.remove(token) {
            inner.pending = Some(route);
        }
    }
    if let Some(window) = app.get_webview_window(crate::MAIN) {
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit_to(crate::MAIN, "foks://chat-notification", ());
}
pub fn delivery_failed(app: &tauri::AppHandle) {
    let _ = app.emit_to(crate::MAIN, "foks://chat-notification-error", ());
}
pub fn conceal(app: &tauri::AppHandle) {
    let local = app.state::<LocalState>();
    if let Ok(mut inner) = local.0.lock() {
        inner.generation = None;
    }
    platform::clear();
}
