mod common;

use foks_server_db::{Error, UserMutation, UsernameMutation};

#[test]
fn database_username_mutations_recheck_role_and_preserve_committed_receipts() {
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture.commit(None).unwrap();
    let sql = rusqlite::Connection::open(&fixture.path).unwrap();
    let host = foks_proto::EntityId::from_bytes([vec![2], vec![7; 32]].concat()).unwrap();
    sql.execute("INSERT INTO host_metadata(singleton,host_id,canonical_name,bootstrap_blob,key_manifest_blob) VALUES(1,?1,'localhost',x'01',x'01')", [host.as_bytes()]).unwrap();
    let uid = foks_proto::EntityId::from_bytes(vec![1; 33]).unwrap();
    let prior = fixture.database.current_root().unwrap().unwrap();
    let name = UsernameMutation {
        normalized_name: b"renamed".to_vec(),
        username_utf8: b"Renamed".to_vec(),
        commitment_key: [0x71; 16],
        token: [0x72; 17],
        name_sequence: 1,
        reservation_expires_at: 2000,
    };
    fixture
        .database
        .reserve_name_for_username_change(
            uid.as_bytes(),
            &[4; 33],
            b"renamed",
            &name.token,
            1_000_001,
            2_000_000,
        )
        .unwrap();
    let leaves = [
        (
            foks_merkle_store::chain_key(0, &uid, 2, Some(&[0x33; 32])).unwrap(),
            [0x73; 32],
        ),
        (
            foks_merkle_store::username_key(b"renamed", &host, 1).unwrap(),
            foks_merkle_store::username_leaf(&uid).unwrap(),
        ),
    ];
    let changes = leaves
        .iter()
        .map(|(key, value)| foks_merkle_store::LeafChange::Set {
            key: *key,
            value: *value,
        })
        .collect::<Vec<_>>();
    let commit =
        foks_merkle_store::prepare(&fixture.database.node_reader(), prior.root_node, &changes)
            .unwrap();
    let mut mutation = UserMutation {
        username: Some(&name),
        uid: uid.as_bytes(),
        signer_device_id: &[4; 33],
        authenticated_credential_id: &[4; 33],
        expected_sequence: 2,
        expected_tail_hash: &[0x32; 32],
        link_hash: &[0x73; 32],
        exact_link: b"rename-link",
        current_tree_location: &[0x33; 32],
        next_tree_location: &[0x74; 32],
        added_credential: None,
        revoked_device_id: None,
        shared_keys: &[],
        parcels: &[],
        seed_chain: &[],
        passphrase: None,
        user_settings: None,
        cited_root_epoch: 1,
        expected_root_epoch: 1,
        expected_root_hash: &[0x34; 32],
        merkle_commit: &commit,
        merkle_leaves: &leaves,
        root_epoch: 2,
        root_hash: &[0x75; 32],
        exact_root: b"root-two",
        exact_signed_root: b"signed-root-two",
        back_pointers: &[(1, [0x34; 32])],
        idempotency_key: &[0x76; 32],
        request_hash: &[0x77; 32],
        response: b"",
        now: 1_000_002,
        idempotency_expires_at: 2_000_002,
    };
    for (active, role) in [(1, 1), (1, 2), (0, 3)] {
        sql.execute("UPDATE devices SET active=?1,role_type=?2", [active, role])
            .unwrap();
        assert!(matches!(
            fixture.database.reserve_name_for_username_change(
                uid.as_bytes(),
                &[4; 33],
                b"forbidden",
                &[0x78; 17],
                1_000_002,
                2_000_000
            ),
            Err(Error::AuthorizationChanged)
        ));
        assert!(matches!(
            fixture.database.change_username_display(
                uid.as_bytes(),
                &[4; 33],
                b"fixtureuser",
                b"Fixture User",
                b"FIXTURE USER"
            ),
            Err(Error::AuthorizationChanged)
        ));
        assert!(matches!(
            fixture.database.commit_user_mutation(&mutation),
            Err(Error::AuthorizationChanged)
        ));
        assert_eq!(
            sql.query_row(
                "SELECT count(*) FROM names WHERE normalized_name=x'666f7262696464656e'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row("SELECT username_utf8 FROM users", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            b"Fixture User"
        );
        assert_eq!(
            sql.query_row("SELECT seqno FROM user_chain_heads", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(sql
            .query_row(
                "SELECT uid IS NULL FROM names WHERE normalized_name=?1",
                [b"renamed".as_slice()],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
    }
    sql.execute("UPDATE devices SET active=1,role_type=3", [])
        .unwrap();
    let parent = [vec![foks_proto::ENTITY_YUBI], vec![0x7a; 33]].concat();
    let old_subkey = [vec![foks_proto::ENTITY_SUBKEY], vec![0x7b; 32]].concat();
    let new_subkey = [vec![foks_proto::ENTITY_SUBKEY], vec![0x7c; 32]].concat();
    sql.execute("INSERT INTO devices(device_id,uid,active,role_type,visibility,subkey_id,hepk_fingerprint,self_token,exact_hepk,exact_name)
        SELECT ?1,uid,1,3,0,?2,hepk_fingerprint,?3,exact_hepk,exact_name FROM devices WHERE device_id=?4",
        rusqlite::params![parent, new_subkey, [0x7du8;17].as_slice(), [4u8;33].as_slice()]).unwrap();
    // Both the transport credential and signed parent must still identify the
    // same active OWNER row, not merely two independently authorized devices.
    mutation.signer_device_id = &parent;
    mutation.authenticated_credential_id = &old_subkey;
    assert!(matches!(
        fixture.database.commit_user_mutation(&mutation),
        Err(Error::AuthorizationChanged)
    ));
    mutation.signer_device_id = &[4; 33];
    mutation.authenticated_credential_id = &new_subkey;
    assert!(matches!(
        fixture.database.commit_user_mutation(&mutation),
        Err(Error::AuthorizationChanged)
    ));
    mutation.authenticated_credential_id = &[4; 33];
    fixture.database.commit_user_mutation(&mutation).unwrap();
    sql.execute("UPDATE devices SET active=0,role_type=1", [])
        .unwrap();
    // Historical success is a receipt lookup, not permission to author a new mutation.
    assert_eq!(
        fixture.database.commit_user_mutation(&mutation).unwrap(),
        b""
    );
    assert_eq!(
        sql.query_row("SELECT seqno FROM user_chain_heads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
