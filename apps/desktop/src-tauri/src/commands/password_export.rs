//! Exporting a vault's logins to a CSV file.
//!
//! The host reads each login of a store at the version the catalog listed
//! and writes the file it names through the native save dialog; the
//! renderer receives only how many logins were written. The file has the
//! columns of a Bitwarden export, which the desktop's import reads back and
//! other password managers accept: `folder`, `name`, `login_uri`,
//! `login_username`, `login_password`, and the login's other lines as
//! `notes`. Like every password manager's CSV export it is plain text; the
//! renderer asks for a typed confirmation first, and the file is created
//! through a temporary file that is readable only by its owner on Unix.
//!
//! A login is what the Files list shows as a Password: a small file the
//! agent's listing classified as a login, or, where it reported no shape, a
//! small file under `/logins/`. Documents, files and Trash are not exported.

use crate::agent::AgentError;
use crate::applock::UnlockedInterval;
use crate::commands::context::AppState;
use crate::commands::trash::{decode_original_path, in_trash};
use crate::commands::validation::{invalid_request, require_main_window};
use crate::commands::vault::{read_file_content, select_download_destination};
use foks_agent_proto::KvEntryShape;
use foks_desktop::{CatalogItem, CatalogStoreReadState};
use serde::Serialize;
use std::path::Path;
use tauri_plugin_dialog::DialogExt as _;
use zeroize::Zeroizing;

/// The header row: the columns of a Bitwarden CSV export.
pub(super) const EXPORT_HEADER: &str =
    "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LoginExportDto {
    /// False when the save dialog was cancelled.
    pub saved: bool,
    pub exported: usize,
    /// Logins left out because their text is not UTF-8.
    pub skipped: usize,
}

/// Whether `item` is a login the export writes.
pub(super) fn is_exported_login(item: &CatalogItem) -> bool {
    let path = item.metadata.path.as_str();
    item.metadata.node_type == "small-file"
        && !in_trash(path)
        && match item.metadata.shape {
            Some(shape) => shape == KvEntryShape::Login,
            None => path.starts_with("/logins/"),
        }
}

/// One stored path component as a person reads it: percent-decoded as the
/// Files list decodes it, or as stored when it does not decode. A folder
/// keeps its stored form when decoding would give it a `/`.
fn readable_component(component: &str, folder: bool) -> String {
    match decode_original_path(component) {
        Some(decoded) if !(folder && decoded.contains('/')) => decoded,
        _ => component.to_owned(),
    }
}

/// The folder and name columns of the login at `path`. Folders below
/// `/logins` are written relative to it, so that an import into `/logins`
/// puts each login back in its folder.
pub(super) fn export_location(path: &str) -> (String, String) {
    let mut components: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let name = components.pop().unwrap_or_default();
    if components.first() == Some(&"logins") {
        components.remove(0);
    }
    let folder = components
        .iter()
        .map(|component| readable_component(component, true))
        .collect::<Vec<_>>()
        .join("/");
    (folder, readable_component(name, false))
}

/// The key of a field line, as `foks_client::kv::login` reads one, with
/// its value.
fn field(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    (!key.is_empty()
        && !key.contains(|c: char| c.is_ascii_whitespace())
        && !value.starts_with("//"))
    .then(|| (key, value.strip_prefix(' ').unwrap_or(value)))
}

