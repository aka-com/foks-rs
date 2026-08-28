mod common;

use foks_merkle_store::{prepare, LeafChange};
use foks_server_db::{
    KvDirectoryMutation, KvRootMutation, TeamHeader, TeamMemberMutation, TeamMutation,
    TeamMutationFailurePoint, TeamSharedKeyMutation,
};

#[test]
fn every_team_publication_boundary_is_atomic() {
    for point in [
        TeamMutationFailurePoint::Chain,
        TeamMutationFailurePoint::Projection,
        TeamMutationFailurePoint::MerkleNodes,
        TeamMutationFailurePoint::MerkleRoot,
        TeamMutationFailurePoint::Receipt,
    ] {
        let mut fixture = common::TestDatabase::new();
        fixture.reserve(1_000_000);
        fixture.commit(None).unwrap();
        let prior = fixture.database.current_root().unwrap().unwrap();
        let leaf = ([0x71; 32], [0x72; 32]);
        let commit = prepare(
            &fixture.database.node_reader(),
            prior.root_node,
            &[LeafChange::Set {
                key: leaf.0,
                value: leaf.1,
            }],
        )
        .unwrap();
        let member = TeamMemberMutation {
            party_id: &[1; 33],
            scoped_host_id: None,
            source_role_type: 3,
            source_visibility: 0,
            role_type: 3,
            visibility: 0,
            generation: 1,
            verify_key: &[14; 33],
            hepk_fingerprint: &[0x73; 32],
            removal_key_commitment: Some(&[0x74; 32]),
        };
        let key = TeamSharedKeyMutation {
            role_type: 3,
            visibility: 0,
            generation: 1,
            verify_key: &[15; 33],
            exact_hepk: b"team-hepk",
        };
        let team = {
            let mut team = [0x75; 33];
            team[0] = foks_proto::ENTITY_AD_HOC_TEAM;
            team
        };
        let mutation = TeamMutation {
            team_id: &team,
            signer_credential_id: &[4; 33],
            header: Some(TeamHeader {
                kind: foks_proto::ENTITY_AD_HOC_TEAM,
                host_id: &[2; 33],
                normalized_name: None,
                team_name_utf8: b"",
                name_sequence: 0,
                name_commitment_key: None,
                reservation_token: None,
                reservation_expires_at: None,
            }),
            expected_sequence: 1,
            expected_tail_hash: None,
            link_hash: &[0x76; 32],
            exact_link: b"team-link-one",
            next_tree_location: &[0x77; 32],
            members: &[member],
            shared_keys: &[key],
            parcels: &[],
            seed_chain: &[],
            removal_boxes: &[],
            expected_root_epoch: 1,
            expected_root_hash: &prior.root_hash,
            merkle_commit: &commit,
            merkle_leaves: &[leaf],
            root_epoch: 2,
            root_hash: &[0x78; 32],
            exact_root: b"team-root-two",
            exact_signed_root: b"signed-team-root-two",
            back_pointers: &[(1, prior.root_hash)],
            idempotency_key: &[0x79; 32],
            request_hash: &[0x7a; 32],
            response: b"",
            now: 1_000_001,
            receipt_expires_at: 2_000_001,
        };
        assert!(fixture
            .database
            .commit_team_mutation_with_failure(&mutation, Some(point))
            .is_err());
        assert!(fixture.database.team(&team).unwrap().is_none(), "{point:?}");
        assert_eq!(fixture.database.current_root().unwrap().unwrap(), prior);
        fixture.database.commit_team_mutation(&mutation).unwrap();
        let stored = fixture.database.team(&team).unwrap().unwrap();
        assert_eq!(stored.links.len(), 1);
        assert_eq!(stored.members.len(), 1);
        assert_eq!(stored.shared_keys.len(), 1);
        assert!(fixture.database.ensure_kv_namespace(&team).unwrap());
        let kv_root = [0x7b; 16];
        fixture
            .database
            .put_kv_directory(&KvDirectoryMutation {
                uid: &team,
                id: &kv_root,
                version: 1,
                key_role: 3,
                key_visibility: 0,
                key_generation: 1,
                status: 0,
                exact: b"team-kv-directory",
            })
            .unwrap();
        fixture
            .database
            .put_kv_root(&KvRootMutation {
                uid: &team,
                version: 1,
                directory_id: &kv_root,
                directory_version: 1,
                key_role: 3,
                key_visibility: 0,
                key_generation: 1,
                exact: b"team-kv-root",
            })
            .unwrap();

        let reader =
            foks_server_db::ReadDatabase::open(&fixture.path, foks_server_db::Config::default())
                .unwrap();
        let authority = reader
            .team_view_authority(&team, &[1; 33], &[2; 33], 3, 0, 1)
            .unwrap()
            .unwrap();
        fixture
            .database
            .issue_team_view_challenge(
                &[0x81; 32],
                &[0x82; 32],
                &authority,
                &[0x83; 16],
                2_000_000,
                1_000_002,
            )
            .unwrap();
        assert!(fixture
            .database
            .activate_team_view_challenge(&[0x81; 32], &[0x84; 32], 1_000_003)
            .unwrap()
            .is_some());
        assert!(fixture
            .database
            .activate_team_view_challenge(&[0x81; 32], &[0x84; 32], 1_000_004)
            .unwrap()
            .is_some());
        assert!(matches!(
            fixture
                .database
                .activate_team_view_challenge(&[0x81; 32], &[0x85; 32], 1_000_004),
            Err(foks_server_db::Error::ReceiptConflict)
        ));
        assert!(reader
            .resolve_team_view_token(&[0x82; 32], 1_000_005)
            .unwrap()
            .is_some());
        assert!(reader
            .resolve_team_view_token(&[0x82; 32], 2_000_000)
            .unwrap()
            .is_none());

        fixture
            .database
            .issue_team_view_challenge(
                &[0x86; 32],
                &[0x87; 32],
                &authority,
                &[0x88; 16],
                1_000_010,
                1_000_006,
            )
            .unwrap();
        {
            let sabotage = rusqlite::Connection::open(&fixture.path).unwrap();
            sabotage
                .execute_batch(
                    "CREATE TRIGGER ignore_team_view_activation
                     BEFORE UPDATE OF consumed ON team_view_challenges
                     WHEN OLD.consumed = 0
                     BEGIN SELECT RAISE(IGNORE); END;",
                )
                .unwrap();
            assert!(matches!(
                fixture
                    .database
                    .activate_team_view_challenge(&[0x86; 32], &[0x89; 32], 1_000_009),
                Err(foks_server_db::Error::Invalid(
                    "team-view challenge activation transition"
                ))
            ));
            let stored: i64 = sabotage
                .query_row(
                    "SELECT count(*) FROM team_view_tokens WHERE token_hash = ?1",
                    [[0x87; 32]],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(stored, 0);
            sabotage
                .execute_batch("DROP TRIGGER ignore_team_view_activation")
                .unwrap();
        }
        assert!(fixture
            .database
            .activate_team_view_challenge(&[0x86; 32], &[0x89; 32], 1_000_010)
            .unwrap()
            .is_none());

        let connection = rusqlite::Connection::open(&fixture.path).unwrap();
        let namespace: (i64, Vec<u8>) = connection
            .query_row(
                "SELECT party_kind, host_id FROM kv_namespaces WHERE namespace_id = ?1",
                [team],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            namespace,
            (i64::from(foks_proto::ENTITY_AD_HOC_TEAM), vec![2; 33])
        );
        let stored_tokens: i64 = connection
            .query_row(
                "SELECT count(*) FROM team_view_tokens WHERE token_hash = ?1",
                [[0x82; 32]],
                |row| row.get(0),
            )
            .unwrap();
        let raw_tokens: i64 = connection
            .query_row(
                "SELECT count(*) FROM team_view_tokens WHERE token_hash = ?1",
                [[0x83; 32]],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_tokens, 1);
        assert_eq!(raw_tokens, 0, "raw bearer tokens must never be stored");

        let admin = reader
            .team_admin_authority(&team, &[1_u8; 33], 3, 1)
            .unwrap()
            .unwrap();
        fixture
            .database
            .issue_team_admin_token(&[0x90; 32], &admin, 2_000_100, 1_000_008)
            .unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER ignore_team_admin_activation
                 BEFORE UPDATE OF activation_hash ON team_admin_tokens
                 WHEN OLD.activation_hash IS NULL
                 BEGIN SELECT RAISE(IGNORE); END;",
            )
            .unwrap();
        assert!(matches!(
            fixture
                .database
                .activate_team_admin_token(&[0x90; 32], &[0x91; 32], 1_000_009),
            Err(foks_server_db::Error::Invalid(
                "team-admin token activation transition"
            ))
        ));
        let activation: Option<Vec<u8>> = connection
            .query_row(
                "SELECT activation_hash FROM team_admin_tokens WHERE token_hash = ?1",
                [[0x90; 32]],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(activation, None);
        connection
            .execute_batch("DROP TRIGGER ignore_team_admin_activation")
            .unwrap();
        assert!(reader
            .resolve_team_admin_token(&[0x90; 32], 1_000_009)
            .unwrap()
            .is_none());
        assert!(fixture
            .database
            .activate_team_admin_token(&[0x90; 32], &[0x91; 32], 1_000_009)
            .unwrap()
            .is_some());
        assert!(fixture
            .database
            .activate_team_admin_token(&[0x90; 32], &[0x91; 32], 1_000_010)
            .unwrap()
            .is_some());
        assert!(matches!(
            fixture
                .database
                .activate_team_admin_token(&[0x90; 32], &[0x92; 32], 1_000_010),
            Err(foks_server_db::Error::ReceiptConflict)
        ));
        assert!(reader
            .resolve_team_admin_token(&[0x90; 32], 1_000_011)
            .unwrap()
            .is_some());
        assert!(reader
            .resolve_team_admin_token(&[0x90; 32], 2_000_100)
            .unwrap()
            .is_none());
        connection
            .execute(
                "DELETE FROM team_members WHERE team_id = ?1 AND party_id = ?2",
                rusqlite::params![team, [1_u8; 33]],
            )
            .unwrap();
        assert!(reader
            .resolve_team_view_token(&[0x82; 32], 1_000_007)
            .unwrap()
            .is_none());
        assert!(reader
            .resolve_team_admin_token(&[0x90; 32], 1_000_012)
            .unwrap()
            .is_none());
    }
}
