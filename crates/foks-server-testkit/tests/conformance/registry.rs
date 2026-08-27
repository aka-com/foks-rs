use std::collections::{BTreeMap, BTreeSet};

use foks_server::rpc::ROUTES;
use serde::Deserialize;

type Scenario = fn();

struct Coverage {
    name: &'static str,
    run: Scenario,
    routes: &'static [(&'static str, &'static str)],
}

#[derive(Deserialize)]
struct Contract {
    route: Vec<DeclaredRoute>,
}

#[derive(Deserialize)]
struct DeclaredRoute {
    protocol: String,
    method: String,
}

const COVERAGE: &[Coverage] = &[
    Coverage {
        name: "probe_and_pin",
        run: crate::probe_and_pin::probe_and_pin_success,
        routes: &[
            ("Probe", "probe"),
            ("Reg", "reserveUsername"),
            ("Reg", "selectVHost"),
            ("MerkleQuery", "getCurrentRoot"),
            ("MerkleQuery", "selectVHost"),
        ],
    },
    Coverage {
        name: "signup_and_user",
        run: crate::signup_and_user::signup_and_user_success,
        routes: &[
            ("Reg", "signup"),
            ("Reg", "getClientCertChain"),
            ("User", "loadUserChain"),
            ("User", "getPukForRole"),
            ("MerkleQuery", "getHistoricalRoots"),
        ],
    },
    Coverage {
        name: "authorization",
        run: crate::authorization::authorization_and_unsupported_success,
        routes: &[("User", "getHostConfig")],
    },
    Coverage {
        name: "kv_small",
        run: crate::kv_small::kv_small_success,
        routes: &[
            ("KvStore", "mkdir"),
            ("KvStore", "put"),
            ("KvStore", "putRoot"),
            ("KvStore", "putSmallFileOrSymlink"),
            ("KvStore", "getRoot"),
            ("KvStore", "getNode"),
            ("KvStore", "getDir"),
            ("KvStore", "cacheCheck"),
            ("KvStore", "list"),
            ("KvStore", "lockAcquire"),
            ("KvStore", "lockRelease"),
            ("KvStore", "selectVHost"),
        ],
    },
    Coverage {
        name: "kv_large",
        run: crate::kv_large::kv_large_success,
        routes: &[
            ("KvStore", "fileUploadInit"),
            ("KvStore", "fileUploadChunk"),
            ("KvStore", "getEncryptedChunk"),
        ],
    },
];

#[test]
fn every_registered_route_has_an_executable_client_server_scenario() {
    let mut covered = BTreeMap::<(&str, &str), BTreeSet<&str>>::new();
    for scenario in COVERAGE {
        let _callable = scenario.run;
        assert!(!scenario.name.is_empty());
        for route in scenario.routes {
            covered.entry(*route).or_default().insert(scenario.name);
        }
    }
    for route in ROUTES {
        assert!(
            covered.contains_key(&(route.protocol, route.method)),
            "{}::{} has no executable client/server scenario",
            route.protocol,
            route.method
        );
    }
    let contract: Contract = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../foks-server/protocol-v1.toml"
    )))
    .unwrap();
    let declared = contract
        .route
        .iter()
        .map(|route| (route.protocol.as_str(), route.method.as_str()))
        .collect::<BTreeSet<_>>();
    let registered = ROUTES
        .iter()
        .map(|route| (route.protocol, route.method))
        .collect::<BTreeSet<_>>();
    assert_eq!(declared, registered);
    for route in covered.keys() {
        assert!(
            ROUTES
                .iter()
                .any(|registered| (registered.protocol, registered.method) == *route),
            "coverage references unregistered route {}::{}",
            route.0,
            route.1
        );
    }
}
