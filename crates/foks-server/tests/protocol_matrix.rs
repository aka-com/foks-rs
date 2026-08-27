use std::collections::{BTreeMap, BTreeSet};

use foks_rpc::{
    encode_call, read_call, DEFAULT_MAX_FRAME_LENGTH, PROBE_METHOD_POSITION, PROBE_PROTOCOL_ID,
};
use foks_server::rpc::{route_call, Listener, RouteError, ROUTES, SERVICES};
use foks_snowpack::{encode, Value};
use serde::Deserialize;

const CONTRACT: &str = include_str!("../protocol-v1.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    version: u64,
    baseline: String,
    status_codes: BTreeMap<String, u64>,
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
struct Route {
    protocol: String,
    protocol_id: u64,
    method: String,
    position: u64,
    listener: String,
    authentication: String,
    request: String,
    result: String,
    statuses: Vec<String>,
    max_request_bytes: usize,
    supported: bool,
}

#[test]
fn protocol_contract_is_valid_and_exactly_registered() {
    let contract: Contract = toml::from_str(CONTRACT).expect("valid protocol-v1.toml");
    assert_eq!(contract.version, 1);
    assert_eq!(contract.baseline, "foks-v0.1.9");
    assert_eq!(
        contract.status_codes,
        BTreeMap::from([
            ("bad_args".to_owned(), 1030),
            ("kv_noent".to_owned(), 8016),
            ("locked".to_owned(), 8014),
            ("name_in_use".to_owned(), 1023),
            ("not_found".to_owned(), 1049),
            ("ok".to_owned(), 0),
            ("permission_denied".to_owned(), 1013),
            ("quota_exceeded".to_owned(), 1060),
            ("rate_limited".to_owned(), 1012),
            ("stale_cache".to_owned(), 8012),
            ("stale_root".to_owned(), 1014),
            ("tx_retry".to_owned(), 1014),
            ("unsupported".to_owned(), 1020),
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
    let registered_routes = ROUTES
        .iter()
        .map(|route| Route {
            protocol: route.protocol.into(),
            protocol_id: route.protocol_id,
            method: route.method.into(),
            position: route.position,
            listener: route.listener.into(),
            authentication: route.authentication.into(),
            request: route.request.into(),
            result: route.result.into(),
            statuses: route
                .statuses
                .iter()
                .map(|status| (*status).into())
                .collect(),
            max_request_bytes: route.max_request_bytes,
            supported: route.supported,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(declared_routes, registered_routes);
}

fn validate_unique_contract_entries(contract: &Contract) {
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
        assert!(route_keys.insert((route.protocol_id, route.position)));
        assert!(route_names.insert((&route.protocol, &route.method)));
        assert!(!route.authentication.is_empty());
        assert!(!route.request.is_empty());
        assert!(!route.result.is_empty());
        assert!(!route.statuses.is_empty());
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
