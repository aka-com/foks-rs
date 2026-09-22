use std::sync::Arc;

use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};

use crate::auth::Principal;
use crate::rpc::{RouteId, RoutedCall};

use super::super::ServerData;

pub(super) fn response(
    data: &ServerData,
    call: RoutedCall,
    principal: Option<&Principal>,
) -> Result<Vec<u8>, RpcStatus> {
    let writer = data.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
    let sequence = call.call.sequence();
    match call.route.id {
        RouteId::LogSendLogSendInit => encode_success_response_at(
            &crate::services::log_upload::log_send_init(
                call.call.argument(),
                principal.map(Principal::uid),
                writer,
                Arc::clone(&data.clock),
                data.entropy.as_ref(),
            )?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::LogSendLogSendInitFile => {
            crate::services::log_upload::log_send_init_file(
                call.call.argument(),
                writer,
                Arc::clone(&data.clock),
            )?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::LogSendLogSendUploadBlock => {
            crate::services::log_upload::log_send_upload_block(
                call.call.argument(),
                writer,
                Arc::clone(&data.clock),
            )?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        _ => Err(RpcStatus::Unsupported),
    }
}
