//! Platform effects are separate from synchronization and notification policy.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(not(target_os = "macos"))]
pub fn available() -> bool {
    false
}
#[cfg(not(target_os = "macos"))]
pub fn install(_: &tauri::AppHandle) {}
#[cfg(not(target_os = "macos"))]
pub async fn permission() -> bool {
    false
}
#[cfg(not(target_os = "macos"))]
pub fn display(_: &tauri::AppHandle, _: &str, _: &str) -> Result<(), crate::agent::AgentError> {
    Err(super::error(
        "Desktop notifications are unavailable on this platform.",
    ))
}
#[cfg(not(target_os = "macos"))]
pub fn clear() {}
#[cfg(not(target_os = "macos"))]
pub fn open_settings() -> Result<(), crate::agent::AgentError> {
    Err(super::error(
        "Notification settings are unavailable on this platform.",
    ))
}
