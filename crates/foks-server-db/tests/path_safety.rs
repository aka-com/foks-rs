#![cfg(unix)]

use foks_server_db::{DatabasePathIdentity, Error};

#[test]
fn captured_database_identity_rejects_a_late_hardlink() {
    let temporary = tempfile::tempdir().unwrap();
    let database = temporary.path().join("server.sqlite");
    let identity = DatabasePathIdentity::prepare(&database).unwrap();
    std::fs::hard_link(&database, temporary.path().join("alias.sqlite")).unwrap();

    assert!(matches!(
        identity.recheck(),
        Err(Error::UnsafeDatabasePath(
            "database file must have exactly one hardlink"
        ))
    ));
}

#[test]
fn captured_database_identity_rejects_path_replacement() {
    let temporary = tempfile::tempdir().unwrap();
    let database = temporary.path().join("server.sqlite");
    let identity = DatabasePathIdentity::prepare(&database).unwrap();
    std::fs::rename(&database, temporary.path().join("replaced.sqlite")).unwrap();
    std::fs::File::create(&database).unwrap();

    assert!(matches!(
        identity.recheck(),
        Err(Error::UnsafeDatabasePath("database path identity changed"))
    ));
}
