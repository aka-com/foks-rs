//! Platform effects are separate from synchronization and notification policy.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
#[cfg(all(unix, not(target_os = "macos")))]
pub use linux::*;
#[cfg(not(unix))]
pub const BACKEND: &str = "none";
#[cfg(not(unix))]
pub const GROUPED: bool = false;
#[cfg(not(unix))]
pub fn available() -> bool {
    false
}
#[cfg(not(unix))]
pub fn service() -> Option<String> {
    None
}
#[cfg(not(unix))]
pub async fn refresh() {}
#[cfg(not(unix))]
pub fn install(_: &tauri::AppHandle) {}
#[cfg(not(unix))]
pub async fn permission() -> bool {
    false
}
#[cfg(not(unix))]
pub fn refusal() -> crate::agent::AgentError {
    super::error("Desktop notifications are unavailable on this platform.")
}
#[cfg(not(unix))]
pub fn display(_: &tauri::AppHandle, _: &super::Alert) -> Result<(), crate::agent::AgentError> {
    Err(super::error(
        "Desktop notifications are unavailable on this platform.",
    ))
}
#[cfg(not(unix))]
pub fn clear() {}
#[cfg(not(unix))]
pub fn open_settings() -> Result<(), crate::agent::AgentError> {
    Err(super::error(
        "Notification settings are unavailable on this platform.",
    ))
}
