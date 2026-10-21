//! The team view re-mint arc against the Rust server (PLAN.md P1-1, the
//! admit / write / refuse / write sequence of go-foks `5a9ff13`): a member's
//! team KV write session whose view the server refuses activates one
//! replacement and replays the request when the team keys are unchanged; when
//! a role change rotated them the replacement is refused, nothing is replayed
//! and the session returns the server's refusal.

use std::cell::Cell;
use std::io::Cursor;

use foks_client::{
    AddLocalTeamMemberRequest, ChangeTeamMemberRequest, KvWriteOptions, NamedTeamSecrets,
    TeamMemberSelector, TeamPtkRotationSeed, VerifiedMemberParty,
};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::{TestAccountSpec, TestClient};

use crate::support::Fixture;

/// Domain separation for stored team view token hashes.
const TEAM_VIEW_TOKEN_TYPE_ID: u64 = 0x6d10_7e4a_464f_4b53;

fn member_options() -> KvWriteOptions {
    KvWriteOptions {
        read_role: Role::member(0),
        write_role: Role::member(0),
        overwrite: false,
        expected_version: None,
    }
}

#[test]
fn team_view_remint_replays_after_reclaim_and_refuses_after_key_rotation() {
    let fixture = Fixture::start("team-view-remint");
    let owner = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("remintowner", 0x91))
        .unwrap();
    let created = fixture
        .client
        .foks()
        .create_single_owner_named_team(
            fixture.host(),
            &owner.credential,
            "remintteam",
            &NamedTeamSecrets {
                member_min: SecretSeed::new([0x92; 32]),
                member: SecretSeed::new([0x93; 32]),
                admin: SecretSeed::new([0x94; 32]),
                owner: SecretSeed::new([0x95; 32]),
                removal_key: SecretSeed::new([0x96; 32]),
                team_name_commitment_key: [0x97; 16],
            },
        )
        .unwrap();

    // Admit a member at Member(0).
    let member_client = TestClient::new(&fixture.environment, "team-view-remint-member").unwrap();
    let member_host = member_client.probe_and_pin().unwrap();
    let member = member_client
        .create_account(
            &member_host.pinned,
            &TestAccountSpec::new("remintmember", 0x98),
        )
        .unwrap();
    fixture
        .client
        .foks()
        .add_local_user_to_named_team(
            fixture.host(),
            &owner.credential,
            &created.team,
            &AddLocalTeamMemberRequest {
                target_user: &member.authenticated.verified,
                destination_role: Role::member(0),
                removal_key: &SecretSeed::new([0x99; 32]),
            },
        )
        .unwrap();

    // The owner provisions a root members can write.
    let owner_user = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &owner.credential)
        .unwrap();
    let owner_team = fixture
        .client
        .foks()
        .load_and_pin_team(
            fixture.host(),
            &owner.credential,
            &owner_user.verified,
            &owner_user.puks,
            &created.team,
        )
        .unwrap();
    let mut owner_protected = fixture.client.open_protected_store().unwrap();
    let root = fixture
        .client
        .foks()
        .team_kv_write_session(
            fixture.host(),
            &owner.credential,
            &owner_team,
            fixture.client.soft_state_path(),
            &mut owner_protected,
        )
        .unwrap()
        .ensure_root(Role::member(0), Role::member(0))
        .unwrap()[0]
        .root_directory_id;

    let foks = member_client.foks();
    let host = &member_host.pinned;
    let credential = &member.credential;
    let member_user = foks.authenticate_and_pin(host, credential).unwrap();
    let load = || {
        foks.load_and_pin_team(
            host,
            credential,
            &member_user.verified,
            &member_user.puks,
            &created.team,
        )
        .unwrap()
    };
    let remints = Cell::new(0);
    let mut protected = member_client.open_protected_store().unwrap();

    // Admitted member writes, then the server reclaims its view (as it does
    // past the per-member view limit or after expiry) and the member writes
    // again: the server answers TEAM_VO_BEARER_TOKEN_NOT_FOUND_ERROR, the
    // session re-mints once and the replayed request succeeds.
    let team = load();
    {
        let mut session = foks
            .team_kv_write_session(
                host,
                credential,
                &team,
                member_client.soft_state_path(),
                &mut protected,
            )
            .unwrap();
        session.set_team_view_remint(Box::new(|| {
            remints.set(remints.get() + 1);
            foks.remint_team_view(
                host,
                credential,
                &member_user.verified,
                &member_user.puks,
                &team,
            )
        }));
        session
            .put_file(
                root,
                "first.txt",
                &mut Cursor::new(b"before"),
                member_options(),
            )
            .unwrap();
        assert_eq!(remints.get(), 0);
        let database = rusqlite::Connection::open(fixture.environment.database_path()).unwrap();
        let reclaimed = database
            .execute(
                "DELETE FROM team_view_tokens WHERE token_hash = ?1",
                [foks_crypto::prefixed_hash(
                    TEAM_VIEW_TOKEN_TYPE_ID,
                    &team.view_token,
                )],
            )
            .unwrap();
        assert_eq!(reclaimed, 1);
        session
            .put_file(
                root,
                "second.txt",
                &mut Cursor::new(b"after"),
                member_options(),
            )
            .unwrap();
        assert_eq!(remints.get(), 1);
    }

    // A role change that rotates the team keys supersedes the member's
    // view: the server answers TEAM_BEARER_TOKEN_STALE_ERROR, and the
    // replacement view is refused because the reloaded team no longer opens
    // the session's keys (its sealed material is for the old generation).
    // The request is not replayed.
    let team = load();
    let mut session = foks
        .team_kv_write_session(
            host,
            credential,
            &team,
            member_client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    remints.set(0);
    session.set_team_view_remint(Box::new(|| {
        remints.set(remints.get() + 1);
        foks.remint_team_view(
            host,
            credential,
            &member_user.verified,
            &member_user.puks,
            &team,
        )
    }));
    let owner_user = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &owner.credential)
        .unwrap();
    let demoted_user = foks.authenticate_and_pin(host, credential).unwrap();
    let selector = TeamMemberSelector {
        party: &member.credential.uid,
        host: None,
        // A user joins with its own owner key as the source role.
        source_role: Role::OWNER,
    };
    let replacement = Some(VerifiedMemberParty::User(&demoted_user.verified));
    let current = fixture
        .client
        .foks()
        .load_and_pin_team(
            fixture.host(),
            &owner.credential,
            &owner_user.verified,
            &owner_user.puks,
            &created.team,
        )
        .unwrap();
    let roles = fixture
        .client
        .foks()
        .team_member_rotation_roles(&current, selector, Role::member(-0x4000), replacement)
        .unwrap();
    assert!(roles.contains(&Role::member(0)), "rotated roles: {roles:?}");
    let seeds = roles
        .iter()
        .enumerate()
        .map(|(index, _)| SecretSeed::new([0xa0 + index as u8; 32]))
        .collect::<Vec<_>>();
    let rotations = roles
        .iter()
        .zip(&seeds)
        .map(|(role, seed)| TeamPtkRotationSeed { role: *role, seed })
        .collect::<Vec<_>>();
    fixture
        .client
        .foks()
        .change_team_member_and_rotate_ptks(
            fixture.host(),
            &owner.credential,
            &created.team,
            &ChangeTeamMemberRequest {
                target: selector,
                destination_role: Role::member(-0x4000),
                replacement,
                rotations: &rotations,
                remaining_parties: &[],
            },
            &mut owner_protected,
        )
        .unwrap();
    let refused = session.put_file(
        root,
        "third.txt",
        &mut Cursor::new(b"refused"),
        member_options(),
    );
    // The session reports the refusal it hit, not the failed recovery.
    assert!(
        matches!(&refused, Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus { code, .. }))
            if *code == foks_rpc::STATUS_TEAM_BEARER_TOKEN_STALE_ERROR),
        "unexpected result after the role change: {:?}",
        refused.err()
    );
    assert_eq!(remints.get(), 1);
    drop(session);
    let replacement = foks.remint_team_view(
        host,
        credential,
        &member_user.verified,
        &member_user.puks,
        &team,
    );
    assert!(
        matches!(&replacement, Err(foks_client::Error::TeamBinding(message))
            if message.contains("team keys changed")),
        "unexpected replacement: {:?}",
        replacement.err()
    );
}
