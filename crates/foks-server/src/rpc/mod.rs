//! Bounded RPC routing metadata and, in later phases, handler composition.

mod router;
mod routes;

pub use router::{route_call, Listener, RouteError, RoutedCall};
pub use routes::{route, RouteSpec, ServiceSpec, ROUTES, SERVICES};
