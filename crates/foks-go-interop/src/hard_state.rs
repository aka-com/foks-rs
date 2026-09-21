use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use foks_snowpack::{decode, encode, Value};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest as _, Sha512_256};

use crate::{array_range, binary, exact_array, hex, text, Candidate};

const HARD_STATE_NAME: &str = "foks.hard.sqlite";
const USER_DATA_TYPE: i64 = 11;
const MAXIMUM_USER_ROWS: usize = 1_024;
const MAXIMUM_DATABASE_VALUE_BYTES: usize = 64 * 1024;
const LOCAL_USER_INDEX_AT_HOST_TYPE_ID: u64 = 0xef52_df3b_df52_d9d1;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Identity {
    host_id_hex: String,
    user_id_hex: String,
    device_id_hex: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Annotation {
    username: Option<String>,
    server_hint: Option<String>,
}

pub(super) fn enrich(root: &Path, candidates: &mut [Candidate]) {
    let Ok(annotations) = read_annotations(&root.join(HARD_STATE_NAME)) else {
        return;
    };
    for candidate in candidates {
        let identity = Identity {
            host_id_hex: candidate.host_id_hex.clone(),
            user_id_hex: candidate.user_id_hex.clone(),
            device_id_hex: candidate.device_id_hex.clone(),
        };
        if let Some(annotation) = annotations.get(&identity) {
            candidate.username.clone_from(&annotation.username);
            candidate.server_hint.clone_from(&annotation.server_hint);
        }
    }
}

fn read_annotations(path: &Path) -> rusqlite::Result<BTreeMap<Identity, Annotation>> {
    if !path.exists() || !safe_database_file(path) {
        return Ok(BTreeMap::new());
    }
    let Ok(path) = std::fs::canonicalize(path) else {
        return Ok(BTreeMap::new());
    };
    if !safe_database_file(&path) {
        return Ok(BTreeMap::new());
    }
    let connection = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(Duration::from_millis(50))?;
    connection.execute_batch("PRAGMA query_only = ON; PRAGMA trusted_schema = OFF;")?;
    let mut statement = connection.prepare(
        "SELECT scope.label, scoped_data.key, scoped_data.val
         FROM scoped_data
         JOIN scope ON scope.id = scoped_data.scope_id
         WHERE scoped_data.typ = ?1
         LIMIT ?2",
    )?;
    let rows = statement.query_map((USER_DATA_TYPE, (MAXIMUM_USER_ROWS + 1) as i64), |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    let mut annotations = BTreeMap::new();
    for (index, row) in rows.enumerate() {
        if index == MAXIMUM_USER_ROWS {
            return Ok(BTreeMap::new());
        }
        let Ok((scope, key, value)) = row else {
            continue;
        };
        if [scope.len(), key.len(), value.len()]
            .into_iter()
            .any(|length| length > MAXIMUM_DATABASE_VALUE_BYTES)
        {
            continue;
        }
        let Some((identity, annotation)) = decode_user_row(&scope, &key, &value) else {
            continue;
        };
        if annotations.insert(identity.clone(), annotation).is_some() {
            annotations.remove(&identity);
        }
    }
    Ok(annotations)
}

fn decode_user_row(scope: &[u8], key: &[u8], value: &[u8]) -> Option<(Identity, Annotation)> {
    let scope = decode(scope).ok()?;
    let scope_host = entity(binary(&scope, "hard-state host scope").ok()?, 2, 33)?;

    let value = decode(value).ok()?;
    let fields = array_range(&value, 10, 16, "hard-state user info").ok()?;
    let fqu = exact_array(&fields[0], 2, "hard-state fully-qualified user").ok()?;
    let value_user = entity(binary(&fqu[0], "hard-state user id").ok()?, 1, 33)?;
    let value_host = entity(binary(&fqu[1], "hard-state host id").ok()?, 2, 33)?;
    let value_device = device(binary(&fields[8], "hard-state device id").ok()?)?;
    if scope_host != value_host || !user_database_key_matches(key, &value_user, &value_device) {
        return None;
    }

    let names = exact_array(&fields[1], 2, "hard-state username").ok()?;
    let normalized = text(&names[0], "hard-state normalized username").ok()?;
    let display = text(&names[1], "hard-state display username").ok()?;
    let username = bounded_display_text(
        if display.is_empty() {
            normalized
        } else {
            display
        },
        256,
    );
    let server_hint =
        bounded_display_text(text(&fields[2], "hard-state server address").ok()?, 2_048);
    Some((
        Identity {
            host_id_hex: hex(&value_host),
            user_id_hex: hex(&value_user),
            device_id_hex: hex(&value_device),
        },
        Annotation {
            username,
            server_hint,
        },
    ))
}

fn user_database_key_matches(database_key: &[u8], user: &[u8], device: &[u8]) -> bool {
    if database_key.len() != 32 {
        return false;
    }
    let Ok(encoded) = encode(&Value::Array(vec![
        Value::Binary(user.to_vec()),
        Value::Binary(device.to_vec()),
    ])) else {
        return false;
    };
    let mut hash = Sha512_256::new();
    hash.update(LOCAL_USER_INDEX_AT_HOST_TYPE_ID.to_be_bytes());
    hash.update(encoded);
    hash.finalize().as_slice() == database_key
}

fn entity(bytes: &[u8], kind: u8, length: usize) -> Option<Vec<u8>> {
    (bytes.len() == length && bytes.first() == Some(&kind)).then(|| bytes.to_vec())
}

fn device(bytes: &[u8]) -> Option<Vec<u8>> {
    (matches!(bytes.len(), 33 | 34)
        && bytes
            .first()
            .is_some_and(|kind| matches!(*kind, 4 | 8 | 16 | 19)))
    .then(|| bytes.to_vec())
}

fn bounded_display_text(value: String, maximum: usize) -> Option<String> {
    (!value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control))
        .then_some(value)
}

fn safe_database_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o022 != 0
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_snowpack::encode;
    use rusqlite::params;
    use std::os::unix::fs::PermissionsExt as _;

    fn typed(kind: u8, fill: u8) -> Vec<u8> {
        let mut value = vec![kind];
        value.extend([fill; 32]);
        value
    }

    fn candidate() -> Candidate {
        Candidate {
            id: "a".repeat(64),
            username: None,
            server_hint: None,
            host_id_hex: hex(&typed(2, 2)),
            user_id_hex: hex(&typed(1, 1)),
            device_id_hex: hex(&typed(4, 4)),
            role: "owner".to_owned(),
            storage: crate::StorageKind::MacosKeychain,
            hidden: false,
            provisional: false,
            pairable: true,
            copyable: cfg!(target_os = "macos"),
        }
    }

    fn create_database(path: &Path, key_device: Vec<u8>, value_device: Vec<u8>) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE scope (id INTEGER PRIMARY KEY, label BLOB NOT NULL);
                 CREATE TABLE scoped_data (
                    scope_id INTEGER NOT NULL,
                    typ INTEGER NOT NULL,
                    key BLOB NOT NULL,
                    val BLOB
                 );",
            )
            .unwrap();
        let host = typed(2, 2);
        let user = typed(1, 1);
        let scope = encode(&Value::Binary(host.clone())).unwrap();
        let encoded_key = encode(&Value::Array(vec![
            Value::Binary(user.clone()),
            Value::Binary(key_device),
        ]))
        .unwrap();
        let mut hash = Sha512_256::new();
        hash.update(LOCAL_USER_INDEX_AT_HOST_TYPE_ID.to_be_bytes());
        hash.update(encoded_key);
        let key = hash.finalize().to_vec();
        let value = encode(&Value::Array(vec![
            Value::Array(vec![Value::Binary(user), Value::Binary(host)]),
            Value::Array(vec![
                Value::Text(b"alice".to_vec()),
                Value::Text(b"Alice".to_vec()),
            ]),
            Value::Text(b"foks.example:4430".to_vec()),
            Value::Bool(true),
            Value::Null,
            Value::Null,
            Value::Unsigned(0),
            Value::Null,
            Value::Binary(value_device),
            Value::Text(Vec::new()),
        ]))
        .unwrap();
        connection
            .execute("INSERT INTO scope (id, label) VALUES (1, ?1)", [&scope])
            .unwrap();
        connection
            .execute(
                "INSERT INTO scoped_data (scope_id, typ, key, val) VALUES (1, ?1, ?2, ?3)",
                params![USER_DATA_TYPE, key, value],
            )
            .unwrap();
    }

    #[test]
    fn exact_identity_enriches_candidate() {
        let directory = tempfile::tempdir().unwrap();
        create_database(
            &directory.path().join(HARD_STATE_NAME),
            typed(4, 4),
            typed(4, 4),
        );
        assert_eq!(
            read_annotations(&directory.path().join(HARD_STATE_NAME))
                .unwrap()
                .len(),
            1
        );
        let mut candidates = vec![candidate()];
        enrich(directory.path(), &mut candidates);
        assert_eq!(candidates[0].username.as_deref(), Some("Alice"));
        assert_eq!(
            candidates[0].server_hint.as_deref(),
            Some("foks.example:4430")
        );
    }

    #[test]
    fn mismatched_redundant_identity_is_not_used() {
        let directory = tempfile::tempdir().unwrap();
        create_database(
            &directory.path().join(HARD_STATE_NAME),
            typed(4, 4),
            typed(4, 9),
        );
        let mut candidates = vec![candidate()];
        enrich(directory.path(), &mut candidates);
        assert_eq!(candidates[0].username, None);
        assert_eq!(candidates[0].server_hint, None);
    }

    #[test]
    fn absent_corrupt_and_unsafe_databases_leave_candidates_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let mut candidates = vec![candidate()];
        enrich(directory.path(), &mut candidates);
        assert_eq!(candidates, vec![candidate()]);

        let path = directory.path().join(HARD_STATE_NAME);
        std::fs::write(&path, b"not sqlite").unwrap();
        enrich(directory.path(), &mut candidates);
        assert_eq!(candidates, vec![candidate()]);

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        enrich(directory.path(), &mut candidates);
        assert_eq!(candidates, vec![candidate()]);
    }
}
