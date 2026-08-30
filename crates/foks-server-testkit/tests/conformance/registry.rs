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
        name: "go_client_activation",
        run: crate::go_client_activation::go_client_activation_success,
        routes: &[("Reg", "getServerConfig"), ("User", "ping")],
    },
    Coverage {
        name: "go_client_housekeeping",
        run: crate::go_client_activation::go_client_activation_success,
        routes: &[
            ("Reg", "getClientVersionInfo"),
            ("User", "getDeviceNag"),
            ("User", "clearDeviceNag"),
        ],
    },
    Coverage {
        name: "go_client_identity",
        run: crate::go_client_activation::go_client_activation_success,
        routes: &[
            ("Reg", "checkNameExists"),
            ("Reg", "probeKeyExists"),
            ("Reg", "resolveUsername"),
            ("User", "resolveUsername"),
        ],
    },
    Coverage {
        name: "go_client_merkle_queries",
        run: crate::go_client_merkle::go_client_merkle_queries,
        routes: &[
            ("MerkleQuery", "lookup"),
            ("MerkleQuery", "getCurrentRootHash"),
            ("MerkleQuery", "checkKeyExists"),
            ("MerkleQuery", "mLookup"),
        ],
    },
    Coverage {
        name: "go_client_generic",
        run: crate::go_client_generic::generic_membership_chains_and_trusted_team_lists_work,
        routes: &[
            ("User", "postGenericLink"),
            ("User", "loadGenericChain"),
            ("User", "getTeamListServerTrust"),
            ("TeamLoader", "loadTeamMembershipChain"),
            ("TeamAdmin", "postTeamMembershipLink"),
        ],
    },
    Coverage {
        name: "probe_and_pin",
        run: crate::probe_and_pin::probe_and_pin_success,
        routes: &[
            ("Probe", "probe"),
            ("Reg", "reserveUsername"),
            ("Reg", "selectVHost"),
            ("MerkleQuery", "getCurrentRoot"),
            ("MerkleQuery", "getCurrentRootSigned"),
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
        name: "signup_invites",
        run: crate::signup_invites::signup_invites_success,
        routes: &[("Reg", "checkInviteCode")],
    },
    Coverage {
        name: "passphrases",
        run: crate::passphrases::signup_set_change_and_public_login_cover_the_passphrase_lifecycle,
        routes: &[
            ("Reg", "signup"),
            ("Reg", "getLoginChallenge"),
            ("Reg", "login"),
            ("Reg", "stretchVersion"),
            ("User", "setPassphrase"),
            ("User", "changePassphrase"),
            ("User", "getSalt"),
            ("User", "nextPassphraseGeneration"),
            ("User", "stretchVersion"),
            ("User", "revokeDevice"),
            ("User", "getPpeParcel"),
        ],
    },
    Coverage {
        name: "authorization",
        run: crate::authorization::authorization_and_unsupported_success,
        routes: &[("User", "getHostConfig")],
    },
    Coverage {
        name: "provisioning",
        run: crate::provisioning::provisioning_success,
        routes: &[("User", "provisionDevice"), ("User", "revokeDevice")],
    },
    Coverage {
        name: "recovery",
        run: crate::recovery::recovery_success,
        routes: &[
            ("Reg", "getUIDLookupChallege"),
            ("Reg", "lookupUIDByDevice"),
        ],
    },
    Coverage {
        name: "yubikey_lifecycle",
        run: crate::yubikey::software_owner_provisions_recovers_manages_and_revokes_yubikey,
        routes: &[
            ("Reg", "getSubkeyBoxChallenge"),
            ("Reg", "loadSubkeyBox"),
            ("User", "putYubiManagementKey"),
            ("User", "getYubiManagementKey"),
            ("User", "getAllYubiManagementKeys"),
        ],
    },
    Coverage {
        name: "federation_lifecycle",
        run: crate::federation::federation_lifecycle,
        routes: &[
            ("Reg", "loadUserChain"),
            ("User", "grantRemoteViewPermissionForUser"),
            ("TeamLoader", "loadTeamChain"),
            ("TeamLoader", "loadTeamRemoteViewTokens"),
            ("TeamMember", "grantRemoteViewPermissionForTeam"),
        ],
    },
    Coverage {
        name: "unsupported_federation_routes",
        run: crate::federation::unsupported_federation_routes,
        routes: &[("Beacon", "beaconLookup")],
    },
    Coverage {
        name: "team_create_edit_and_kv",
        run: crate::team_create::public_client_creates_and_loads_named_and_adhoc_teams,
        routes: &[
            ("TeamLoader", "getTeamVOBearerTokenChallenge"),
            ("TeamLoader", "activateTeamVOBearerToken"),
            ("TeamLoader", "loadTeamChain"),
            ("TeamAdmin", "reserveTeamname"),
            ("TeamAdmin", "createTeam"),
            ("TeamAdmin", "editTeam"),
            ("TeamAdmin", "makeInertTeamBearerToken"),
            ("TeamAdmin", "activateTeamBearerToken"),
            ("TeamAdmin", "loadRemovalKeyBoxForTeamAdmin"),
            ("TeamAdmin", "createTeamAdHoc"),
        ],
    },
    Coverage {
        name: "unsupported_team_routes",
        run: crate::authorization::unsupported_team_routes_return_typed_status,
        routes: &[
            ("TeamLoader", "checkTeamVOBearerToken"),
            ("TeamLoader", "loadRemovalForMember"),
            ("TeamLoader", "getServerConfig"),
            ("TeamAdmin", "checkTeamBearerToken"),
            ("TeamAdmin", "putTeamCert"),
            ("TeamAdmin", "getCurrentTeamCerts"),
            ("TeamAdmin", "loadTeamRemoteJoinReq"),
            ("TeamAdmin", "postTeamRemoval"),
            ("TeamAdmin", "loadTeamRawInbox"),
            ("TeamAdmin", "rejectJoinReq"),
            ("TeamAdmin", "getTeamConfig"),
        ],
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
    Coverage {
        name: "go_client_kv",
        run: crate::go_client_kv::go_style_kv_get_usage_directory_nodes_and_time_lists_work,
        routes: &[("KvStore", "get"), ("KvStore", "usage")],
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
        assert_eq!(
            route.coverage.iter().copied().collect::<BTreeSet<_>>(),
            covered[&(route.protocol, route.method)],
            "{}::{} has stale generated coverage metadata",
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
