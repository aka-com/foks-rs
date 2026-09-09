//! Bounded RPC routing metadata and handler composition.

mod router;
mod routes;

pub use router::{route_call, Listener, RouteError, RoutedCall};
pub use routes::{route, RouteId, RouteSpec, ServiceSpec, ROUTES, SERVICES};
