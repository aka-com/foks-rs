//! Vault catalogs, version-bound item reads and writes, and file transfer.

use crate::agent::AgentError;
use crate::applock::UnlockedInterval;
use crate::commands::context::AppState;
use crate::commands::execution::{apply_kv_mutation, map_mutation_error, MutationKind};
use crate::commands::preparation::{check_mutation_access, prepare_catalog_mutation};
use crate::commands::store_key::store_id;
use crate::commands::types::{CommandAck, MutationDto, RoleDto};
use crate::commands::validation::{
    invalid_request, invalid_response, require_main_window, serialize_secret, DOWNLOAD_CHUNK_BYTES,
    MAXIMUM_CLIPBOARD_TEXT_BYTES, MAXIMUM_DOWNLOAD_BYTES, MAXIMUM_TEXT_ITEM_BYTES,
};
use foks_agent_proto::{
    KvChunkResult, KvEntryShape, KvReadResult, KvRole, KvStoreRef, KvUploadHeader, Operation,
};
use foks_desktop::{
    CatalogFailureScope, CatalogInventoryState, CatalogItem, CatalogSnapshot, CatalogStoreRef,
    CatalogStoreSummary, KvAccountMutation, KvItemRead, KvItemValue,
};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::Manager as _;
use tauri_plugin_dialog::DialogExt as _;
use zeroize::Zeroizing;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StoreDto {
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub server: String,
    pub account: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_phase: Option<String>,
    /// The agent's pinned team chain sequence, when it reported one. The
    /// renderer reuses a cached roster while this is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_seqno: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ItemDto {
    #[serde(rename = "direntId")]
    pub dirent_id: String,
    pub store: String,
    pub path: String,
    pub kind: &'static str,
    pub size: Option<u64>,
    pub version: u64,
    pub read: RoleDto,
    pub write: RoleDto,
    /// `login` or `other` for a small file the agent's listing classified;
    /// absent for other node types. It carries no field value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFailureDto {
    pub scope: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    pub error: AgentError,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogInventoryDto {
    pub profile: String,
    pub accounts_complete: bool,
    pub teams_complete: bool,
}

impl From<&CatalogInventoryState> for CatalogInventoryDto {
    fn from(state: &CatalogInventoryState) -> Self {
        Self {
            profile: state.profile.clone(),
            accounts_complete: state.accounts_complete,
            teams_complete: state.teams_complete,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogStoreReadDto {
    pub store: String,
    pub state: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDto {
    pub profiles: Vec<String>,
    pub stores: Vec<StoreDto>,
    pub known_stores: Vec<StoreDto>,
    pub inventory: Vec<CatalogInventoryDto>,
    pub store_reads: Vec<CatalogStoreReadDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_item_reads: Option<Vec<String>>,
    pub items: Vec<ItemDto>,
    pub failures: Vec<CatalogFailureDto>,
    pub blocked_profiles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_metadata: Option<CatalogLocalMetadataDto>,
    /// Identifies the native snapshot this response came from. Later reads
    /// that pass it back fail when the snapshot has since been replaced.
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogLocalMetadataDto {
    pub accounts: Vec<super::accounts::AccountDto>,
    pub profiles: Vec<CatalogProfileDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProfileDto {
    pub profile: String,
    pub label: Option<String>,
    pub configured_endpoint: String,
    pub status: Option<super::servers::ServerStatusSnapshotDto>,
    pub error: Option<AgentError>,
}

impl CatalogDto {
    pub(super) fn from_snapshot(snapshot: &CatalogSnapshot) -> Result<Self, AgentError> {
        Ok(Self {
            profiles: snapshot.profiles.clone(),
            full_item_reads: snapshot.full_item_reads.clone(),
            stores: snapshot
                .stores
                .iter()
                .map(store_dto)
                .collect::<Result<Vec<_>, _>>()?,
            known_stores: snapshot
                .known_stores
                .iter()
                .map(store_dto)
                .collect::<Result<Vec<_>, _>>()?,
            inventory: snapshot
                .inventory
                .iter()
                .map(CatalogInventoryDto::from)
                .collect(),
            store_reads: snapshot
                .store_reads
                .iter()
                .map(|read| CatalogStoreReadDto {
                    store: store_id(&read.store),
                    state: match read.state {
                        foks_desktop::CatalogStoreReadState::NotLoaded => "not-loaded",
                        foks_desktop::CatalogStoreReadState::Complete => "complete",
                        foks_desktop::CatalogStoreReadState::Failed => "failed",
                    },
                })
                .collect(),
            items: snapshot
                .items
                .iter()
                .map(item_dto)
                .collect::<Result<Vec<_>, _>>()?,
            failures: snapshot
                .failures
                .iter()
                .map(|failure| {
                    let (scope, profile, source, store) = match &failure.scope {
                        CatalogFailureScope::Profile { profile, source } => {
                            ("profile", Some(profile.clone()), Some(source.clone()), None)
                        }
                        CatalogFailureScope::Store(store) => (
                            "store",
                            Some(store.profile().to_owned()),
                            None,
                            Some(store_id(store)),
                        ),
                    };
                    CatalogFailureDto {
                        scope,
                        profile,
                        source,
                        store,
                        error: AgentError::from_desktop(failure.error.clone()),
                    }
                })
                .collect(),
            blocked_profiles: snapshot.blocked_profiles.clone(),
            local_metadata: None,
            generation: 0,
        })
    }
}

fn store_dto(store: &CatalogStoreSummary) -> Result<StoreDto, AgentError> {
    Ok(match store {
        CatalogStoreSummary::Account { store } => StoreDto {
            id: store_id(&CatalogStoreRef::Account(store.clone())),
            kind: "account",
            name: store.account_alias.clone(),
            server: store.profile.clone(),
            account: store.account_alias.clone(),
            alias: None,
            active: None,
            team_kind: None,
            team_id_hex: None,
            creation_phase: None,
            chain_seqno: None,
        },
        CatalogStoreSummary::Team {
            store,
            kind,
            name,
            active,
            creation_phase,
            chain_seqno,
        } => {
            let team_kind = match kind.as_str() {
                "named" => "named",
                "ad-hoc" => "adhoc",
                other => {
                    return Err(AgentError::new(
                        "invalid-response",
                        format!("Unsupported group kind: {other}"),
                        false,
                    ));
                }
            };
            StoreDto {
                id: store_id(&CatalogStoreRef::Team(store.clone())),
                kind: "team",
                name: name.clone().unwrap_or_else(|| store.team_alias.clone()),
                server: store.profile.clone(),
                account: store.account_alias.clone(),
                alias: Some(store.team_alias.clone()),
                active: Some(*active),
                team_kind: Some(team_kind.to_owned()),
                team_id_hex: Some(store.team_id.clone()),
                creation_phase: creation_phase.clone(),
                chain_seqno: *chain_seqno,
            }
        }
    })
}

fn item_dto(item: &CatalogItem) -> Result<ItemDto, AgentError> {
    Ok(ItemDto {
        store: store_id(&item.store),
        path: item.metadata.path.clone(),
        kind: match item.metadata.node_type.as_str() {
            "small-file" => "Secret",
            "file" => "File",
            "symlink" => "Link",
            "directory" => "Folder",
            other => {
                return Err(AgentError::new(
                    "invalid-response",
                    format!("Unsupported vault item type: {other}"),
                    false,
                ));
            }
        },
        // Large-file sizes remain optional for legacy and Go-created files.
        dirent_id: item
            .metadata
            .dirent_id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        size: item.metadata.size,
        version: item.metadata.version,
        read: item.metadata.read_role.into(),
        write: item.metadata.write_role.into(),
        shape: item.metadata.shape.map(|shape| match shape {
            KvEntryShape::Login => "login",
            KvEntryShape::Other => "other",
        }),
    })
}

#[derive(Debug, Serialize)]
pub struct ReadItemDto {
    pub store: String,
    pub path: String,
    pub version: u64,
    #[serde(serialize_with = "serialize_secret")]
    pub value: Zeroizing<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadResult {
    pub saved: bool,
}

/// The acknowledgement of an item copy. The host clears the clipboard
/// `clear_after_seconds` after the write unless it no longer holds the copied
/// value, so the renderer can state when that happens.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardAck {
    pub ok: bool,
    pub clear_after_seconds: u64,
}

impl ClipboardAck {
    fn written() -> Self {
        Self {
            ok: true,
            clear_after_seconds: crate::clipboard::AUTO_CLEAR_SECONDS,
        }
    }
}

/// Checks every agent call, including the chunk calls inside the shared read
/// helper. The owned interval outlives a cancelled native command's worker.
struct IntervalTransport {
    inner: Arc<dyn foks_desktop::AgentTransport>,
    interval: UnlockedInterval,
}

impl foks_desktop::AgentTransport for IntervalTransport {
    fn call_with_deadline(
        &self,
        mut operation: Operation,
        deadline: &foks_local_deadline::Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<serde_json::Value, foks_desktop::AgentError> {
        if self.interval.check().is_err() {
            operation.zeroize_plaintext();
            return Err(foks_desktop::AgentError::Cancelled);
        }
        let mut result = self
            .inner
            .call_with_deadline(operation, deadline, cancelled);
        if self.interval.checkpoint("after-agent-read").is_err() {
            foks_desktop::discard_transport_result(&mut result);
            return Err(foks_desktop::AgentError::Cancelled);
        }
        result
    }

    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        call_in_interval(self.inner.as_ref(), &self.interval, operation)
    }
}

/// Makes one agent call inside `interval`: refused once the interval has
/// ended, and its result cleared and discarded when the interval ended while
/// it ran.
fn call_in_interval(
    transport: &dyn foks_desktop::AgentTransport,
    interval: &UnlockedInterval,
    operation: Operation,
) -> Result<serde_json::Value, foks_desktop::AgentError> {
    interval
        .check()
        .map_err(|_| foks_desktop::AgentError::Cancelled)?;
    let mut result = transport.call(operation);
    if interval.checkpoint("after-agent-read").is_err() {
        if let Ok(value) = &mut result {
            clear_response(value);
        }
        return Err(foks_desktop::AgentError::Cancelled);
    }
    result
}

pub(super) fn clear_response(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(value) => zeroize::Zeroize::zeroize(value),
        serde_json::Value::Array(values) => values.iter_mut().for_each(clear_response),
        serde_json::Value::Object(values) => values.values_mut().for_each(clear_response),
        value => *value = serde_json::Value::Null,
    }
}

/// The agent answers `conflict` when the entry it finds at a read's path is
/// no longer the listed version or dirent. For a read that is a stale
/// listing, not an edit conflict, and the renderer answers `conflict` with
/// the edit-conflict sheet, so the read commands report it as the host's own
/// lookup does.
pub(super) fn stale_read(error: AgentError) -> AgentError {
    if error.code == "conflict" {
        AgentError::new(
            "item-version-mismatch",
            "This item changed. Refresh the vault before reading it.",
            true,
        )
    } else {
        error
    }
}

pub(super) fn read_text(
    transport: Arc<dyn foks_desktop::AgentTransport>,
    item: &CatalogItem,
    interval: &UnlockedInterval,
) -> Result<ReadItemDto, AgentError> {
    let transport = IntervalTransport {
        inner: transport,
        interval: interval.clone(),
    };
    let result = foks_desktop::read_catalog_item(&transport, item);
    interval.check()?;
    let KvItemRead {
        path,
        version,
        value,
        ..
    } = result
        .map_err(AgentError::from_desktop)
        .map_err(stale_read)?;
    let value = match value {
        KvItemValue::File(bytes) => std::str::from_utf8(&bytes)
            .map(|text| Zeroizing::new(text.to_owned()))
            .map_err(|_| {
                AgentError::new(
                    "not-text",
                    "This file is binary or non-UTF-8. Download the file to view it.",
                    false,
                )
            })?,
        KvItemValue::Symlink(target) => Zeroizing::new(target.to_string()),
        KvItemValue::Directory => {
            return Err(AgentError::new(
                "not-readable",
                "Folders do not have readable content.",
                false,
            ))
        }
    };
    Ok(ReadItemDto {
        store: store_id(&item.store),
        path,
        version,
        value,
    })
}

pub(super) fn protocol_store(store: &CatalogStoreRef) -> KvStoreRef {
    match store {
        CatalogStoreRef::Account(store) => KvStoreRef::Account(store.clone()),
        CatalogStoreRef::Team(store) => KvStoreRef::Team(store.clone()),
    }
}

/// How much one vault stores on its server, as the server's KV usage read
/// reports it: the number of files and their total ciphertext bytes. It
/// carries no limit. The server enforces its quota on a larger measure
/// (directories, metadata and envelopes as well as file ciphertext), so the
/// renderer shows these figures without a fraction of a quota.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct StoreUsageDto {
    pub files: u64,
    pub bytes: u64,
}

/// Reads one vault's usage through the agent's data-read path: binds the
/// vault's account, narrows the binding to the team for a team vault, and
/// asks for usage. The agent checks the binding's host and user against the
/// account again before and after the read; this side checks that the binding
/// names the account it asked for.
pub(super) fn read_store_usage(
    transport: &dyn foks_desktop::AgentTransport,
    store: &CatalogStoreRef,
) -> Result<StoreUsageDto, AgentError> {
    use foks_agent_proto::data::{DataRead, DataScope, DataUsage};
    let (profile, account_alias, team_id) = match store {
        CatalogStoreRef::Account(store) => (&store.profile, &store.account_alias, None),
        CatalogStoreRef::Team(store) => (
            &store.profile,
            &store.account_alias,
            Some(store.team_id.clone()),
        ),
    };
    let mut scope: DataScope = serde_json::from_value(
        transport
            .call(Operation::BindDataAccount {
                profile: profile.clone(),
                account_alias: account_alias.clone(),
            })
            .map_err(AgentError::from_desktop)?,
    )
    .map_err(|_| invalid_response("Invalid account binding."))?;
    if &scope.profile != profile || &scope.account_alias != account_alias || scope.team_id.is_some()
    {
        return Err(invalid_response(
            "The agent bound a different account than the vault's.",
        ));
    }
    scope.team_id = team_id;
    let usage: DataUsage = serde_json::from_value(
        transport
            .call(Operation::ReadData {
                scope,
                query: DataRead::Usage,
            })
            .map_err(AgentError::from_desktop)?,
    )
    .map_err(|_| invalid_response("Invalid storage usage."))?;
    Ok(StoreUsageDto {
        files: usage.files,
        bytes: usage.bytes,
    })
}

/// Saves a File item to `destination` through a temporary file beside it.
/// The read and the sync run under `interval` and `deadline`; only replacing
/// the destination runs inside the interval's publication gate. Any failure
/// after the interval ended, including creating or syncing the temporary
/// file, is reported as `app-locked`.
pub(super) fn download_to_path(
    transport: &dyn foks_desktop::AgentTransport,
    item: &CatalogItem,
    destination: &Path,
    interval: &UnlockedInterval,
    deadline: &foks_local_deadline::Deadline,
) -> Result<(), AgentError> {
    download_in_interval(transport, item, destination, interval, deadline).or_else(|error| {
        interval.check()?;
        Err(error)
    })
}

fn download_in_interval(
    transport: &dyn foks_desktop::AgentTransport,
    item: &CatalogItem,
    destination: &Path,
    interval: &UnlockedInterval,
    deadline: &foks_local_deadline::Deadline,
) -> Result<(), AgentError> {
    let parent = destination.parent().ok_or_else(|| {
        AgentError::new(
            "download-path",
            "The chosen destination path has no parent folder.",
            false,
        )
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        AgentError::new(
            "download-write",
            format!("Could not create temporary download file: {error}"),
            true,
        )
    })?;
    read_file_content(transport, item, interval, deadline, &mut |content| {
        temporary.write_all(content).map_err(|error| {
            AgentError::new(
                "download-write",
                format!("Could not save the file: {error}"),
                true,
            )
        })
    })?;
    temporary.as_file().sync_all().map_err(|error| {
        AgentError::new(
            "download-write",
            format!("Could not finish saving the file: {error}"),
            true,
        )
    })?;
    // All transfer and sync work precedes the short publication critical section.
    // NamedTempFile removes the unpublished plaintext on every rejected path.
    interval.publish(|| {
        deadline.check().map_err(super::deadline::clock_error)?;
        temporary.persist(destination).map_err(|error| {
            AgentError::new(
                "download-write",
                format!(
                    "Could not save the downloaded file to destination: {}",
                    error.error
                ),
                true,
            )
        })?;
        Ok(())
    })
}

/// Reads a File item at the catalog's version and entry identity and hands
/// its content to `sink` in order: a small file in one piece, a large file
/// one bound chunk at a time. Every agent call and every piece is checked
/// against `interval` and `deadline` before `sink` sees it, and each piece is
/// zeroized after `sink` returns. A failure after the interval ended is
/// reported as `app-locked`. Other node types are refused with `not-file`.
pub(super) fn read_file_content(
    transport: &dyn foks_desktop::AgentTransport,
    item: &CatalogItem,
    interval: &UnlockedInterval,
    deadline: &foks_local_deadline::Deadline,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), AgentError>,
) -> Result<(), AgentError> {
    read_file_content_in_interval(transport, item, interval, deadline, sink).or_else(|error| {
        interval.check()?;
        Err(error)
    })
}

fn read_file_content_in_interval(
    transport: &dyn foks_desktop::AgentTransport,
    item: &CatalogItem,
    interval: &UnlockedInterval,
    deadline: &foks_local_deadline::Deadline,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), AgentError>,
) -> Result<(), AgentError> {
    deadline.check().map_err(super::deadline::clock_error)?;
    interval.check()?;
    let store = protocol_store(&item.store);
    let value = call_in_interval(
        transport,
        interval,
        Operation::ReadKv {
            store: store.clone(),
            path: item.metadata.path.clone(),
            dirent_id: item.metadata.dirent_id,
            version: item.metadata.version,
        },
    )
    .map_err(AgentError::from_desktop)?;
    let mut read: KvReadResult = serde_json::from_value(value)
        .map_err(|error| AgentError::new("invalid-response", error.to_string(), false))?;
    // A legacy file can have a locally measured catalog size while its node
    // metadata still has none. Compare sizes only when both paths report one.
    let size_matches = match (item.metadata.size, read.size) {
        (Some(expected), Some(actual)) => expected == actual,
        _ => true,
    };
    let metadata_matches = read.store == store
        && read.path == item.metadata.path
        && read.dirent_id == item.metadata.dirent_id
        && read.version == item.metadata.version
        && read.node_type == item.metadata.node_type
        && size_matches
        && read.read_role == item.metadata.read_role
        && read.write_role == item.metadata.write_role;
    if !metadata_matches {
        zeroize_read_payload(&mut read);
        return Err(AgentError::new(
            "response-binding",
            "The agent returned data for a different item.",
            true,
        ));
    }
    match read.node_type.as_str() {
        "small-file" => {
            if read.symlink_target.is_some() {
                zeroize_read_payload(&mut read);
                return Err(AgentError::new(
                    "invalid-response",
                    "The agent returned an unexpected link target in a file response.",
                    false,
                ));
            }
            let content = Zeroizing::new(read.content.take().ok_or_else(|| {
                AgentError::new(
                    "invalid-response",
                    "The agent response is missing file content.",
                    false,
                )
            })?);
            if Some(content.len() as u64) != read.size {
                return Err(AgentError::new(
                    "invalid-response",
                    "The received file length does not match its catalog size.",
                    false,
                ));
            }
            deadline.check().map_err(super::deadline::clock_error)?;
            interval.check()?;
            sink(&content)?;
        }
        "file" => {
            if read.content.is_some() || read.symlink_target.is_some() {
                zeroize_read_payload(&mut read);
                return Err(AgentError::new(
                    "invalid-response",
                    "The agent returned unexpected inline data in a chunked response.",
                    false,
                ));
            }
            // Legacy files may not report a size, so the authenticated
            // end-of-file flag still controls this loop. The bound also
            // protects against an agent that never sets that flag.
            let mut offset = 0u64;
            loop {
                let length = DOWNLOAD_CHUNK_BYTES;
                let value = call_in_interval(
                    transport,
                    interval,
                    Operation::ReadKvChunk {
                        store: store.clone(),
                        path: item.metadata.path.clone(),
                        dirent_id: item.metadata.dirent_id,
                        version: item.metadata.version,
                        offset,
                        length,
                    },
                )
                .map_err(AgentError::from_desktop)?;
                let mut chunk: KvChunkResult = serde_json::from_value(value).map_err(|error| {
                    AgentError::new("invalid-response", error.to_string(), false)
                })?;
                let content = Zeroizing::new(std::mem::take(&mut chunk.content));
                deadline.check().map_err(super::deadline::clock_error)?;
                interval.check()?;
                let next = offset.checked_add(content.len() as u64).ok_or_else(|| {
                    AgentError::new(
                        "invalid-response",
                        "File download chunk offset calculation overflowed.",
                        false,
                    )
                })?;
                let valid = chunk.store == store
                    && chunk.path == item.metadata.path
                    && chunk.dirent_id == item.metadata.dirent_id
                    && chunk.version == item.metadata.version
                    && chunk.offset == offset
                    && !content.is_empty()
                    && content.len() <= length as usize
                    && next <= MAXIMUM_DOWNLOAD_BYTES;
                if !valid {
                    return Err(AgentError::new(
                        "response-binding",
                        "The agent returned an invalid file chunk.",
                        true,
                    ));
                }
                let eof = chunk.eof;
                sink(&content)?;
                offset = next;
                deadline.check().map_err(super::deadline::clock_error)?;
                interval.checkpoint("between-chunks")?;
                if eof {
                    if item
                        .metadata
                        .size
                        .or(read.size)
                        .is_some_and(|expected| expected != offset)
                    {
                        return Err(AgentError::new(
                            "response-binding",
                            "The downloaded file length does not match its authenticated size.",
                            false,
                        ));
                    }
                    break;
                }
            }
        }
        _ => {
            zeroize_read_payload(&mut read);
            return Err(AgentError::new(
                "not-file",
                "Only File items can be downloaded.",
                false,
            ));
        }
    }
    Ok(())
}

pub(super) fn select_download_destination<T>(
    interval: &UnlockedInterval,
    select: impl FnOnce() -> T,
) -> Result<T, AgentError> {
    interval.check()?;
    let selected = select();
    interval.checkpoint("after-dialog")?;
    Ok(selected)
}

fn zeroize_read_payload(read: &mut KvReadResult) {
    if let Some(content) = &mut read.content {
        zeroize::Zeroize::zeroize(content);
    }
    if let Some(target) = &mut read.symlink_target {
        zeroize::Zeroize::zeroize(target);
    }
}

pub(super) fn parse_item_role(value: &str) -> Result<KvRole, AgentError> {
    match value {
        "Owner" => Ok(KvRole::Owner),
        "Admin" => Ok(KvRole::Admin),
        value => {
            let Some(visibility) = value.strip_prefix("Member:") else {
                return Err(invalid_request(
                    "Role must be 'Owner', 'Admin', or 'Member:<visibility>'.",
                ));
            };
            let parsed = visibility.parse::<i16>().map_err(|_| {
                invalid_request(
                    "Visibility level must be a whole number between -32,768 and 32,767 (for example Member:10).",
                )
            })?;
            if parsed.to_string() != visibility {
                return Err(invalid_request(
                    "Member visibility must be a valid signed integer without leading zeros or spaces.",
                ));
            }
            Ok(KvRole::Member { visibility: parsed })
        }
    }
}

pub(super) fn create_item_roles(
    store: &CatalogStoreRef,
    read_role: Option<&str>,
    write_role: Option<&str>,
) -> Result<(KvRole, KvRole), AgentError> {
    match store {
        CatalogStoreRef::Account(_) => match (read_role, write_role) {
            (None, None) => Ok((KvRole::Owner, KvRole::Owner)),
            _ => Err(invalid_request(
                "Account item roles are fixed to Owner; omit read and write roles.",
            )),
        },
        CatalogStoreRef::Team(_) => match (read_role, write_role) {
            (Some(read_role), Some(write_role)) => {
                Ok((parse_item_role(read_role)?, parse_item_role(write_role)?))
            }
            _ => Err(invalid_request(
                "Group item creation requires both read and write roles.",
            )),
        },
    }
}

pub(super) fn set_create_operation_roles(
    mut operation: Operation,
    read_role: KvRole,
    write_role: KvRole,
) -> Result<Operation, AgentError> {
    set_create_operation_roles_in_place(&mut operation, read_role, write_role)?;
    Ok(operation)
}

fn set_create_operation_roles_in_place(
    operation: &mut Operation,
    read_role: KvRole,
    write_role: KvRole,
) -> Result<(), AgentError> {
    match operation {
        Operation::PutKv {
            read_role: operation_read,
            write_role: operation_write,
            ..
        }
        | Operation::MkdirKv {
            read_role: operation_read,
            write_role: operation_write,
            ..
        } => {
            *operation_read = read_role;
            *operation_write = write_role;
            Ok(())
        }
        _ => Err(AgentError::new(
            "invalid-builder",
            "Failed to create item: unsupported operation.",
            false,
        )),
    }
}

pub(super) fn set_create_mutation_roles(
    mut mutation: KvAccountMutation,
    read_role: KvRole,
    write_role: KvRole,
) -> Result<KvAccountMutation, AgentError> {
    match &mut mutation {
        KvAccountMutation::Inline(operation) => {
            set_create_operation_roles_in_place(operation, read_role, write_role)?;
        }
        KvAccountMutation::Stream { header, .. } => {
            header.read_role = read_role;
            header.write_role = write_role;
        }
    }
    Ok(mutation)
}

pub(super) fn take_text_value(value: String) -> Result<Vec<u8>, AgentError> {
    let mut value = Zeroizing::new(value);
    if value.len() > MAXIMUM_TEXT_ITEM_BYTES {
        return Err(invalid_request(
            "Secret values must be at most 2,040 bytes.",
        ));
    }
    Ok(std::mem::take(&mut *value).into_bytes())
}

pub(super) fn require_file_item(item: &CatalogItem) -> Result<(), AgentError> {
    if matches!(item.metadata.node_type.as_str(), "file" | "small-file") {
        Ok(())
    } else {
        Err(invalid_request(
            "Only files can be replaced from a local file.",
        ))
    }
}

pub(super) fn require_text_item(item: &CatalogItem) -> Result<(), AgentError> {
    if item.metadata.node_type == "small-file" {
        Ok(())
    } else {
        Err(invalid_request("Only secrets can be edited as text."))
    }
}

pub(super) fn file_create_header(
    store: &CatalogStoreRef,
    path: &str,
    total_length: u64,
    read_role: Option<&str>,
    write_role: Option<&str>,
) -> Result<KvUploadHeader, AgentError> {
    let (read_role, write_role) = create_item_roles(store, read_role, write_role)?;
    let mut header =
        foks_desktop::create_kv_file_upload(store, path, total_length).map_err(invalid_request)?;
    header.read_role = read_role;
    header.write_role = write_role;
    Ok(header)
}

pub(super) fn file_edit_header(
    item: &CatalogItem,
    total_length: u64,
) -> Result<KvUploadHeader, AgentError> {
    foks_desktop::edit_kv_file_upload(item, total_length).map_err(invalid_request)
}

fn open_regular_file(path: &Path) -> Result<(File, u64), AgentError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|error| {
        AgentError::new(
            "upload-source",
            format!("Could not open the selected file: {error}"),
            false,
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        AgentError::new(
            "upload-source",
            format!("Could not inspect the selected file: {error}"),
            false,
        )
    })?;
    if !metadata.is_file() {
        return Err(AgentError::new(
            "upload-source",
            "Only a regular file can be imported.",
            false,
        ));
    }
    Ok((file, metadata.len()))
}

struct SourceReader {
    file: File,
    read_error: Option<String>,
}

impl std::io::Read for SourceReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.file.read(buffer) {
            Ok(count) => Ok(count),
            Err(error) => {
                self.read_error = Some(error.to_string());
                Err(error)
            }
        }
    }
}

