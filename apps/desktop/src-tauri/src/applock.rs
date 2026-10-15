//! Rust-side gate for every command that can expose a vault value.
//!
//! Window labels provide defense-in-depth, while the app lock provides runtime
//! access control. Value-returning commands are blocked when locked.
//! Authentication is delegated to the operating system; FOKS does not store
//! account passwords.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
    publication: Mutex<()>,
    locked: AtomicBool,
    generation: AtomicU64,
    authenticating: AtomicBool,
    capability: Capability,
}

impl AppLock {
    pub fn new() -> Self {
        Self::for_capability(platform_capability())
    }

    fn for_capability(capability: Capability) -> Self {
        Self {
            publication: Mutex::new(()),
            // Platforms supporting OS authentication start locked. On unsupported
            // platforms, start unlocked to avoid blocking access.
            locked: AtomicBool::new(capability.available),
            generation: AtomicU64::new(0),
            authenticating: AtomicBool::new(false),
            capability,
        }
    }

    pub(crate) fn permits_generation(&self, generation: u64) -> bool {
        !self.locked.load(Ordering::Acquire)
            && self.generation.load(Ordering::Acquire) == generation
    }

    pub fn state(&self) -> LockStateDto {
        LockStateDto {
            locked: self.locked.load(Ordering::Acquire),
            available: self.capability.available,
            mechanism: self.capability.mechanism,
            unavailable_reason: self.capability.reason.clone(),
        }
    }

    fn begin_unlock(&self) -> Result<Option<u64>, AgentError> {
        let _publication = self
            .publication
            .lock()
            .map_err(|_| publication_unavailable())?;
        if !self.locked.load(Ordering::Acquire) {
            return Ok(None);
        }
        if self.authenticating.swap(true, Ordering::AcqRel) {
            return Err(AgentError::new(
                "authentication-in-progress",
                "An unlock prompt is already open.",
                false,
            ));
        }
        Ok(Some(self.generation.load(Ordering::Acquire)))
    }

    fn complete_unlock(&self, generation: u64) -> Result<(), AgentError> {
        let _publication = self
            .publication
            .lock()
            .map_err(|_| publication_unavailable())?;
        if self.generation.load(Ordering::Acquire) != generation {
            return Err(AgentError::new(
                "authentication-interrupted",
                "FOKS locked again while authentication was in progress. Unlock it again.",
                false,
            ));
        }
        self.locked.store(false, Ordering::Release);
        Ok(())
    }

