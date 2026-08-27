mod common;

use foks_server_db::{Config, Database, Error, ReadDatabase};

#[test]
fn reopen_and_online_backup_preserve_exact_signed_bytes() {
    let mut original = common::TestDatabase::new();
    original.reserve(1_000_000);
    original.commit(None).unwrap();

    let backup_path = original.path.with_file_name("backup.sqlite");
    original.database.online_backup(&backup_path).unwrap();
    let backup = Database::open(&backup_path, Config::default()).unwrap();
    assert_eq!(
        backup.current_root().unwrap(),
        original.database.current_root().unwrap()
    );
    assert_eq!(
        backup.identity(&[1; 33]).unwrap(),
        original.database.identity(&[1; 33]).unwrap()
    );

    drop(backup);
    let reopened = Database::open(&original.path, Config::default()).unwrap();
    assert_eq!(
        reopened.current_root().unwrap().unwrap().exact_signed_root,
        b"signed-root"
    );
}

#[test]
fn online_backup_can_be_cancelled_between_steps() {
    let original = common::TestDatabase::new();
    let source = ReadDatabase::open(&original.path, Config::default()).unwrap();
    let backup_path = original.path.with_file_name("cancelled-backup.sqlite");

    assert!(matches!(
        source.online_backup_until(&backup_path, || true),
        Err(Error::Invalid("online backup cancelled"))
    ));
}