pub(super) fn upload_file(
    transport: &dyn foks_desktop::AgentTransport,
    mut header: KvUploadHeader,
    source: &Path,
    kind: MutationKind,
) -> Result<(), AgentError> {
    foks_desktop::ensure_upload_identity(transport, &mut header)
        .map_err(AgentError::from_desktop)?;
    let upload_id = header.upload_id.map(|id| id.to_string());
    let (file, total_length) = open_regular_file(source)?;
    header.total_length = total_length;
    let mut reader = SourceReader {
        file,
        read_error: None,
    };
    let result = transport.put_kv_stream(header, &mut reader);
    if let Some(detail) = reader.read_error {
        return Err(AgentError::new(
            "upload-source",
            format!("The selected file could not be read: {detail}"),
            false,
        ));
    }
    if matches!(result, Err(foks_desktop::AgentError::Transport(_))) {
        // If the transport failed, verify whether the file size actually changed
        // before treating it as a source modification.
        if reader
            .file
            .metadata()
            .is_ok_and(|metadata| metadata.len() != total_length)
        {
            return Err(AgentError::new(
                "upload-source-changed",
                "The selected file changed while being read. Drop or choose it again.",
                false,
            ));
        }
    }
    result.map(|_| ()).map_err(|error| {
        let mut error = map_mutation_error(error, kind);
        if let Some(id) = upload_id {
            if let Some(hint) = upload_handle_hint(&error, &id) {
                error.message = format!("{} {hint}", error.message);
            }
        }
        error
    })
}

