use foks_client::NamedTeamSecrets;
use foks_client_db::HardStateStore;
use foks_proto::{SecretSeed, CHAIN_TYPE_TEAM_MEMBERSHIP};
use foks_server_testkit::TestAccountSpec;
use foks_snowpack::{decode, encode, Value};

use crate::support::Fixture;

fn secrets(fill: u8) -> NamedTeamSecrets {
    NamedTeamSecrets {
        member_min: SecretSeed::new([fill; 32]),
        member: SecretSeed::new([fill + 1; 32]),
        admin: SecretSeed::new([fill + 2; 32]),
        owner: SecretSeed::new([fill + 3; 32]),
        removal_key: SecretSeed::new([fill + 4; 32]),
        team_name_commitment_key: [fill + 5; 16],
    }
}

/// The transcript a complete load persists, rebuilt here from the server's
/// own full response so an incremental load's transcript can be compared to
/// it byte for byte.
fn full_transcript(fixture: &Fixture, credential: &foks_client::DeviceCredential) -> Vec<u8> {
    let chain = fixture
        .client
        .foks()
        .load_generic_chain(fixture.host(), credential, CHAIN_TYPE_TEAM_MEMBERSHIP, 1)
        .unwrap();
    let mut values = vec![Value::Unsigned(CHAIN_TYPE_TEAM_MEMBERSHIP)];
    values.extend(
        chain
            .links
            .iter()
            .map(|link| decode(&link.encoded().unwrap()).unwrap()),
    );
    encode(&Value::Array(values)).unwrap()
}

fn pinned(fixture: &Fixture, uid: &foks_proto::EntityId) -> foks_client_db::StoredUserGenericChain {
    HardStateStore::open(fixture.client.hard_state_path())
        .unwrap()
        .user_generic_chain(
            fixture.host().host_id().as_bytes(),
            uid.as_bytes(),
            CHAIN_TYPE_TEAM_MEMBERSHIP,
        )
        .unwrap()
        .expect("an accepted membership chain is pinned")
}

#[test]
fn membership_chains_resume_from_their_pinned_tail_and_match_a_full_load() {
    let fixture = Fixture::start("membership-increment");
    let account = fixture
        .client
        .create_account(
            fixture.host(),
            &TestAccountSpec::new("incrementowner", 0xc7),
        )
        .unwrap();
    let first = fixture
        .client
        .foks()
        .create_single_owner_named_team(
            fixture.host(),
            &account.credential,
            "incrementone",
            &secrets(0x21),
        )
        .unwrap();
    let user = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &account.credential)
        .unwrap();
    let complete = fixture
        .client
        .foks()
        .authenticated_user_team_memberships(fixture.host(), &account.credential, &user.verified)
        .unwrap();
    assert_eq!(complete.active().count(), 1);
    let tail = pinned(&fixture, &account.credential.uid);
    assert_eq!(tail.sequence, 1);
    assert_eq!(
        tail.chain_bytes,
        full_transcript(&fixture, &account.credential)
    );

    let second = fixture
        .client
        .foks()
        .create_single_owner_named_team(
            fixture.host(),
            &account.credential,
            "incrementtwo",
            &secrets(0x31),
        )
        .unwrap();
    let user = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &account.credential)
        .unwrap();
    let resumed = fixture
        .client
        .foks()
        .authenticated_user_team_memberships(fixture.host(), &account.credential, &user.verified)
        .unwrap();
    assert_eq!(resumed.active().count(), 2);
    assert!(resumed
        .active()
        .any(|event| event.membership.team == first.team));
    assert!(resumed
        .active()
        .any(|event| event.membership.team == second.team));
    assert_eq!(resumed.next_sequence, 3);
    let advanced = pinned(&fixture, &account.credential.uid);
    assert_eq!(advanced.sequence, 2);
    // The transcript a resumed load persists is the one a complete load
    // would have persisted, link value for link value.
    assert_eq!(
        advanced.chain_bytes,
        full_transcript(&fixture, &account.credential)
    );

    // A resume with nothing to fetch returns the same verified set and leaves
    // the pinned transcript untouched.
    let unchanged = fixture
        .client
        .foks()
        .authenticated_user_team_memberships(fixture.host(), &account.credential, &user.verified)
        .unwrap();
    assert_eq!(unchanged.events, resumed.events);
    assert_eq!(unchanged.current, resumed.current);
    assert_eq!(unchanged.next_sequence, resumed.next_sequence);
    assert_eq!(
        pinned(&fixture, &account.credential.uid).chain_bytes,
        advanced.chain_bytes
    );

    // The route serves the suffix the increment verifier expects: one leading
    // location for the link it starts at, then each returned link's own.
    let suffix = fixture
        .client
        .foks()
        .load_generic_chain(
            fixture.host(),
            &account.credential,
            CHAIN_TYPE_TEAM_MEMBERSHIP,
            2,
        )
        .unwrap();
    assert_eq!(suffix.links.len(), 1);
    assert_eq!(suffix.locations.len(), 2);
    assert_eq!(suffix.merkle.paths().len(), 2);

    // A pin ahead of the server's chain — the shape a server-side chain reset
    // leaves behind — is refused with the status the client answers by
    // reloading from the eldest link.
    let beyond = fixture.client.foks().load_generic_chain(
        fixture.host(),
        &account.credential,
        CHAIN_TYPE_TEAM_MEMBERSHIP,
        4,
    );
    assert!(matches!(
        beyond,
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: foks_rpc::STATUS_BAD_ARGS_ERROR,
            ..
        }))
    ));
}

