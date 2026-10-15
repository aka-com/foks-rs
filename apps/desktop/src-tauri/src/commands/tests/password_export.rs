use super::support::account_ref;
use crate::agent::AgentError;
use crate::commands::password_export::{
    csv_line, export_location, export_record, is_exported_login, login_columns, write_export,
    EXPORT_HEADER,
};
use crate::commands::password_import::{csv_records, parse_export};
use foks_agent_proto::{KvEntryMetadata, KvEntryShape, KvReadResult, KvRole, Operation};
use foks_desktop::{CatalogItem, CatalogStoreRef};
use std::collections::BTreeMap;
use std::sync::Mutex;

fn item(path: &str, node_type: &str, shape: Option<KvEntryShape>) -> CatalogItem {
    CatalogItem {
        store: CatalogStoreRef::Account(account_ref("foks.example", "personal")),
        metadata: KvEntryMetadata {
            dirent_id: [3; 16],
            path: path.to_owned(),
            node_type: node_type.to_owned(),
            version: 2,
            size: None,
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            shape,
        },
    }
}

#[test]
fn only_what_the_files_list_shows_as_a_password_is_exported() {
    let login = Some(KvEntryShape::Login);
    let other = Some(KvEntryShape::Other);
    assert!(is_exported_login(&item("/bank", "small-file", login)));
    assert!(is_exported_login(&item("/logins/a", "small-file", None)));
    assert!(!is_exported_login(&item("/logins/a", "small-file", other)));
    assert!(!is_exported_login(&item("/notes", "small-file", None)));
    assert!(!is_exported_login(&item("/logins/big", "file", None)));
    assert!(!is_exported_login(&item("/logins", "directory", None)));
    assert!(!is_exported_login(&item(
        "/.trash/owner/20240102T030405~%2Fbank",
        "small-file",
        login
    )));
}

#[test]
fn folders_below_logins_are_relative_to_it_and_names_are_decoded() {
    assert_eq!(
        export_location("/logins/Work/Mail"),
        ("Work".to_owned(), "Mail".to_owned())
    );
    assert_eq!(
        export_location("/logins/github.com"),
        (String::new(), "github.com".to_owned())
    );
    assert_eq!(
        export_location("/keys/50%25 off %2F sale"),
        ("keys".to_owned(), "50% off / sale".to_owned())
    );
    // A folder whose decoded name holds a `/` keeps its stored form, and a
    // name that does not decode is written as stored.
    assert_eq!(
        export_location("/a%2Fb/50% off"),
        ("a%2Fb".to_owned(), "50% off".to_owned())
    );
}

#[test]
fn a_login_is_split_into_its_fields_and_its_other_lines() {
    let (username, password, url, notes) = login_columns(
        "user: ana\r\npassword:  spaced \nurl: https://x.example\nnote: first\nnote:\npin: 1234\nnote: last\n\n",
    );
    assert_eq!(username.as_str(), "ana");
    assert_eq!(password.as_str(), " spaced ");
    assert_eq!(url.as_str(), "https://x.example");
    assert_eq!(notes.as_str(), "first\n\npin: 1234\nlast");
    // Only the first line naming a field is its column, as for Copy; a
    // later one stays in the notes rather than being dropped.
    let (username, password, _, notes) =
        login_columns("username: ana\npassword: new\npassword: old\nuser: ana-2");
    assert_eq!(username.as_str(), "ana");
    assert_eq!(password.as_str(), "new");
    assert_eq!(notes.as_str(), "password: old\nuser: ana-2");
    let (username, _, url, notes) = login_columns("password: p\nwebsite: https://y.example");
    assert_eq!(username.as_str(), "");
    assert_eq!(url.as_str(), "https://y.example");
    assert_eq!(notes.as_str(), "");
}

