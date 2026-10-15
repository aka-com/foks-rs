//! A database at SQLite's `max_page_count` ceiling reports over quota (1060)
//! on every write route, rather than a retryable or route-specific failure.

use std::io::Cursor;

use foks_client::{AdHocTeamSecrets, KvWriteOptions};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment, TestProfile};

const ATTEMPTS: usize = 64;

fn options() -> KvWriteOptions {
    KvWriteOptions {
        read_role: Role::OWNER,
        write_role: Role::OWNER,
        overwrite: false,
        expected_version: None,
    }
}

fn secrets(seed: u8) -> AdHocTeamSecrets {
    AdHocTeamSecrets {
        member_min: SecretSeed::new([seed; 32]),
        member: SecretSeed::new([seed.wrapping_add(1); 32]),
        admin: SecretSeed::new([seed.wrapping_add(2); 32]),
        owner: SecretSeed::new([seed.wrapping_add(3); 32]),
    }
}

fn assert_over_quota(route: &str, attempts: usize, error: &foks_client::Error) {
    assert!(
        matches!(
            error,
            foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_OVER_QUOTA_ERROR,
                ..
            })
        ),
        "{route} failed after {attempts} successful writes with {error:?}"
    );
}

/// Repeats `write` until it fails and returns the failure. Each success
/// consumes pages, so the ceiling is reached within `ATTEMPTS` writes.
fn until_failure<T>(
    route: &str,
    mut write: impl FnMut(usize) -> foks_client::Result<T>,
) -> (usize, foks_client::Error) {
    for attempt in 0..ATTEMPTS {
        if let Err(error) = write(attempt) {
            return (attempt, error);
        }
    }
    panic!("{route} did not reach the database ceiling in {ATTEMPTS} writes");
}

#[test]
fn full_database_reports_over_quota_on_signup_team_creation_and_kv_writes() {
    let environment = TestEnvironment::with_profile(TestProfile::TinyDatabase).unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "full-database-owner").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let owner = client
        .create_account(&probe.pinned, &TestAccountSpec::new("fullowner", 0xc1))
        .unwrap();
    let root = owner.kv_projection[0].root_directory_id;
    let mut protected = client.open_protected_store().unwrap();
    let mut session = client
        .foks()
        .user_kv_write_session(
            &probe.pinned,
            &owner.credential,
            &owner.authenticated.verified,
            &owner.authenticated.puks,
            client.soft_state_path(),
            &mut protected,
        )
        .unwrap();

    // Fill the free pages with progressively smaller files.
    for size in [64 * 1024, 4 * 1024, 64] {
        let (attempts, error) = until_failure("KV write", |attempt| {
            let name = format!("fill-{size}-{attempt:02}.bin");
            session.put_file(root, &name, &mut Cursor::new(vec![0x46; size]), options())
        });
        assert_over_quota("KV write", attempts, &error);
    }

    let (attempts, error) = until_failure("team creation", |attempt| {
        client.foks().create_single_owner_adhoc_team(
            &probe.pinned,
            &owner.credential,
            &secrets(0x10 + 4 * attempt as u8),
        )
    });
    assert_over_quota("team creation", attempts, &error);

    let (attempts, error) = until_failure("signup", |attempt| {
        let joiner =
            TestClient::new(&environment, &format!("full-database-joiner-{attempt}")).unwrap();
        let probe = joiner.probe_and_pin().unwrap();
        joiner.create_account(
            &probe.pinned,
            &TestAccountSpec::new(format!("fulljoiner{attempt}"), 0x40 + attempt as u8),
        )
    });
    assert_over_quota("signup", attempts, &error);

    server.shutdown().unwrap();
}
