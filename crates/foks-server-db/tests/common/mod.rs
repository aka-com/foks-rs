#![allow(dead_code)]

use foks_merkle_store::{prepare, LeafChange, MemoryStore, EMPTY_ROOT};
use foks_server_db::{CommitOutcome, Config, Database, FailurePoint, IdentityMutation};

pub struct TestDatabase {
    _directory: tempfile::TempDir,
    pub path: std::path::PathBuf,
    pub database: Database,
}

impl TestDatabase {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("foks-server.sqlite");
        let database = Database::open(&path, Config::default()).unwrap();
        Self {
            _directory: directory,
            path,
            database,
        }
    }

    pub fn reserve(&mut self, now: u64) {
        self.database
            .reserve_name(b"fixtureuser", &[0x44; 32], 1, now, now + 1_000_000)
            .unwrap();
    }

    pub fn commit(
        &mut self,
        failure: Option<FailurePoint>,
    ) -> foks_server_db::Result<CommitOutcome> {
        commit_with_request(&mut self.database, failure, [0x99; 32])
    }
}

pub fn commit_with_request(
    database: &mut Database,
    failure: Option<FailurePoint>,
    request_hash: [u8; 32],
) -> foks_server_db::Result<CommitOutcome> {
    commit_with_request_at(database, failure, request_hash, 1_000_000)
}

pub fn commit_with_request_at(
    database: &mut Database,
    failure: Option<FailurePoint>,
    request_hash: [u8; 32],
    now: u64,
) -> foks_server_db::Result<CommitOutcome> {
    let leaf = ([0x10; 32], [0x20; 32]);
    let merkle_commit = prepare(
        &MemoryStore::default(),
        EMPTY_ROOT,
        &[LeafChange::Set {
            key: leaf.0,
            value: leaf.1,
        }],
    )
    .unwrap();
    let mutation = IdentityMutation {
        normalized_name: b"fixtureuser",
        reservation_token: &[0x44; 32],
        reservation_sequence: 1,
        uid: &[1; 33],
        device_id: &[4; 33],
        device_hepk_fingerprint: &[0x31; 32],
        exact_device_hepk: b"device-hepk",
        link_hash: &[0x32; 32],
        exact_link: b"exact-link",
        tree_location: &[0x33; 32],
        shared_role_type: 3,
        shared_visibility: 0,
        shared_generation: 1,
        shared_verify_key: &[14; 33],
        exact_shared_hepk: b"shared-hepk",
        exact_parcel: b"parcel",
        expected_root_hash: None,
        merkle_commit: &merkle_commit,
        merkle_leaves: &[leaf],
        root_epoch: 1,
        root_hash: &[0x34; 32],
        exact_root: b"root",
        exact_signed_root: b"signed-root",
        back_pointers: &[],
        idempotency_key: &[0x55; 16],
        request_hash: &request_hash,
        response: b"response",
        now,
        receipt_expires_at: now + 1_000_000,
    };
    match failure {
        Some(point) => database.commit_identity_with_failure(&mutation, Some(point)),
        None => database.commit_identity(&mutation),
    }
}
