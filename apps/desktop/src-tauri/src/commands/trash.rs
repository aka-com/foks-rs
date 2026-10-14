//! Vault Trash: deletion as a move into `/.trash` within the same store,
//! restoration as a move back, and permanent removal only inside Trash.
//!
//! Dirent names are sealed under the key of the directory that holds them,
//! so one shared Trash folder would expose names to readers of the wrong
//! role. Each read role gets its own folder, `/.trash/<role-tag>/`, created
//! with that read role, and an entry moved into it keeps its own roles. A
//! folder is filed by its own directory read role, since the names of its
//! children stay sealed under its key. A role folder is used only when its
//! read role matches; otherwise the next free `/.trash/<role-tag>.<n>` is.
//!
//! An entry is named `<UTC yyyymmddThhmmss>~<percent-encoded original path>`.
//! When that name would exceed the 255-byte component limit, it is
//! `<stamp>~#<hash>` instead and a small file named `<that name>.origin`
//! beside it holds the original path. Go FOKS clients list `.trash` as an
//! ordinary folder; nothing here changes the wire or storage format.

use crate::agent::AgentError;
use crate::commands::context::AppState;
use crate::commands::execution::{
    ambiguous_worker_failure, execute_kv_mutation, kv_mutation_store, MutationKind,
};
use crate::commands::preparation::{check_mutation_access, prepare_catalog_mutation};
use crate::commands::validation::{invalid_request, require_main_window};
use foks_agent_proto::{KvPrecondition, KvRole, Operation};
use foks_desktop::{CatalogItem, CatalogStoreRef, KvAccountMutation};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

/// The folder that holds every role's Trash folder in a store.
pub(super) const TRASH_ROOT: &str = "/.trash";

/// The longest dirent name the KV protocol accepts, in bytes.
const MAXIMUM_NAME_BYTES: usize = 255;

/// How many `/.trash/<role-tag>.<n>` names are tried past one held by an
/// item with other roles.
const MAXIMUM_ROLE_FOLDER_ATTEMPTS: u32 = 8;

/// The suffix of the small file that records a hashed entry's original path.
const ORIGIN_SUFFIX: &str = ".origin";

/// Team roots are readable through this low-visibility Member key, which both
/// team kinds provision. `/.trash` uses it for reading and writing so that
/// every member who can delete an item can also add the Trash folder for that
/// item's read role. The role folders it holds name only roles.
const TEAM_TRASH_ROOT_ROLE: KvRole = KvRole::Member {
    visibility: -0x4000,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TrashMutationDto {
    pub applied: bool,
    /// Where the entry is now: its Trash path, or the restored path.
    pub path: String,
}

/// One write of a Trash plan, with the context a failure is reported in.
#[derive(Debug, PartialEq)]
pub(super) struct TrashStep {
    pub(super) operation: Operation,
    pub(super) kind: MutationKind,
    pub(super) failure: Option<&'static str>,
}

/// The writes that carry out one Trash command, in order, and the path the
/// selected entry ends at.
#[derive(Debug, PartialEq)]
pub(super) struct TrashPlan {
    pub(super) steps: Vec<TrashStep>,
    pub(super) path: String,
}

/// Whether `path` is the Trash root or lies under it.
pub(super) fn in_trash(path: &str) -> bool {
    path == TRASH_ROOT
        || path
            .strip_prefix(TRASH_ROOT)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The folder name for one read role: `owner`, `admin` or `member-<visibility>`.
pub(super) fn role_tag(role: KvRole) -> String {
    match role {
        KvRole::Owner => "owner".to_owned(),
        KvRole::Admin => "admin".to_owned(),
        KvRole::Member { visibility } => format!("member-{visibility}"),
    }
}

/// `yyyymmddThhmmss` in UTC for a count of seconds since the Unix epoch.
pub(super) fn utc_stamp(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX / 2);
    let rest = seconds % 86_400;
    // Civil date from a day count (H. Hinnant, "chrono-Compatible Low-Level
    // Date Algorithms").
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// Encodes `%`, `/` and ASCII control bytes, leaving every other character
/// readable in a listing.
pub(super) fn encode_original_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len() + 8);
    for character in path.chars() {
        if character == '%' || character == '/' || character.is_ascii_control() {
            encoded.push_str(&format!("%{:02X}", u32::from(character)));
        } else {
            encoded.push(character);
        }
    }
    encoded
}

