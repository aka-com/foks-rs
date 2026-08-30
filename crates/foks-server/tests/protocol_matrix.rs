use std::collections::{BTreeMap, BTreeSet, HashSet};

use foks_rpc::{
    encode_call, read_call, DEFAULT_MAX_FRAME_LENGTH, PROBE_METHOD_POSITION, PROBE_PROTOCOL_ID,
    TEAM_LOADER_PROTOCOL_ID, TEAM_LOAD_CHAIN_METHOD_POSITION,
};
use foks_server::rpc::{route_call, Listener, RouteError, RouteId, ROUTES, SERVICES};
use foks_snowpack::{encode, Value};
use serde::Deserialize;

const CONTRACT: &str = include_str!("../protocol-v1.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    version: u64,
    baseline: String,
    upstream_module: String,
    upstream_version: String,
    upstream_sum: String,
    upstream_commit: String,
    status_codes: BTreeMap<String, u64>,
    protocol: Vec<Protocol>,
    service: Vec<Service>,
    route: Vec<Route>,
}

#[test]
fn registered_routes_enforce_listener_and_argument_limits() {
    let call = |method, argument: &[u8]| {
        let frame = encode_call(PROBE_PROTOCOL_ID, method, argument, 0).unwrap();
        read_call(&mut std::io::Cursor::new(frame), DEFAULT_MAX_FRAME_LENGTH).unwrap()
    };
    let nil = encode(&Value::Null).unwrap();
    assert_eq!(
        route_call(call(PROBE_METHOD_POSITION, &nil), Listener::Probe)
            .unwrap()
            .route
            .method,
        "probe"
    );
    assert!(matches!(
        route_call(call(PROBE_METHOD_POSITION, &nil), Listener::Authenticated),
        Err(RouteError::WrongListener)
    ));

    let oversized = encode(&Value::Binary(vec![0; 4097])).unwrap();
    assert!(matches!(
        route_call(call(PROBE_METHOD_POSITION, &oversized), Listener::Probe),
        Err(RouteError::RequestTooLarge { .. })
    ));

    assert!(matches!(
        route_call(call(u64::MAX, &nil), Listener::Probe),
        Err(RouteError::Unknown { .. })
    ));
}

#[test]
fn team_chain_route_is_shared_by_public_and_authenticated_listeners_only() {
    let nil = encode(&Value::Null).unwrap();
    let call = || {
        let frame = encode_call(
            TEAM_LOADER_PROTOCOL_ID,
            TEAM_LOAD_CHAIN_METHOD_POSITION,
            &nil,
            0,
        )
        .unwrap();
        read_call(&mut std::io::Cursor::new(frame), DEFAULT_MAX_FRAME_LENGTH).unwrap()
    };
    let public = route_call(call(), Listener::PublicServices).unwrap();
    let authenticated = route_call(call(), Listener::Authenticated).unwrap();
    assert_eq!(public.route.id, authenticated.route.id);
    assert!(matches!(
        route_call(call(), Listener::Probe),
        Err(RouteError::WrongListener)
    ));
}

#[test]
fn principal_bound_routes_are_authenticated_only() {
    for route in ROUTES.iter().filter(|route| {
        route.authentication.starts_with("active_") || route.authentication.starts_with("current_")
    }) {
        if route.id == RouteId::TeamLoaderLoadTeamChain {
            assert_eq!(route.authentication, "active_team_or_remote_view_token");
            assert_eq!(route.listeners, ["public_services", "authenticated"]);
        } else {
            assert_eq!(
                route.listeners,
                ["authenticated"],
                "principal-bound route {}.{} is exposed on another listener",
                route.protocol,
                route.method
            );
        }
    }
}

