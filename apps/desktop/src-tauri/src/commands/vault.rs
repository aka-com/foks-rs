//! Vault catalogs, version-bound item reads and writes, and file transfer.

use crate::agent::AgentError;
use crate::commands::context::AppState;
use crate::commands::execution::{
    ambiguous_worker_failure, apply_kv_mutation, map_mutation_error, MutationKind,
};
use crate::commands::preparation::{check_mutation_access, prepare_catalog_mutation};
use crate::commands::types::{CommandAck, MutationDto, RoleDto};
use crate::commands::validation::{
    invalid_request, require_main_window, serialize_secret, DOWNLOAD_CHUNK_BYTES,
    MAXIMUM_CLIPBOARD_TEXT_BYTES, MAXIMUM_DOWNLOAD_BYTES, MAXIMUM_TEXT_ITEM_BYTES,
};
use foks_agent_proto::{
    KvChunkResult, KvReadResult, KvRole, KvStoreRef, KvUploadHeader, Operation,
};
use foks_desktop::{
    CatalogFailureScope, CatalogInventoryState, CatalogItem, CatalogSnapshot, CatalogStoreRef,
    CatalogStoreSummary, KvAccountMutation, KvItemRead, KvItemValue,
};
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{Manager as _, State};
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
    pub store: String,
    pub path: String,
    pub kind: &'static str,
    pub size: Option<u64>,
    pub version: u64,
    pub read: RoleDto,
    pub write: RoleDto,
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
    pub configured_probe: String,
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

