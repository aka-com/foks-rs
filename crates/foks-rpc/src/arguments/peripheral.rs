use foks_snowpack::{decode, Value};

use crate::{Error, Result};

pub const LOG_SEND_ID_BYTES: usize = 17;
pub const LOG_SEND_HASH_BYTES: usize = 32;
pub const MAXIMUM_LOG_SEND_BLOCK_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogSendInitFileArgument {
    pub id: [u8; LOG_SEND_ID_BYTES],
    pub file_id: u64,
    pub filename: Vec<u8>,
    pub content_length: u64,
    pub content_hash: [u8; LOG_SEND_HASH_BYTES],
    pub block_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogSendUploadBlockArgument {
    pub id: [u8; LOG_SEND_ID_BYTES],
    pub file_id: u64,
    pub block_number: u64,
    pub block: Vec<u8>,
}

pub fn decode_join_waitlist(bytes: &[u8]) -> Result<Vec<u8>> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("join-waitlist argument wrapper"));
    };
    let [Value::Text(email)] = fields.as_slice() else {
        return Err(shape("join-waitlist argument"));
    };
    Ok(email.clone())
}

pub fn decode_log_send_init_file(bytes: &[u8]) -> Result<LogSendInitFileArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("log-send init-file argument wrapper"));
    };
    let [id, Value::Unsigned(file_id), Value::Text(filename), Value::Unsigned(content_length), content_hash, Value::Unsigned(block_count)] =
        fields.as_slice()
    else {
        return Err(shape("log-send init-file argument"));
    };
    Ok(LogSendInitFileArgument {
        id: fixed_binary(id, "log-send identifier")?,
        file_id: *file_id,
        filename: filename.clone(),
        content_length: *content_length,
        content_hash: fixed_binary(content_hash, "log-send content hash")?,
        block_count: *block_count,
    })
}

pub fn decode_log_send_upload_block(bytes: &[u8]) -> Result<LogSendUploadBlockArgument> {
    let Value::Array(fields) = decode(bytes)? else {
        return Err(shape("log-send upload argument wrapper"));
    };
    let [id, Value::Unsigned(file_id), Value::Unsigned(block_number), Value::Binary(block)] =
        fields.as_slice()
    else {
        return Err(shape("log-send upload argument"));
    };
    if block.len() > MAXIMUM_LOG_SEND_BLOCK_BYTES {
        return Err(shape("bounded log-send block"));
    }
    Ok(LogSendUploadBlockArgument {
        id: fixed_binary(id, "log-send identifier")?,
        file_id: *file_id,
        block_number: *block_number,
        block: block.clone(),
    })
}

fn fixed_binary<const N: usize>(value: &Value, expected: &'static str) -> Result<[u8; N]> {
    let Value::Binary(bytes) = value else {
        return Err(shape(expected));
    };
    bytes.as_slice().try_into().map_err(|_| shape(expected))
}

fn shape(expected: &'static str) -> Error {
    Error::Envelope {
        expected,
        found: "another Snowpack value",
    }
}

#[cfg(test)]
mod tests {
    use foks_snowpack::encode;

    use super::*;

    #[test]
    fn decodes_exact_go_positional_arguments() {
        let waitlist = encode(&Value::Array(vec![Value::Text(b"a@example.com".to_vec())])).unwrap();
        assert_eq!(decode_join_waitlist(&waitlist).unwrap(), b"a@example.com");

        let init = encode(&Value::Array(vec![
            Value::Binary(vec![48; 17]),
            Value::Unsigned(7),
            Value::Text(b"client.log".to_vec()),
            Value::Unsigned(3),
            Value::Binary(vec![9; 32]),
            Value::Unsigned(1),
        ]))
        .unwrap();
        assert_eq!(
            decode_log_send_init_file(&init).unwrap(),
            LogSendInitFileArgument {
                id: [48; 17],
                file_id: 7,
                filename: b"client.log".to_vec(),
                content_length: 3,
                content_hash: [9; 32],
                block_count: 1,
            }
        );

        let upload = encode(&Value::Array(vec![
            Value::Binary(vec![48; 17]),
            Value::Unsigned(7),
            Value::Unsigned(0),
            Value::Binary(vec![1, 2, 3]),
        ]))
        .unwrap();
        assert_eq!(
            decode_log_send_upload_block(&upload).unwrap(),
            LogSendUploadBlockArgument {
                id: [48; 17],
                file_id: 7,
                block_number: 0,
                block: vec![1, 2, 3],
            }
        );
    }

    #[test]
    fn rejects_appended_fields_and_oversized_blocks() {
        let appended = encode(&Value::Array(vec![
            Value::Text(b"a@example.com".to_vec()),
            Value::Null,
        ]))
        .unwrap();
        assert!(decode_join_waitlist(&appended).is_err());

        let upload = encode(&Value::Array(vec![
            Value::Binary(vec![48; 17]),
            Value::Unsigned(7),
            Value::Unsigned(0),
            Value::Binary(vec![0; MAXIMUM_LOG_SEND_BLOCK_BYTES + 1]),
        ]))
        .unwrap();
        assert!(decode_log_send_upload_block(&upload).is_err());
    }
}