pub(super) fn decode_original_path(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = encoded.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// The name an entry takes in Trash, and the name of the small file that
/// records its original path when the name itself cannot.
pub(super) fn trash_entry_name(stamp: &str, original: &str) -> (String, Option<String>) {
    let name = format!("{stamp}~{}", encode_original_path(original));
    if name.len() <= MAXIMUM_NAME_BYTES {
        return (name, None);
    }
    let digest = Sha256::digest(original.as_bytes());
    let hash: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let name = format!("{stamp}~#{hash}");
    let origin = format!("{name}{ORIGIN_SUFFIX}");
    (name, Some(origin))
}

/// What a Trash entry's name records.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum TrashName {
    /// The original path is in the name.
    Path { stamp: String, original: String },
    /// The original path is in the `.origin` file beside the entry.
    Hashed { stamp: String },
    /// The small file that holds a hashed entry's original path.
    Origin,
}

pub(super) fn parse_trash_name(name: &str) -> Option<TrashName> {
    let (stamp, rest) = name.split_once('~')?;
    let valid_stamp = stamp.len() == 15
        && stamp.as_bytes()[8] == b'T'
        && stamp
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 8 || byte.is_ascii_digit());
    if !valid_stamp {
        return None;
    }
    let hashed = |hash: &str| hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
    if let Some(hash) = rest.strip_prefix('#') {
        if hashed(hash) {
            return Some(TrashName::Hashed {
                stamp: stamp.to_owned(),
            });
        }
        return hash
            .strip_suffix(ORIGIN_SUFFIX)
            .filter(|hash| hashed(hash))
            .map(|_| TrashName::Origin);
    }
    let original = decode_original_path(rest)?;
    original.starts_with('/').then(|| TrashName::Path {
        stamp: stamp.to_owned(),
        original,
    })
}

/// The name of an entry that sits directly in a role's Trash folder, or
/// `None` for any other path.
fn trash_entry_component(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(TRASH_ROOT)?.strip_prefix('/')?;
    let (tag, name) = rest.split_once('/')?;
    (!tag.is_empty() && !name.is_empty() && !name.contains('/')).then_some(name)
}

fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(index) => &path[..index],
    }
}

fn kv_store(store: &CatalogStoreRef) -> foks_agent_proto::KvStoreRef {
    match store {
        CatalogStoreRef::Account(store) => foks_agent_proto::KvStoreRef::Account(store.clone()),
        CatalogStoreRef::Team(store) => foks_agent_proto::KvStoreRef::Team(store.clone()),
    }
}

fn mkdir_step(
    store: &CatalogStoreRef,
    path: &str,
    read_role: KvRole,
    write_role: KvRole,
    mkdir_p: bool,
    failure: &'static str,
) -> TrashStep {
    TrashStep {
        operation: Operation::MkdirKv {
            store: kv_store(store),
            path: path.to_owned(),
            read_role,
            write_role,
            precondition: KvPrecondition::Create,
            mkdir_p,
        },
        kind: MutationKind::Create,
        failure: Some(failure),
    }
}

fn remove_step(item: &CatalogItem) -> TrashStep {
    TrashStep {
        operation: Operation::RemoveKv {
            expected_dirent_id: Some(item.metadata.dirent_id),
            store: kv_store(&item.store),
            path: item.metadata.path.clone(),
            recursive: item.metadata.node_type == "directory",
            precondition: KvPrecondition::ExactVersion {
                version: item.metadata.version,
            },
        },
        kind: MutationKind::Guarded,
        failure: None,
    }
}

fn move_step(item: &CatalogItem, destination: &str) -> Result<TrashStep, AgentError> {
    Ok(TrashStep {
        operation: foks_desktop::move_kv_operation(item, destination).map_err(invalid_request)?,
        kind: MutationKind::Guarded,
        failure: None,
    })
}

fn find<'a>(store_items: &'a [CatalogItem], path: &str) -> Option<&'a CatalogItem> {
    store_items.iter().find(|item| item.metadata.path == path)
}

/// The Trash folder for entries of `read_role`: `/.trash/<role-tag>`, or the
/// first free `/.trash/<role-tag>.<n>` when another item holds that name with
/// other roles. Moving an entry into a folder of another read role would seal
/// its name under that role's key.
fn role_folder(store_items: &[CatalogItem], read_role: KvRole) -> Result<String, AgentError> {
    let base = format!("{TRASH_ROOT}/{}", role_tag(read_role));
    for attempt in 1..=MAXIMUM_ROLE_FOLDER_ATTEMPTS {
        let folder = if attempt == 1 {
            base.clone()
        } else {
            format!("{base}.{attempt}")
        };
        match find(store_items, &folder) {
            None => return Ok(folder),
            Some(existing)
                if existing.metadata.node_type == "directory"
                    && existing.metadata.read_role == read_role =>
            {
                return Ok(folder);
            }
            Some(_) => {}
        }
    }
    Err(invalid_request(format!(
        "Folders named {base} in Trash have other read access, so this item cannot be moved there."
    )))
}

