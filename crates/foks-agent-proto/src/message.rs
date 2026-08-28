use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    pub operation: Operation,
}

impl Request {
    pub fn new(id: u64, operation: Operation) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            operation,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "kebab-case")]
pub enum Operation {
    Ping,
    ListProfiles,
    Probe { profile: String },
    ListAccounts { profile: String },
    SyncAccount { profile: String, alias: String },
    ListKv { profile: String, alias: String },
    ListTeams { profile: String },
    SyncTeam { profile: String, team_alias: String },
    RunDueJobs { profile: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    #[serde(flatten)]
    pub result: ResponseResult,
}

impl Response {
    pub fn success(id: u64, value: serde_json::Value) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            result: ResponseResult::Success { value },
        }
    }

    pub fn error(id: u64, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            result: ResponseResult::Error {
                code,
                message: message.into(),
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ResponseResult {
    Success { value: serde_json::Value },
    Error { code: ErrorCode, message: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    InvalidRequest,
    VersionMismatch,
    Busy,
    DeadlineExceeded,
    OperationFailed,
}
