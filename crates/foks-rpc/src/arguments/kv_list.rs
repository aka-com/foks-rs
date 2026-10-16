//! Post-authority List fields, with the native server's existing error ordering.
use crate::{KvListCursor, RpcStatus};
use foks_snowpack::{decode, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KvListArguments {
    pub parent: [u8; 16],
    pub cursor: KvListCursor,
    pub number: u64,
    pub load_small: bool,
}

/// Decode only List fields after the caller has resolved current KV authority.
/// The first field is deliberately opaque here: success does not authenticate it.
/// Time remains u64 so parent permission can precede database range conversion.
pub fn decode_kv_list_arguments(argument: &[u8]) -> Result<KvListArguments, RpcStatus> {
    let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments("KV argument is not a struct"));
    };
    if fields.len() != 3 {
        return Err(bad_arguments("KV argument has the wrong shape"));
    }
    let Value::Binary(parent) = &fields[1] else {
        return Err(bad_arguments("KV directory ID is not binary"));
    };
    let parent = parent
        .as_slice()
        .try_into()
        .map_err(|_| bad_arguments("KV directory ID has the wrong width"))?;
    let Value::Array(pagination) = &fields[2] else {
        return Err(bad_arguments("KV pagination is not a struct"));
    };
    let [cursor, Value::Unsigned(number), Value::Bool(load_small)] = pagination.as_slice() else {
        return Err(bad_arguments("KV pagination has the wrong shape"));
    };
    const GO_DEFAULT_PAGE_ENTRIES: u64 = 4096;
    let number = if *number == 0 || *number > GO_DEFAULT_PAGE_ENTRIES {
        GO_DEFAULT_PAGE_ENTRIES
    } else {
        *number
    };
    Ok(KvListArguments {
        parent,
        cursor: list_cursor(cursor)?,
        number,
        load_small: *load_small,
    })
}

fn list_cursor(value: &Value) -> Result<KvListCursor, RpcStatus> {
    let Value::Array(fields) = value else {
        return Err(bad_arguments("KV list cursor is not a struct"));
    };
    match fields.as_slice() {
        [Value::Unsigned(0), Value::Variant(None)] => Ok(KvListCursor::None),
        [Value::Unsigned(1), Value::Variant(Some((tag, value)))] if tag == b"1" => {
            let Value::Binary(bytes) = value.as_ref() else {
                return Err(bad_arguments("KV list MAC cursor is not binary"));
            };
            bytes
                .as_slice()
                .try_into()
                .map(KvListCursor::Mac)
                .map_err(|_| bad_arguments("KV list MAC cursor has the wrong width"))
        }
        [Value::Unsigned(2), Value::Variant(Some((tag, value)))] if tag == b"2" => {
            let Value::Unsigned(time) = value.as_ref() else {
                return Err(bad_arguments("KV list time cursor is not unsigned"));
            };
            Ok(KvListCursor::Time(*time))
        }
        _ => Err(bad_arguments("unsupported KV list cursor")),
    }
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

#[cfg(test)]
mod tests;
