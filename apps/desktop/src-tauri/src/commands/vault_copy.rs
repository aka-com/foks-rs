//! Copying an item into another vault: a personal secret into a team, or a
//! file from one team to another, on the same server or a different one.
//!
//! The host reads the source at the version and dirent the catalog listed
//! and creates a new item in the destination store with a `Create`
//! precondition and the roles the renderer chose. No content crosses IPC to
//! the renderer, and none is written to disk in plaintext. The source is not
//! changed; a renderer that moves the item sends it to Trash afterwards as a
//! separate command.
//!
//! A large file is read whole before the upload starts. The agent admits an
//! upload exclusively per profile, so chunk reads from the same profile would
//! wait behind the upload that consumes them. Until the upload reads them
//! back, the chunks are held in an unlinked temporary file, each sealed with
//! ChaCha20-Poly1305 under a key that exists only in this process's memory,
//! with the chunk's sequence number as its nonce.

use crate::agent::AgentError;
use crate::commands::context::AppState;
use crate::commands::execution::{
    execute_kv_mutation, map_mutation_error, run_kv_writes, MutationKind,
};
use crate::commands::preparation::{check_mutation_access, prepare_catalog_mutation};
use crate::commands::types::MutationDto;
use crate::commands::validation::{invalid_request, require_main_window};
use crate::commands::vault::{
    create_item_roles, read_file_content, set_create_mutation_roles, set_create_operation_roles,
};
use chacha20poly1305::aead::{Aead as _, KeyInit as _};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use foks_agent_proto::KvRole;
use foks_desktop::{CatalogItem, CatalogStoreRef, KvAccountMutation, KvItemValue};
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use tauri::State;
use zeroize::Zeroizing;

/// The Poly1305 tag each sealed chunk carries after its ciphertext.
const TAG_BYTES: usize = 16;

/// Where a copy lands: a path in another store, with the roles it takes.
#[derive(Debug)]
pub(super) struct CopyTarget {
    pub(super) store: CatalogStoreRef,
    pub(super) path: String,
    pub(super) read_role: KvRole,
    pub(super) write_role: KvRole,
}

/// A source item's content, read and held for the write that follows.
pub(super) enum CopySource {
    /// A small file's bytes, written back inline.
    File(Zeroizing<Vec<u8>>),
    /// A link's target.
    Link(Zeroizing<String>),
    /// A large file's chunks, sealed in a temporary file.
    Spooled(SealedSpool),
}

/// Chunks sealed into an unlinked temporary file, in order, under a key that
/// is held only here and zeroized with it.
pub(super) struct SealedSpool {
    file: File,
    key: Zeroizing<[u8; 32]>,
    /// The plaintext length of each chunk, in order.
    lengths: Vec<usize>,
    total: u64,
}

fn spool_error(detail: impl std::fmt::Display) -> AgentError {
    AgentError::new(
        "copy-buffer",
        format!("Could not hold the file while copying it: {detail}"),
        true,
    )
}

/// The nonce of chunk `index`: its sequence number, little-endian. Each
/// spool has its own key, so no nonce repeats under one key.
fn chunk_nonce(index: usize) -> Nonce {
    let mut nonce = [0u8; 12];
    nonce[..8].copy_from_slice(&(index as u64).to_le_bytes());
    Nonce::from(nonce)
}

impl SealedSpool {
    pub(super) fn new() -> Result<Self, AgentError> {
        let mut key = Zeroizing::new([0u8; 32]);
        getrandom::fill(key.as_mut()).map_err(|_| spool_error("no system randomness"))?;
        Ok(Self {
            file: tempfile::tempfile().map_err(spool_error)?,
            key,
            lengths: Vec::new(),
            total: 0,
        })
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new((&*self.key).into())
    }

    /// Seals `chunk` and appends it.
    pub(super) fn append(&mut self, chunk: &[u8]) -> Result<(), AgentError> {
        let sealed = self
            .cipher()
            .encrypt(&chunk_nonce(self.lengths.len()), chunk)
            .map_err(|_| spool_error("sealing failed"))?;
        self.file.write_all(&sealed).map_err(spool_error)?;
        self.lengths.push(chunk.len());
        self.total += chunk.len() as u64;
        Ok(())
    }

