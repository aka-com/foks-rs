use foks_rpc::{encode_bare_success_response_at, encode_bare_void_success_response_at, RpcStatus};

use crate::rpc::{RouteId, RoutedCall};

use super::super::ServerData;

pub(super) fn response(data: &ServerData, call: RoutedCall) -> Result<Vec<u8>, RpcStatus> {
    let sequence = call.call.sequence();
    match call.route.id {
        RouteId::KexSend => {
            data.kex_relay.send(call.call.argument())?;
            encode_bare_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::KexReceive => {
            let message = data.kex_relay.receive(call.call.argument())?;
            encode_bare_success_response_at(&message.encoded().map_err(bad_arguments)?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        _ => Err(RpcStatus::Unsupported),
    }
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}
