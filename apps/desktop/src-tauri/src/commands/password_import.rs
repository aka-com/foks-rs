//! Importing logins from a CSV file that another password manager exported.
//!
//! The host opens the file through the native picker, parses it and holds
//! the parsed logins under an opaque id until the renderer commits or
//! discards them, another file replaces them, or the app locks. The renderer
//! receives the id, the file's name and, for each login, the name and folder
//! it will take. It never receives a user name, password, website or note,
//! or the file's path.
//!
//! Columns are found by their header, which covers the CSV exports of
//! 1Password, Bitwarden, KeePass, KeePassXC, LastPass and the Chrome, Edge,
//! Firefox and Safari password managers. Each login is written as one small
//! file in the layout the New password sheet writes, `user:`, `password:`
//! and `url:` lines, followed by one `note:` line for each line of its notes.
//! Every line is then a field line, so the agent's listing classifies the
//! item as a login (`foks_client::kv::login`). A login longer than 2,040
//! bytes is stored as a file node, which the desktop lists as a Document.
//! A login's name is made unique within its folder and never equals a
//! folder that other logins of the file are written in. Each write is a
//! `Create`, so an occupied path is reported and left unchanged, and running
//! the same import again skips the logins it saved.

use crate::agent::AgentError;
use crate::commands::context::AppState;
use crate::commands::execution::{execute_kv_mutation, run_kv_writes, MutationKind};
use crate::commands::preparation::{check_mutation_access, prepare_catalog_mutation};
use crate::commands::trash::in_trash;
use crate::commands::types::CommandAck;
use crate::commands::validation::{invalid_request, require_main_window, MAXIMUM_TEXT_ITEM_BYTES};
use crate::commands::vault::{create_item_roles, set_create_mutation_roles};
use foks_agent_proto::KvRole;
use foks_desktop::CatalogStoreRef;
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::Read as _;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::{Manager as _, State};
use tauri_plugin_dialog::DialogExt as _;
use zeroize::Zeroizing;

/// The largest file an import reads.
pub(super) const MAXIMUM_IMPORT_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// The most logins one file may hold.
pub(super) const MAXIMUM_IMPORT_ENTRIES: usize = 10_000;
/// The longest name or folder an import gives, in bytes after encoding,
/// which leaves room in the 255-byte component limit for a ` (n)` suffix.
const MAXIMUM_NAME_BYTES: usize = 240;

/// What a column of the file holds, by its header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Column {
    Title,
    Username,
    Password,
    Url,
    Notes,
    Folder,
    /// Bitwarden's item type: `login`, `note`, `card`, `identity` or `sshkey`.
    Type,
}

fn column(header: &str) -> Option<Column> {
    Some(match header.trim().to_ascii_lowercase().as_str() {
        "title" | "name" | "account" => Column::Title,
        "username" | "user name" | "login name" | "login_username" | "user" => Column::Username,
        "password" | "login_password" => Column::Password,
        "url" | "website" | "web site" | "login_uri" | "uri" => Column::Url,
        "notes" | "note" | "comments" | "extra" => Column::Notes,
        "folder" | "group" | "grouping" => Column::Folder,
        "type" => Column::Type,
        _ => return None,
    })
}

/// One login read from the file, ready to write.
#[derive(Clone)]
pub(super) struct ImportEntry {
    /// The folders below the destination it is written in, each an encoded
    /// path component.
    pub(super) folder: Vec<String>,
    /// Its encoded path component.
    pub(super) name: String,
    /// The text of the small file.
    pub(super) content: Zeroizing<String>,
}

/// A row of the file that is not imported, and why.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkippedRowDto {
    /// The row's number in the file, counting the header as row 1.
    pub row: usize,
    /// The row's title, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub reason: String,
}