/// What a failed upload's message says about its handle. The desktop issues
/// the handle before the stream starts, and the agent records it only in
/// `register_upload_intent`, after its admission, capability, credential,
/// clock and capacity checks; refusals there leave nothing for the Uploads
/// list to show. The hint is therefore given only for failures that can
/// follow registration: an ambiguous outcome, a deadline or cancellation, a
/// conflict, a full spool or an existing handle with another intent, and the
/// server failures of the upload itself (the agent does no network work
/// before it registers the handle). A lost agent connection and a busy
/// profile lock can come before or after registration, so their hint says
/// that the agent may have recorded the handle.
pub(super) fn upload_handle_hint(error: &AgentError, id: &str) -> Option<String> {
    const RECORDED: &[&str] = &[
        "ambiguous",
        "deadline-exceeded",
        "cancelled",
        "conflict",
        "already-exists",
        "upload-spool-full",
        "upload-identity-conflict",
        "server-unavailable",
        "rate-limited",
        "quota-exceeded",
        "operation-failed",
    ];
    const UNCERTAIN: &[&str] = &["agent-lost", "profile-busy"];
    let code = error.code.as_str();
    if UNCERTAIN.contains(&code) {
        return Some(format!(
            "If the agent recorded upload handle {id} before the failure, open Uploads to inspect or resume prepared content."
        ));
    }
    (error.ambiguous || RECORDED.contains(&code)).then(|| {
        format!("Upload handle: {id}. Open Uploads to inspect or resume prepared content.")
    })
}

