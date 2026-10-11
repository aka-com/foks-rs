//! Immutable context for read adapters. These values are bindings, not authority.
pub use foks_proto::SubmissionHandle;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize as _;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataScope {
    pub profile: String,
    pub account_alias: String,
    pub host_id: String,
    pub user_id: String,
    pub team_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "read", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DataRead {
    ResolveTeam {
        selector: String,
    },
    Catalog,
    Stat {
        path: String,
        version: Option<u64>,
    },
    Entry {
        path: String,
        version: u64,
    },
    Chunk {
        path: String,
        version: u64,
        offset: u64,
        length: u32,
    },
    Usage,
    Members,
    Memberships,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataUsage {
    pub files: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataCatalogEntry {
    pub path: String,
    pub node_type: String,
    pub node_id: String,
    pub version: u64,
    pub size: Option<u64>,
    pub modified_microseconds: u64,
    pub read_role: super::KvRole,
    pub write_role: super::KvRole,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataCatalog {
    pub user_chain_sequence: u64,
    pub team_chain_sequence: Option<u64>,
    pub root_id: Option<String>,
    pub snapshot_version: u64,
    pub entries: Vec<DataCatalogEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataMember {
    pub name: String,
    pub host: String,
    pub source_role: String,
    pub destination_role: String,
    pub added_millis: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataMembership {
    pub team: String,
    pub team_id: String,
    pub source_role: String,
    pub destination_role: String,
    pub via: Option<String>,
    pub index_range: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataMemberships {
    pub complete: bool,
    pub teams: Vec<DataMembership>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataEntry {
    pub path: String,
    pub version: u64,
    pub node_type: String,
    pub size: Option<u64>,
    pub read_role: super::KvRole,
    pub write_role: super::KvRole,
    #[serde(with = "crate::base64_bytes::optional")]
    pub content: Option<Vec<u8>>,
    pub symlink_target: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataChunk {
    pub path: String,
    pub version: u64,
    pub offset: u64,
    #[serde(with = "crate::base64_bytes")]
    pub content: Vec<u8>,
    pub eof: bool,
}

impl Drop for DataEntry {
    fn drop(&mut self) {
        if let Some(content) = &mut self.content {
            content.zeroize();
        }
        if let Some(target) = &mut self.symlink_target {
            target.zeroize();
        }
    }
}

impl Drop for DataChunk {
    fn drop(&mut self) {
        self.content.zeroize();
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataWriteKind {
    Put,
    Mkdir,
    Remove,
    Move,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataWriteSpec {
    pub kind: DataWriteKind,
    pub path: String,
    pub destination: Option<String>,
    pub team_selector: Option<String>,
    pub overwrite: bool,
    pub mkdir_p: bool,
    pub recursive: bool,
    pub body_length: u64,
    pub body_hash: [u8; 32],
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataWriteStatus {
    Prepared,
    Committed,
    Rejected,
    SubmissionUnknown,
    Expired,
    NotRecorded,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataWriteOutcome {
    pub submission_id: String,
    pub status: DataWriteStatus,
    /// True when ancillary namespace steps committed but completion is unproven.
    pub partial: bool,
    pub node_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataSubmission {
    pub scope: DataScope,
    pub submission_id: String,
}

#[derive(Deserialize, Serialize)]
pub struct DataStat {
    pub path: String,
    pub version: Option<u64>,
    #[serde(with = "crate::base64_bytes::optional")]
    pub dirent: Option<Vec<u8>>,
    #[serde(with = "crate::base64_bytes::optional")]
    pub directory: Option<Vec<u8>>,
    #[serde(with = "crate::base64_bytes::optional")]
    pub node: Option<Vec<u8>>,
    pub size: Option<u64>,
    pub target: Option<String>,
}

/// Whether the adapter clock admits new assistant data writes for one account.
#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterClockAdmission {
    /// No admission has anchored the clock yet; the first one will.
    Unanchored,
    Trusted,
    /// New writes and pruning fail with `clock-untrusted` until a repair
    /// re-anchors the clock at a confirmed time.
    Paused,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterClockStatus {
    pub admission: AdapterClockAdmission,
    /// The computer's wall time when the agent checked, in Unix seconds.
    pub local_time: u64,
    /// The latest wall time admission accepted; absent until anchored.
    pub validated_time_floor: Option<u64>,
    /// Handles issued before this time can never execute.
    pub reject_issued_before: Option<u64>,
}

/// A two-step repair: the preview returns a digest of the displayed clock
/// state and the proposed time, and only that digest confirms.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "step", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AdapterClockRepairAction {
    Preview {
        unix_seconds: u64,
    },
    Confirm {
        unix_seconds: u64,
        confirmation_digest: String,
    },
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterClockPreview {
    pub host_id: String,
    pub user_id: String,
    pub anchor_wall: u64,
    pub validated_time_floor: u64,
    pub reject_issued_before: u64,
    pub proposed_wall: u64,
    pub confirmation_digest: String,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterClockRepair {
    pub preview: AdapterClockPreview,
    pub resulting_floor: u64,
    pub checkpoint_digest: String,
}