#[test]
fn a_membership_pin_left_behind_still_resumes_onto_the_same_verified_set() {
    let fixture = Fixture::start("membership-increment-stale");
    let account = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("staleowner", 0xc9))
        .unwrap();
    for (index, name) in ["staleone", "staletwo"].into_iter().enumerate() {
        fixture
            .client
            .foks()
            .create_single_owner_named_team(
                fixture.host(),
                &account.credential,
                name,
                &secrets(0x41 + (index as u8) * 0x10),
            )
            .unwrap();
    }
    let user = fixture
        .client
        .foks()
        .authenticate_and_pin(fixture.host(), &account.credential)
        .unwrap();
    let complete = fixture
        .client
        .foks()
        .authenticated_user_team_memberships(fixture.host(), &account.credential, &user.verified)
        .unwrap();
    assert_eq!(complete.active().count(), 2);
    let full_bytes = pinned(&fixture, &account.credential.uid).chain_bytes;

    // Roll the pin back to its first link, the state a client that stopped
    // between two membership links is left in.
    let Value::Array(values) = decode(&full_bytes).unwrap() else {
        panic!("pinned transcript is not an array");
    };
    let behind = encode(&Value::Array(values[..2].to_vec())).unwrap();
    let first_link_hash = foks_crypto::prefixed_hash_signable(
        foks_proto::LINK_OUTER_TYPE_ID,
        &encode(&values[1]).unwrap(),
    )
    .unwrap();
    rusqlite::Connection::open(fixture.client.hard_state_path())
        .unwrap()
        .execute(
            "UPDATE user_generic_chains SET seqno = ?1, tail_hash = ?2, chain_bytes = ?3
             WHERE uid = ?4 AND chain_type = ?5",
            rusqlite::params![
                1_i64,
                first_link_hash.as_slice(),
                behind,
                account.credential.uid.as_bytes(),
                CHAIN_TYPE_TEAM_MEMBERSHIP as i64,
            ],
        )
        .unwrap();
    assert_eq!(pinned(&fixture, &account.credential.uid).sequence, 1);

    let resumed = fixture
        .client
        .foks()
        .authenticated_user_team_memberships(fixture.host(), &account.credential, &user.verified)
        .unwrap();
    assert_eq!(resumed.active().count(), complete.active().count());
    assert_eq!(resumed.events, complete.events);
    assert_eq!(resumed.current, complete.current);
    assert_eq!(resumed.next_sequence, complete.next_sequence);
    let readvanced = pinned(&fixture, &account.credential.uid);
    assert_eq!(readvanced.sequence, 2);
    assert_eq!(readvanced.chain_bytes, full_bytes);
}
