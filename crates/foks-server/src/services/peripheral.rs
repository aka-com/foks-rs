use std::sync::Arc;

use foks_rpc::RpcStatus;
use foks_snowpack::Value;

use crate::{Entropy, WriterHandle};

const WAITLIST_ID_BYTES: usize = 13;
const WAITLIST_ID_TYPE: u8 = 1;
const LOG_SEND_ID_BYTES: usize = 17;
const LOG_SEND_ID_TYPE: u8 = 48;

pub(crate) fn join_waitlist(
    argument: &[u8],
    writer: &WriterHandle,
    clock: Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    let email = foks_rpc::arguments::decode_join_waitlist(argument).map_err(bad_arguments)?;
    let email = String::from_utf8(email).map_err(bad_arguments)?;
    let mut id = [0_u8; WAITLIST_ID_BYTES];
    entropy.fill(&mut id).map_err(map_write_error)?;
    id[0] = WAITLIST_ID_TYPE;
    writer
        .call_with_current_time(clock, move |database, now| {
            database.join_waitlist(&id, &email, now)?;
            Ok(())
        })
        .map_err(map_identifier_write_error)?;
    encode_binary(&id)
}

pub(crate) fn log_send_init(
    argument: &[u8],
    uid: Option<&[u8]>,
    writer: &WriterHandle,
    clock: Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    foks_rpc::arguments::decode_void(argument).map_err(bad_arguments)?;
    let mut id = [0_u8; LOG_SEND_ID_BYTES];
    entropy.fill(&mut id).map_err(map_write_error)?;
    id[0] = LOG_SEND_ID_TYPE;
    let uid = uid.map(<[u8]>::to_vec);
    writer
        .call_with_current_time(clock, move |database, now| {
            database.begin_log_send(&id, uid.as_deref(), now)?;
            Ok(())
        })
        .map_err(map_identifier_write_error)?;
    encode_binary(&id)
}

pub(crate) fn log_send_init_file(
    argument: &[u8],
    writer: &WriterHandle,
    clock: Arc<dyn foks_server_db::Clock>,
) -> Result<(), RpcStatus> {
    let request =
        foks_rpc::arguments::decode_log_send_init_file(argument).map_err(bad_arguments)?;
    let filename = String::from_utf8(request.filename).map_err(bad_arguments)?;
    writer
        .call_with_current_time(clock, move |database, now| {
            database.begin_log_send_file(&foks_server_db::LogSendFileMutation {
                log_send_id: &request.id,
                file_id: request.file_id,
                filename: &filename,
                content_length: request.content_length,
                block_count: request.block_count,
                content_hash: &request.content_hash,
                now,
            })?;
            Ok(())
        })
        .map_err(map_write_error)
}

pub(crate) fn log_send_upload_block(
    argument: &[u8],
    writer: &WriterHandle,
    clock: Arc<dyn foks_server_db::Clock>,
) -> Result<(), RpcStatus> {
    let request =
        foks_rpc::arguments::decode_log_send_upload_block(argument).map_err(bad_arguments)?;
    writer
        .call_with_current_time(clock, move |database, now| {
            database.put_log_send_block(&foks_server_db::LogSendBlockMutation {
                log_send_id: &request.id,
                file_id: request.file_id,
                block_number: request.block_number,
                block: &request.block,
                now,
            })?;
            Ok(())
        })
        .map_err(map_write_error)
}

fn encode_binary(bytes: &[u8]) -> Result<Vec<u8>, RpcStatus> {
    foks_snowpack::encode(&Value::Binary(bytes.to_vec())).map_err(map_write_error)
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

fn map_write_error(error: impl Into<crate::Error>) -> RpcStatus {
    match error.into() {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::Invalid(message)) => bad_arguments(message),
        crate::Error::Database(foks_server_db::Error::Capacity(message)) => {
            bad_arguments(format_args!("{message} exceeds configured capacity"))
        }
        crate::Error::Database(foks_server_db::Error::NotFound(message)) => {
            RpcStatus::NotFound(message.to_owned())
        }
        crate::Error::Database(foks_server_db::Error::Duplicate(message)) => {
            RpcStatus::Duplicate(message.to_owned())
        }
        crate::Error::Database(error) if error.is_quota() => RpcStatus::RateLimited,
        _ => RpcStatus::TransactionRetry,
    }
}

fn map_identifier_write_error(error: crate::Error) -> RpcStatus {
    if matches!(
        error,
        crate::Error::Database(foks_server_db::Error::Duplicate(_))
    ) {
        // A collision in a freshly generated random identifier is transient;
        // it is not a duplicate operation submitted by the caller.
        RpcStatus::TransactionRetry
    } else {
        map_write_error(error)
    }
}
