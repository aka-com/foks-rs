mod common;

use foks_merkle_store::{prepare, LeafChange};
use foks_server_db::{
    KvDirectoryMutation, KvRootMutation, TeamAdminTokenActivation, TeamAdminTokenBinding,
    TeamAdminTokenIssue, TeamHeader, TeamMemberMutation, TeamMutation, TeamMutationFailurePoint,
    TeamParcelMutation, TeamRemovalBoxMutation, TeamRemovalProofMutation, TeamSharedKeyMutation,
};

fn admin_activation<'a>(
    token_hash: &'a [u8; 32],
    activation_hash: &'a [u8; 32],
    now: u64,
    team_id: &'a [u8],
) -> TeamAdminTokenActivation<'a> {
    TeamAdminTokenActivation {
        token_hash,
        activation_hash,
        binding: TeamAdminTokenBinding {
            team_id,
            holder_id: &[1; 33],
            ptk_role_type: 3,
            ptk_generation: 1,
        },
        now,
    }
}

fn reader_team_parcels(
    path: &std::path::Path,
    team: &[u8],
    party: &[u8],
    source_role_type: u64,
) -> Vec<Vec<u8>> {
    foks_server_db::ReadDatabase::open(path, foks_server_db::Config::default())
        .unwrap()
        .team_parcels(team, party, source_role_type, 0)
        .unwrap()
}