/// What a file holds: the logins to write, the rows left out, and the
/// headers of the columns that are not read.
pub(super) struct ParsedImport {
    pub(super) entries: Vec<ImportEntry>,
    pub(super) skipped: Vec<SkippedRowDto>,
    pub(super) ignored_columns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEntryDto {
    pub name: String,
    /// The folders below the destination, `/`-separated, or empty.
    pub folder: String,
    /// Whether the login is longer than a small file holds, so that it is
    /// stored as a file and listed as a Document.
    pub document: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordImportPreviewDto {
    pub id: String,
    pub file_name: String,
    pub entries: Vec<ImportEntryDto>,
    pub skipped: Vec<SkippedRowDto>,
    pub ignored_columns: Vec<String>,
}

/// The outcome of one login of a commit: `created`; `exists` when the path
/// was occupied; `failed` when the write certainly did not apply;
/// `uncertain` when its outcome is unknown; `not-started` when an earlier
/// failure stopped the run before it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ImportOutcomeDto {
    pub index: u32,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn file_error(message: impl Into<String>) -> AgentError {
    AgentError::new("import-file", message, false)
}

/// Splits CSV text into records of fields, as RFC 4180 describes: fields
/// are separated by `delimiter` and records by `\n` or `\r\n`, and a field
/// in double quotes may hold the delimiter, line breaks and `""` for one
/// quote. Each field is copied once into a buffer of its final size, so no
/// plaintext is left behind in a reallocated one.
pub(super) fn csv_records(
    text: &str,
    delimiter: u8,
) -> Result<Vec<Vec<Zeroizing<String>>>, AgentError> {
    let bytes = text.as_bytes();
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let field = if bytes[index] == b'"' {
            let start = index + 1;
            let mut end = start;
            loop {
                let Some(offset) = bytes[end..].iter().position(|byte| *byte == b'"') else {
                    return Err(file_error(format!(
                        "Row {} has a quoted value that does not end.",
                        records.len() + 1
                    )));
                };
                end += offset;
                if bytes.get(end + 1) == Some(&b'"') {
                    end += 2;
                } else {
                    break;
                }
            }
            let quoted = &text[start..end];
            let mut field = Zeroizing::new(String::with_capacity(quoted.len()));
            for (position, piece) in quoted.split("\"\"").enumerate() {
                if position > 0 {
                    field.push('"');
                }
                field.push_str(piece);
            }
            index = end + 1;
            field
        } else {
            let end = bytes[index..]
                .iter()
                .position(|byte| *byte == delimiter || *byte == b'\n' || *byte == b'\r')
                .map_or(bytes.len(), |offset| index + offset);
            let field = Zeroizing::new(text[index..end].to_owned());
            index = end;
            field
        };
        record.push(field);
        match bytes.get(index) {
            None => {}
            Some(byte) if *byte == delimiter => {
                index += 1;
                // A delimiter at the very end leaves one more empty field.
                if index == bytes.len() {
                    record.push(Zeroizing::new(String::new()));
                }
                continue;
            }
            Some(b'\r') if bytes.get(index + 1) == Some(&b'\n') => index += 2,
            Some(b'\n' | b'\r') => index += 1,
            Some(_) => {
                return Err(file_error(format!(
                    "Row {} has text after a closing quote.",
                    records.len() + 1
                )))
            }
        }
        records.push(std::mem::take(&mut record));
    }
    if !record.is_empty() {
        records.push(record);
    }
    Ok(records)
}

/// The delimiter of a file whose first line is `header`: a comma, unless
/// that line has none and has semicolons or tabs instead.
fn delimiter(header: &str) -> u8 {
    let line = header.split('\n').next().unwrap_or_default();
    if line.contains(',') {
        b','
    } else if line.contains(';') {
        b';'
    } else if line.contains('\t') {
        b'\t'
    } else {
        b','
    }
}

/// `text` as one path component: whitespace control characters become
/// spaces, other control characters are dropped, `%` and `/` are
/// percent-encoded as the Files list decodes them, and the result is
/// trimmed and cut to [`MAXIMUM_NAME_BYTES`]. `None` when nothing usable is
/// left.
pub(super) fn path_component(text: &str) -> Option<String> {
    let mut component = String::new();
    for character in text.trim().chars() {
        let piece = match character {
            '%' => "%25".to_owned(),
            '/' => "%2F".to_owned(),
            '\t' | '\n' | '\r' => " ".to_owned(),
            character if character.is_control() => continue,
            character => character.to_string(),
        };
        if component.len() + piece.len() > MAXIMUM_NAME_BYTES {
            break;
        }
        component.push_str(&piece);
    }
    let component = component.trim().to_owned();
    (!component.is_empty() && component != "." && component != "..").then_some(component)
}

/// The name a login takes: its title, else its website's host, else its
/// user name, else `Login`.
fn entry_name(title: &str, url: &str, username: &str) -> String {
    let host = || {
        let url = url.trim();
        url::Url::parse(url)
            .ok()
            .or_else(|| url::Url::parse(&format!("https://{url}")).ok())
            .and_then(|parsed| parsed.host_str().map(str::to_owned))
    };
    path_component(title)
        .or_else(|| host().and_then(|host| path_component(&host)))
        .or_else(|| path_component(username))
        .unwrap_or_else(|| "Login".to_owned())
}

/// The website of a login and the other addresses its URL column lists.
/// Bitwarden writes every URI of a login into `login_uri`, joined by commas;
/// the value is split only when each part is an absolute URL, so a single
/// address with a comma in it is kept whole.
fn website_and_others(value: &str) -> (&str, Vec<&str>) {
    let value = value.trim();
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    if parts.len() > 1 && parts.iter().all(|part| url::Url::parse(part).is_ok()) {
        (parts[0], parts[1..].to_vec())
    } else {
        (value, Vec::new())
    }
}

/// The folders a login's folder or group value names below the destination.
/// KeePassXC names its top group `Root` in every row, so a first `Root` of a
/// `Group` column is dropped.
fn entry_folder(value: &str, keepass_group: bool) -> Vec<String> {
    let mut folders: Vec<String> = value.split('/').filter_map(path_component).collect();
    if keepass_group && folders.first().map(String::as_str) == Some("Root") {
        folders.remove(0);
    }
    folders
}

/// The small file a login is stored as: the three lines the New password
/// sheet writes, then a `note:` line for each line of `notes`, which may end
/// in `\n`, `\r\n` or `\r`.
pub(super) fn login_text(
    username: &str,
    password: &str,
    url: &str,
    notes: &str,
) -> Zeroizing<String> {
    let notes = notes.trim_end_matches(['\n', '\r']);
    let note_lines: Vec<&str> = if notes.trim().is_empty() {
        Vec::new()
    } else {
        notes
            .split('\n')
            .flat_map(|line| line.strip_suffix('\r').unwrap_or(line).split('\r'))
            .collect()
    };
    let capacity = 26
        + username.len()
        + password.len()
        + url.len()
        + note_lines.iter().map(|line| line.len() + 7).sum::<usize>();
    let mut text = Zeroizing::new(String::with_capacity(capacity));
    for (key, value) in [("user", username), ("password", password), ("url", url)] {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(key);
        text.push_str(": ");
        text.push_str(value);
    }
    for line in note_lines {
        text.push_str("\nnote:");
        if !line.is_empty() {
            text.push(' ');
            text.push_str(line);
        }
    }
    text
}

/// Reads a CSV export into the logins it holds. The text must be UTF-8, with
/// or without a byte order mark, and its first row must be a header with a
/// password column.
pub(super) fn parse_export(bytes: &[u8]) -> Result<ParsedImport, AgentError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        file_error("This file is not UTF-8 text. Export it again as a UTF-8 CSV file.")
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut records = csv_records(text, delimiter(text))?.into_iter();
    let header = records
        .next()
        .ok_or_else(|| file_error("This file is empty."))?;
    let mut columns: Vec<Option<Column>> = Vec::with_capacity(header.len());
    let mut ignored_columns = Vec::new();
    for name in &header {
        let found = column(name).filter(|found| !columns.contains(&Some(*found)));
        if found.is_none() && !name.trim().is_empty() {
            ignored_columns.push(name.trim().to_owned());
        }
        columns.push(found);
    }
    if !columns.contains(&Some(Column::Password)) {
        return Err(file_error(
            "This file has no password column. Choose a CSV export from 1Password, Bitwarden, KeePass, LastPass or a browser.",
        ));
    }
    let keepass_group = header
        .iter()
        .zip(&columns)
        .any(|(name, found)| *found == Some(Column::Folder) && name.trim() == "Group");
    // Each login's folders, name and text, before names are made unique.
    let mut rows: Vec<(Vec<String>, String, Zeroizing<String>)> = Vec::new();
    // Every folder a login is written in, and each folder above it.
    let mut folders: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut skipped = Vec::new();
    for (position, record) in records.enumerate() {
        let row = position + 2;
        let value = |wanted: Column| -> &str {
            columns
                .iter()
                .position(|found| *found == Some(wanted))
                .and_then(|at| record.get(at))
                .map_or("", |field| field.as_str())
        };
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        let title = value(Column::Title).trim();
        let skip = |reason: String| SkippedRowDto {
            row,
            name: (!title.is_empty()).then(|| title.to_owned()),
            reason,
        };
        let kind = value(Column::Type).trim();
        if !kind.is_empty() && !kind.eq_ignore_ascii_case("login") {
            skipped.push(skip(format!(
                "A {} entry, not a login.",
                kind.to_ascii_lowercase()
            )));
            continue;
        }
        let (username, password, url) = (
            value(Column::Username),
            value(Column::Password),
            value(Column::Url),
        );
        if username.trim().is_empty() && password.is_empty() && url.trim().is_empty() {
            skipped.push(skip("It has no user name, password or website.".to_owned()));
            continue;
        }
        let broken = [
            ("password", password),
            ("user name", username),
            ("website", url),
        ]
        .into_iter()
        .find(|(_, value)| value.contains(['\n', '\r']));
        if let Some((field, _)) = broken {
            skipped.push(skip(format!(
                "Its {field} contains a line break, which a login field cannot hold."
            )));
            continue;
        }
        // LastPass exports each secure note as a row whose website is
        // `http://sn`, with the note's text in its notes column.
        if url.trim() == "http://sn" && username.trim().is_empty() && password.is_empty() {
            skipped.push(skip("A secure note, not a login.".to_owned()));
            continue;
        }
        if rows.len() == MAXIMUM_IMPORT_ENTRIES {
            return Err(file_error(format!(
                "This file holds more than {MAXIMUM_IMPORT_ENTRIES} logins. Split it and import each part."
            )));
        }
        let folder = entry_folder(value(Column::Folder), keepass_group);
        for depth in 1..=folder.len() {
            folders.insert(folder[..depth].to_vec());
        }
        let (url, other_urls) = website_and_others(url);
        let notes = value(Column::Notes);
        let content = if other_urls.is_empty() {
            login_text(username.trim(), password, url, notes)
        } else {
            let mut lines = other_urls;
            lines.push(notes);
            login_text(
                username.trim(),
                password,
                url,
                &Zeroizing::new(lines.join("\n")),
            )
        };
        rows.push((folder, entry_name(title, url, username), content));
    }
    // A login does not take the name of a folder that other logins are
    // written in, which would put a file where they need a folder, nor a
    // name another login of its folder took.
    let mut taken = BTreeSet::new();
    let mut entries = Vec::with_capacity(rows.len());
    for (folder, base, content) in rows {
        let mut path = folder;
        path.push(base.clone());
        let mut copy = 1;
        while folders.contains(&path) || taken.contains(&path) {
            copy += 1;
            if let Some(name) = path.last_mut() {
                *name = format!("{base} ({copy})");
            }
        }
        taken.insert(path.clone());
        let name = path.pop().unwrap_or(base);
        entries.push(ImportEntry {
            folder: path,
            name,
            content,
        });
    }
    Ok(ParsedImport {
        entries,
        skipped,
        ignored_columns,
    })
}