/// Plans moving `item` into its read role's Trash folder. `store_items` is
/// the catalog of the item's store, which locates and checks the Trash
/// folders the move needs before any write is sent.
pub(super) fn trash_plan(
    store_items: &[CatalogItem],
    item: &CatalogItem,
    seconds: u64,
) -> Result<TrashPlan, AgentError> {
    let path = item.metadata.path.as_str();
    if in_trash(path) {
        return Err(invalid_request(
            "This item is already in Trash. Delete it permanently or restore it.",
        ));
    }
    let read_role = item.metadata.read_role;
    let mut steps = Vec::new();
    match find(store_items, TRASH_ROOT) {
        None => {
            let root_role = match item.store {
                CatalogStoreRef::Account(_) => KvRole::Owner,
                CatalogStoreRef::Team(_) => TEAM_TRASH_ROOT_ROLE,
            };
            steps.push(mkdir_step(
                &item.store,
                TRASH_ROOT,
                root_role,
                root_role,
                false,
                "Trash could not be created at the top level of this vault. In a team, only an owner can create it; an owner who moves any item to Trash creates it for everyone.",
            ));
        }
        Some(existing) if existing.metadata.node_type != "directory" => {
            return Err(invalid_request(
                "An item named .trash at the top level of this vault is not a folder, so Trash cannot use it. Rename or move that item first.",
            ));
        }
        Some(_) => {}
    }
    let folder = role_folder(store_items, read_role)?;
    if find(store_items, &folder).is_none() {
        steps.push(mkdir_step(
            &item.store,
            &folder,
            read_role,
            read_role,
            false,
            "The Trash folder for this item's read access could not be created.",
        ));
    }
    let (name, origin) = trash_entry_name(&utc_stamp(seconds), path);
    let destination = format!("{folder}/{name}");
    if find(store_items, &destination).is_some() {
        return Err(AgentError::new(
            "conflict",
            "An entry with the same name is already in Trash. Try again in a second.",
            false,
        ));
    }
    if let Some(origin) = origin {
        // Written before the move so the entry is never in Trash without it.
        steps.push(TrashStep {
            operation: Operation::PutKv {
                store: kv_store(&item.store),
                path: format!("{folder}/{origin}"),
                content: path.as_bytes().to_vec(),
                read_role,
                write_role: item.metadata.write_role,
                precondition: KvPrecondition::Create,
                mkdir_p: false,
            },
            kind: MutationKind::Create,
            failure: Some(
                "The record of this item's original location could not be saved in Trash.",
            ),
        });
    }
    steps.push(move_step(item, &destination)?);
    Ok(TrashPlan {
        steps,
        path: destination,
    })
}

/// Plans moving a Trash entry to `destination`, outside Trash. A missing
/// destination folder is created with the entry's own roles, and a hashed
/// entry's `.origin` file is removed once the entry has moved.
pub(super) fn restore_plan(
    store_items: &[CatalogItem],
    entry: &CatalogItem,
    destination: &str,
) -> Result<TrashPlan, AgentError> {
    let name = trash_entry_component(&entry.metadata.path)
        .ok_or_else(|| invalid_request("Only entries in Trash can be restored."))?;
    // An entry another client placed in Trash under any other name can be
    // restored too; only an `.origin` file belongs to its entry.
    let hashed = match parse_trash_name(name) {
        Some(TrashName::Hashed { .. }) => true,
        Some(TrashName::Path { .. }) | None => false,
        Some(TrashName::Origin) => {
            return Err(invalid_request(
                "This file records where a Trash entry came from and is restored with that entry.",
            ));
        }
    };
    if !destination.starts_with('/') || destination == "/" || destination.ends_with('/') {
        return Err(invalid_request(
            "Enter an absolute path below the vault root.",
        ));
    }
    if in_trash(destination) {
        return Err(invalid_request(
            "Choose a location outside Trash to restore to.",
        ));
    }
    if find(store_items, destination).is_some() {
        return Err(AgentError::new(
            "already-exists",
            format!("Another item is already at {destination}."),
            false,
        ));
    }
    let mut steps = Vec::new();
    let parent = parent_of(destination);
    if parent != "/" {
        match find(store_items, parent) {
            Some(existing) if existing.metadata.node_type != "directory" => {
                return Err(invalid_request(format!(
                    "{parent} is not a folder, so nothing can be restored into it."
                )));
            }
            Some(_) => {}
            None => steps.push(mkdir_step(
                &entry.store,
                parent,
                entry.metadata.read_role,
                entry.metadata.write_role,
                true,
                "The folder to restore into could not be created.",
            )),
        }
    }
    steps.push(move_step(entry, destination)?);
    if hashed {
        if let Some(origin) = find(
            store_items,
            &format!("{}{ORIGIN_SUFFIX}", entry.metadata.path),
        ) {
            steps.push(remove_step(origin));
        }
    }
    Ok(TrashPlan {
        steps,
        path: destination.to_owned(),
    })
}

