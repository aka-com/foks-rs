//! Private spike ABI. Raw JSON command bytes are passed into Rust without JS
//! parsing (all u64 values remain exact). SQL values encode i64 as strings.
use crate::{
    normalize_creation_times, put_dirents, Error, Limits, Mutation, Result, Sql, Statement, Value,
    MAX_INPUT_BYTES,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    PutDirents {
        uid: Vec<u8>,
        precondition: Option<Vec<u8>>,
        role: Vec<u8>,
        mutations: Vec<Mutation>,
        limits: Limits,
        normalize_at: Option<u64>,
    },
    ProbeTypes {
        integer: Value,
        blob: Vec<u8>,
        text: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub values: Vec<Value>,
}

pub fn run(sql: &mut impl Sql, bytes: &[u8]) -> Result<Reply> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(Error::Bounds);
    }
    let command: Command = serde_json::from_slice(bytes).map_err(|_| Error::Invalid("command"))?;
    match command {
        Command::PutDirents {
            uid,
            precondition,
            role,
            mut mutations,
            limits,
            normalize_at,
        } => {
            let role = foks_proto::Role::decode(&role).map_err(|_| Error::Invalid("role"))?;
            let precondition = precondition
                .as_deref()
                .map(foks_proto::KvPathVersionVector::decode)
                .transpose()
                .map_err(|_| Error::Invalid("precondition"))?;
            if let Some(now) = normalize_at {
                normalize_creation_times(sql, &uid, &mut mutations, now)?;
            }
            put_dirents(sql, &uid, precondition.as_ref(), role, &mutations, &limits)?;
            // Trigger-produced usage is read in the same transaction and is
            // returned only if the host subsequently succeeds at COMMIT.
            let values = sql
                .query(Statement::Usage, &[Value::Blob(uid)])?
                .ok_or(Error::Storage)?;
            Ok(Reply { values })
        }
        Command::ProbeTypes {
            integer,
            blob,
            text,
        } => {
            let values = sql
                .query(
                    Statement::ProbeTypes,
                    &[integer, Value::Blob(blob), Value::Null, Value::Text(text)],
                )?
                .ok_or(Error::Storage)?;
            Ok(Reply { values })
        }
    }
}