pub(super) fn store_id(store: &CatalogStoreRef) -> String {
    match store {
        CatalogStoreRef::Account(store) => serde_json::json!({
            "kind": "account",
            "profile": store.profile,
            "accountAlias": store.account_alias,
        }),
        CatalogStoreRef::Team(store) => serde_json::json!({
            "kind": "team",
            "profile": store.profile,
            "accountAlias": store.account_alias,
            "teamAlias": store.team_alias,
            "teamId": store.team_id,
        }),
    }
    .to_string()
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
        // Catalog metadata intentionally omits plaintext content sizes.
        size: item.metadata.size,
        version: item.metadata.version,
        read: item.metadata.read_role.into(),
        write: item.metadata.write_role.into(),
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

pub(super) fn read_text(
    transport: &dyn foks_desktop::AgentTransport,
    item: &CatalogItem,
) -> Result<ReadItemDto, AgentError> {
    let KvItemRead {
        path,
        version,
        value,
        ..
    } = foks_desktop::read_catalog_item(transport, item).map_err(AgentError::from_desktop)?;
    let value = match value {
        KvItemValue::File(bytes) => String::from_utf8(bytes.to_vec())
            .map(Zeroizing::new)
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

fn protocol_store(store: &CatalogStoreRef) -> KvStoreRef {
    match store {
        CatalogStoreRef::Account(store) => KvStoreRef::Account(store.clone()),
        CatalogStoreRef::Team(store) => KvStoreRef::Team(store.clone()),
    }
}

pub(super) fn download_to_path(
    transport: &dyn foks_desktop::AgentTransport,
    item: &CatalogItem,
    destination: &Path,
) -> Result<(), AgentError> {
    let store = protocol_store(&item.store);
    let value = transport
        .call(Operation::ReadKv {
            store: store.clone(),
            path: item.metadata.path.clone(),
            version: item.metadata.version,
        })
        .map_err(AgentError::from_desktop)?;
    let mut read: KvReadResult = serde_json::from_value(value)
        .map_err(|error| AgentError::new("invalid-response", error.to_string(), false))?;
    // The size is deliberately not bound. A catalog entry never carries one,
    // because FOKS does not expose a plaintext size in node metadata, and a
    // large-file read no longer reports one either, so comparing them refuses
    // every download. The store, path, version, node type and both roles are
    // what identify the entry; a small file's assembled length is still
    // checked against the size that read reports for it.
    let metadata_matches = read.store == store
        && read.path == item.metadata.path
        && read.version == item.metadata.version
        && read.node_type == item.metadata.node_type
        && read.read_role == item.metadata.read_role
        && read.write_role == item.metadata.write_role;
    if !metadata_matches {
        if let Some(content) = &mut read.content {
            zeroize::Zeroize::zeroize(content);
        }
        if let Some(target) = &mut read.symlink_target {
            zeroize::Zeroize::zeroize(target);
        }
        return Err(AgentError::new(
            "response-binding",
            "The agent returned data for a different item.",
            true,
        ));
    }
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
            let mut content = read.content.take().ok_or_else(|| {
                AgentError::new(
                    "invalid-response",
                    "The agent response is missing file content.",
                    false,
                )
            })?;
            if Some(content.len() as u64) != read.size {
                zeroize::Zeroize::zeroize(&mut content);
                return Err(AgentError::new(
                    "invalid-response",
                    "The received file length does not match its catalog size.",
                    false,
                ));
            }
            temporary.write_all(&content).map_err(|error| {
                AgentError::new(
                    "download-write",
                    format!("Could not save the file: {error}"),
                    true,
                )
            })?;
            zeroize::Zeroize::zeroize(&mut content);
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
            // A large file reports no size: the agent would have to download
            // and decrypt the whole file to measure one, doubling the bytes
            // this loop is about to move. The end-of-file flag ends the loop,
            // and `MAXIMUM_DOWNLOAD_BYTES` bounds it against an agent that
            // never sets that flag.
            let mut offset = 0u64;
            loop {
                let length = DOWNLOAD_CHUNK_BYTES;
                let value = transport
                    .call(Operation::ReadKvChunk {
                        store: store.clone(),
                        path: item.metadata.path.clone(),
                        version: item.metadata.version,
                        offset,
                        length,
                    })
                    .map_err(AgentError::from_desktop)?;
                let mut chunk: KvChunkResult = serde_json::from_value(value).map_err(|error| {
                    AgentError::new("invalid-response", error.to_string(), false)
                })?;
                let next = offset
                    .checked_add(chunk.content.len() as u64)
                    .ok_or_else(|| {
                        AgentError::new(
                            "invalid-response",
                            "File download chunk offset calculation overflowed.",
                            false,
                        )
                    })?;
                let valid = chunk.store == store
                    && chunk.path == item.metadata.path
                    && chunk.version == item.metadata.version
                    && chunk.offset == offset
                    && !chunk.content.is_empty()
                    && chunk.content.len() <= length as usize
                    && next <= MAXIMUM_DOWNLOAD_BYTES;
                if !valid {
                    zeroize::Zeroize::zeroize(&mut chunk.content);
                    return Err(AgentError::new(
                        "response-binding",
                        "The agent returned an invalid file chunk.",
                        true,
                    ));
                }
                let eof = chunk.eof;
                let write = temporary.write_all(&chunk.content);
                zeroize::Zeroize::zeroize(&mut chunk.content);
                write.map_err(|error| {
                    AgentError::new(
                        "download-write",
                        format!("Could not save the file: {error}"),
                        true,
                    )
                })?;
                offset = next;
                if eof {
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
    temporary.as_file().sync_all().map_err(|error| {
        AgentError::new(
            "download-write",
            format!("Could not finish saving the file: {error}"),
            true,
        )
    })?;
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
        | Operation::PutKvSymlink {
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

pub(super) fn remove_item_operation(item: &CatalogItem) -> Result<Operation, AgentError> {
    // Folder deletion is not supported.
    if item.metadata.node_type == "directory" {
        return Err(invalid_request(
            "Folder removal is not currently supported.",
        ));
    }
    foks_desktop::remove_kv_operation(item, false).map_err(invalid_request)
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
    result
        .map(|_| ())
        .map_err(|error| map_mutation_error(error, kind))
}

async fn apply_file_upload(
    state: &AppState,
    header: KvUploadHeader,
    source: PathBuf,
    kind: MutationKind,
) -> Result<MutationDto, AgentError> {
    state.invalidate_catalog_items(&super::execution::catalog_store(&header.store));
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        upload_file(transport.as_ref(), header, &source, kind)
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            state,
            format!("The file import stopped before reporting its outcome: {error}"),
        )
    })?;
    if result.as_ref().is_err_and(|error| error.ambiguous) {
        state
            .mutation_requires_refresh
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
                configured_probe: profile.probe.clone(),
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
    let transport = state.agent.transport();
    let worker_state = state.clone();
    let worker_app = app.clone();
    let (snapshot, profiles) = tauri::async_runtime::spawn_blocking(move || {
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
                        crate::applock::require_unlocked_generation(&worker_app, access)?;
                        let mut sent = Ok(());
                        if !worker_state.publish_catalog_snapshot(
                            generation,
                            snapshot.clone(),
                            |published, accepted| {
                                sent = (|| {
                                    let dto = catalog_dto(accepted, published, Some(&profiles))?;
                                    crate::applock::require_unlocked_generation(
                                        &worker_app,
                                        access,
                                    )?;
                                    channel
                                        .send(dto)
                                        .map_err(|error| AgentError::unknown(error.to_string()))
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
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
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
            foks_desktop::load_catalog_cancellable_with_profiles(transport, names, token, fresh)
                .map_err(AgentError::from_desktop)
        } else {
            foks_desktop::load_stores_cancellable(transport, token)
                .map_err(AgentError::from_desktop)
        }?;
        Ok::<_, AgentError>((snapshot, profiles))
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Failed to load vault catalog: {error}")))??;
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
    state: State<'_, AppState>,
) -> Result<CatalogDto, AgentError> {
    require_main_window(&webview)?;
    load_catalog(&state, webview.app_handle(), false, false, None).await
}

#[tauri::command]
pub async fn list_catalog(
    webview: tauri::Webview,
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
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
    let transport = state
        .agent
        .transport_for("list_profile_catalog", Some(&profile));
    let (snapshot, profiles) = tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Failed to load profile catalog: {error}")))??;
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
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<ReadItemDto, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_item(&store_id, &path, version)?;
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || read_text(transport.as_ref(), &item))
        .await
        .map_err(|error| AgentError::unknown(format!("failed to read item: {error}")))?
}

#[tauri::command]
pub async fn copy_item_value(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<CommandAck, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_item(&store_id, &path, version)?;
    let transport = state.agent.transport();
    let value = tauri::async_runtime::spawn_blocking(move || {
        read_text(transport.as_ref(), &item).map(|read| read.value)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("failed to read item: {error}")))??;
    crate::clipboard::copy_with_hygiene(&app, value)
        .map_err(|error| AgentError::new("clipboard", error, true))?;
    Ok(CommandAck { ok: true })
}

#[tauri::command]
pub fn copy_item_path(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<CommandAck, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_item(&store_id, &path, version)?;
    crate::clipboard::copy_with_hygiene(&app, Zeroizing::new(item.metadata.path))
        .map_err(|error| AgentError::new("clipboard", error, true))?;
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
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<DownloadResult, AgentError> {
    crate::applock::require_unlocked(&app)?;
    require_main_window(&webview)?;
    let item = state.selected_item(&store_id, &path, version)?;
    let suggested = item
        .metadata
        .path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("download")
        .to_owned();
    let picker_app = app.clone();
    let destination = tauri::async_runtime::spawn_blocking(move || {
        picker_app
            .dialog()
            .file()
            .set_file_name(suggested)
            .blocking_save_file()
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Save dialog failed: {error}")))?;
    let Some(destination) = destination else {
        return Ok(DownloadResult { saved: false });
    };
    let destination = destination
        .into_path()
        .map_err(|error| AgentError::new("download-path", error.to_string(), false))?;
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        download_to_path(transport.as_ref(), &item, &destination)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("download failed: {error}")))??;
    Ok(DownloadResult { saved: true })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_text_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
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
#[allow(clippy::too_many_arguments)]
pub async fn create_link(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    target: String,
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
    let target = Zeroizing::new(target);
    let operation = foks_desktop::create_kv_symlink_operation(&store, &path, target.as_str())
        .map_err(invalid_request)?;
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
pub async fn create_folder(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
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
pub async fn edit_text_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
    value: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = state.selected_mutation_item(&store_id, &path, version)?;
    require_text_item(&item)?;
    let mutation = foks_desktop::edit_kv_file_mutation(&item, take_text_value(value)?)
        .map_err(invalid_request)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_kv_mutation(&state, mutation, MutationKind::Guarded).await
}

#[tauri::command]
pub async fn remove_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = state.selected_mutation_item(&store_id, &path, version)?;
    let operation = remove_item_operation(&item)?;
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
    state: State<'_, AppState>,
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
    // Validate the destination file header before consuming the staged drop path.
    let header = file_create_header(
        &store,
        &path,
        0,
        read_role.as_deref(),
        write_role.as_deref(),
    )?;
    let source_path = Zeroizing::new(source_path);
    let source = state.take_drop_path(source_path.as_str())?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_file_upload(&state, header, source, MutationKind::Create).await
}

#[tauri::command]
pub async fn pick_and_import_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
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
    let header = file_create_header(
        &store,
        &path,
        0,
        read_role.as_deref(),
        write_role.as_deref(),
    )?;
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
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_file_upload(&state, header, source, MutationKind::Create).await
}

#[tauri::command]
pub async fn replace_dropped_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
    source_path: String,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = state.selected_mutation_item(&store_id, &path, version)?;
    require_file_item(&item)?;
    let header = file_edit_header(&item, 0)?;
    let source_path = Zeroizing::new(source_path);
    let source = state.take_drop_path(source_path.as_str())?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_file_upload(&state, header, source, MutationKind::Guarded).await
}

#[tauri::command]
pub async fn pick_and_replace_file(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = state.selected_mutation_item(&store_id, &path, version)?;
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
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_file_upload(&state, header, source, MutationKind::Guarded).await
}
