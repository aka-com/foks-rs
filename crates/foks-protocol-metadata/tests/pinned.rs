use foks_protocol_metadata::{
    merge, parse_artifact, parse_policy, render_contract, render_digest_manifest,
    render_protocol_ids, render_routes, render_status_codes, PINNED_PROTOCOL_METADATA_SHA256,
};

const UPSTREAM: &str = include_str!("../../foks-server/protocol/upstream-v0.1.9.json");
const POLICY: &str = include_str!("../../foks-server/protocol/policy-v1.toml");
const PROTOCOL_IDS: &str = include_str!("../../foks-rpc/src/generated/protocol_ids.rs");
const STATUS_CODES: &str = include_str!("../../foks-rpc/src/generated/status_codes.rs");
const ROUTES: &str = include_str!("../../foks-server/src/rpc/generated/routes.rs");
const CONTRACT: &str = include_str!("../../foks-server/protocol-v1.toml");
const DIGEST: &str = include_str!("../../foks-server/protocol/upstream-v0.1.9.sha256");

#[test]
fn checked_outputs_are_the_exact_policy_merge() {
    let artifact = parse_artifact(UPSTREAM).unwrap();
    let policy = parse_policy(POLICY).unwrap();
    let merged = merge(&artifact, &policy).unwrap();
    assert_eq!(render_protocol_ids(&merged), PROTOCOL_IDS);
    assert_eq!(render_status_codes(&merged), STATUS_CODES);
    assert_eq!(render_routes(&merged), ROUTES);
    assert_eq!(render_contract(&merged), CONTRACT);
    assert_eq!(
        render_digest_manifest(UPSTREAM.as_bytes(), &artifact),
        DIGEST
    );
}

#[test]
fn client_policy_pin_is_the_checked_artifact_digest() {
    let digest = DIGEST
        .lines()
        .find_map(|line| line.strip_prefix("artifact_sha256  "))
        .expect("digest manifest has an artifact checksum");
    assert_eq!(PINNED_PROTOCOL_METADATA_SHA256, digest);
}

#[test]
fn baseline_is_the_checksum_pinned_v019_module() {
    let artifact = parse_artifact(UPSTREAM).unwrap();
    assert_eq!(artifact.source.module, "github.com/foks-proj/go-foks");
    assert_eq!(artifact.source.version, "v0.1.9");
    assert_eq!(
        artifact.source.sum,
        "h1:esVU4H00tL7kwyZXbPxfgTDJSBeqGRa5xPFv5xb/Ph0="
    );
    assert_eq!(
        artifact.source.go_mod_sum,
        "h1:x2wPw159s6fYDrvdpTCqqML392rkjqAniUCdkRIkudY="
    );
    assert_eq!(
        artifact.source.commit,
        "f07a5816f54120f5fb4985cf980a7c45d74449b1"
    );
}

#[test]
fn merkle_root_result_types_are_exactly_pinned() {
    let artifact = parse_artifact(UPSTREAM).unwrap();
    let protocol = artifact
        .protocols
        .iter()
        .find(|protocol| protocol.name == "MerkleQuery")
        .unwrap();
    let result_at = |position| {
        protocol
            .methods
            .iter()
            .find(|method| method.position == position)
            .map(|method| method.result_type.as_str())
            .unwrap()
    };
    assert_eq!(result_at(2), "lib.MerkleRoot");
    assert_eq!(result_at(3), "lib.TreeRoot");
    assert_eq!(result_at(5), "lib.SignedMerkleRoot");
}