async fn apply_file_upload(
    state: &AppState,
    header: KvUploadHeader,
    source: PathBuf,
    kind: MutationKind,
) -> Result<MutationDto, AgentError> {
    state.invalidate_catalog_items(&super::execution::catalog_store(&header.store));
    let transport = state.transport()?;
    let result = {
        let deadline = state.deadline()?.clone();
        super::deadline::run(
            &deadline,
            true,
            move || upload_file(transport.as_ref(), header, &source, kind),
            |_| {},
        )
        .await
    };
    if result.as_ref().is_err_and(|error| error.ambiguous) {
        state
            .mutation_requires_refresh()
            .store(true, Ordering::Release);
    }
    result.map(|()| MutationDto { applied: true })
}

pub(super) fn catalog_local_metadata(
    snapshot: &CatalogSnapshot,
    profiles: &[super::servers::ProfileSummary],
) -> Result<CatalogLocalMetadataDto, AgentError> {
    struct CachedOnly;
    impl foks_desktop::AgentTransport for CachedOnly {
        fn call(&self, _: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
            Err(foks_desktop::AgentError::Transport(
                "Catalog metadata is not loaded.".into(),
            ))
        }
    }
    let accounts = super::accounts::load_accounts(&CachedOnly, snapshot)?;
    let profiles = profiles
        .iter()
        .map(|profile| {
            let result = snapshot
                .profile_overviews
                .iter()
                .find(|overview| overview.profile == profile.name)
                .map(|overview| match overview.server_status.clone() {
                    foks_agent_proto::ResponseResult::Success { value } => {
                        super::servers::server_status_response(
                            value,
                            &profile.name,
                            &profile.probe,
                            !matches!(
                                profile.protocol,
                                super::servers::ProfileProtocolSummary::V019
                            ),
                        )
                    }
                    foks_agent_proto::ResponseResult::Error {
                        code,
                        message,
                        fields,
                    } => Err(AgentError::from_desktop(
                        foks_desktop::AgentError::Protocol {
                            code,
                            message,
                            fields: fields.into(),
                        },
                    )),
                });
            let (status, error) = match result {
                Some(Ok(status)) => (Some(status), None),
                Some(Err(error)) => (None, Some(error)),
                None => (None, None),
            };
            CatalogProfileDto {
                profile: profile.name.clone(),
                label: profile.label.clone(),
                configured_endpoint: profile.probe.clone(),
                status,
                error,
            }
        })
        .collect();
    Ok(CatalogLocalMetadataDto { accounts, profiles })
}

