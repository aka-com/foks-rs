use foks_protocol_metadata::{merge, parse_artifact, parse_policy, MetadataError};

const ARTIFACT: &str = r#"{
  "schema_version": 2,
  "source": {"module":"github.com/foks-proj/go-foks","version":"v0.1.9","sum":"sum","go_mod_sum":"mod","commit":"abc"},
  "protocols": [{"name":"Probe","unique_id":1,"go_file":"proto/rem/probe.go","argument_header":true,"result_header":true,"methods":[{"name":"probe","position":1,"qualified_name":"Probe.probe","result_type":"rem.ProbeRes"}]}],
  "statuses": [{"name":"NOT_IMPLEMENTED","value":1020,"go_file":"proto/lib/status.go"},{"name":"OK","value":0,"go_file":"proto/lib/status.go"}],
  "services": [{"name":"Probe","value":10,"go_file":"proto/lib/common.go"}],
  "sources": [{"path":"proto-src/rem/probe.snowp","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","semantic_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]
}"#;

const POLICY: &str = r#"
version = 1
baseline = "foks-v0.1.9"
[status_aliases]
ok = "OK"
unsupported = "NOT_IMPLEMENTED"
[[protocol]]
name = "Probe"
upstream = "Probe"
id_constant = "PROBE_PROTOCOL_ID"
[[service]]
name = "probe"
upstream = "Probe"
listener = "probe"
supported = true
[[route]]
protocol = "Probe"
method = "probe"
position_constant = "PROBE_METHOD_POSITION"
listeners = ["probe"]
authentication = "public"
request = "ProbeArgument"
result = "ProbeResponse"
statuses = ["ok"]
max_request_bytes = 4096
supported = true
coverage = ["probe_conformance"]
"#;

#[test]
fn merges_policy_with_upstream_numbers() {
    let artifact = parse_artifact(ARTIFACT).unwrap();
    let policy = parse_policy(POLICY).unwrap();
    let merged = merge(&artifact, &policy).unwrap();
    assert_eq!(merged.services[0].service_type, 10);
    assert_eq!(merged.routes[0].protocol_id, 1);
    assert_eq!(merged.routes[0].position, 1);
    assert_eq!(merged.status_codes["ok"], 0);
}

#[test]
fn rejects_unknown_policy_method() {
    let artifact = parse_artifact(ARTIFACT).unwrap();
    let policy =
        parse_policy(&POLICY.replace("method = \"probe\"", "method = \"missing\"")).unwrap();
    assert!(matches!(
        merge(&artifact, &policy),
        Err(MetadataError::Invalid(_))
    ));
}

#[test]
fn rejects_duplicate_wire_positions() {
    let duplicate = ARTIFACT.replace(
        "]}],\n  \"statuses\"",
        ",{\"name\":\"other\",\"position\":1,\"qualified_name\":\"Probe.other\",\"result_type\":\"void\"}]}],\n  \"statuses\"",
    );
    assert!(matches!(
        parse_artifact(&duplicate),
        Err(MetadataError::Invalid(_))
    ));
}

#[test]
fn rejects_executable_policy_for_unsupported_route() {
    let artifact = parse_artifact(ARTIFACT).unwrap();
    let malformed = POLICY.replace("supported = true\ncoverage", "supported = false\ncoverage");
    let policy = parse_policy(&malformed).unwrap();
    assert!(matches!(
        merge(&artifact, &policy),
        Err(MetadataError::Invalid(_))
    ));
}

#[test]
fn route_listeners_must_be_nonempty_unique_and_known() {
    let artifact = parse_artifact(ARTIFACT).unwrap();
    for malformed in [
        POLICY.replace("listeners = [\"probe\"]", "listeners = []"),
        POLICY.replace(
            "listeners = [\"probe\"]",
            "listeners = [\"probe\", \"probe\"]",
        ),
        POLICY.replace("listeners = [\"probe\"]", "listeners = [\"unknown\"]"),
    ] {
        let policy = parse_policy(&malformed).unwrap();
        assert!(matches!(
            merge(&artifact, &policy),
            Err(MetadataError::Invalid(_))
        ));
    }
}

#[test]
fn principal_bound_routes_must_use_the_authenticated_listener() {
    let artifact = parse_artifact(ARTIFACT).unwrap();
    let malformed = POLICY.replace(
        "authentication = \"public\"",
        "authentication = \"active_device_mtls\"",
    );
    let policy = parse_policy(&malformed).unwrap();
    assert!(matches!(
        merge(&artifact, &policy),
        Err(MetadataError::Invalid(_))
    ));
}

#[test]
fn rejects_asymmetric_protocol_headers() {
    let artifact =
        parse_artifact(&ARTIFACT.replace("\"result_header\":true", "\"result_header\":false"))
            .unwrap();
    let policy = parse_policy(POLICY).unwrap();
    assert!(matches!(
        merge(&artifact, &policy),
        Err(MetadataError::Invalid(_))
    ));
}
