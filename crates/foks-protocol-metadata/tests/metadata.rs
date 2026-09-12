use foks_protocol_metadata::{
    merge, parse_artifact, parse_policy, render_protocol_ids, MetadataError,
};

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

fn local_policy() -> foks_protocol_metadata::Policy {
    let mut policy = parse_policy(POLICY).unwrap();
    let mut route = policy.routes[0].clone();
    route.method = "fennecChatCapabilities".into();
    route.local_position = Some(65536);
    route.position_constant = Some("CHAT_CAPABILITIES_METHOD_POSITION".into());
    route.listeners = vec!["authenticated".into()];
    route.authentication = "active_device_mtls".into();
    policy.routes.push(route);
    policy
}

#[test]
fn local_extensions_use_the_same_policy_and_generation_path() {
    let artifact = parse_artifact(ARTIFACT).unwrap();
    let policy = local_policy();
    let merged = merge(&artifact, &policy).unwrap();
    assert_eq!(merged.routes[0].position, 1);
    let extension = &merged.routes[1];
    assert_eq!(extension.protocol_id, 1);
    assert_eq!(extension.position, 65536);
    assert_eq!(extension.upstream_result, "<local-extension>");
    assert!(extension.argument_header && extension.result_header);
    assert!(render_protocol_ids(&merged)
        .contains("pub const CHAT_CAPABILITIES_METHOD_POSITION: u64 = 65536;"));
    assert!(foks_protocol_metadata::render_routes(&merged)
        .contains("RouteId::ProbeFennecChatCapabilities"));
    assert!(foks_protocol_metadata::render_contract(&merged)
        .contains("upstream_result = \"<local-extension>\""));
}

#[test]
fn extensions_cannot_override_upstream_or_bypass_policy() {
    let artifact = parse_artifact(ARTIFACT).unwrap();
    for mutate in [
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].local_position = Some(1),
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].local_position = Some(131072),
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].local_position = None,
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].upstream_method = Some("probe".into()),
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].method = "probe".into(),
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].method = "fennec".into(),
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].method = "fennec-Chat".into(),
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].position_constant = None,
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].listeners = vec!["probe".into()],
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].statuses = vec!["invented".into()],
        |p: &mut foks_protocol_metadata::Policy| p.routes[1].coverage.clear(),
        |p: &mut foks_protocol_metadata::Policy| {
            let mut duplicate = p.routes[1].clone();
            duplicate.method = "fennecOther".into();
            duplicate.position_constant = Some("OTHER_METHOD_POSITION".into());
            p.routes.push(duplicate);
        },
    ] {
        let mut policy = local_policy();
        mutate(&mut policy);
        assert!(merge(&artifact, &policy).is_err(), "accepted {policy:?}");
    }
    // An upstream method need not be exposed locally to reserve its dispatch key.
    let mut future = artifact;
    future.protocols[0].methods[0].position = 65536;
    let mut local_only = local_policy();
    local_only.routes.remove(0);
    assert!(merge(&future, &local_only).is_err());
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
fn preserves_asymmetric_protocol_headers_by_direction() {
    let artifact =
        parse_artifact(&ARTIFACT.replace("\"result_header\":true", "\"result_header\":false"))
            .unwrap();
    let policy = parse_policy(POLICY).unwrap();
    let merged = merge(&artifact, &policy).unwrap();
    let rendered = render_protocol_ids(&merged);
    assert!(rendered.contains("pub const fn is_headerless_argument_protocol"));
    assert!(rendered.contains("pub const fn is_headerless_result_protocol"));
    let argument = rendered
        .split("pub const fn is_headerless_argument_protocol")
        .nth(1)
        .unwrap()
        .split("pub const fn is_headerless_result_protocol")
        .next()
        .unwrap();
    let result = rendered
        .split("pub const fn is_headerless_result_protocol")
        .nth(1)
        .unwrap();
    assert!(!argument.contains("0x00000001"));
    assert!(result.contains("0x00000001"));
}

#[test]
fn rejects_unimplemented_local_result_labels() {
    let artifact = parse_artifact(ARTIFACT).unwrap();
    let policy =
        parse_policy(&POLICY.replace("result = \"ProbeResponse\"", "result = \"Banana\"")).unwrap();
    assert!(matches!(
        merge(&artifact, &policy),
        Err(MetadataError::Invalid(_))
    ));
}
