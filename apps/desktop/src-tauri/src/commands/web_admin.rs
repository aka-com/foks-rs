//! An isolated, non-persistent hosted-admin window with no main-window IPC authority.
use super::{
    context::AppState,
    validation::{invalid_request, invalid_response, require_main_window, valid_local_name},
};
use crate::{agent::AgentError, applock::AppLock};
use foks_agent_proto::{
    admin::{AdminAction, AdminNavigation, AdminPolicy},
    Operation, SecretString,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};
use tauri::{Manager as _, State};
use url::Url;
struct Binding {
    origin: String,
    lock: Arc<AppLock>,
    generation: u64,
    expires: std::time::Instant,
}
fn windows() -> &'static Mutex<BTreeMap<String, Binding>> {
    static WINDOWS: OnceLock<Mutex<BTreeMap<String, Binding>>> = OnceLock::new();
    WINDOWS.get_or_init(Default::default)
}
fn same_origin(origin: &str, url: &Url) -> bool {
    url.scheme() == "https"
        && url.origin().ascii_serialization() == origin
        && url.username().is_empty()
        && url.password().is_none()
        && url.as_str().len() <= 8192
}
pub(crate) fn navigation_allowed(label: &str, url: &Url) -> bool {
    windows()
        .lock()
        .ok()
        .and_then(|m| {
            m.get(label).map(|b| {
                b.lock.permits_generation(b.generation)
                    && std::time::Instant::now() < b.expires
                    && same_origin(&b.origin, url)
            })
        })
        .unwrap_or(false)
}
pub(crate) fn close_all(app: &tauri::AppHandle) {
    let keys = windows()
        .lock()
        .map(|mut m| {
            let keys = m.keys().cloned().collect::<Vec<_>>();
            m.clear();
            keys
        })
        .unwrap_or_default();
    for key in keys {
        if let Some(window) = app.get_webview_window(&key) {
            let _ = window.close();
        }
    }
}
fn checked_navigation(p: AdminNavigation, expected: &AdminPolicy) -> Result<Url, AgentError> {
    if p.profile != expected.profile
        || p.account_alias != expected.account_alias
        || p.host_id != expected.host_id
        || p.uid != expected.uid
        || p.destination != expected.destination
    {
        return Err(invalid_response(
            "Admin handoff changed the selected account or destination.",
        ));
    }
    let destination = Url::parse(&expected.destination)
        .map_err(|_| invalid_response("Invalid admin destination."))?;
    let url = Url::parse(p.url.expose()).map_err(|_| invalid_response("Invalid admin handoff."))?;
    if destination.path() != "/"
        || destination.query().is_some()
        || destination.fragment().is_some()
        || !same_origin(&destination.origin().ascii_serialization(), &url)
        || url.path() != "/"
        || url.fragment().is_some()
        || url.query().is_none()
    {
        return Err(invalid_response("Admin destination is not allowed."));
    }
    Ok(url)
}
#[tauri::command]
pub async fn configure_web_admin(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
    destination: String,
) -> Result<serde_json::Value, AgentError> {
    require_main_window(&webview)?;
    let generation = crate::applock::unlocked_generation(webview.app_handle())?;
    if !valid_local_name(&profile) || !valid_local_name(&account_alias) || destination.len() > 2048
    {
        return Err(invalid_request("Invalid admin configuration."));
    }
    let _mutation = state.begin_mutation()?;
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        super::servers::require_transport_profile(transport.as_ref(), &profile)?;
        transport
            .call(Operation::WebAdmin {
                profile,
                account_alias,
                action: AdminAction::Configure { destination },
            })
            .map_err(AgentError::from_desktop)
    })
    .await
    .map_err(|_| invalid_request("Admin configuration interrupted."))??;
    crate::applock::require_unlocked_generation(webview.app_handle(), generation)?;
    close_all(webview.app_handle());
    Ok(serde_json::json!({"ok":true}))
}
#[tauri::command]
pub async fn open_web_admin(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    account_alias: String,
    pin: Option<SecretString>,
) -> Result<serde_json::Value, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle().clone();
    let generation = crate::applock::unlocked_generation(&app)?;
    if !valid_local_name(&profile)
        || !valid_local_name(&account_alias)
        || !(AdminAction::Prepare { pin: pin.clone() }).validate()
    {
        return Err(invalid_request("Invalid admin account."));
    }
    let _mutation = state.begin_mutation()?;
    let transport = state.agent.transport();
    let expected_alias = account_alias.clone();
    let url = tauri::async_runtime::spawn_blocking(move || {
        super::servers::require_transport_profile(transport.as_ref(), &profile)?;
        let policy = transport
            .call(Operation::WebAdmin {
                profile: profile.clone(),
                account_alias: account_alias.clone(),
                action: AdminAction::Policy,
            })
            .map_err(AgentError::from_desktop)?;
        let expected: AdminPolicy = serde_json::from_value(policy)
            .map_err(|_| invalid_response("Invalid admin policy."))?;
        if expected.profile != profile || expected.account_alias != account_alias {
            return Err(invalid_response("Admin policy changed account."));
        }
        let value = transport
            .call(Operation::WebAdmin {
                profile,
                account_alias,
                action: AdminAction::Prepare { pin },
            })
            .map_err(AgentError::from_desktop)?;
        let p: AdminNavigation = serde_json::from_value(value)
            .map_err(|_| invalid_response("Invalid admin handoff."))?;
        checked_navigation(p, &expected)
    })
    .await
    .map_err(|_| invalid_request("Admin handoff interrupted."))??;
    crate::applock::require_unlocked_generation(&app, generation)?;
    close_all(&app);
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|_| invalid_request("Cannot create private admin window."))?;
    let label = format!(
        "host-admin-{}",
        nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let lock = app.state::<Arc<AppLock>>().inner().clone();
    windows()
        .lock()
        .map_err(|_| invalid_request("Cannot bind admin window."))?
        .insert(
            label.clone(),
            Binding {
                origin: url.origin().ascii_serialization(),
                lock,
                generation,
                expires: std::time::Instant::now() + std::time::Duration::from_secs(300),
            },
        );
    let window = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(url))
        .title(format!("Host administration · {expected_alias}"))
        .incognito(true)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build();
    let window = match window {
        Ok(w) => w,
        Err(_) => {
            windows().lock().ok().map(|mut m| m.remove(&label));
            return Err(invalid_request("Cannot open private admin window."));
        }
    };
    let closing = label.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            if let Ok(mut map) = windows().lock() {
                map.remove(&closing);
            }
        }
    });
    if crate::applock::require_unlocked_generation(&app, generation).is_err() {
        // Lock may have drained the registry before build returned. Close the
        // actual handle as well, so that interleaving cannot orphan a window.
        let _ = window.close();
        if let Ok(mut map) = windows().lock() {
            map.remove(&label);
        }
        return Err(invalid_request(
            "Application locked before admin navigation.",
        ));
    }
    let expiring_app = app.clone();
    let expiring_label = label.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        if let Some(window) = expiring_app.get_webview_window(&expiring_label) {
            let _ = window.close();
        }
        if let Ok(mut map) = windows().lock() {
            map.remove(&expiring_label);
        }
    });
    Ok(serde_json::json!({"ok":true}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admin_handoff_and_redirects_keep_the_selected_origin_and_identity() {
        let expected = AdminPolicy {
            profile: "host".into(),
            account_alias: "work".into(),
            host_id: "host-id".into(),
            uid: "uid".into(),
            destination: "https://admin.example/".into(),
        };
        let make = |url: &str| AdminNavigation {
            profile: expected.profile.clone(),
            account_alias: expected.account_alias.clone(),
            host_id: expected.host_id.clone(),
            uid: expected.uid.clone(),
            destination: expected.destination.clone(),
            url: SecretString::new(url),
        };
        assert!(
            checked_navigation(make("https://admin.example/?session=secret"), &expected).is_ok()
        );
        for url in [
            "https://evil.example/?session=secret",
            "https://admin.example.evil/?session=secret",
            "http://admin.example/?session=secret",
            "https://admin.example/?session=secret#x",
        ] {
            assert!(checked_navigation(make(url), &expected).is_err());
        }
        let mut wrong = make("https://admin.example/?session=secret");
        wrong.uid = "other".into();
        assert!(checked_navigation(wrong, &expected).is_err());
        assert!(same_origin(
            "https://admin.example",
            &Url::parse("https://admin.example/users").unwrap()
        ));
        assert!(!same_origin(
            "https://admin.example",
            &Url::parse("https://attacker.example/redirect").unwrap()
        ));
        assert!(!navigation_allowed(
            "host-admin-unregistered",
            &Url::parse("https://admin.example").unwrap()
        ));
    }
}
