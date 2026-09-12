//! Desktop IPC domains. Tauri registration lives in `crate::run`.
//! Shared context and validation preserve the same authorization boundary across domains.

pub(crate) mod accounts;
pub(crate) mod application;
pub(crate) mod chat;
pub(crate) mod chat_local;
mod context;
pub(crate) mod enrollment;
mod execution;
pub(crate) mod groups;
pub(crate) mod servers;
mod types;
mod validation;
pub(crate) mod vault;
pub(crate) mod yubikey;

pub use context::{AppState, MAIN};
pub use validation::require_main_window;

#[cfg(test)]
mod tests;
