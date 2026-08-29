use foks_rpc::DecodedCall;
use thiserror::Error;

use super::{route, RouteSpec};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Listener {
    Probe,
    PublicServices,
    Authenticated,
}

impl Listener {
    fn contract_name(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::PublicServices => "public_services",
            Self::Authenticated => "authenticated",
        }
    }
}

#[derive(Debug, Error)]
pub enum RouteError {
    #[error("unknown FOKS RPC protocol {protocol_id:#010x} method {position}")]
    Unknown { protocol_id: u64, position: u64 },
    #[error("RPC method is not available on this listener")]
    WrongListener,
    #[error("RPC argument is {received} bytes, limit is {maximum}")]
    RequestTooLarge { received: usize, maximum: usize },
}

#[derive(Debug)]
pub struct RoutedCall {
    pub route: &'static RouteSpec,
    pub call: DecodedCall,
}

/// Applies the registered dispatch key, listener boundary, and per-method limit.
pub fn route_call(call: DecodedCall, listener: Listener) -> Result<RoutedCall, RouteError> {
    let route = route(call.protocol_id(), call.method_position()).ok_or(RouteError::Unknown {
        protocol_id: call.protocol_id(),
        position: call.method_position(),
    })?;
    if !route.listeners.contains(&listener.contract_name()) {
        return Err(RouteError::WrongListener);
    }
    if call.argument().len() > route.max_request_bytes {
        return Err(RouteError::RequestTooLarge {
            received: call.argument().len(),
            maximum: route.max_request_bytes,
        });
    }
    Ok(RoutedCall { route, call })
}