/// The configured profiles, decoded into the desktop's own summary. The
/// catalog walk takes this listing rather than issuing its own.
pub(super) fn catalog_profile_listing(
    transport: &dyn foks_desktop::AgentTransport,
) -> Result<Vec<super::servers::ProfileSummary>, AgentError> {
    serde_json::from_value(
        transport
            .call(Operation::ListProfiles)
            .map_err(AgentError::from_desktop)?,
    )
    .map_err(|error| super::validation::invalid_response(error.to_string()))
}

/// A catalog response at the published generation. `profiles` attaches the
/// locally known server facts and accounts, which spare the renderer a
/// `list_servers`, a `describe_server_status` per server and a `list_accounts`
/// over the agent's single local lane.
pub(super) fn catalog_dto(
    snapshot: &CatalogSnapshot,
    generation: u64,
    profiles: Option<&[super::servers::ProfileSummary]>,
) -> Result<CatalogDto, AgentError> {
    let mut dto = CatalogDto::from_snapshot(snapshot)?;
    if let Some(profiles) = profiles {
        dto.local_metadata = Some(catalog_local_metadata(snapshot, profiles)?);
    }
    dto.generation = generation;
    Ok(dto)
}

async fn load_catalog(
    state: &AppState,
    app: &tauri::AppHandle,
    include_items: bool,
    requested_fresh: bool,
    on_partial: Option<tauri::ipc::Channel<CatalogDto>>,
) -> Result<CatalogDto, AgentError> {
    let access = crate::applock::unlocked_generation(app)?;
    // A store-only read walks no KV tree, so nothing it reads can be served
    // from the agent's retained catalogs and it never settles the epoch.
    let (fresh, epoch) = if include_items {
        state.catalog_read_freshness(requested_fresh)
    } else {
        (false, 0)
    };
    // Store-only discovery does not replace the accepted full catalog or
    // cancel a concurrent catalog refresh.
    let (generation, token) = if include_items {
        state.begin_catalog_load_checked()?
    } else {
        (
            state.catalog_at(None)?.0,
            foks_desktop::CatalogLoadToken::default(),
        )
    };
    let transport = state.transport()?;
    let worker_state = state.clone();
    let worker_app = app.clone();
    let (snapshot, profiles) = {
        let deadline = state.deadline()?.clone();
        super::deadline::run(
            &deadline,
            false,
            move || {
                // An item read takes this listing once and hands it to the catalog
                // walk, so the walk issues no listing of its own and the local
                // metadata attached below is built from the same registry view. A
                // store-only read walks no KV tree and attaches no metadata, so it
                // does not pay for the listing.
                let profiles: Vec<super::servers::ProfileSummary> = if include_items {
                    catalog_profile_listing(transport.as_ref())?
                } else {
                    Vec::new()
                };
                let names: Vec<String> = profiles
                    .iter()
                    .map(|profile| profile.name.clone())
                    .collect();
                let snapshot = if let Some(channel) = on_partial {
                    let failure = std::sync::Mutex::new(None);
                    let snapshot = foks_desktop::load_catalog_progressive_with_profiles(
                        transport,
                        names,
                        token.clone(),
                        fresh,
                        |snapshot| {
                            let result = (|| {
                                worker_state
                                    .deadline()?
                                    .check()
                                    .map_err(super::deadline::clock_error)?;
                                crate::applock::require_unlocked_generation(&worker_app, access)?;
                                let mut sent = Ok(());
                                if !worker_state.publish_catalog_snapshot(
                                    generation,
                                    snapshot.clone(),
                                    |published, accepted| {
                                        sent = (|| {
                                            let dto =
                                                catalog_dto(accepted, published, Some(&profiles))?;
                                            crate::applock::require_unlocked_generation(
                                                &worker_app,
                                                access,
                                            )?;
                                            worker_state
                                                .deadline()?
                                                .check()
                                                .map_err(super::deadline::clock_error)?;
                                            channel.send(dto).map_err(|error| {
                                                AgentError::unknown(error.to_string())
                                            })
                                        })();
                                    },
                                ) {
                                    return Err(super::context::catalog_changed_during_read());
                                }
                                sent
                            })();
                            if let Err(error) = result {
                                token.cancel();
                                *failure
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                    Some(error);
                            }
                        },
                    )
                    .map_err(AgentError::from_desktop);
                    if let Some(error) = failure
                        .into_inner()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                    {
                        return Err(error);
                    }
                    snapshot
                } else if include_items {
                    foks_desktop::load_catalog_cancellable_with_profiles(
                        transport, names, token, fresh,
                    )
                    .map_err(AgentError::from_desktop)
                } else {
                    foks_desktop::load_stores_cancellable(transport, token)
                        .map_err(AgentError::from_desktop)
                }?;
                Ok::<_, AgentError>((snapshot, profiles))
            },
            |_| {},
        )
        .await
    }?;
    let mut dto = catalog_dto(&snapshot, generation, None)?;
    // A load that lost its generation to a later load or mutation must not be
    // reported as the current snapshot: the reads that follow it would answer
    // from whichever snapshot replaced it.
    crate::applock::require_unlocked_generation(app, access)?;
    if include_items {
        let mut result = Ok(dto);
        if !state.publish_catalog_snapshot(generation, snapshot, |published, accepted| {
            result = catalog_dto(accepted, published, Some(&profiles));
        }) {
            return Err(super::context::catalog_changed_during_read());
        }
        dto = result?;
        if fresh {
            state.note_fresh_catalog_read(epoch);
        }
    }
    crate::applock::require_unlocked_generation(app, access)?;
    Ok(dto)
}

