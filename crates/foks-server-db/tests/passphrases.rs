mod common;

use foks_server_db::{Config, Error, FailurePoint, PassphraseMutation};

const UID: [u8; 33] = [1; 33];
const DEVICE: [u8; 33] = [4; 33];
const VERIFY_KEY: [u8; 33] = [17; 33];
const SALT: [u8; 16] = [9; 16];

fn mutation(generation: u64, now: u64) -> PassphraseMutation<'static> {
    mutation_for_puk(generation, 1, now)
}

fn mutation_for_puk(generation: u64, puk_generation: u64, now: u64) -> PassphraseMutation<'static> {
    PassphraseMutation {
        verify_key: &VERIFY_KEY,
        salt: &SALT,
        generation,
        exact_skmwk_box: b"skmwk-box",
        exact_passphrase_box: b"passphrase-box",
        exact_puk_box: Some(b"puk-box"),
        puk_generation: Some(puk_generation),
        stretch_version: 1,
        now,
    }
}

#[test]
fn passphrase_mutations_recheck_the_current_owner_puk_generation() {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    let rotation = rusqlite::Connection::open(&test.path).unwrap();
    rotation
        .execute(
            "INSERT INTO shared_keys
             (uid, role_type, visibility, generation, verify_key, exact_hepk)
             VALUES (?1, 3, 0, 2, ?2, ?3)",
            rusqlite::params![UID, [15u8; 33], b"rotated-owner-hepk"],
        )
        .unwrap();

    assert!(matches!(
        test.database
            .set_passphrase(&UID, &DEVICE, mutation_for_puk(1, 1, 2_000_000)),
        Err(Error::AuthorizationChanged)
    ));
    assert!(test.database.passphrase(&UID).unwrap().is_none());

    test.database
        .set_passphrase(&UID, &DEVICE, mutation_for_puk(1, 2, 2_000_001))
        .unwrap();
    assert!(matches!(
        test.database
            .change_passphrase(&UID, &DEVICE, mutation_for_puk(2, 1, 2_000_002)),
        Err(Error::AuthorizationChanged)
    ));
    assert_eq!(
        test.database.passphrase(&UID).unwrap().unwrap().generation,
        1
    );
}

#[test]
fn passphrase_mutations_require_an_owner_role_device() {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    test.database
        .set_passphrase(&UID, &DEVICE, mutation(1, 2_000_000))
        .unwrap();

    const MEMBER: [u8; 33] = [5; 33];
    let connection = rusqlite::Connection::open(&test.path).unwrap();
    connection
        .execute(
            "INSERT INTO devices
             (device_id, uid, active, role_type, visibility, subkey_id,
              hepk_fingerprint, exact_hepk, exact_name)
             VALUES (?1, ?2, 1, 1, 0, NULL, ?3, ?4, ?5)",
            rusqlite::params![MEMBER, UID, [0x11u8; 32], b"member-hepk", b"member"],
        )
        .unwrap();
    assert!(matches!(
        test.database
            .change_passphrase(&UID, &MEMBER, mutation(2, 2_000_001)),
        Err(Error::AuthorizationChanged)
    ));
    assert_eq!(
        test.database.passphrase(&UID).unwrap().unwrap().generation,
        1
    );
}

#[test]
fn passphrase_generations_are_atomic_and_strictly_sequential() {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();

    test.database
        .set_passphrase(&UID, &DEVICE, mutation(1, 2_000_000))
        .unwrap();
    assert_eq!(
        test.database.passphrase(&UID).unwrap().unwrap().generation,
        1
    );
    assert!(matches!(
        test.database
            .set_passphrase(&UID, &DEVICE, mutation(1, 2_000_001)),
        Err(Error::PassphraseGeneration)
    ));
    assert!(matches!(
        test.database
            .change_passphrase(&UID, &DEVICE, mutation(3, 2_000_002)),
        Err(Error::PassphraseGeneration)
    ));
    test.database
        .change_passphrase(&UID, &DEVICE, mutation(2, 2_000_003))
        .unwrap();
    assert_eq!(
        test.database.passphrase(&UID).unwrap().unwrap().generation,
        2
    );
}

#[test]
fn passphrase_mutations_require_an_active_credential_at_commit_time() {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    let credential_state = rusqlite::Connection::open(&test.path).unwrap();

    credential_state
        .execute(
            "UPDATE devices SET active = 0 WHERE uid = ?1 AND device_id = ?2",
            rusqlite::params![UID, DEVICE],
        )
        .unwrap();
    assert!(matches!(
        test.database
            .set_passphrase(&UID, &DEVICE, mutation(1, 2_000_000)),
        Err(Error::AuthorizationChanged)
    ));
    assert!(test.database.passphrase(&UID).unwrap().is_none());

    credential_state
        .execute(
            "UPDATE devices SET active = 1 WHERE uid = ?1 AND device_id = ?2",
            rusqlite::params![UID, DEVICE],
        )
        .unwrap();
    test.database
        .set_passphrase(&UID, &DEVICE, mutation(1, 2_000_001))
        .unwrap();

    credential_state
        .execute(
            "UPDATE devices SET active = 0 WHERE uid = ?1 AND device_id = ?2",
            rusqlite::params![UID, DEVICE],
        )
        .unwrap();
    assert!(matches!(
        test.database
            .change_passphrase(&UID, &DEVICE, mutation(2, 2_000_002)),
        Err(Error::AuthorizationChanged)
    ));
    assert_eq!(
        test.database.passphrase(&UID).unwrap().unwrap().generation,
        1
    );
}

