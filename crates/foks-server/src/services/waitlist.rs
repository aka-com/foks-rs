use std::sync::Arc;

use foks_rpc::RpcStatus;
use foks_snowpack::Value;

use crate::{Entropy, WriterHandle};

const WAITLIST_ID_BYTES: usize = 13;
const WAITLIST_ID_TYPE: u8 = 1;

pub(crate) fn join_waitlist(
    argument: &[u8],
    writer: &WriterHandle,
    clock: Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    let email = foks_rpc::arguments::decode_join_waitlist(argument)
        .map_err(|error| RpcStatus::BadArguments(error.to_string()))?;
    let email =
        String::from_utf8(email).map_err(|error| RpcStatus::BadArguments(error.to_string()))?;
    let mut id = [0_u8; WAITLIST_ID_BYTES];
    entropy.fill(&mut id).map_err(map_write_error)?;
    id[0] = WAITLIST_ID_TYPE;
    writer
        .call_with_current_time(clock, move |database, now| {
            database.join_waitlist(&id, &email, now)?;
            Ok(())
        })
        .map_err(map_identifier_write_error)?;
    foks_snowpack::encode(&Value::Binary(id.to_vec())).map_err(map_write_error)
}

fn map_write_error(error: impl Into<crate::Error>) -> RpcStatus {
    match error.into() {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::Invalid(message)) => {
            RpcStatus::BadArguments(message.to_owned())
        }
        crate::Error::Database(foks_server_db::Error::Duplicate(message)) => {
            RpcStatus::Duplicate(message.to_owned())
        }
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