    /// The number of plaintext bytes held.
    pub(super) fn len(&self) -> u64 {
        self.total
    }

    /// Reads the chunks back in order, opening each as it is reached.
    pub(super) fn into_reader(mut self) -> Result<SpoolReader, AgentError> {
        self.file.seek(SeekFrom::Start(0)).map_err(spool_error)?;
        Ok(SpoolReader {
            spool: self,
            next: 0,
            chunk: Zeroizing::new(Vec::new()),
            position: 0,
            failure: None,
        })
    }
}

#[cfg(test)]
impl SealedSpool {
    /// The temporary file, for tests that inspect or alter what is held.
    pub(super) fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

/// Reads a [`SealedSpool`]'s plaintext back. A chunk that fails to open
/// ends the read with an error, which is kept for the caller to report.
pub(super) struct SpoolReader {
    spool: SealedSpool,
    next: usize,
    chunk: Zeroizing<Vec<u8>>,
    position: usize,
    pub(super) failure: Option<String>,
}

impl SpoolReader {
    fn open_next(&mut self) -> std::io::Result<bool> {
        let Some(&length) = self.spool.lengths.get(self.next) else {
            return Ok(false);
        };
        let mut sealed = vec![0u8; length + TAG_BYTES];
        self.spool.file.read_exact(&mut sealed)?;
        let opened = self
            .spool
            .cipher()
            .decrypt(&chunk_nonce(self.next), sealed.as_slice())
            .map_err(|_| std::io::Error::other("a held chunk failed authentication"))?;
        self.chunk = Zeroizing::new(opened);
        self.position = 0;
        self.next += 1;
        Ok(true)
    }
}

impl std::io::Read for SpoolReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        while self.position == self.chunk.len() {
            match self.open_next() {
                Ok(true) => {}
                Ok(false) => return Ok(0),
                Err(error) => {
                    self.failure = Some(error.to_string());
                    return Err(error);
                }
            }
        }
        let count = buffer.len().min(self.chunk.len() - self.position);
        buffer[..count].copy_from_slice(&self.chunk[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

/// Reads the content a copy of `item` needs. A secret or small file is held
/// in memory and a link's target likewise; a large file is sealed into a
/// [`SealedSpool`]. Folders are refused.
pub(super) fn read_copy_source(
    transport: &dyn foks_desktop::AgentTransport,
    item: &CatalogItem,
) -> Result<CopySource, AgentError> {
    match item.metadata.node_type.as_str() {
        "small-file" => {
            let mut content = Zeroizing::new(Vec::new());
            read_file_content(transport, item, &mut |piece| {
                content.extend_from_slice(piece);
                Ok(())
            })?;
            Ok(CopySource::File(content))
        }
        "file" => {
            let mut spool = SealedSpool::new()?;
            read_file_content(transport, item, &mut |piece| spool.append(piece))?;
            Ok(CopySource::Spooled(spool))
        }
        "symlink" => match foks_desktop::read_catalog_item(transport, item)
            .map_err(AgentError::from_desktop)?
            .value
        {
            KvItemValue::Symlink(target) => Ok(CopySource::Link(target)),
            _ => Err(AgentError::new(
                "invalid-response",
                "The agent returned file content for a link.",
                false,
            )),
        },
        "directory" => Err(invalid_request(
            "Folders cannot be copied to another vault. Copy the items in it one at a time.",
        )),
        _ => Err(invalid_request("This item cannot be copied.")),
    }
}

/// Creates the copy at `target`. The write is one `Create`, so it fails with
/// `already-exists` rather than replacing an item at that path.
pub(super) fn write_copy(
    transport: &dyn foks_desktop::AgentTransport,
    source: CopySource,
    target: &CopyTarget,
) -> Result<(), AgentError> {
    match source {
        CopySource::File(mut content) => {
            let mutation = foks_desktop::create_kv_file_mutation(
                &target.store,
                &target.path,
                std::mem::take(&mut *content),
            )
            .map_err(invalid_request)?;
            let mutation =
                set_create_mutation_roles(mutation, target.read_role, target.write_role)?;
            execute_kv_mutation(transport, mutation, MutationKind::Create)
        }
        CopySource::Link(link) => {
            let operation = foks_desktop::create_kv_symlink_operation(
                &target.store,
                &target.path,
                link.as_str(),
            )
            .map_err(invalid_request)?;
            let operation =
                set_create_operation_roles(operation, target.read_role, target.write_role)?;
            execute_kv_mutation(
                transport,
                KvAccountMutation::Inline(operation),
                MutationKind::Create,
            )
        }
        CopySource::Spooled(spool) => {
            let mut header =
                foks_desktop::create_kv_file_upload(&target.store, &target.path, spool.len())
                    .map_err(invalid_request)?;
            header.read_role = target.read_role;
            header.write_role = target.write_role;
            let mut reader = spool.into_reader()?;
            let result = transport.put_kv_stream(header, &mut reader);
            if let Some(detail) = reader.failure {
                return Err(spool_error(detail));
            }
            result
                .map(|_| ())
                .map_err(|error| map_mutation_error(error, MutationKind::Create))
        }
    }
}

/// The profile a store identifier names, or `None` when it names none.
fn store_profile(id: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(id)
        .ok()?
        .get("profile")?
        .as_str()
        .map(str::to_owned)
}

/// The mutation scope of a copy: the profile both stores belong to, or the
/// whole application when they belong to different profiles, so that the
/// reservation covers the profile read from as well as the one written to.
pub(super) fn copy_scope(
    state: &AppState,
    source: &str,
    target: &str,
) -> Result<AppState, AgentError> {
    match (store_profile(source), store_profile(target)) {
        (Some(source_profile), Some(target_profile)) if source_profile == target_profile => {
            state.for_profile(&source_profile)
        }
        (Some(_), Some(_)) => Ok(state.clone()),
        _ => Err(invalid_request("Select a valid vault store.")),
    }
}

/// The catalog item a copy reads: the listed version of the listed dirent.
pub(super) fn selected_copy_source(
    state: &AppState,
    store: &str,
    path: &str,
    version: u64,
    dirent_id: &str,
) -> Result<CatalogItem, AgentError> {
    let stale = || {
        AgentError::new(
            "conflict",
            "This item changed or was removed. Refresh the vault before copying it.",
            false,
        )
    };
    let item =
        state
            .selected_item(store, path, version)
            .map_err(|error| match error.code.as_str() {
                "item-version-mismatch" | "item-not-found" => stale(),
                _ => error,
            })?;
    let listed: String = item
        .metadata
        .dirent_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if listed != dirent_id {
        return Err(stale());
    }
    Ok(item)
}

/// Copies a secret, file or link into another vault at `target_path`, with
/// the given roles in a team. The source is read at the catalog's version
/// and left unchanged.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn copy_item_to_store(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
    target_store_id: String,
    target_path: String,
    read_role: Option<String>,
    write_role: Option<String>,
) -> Result<MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = copy_scope(&state, &store_id, &target_store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = selected_copy_source(&state, &store_id, &path, version, &dirent_id)?;
    let store = state.selected_create_store(&target_store_id)?;
    if store == item.store {
        return Err(invalid_request(
            "Choose another vault. Rename or move changes where an item is within its vault.",
        ));
    }
    let (read_role, write_role) =
        create_item_roles(&store, read_role.as_deref(), write_role.as_deref())?;
    // Validate the destination before reading anything.
    foks_desktop::create_kv_file_upload(&store, &target_path, 0).map_err(invalid_request)?;
    let target = CopyTarget {
        store,
        path: target_path,
        read_role,
        write_role,
    };
    let transport = state.agent.transport();
    let source =
        tauri::async_runtime::spawn_blocking(move || read_copy_source(transport.as_ref(), &item))
            .await
            .map_err(|error| {
                AgentError::unknown(format!("Failed to read the item to copy: {error}"))
            })??;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    let transport = state.agent.transport();
    let store = target.store.clone();
    run_kv_writes(&state, Some(&store), move || {
        write_copy(transport.as_ref(), source, &target)
    })
    .await
    .map(|()| MutationDto { applied: true })
}
