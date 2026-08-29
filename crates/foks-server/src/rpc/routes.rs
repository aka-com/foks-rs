#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceSpec {
    pub name: &'static str,
    pub service_type: u64,
    pub listener: &'static str,
    pub supported: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteSpec {
    pub id: RouteId,
    pub protocol: &'static str,
    pub protocol_id: u64,
    pub method: &'static str,
    pub position: u64,
    pub listeners: &'static [&'static str],
    pub authentication: &'static str,
    pub request: &'static str,
    pub result: &'static str,
    pub statuses: &'static [&'static str],
    pub max_request_bytes: usize,
    pub supported: bool,
    pub coverage: &'static [&'static str],
}

#[path = "generated/routes.rs"]
mod generated;

pub use generated::{RouteId, ROUTES, SERVICES};

/// Resolves a wire dispatch key without allocating or accepting an unknown method.
pub fn route(protocol_id: u64, position: u64) -> Option<&'static RouteSpec> {
    ROUTES
        .iter()
        .find(|route| route.protocol_id == protocol_id && route.position == position)
}