#[test]
fn authentication_policy_matches_listener_principal_availability() {
    // The session dispatcher (net/session.rs) constructs a Principal ONLY on the
    // authenticated listener; probe and public_services handlers always receive
    // principal = None. A route's declared `authentication` must therefore agree
    // with where it is reachable: a principal-bound policy (active_*/current_*)
    // can only be enforced where a principal exists, and an unauthenticated
    // listener may only carry the fixed bootstrap/token vocabulary. This guards
    // against future drift between the declared policy and the handler's actual
    // enforcement surface.
    const UNAUTHENTICATED_POLICIES: &[&str] = &[
        "public",
        "delegated_tls",
        "signed_login_challenge",
        "signed_lookup_challenge",
        "signed_signup",
        "signed_subkey_challenge",
        "remote_view_token",
        "team_view_token",
        "self_view_token",
        "delegated_tls_and_view_policy",
    ];
    for route in ROUTES.iter() {
        let principal_bound = route.authentication.starts_with("active_")
            || route.authentication.starts_with("current_");
        let reachable_unauthenticated = route
            .listeners
            .iter()
            .any(|listener| *listener == "probe" || *listener == "public_services");
        if !reachable_unauthenticated {
            // Authenticated-only routes must perform a server-side identity check:
            // a principal-bound policy or a bearer/admin token, never a pure
            // bootstrap policy that assumes no caller identity.
            assert!(
                principal_bound || route.authentication.ends_with("_token"),
                "authenticated-only route {}.{} uses unexpected policy {}",
                route.protocol,
                route.method,
                route.authentication
            );
            continue;
        }
        if route.id == RouteId::TeamLoaderLoadTeamChain {
            // Documented carve-out: also reachable on the authenticated listener,
            // where a principal or a remote-view bearer token authorizes it. On the
            // public path the handler must return only public-verifiable chain data.
            assert_eq!(route.authentication, "active_team_or_remote_view_token");
            assert_eq!(route.listeners, ["public_services", "authenticated"]);
            continue;
        }
        assert!(
            !principal_bound,
            "route {}.{} declares principal-bound policy {} but is reachable on an \
             unauthenticated listener where the dispatcher supplies no principal",
            route.protocol, route.method, route.authentication
        );
        assert!(
            UNAUTHENTICATED_POLICIES.contains(&route.authentication),
            "route {}.{} is reachable without authentication but uses unrecognized \
             policy {}; confirm its handler can enforce this with principal = None",
            route.protocol,
            route.method,
            route.authentication
        );
    }
}

#[test]
fn signup_contract_includes_authoritative_invite_and_saturation_outcomes() {
    let contract: Contract = toml::from_str(CONTRACT).expect("valid protocol-v1.toml");
    let signup = contract
        .route
        .iter()
        .find(|route| route.protocol == "Reg" && route.method == "signup")
        .expect("registration signup route");
    for status in ["bad_invite", "rate_limited"] {
        assert!(
            signup.statuses.iter().any(|candidate| candidate == status),
            "signup contract omits {status}"
        );
    }
}

#[derive(Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
struct Service {
    name: String,
    service_type: u64,
    listener: String,
    supported: bool,
}

#[derive(Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
struct Protocol {
    name: String,
    upstream: String,
    protocol_id: u64,
    argument_header: bool,
    result_header: bool,
}

#[derive(Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
struct Route {
    protocol: String,
    protocol_id: u64,
    method: String,
    position: u64,
    listeners: Vec<String>,
    authentication: String,
    request: String,
    result: String,
    upstream_result: String,
    argument_header: bool,
    result_header: bool,
    statuses: Vec<String>,
    max_request_bytes: usize,
    supported: bool,
    coverage: Vec<String>,
}

