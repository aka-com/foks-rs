//! Certificate-independent identity extension on the pinned server-authenticated transport.
use super::{RouteId, RoutedCall, RpcStatus, ServerData};
use foks_proto::{IdentityClaim, IdentityProof};

pub(super) fn response(data: &ServerData, call: RoutedCall) -> Result<Vec<u8>, RpcStatus> {
    let host: [u8; 33] = data
        .host()?
        .as_bytes()
        .try_into()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let writer = data.writer.as_ref().ok_or(RpcStatus::Unsupported)?;
    let result = match call.route.id {
        RouteId::IdentityFennecCapabilities => {
            foks_rpc::arguments::decode_void(call.call.argument())
                .map_err(super::super::bad_arguments)?;
            foks_snowpack::encode(&foks_snowpack::Value::Unsigned(
                foks_proto::IDENTITY_EXTENSION_VERSION,
            ))
            .map_err(|_| RpcStatus::TransactionRetry)?
        }
        RouteId::IdentityFennecChallenge => {
            let claim = IdentityClaim::decode(call.call.argument()).map_err(|_| denied())?;
            if claim.host.as_bytes() != host {
                return Err(denied());
            }
            let peer = data.peer_ip.ok_or(RpcStatus::Unsupported)?.to_string();
            let admission = foks_crypto::prefixed_hash(0xf04b_a81e_c057_0003, peer.as_bytes());
            let mut challenge = [0; 32];
            data.entropy
                .fill(&mut challenge)
                .map_err(|_| RpcStatus::TransactionRetry)?;
            writer
                .call_with_current_time(data.clock.clone(), move |db, now| {
                    Ok(db.sso_issue_identity_challenge(&claim, challenge, admission, now / 1000)?)
                })
                .map_err(status)?
                .encoded()
                .map_err(|_| RpcStatus::TransactionRetry)?
        }
        RouteId::IdentityFennecProve => {
            let proof = IdentityProof::decode(call.call.argument()).map_err(|_| denied())?;
            writer
                .call_with_current_time(data.clock.clone(), move |db, now| {
                    Ok(db.sso_prove_identity(&host, &proof, now / 1000)?)
                })
                .map_err(status)?
                .encoded()
                .map_err(|_| RpcStatus::TransactionRetry)?
        }
        _ => return Err(RpcStatus::Unsupported),
    };
    foks_rpc::encode_success_response_at(&result, call.call.sequence())
        .map_err(|_| RpcStatus::TransactionRetry)
}
fn denied() -> RpcStatus {
    RpcStatus::PermissionDenied("identity proof could not be verified".into())
}
fn status(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::Database(foks_server_db::Error::Capacity(_)) | crate::Error::WriterQueue => {
            RpcStatus::RateLimited
        }
        _ => denied(),
    }
}
