use std::io::Cursor;
use std::time::Duration;

use foks_client::{AdHocTeamSecrets, KvWriteOptions, NamedTeamSecrets};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::TestAccountSpec;

use crate::support::Fixture;

fn owner_options() -> KvWriteOptions {
    KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    }
}

#[test]
fn named_and_adhoc_team_kv_are_isolated_from_each_other_and_personal_kv() {
    let fixture = Fixture::start("team-kv-client");
    let account = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("teamkvowner", 0x81))
        .unwrap();
    let named = fixture
        .client
        .foks()
        .create_single_owner_named_team(
            fixture.host(),
            &account.credential,
            "teamkvnamed",
            &NamedTeamSecrets {
                member_min: SecretSeed::new([0x11; 32]),
                member: SecretSeed::new([0x12; 32]),
                admin: SecretSeed::new([0x13; 32]),
                owner: SecretSeed::new([0x14; 32]),
                removal_key: SecretSeed::new([0x15; 32]),
                team_name_commitment_key: [0x16; 16],
            },
        )
        .unwrap();
    let adhoc = fixture
        .client
        .foks()
        .create_single_owner_adhoc_team(
            fixture.host(),
            &account.credential,
            &AdHocTeamSecrets {
                member_min: SecretSeed::new([0x21; 32]),
                member: SecretSeed::new([0x22; 32]),
                admin: SecretSeed::new([0x23; 32]),
                owner: SecretSeed::new([0x24; 32]),
            },
        )
        .unwrap();

    let mut protected = fixture.client.open_protected_store().unwrap();
    let mut named_session = fixture
        .client
        .foks()
        .team_kv_write_session(
            fixture.host(),
            &account.credential,
            &named.authenticated,
            fixture.client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    let named_tree = named_session.ensure_root(Role::OWNER, Role::OWNER).unwrap();
    let named_root = named_tree[0].root_directory_id;
    named_session
        .put_file(
            named_root,
            "named.txt",
            &mut Cursor::new(b"named team content"),
            owner_options(),
        )
        .unwrap();
    named_session
        .put_symlink(named_root, "named-link", "named.txt", owner_options())
        .unwrap();
    let child = named_session
        .mkdir(named_root, "child", owner_options())
        .unwrap()
        .node_id
        .object_id();
    named_session
        .put_file(
            child,
            "nested.txt",
            &mut Cursor::new(b"nested team content"),
            owner_options(),
        )
        .unwrap();
    let target = [0x31; 16];
    let lock = named_session
        .acquire_lock(named_root, target, Duration::from_secs(10))
        .unwrap();
    named_session
        .release_lock(named_root, target, lock)
        .unwrap();
    drop(named_session);

    let mut adhoc_session = fixture
        .client
        .foks()
        .team_kv_write_session(
            fixture.host(),
            &account.credential,
            &adhoc.authenticated,
            fixture.client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    let adhoc_tree = adhoc_session.ensure_root(Role::OWNER, Role::OWNER).unwrap();
    let adhoc_root = adhoc_tree[0].root_directory_id;
    assert_ne!(named_root, adhoc_root);
    adhoc_session
        .put_file(
            adhoc_root,
            "adhoc.txt",
            &mut Cursor::new(b"ad-hoc team content"),
            owner_options(),
        )
        .unwrap();
    let adhoc_projection = adhoc_session.sync().unwrap();
    assert_eq!(adhoc_projection[0].entries.len(), 1);
    assert_eq!(adhoc_projection[0].entries[0].name, b"adhoc.txt");
    drop(adhoc_session);

    let named_projection = fixture
        .client
        .foks()
        .sync_team_kv(
            fixture.host(),
            &account.credential,
            &named.authenticated,
            fixture.client.soft_state_path(),
        )
        .unwrap();
    assert_eq!(named_projection.len(), 2);
    assert_eq!(
        named_projection
            .iter()
            .find(|directory| directory.directory_id == named_root)
            .unwrap()
            .entries
            .len(),
        3
    );
    assert!(account
        .kv_projection
        .iter()
        .all(|directory| directory.root_directory_id != named_root
            && directory.root_directory_id != adhoc_root));
}
