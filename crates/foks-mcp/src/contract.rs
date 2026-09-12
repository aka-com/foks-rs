//! The Go-compatible tool surface. Recovery tools are separate local additions.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use zeroize::{Zeroize as _, Zeroizing};

pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const PAGE_ROWS: u32 = 1_000;
pub const ACTIVE_CALLS: usize = 4;
pub const QUEUED_CALLS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolSet {
    Kv,
    Team,
}

/// Never derives Debug: the put content can contain credentials.
#[derive(Deserialize, Serialize)]
#[serde(tag = "tool", content = "arguments")]
pub enum Invocation {
    #[serde(rename = "list")]
    List(PathArgs),
    #[serde(rename = "get")]
    Get(GetArgs),
    #[serde(rename = "stat")]
    Stat(PathArgs),
    #[serde(rename = "usage")]
    Usage(TeamArgs),
    #[serde(rename = "put")]
    Put(PutArgs),
    #[serde(rename = "mkdir")]
    Mkdir(MkdirArgs),
    #[serde(rename = "rm")]
    Remove(RemoveArgs),
    #[serde(rename = "mv")]
    Move(MoveArgs),
    #[serde(rename = "list-memberships")]
    Memberships(EmptyArgs),
    #[serde(rename = "fennec_status")]
    Status(SubmissionArgs),
    #[serde(rename = "fennec_pending")]
    Pending(EmptyArgs),
    #[serde(rename = "team-list")]
    Members(TeamListArgs),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionArgs {
    pub fennec_submission_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyArgs {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TeamListArgs {
    pub team: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TeamArgs {
    #[serde(default)]
    pub team: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PathArgs {
    pub path: String,
    #[serde(default)]
    pub team: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetArgs {
    pub path: String,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub base64: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PutArgs {
    #[serde(default)]
    pub fennec_submission_id: Option<String>,
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub base64: bool,
    #[serde(default)]
    pub mkdir_p: bool,
    #[serde(default)]
    pub overwrite: bool,
}

impl Drop for PutArgs {
    fn drop(&mut self) {
        self.content.zeroize();
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MkdirArgs {
    #[serde(default)]
    pub fennec_submission_id: Option<String>,
    pub path: String,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub mkdir_p: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveArgs {
    #[serde(default)]
    pub fennec_submission_id: Option<String>,
    pub path: String,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MoveArgs {
    #[serde(default)]
    pub fennec_submission_id: Option<String>,
    pub src: String,
    pub dst: String,
    #[serde(default)]
    pub team: Option<String>,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum ContractError {
    #[error("unknown tool or tool unavailable in read-only mode")]
    UnknownTool,
    #[error("invalid tool arguments")]
    InvalidArguments,
    #[error("result exceeds the bounded MCP limit; use the FOKS file client")]
    TooLarge,
    #[error("binary content requires base64=true")]
    Binary,
    #[error("invalid standard base64 content")]
    Base64,
}

impl ToolSet {
    pub fn names(self, read_only: bool) -> &'static [&'static str] {
        match (self, read_only) {
            (Self::Team, _) => &["list", "list-memberships"],
            (Self::Kv, true) => &["list", "get", "stat", "usage"],
            (Self::Kv, false) => &[
                "list",
                "get",
                "stat",
                "usage",
                "put",
                "mkdir",
                "rm",
                "mv",
                "fennec_status",
                "fennec_pending",
            ],
        }
    }

    pub fn parse(
        self,
        read_only: bool,
        name: &str,
        arguments: Map<String, Value>,
    ) -> Result<Invocation, ContractError> {
        if !self.names(read_only).contains(&name) {
            return Err(ContractError::UnknownTool);
        }
        let tag = if self == Self::Team && name == "list" {
            "team-list"
        } else {
            name
        };
        // Serde errors can quote input values; never return them to logs or callers.
        serde_json::from_value(json!({"tool": tag, "arguments": arguments}))
            .map_err(|_| ContractError::InvalidArguments)
    }

    pub fn tools(self, read_only: bool) -> Vec<Tool> {
        self.names(read_only).iter().map(|name| {
            let (required, strings, flags): (&[&str], &[&str], &[&str]) = match (self, *name) {
                (Self::Team, "list") => (&["team"], &["team"], &[]),
                (Self::Team, _) => (&[], &[], &[]),
                (_, "fennec_status") => (&["fennec_submission_id"], &["fennec_submission_id"], &[]),
                (_, "fennec_pending") => (&[], &[], &[]),
                (_, "get") => (&["path"], &["path", "team"], &["base64"]),
                (_, "put") => (&["path", "content"], &["path", "content", "team"], &["base64", "mkdir_p", "overwrite"]),
                (_, "mkdir") => (&["path"], &["path", "team"], &["mkdir_p"]),
                (_, "rm") => (&["path"], &["path", "team"], &["recursive"]),
                (_, "mv") => (&["src", "dst"], &["src", "dst", "team"], &[]),
                (_, "usage") => (&[], &["team"], &[]),
                _ => (&["path"], &["path", "team"], &[]),
            };
            let mut properties = Map::new();
            for key in strings { properties.insert((*key).to_owned(), json!({"type":"string"})); }
            for key in flags { properties.insert((*key).to_owned(), json!({"type":"boolean", "default":false})); }
            let writes = matches!(*name, "put" | "mkdir" | "rm" | "mv");
            if writes { properties.insert("fennec_submission_id".into(), json!({"type":"string", "pattern":"^[0-9a-f]{32}$"})); }
            serde_json::from_value(json!({
                "name": name,
                "description": format!("FOKS {} {name} in the selected account", if self == Self::Kv {"KV"} else {"team"}),
                "inputSchema": {"type":"object", "properties":properties, "required":required, "additionalProperties":false},
                "annotations": {"readOnlyHint":!writes, "destructiveHint":writes, "idempotentHint":false, "openWorldHint":false}
            })).expect("static MCP tool schema")
        }).collect()
    }
}

pub fn normalize_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

pub fn decode_content(content: &str, base64: bool) -> Result<Zeroizing<Vec<u8>>, ContractError> {
    let limit = if base64 {
        MAX_FILE_BYTES.div_ceil(3) * 4
    } else {
        MAX_FILE_BYTES
    };
    if content.len() > limit {
        return Err(ContractError::TooLarge);
    }
    let bytes = Zeroizing::new(if base64 {
        STANDARD
            .decode(content)
            .map_err(|_| ContractError::Base64)?
    } else {
        content.as_bytes().to_vec()
    });
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ContractError::TooLarge);
    }
    Ok(bytes)
}

pub fn encode_content(bytes: &[u8], base64: bool) -> Result<String, ContractError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ContractError::TooLarge);
    }
    if base64 {
        return Ok(STANDARD.encode(bytes));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ContractError::Binary)?;
    if bytes.iter().any(|b| *b < 0x09 || (*b > 0x0d && *b < 0x20)) {
        return Err(ContractError::Binary);
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn tools_are_isolated_and_read_only_is_enforced_on_dispatch() {
        assert_eq!(ToolSet::Kv.tools(false).len(), 10);
        assert_eq!(ToolSet::Team.tools(false).len(), 2);
        assert!(matches!(
            ToolSet::Kv.parse(true, "put", args(json!({"path":"x","content":"secret"}))),
            Err(ContractError::UnknownTool)
        ));
        assert!(ToolSet::Kv
            .parse(
                false,
                "mv",
                args(json!({"src":"a","dst":"b","overwrite":true}))
            )
            .is_err());
        assert!(matches!(
            ToolSet::Team.parse(false, "list", args(json!({"team":"t"}))),
            Ok(Invocation::Members(_))
        ));
        assert!(ToolSet::Team
            .parse(false, "list-memberships", args(json!({"account":"other"})))
            .is_err());
    }

    #[test]
    fn omitted_write_flags_do_not_authorize_overwrite_or_recursive_operations() {
        let Invocation::Put(p) = ToolSet::Kv
            .parse(false, "put", args(json!({"path":"x","content":"y"})))
            .unwrap()
        else {
            panic!()
        };
        assert!(!p.overwrite && !p.mkdir_p && !p.base64);
        assert!(ToolSet::Kv
            .parse(
                false,
                "put",
                args(json!({"path":"x","content":"sensitive","overwrite":"yes"}))
            )
            .is_err());
    }

    #[test]
    fn binary_and_path_rules_match_go_without_touching_host_paths() {
        assert_eq!(normalize_path(""), "/");
        assert_eq!(normalize_path("a/../b"), "/a/../b");
        assert_eq!(encode_content(b"", false).unwrap(), "");
        assert_eq!(encode_content(b"hello\n", false).unwrap(), "hello\n");
        for bytes in [b"a\0".as_slice(), &[255], &[14]] {
            assert_eq!(encode_content(bytes, false), Err(ContractError::Binary));
        }
        let b = [0, 255, 1, 2];
        assert_eq!(encode_content(&b, true).unwrap(), "AP8BAg==");
        assert_eq!(decode_content("AP8BAg==", true).unwrap().as_slice(), b);
        assert!(decode_content("base64:AP8BAg==", true).is_err());
        assert_eq!(
            decode_content(&"x".repeat(MAX_FILE_BYTES + 1), false).unwrap_err(),
            ContractError::TooLarge
        );
    }
}