    fn lock(&self) -> Result<(), AgentError> {
        if !self.capability.available {
            return Err(AgentError::new(
                "app-lock-unavailable",
                self.capability.reason.clone().unwrap_or_else(|| {
                    "Operating system authentication is unavailable for this account.".to_owned()
                }),
                false,
            ));
        }
        // A panicking publisher must not prevent the user from locking.
        let _publication = self
            .publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.locked.store(true, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

#[cfg(test)]
type PublicationCheckpoint = Arc<dyn Fn(&str) + Send + Sync>;

/// Owns one unlocked interval, never a later unlock. Slow work runs without the
/// publication mutex; only the final value handoff or external effect holds it.
#[derive(Clone)]
pub(crate) struct UnlockedInterval {
    lock: Arc<AppLock>,
    generation: u64,
    #[cfg(test)]
    checkpoint: Option<PublicationCheckpoint>,
}

fn publication_unavailable() -> AgentError {
    AgentError::new(
        "app-lock-unavailable",
        "Unable to verify application lock state.",
        false,
    )
}

impl UnlockedInterval {
    fn capture(lock: Arc<AppLock>) -> Result<Self, AgentError> {
        let publication = lock
            .publication
            .lock()
            .map_err(|_| publication_unavailable())?;
        let generation = lock.generation.load(Ordering::Acquire);
        if !lock.permits_generation(generation) {
            return Err(AgentError::new(
                "app-locked",
                "FOKS is locked. Unlock it to continue.",
                false,
            ));
        }
        drop(publication);
        Ok(Self {
            lock,
            generation,
            #[cfg(test)]
            checkpoint: None,
        })
    }

    pub(crate) fn check(&self) -> Result<(), AgentError> {
        if self.lock.permits_generation(self.generation) {
            Ok(())
        } else {
            Err(AgentError::new(
                "app-locked",
                "This operation was interrupted when FOKS locked. Try again after unlocking.",
                false,
            ))
        }
    }

    pub(crate) fn checkpoint(&self, _phase: &'static str) -> Result<(), AgentError> {
        #[cfg(test)]
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint(_phase);
        }
        self.check()
    }

    #[cfg(test)]
    pub(crate) fn with_checkpoint(
        mut self,
        checkpoint: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        self.checkpoint = Some(Arc::new(checkpoint));
        self
    }

    /// Serializes final publication against lock invalidation. Never use this
    /// for dialogs, network I/O, transfers, or temporary-file synchronization.
    pub(crate) fn publish<T>(
        &self,
        publish: impl FnOnce() -> Result<T, AgentError>,
    ) -> Result<T, AgentError> {
        self.checkpoint("before-publication")?;
        let _publication = self
            .lock
            .publication
            .lock()
            .map_err(|_| publication_unavailable())?;
        self.check()?;
        publish()
    }
}

pub(crate) fn unlocked_interval(app: &AppHandle) -> Result<UnlockedInterval, AgentError> {
    let lock = app
        .try_state::<Arc<AppLock>>()
        .ok_or_else(publication_unavailable)?;
    UnlockedInterval::capture(Arc::clone(&lock))
}

#[cfg(test)]
impl AppLock {
    pub(crate) fn unlocked_for_test() -> Arc<Self> {
        let lock = Arc::new(Self::for_capability(Capability {
            available: true,
            reason: None,
            mechanism: "test",
        }));
        lock.locked.store(false, Ordering::Release);
        lock
    }
    pub(crate) fn interval_for_test(self: &Arc<Self>) -> UnlockedInterval {
        UnlockedInterval::capture(self.clone()).unwrap()
    }
    pub(crate) fn invalidate_for_test(&self, unlock: bool) {
        self.lock().unwrap();
        if unlock {
            self.locked.store(false, Ordering::Release);
        }
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

/// Bind plaintext-producing work to one unlocked interval, including lock/unlock races.
pub fn unlocked_generation(app: &AppHandle) -> Result<u64, AgentError> {
    let lock = app.try_state::<Arc<AppLock>>().ok_or_else(|| {
        AgentError::new(
            "app-lock-unavailable",
            "Unable to verify application lock state.",
            false,
        )
    })?;
    let generation = lock.generation.load(Ordering::Acquire);
    require_unlocked(app)?;
    Ok(generation)
}
pub fn require_unlocked_generation(app: &AppHandle, generation: u64) -> Result<(), AgentError> {
    if unlocked_generation(app)? != generation {
        return Err(AgentError::new(
            "chat-interrupted",
            "Chat was closed when the app locked.",
            false,
        ));
    }
    Ok(())
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
            "Unable to verify application lock state.",
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
    webview
        .app_handle()
        .state::<crate::commands::AppState>()
        .invalidate_catalog();
    crate::commands::web_admin::close_all(webview.app_handle());
    crate::commands::chat_local::conceal(webview.app_handle());
    Ok(lock.state())
}

#[tauri::command]
pub async fn unlock_app(
    webview: tauri::Webview,
    lock: State<'_, Arc<AppLock>>,
) -> Result<LockStateDto, AgentError> {
    crate::commands::require_main_window(&webview)?;
    let Some(generation) = lock.begin_unlock()? else {
        return Ok(lock.state());
    };
    let owned = Arc::clone(&lock);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = AuthenticationGuard(&owned.authenticating);
        match authenticate("Unlock FOKS to access your vault") {
            Ok(true) => owned.complete_unlock(generation)?,
            Ok(false) => {}
            Err(message) => {
                return Err(AgentError::new("authentication-failed", message, false));
            }
        }
        Ok(owned.state())
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Unlock prompt failed to complete: {error}")))?
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
                "macOS authentication unavailable ({}).",
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
        reason: Some("App lock is not supported on this platform.".to_owned()),
        mechanism: "none",
    }
}

#[cfg(not(any(target_os = "macos", all(unix, not(target_os = "macos")))))]
fn authenticate(_reason: &str) -> Result<bool, String> {
    Err("App lock authentication is not supported on this platform.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locking_invalidates_work_even_after_unlock() {
        let lock = AppLock::for_capability(Capability {
            available: true,
            reason: None,
            mechanism: "password",
        });
        lock.locked.store(false, Ordering::Release);
        let before = lock.generation.load(Ordering::Acquire);
        assert!(lock.permits_generation(before));
        lock.lock().unwrap();
        assert!(!lock.permits_generation(before));
        lock.locked.store(false, Ordering::Release);
        assert!(!lock.permits_generation(before));
        assert_ne!(before, lock.generation.load(Ordering::Acquire));
    }

    #[test]
    fn publication_completes_before_lock_or_is_rejected_after_lock() {
        let lock = AppLock::unlocked_for_test();
        let interval = lock.interval_for_test();
        let (entered, entered_rx) = std::sync::mpsc::channel();
        let (finish, finish_rx) = std::sync::mpsc::channel();
        let worker_interval = interval.clone();
        let publish = std::thread::spawn(move || {
            worker_interval.publish(|| {
                entered.send(()).unwrap();
                finish_rx.recv().unwrap();
                Ok(())
            })
        });
        entered_rx.recv().unwrap();
        // This is the same mutex lock() takes. Its unavailability proves the
        // effect cannot overlap completed invalidation, without a sleep/race.
        assert!(lock.publication.try_lock().is_err());
        assert!(interval.check().is_ok());
        let invalidate = lock.clone();
        let locking = std::thread::spawn(move || invalidate.lock());
        finish.send(()).unwrap();
        publish.join().unwrap().unwrap();
        locking.join().unwrap().unwrap();
        assert!(interval
            .publish::<()>(|| panic!("old interval published after lock completion"))
            .is_err());
    }

    #[test]
    fn a_later_lock_invalidates_an_already_open_authentication_prompt() {
        let lock = AppLock::unlocked_for_test();
        let old = lock.interval_for_test();
        lock.lock().unwrap();
        let generation = lock.begin_unlock().unwrap().unwrap();
        let reached = Arc::new(std::sync::Barrier::new(2));
        let resume = Arc::new(std::sync::Barrier::new(2));
        let worker_lock = lock.clone();
        let worker_reached = reached.clone();
        let worker_resume = resume.clone();
        let authentication = std::thread::spawn(move || {
            let _guard = AuthenticationGuard(&worker_lock.authenticating);
            worker_reached.wait();
            worker_resume.wait();
            worker_lock.complete_unlock(generation)
        });
        reached.wait();
        lock.lock().unwrap();
        resume.wait();
        assert_eq!(
            authentication.join().unwrap().unwrap_err().code,
            "authentication-interrupted"
        );
        assert!(lock.state().locked);
        assert!(!lock.authenticating.load(Ordering::Acquire));
        assert!(old.check().is_err());
        let next = lock.begin_unlock().unwrap().unwrap();
        let _guard = AuthenticationGuard(&lock.authenticating);
        lock.complete_unlock(next).unwrap();
        assert!(!lock.state().locked);
        assert!(old.check().is_err());
        assert!(lock.interval_for_test().check().is_ok());
    }

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
