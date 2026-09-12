use super::{RouteId, RoutedCall, RpcStatus, ServerData};
use crate::auth::Principal;
pub(super) fn response(
    data: &ServerData,
    call: RoutedCall,
    principal: Option<&Principal>,
) -> Result<Vec<u8>, RpcStatus> {
    let service = data.web_admin.as_ref().ok_or(RpcStatus::Unsupported)?;
    service
        .require_ready()
        .map_err(|_| RpcStatus::Unsupported)?;
    let principal = principal
        .ok_or_else(|| RpcStatus::PermissionDenied("native authentication required".into()))?;
    principal.require_interactive_device()?;
    let uid = principal
        .uid()
        .try_into()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    match call.route.id {
        RouteId::UserNewWebAdminPanelURL => {
            foks_rpc::arguments::decode_void(call.call.argument())
                .map_err(super::super::bad_arguments)?;
            let credential = foks_server_db::WebCredential {
                host: data
                    .host_id
                    .as_slice()
                    .try_into()
                    .map_err(|_| RpcStatus::TransactionRetry)?,
                uid,
                credential: principal
                    .device_id()
                    .try_into()
                    .map_err(|_| RpcStatus::TransactionRetry)?,
                certificate_expires_at_us: principal.certificate_expires_at(),
            };
            let url = service.ticket(credential).map_err(status)?;
            let result =
                foks_snowpack::encode(&foks_snowpack::Value::Text(url.as_bytes().to_vec()))
                    .map_err(|_| RpcStatus::TransactionRetry)?;
            foks_rpc::encode_success_response_at(&result, call.call.sequence())
                .map_err(|_| RpcStatus::TransactionRetry)
        }
        RouteId::UserCheckURL => {
            let foks_snowpack::Value::Array(values) =
                foks_snowpack::decode(call.call.argument()).map_err(super::super::bad_arguments)?
            else {
                return Err(super::super::bad_arguments("invalid admin check"));
            };
            let [foks_snowpack::Value::Text(url)] = values.as_slice() else {
                return Err(super::super::bad_arguments("invalid admin check"));
            };
            let url = std::str::from_utf8(url).map_err(super::super::bad_arguments)?;
            service.check(url, &uid).map_err(status)?;
            foks_rpc::encode_void_success_response_at(call.call.sequence())
                .map_err(|_| RpcStatus::TransactionRetry)
        }
        _ => Err(RpcStatus::Unsupported),
    }
}
fn status(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::Database(foks_server_db::Error::WebWrongUser) => RpcStatus::WrongUser,
        crate::Error::Database(foks_server_db::Error::ReceiptExpired) => RpcStatus::Expired,
        crate::Error::Database(foks_server_db::Error::AuthorizationChanged) => {
            RpcStatus::PermissionDenied("administration access denied".into())
        }
        crate::Error::Config(_) | crate::Error::Protocol(_) => {
            RpcStatus::BadArguments("invalid administration request".into())
        }
        crate::Error::Database(foks_server_db::Error::Capacity(_))
        | crate::Error::WriterQueue
        | crate::Error::ReaderPool => RpcStatus::RateLimited,
        _ => RpcStatus::TransactionRetry,
    }
}
