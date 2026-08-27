use foks_client::{
    AdHocTeamSecrets, AddLocalTeamMemberRequest, KvWriteOptions, NamedTeamSecrets,
    RemoveTeamMemberRequest, TeamPtkRotationSeed,
};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::{TestAccountSpec, TestClient};

use crate::support::Fixture;
use std::io::Cursor;

#[test]
pub(crate) fn public_client_creates_and_loads_named_and_adhoc_teams() {
    let fixture = Fixture::start("team-create-client");
    let account = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("teamowner", 0x31))
        .unwrap();
    let adhoc = AdHocTeamSecrets {
        member_min: SecretSeed::new([0x41; 32]),
        member: SecretSeed::new([0x42; 32]),
        admin: SecretSeed::new([0x43; 32]),
        owner: SecretSeed::new([0x44; 32]),
    };
    let created_adhoc = fixture
        .client
        .foks()
        .create_single_owner_adhoc_team(fixture.host(), &account.credential, &adhoc)
        .unwrap();
    assert_eq!(created_adhoc.authenticated.verified.chain_seqno(), 1);
    assert_eq!(created_adhoc.authenticated.verified.team_name(), b"-");
    assert_eq!(created_adhoc.authenticated.ptks.len(), 4);
    assert_eq!(
        created_adhoc.authenticated.verified.members()[0].role,
        Role::OWNER
    );

    let named = NamedTeamSecrets {
        member_min: SecretSeed::new([0x51; 32]),
        member: SecretSeed::new([0x52; 32]),
        admin: SecretSeed::new([0x53; 32]),
        owner: SecretSeed::new([0x54; 32]),
        removal_key: SecretSeed::new([0x55; 32]),
        team_name_commitment_key: [0x56; 16],
    };
    let created_named = fixture
        .client
        .foks()
        .create_single_owner_named_team(fixture.host(), &account.credential, "smallteam", &named)
        .unwrap();
    assert_eq!(created_named.authenticated.verified.chain_seqno(), 1);
    assert_eq!(
        created_named.authenticated.verified.team_name(),
        b"smallteam"
    );
    assert_eq!(created_named.authenticated.ptks.len(), 4);

    let target_client = TestClient::new(&fixture.environment, "team-member-client").unwrap();
    let target_host = target_client.probe_and_pin().unwrap();
    let target = target_client
        .create_account(
            &target_host.pinned,
            &TestAccountSpec::new("teammember", 0x61),
        )
        .unwrap();
    let removal_key = SecretSeed::new([0x71; 32]);
    let added = fixture
        .client
        .foks()
        .add_local_user_to_named_team(
            fixture.host(),
            &account.credential,
            &created_named.team,
            &AddLocalTeamMemberRequest {
                target_user: &target.authenticated.verified,
                destination_role: Role::member(0),
                removal_key: &removal_key,
            },
        )
        .unwrap();
    assert_eq!(added.authenticated.verified.chain_seqno(), 2);
    assert_eq!(added.authenticated.verified.members().len(), 2);
    let target_view = target_client
        .foks()
        .load_and_pin_team(
            &target_host.pinned,
            &target.credential,
            &target.authenticated.verified,
            &target.authenticated.puks,
            &created_named.team,
        )
        .unwrap();
    assert_eq!(target_view.ptks.len(), 2);

    let member_role = Role::member(0);
    let member_options = KvWriteOptions {
        read_role: member_role,
        write_role: member_role,
        overwrite: false,
        expected_version: None,
    };
    let mut owner_protected = fixture.client.open_protected_store().unwrap();
    let mut owner_kv = fixture
        .client
        .foks()
        .team_kv_write_session(
            fixture.host(),
            &account.credential,
            &added.authenticated,
            fixture.client.soft_state_path(),
            &mut owner_protected,
        )
        .unwrap();
    let team_tree = owner_kv.ensure_root(member_role, Role::OWNER).unwrap();
    let team_root = team_tree[0].root_directory_id;
    let member_root = owner_kv
        .mkdir(team_root, "member-area", member_options)
        .unwrap()
        .node_id
        .object_id();
    drop(owner_kv);
    let mut target_protected = target_client.open_protected_store().unwrap();
    let mut target_kv = target_client
        .foks()
        .team_kv_write_session(
            &target_host.pinned,
            &target.credential,
            &target_view,
            target_client.soft_state_path(),
            &mut target_protected,
        )
        .unwrap();
    target_kv
        .put_file(
            member_root,
            "member.txt",
            &mut Cursor::new(b"member team content"),
            member_options,
        )
        .unwrap();
    assert!(target_kv
        .put_file(
            team_root,
            "forbidden.txt",
            &mut Cursor::new(b"must not be written"),
            member_options,
        )
        .is_err());
    let target_tree = target_kv.sync().unwrap();
    assert_eq!(target_tree.len(), 2);
    assert_eq!(
        target_tree
            .iter()
            .find(|directory| directory.directory_id == member_root)
            .unwrap()
            .entries
            .len(),
        1
    );
    drop(target_kv);

    let rotated_min = SecretSeed::new([0x72; 32]);
    let rotated_member = SecretSeed::new([0x73; 32]);
    let rotations = [
        TeamPtkRotationSeed {
            role: Role::member(-0x4000),
            seed: &rotated_min,
        },
        TeamPtkRotationSeed {
            role: Role::member(0),
            seed: &rotated_member,
        },
    ];
    let removed = fixture
        .client
        .foks()
        .remove_team_member_and_rotate_ptks(
            fixture.host(),
            &account.credential,
            &created_named.team,
            &RemoveTeamMemberRequest {
                target: target.authenticated.verified.uid(),
                rotations: &rotations,
                remaining_users: &[],
            },
        )
        .unwrap();
    assert_eq!(removed.authenticated.verified.chain_seqno(), 3);
    assert_eq!(removed.authenticated.verified.members().len(), 1);
    assert!(target_client
        .foks()
        .sync_team_kv(
            &target_host.pinned,
            &target.credential,
            &target_view,
            target_client.soft_state_path(),
        )
        .is_err());
    let mut rotated_protected = fixture.client.open_protected_store().unwrap();
    let mut rotated_kv = fixture
        .client
        .foks()
        .team_kv_write_session(
            fixture.host(),
            &account.credential,
            &removed.authenticated,
            fixture.client.soft_state_path(),
            &mut rotated_protected,
        )
        .unwrap();
    let before = rotated_kv.sync().unwrap();
    assert!(before
        .iter()
        .flat_map(|directory| &directory.entries)
        .any(|entry| entry.content.as_deref() == Some(b"member team content".as_slice())));
    rotated_kv
        .put_file(
            member_root,
            "after-rotation.txt",
            &mut Cursor::new(b"rotated PTK content"),
            member_options,
        )
        .unwrap();
    let rotated_tree = rotated_kv.sync().unwrap();
    assert_eq!(
        rotated_tree
            .iter()
            .find(|directory| directory.directory_id == member_root)
            .unwrap()
            .entries
            .len(),
        2
    );
    assert!(target_client
        .foks()
        .load_and_pin_team(
            &target_host.pinned,
            &target.credential,
            &target.authenticated.verified,
            &target.authenticated.puks,
            &created_named.team,
        )
        .is_err());
}
