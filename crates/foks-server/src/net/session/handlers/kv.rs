use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};

use crate::auth::Principal;
use crate::rpc::{RouteId, RoutedCall};

use super::super::{permission_denied, ServerData};

pub(super) trait Operations {
    fn dispatch(
        &self,
        route: RouteId,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<crate::services::kv::Response, RpcStatus>;
}

impl Operations for ServerData {
    fn dispatch(
        &self,
        route: RouteId,
        argument: &[u8],
        principal: &Principal,
    ) -> Result<crate::services::kv::Response, RpcStatus> {
        principal.require_ordinary_device()?;
        let database = self.read_database()?;
        crate::services::kv::dispatch(
            route,
            argument,
            principal,
            &database,
            self.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
            &self.clock,
        )
    }
}

pub(super) fn response(
    operations: &impl Operations,
    call: RoutedCall,
    principal: Option<&Principal>,
) -> Result<Vec<u8>, RpcStatus> {
    let principal = principal.ok_or_else(permission_denied)?;
    let sequence = call.call.sequence();
    match operations.dispatch(call.route.id, call.call.argument(), principal)? {
        crate::services::kv::Response::Data(response) => {
            encode_success_response_at(&response, sequence).map_err(|_| RpcStatus::Unsupported)
        }
        crate::services::kv::Response::Void => {
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
    }
}
