use foks_proto::KvPathVersionVector;
use foks_rpc::{
    encode_status_response_at, encode_void_success_response_at, read_void_response, RpcStatus,
    DEFAULT_MAX_FRAME_LENGTH,
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
fn every_typed_status_is_observed_as_an_application_error() {
    let versions = KvPathVersionVector::decode(&fixture("kv-path-version-vector.snowp")).unwrap();
    let statuses = [
        RpcStatus::BadArguments("bad input".into()),
        RpcStatus::DeviceAlreadyProvisioned,
        RpcStatus::Expired,
        RpcStatus::Locked,
        RpcStatus::KvNoEnt,
        RpcStatus::KvPermission {
            operation: 0,
            resource: 0,
        },
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
        RpcStatus::Unsupported,
    ];
    for status in statuses {
        let response = encode_status_response_at(&status, 7).unwrap();
        let mut response = response.as_slice();
        assert!(read_void_response(&mut response, DEFAULT_MAX_FRAME_LENGTH, 7).is_err());
        assert!(response.is_empty());
    }
}