#[test]
fn every_team_publication_boundary_is_atomic() {
    for point in [
        TeamMutationFailurePoint::Chain,
        TeamMutationFailurePoint::Projection,
        TeamMutationFailurePoint::MerkleNodes,
        TeamMutationFailurePoint::MerkleRoot,
        TeamMutationFailurePoint::IdempotencyRecord,
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
        let removal_box = TeamRemovalBoxMutation {
            member_id: &[1; 33],
            member_host_id: &[2; 33],
            source_role_type: 3,
            source_visibility: 0,
            exact_box: b"member-removal-box",
        };
        let removal_proof = TeamRemovalProofMutation {
            commitment: &[0x74; 32],
            member_id: &[1; 33],
            member_host_id: &[2; 33],
            source_role_type: 3,
            source_visibility: 0,
            exact_box: b"member-removal-box",
            exact_removal: b"member-removal-proof",
        };
        let team = {
            let mut team = [0x75; 33];
            team[0] = foks_proto::ENTITY_AD_HOC_TEAM;
            team
        };
        let target = foks_proto::EntityId::from_bytes({
            let mut bytes = vec![1; 33];
            bytes[0] = foks_proto::ENTITY_USER;
            bytes
        })
        .unwrap();
        let sender = foks_proto::EntityId::from_bytes({
            let mut bytes = vec![14; 33];
            bytes[0] = foks_proto::ENTITY_PUK_VERIFY;
            bytes
        })
        .unwrap();
        let parcel_for = |target_role: foks_proto::Role, marker: u8| {
            foks_proto::PukParcel {
                generation: 1,
                role: foks_proto::Role::OWNER,
                hybrid: foks_proto::HybridBox {
                    kem_ciphertext: vec![marker],
                    dh_type: 1,
                    sender_dh: None,
                    nonce: [marker; 16],
                    ciphertext: vec![marker],
                },
                target: target.clone(),
                target_host: None,
                target_role,
                target_generation: 1,
                sender: sender.clone(),
                box_id: [marker; 16],
                temp_dh_key: None,
                seed_chain: Vec::new(),
            }
            .encoded()
            .unwrap()
        };
        let owner_parcel = parcel_for(foks_proto::Role::OWNER, 0x31);
        let admin_parcel = parcel_for(foks_proto::Role::ADMIN, 0x32);
        let parcels = [
            TeamParcelMutation {
                party_id: target.as_bytes(),
                sender_id: sender.as_bytes(),
                target_role_type: 3,
                target_visibility: 0,
                role_type: 3,
                visibility: 0,
                generation: 1,
                exact_parcel: &owner_parcel,
            },
            TeamParcelMutation {
                party_id: target.as_bytes(),
                sender_id: sender.as_bytes(),
                target_role_type: 2,
                target_visibility: 0,
                role_type: 3,
                visibility: 0,
                generation: 1,
                exact_parcel: &admin_parcel,
            },
        ];
        let mut mutation = TeamMutation {
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
                subchain_tree_location_seed: &[0x74; 32],
                member_load_floor_type: 1,
                member_load_floor_visibility: 0,
            }),
            expected_sequence: 1,
            expected_tail_hash: None,
            link_hash: &[0x76; 32],
            exact_link: b"team-link-one",
            next_tree_location: &[0x77; 32],
            members: &[member],
            shared_keys: &[key],
            parcels: &parcels,
            seed_chain: &[],
            removal_boxes: &[removal_box],
            removal_proofs: &[removal_proof],
            remote_member_view_tokens: &[],
            required_local_view_permissions: &[],
            local_view_permissions: &[foks_server_db::TeamLocalViewPermissionMutation {
                target_id: &[1; 33],
                minimum_role_type: 1,
                minimum_role_visibility: 0,
            }],
            generic_link: None,
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
            idempotency_expires_at: 2_000_001,
        };
        // An admission cannot claim a pre-existing invitation grant that is
        // absent at the atomic publication boundary, even if it also asks to
        // create a grant in this same mutation.
        let required = vec![vec![1; 33]];
        mutation.required_local_view_permissions = &required;
        assert!(matches!(
            fixture.database.commit_team_mutation(&mutation),
            Err(foks_server_db::Error::Invalid(
                "local invitation view grant missing"
            ))
        ));
        assert!(fixture.database.team(&team).unwrap().is_none());
        assert_eq!(fixture.database.current_root().unwrap().unwrap(), prior);
        mutation.required_local_view_permissions = &[];
        assert!(fixture
            .database
            .commit_team_mutation_with_failure(&mutation, Some(point))
            .is_err());
        assert!(fixture.database.team(&team).unwrap().is_none(), "{point:?}");
        assert_eq!(fixture.database.current_root().unwrap().unwrap(), prior);
        assert!(fixture
            .database
            .team_removal(&team, &[0x74; 32])
            .unwrap()
            .is_none());
        fixture.database.commit_team_mutation(&mutation).unwrap();
        rusqlite::Connection::open(&fixture.path)
            .unwrap()
            .execute(
                "INSERT INTO team_members
                 (team_id, party_id, scoped_host_id, source_role_type, source_visibility,
                  role_type, visibility, generation, verify_key, hepk_fingerprint,
                  removal_key_commitment)
                 VALUES (?1, ?2, NULL, 2, 0, 2, 0, 1, ?3, ?4, NULL)",
                rusqlite::params![team, [1_u8; 33], [13_u8; 33], [0x73_u8; 32]],
            )
            .unwrap();
        let stored = fixture.database.team(&team).unwrap().unwrap();
        assert_eq!(stored.links.len(), 1);
        assert_eq!(stored.members.len(), 2);
        assert_eq!(stored.shared_keys.len(), 1);
        let owner_parcels = reader_team_parcels(&fixture.path, &team, target.as_bytes(), 3);
        let admin_parcels = reader_team_parcels(&fixture.path, &team, target.as_bytes(), 2);
        assert_eq!(owner_parcels, vec![owner_parcel.clone()]);
        assert_eq!(admin_parcels, vec![admin_parcel.clone()]);
        let removal = fixture
            .database
            .team_removal(&team, &[0x74; 32])
            .unwrap()
            .unwrap();
        assert_eq!(removal.exact_box, b"member-removal-box");
        assert_eq!(removal.exact_removal, b"member-removal-proof");
        rusqlite::Connection::open(&fixture.path)
            .unwrap()
            .execute(
                "UPDATE team_removal_boxes SET exact_box = ?3
                 WHERE team_id = ?1 AND member_id = ?2",
                rusqlite::params![team, [1_u8; 33], b"re-added-member-removal-box"],
            )
            .unwrap();
        let historical = fixture
            .database
            .team_removal(&team, &[0x74; 32])
            .unwrap()
            .unwrap();
        assert_eq!(historical.exact_box, b"member-removal-box");
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
        rusqlite::Connection::open(&fixture.path)
            .unwrap()
            .execute(
                "INSERT INTO capability_key_generations
                 (generation_id, encrypted_file_name, state, created_at, retire_after)
                 VALUES (?1, 'capability.key', 1, 1, NULL)",
                [[0x83; 16]],
            )
            .unwrap();
        let authority = reader
            .team_view_authority(&team, &[1; 33], &[2; 33], 3, 0, 1)
            .unwrap()
            .unwrap();
        assert!(fixture
            .database
            .activate_stateless_team_view_challenge(
                &[0x81; 32],
                &[0x84; 32],
                &[0x82; 32],
                &authority,
                &[0x83; 16],
                2_000_000,
                1_000_003,
            )
            .unwrap()
            .is_some());
        assert!(fixture
            .database
            .activate_stateless_team_view_challenge(
                &[0x81; 32],
                &[0x84; 32],
                &[0x82; 32],
                &authority,
                &[0x83; 16],
                2_000_000,
                1_000_004,
            )
            .unwrap()
            .is_some());
        assert!(matches!(
            fixture.database.activate_stateless_team_view_challenge(
                &[0x81; 32],
                &[0x85; 32],
                &[0x82; 32],
                &authority,
                &[0x83; 16],
                2_000_000,
                1_000_004,
            ),
            Err(foks_server_db::Error::OperationConflict)
        ));
        assert!(reader
            .resolve_team_view_token(&[0x82; 32], 1_000_005)
            .unwrap()
            .is_some());
        assert!(reader
            .resolve_team_view_token(&[0x82; 32], 2_000_000)
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
        connection
            .execute(
                "DELETE FROM team_view_tokens WHERE token_hash = ?1",
                [[0x82; 32]],
            )
            .unwrap();
        assert!(fixture
            .database
            .activate_stateless_team_view_challenge(
                &[0x81; 32],
                &[0x84; 32],
                &[0x82; 32],
                &authority,
                &[0x83; 16],
                2_000_000,
                1_000_011,
            )
            .unwrap()
            .is_none());
        let resurrected: i64 = connection
            .query_row(
                "SELECT count(*) FROM team_view_tokens WHERE token_hash = ?1",
                [[0x82; 32]],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(resurrected, 0);

        let admin = reader
            .team_admin_authority(&team, &[1_u8; 33], 3, 1)
            .unwrap()
            .unwrap();
        assert_eq!(admin.holder_id, vec![1_u8; 33]);
        fixture
            .database
            .issue_team_admin_token(TeamAdminTokenIssue {
                token_hash: &[0x90; 32],
                binding: TeamAdminTokenBinding {
                    team_id: &team,
                    holder_id: &[1_u8; 33],
                    ptk_role_type: 3,
                    ptk_generation: 1,
                },
                expires_at: 2_000_100,
                now: 1_000_008,
            })
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
            fixture.database.activate_team_admin_token(admin_activation(
                &[0x90; 32],
                &[0x91; 32],
                1_000_009,
                &team,
            )),
            Err(foks_server_db::Error::Invalid(
                "team-admin token already activated or invalid"
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
            .activate_team_admin_token(
                admin_activation(&[0x90; 32], &[0x91; 32], 1_000_009, &team,)
            )
            .unwrap()
            .is_some());
        assert!(fixture
            .database
            .activate_team_admin_token(
                admin_activation(&[0x90; 32], &[0x91; 32], 1_000_010, &team,)
            )
            .unwrap()
            .is_some());
        assert!(fixture
            .database
            .activate_team_admin_token(admin_activation(
                &[0x90; 32],
                &[0x91; 32],
                1_000_011,
                &[0xAB; 33],
            ))
            .unwrap()
            .is_none());
        assert!(matches!(
            fixture.database.activate_team_admin_token(admin_activation(
                &[0x90; 32],
                &[0x92; 32],
                1_000_010,
                &team,
            )),
            Err(foks_server_db::Error::OperationConflict)
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
        assert!(
            reader
                .resolve_team_admin_token(&[0x90; 32], 1_000_012)
                .unwrap()
                .is_some(),
            "the bearer holder need not remain a direct roster member"
        );
        connection
            .execute(
                "INSERT INTO team_shared_keys
                 (team_id, role_type, visibility, generation, verify_key, exact_hepk, start_epoch)
                 VALUES (?1, 3, 0, 2, ?2, ?3, 3)",
                rusqlite::params![team, [16_u8; 33], b"rotated-team-hepk"],
            )
            .unwrap();
        assert!(
            reader
                .resolve_team_admin_token(&[0x90; 32], 1_000_013)
                .unwrap()
                .is_none(),
            "a resolved bearer must go stale when its target PTK rotates"
        );
    }
}

#[test]
fn parent_view_authority_survives_child_rotation_until_exact_handoff() {
    let fixture = common::TestDatabase::new();
    let mut parent = [0x21; 33];
    parent[0] = foks_proto::ENTITY_AD_HOC_TEAM;
    let mut child = [0x22; 33];
    child[0] = foks_proto::ENTITY_AD_HOC_TEAM;
    let mut host = [0x23; 33];
    host[0] = foks_proto::ENTITY_HOST;
    let mut old_verify = [0x31; 33];
    old_verify[0] = foks_proto::ENTITY_PTK_VERIFY;
    let mut new_verify = [0x32; 33];
    new_verify[0] = foks_proto::ENTITY_PTK_VERIFY;
    let connection = rusqlite::Connection::open(&fixture.path).unwrap();
    for team in [parent, child] {
        connection
            .execute(
                "INSERT INTO teams
                 (team_id, team_kind, host_id, normalized_name, team_name_utf8,
                  team_name_sequence, team_name_commitment_key, member_load_floor_type,
                  member_load_floor_visibility, created_at)
                 VALUES (?1, ?2, ?3, NULL, X'', 0, NULL, 1, 0, 1)",
                rusqlite::params![team, foks_proto::ENTITY_AD_HOC_TEAM, host],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO team_shared_keys
             (team_id, role_type, visibility, generation, verify_key, exact_hepk, start_epoch)
             VALUES (?1, 2, 0, 1, ?2, X'01', 1),
                    (?1, 2, 0, 2, ?3, X'02', 2)",
            rusqlite::params![child, old_verify, new_verify],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO team_members
             (team_id, party_id, scoped_host_id, source_role_type, source_visibility,
              role_type, visibility, generation, verify_key, hepk_fingerprint,
              removal_key_commitment)
             VALUES (?1, ?2, NULL, 2, 0, 3, 0, 1, ?3, ?4, ?5)",
            rusqlite::params![parent, child, old_verify, [0x41_u8; 32], [0x42_u8; 32]],
        )
        .unwrap();
    let reader =
        foks_server_db::ReadDatabase::open(&fixture.path, foks_server_db::Config::default())
            .unwrap();
    assert!(reader
        .team_view_authority(&parent, &child, &host, 2, 0, 1)
        .unwrap()
        .is_some());
    connection
        .execute(
            "UPDATE team_members SET generation = 2, verify_key = ?3
             WHERE team_id = ?1 AND party_id = ?2",
            rusqlite::params![parent, child, new_verify],
        )
        .unwrap();
    assert!(reader
        .team_view_authority(&parent, &child, &host, 2, 0, 1)
        .unwrap()
        .is_none());
    assert!(reader
        .team_view_authority(&parent, &child, &host, 2, 0, 2)
        .unwrap()
        .is_some());
}
