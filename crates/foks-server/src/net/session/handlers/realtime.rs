use super::super::{permission_denied, ServerData};
use crate::{auth::Principal, rpc::RoutedCall, services::realtime::Response};
use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};
pub(super) fn response(
    data: &ServerData,
    call: RoutedCall,
    principal: Option<&Principal>,
) -> Result<Vec<u8>, RpcStatus> {
    let principal = principal.ok_or_else(permission_denied)?;
    let database = data.read_database()?;
    let result = data.realtime.dispatch(
        call.route.position,
        call.call.argument(),
        principal,
        &data.host_id,
        &database,
        data.writer.as_ref().ok_or(RpcStatus::Unsupported)?,
        &data.clock,
    )?;
    match result {
        Response::Void => encode_void_success_response_at(call.call.sequence()),
        Response::Data(bytes) => encode_success_response_at(&bytes, call.call.sequence()),
    }
    .map_err(|_| RpcStatus::TransactionRetry)
}