#[test]
fn protocol_contract_is_valid_and_exactly_registered() {
    let contract: Contract = toml::from_str(CONTRACT).expect("valid protocol-v1.toml");
    assert_eq!(contract.version, 1);
    assert_eq!(contract.baseline, "foks-v0.1.9");
    assert_eq!(contract.upstream_module, "github.com/foks-proj/go-foks");
    assert_eq!(contract.upstream_version, "v0.1.9");
    assert_eq!(
        contract.upstream_sum,
        "h1:esVU4H00tL7kwyZXbPxfgTDJSBeqGRa5xPFv5xb/Ph0="
    );
    assert_eq!(
        contract.upstream_commit,
        "f07a5816f54120f5fb4985cf980a7c45d74449b1"
    );
    assert_eq!(
        contract.status_codes,
        BTreeMap::from([
            ("bad_args".to_owned(), 1030),
            ("bad_invite".to_owned(), 1019),
            ("bad_passphrase".to_owned(), 1011),
            ("device_already_provisioned".to_owned(), 1072),
            ("expired".to_owned(), 1062),
            ("key_not_found".to_owned(), 1025),
            ("kv_noent".to_owned(), 8016),
            ("kv_permission".to_owned(), 8011),
            ("lock_timeout".to_owned(), 8015),
            ("locked".to_owned(), 8014),
            ("merkle_leaf_not_found".to_owned(), 4002),
            ("merkle_no_root".to_owned(), 4001),
            ("name_in_use".to_owned(), 1023),
            ("not_found".to_owned(), 1049),
            ("ok".to_owned(), 0),
            ("passphrase_not_found".to_owned(), 1043),
            ("permission_denied".to_owned(), 1013),
            ("quota_exceeded".to_owned(), 1060),
            ("rate_limited".to_owned(), 1012),
            ("revoke_race".to_owned(), 1044),
            ("stale_cache".to_owned(), 8012),
            ("stale_root".to_owned(), 1014),
            ("tx_retry".to_owned(), 1014),
            ("team_error".to_owned(), 7001),
            ("team_race".to_owned(), 7002),
            ("team_bearer_token_stale".to_owned(), 7003),
            ("team_not_found".to_owned(), 7004),
            ("team_cert".to_owned(), 7005),
            ("team_roster".to_owned(), 7006),
            ("team_key".to_owned(), 7007),
            ("team_no_source_role".to_owned(), 7008),
            ("team_removal_key".to_owned(), 7009),
            ("team_explore".to_owned(), 7010),
            ("team_adhoc_creator_included".to_owned(), 7101),
            ("team_adhoc_open_viewership".to_owned(), 7102),
            ("team_adhoc_invalid_change".to_owned(), 7103),
            ("team_adhoc_duplicate".to_owned(), 7104),
            ("unsupported".to_owned(), 1020),
            ("user_not_found".to_owned(), 1027),
        ])
    );

    validate_unique_contract_entries(&contract);

    let declared_services = contract.service.into_iter().collect::<BTreeSet<_>>();
    let registered_services = SERVICES
        .iter()
        .map(|service| Service {
            name: service.name.into(),
            service_type: service.service_type,
            listener: service.listener.into(),
            supported: service.supported,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(declared_services, registered_services);

    let declared_routes = contract.route.into_iter().collect::<BTreeSet<_>>();
    let route_ids = ROUTES.iter().map(|route| route.id).collect::<HashSet<_>>();
    assert_eq!(
        route_ids.len(),
        ROUTES.len(),
        "generated route IDs must be unique"
    );
    let registered_routes = ROUTES
        .iter()
        .map(|route| Route {
            protocol: route.protocol.into(),
            protocol_id: route.protocol_id,
            method: route.method.into(),
            position: route.position,
            listeners: route
                .listeners
                .iter()
                .map(|listener| (*listener).into())
                .collect(),
            authentication: route.authentication.into(),
            request: route.request.into(),
            result: route.result.into(),
            upstream_result: route.upstream_result.into(),
            argument_header: route.argument_header,
            result_header: route.result_header,
            statuses: route
                .statuses
                .iter()
                .map(|status| (*status).into())
                .collect(),
            max_request_bytes: route.max_request_bytes,
            supported: route.supported,
            coverage: route
                .coverage
                .iter()
                .map(|coverage| (*coverage).into())
                .collect(),
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(declared_routes, registered_routes);
}

fn validate_unique_contract_entries(contract: &Contract) {
    let mut protocol_names = BTreeSet::new();
    let mut protocol_ids = BTreeSet::new();
    for protocol in &contract.protocol {
        assert!(protocol_names.insert(&protocol.name));
        assert!(protocol_ids.insert(protocol.protocol_id));
        assert!(!protocol.upstream.is_empty());
    }
    let mut service_names = BTreeSet::new();
    let mut service_types = BTreeSet::new();
    for service in &contract.service {
        assert!(service_names.insert(&service.name));
        assert!(service_types.insert(service.service_type));
        assert!(matches!(
            service.listener.as_str(),
            "probe" | "public_services" | "authenticated"
        ));
    }

    let mut route_keys = BTreeSet::new();
    let mut route_names = BTreeSet::new();
    for route in &contract.route {
        let protocol = contract
            .protocol
            .iter()
            .find(|protocol| protocol.name == route.protocol)
            .expect("route references a declared protocol");
        assert_eq!(route.protocol_id, protocol.protocol_id);
        assert_eq!(route.argument_header, protocol.argument_header);
        assert_eq!(route.result_header, protocol.result_header);
        assert!(route_keys.insert((route.protocol_id, route.position)));
        assert!(route_names.insert((&route.protocol, &route.method)));
        assert!(!route.listeners.is_empty());
        assert_eq!(
            route.listeners.iter().collect::<BTreeSet<_>>().len(),
            route.listeners.len()
        );
        assert!(route.listeners.iter().all(|listener| matches!(
            listener.as_str(),
            "probe" | "public_services" | "authenticated"
        )));
        assert!(!route.authentication.is_empty());
        assert!(!route.request.is_empty());
        assert!(!route.result.is_empty());
        assert!(!route.statuses.is_empty());
        assert!(!route.coverage.is_empty());
        assert!(
            route
                .statuses
                .iter()
                .all(|status| contract.status_codes.contains_key(status)),
            "{}::{} uses an unknown status",
            route.protocol,
            route.method
        );
        assert!((1..=16 * 1024 * 1024).contains(&route.max_request_bytes));
        if !route.supported {
            assert_eq!(route.statuses, ["unsupported"]);
        }
    }
}
