//! Rust-side gate for every command that can expose a vault value.
//!
//! The window label is only defence in depth. The app lock is the runtime
//! control: even script executing in the sole webview cannot read a value
//! while this state is locked. Authentication is delegated to the operating
//! system; FOKS never receives an account password.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Manager as _, State};

use crate::agent::AgentError;

#[cfg(all(unix, not(target_os = "macos")))]
mod polkit;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockStateDto {
    pub locked: bool,
    pub available: bool,
    pub mechanism: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

pub struct AppLock {
    locked: AtomicBool,
    authenticating: AtomicBool,
    capability: Capability,
}

impl AppLock {
    pub fn new() -> Self {
        Self::for_capability(platform_capability())
    }

    fn for_capability(capability: Capability) -> Self {
        Self {
            // The lock is a useful command boundary only after it has actually
            // been armed. Start closed on platforms that can present the OS
            // authenticator; unavailable platforms must remain open so the
            // user cannot be stranded behind an impossible prompt.
            locked: AtomicBool::new(capability.available),
            authenticating: AtomicBool::new(false),
            capability,
        }
    }

    pub fn state(&self) -> LockStateDto {
        LockStateDto {
            locked: self.locked.load(Ordering::Acquire),
            available: self.capability.available,
            mechanism: self.capability.mechanism,
            unavailable_reason: self.capability.reason.clone(),
        }
    }

    fn lock(&self) -> Result<(), AgentError> {
        if !self.capability.available {
            return Err(AgentError::new(
                "app-lock-unavailable",
                self.capability.reason.clone().unwrap_or_else(|| {
                    "The operating system cannot authenticate this account.".to_owned()
                }),
                false,
            ));
        }
        self.locked.store(true, Ordering::Release);
        Ok(())
    }
}

impl Default for AppLock {
    fn default() -> Self {
        Self::new()
    }
}

struct AuthenticationGuard<'a>(&'a AtomicBool);

impl Drop for AuthenticationGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub fn require_unlocked(app: &AppHandle) -> Result<(), AgentError> {
    match app.try_state::<Arc<AppLock>>() {
        Some(lock) if lock.locked.load(Ordering::Acquire) => Err(AgentError::new(
            "app-locked",
            "FOKS is locked. Unlock it to continue.",
            false,
        )),
        Some(_) => Ok(()),
        None => Err(AgentError::new(
            "app-lock-unavailable",
            "FOKS could not verify its app-lock state.",
            false,
        )),
    }
}

#[tauri::command]
pub fn app_lock_state(
    webview: tauri::Webview,
    lock: State<'_, Arc<AppLock>>,
) -> Result<LockStateDto, AgentError> {
    crate::commands::require_main_window(&webview)?;
    Ok(lock.state())
}

#[tauri::command]
pub fn lock_app(
    webview: tauri::Webview,
    lock: State<'_, Arc<AppLock>>,
) -> Result<LockStateDto, AgentError> {
    crate::commands::require_main_window(&webview)?;
    lock.lock()?;
    Ok(lock.state())
}

#[tauri::command]
pub async fn unlock_app(
    webview: tauri::Webview,
    lock: State<'_, Arc<AppLock>>,
) -> Result<LockStateDto, AgentError> {
    crate::commands::require_main_window(&webview)?;
    if !lock.locked.load(Ordering::Acquire) {
        return Ok(lock.state());
    }
    if lock.authenticating.swap(true, Ordering::AcqRel) {
        return Err(AgentError::new(
            "authentication-in-progress",
            "An unlock prompt is already open.",
            false,
        ));
    }
    let owned = Arc::clone(&lock);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = AuthenticationGuard(&owned.authenticating);
        match authenticate("Unlock FOKS to read vault items") {
            Ok(true) => owned.locked.store(false, Ordering::Release),
            Ok(false) => {}
            Err(message) => {
                return Err(AgentError::new("authentication-failed", message, false));
            }
        }
        Ok(owned.state())
    })
    .await
    .map_err(|error| AgentError::unknown(format!("the unlock prompt did not finish: {error}")))?
}

#[derive(Clone)]
struct Capability {
    available: bool,
    reason: Option<String>,
    mechanism: &'static str,
}

#[cfg(target_os = "macos")]
fn platform_capability() -> Capability {
    use objc2_local_authentication::{LAContext, LAPolicy};

    let context = unsafe { LAContext::new() };
    let biometry = unsafe {
        context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics)
    }
    .is_ok();
    let context = unsafe { LAContext::new() };
    match unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication) } {
        Ok(()) => Capability {
            available: true,
            reason: None,
            mechanism: if biometry { "biometry" } else { "password" },
        },
        Err(error) => Capability {
            available: false,
            reason: Some(format!(
                "macOS cannot authenticate this account right now ({}).",
                error.localizedDescription()
            )),
            mechanism: "none",
        },
    }
}

#[cfg(target_os = "macos")]
fn authenticate(reason: &str) -> Result<bool, String> {
    use block2::RcBlock;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    let (sender, receiver) = std::sync::mpsc::channel();
    let context = unsafe { LAContext::new() };
    let reason = NSString::from_str(reason);
    let handler = RcBlock::new(move |success: objc2::runtime::Bool, error: *mut NSError| {
        let outcome = if success.as_bool() {
            Ok(true)
        } else if error.is_null() {
            Ok(false)
        } else {
            let error = unsafe { &*error };
            match error.code() {
                -2 | -4 | -9 => Ok(false),
                _ => Err(error.localizedDescription().to_string()),
            }
        };
        let _ = sender.send(outcome);
    });
    unsafe {
        context.evaluatePolicy_localizedReason_reply(
            LAPolicy::DeviceOwnerAuthentication,
            &reason,
            &handler,
        );
    }
    const AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
    match receiver.recv_timeout(AUTH_TIMEOUT) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err("Authentication prompt timed out.".to_owned())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Ok(false),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_capability() -> Capability {
    static CAPABILITY: std::sync::OnceLock<Capability> = std::sync::OnceLock::new();
    CAPABILITY.get_or_init(polkit::capability).clone()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn authenticate(reason: &str) -> Result<bool, String> {
    polkit::authenticate(reason)
}

#[cfg(not(any(target_os = "macos", all(unix, not(target_os = "macos")))))]
fn platform_capability() -> Capability {
    Capability {
        available: false,
        reason: Some("The FOKS app lock is not available on this platform.".to_owned()),
        mechanism: "none",
    }
}

#[cfg(not(any(target_os = "macos", all(unix, not(target_os = "macos")))))]
fn authenticate(_reason: &str) -> Result<bool, String> {
    Err("The FOKS app lock is not available on this platform.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_never_arms_when_the_platform_cannot_unlock_it() {
        let lock = AppLock::default();
        if !platform_capability().available {
            assert_eq!(lock.lock().unwrap_err().code, "app-lock-unavailable");
            assert!(!lock.state().locked);
        }
    }

    #[test]
    fn a_fresh_lock_is_armed_exactly_when_authentication_is_available() {
        let available = AppLock::for_capability(Capability {
            available: true,
            reason: None,
            mechanism: "password",
        });
        assert!(available.state().locked);
        let unavailable = AppLock::for_capability(Capability {
            available: false,
            reason: Some("not installed".to_owned()),
            mechanism: "none",
        });
        assert!(!unavailable.state().locked);
        assert_eq!(
            unavailable.state().unavailable_reason.as_deref(),
            Some("not installed")
        );
    }
}