/// The user name, password and website of a login's text, the fields Copy
/// finds, and its other lines as notes: the value of each `note:` line
/// the import writes, and every other line as it is. Copy reads the first
/// line naming a field, so only that line becomes its column; a later line
/// naming the same field, such as an earlier password kept below the
/// current one, stays in the notes.
pub(super) fn login_columns(
    text: &str,
) -> (
    Zeroizing<String>,
    Zeroizing<String>,
    Zeroizing<String>,
    Zeroizing<String>,
) {
    use foks_client_app::{login_field, LoginField};
    let take = |wanted| Zeroizing::new(login_field(text, wanted).unwrap_or_default().to_owned());
    let mut notes = Zeroizing::new(String::with_capacity(text.len()));
    // The user name, password and website lines already passed.
    let mut passed = [false; 3];
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let column = match field(line).map(|(key, _)| key) {
            Some("user" | "username") => Some(0),
            Some("password") => Some(1),
            Some("url" | "website") => Some(2),
            _ => None,
        };
        if let Some(at) = column.filter(|at| !passed[*at]) {
            passed[at] = true;
            continue;
        }
        let note = match field(line) {
            Some(("note", value)) => value,
            _ => line,
        };
        if !notes.is_empty() {
            notes.push('\n');
        }
        notes.push_str(note);
    }
    let trimmed = notes.trim_end_matches('\n').len();
    notes.truncate(trimmed);
    (
        take(LoginField::Username),
        take(LoginField::Password),
        take(LoginField::Url),
        notes,
    )
}

/// One CSV record: every field in double quotes, with `"` doubled, and a
/// `\r\n` line ending.
pub(super) fn csv_line(fields: &[&str]) -> Zeroizing<String> {
    let capacity = fields
        .iter()
        .map(|field| 2 * field.len() + 3)
        .sum::<usize>()
        + 2;
    let mut line = Zeroizing::new(String::with_capacity(capacity));
    for (position, field) in fields.iter().enumerate() {
        if position > 0 {
            line.push(',');
        }
        line.push('"');
        for (index, piece) in field.split('"').enumerate() {
            if index > 0 {
                line.push_str("\"\"");
            }
            line.push_str(piece);
        }
        line.push('"');
    }
    line.push_str("\r\n");
    line
}

/// The CSV record of the login at `path` with `content`, or `None` when the
/// content is not UTF-8 text.
pub(super) fn export_record(path: &str, content: &[u8]) -> Option<Zeroizing<String>> {
    let text = std::str::from_utf8(content).ok()?;
    let (folder, name) = export_location(path);
    let (username, password, url, notes) = login_columns(text);
    Some(csv_line(&[
        &folder, "", "login", &name, &notes, "", "0", &url, &username, &password, "",
    ]))
}

/// Reads each of `items` inside `interval` and before `deadline`, and writes
/// its record to `sink` after the header. Returns how many were written and
/// how many were left out because their text is not UTF-8.
pub(super) fn write_export(
    transport: &dyn foks_desktop::AgentTransport,
    items: &[CatalogItem],
    interval: &UnlockedInterval,
    deadline: &foks_local_deadline::Deadline,
    sink: &mut dyn std::io::Write,
) -> Result<(usize, usize), AgentError> {
    let write_error = |error: std::io::Error| {
        AgentError::new(
            "export-write",
            format!("Could not write the export: {error}"),
            true,
        )
    };
    sink.write_all(EXPORT_HEADER.as_bytes())
        .and_then(|()| sink.write_all(b"\r\n"))
        .map_err(write_error)?;
    let (mut exported, mut skipped) = (0, 0);
    for item in items {
        let mut content = Zeroizing::new(Vec::new());
        read_file_content(transport, item, interval, deadline, &mut |piece| {
            content.extend_from_slice(piece);
            Ok(())
        })?;
        match export_record(&item.metadata.path, &content) {
            Some(record) => {
                sink.write_all(record.as_bytes()).map_err(write_error)?;
                exported += 1;
            }
            None => skipped += 1,
        }
    }
    Ok((exported, skipped))
}

/// The logins of the store `id`, in path order, from a catalog that read
/// all of that store's items.
fn store_logins(state: &AppState, id: &str) -> Result<Vec<CatalogItem>, AgentError> {
    let (store, _) = state.selected_store(id)?;
    let (_, catalog) = state.catalog_at(None)?;
    let refresh = || {
        AgentError::new(
            "catalog-required",
            "Refresh this vault before exporting its passwords.",
            true,
        )
    };
    let catalog = catalog.ok_or_else(refresh)?;
    if !catalog
        .store_reads
        .iter()
        .any(|read| read.store == store && read.state == CatalogStoreReadState::Complete)
    {
        return Err(refresh());
    }
    let mut logins: Vec<CatalogItem> = catalog
        .items
        .into_iter()
        .filter(|item| item.store == store && is_exported_login(item))
        .collect();
    logins.sort_by(|a, b| a.metadata.path.cmp(&b.metadata.path));
    Ok(logins)
}

