use foks_proto::{KvPathVersionVector, TeamChain};
use foks_rpc::{
    encode_bare_success_response_at, encode_bare_void_success_response_at,
    encode_status_response_at, encode_void_success_response_at, read_bare_response,
    read_bare_void_response, read_void_response, RpcStatus, DEFAULT_MAX_FRAME_LENGTH,
};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../foks-snowpack/tests/fixtures/foks-v0.1.9/user")
            .join(path),
    )
    .unwrap()
}

#[test]
fn stale_cache_error_matches_the_official_go_frame() {
    let versions = KvPathVersionVector::decode(&fixture("kv-path-version-vector.snowp")).unwrap();
    assert_eq!(
        encode_status_response_at(&RpcStatus::StaleCache(versions), 1).unwrap(),
        fixture("kv-stale-cache-response.frame")
    );
}

#[test]
fn identity_lookup_errors_match_the_official_go_frames() {
    assert_eq!(
        encode_status_response_at(&RpcStatus::UserNotFound, 0).unwrap(),
        fixture("reg-check-name-not-found-response.frame")
    );
    assert_eq!(
        encode_status_response_at(&RpcStatus::KeyNotFound("probed key".to_owned()), 0).unwrap(),
        fixture("reg-probe-key-not-found-response.frame")
    );
}

#[test]
fn merkle_lookup_errors_match_the_official_go_frames() {
    assert_eq!(
        encode_status_response_at(&RpcStatus::MerkleNoRoot, 0).unwrap(),
        fixture("merkle-no-root-response.frame")
    );
    assert_eq!(
        encode_status_response_at(&RpcStatus::MerkleLeafNotFound, 0).unwrap(),
        fixture("merkle-leaf-not-found-response.frame")
    );
}

#[test]
fn void_success_is_accepted_by_the_existing_client_decoder() {
    let response = encode_void_success_response_at(42).unwrap();
    let mut response = response.as_slice();
    read_void_response(&mut response, DEFAULT_MAX_FRAME_LENGTH, 42).unwrap();
    assert!(response.is_empty());
}

#[test]
fn void_success_matches_the_official_go_frame() {
    assert_eq!(
        encode_void_success_response_at(0).unwrap(),
        fixture("reg-select-vhost-response.frame")
    );
}

#[test]
fn headerless_void_success_matches_the_official_go_frame() {
    let expected = fixture("team-create-response.frame");
    assert_eq!(encode_bare_void_success_response_at(0).unwrap(), expected);
    let mut response = expected.as_slice();
    read_bare_void_response(&mut response, DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
    assert!(response.is_empty());
}

#[test]
fn headerless_nonvoid_success_matches_the_generated_go_client_envelope() {
    let chain = fixture("team-chain.snowp");
    let expected = fixture("team-load-response.frame");
    assert_eq!(
        encode_bare_success_response_at(&chain, 0).unwrap(),
        expected
    );
    let decoded =
        read_bare_response(&mut expected.as_slice(), DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
    assert_eq!(decoded, chain);
    TeamChain::decode(&decoded).unwrap();
}

#[test]
fn every_typed_status_is_observed_as_an_application_error() {
    let versions = KvPathVersionVector::decode(&fixture("kv-path-version-vector.snowp")).unwrap();
    let statuses = [
        RpcStatus::BadArguments("bad input".into()),
        RpcStatus::DeviceAlreadyProvisioned,
        RpcStatus::Expired,
        RpcStatus::Locked,
        RpcStatus::MerkleLeafNotFound,
        RpcStatus::MerkleNoRoot,
        RpcStatus::MerkleVerify("proof verification failed".into()),
        RpcStatus::KvNoEnt,
        RpcStatus::KvRace("raced".into()),
        RpcStatus::KvPermission {
            operation: 0,
            resource: 0,
        },
        RpcStatus::KeyNotFound("key".into()),
        RpcStatus::NameInUse,
        RpcStatus::NotFound("missing".into()),
        RpcStatus::PermissionDenied("denied".into()),
        RpcStatus::QuotaExceeded,
        RpcStatus::RateLimited,
        RpcStatus::StaleCache(versions),
        RpcStatus::StaleRoot,
        RpcStatus::TransactionRetry,
        RpcStatus::TeamError("team".into()),
        RpcStatus::TeamRace("race".into()),
        RpcStatus::TeamBearerTokenStale("stale".into()),
        RpcStatus::TeamNotFound,
        RpcStatus::TeamCertificate("certificate".into()),
        RpcStatus::TeamRoster("roster".into()),
        RpcStatus::TeamKey("key".into()),
        RpcStatus::TeamNoSourceRole,
        RpcStatus::TeamRemovalKey("removal".into()),
        RpcStatus::TeamExplore("explore".into()),
        RpcStatus::TeamAdHocCreatorIncluded,
        RpcStatus::TeamAdHocOpenViewership,
        RpcStatus::TeamAdHocInvalidChange("change".into()),
        RpcStatus::TeamAdHocDuplicate,
        RpcStatus::UserNotFound,
        RpcStatus::Unsupported,
    ];
    for status in statuses {
        let response = encode_status_response_at(&status, 7).unwrap();
        let mut response = response.as_slice();
        assert!(read_void_response(&mut response, DEFAULT_MAX_FRAME_LENGTH, 7).is_err());
        assert!(response.is_empty());
    }
}

fn main_fixture(path: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../foks-snowpack/tests/fixtures/foks-main/team")
            .join(path),
    )
    .unwrap()
}

/// go-foks main (`5a9ff13`, `df2acf1`) frames generated by
/// `tools/foks-main-oracle` `TestTeamStatusFixtures`.
#[test]
fn team_token_and_roster_statuses_match_the_go_main_frames() {
    for (status, name) in [
        (
            RpcStatus::TeamVOBearerTokenNotFound,
            "vo-bearer-token-not-found-response.frame",
        ),
        (
            RpcStatus::TeamRoster("edit would leave team without an owner".into()),
            "ownerless-edit-roster-error-response.frame",
        ),
    ] {
        let expected = main_fixture(name);
        assert_eq!(encode_status_response_at(&status, 1).unwrap(), expected);
    }

    let frame = main_fixture("vo-bearer-token-not-found-response.frame");
    match read_void_response(&mut frame.as_slice(), DEFAULT_MAX_FRAME_LENGTH, 1) {
        Err(foks_rpc::Error::RemoteStatus { code, detail }) => {
            assert_eq!(code, foks_rpc::STATUS_TEAM_VO_BEARER_TOKEN_NOT_FOUND_ERROR);
            assert_eq!(detail, Default::default());
        }
        other => panic!("unexpected decode {other:?}"),
    }
    let frame = main_fixture("ownerless-edit-roster-error-response.frame");
    match read_void_response(&mut frame.as_slice(), DEFAULT_MAX_FRAME_LENGTH, 1) {
        Err(error @ foks_rpc::Error::RemoteStatus { code, .. }) => {
            assert_eq!(code, foks_rpc::STATUS_TEAM_ROSTER_ERROR);
            assert!(error
                .to_string()
                .contains("edit would leave team without an owner"));
        }
        other => panic!("unexpected decode {other:?}"),
    }
}