#[tauri::command]
pub async fn list_stores(
    webview: tauri::Webview,
    state: super::deadline::CommandState,
) -> Result<CatalogDto, AgentError> {
    require_main_window(&webview)?;
    load_catalog(&state, webview.app_handle(), false, false, None).await
}

#[tauri::command]
pub async fn list_catalog(
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    fresh: Option<bool>,
) -> Result<CatalogDto, AgentError> {
    require_main_window(&webview)?;
    load_catalog(
        &state,
        webview.app_handle(),
        true,
        fresh.unwrap_or(false),
        None,
    )
    .await
}

#[tauri::command]
pub async fn list_profile_catalog(
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    profile: String,
) -> Result<CatalogDto, AgentError> {
    require_main_window(&webview)?;
    let app = webview.app_handle();
    let access = crate::applock::unlocked_generation(app)?;
    let state = state.for_profile(&profile)?;
    let (generation, token) = state.begin_catalog_load_checked()?;
    // A read of one profile settles that profile alone: it says nothing about
    // the other profiles, whose first pages the agent may still hold.
    let (fresh, epoch) = state.catalog_read_freshness(false);
    let transport = state.transport_for("list_profile_catalog", Some(&profile))?;
    let (snapshot, profiles) = {
        let deadline = state.deadline()?.clone();
        super::deadline::run(
            &deadline,
            false,
            move || {
                // The catalog walk takes this listing rather than issuing its own.
                let mut profiles: Vec<super::servers::ProfileSummary> = serde_json::from_value(
                    transport
                        .call_cancellable(Operation::ListProfiles, &|| token.is_cancelled())
                        .map_err(AgentError::from_desktop)?,
                )
                .map_err(|error| super::validation::invalid_response(error.to_string()))?;
                let names = profiles
                    .iter()
                    .map(|candidate| candidate.name.clone())
                    .collect::<Vec<_>>();
                profiles.retain(|candidate| candidate.name == profile);
                let snapshot = foks_desktop::load_profile_catalog_with_profiles(
                    transport, profile, &names, token, fresh,
                )
                .map_err(AgentError::from_desktop)?;
                Ok::<_, AgentError>((snapshot, profiles))
            },
            |_| {},
        )
        .await
    }?;
    crate::applock::require_unlocked_generation(app, access)?;
    let mut result = Err(super::context::catalog_changed_during_read());
    if !state.publish_catalog_snapshot(generation, snapshot, |published, accepted| {
        result = catalog_dto(accepted, published, Some(&profiles));
    }) {
        return Err(super::context::catalog_changed_during_read());
    }
    if fresh {
        state.note_fresh_catalog_read(epoch);
    }
    crate::applock::require_unlocked_generation(app, access)?;
    result
}

#[tauri::command]
pub async fn list_catalog_progressive(
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    on_partial: tauri::ipc::Channel<CatalogDto>,
    fresh: Option<bool>,
) -> Result<CatalogDto, AgentError> {
    require_main_window(&webview)?;
    load_catalog(
        &state,
        webview.app_handle(),
        true,
        fresh.unwrap_or(false),
        Some(on_partial),
    )
    .await
}

#[tauri::command]
pub async fn read_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
) -> Result<ReadItemDto, AgentError> {
    let interval = crate::applock::unlocked_interval(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_read_item(&store_id, &path, version, &dirent_id)?;
    let transport = state.transport()?;
    let worker_interval = interval.clone();
    let result = super::deadline::run(
        state.deadline()?,
        false,
        move || read_text(transport, &item, &worker_interval),
        |read| zeroize::Zeroize::zeroize(&mut *read.value),
    )
    .await?;
    // Errors have no plaintext result to publish; preserve the worker's terminal
    // error classification before applying the successful-result publication gate.
    // The renderer still needs its epoch check: an IPC reply already dispatched
    // before locking can arrive afterward.
    interval.publish(|| {
        state
            .deadline()?
            .check()
            .map_err(super::deadline::clock_error)?;
        Ok(result)
    })
}

/// Reads how much a vault in the current catalog stores. The server computes
/// usage by decoding the vault's nodes on each request, so the renderer asks
/// only when a view that shows it opens, never on a refresh cadence.
#[tauri::command]
pub async fn store_usage(
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
) -> Result<StoreUsageDto, AgentError> {
    require_main_window(&webview)?;
    let interval = crate::applock::unlocked_interval(webview.app_handle())?;
    let (store, _) = state.selected_store(&store_id)?;
    let transport = state.transport()?;
    let worker_interval = interval.clone();
    let usage = super::deadline::run(
        state.deadline()?,
        false,
        move || {
            let transport = IntervalTransport {
                inner: transport,
                interval: worker_interval.clone(),
            };
            read_store_usage(&transport, &store).or_else(|error| {
                worker_interval.check()?;
                Err(error)
            })
        },
        |_| {},
    )
    .await?;
    interval.publish(|| {
        state
            .deadline()?
            .check()
            .map_err(super::deadline::clock_error)?;
        Ok(usage)
    })
}

/// Copies an item's whole stored text; the same command as
/// [`copy_item_field`] with [`ItemField::Value`].
#[tauri::command]
pub async fn copy_item_value(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
) -> Result<ClipboardAck, AgentError> {
    copy_item_field(
        app,
        webview,
        state,
        store_id,
        path,
        version,
        dirent_id,
        ItemField::Value,
    )
    .await
}

/// The part of a stored item [`copy_item_field`] puts on the clipboard.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ItemField {
    Password,
    Username,
    Url,
    /// The whole stored text, which [`copy_item_value`] copies.
    Value,
}