/// Plans permanently removing an entry in Trash: recursively for a folder,
/// together with the `.origin` file of a hashed entry. Items outside Trash
/// are refused; they are moved to Trash instead.
pub(super) fn removal_plan(
    store_items: &[CatalogItem],
    item: &CatalogItem,
) -> Result<TrashPlan, AgentError> {
    let path = item.metadata.path.as_str();
    if !path
        .strip_prefix(TRASH_ROOT)
        .is_some_and(|rest| rest.starts_with('/'))
    {
        return Err(invalid_request(
            "Only items in Trash can be deleted permanently. Move this item to Trash first.",
        ));
    }
    let mut steps = vec![remove_step(item)];
    if let Some(TrashName::Hashed { .. }) = trash_entry_component(path).and_then(parse_trash_name) {
        if let Some(origin) = find(store_items, &format!("{path}{ORIGIN_SUFFIX}")) {
            steps.push(remove_step(origin));
        }
    }
    Ok(TrashPlan {
        steps,
        path: path.to_owned(),
    })
}

/// Sends a plan's writes in order and stops at the first failure. Earlier
/// writes stay applied: an empty Trash folder or an `.origin` file without
/// its entry is harmless, and the catalog the caller retired shows both.
pub(super) fn execute_trash_plan(
    transport: &dyn foks_desktop::AgentTransport,
    steps: Vec<TrashStep>,
) -> Result<(), AgentError> {
    for step in steps {
        execute_kv_mutation(
            transport,
            KvAccountMutation::Inline(step.operation),
            step.kind,
        )
        .map_err(|mut error| {
            if let Some(context) = step.failure {
                if !error.ambiguous {
                    error.message = format!("{context} {}", error.message);
                }
            }
            error
        })?;
    }
    Ok(())
}

async fn apply_trash_plan(
    state: &AppState,
    plan: TrashPlan,
) -> Result<TrashMutationDto, AgentError> {
    match plan
        .steps
        .first()
        .and_then(|step| kv_mutation_store(&KvAccountMutation::Inline(step.operation.clone())))
    {
        Some(store) => state.invalidate_catalog_items(&store),
        None => state.invalidate_catalog(),
    }
    let transport = state.agent.transport();
    let TrashPlan { steps, path } = plan;
    let result =
        tauri::async_runtime::spawn_blocking(move || execute_trash_plan(transport.as_ref(), steps))
            .await
            .map_err(|error| {
                ambiguous_worker_failure(
                    state,
                    format!(
                        "The background operation stopped before reporting its outcome: {error}"
                    ),
                )
            })?;
    match result {
        Ok(()) => Ok(TrashMutationDto {
            applied: true,
            path,
        }),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

fn store_items(state: &AppState, store: &CatalogStoreRef) -> Result<Vec<CatalogItem>, AgentError> {
    let (_, catalog) = state.catalog_at(None)?;
    let catalog = catalog.ok_or_else(|| {
        AgentError::new(
            "catalog-required",
            "Refresh the vault before changing Trash.",
            true,
        )
    })?;
    Ok(catalog
        .items
        .into_iter()
        .filter(|item| &item.store == store)
        .collect())
}

fn now_seconds() -> Result<u64, AgentError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|_| AgentError::unknown("The system clock is set before 1970."))
}

/// Moves an item, or a folder with everything in it, into Trash.
#[tauri::command]
pub async fn trash_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
) -> Result<TrashMutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = state.selected_bound_mutation_item(&store_id, &path, version, &dirent_id)?;
    let plan = trash_plan(&store_items(&state, &item.store)?, &item, now_seconds()?)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_trash_plan(&state, plan).await
}

/// Moves a Trash entry back out of Trash, to its original path or another.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn restore_trash_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
    destination: String,
) -> Result<TrashMutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let entry = state.selected_bound_mutation_item(&store_id, &path, version, &dirent_id)?;
    let plan = restore_plan(&store_items(&state, &entry.store)?, &entry, &destination)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_trash_plan(&state, plan).await
}

/// Permanently removes an entry in Trash. Earlier encrypted versions stay on
/// the server, which keeps every dirent version.
#[tauri::command]
pub async fn remove_item(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    store_id: String,
    path: String,
    version: u64,
    dirent_id: String,
) -> Result<crate::commands::types::MutationDto, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let item = state.selected_bound_mutation_item(&store_id, &path, version, &dirent_id)?;
    let plan = removal_plan(&store_items(&state, &item.store)?, &item)?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    apply_trash_plan(&state, plan)
        .await
        .map(|_| crate::commands::types::MutationDto { applied: true })
}