/// Reads the file at `path`, refusing one larger than
/// [`MAXIMUM_IMPORT_FILE_BYTES`].
fn read_export_file(path: &Path) -> Result<Zeroizing<Vec<u8>>, AgentError> {
    let unreadable =
        |error: std::io::Error| file_error(format!("Could not read this file: {error}"));
    let file = std::fs::File::open(path).map_err(unreadable)?;
    let length = file.metadata().map_err(unreadable)?.len();
    let too_large = || {
        file_error(format!(
            "This file is larger than {} MB. Split it and import each part.",
            MAXIMUM_IMPORT_FILE_BYTES / (1024 * 1024)
        ))
    };
    if length > MAXIMUM_IMPORT_FILE_BYTES {
        return Err(too_large());
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(length as usize + 1));
    file.take(MAXIMUM_IMPORT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    if bytes.len() as u64 > MAXIMUM_IMPORT_FILE_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

/// A parsed file held for the renderer's commit.
struct HeldImport {
    id: String,
    /// The unlocked interval the file was read in.
    generation: u64,
    entries: Vec<ImportEntry>,
}

/// The one import the host holds at a time.
#[derive(Default)]
pub struct PasswordImports(Mutex<Option<HeldImport>>);

impl PasswordImports {
    fn held(&self) -> std::sync::MutexGuard<'_, Option<HeldImport>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Holds `parsed`, replacing any import held before, and describes it.
    pub(super) fn hold(
        &self,
        generation: u64,
        file_name: String,
        parsed: ParsedImport,
    ) -> Result<PasswordImportPreviewDto, AgentError> {
        let mut id = [0u8; 16];
        getrandom::fill(&mut id)
            .map_err(|_| AgentError::unknown("No system randomness for the import."))?;
        let id: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
        let entries = parsed
            .entries
            .iter()
            .map(|entry| ImportEntryDto {
                name: entry.name.clone(),
                folder: entry.folder.join("/"),
                document: entry.content.len() > MAXIMUM_TEXT_ITEM_BYTES,
            })
            .collect();
        *self.held() = Some(HeldImport {
            id: id.clone(),
            generation,
            entries: parsed.entries,
        });
        Ok(PasswordImportPreviewDto {
            id,
            file_name,
            entries,
            skipped: parsed.skipped,
            ignored_columns: parsed.ignored_columns,
        })
    }

    /// Copies the entries at `indexes` out of the import `id`, which must
    /// have been read in the unlocked interval `generation`.
    pub(super) fn selected(
        &self,
        id: &str,
        generation: u64,
        indexes: &[u32],
    ) -> Result<Vec<(u32, ImportEntry)>, AgentError> {
        let held = self.held();
        let Some(held) = held
            .as_ref()
            .filter(|held| held.id == id && held.generation == generation)
        else {
            return Err(AgentError::new(
                "import-expired",
                "This import is no longer held. Choose the file again.",
                false,
            ));
        };
        if indexes.is_empty() {
            return Err(invalid_request("Choose at least one login to import."));
        }
        let mut seen = BTreeSet::new();
        indexes
            .iter()
            .map(|&index| {
                let entry = held
                    .entries
                    .get(index as usize)
                    .filter(|_| seen.insert(index))
                    .ok_or_else(|| invalid_request("Choose logins this import holds."))?;
                Ok((index, entry.clone()))
            })
            .collect()
    }

    /// Drops the import `id`, or any import without one.
    pub(super) fn discard(&self, id: Option<&str>) {
        let mut held = self.held();
        if id.is_none_or(|id| held.as_ref().is_some_and(|import| import.id == id)) {
            *held = None;
        }
    }
}

/// Drops the held import, for the app lock.
pub(crate) fn discard_held(app: &tauri::AppHandle) {
    if let Some(imports) = app.try_state::<PasswordImports>() {
        imports.discard(None);
    }
}

/// Where a commit writes: a folder of a store, with the roles each login
/// takes.
#[derive(Debug)]
pub(super) struct ImportTarget {
    pub(super) store: CatalogStoreRef,
    pub(super) folder: String,
    pub(super) read_role: KvRole,
    pub(super) write_role: KvRole,
}

/// `folder` when it is an absolute folder path outside Trash: `/`, or
/// components that are not empty, `.` or `..`, with no trailing slash.
pub(super) fn destination_folder(folder: String) -> Result<String, AgentError> {
    let valid = folder == "/"
        || (folder.starts_with('/')
            && folder[1..]
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."));
    if !valid || folder.len() > 1024 {
        return Err(invalid_request("Enter an absolute folder path."));
    }
    if in_trash(&folder) {
        return Err(invalid_request("Logins cannot be imported into Trash."));
    }
    Ok(folder)
}

/// The path `entry` is written at below `folder`.
pub(super) fn import_path(folder: &str, entry: &ImportEntry) -> String {
    let mut path = if folder == "/" {
        String::new()
    } else {
        folder.to_owned()
    };
    for component in entry.folder.iter().chain(std::iter::once(&entry.name)) {
        path.push('/');
        path.push_str(component);
    }
    path
}

fn write_entry(
    transport: &dyn foks_desktop::AgentTransport,
    target: &ImportTarget,
    mut entry: ImportEntry,
) -> Result<(), AgentError> {
    let path = import_path(&target.folder, &entry);
    let content = std::mem::take(&mut *entry.content).into_bytes();
    let mutation = foks_desktop::create_kv_file_mutation(&target.store, &path, content)
        .map_err(invalid_request)?;
    let mutation = set_create_mutation_roles(mutation, target.read_role, target.write_role)?;
    execute_kv_mutation(transport, mutation, MutationKind::Create)
}

/// Writes `entries` one at a time and reports each outcome to `report` as
/// it is known. An occupied path is reported and the run goes on; any other
/// failure, including `permitted` refusing because the app locked, ends the
/// run, and the entries after it are reported as not started.
pub(super) fn write_entries(
    transport: &dyn foks_desktop::AgentTransport,
    target: &ImportTarget,
    entries: Vec<(u32, ImportEntry)>,
    permitted: &dyn Fn() -> Result<(), AgentError>,
    report: &dyn Fn(&ImportOutcomeDto),
) -> Vec<ImportOutcomeDto> {
    let mut outcomes = Vec::with_capacity(entries.len());
    let mut stopped: Option<&'static str> = None;
    for (index, entry) in entries {
        let outcome = |status, message: Option<String>| ImportOutcomeDto {
            index,
            status,
            message,
        };
        let outcome = match stopped {
            Some(reason) => outcome("not-started", Some(reason.to_owned())),
            None => match permitted().and_then(|()| write_entry(transport, target, entry)) {
                Ok(()) => outcome("created", None),
                Err(error) if error.code == "already-exists" => outcome(
                    "exists",
                    Some("Something with this name is already in the vault.".to_owned()),
                ),
                Err(error) if error.ambiguous => {
                    stopped = Some("Not started: the outcome of an earlier login is unknown.");
                    outcome("uncertain", Some(error.message))
                }
                Err(error) => {
                    stopped = Some("Not started: an earlier login failed.");
                    outcome("failed", Some(error.message))
                }
            },
        };
        report(&outcome);
        outcomes.push(outcome);
    }
    outcomes
}

/// Opens a CSV export through the native picker and holds the logins it
/// holds. Resolves with `None` when the picker is cancelled.
#[tauri::command]
pub async fn begin_password_import(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    imports: State<'_, PasswordImports>,
) -> Result<Option<PasswordImportPreviewDto>, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let picker_app = app.clone();
    let parsed = tauri::async_runtime::spawn_blocking(move || {
        let Some(source) = picker_app
            .dialog()
            .file()
            .set_title("Choose a CSV export of your passwords")
            .add_filter("CSV", &["csv", "CSV"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let source = source
            .into_path()
            .map_err(|error| AgentError::new("upload-source", error.to_string(), false))?;
        let file_name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = read_export_file(&source)?;
        Ok::<_, AgentError>(Some((file_name, parse_export(&bytes)?)))
    })
    .await
    .map_err(|error| AgentError::unknown(format!("The import stopped: {error}")))??;
    let Some((file_name, parsed)) = parsed else {
        return Ok(None);
    };
    // The app may have locked while the picker was open.
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    imports.hold(unlocked, file_name, parsed).map(Some)
}

/// Writes the held logins at `entries` into `folder` of a store, with the
/// given roles in a team, and reports each outcome over `on_progress` as
/// well as in the result. The import stays held, so a failed or unknown
/// login can be sent again: its `Create` cannot replace one that was saved.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn commit_password_import(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    imports: State<'_, PasswordImports>,
    import_id: String,
    entries: Vec<u32>,
    store_id: String,
    folder: String,
    read_role: Option<String>,
    write_role: Option<String>,
    on_progress: tauri::ipc::Channel<ImportOutcomeDto>,
) -> Result<Vec<ImportOutcomeDto>, AgentError> {
    let unlocked = crate::applock::unlocked_generation(&app)?;
    require_main_window(&webview)?;
    let folder = destination_folder(folder)?;
    let selected = imports.selected(&import_id, unlocked, &entries)?;
    let state = state.for_store(&store_id)?;
    let _permit = prepare_catalog_mutation(&state).await?;
    let store = state.selected_create_store(&store_id)?;
    let (read_role, write_role) =
        create_item_roles(&store, read_role.as_deref(), write_role.as_deref())?;
    check_mutation_access(unlocked, crate::applock::unlocked_generation(&app))?;
    let target = ImportTarget {
        store: store.clone(),
        folder,
        read_role,
        write_role,
    };
    let transport = state.agent.transport();
    let outcomes = run_kv_writes(&state, Some(&store), move || {
        let permitted =
            || check_mutation_access(unlocked, crate::applock::unlocked_generation(&app));
        Ok(write_entries(
            transport.as_ref(),
            &target,
            selected,
            &permitted,
            &|outcome| {
                let _ = on_progress.send(outcome.clone());
            },
        ))
    })
    .await?;
    if outcomes.iter().any(|outcome| outcome.status == "uncertain") {
        state
            .mutation_requires_refresh()
            .store(true, Ordering::Release);
    }
    Ok(outcomes)
}

/// Drops the held import `import_id`.
#[tauri::command]
pub fn discard_password_import(
    webview: tauri::Webview,
    imports: State<'_, PasswordImports>,
    import_id: String,
) -> Result<CommandAck, AgentError> {
    require_main_window(&webview)?;
    imports.discard(Some(&import_id));
    Ok(CommandAck { ok: true })
}