#[test]
fn passphrase_change_waiting_behind_revocation_fails_closed() {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    test.database
        .set_passphrase(&UID, &DEVICE, mutation(1, 2_000_000))
        .unwrap();

    let mut contender = foks_server_db::Database::open(&test.path, Config::default()).unwrap();
    let revocation = rusqlite::Connection::open(&test.path).unwrap();
    revocation.execute_batch("BEGIN IMMEDIATE").unwrap();
    revocation
        .execute(
            "UPDATE devices SET active = 0 WHERE uid = ?1 AND device_id = ?2",
            rusqlite::params![UID, DEVICE],
        )
        .unwrap();

    let (started_sender, started_receiver) = std::sync::mpsc::sync_channel(1);
    let change = std::thread::spawn(move || {
        started_sender.send(()).unwrap();
        contender.change_passphrase(&UID, &DEVICE, mutation(2, 2_000_001))
    });
    started_receiver.recv().unwrap();
    revocation.execute_batch("COMMIT").unwrap();

    assert!(matches!(
        change.join().unwrap(),
        Err(Error::AuthorizationChanged)
    ));
    assert_eq!(
        test.database.passphrase(&UID).unwrap().unwrap().generation,
        1
    );
}

#[test]
fn signup_passphrase_rolls_back_with_the_identity_transaction() {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    assert!(matches!(
        common::commit_with_passphrase(
            &mut test.database,
            Some(FailurePoint::Passphrase),
            mutation(1, 1_000_000),
        ),
        Err(Error::Injected(FailurePoint::Passphrase))
    ));
    assert!(test.database.identity(&UID).unwrap().is_none());
    assert!(test.database.passphrase(&UID).unwrap().is_none());
}

#[test]
fn signup_passphrase_requires_owner_puk_generation_one() {
    let mut test = common::TestDatabase::new();
    test.reserve(1_000_000);
    assert!(matches!(
        common::commit_with_passphrase(&mut test.database, None, mutation_for_puk(1, 2, 1_000_000)),
        Err(Error::PassphraseGeneration)
    ));
    assert!(test.database.identity(&UID).unwrap().is_none());
}

#[test]
fn login_challenges_are_one_time_and_bad_attempts_are_bounded() {
    let mut test = common::TestDatabase::with_config(Config {
        maximum_bad_passphrase_attempts: 2,
        maximum_active_passphrase_challenges: 2,
        maximum_passphrase_challenges_per_user: 1,
        ..Config::default()
    });
    test.reserve(1_000_000);
    test.commit(None).unwrap();
    test.database
        .set_passphrase(&UID, &DEVICE, mutation(1, 2_000_000))
        .unwrap();
    let host = [2; 33];
    let key_generation = [3; 16];
    test.database
        .issue_passphrase_challenge(&[4; 32], &UID, &host, &key_generation, 4_000_000, 3_000_000)
        .unwrap();
    assert!(matches!(
        test.database.issue_passphrase_challenge(
            &[5; 32],
            &UID,
            &host,
            &key_generation,
            4_000_000,
            3_000_000,
        ),
        Err(Error::QuotaExceeded)
    ));
    assert!(test
        .database
        .consume_passphrase_challenge(
            &[4; 32],
            &UID,
            &host,
            &key_generation,
            &VERIFY_KEY,
            3_000_001,
        )
        .unwrap()
        .is_some());
    assert!(test
        .database
        .consume_passphrase_challenge(
            &[4; 32],
            &UID,
            &host,
            &key_generation,
            &VERIFY_KEY,
            3_000_002,
        )
        .unwrap()
        .is_none());

    test.database
        .issue_passphrase_challenge(&[6; 32], &UID, &host, &key_generation, 4_000_000, 3_000_003)
        .unwrap();
    test.database
        .consume_failed_passphrase_challenge(&[6; 32], &UID, &host, &key_generation, 3_000_004)
        .unwrap();
    assert!(test
        .database
        .consume_passphrase_challenge(
            &[6; 32],
            &UID,
            &host,
            &key_generation,
            &VERIFY_KEY,
            3_000_005,
        )
        .unwrap()
        .is_none());

    assert!(test
        .database
        .passphrase_for_login(&UID, 3_000_006)
        .unwrap()
        .is_some());
    assert!(test
        .database
        .passphrase_for_login(&UID, 3_000_007)
        .unwrap()
        .is_some());
    assert!(matches!(
        test.database.passphrase_for_login(&UID, 3_000_008),
        Err(Error::PassphraseRateLimited)
    ));
}
