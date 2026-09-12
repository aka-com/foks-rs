//! Standalone host administration. Disabled unless its full service is configured.
mod clock;
pub mod operator;
pub use clock::{AdminClock, AdminElapsedClock, SuspendClock};
mod config;
mod service;
pub use config::WebAdminConfig;
pub use service::WebAdminService;
mod http;
mod render;
pub use http::WebAdminHttpServer;