#[test]
fn records_quote_every_field_and_read_back_through_the_import() {
    assert_eq!(
        csv_line(&["a", "say \"hi\"", "x,\ny"]).as_str(),
        "\"a\",\"say \"\"hi\"\"\",\"x,\ny\"\r\n"
    );
    assert!(export_record("/logins/x", &[0xff, 0xfe]).is_none());

    let mut file = format!("{EXPORT_HEADER}\r\n");
    for (path, text) in [
        (
            "/logins/Work/Mail",
            "user: ana\npassword: pa\"ss,word\nurl: https://mail.example\nnote: line one\nnote: line two",
        ),
        ("/logins/github.com", "user: ana\npassword: gh\nurl: "),
    ] {
        file.push_str(&export_record(path, text.as_bytes()).unwrap());
    }
    let header = csv_records(&file, b',').unwrap().remove(0);
    assert_eq!(header.len(), 11);
    let parsed = parse_export(file.as_bytes()).unwrap();
    assert!(parsed.skipped.is_empty());
    let entries: BTreeMap<_, _> = parsed
        .entries
        .iter()
        .map(|entry| {
            (
                format!("{}/{}", entry.folder.join("/"), entry.name),
                entry.content.as_str(),
            )
        })
        .collect();
    assert_eq!(
        entries,
        BTreeMap::from([
            (
                "Work/Mail".to_owned(),
                "user: ana\npassword: pa\"ss,word\nurl: https://mail.example\nnote: line one\nnote: line two"
            ),
            ("/github.com".to_owned(), "user: ana\npassword: gh\nurl: "),
        ])
    );
}

/// Serves each read with the text recorded for its path.
struct LoginTransport {
    texts: BTreeMap<String, Vec<u8>>,
    reads: Mutex<Vec<String>>,
}

impl foks_desktop::AgentTransport for LoginTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        let Operation::ReadKv {
            store,
            path,
            version,
        } = operation
        else {
            panic!("unexpected operation {operation:?}");
        };
        self.reads.lock().unwrap().push(path.clone());
        let content = self.texts[&path].clone();
        serde_json::to_value(KvReadResult {
            store,
            path,
            version,
            node_type: "small-file".to_owned(),
            size: Some(content.len() as u64),
            read_role: KvRole::Owner,
            write_role: KvRole::Owner,
            content: Some(content),
            symlink_target: None,
        })
        .map_err(|error| foks_desktop::AgentError::Transport(error.to_string()))
    }
}

fn transport() -> LoginTransport {
    LoginTransport {
        texts: BTreeMap::from([
            (
                "/logins/a".to_owned(),
                b"user: a\npassword: pa\nurl: ".to_vec(),
            ),
            ("/logins/b".to_owned(), vec![0xff, 0x00]),
            (
                "/logins/c".to_owned(),
                b"user: c\npassword: pc\nurl: ".to_vec(),
            ),
        ]),
        reads: Mutex::new(Vec::new()),
    }
}

fn logins() -> Vec<CatalogItem> {
    ["/logins/a", "/logins/b", "/logins/c"]
        .into_iter()
        .map(|path| item(path, "small-file", None))
        .collect()
}

#[test]
fn every_login_is_read_at_its_listed_version_and_written_after_the_header() {
    let transport = transport();
    let mut file = Vec::new();
    let counts = write_export(&transport, &logins(), &|| Ok(()), &mut file).unwrap();
    assert_eq!(counts, (2, 1));
    let text = String::from_utf8(file).unwrap();
    assert!(text.starts_with(&format!("{EXPORT_HEADER}\r\n")));
    assert_eq!(text.matches("\r\n").count(), 3);
    assert!(text.contains("\"pa\"") && text.contains("\"pc\""));
    assert_eq!(
        transport.reads.lock().unwrap().as_slice(),
        ["/logins/a", "/logins/b", "/logins/c"]
    );
}

#[test]
fn a_lock_during_the_export_stops_it_before_the_next_read() {
    let transport = transport();
    let calls = Mutex::new(0);
    let error = write_export(
        &transport,
        &logins(),
        &|| {
            let mut calls = calls.lock().unwrap();
            *calls += 1;
            if *calls == 2 {
                Err(AgentError::new("app-locked", "locked", false))
            } else {
                Ok(())
            }
        },
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(error.code, "app-locked");
    assert_eq!(transport.reads.lock().unwrap().len(), 1);
}