/// Selects one field of a read item's text. Login fields are found by the
/// same parser the agent's listing uses to classify a login, so an item
/// listed as one has a password line this finds; that line may be empty, and
/// an empty or absent field is reported as `field-missing`. The returned
/// value is a fresh zeroizing copy; the full text is dropped, and zeroized,
/// with `value`.
pub(super) fn item_field(
    value: Zeroizing<String>,
    field: ItemField,
) -> Result<Zeroizing<String>, AgentError> {
    let (login_field, name) = match field {
        ItemField::Value => return Ok(value),
        ItemField::Password => (foks_client_app::LoginField::Password, "password"),
        ItemField::Username => (foks_client_app::LoginField::Username, "user name"),
        ItemField::Url => (foks_client_app::LoginField::Url, "website"),
    };
    foks_client_app::login_field(&value, login_field)
        .map(|found| Zeroizing::new(found.to_owned()))
        .ok_or_else(|| AgentError::new("field-missing", format!("This item has no {name}."), false))
}

/// Copies one field of an item, for example only the password of a login
/// rather than its whole stored text, after an exact-version read and with
/// clipboard hygiene. No content crosses IPC; the response says when the
/// clipboard is cleared.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn copy_item_field(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
    field: ItemField,
) -> Result<ClipboardAck, AgentError> {
    let interval = crate::applock::unlocked_interval(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_read_item(&store_id, &path, version, &dirent_id)?;
    let transport = state.transport()?;
    let worker_interval = interval.clone();
    let value = super::deadline::run(
        state.deadline()?,
        false,
        move || {
            read_text(transport, &item, &worker_interval)
                .and_then(|read| item_field(read.value, field))
        },
        |value| zeroize::Zeroize::zeroize(&mut **value),
    )
    .await?;
    interval.publish(|| {
        state
            .deadline()?
            .check()
            .map_err(super::deadline::clock_error)?;
        crate::clipboard::copy_with_hygiene(&app, value)
            .map_err(|error| AgentError::new("clipboard", error, true))
    })?;
    Ok(ClipboardAck::written())
}

/// The address a login's website field names, as the browser is given it.
/// A value with no scheme, such as `example.com/login`, is taken as https; a
/// value has a scheme only when the text before its first `://` is one
/// (`[A-Za-z][A-Za-z0-9+.-]*`), so a `://` in a path or query, as in
/// `example.com/signin?next=https://app.example.com/`, does not count. Only an
/// address [`super::chat::safe_external_url`] accepts is returned: an http or
/// https URL with a host and no user name or password.
pub(super) fn website_url(field: &str) -> Option<url::Url> {
    let field = field.trim();
    let has_scheme = field.split_once("://").is_some_and(|(scheme, _)| {
        let mut chars = scheme.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    if has_scheme {
        super::chat::safe_external_url(field)
    } else {
        super::chat::safe_external_url(&format!("https://{field}"))
    }
}

/// Opens a login's website in the default browser. The `url` field is read
/// at the exact version and selected in Rust as for [`copy_item_field`], an
/// item without one fails with `field-missing`, and a value that is not an
/// http or https address ([`website_url`]) fails with `website-unsupported`.
/// Neither the address nor any other content crosses IPC.
#[tauri::command]
pub async fn open_item_url(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
) -> Result<CommandAck, AgentError> {
    let interval = crate::applock::unlocked_interval(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_read_item(&store_id, &path, version, &dirent_id)?;
    let transport = state.transport()?;
    let worker_interval = interval.clone();
    let field = super::deadline::run(
        state.deadline()?,
        false,
        move || {
            read_text(transport, &item, &worker_interval)
                .and_then(|read| item_field(read.value, ItemField::Url))
        },
        |value| zeroize::Zeroize::zeroize(&mut **value),
    )
    .await?;
    let url = website_url(&field).ok_or_else(|| {
        AgentError::new(
            "website-unsupported",
            "This item's website is not an http or https address.",
            false,
        )
    })?;
    drop(field);
    // The hand-off to the browser runs inside the interval's publication
    // gate, so a lock that engaged during the read stops it.
    super::chat::open_external_url(url, interval, state.deadline()?).await?;
    Ok(CommandAck { ok: true })
}

#[tauri::command]
pub fn copy_text(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    text: String,
) -> Result<CommandAck, AgentError> {
    require_main_window(&webview)?;
    if text.len() > MAXIMUM_CLIPBOARD_TEXT_BYTES {
        return Err(invalid_request(
            "Text exceeds maximum allowable clipboard size of 1 MB.",
        ));
    }
    crate::clipboard::copy_with_hygiene(&app, Zeroizing::new(text))
        .map_err(|error| AgentError::new("clipboard", error, true))?;
    Ok(CommandAck { ok: true })
}

#[tauri::command]
pub async fn download_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
) -> Result<DownloadResult, AgentError> {
    let interval = crate::applock::unlocked_interval(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_read_item(&store_id, &path, version, &dirent_id)?;
    let suggested = item
        .metadata
        .path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("download")
        .to_owned();
    let picker_app = app.clone();
    let picker_interval = interval.clone();
    let destination = tauri::async_runtime::spawn_blocking(move || {
        select_download_destination(&picker_interval, || {
            picker_app
                .dialog()
                .file()
                .set_file_name(suggested)
                .blocking_save_file()
        })
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Save dialog failed: {error}")))??;
    interval.check()?;
    let Some(destination) = destination else {
        return Ok(DownloadResult { saved: false });
    };
    let destination = destination
        .into_path()
        .map_err(|error| AgentError::new("download-path", error.to_string(), false))?;
    let state = state.begin_operation(foks_agent_proto::DeadlineClass::Bulk)?;
    let transport = state.transport()?;
    let worker_interval = interval.clone();
    let deadline = state.deadline()?.clone();
    let worker_deadline = deadline.clone();
    super::deadline::run(
        &deadline,
        false,
        move || {
            download_to_path(
                transport.as_ref(),
                &item,
                &destination,
                &worker_interval,
                &worker_deadline,
            )
            .map_err(stale_read)
        },
        |_| {},
    )
    .await?;
    Ok(DownloadResult { saved: true })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_text_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    value: String,
    read_role: Option<String>,
    write_role: Option<String>,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let store = state.selected_create_store(&store_id)?;
    let (read_role, write_role) =
        create_item_roles(&store, read_role.as_deref(), write_role.as_deref())?;
    let mutation = foks_desktop::create_kv_file_mutation(&store, &path, take_text_value(value)?)
        .map_err(invalid_request)?;
    let mutation = set_create_mutation_roles(mutation, read_role, write_role)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_kv_mutation(&state, mutation, MutationKind::Create).await
}

#[tauri::command]
pub async fn create_folder(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    read_role: Option<String>,
    write_role: Option<String>,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let store = state.selected_create_store(&store_id)?;
    let (read_role, write_role) =
        create_item_roles(&store, read_role.as_deref(), write_role.as_deref())?;
    let operation =
        foks_desktop::create_kv_directory_operation(&store, &path).map_err(invalid_request)?;
    let operation = set_create_operation_roles(operation, read_role, write_role)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_kv_mutation(
        &state,
        KvAccountMutation::Inline(operation),
        MutationKind::Create,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn edit_text_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
    value: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = state.selected_bound_mutation_item(&store_id, &path, version, &dirent_id)?;
    require_text_item(&item)?;
    let mutation = foks_desktop::edit_kv_file_mutation(&item, take_text_value(value)?)
        .map_err(invalid_request)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_kv_mutation(&state, mutation, MutationKind::Guarded).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn move_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
    destination: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = state.selected_bound_mutation_item(&store_id, &path, version, &dirent_id)?;
    let operation =
        foks_desktop::move_kv_operation(&item, &destination).map_err(invalid_request)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_kv_mutation(
        &state,
        KvAccountMutation::Inline(operation),
        MutationKind::Guarded,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn import_dropped_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    source_path: String,
    read_role: Option<String>,
    write_role: Option<String>,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let store = state.selected_create_store(&store_id)?;
    // Validate the destination before resolving the native-authorized source.
    let header = file_create_header(
        &store,
        &path,
        0,
        read_role.as_deref(),
        write_role.as_deref(),
    )?;
    let source_path = Zeroizing::new(source_path);
    let source = state.upload_path(source_path.as_str())?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    let result = apply_file_upload(&state, header, source, MutationKind::Create).await;
    if result.is_ok() {
        state.release_upload_path(source_path.as_str());
    }
    result
}

#[tauri::command]
pub async fn pick_import_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
) -> Result<Option<String>, AgentError> {
    require_main_window(&webview)?;
    let picker_app = app.clone();
    let source = tauri::async_runtime::spawn_blocking(move || {
        picker_app.dialog().file().blocking_pick_file()
    })
    .await
    .map_err(|error| AgentError::unknown(format!("file picker failed: {error}")))?;
    let Some(source) = source else {
        return Ok(None);
    };
    let source = source
        .into_path()
        .map_err(|error| AgentError::new("upload-source", error.to_string(), false))?;
    state.record_picked_path(source).map(Some)
}

#[tauri::command]
pub fn release_import_file(
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    source_path: String,
) -> Result<CommandAck, AgentError> {
    require_main_window(&webview)?;
    let source_path = Zeroizing::new(source_path);
    state.release_upload_path(source_path.as_str());
    Ok(CommandAck { ok: true })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn replace_dropped_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
    source_path: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = state.selected_bound_mutation_item(&store_id, &path, version, &dirent_id)?;
    require_file_item(&item)?;
    let header = file_edit_header(&item, 0)?;
    let source_path = Zeroizing::new(source_path);
    let source = state.upload_path(source_path.as_str())?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    let result = apply_file_upload(&state, header, source, MutationKind::Guarded).await;
    if result.is_ok() {
        state.release_upload_path(source_path.as_str());
    }
    result
}

#[tauri::command]
pub async fn pick_and_replace_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let prepared = state
        .begin_operation(foks_agent_proto::DeadlineClass::Ordinary)?
        .for_store(&store_id)?;
    let original_state = state;
    let state = prepared;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = state.selected_bound_mutation_item(&store_id, &path, version, &dirent_id)?;
    require_file_item(&item)?;
    let header = file_edit_header(&item, 0)?;
    let picker_app = app.clone();
    let source = tauri::async_runtime::spawn_blocking(move || {
        picker_app.dialog().file().blocking_pick_file()
    })
    .await
    .map_err(|error| AgentError::unknown(format!("file picker failed: {error}")))?;
    let Some(source) = source else {
        return Ok(MutationDto { applied: false });
    };
    let source = source
        .into_path()
        .map_err(|error| AgentError::new("upload-source", error.to_string(), false))?;
    let state = original_state
        .begin_operation(foks_agent_proto::DeadlineClass::Bulk)?
        .for_store(&store_id)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_file_upload(&state, header, source, MutationKind::Guarded).await
}

fn parse_upload_identity(text: &str) -> Result<foks_agent_proto::UploadHandle, AgentError> {
    text.parse()
        .map_err(|_| invalid_request("Invalid upload handle."))
}
#[tauri::command]
pub async fn list_file_uploads(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    after: Option<[u8; 32]>,
) -> Result<serde_json::Value, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    list_file_uploads_in(
        &state,
        |state| state.transport(),
        &store_id,
        after,
        || check_mutation_access(unlocked, crate::applock::unlocked_generation(&app)),
    )
    .await
}

/// [`list_file_uploads`] once the command is admitted: `transport` binds the
/// agent transport and `access` checks that the application is still
/// unlocked, which the listing must be when it answers.
pub(super) async fn list_file_uploads_in(
    state: &AppState,
    transport: impl FnOnce(&AppState) -> Result<Arc<dyn foks_desktop::AgentTransport>, AgentError>,
    store_id: &str,
    after: Option<[u8; 32]>,
    access: impl Fn() -> Result<(), AgentError>,
) -> Result<serde_json::Value, AgentError> {
    let (store, _) = state.selected_store(store_id)?;
    let transport = transport(state)?;
    let result = {
        let deadline = state.deadline()?.clone();
        super::deadline::run(
            &deadline,
            false,
            move || {
                transport
                    .call(Operation::ListKvUploads {
                        store: protocol_store(&store),
                        after,
                    })
                    .map_err(AgentError::from_desktop)
            },
            clear_response,
        )
        .await
    }?;
    access()?;
    Ok(result)
}

async fn change_file_upload(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    upload_id: String,
    resume: bool,
) -> Result<serde_json::Value, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    change_file_upload_in(
        &state,
        |state| state.transport(),
        &store_id,
        &upload_id,
        resume,
        || check_mutation_access(unlocked, crate::applock::unlocked_generation(&app)),
    )
    .await
}

/// Resumes or cancels a recorded upload once the command is admitted. The
/// upload reconciliation reservation is taken before anything else, and the
/// store's catalog items are discarded before the agent is asked, since a
/// resume can publish the file. `access` is checked before the agent call
/// only: a lock taken while the agent resumed or cancelled the upload does
/// not undo the change, and the report holds no plaintext, so answering
/// `app-locked` afterwards would report an applied change as never
/// submitted.
pub(super) async fn change_file_upload_in(
    state: &AppState,
    transport: impl FnOnce(&AppState) -> Result<Arc<dyn foks_desktop::AgentTransport>, AgentError>,
    store_id: &str,
    upload_id: &str,
    resume: bool,
    access: impl Fn() -> Result<(), AgentError>,
) -> Result<serde_json::Value, AgentError> {
    let _permit = state.begin_upload_reconciliation()?;
    let (store, _) = state.selected_store(store_id)?;
    let upload_id = parse_upload_identity(upload_id)?;
    access()?;
    state.invalidate_catalog_items(&store);
    let transport = transport(state)?;
    let operation = if resume {
        Operation::ResumeKvUpload {
            store: protocol_store(&store),
            upload_id,
        }
    } else {
        Operation::CancelKvUpload {
            store: protocol_store(&store),
            upload_id,
        }
    };
    let result = {
        let deadline = state.deadline()?.clone();
        super::deadline::run(
            &deadline,
            true,
            move || transport.call(operation).map_err(AgentError::from_desktop),
            clear_response,
        )
        .await
    };
    if result.as_ref().is_err_and(|e| e.ambiguous) {
        state
            .mutation_requires_refresh()
            .store(true, Ordering::Release);
    }
    result
}
#[tauri::command]
pub async fn resume_file_upload(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    upload_id: String,
) -> Result<serde_json::Value, AgentError> {
    require_main_window(&webview)?;
    change_file_upload(app, webview, state, store_id, upload_id, true).await
}
#[tauri::command]
pub async fn cancel_file_upload(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
    upload_id: String,
) -> Result<serde_json::Value, AgentError> {
    require_main_window(&webview)?;
    change_file_upload(app, webview, state, store_id, upload_id, false).await
}