/// Writes the export to `destination` through a temporary file beside it,
/// which replaces the destination only once every login is written. Only the
/// replacement runs inside the interval's publication gate. Any failure after
/// the interval ended, including creating or syncing the temporary file, is
/// reported as `app-locked`.
pub(super) fn export_to_path(
    transport: &dyn foks_desktop::AgentTransport,
    items: &[CatalogItem],
    interval: &UnlockedInterval,
    deadline: &foks_local_deadline::Deadline,
    destination: &Path,
) -> Result<(usize, usize), AgentError> {
    export_in_interval(transport, items, interval, deadline, destination).or_else(|error| {
        interval.check()?;
        Err(error)
    })
}

fn export_in_interval(
    transport: &dyn foks_desktop::AgentTransport,
    items: &[CatalogItem],
    interval: &UnlockedInterval,
    deadline: &foks_local_deadline::Deadline,
    destination: &Path,
) -> Result<(usize, usize), AgentError> {
    let write_error = |error: std::io::Error| {
        AgentError::new(
            "export-write",
            format!("Could not write the export: {error}"),
            true,
        )
    };
    let parent = destination.parent().ok_or_else(|| {
        AgentError::new(
            "export-write",
            "The chosen file has no parent folder.",
            false,
        )
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(write_error)?;
    let counts = write_export(
        transport,
        items,
        interval,
        deadline,
        temporary.as_file_mut(),
    )?;
    temporary.as_file().sync_all().map_err(write_error)?;
    // NamedTempFile removes the unpublished plaintext on every rejected path.
    interval.publish(|| {
        deadline.check().map_err(super::deadline::clock_error)?;
        temporary
            .persist(destination)
            .map_err(|error| write_error(error.error))
    })?;
    Ok(counts)
}

/// Writes the logins of a store to a CSV file chosen in the native save
/// dialog.
#[tauri::command]
pub async fn export_logins(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: super::deadline::CommandState,
    store_id: String,
) -> Result<LoginExportDto, AgentError> {
    let interval = crate::applock::unlocked_interval(&app)?;
    require_main_window(&webview)?;
    let logins = store_logins(&state, &store_id)?;
    if logins.is_empty() {
        return Err(invalid_request("This vault has no passwords to export."));
    }
    let picker_app = app.clone();
    let picker_interval = interval.clone();
    let destination = tauri::async_runtime::spawn_blocking(move || {
        select_download_destination(&picker_interval, || {
            picker_app
                .dialog()
                .file()
                .set_title("Save passwords as plain text")
                .set_file_name("passwords.csv")
                .add_filter("CSV", &["csv"])
                .blocking_save_file()
        })
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Save dialog failed: {error}")))??;
    let Some(destination) = destination else {
        return Ok(LoginExportDto {
            saved: false,
            exported: 0,
            skipped: 0,
        });
    };
    let destination = destination
        .into_path()
        .map_err(|error| AgentError::new("export-write", error.to_string(), false))?;
    // The dialog's wait is not part of the export's budget.
    let state = state.begin_operation(foks_agent_proto::DeadlineClass::Bulk)?;
    let transport = state.transport()?;
    let deadline = state.deadline()?.clone();
    let worker_deadline = deadline.clone();
    let (exported, skipped) = super::deadline::run(
        &deadline,
        false,
        move || {
            export_to_path(
                transport.as_ref(),
                &logins,
                &interval,
                &worker_deadline,
                &destination,
            )
        },
        |_| {},
    )
    .await?;
    Ok(LoginExportDto {
        saved: true,
        exported,
        skipped,
    })
}
