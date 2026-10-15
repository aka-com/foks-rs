//! foks-verify stages anchored at the official foks.app v0.1.9 fixtures. The
//! first input byte selects a stage; the remaining bytes replace one network
//! input while every other argument stays authenticated, so mutations reach
//! the chain, Merkle and signature checks instead of failing at the anchor.
#![allow(dead_code)]
use foks_proto::{EntityId, HostchainTail, TeamChain, UserChain};
use foks_snowpack::{encode, Value};
use foks_verify::*;
use std::sync::OnceLock;

macro_rules! fixture {
    ($path:literal) => {
        include_bytes!(concat!(
            "../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/",
            $path
        ))
    };
}

pub const PROBE: &[u8] = fixture!("foks.app/probe-response.snowp");
pub const SIGNED_ROOT: &[u8] = fixture!("foks.app/signed-merkle-root.snowp");
pub const USER_CHAIN: &[u8] = fixture!("user/user-chain.snowp");
pub const USER_ROOT: &[u8] = fixture!("user/merkle-root-998.snowp");
pub const USER_HISTORY: &[u8] = fixture!("user/merkle-historical-response.snowp");
pub const TEAM_CHAIN: &[u8] = fixture!("user/team-chain.snowp");
pub const TEAM_ROOT: &[u8] = fixture!("user/team-merkle-root-996.snowp");
pub const TEAM_HISTORY: &[u8] = fixture!("user/team-merkle-historical-response.snowp");

pub struct Anchors {
    public: VerifiedPublicHost,
    tail: HostchainTail,
    empty_history: Vec<u8>,
    uid: EntityId,
    user_host: EntityId,
    user: VerifiedMerkleAdvance,
    user_state: VerifiedUserState,
    non_self_user_state: VerifiedUserState,
    user_noop: Vec<u8>,
    team: EntityId,
    team_host: EntityId,
    team_advance: VerifiedMerkleAdvance,
    team_state: VerifiedTeamState,
    team_noop: Vec<u8>,
}

fn binary_entity(bytes: &[u8]) -> EntityId {
    let Value::Binary(bytes) = foks_snowpack::decode(bytes).unwrap() else {
        panic!("fixture is not binary");
    };
    EntityId::from_bytes(bytes).unwrap()
}

/// The refresh response for a chain that has not advanced since the full
/// response: no links, the tail location, and the Merkle paths of the last
/// name link and the tail. foks-verify's incremental refresh tests build the
/// same response.
fn noop_response(full_response: &[u8], name_path_count: u64, team: bool) -> Vec<u8> {
    let Value::Array(mut fields) = foks_snowpack::decode(full_response).unwrap() else {
        panic!("chain fixture is not an array");
    };
    let Value::Array(mut merkle) = fields[3].clone() else {
        panic!("chain Merkle evidence is not an array");
    };
    let Value::Array(paths) = merkle[1].clone() else {
        panic!("chain Merkle paths are not an array");
    };
    let name_path = usize::try_from(name_path_count).unwrap() - 1;
    merkle[1] = Value::Array(vec![
        paths[name_path].clone(),
        paths.last().unwrap().clone(),
    ]);
    let Value::Array(locations) = fields[1].clone() else {
        panic!("chain locations are not an array");
    };
    fields[0] = Value::Null;
    fields[1] = Value::Array(vec![locations.last().unwrap().clone()]);
    fields[2] = Value::Null;
    fields[3] = Value::Array(merkle);
    if team {
        fields[5] = Value::Unsigned(1);
        fields[9] = Value::Array(vec![Value::Null]);
    } else {
        fields[4] = Value::Null;
        fields[6] = Value::Unsigned(1);
        fields[7] = Value::Array(vec![Value::Null]);
    }
    encode(&Value::Array(fields)).unwrap()
}

pub fn anchors() -> &'static Anchors {
    static ANCHORS: OnceLock<Anchors> = OnceLock::new();
    ANCHORS.get_or_init(|| {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let tail = HostchainTail {
            seqno: public.snapshot.chain_seqno(),
            hash: public.snapshot.chain_tail_hash(),
        };
        let user = verify_merkle_advance(
            public.snapshot.merkle_root(),
            USER_ROOT,
            USER_HISTORY,
            &tail,
        )
        .unwrap();
        let team_advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            TEAM_ROOT,
            TEAM_HISTORY,
            &tail,
        )
        .unwrap();
        let user_chain = UserChain::decode(USER_CHAIN).unwrap();
        let team_chain = TeamChain::decode(TEAM_CHAIN).unwrap();
        let uid = binary_entity(fixture!("user/uid.snowp"));
        let user_host = user_chain.links[0].decode_eldest().unwrap().host;
        let team = binary_entity(fixture!("user/team-id.snowp"));
        let team_host = team_chain.links[0].decode_team_group_change().unwrap().host;
        let roots = user.authenticated_roots();
        let user_state = verify_user_chain(USER_CHAIN, &uid, &user_host, roots, &user).unwrap();
        let non_self_user_state =
            verify_non_self_user_chain(USER_CHAIN, &uid, &user_host, roots, &user).unwrap();
        let team_state = verify_team_chain(
            TEAM_CHAIN,
            &team,
            &team_host,
            team_advance.authenticated_roots(),
            &team_advance,
        )
        .unwrap();
        Anchors {
            empty_history: encode(&Value::Array(vec![Value::Null, Value::Null])).unwrap(),
            user_noop: noop_response(USER_CHAIN, user_chain.num_username_links, false),
            team_noop: noop_response(TEAM_CHAIN, team_chain.num_team_name_links, true),
            uid,
            user_host,
            user_state,
            non_self_user_state,
            team,
            team_host,
            team_state,
            public,
            tail,
            user,
            team_advance,
        }
    })
}

pub struct Stage {
    pub name: &'static str,
    /// The seed name and an input the stage accepts.
    pub seed: (&'static str, fn(&Anchors) -> Vec<u8>),
    pub run: fn(&Anchors, &[u8]) -> bool,
}

pub static STAGES: &[Stage] = &[
    Stage {
        name: "public host",
        seed: ("probe-response", |_| PROBE.to_vec()),
        run: |_, probe| verify_public_host("foks.app", probe).is_ok(),
    },
    Stage {
        name: "user chain",
        seed: ("user-chain", |_| USER_CHAIN.to_vec()),
        run: |a, chain| {
            let _ = user_chain_root_epochs(chain);
            // A single chain response is also valid persisted evidence.
            let _ = user_evidence_root_epochs(chain);
            verify_user_chain(
                chain,
                &a.uid,
                &a.user_host,
                a.user.authenticated_roots(),
                &a.user,
            )
            .is_ok()
        },
    },
    Stage {
        name: "non-self user chain",
        seed: ("user-chain", |_| USER_CHAIN.to_vec()),
        run: |a, chain| {
            verify_non_self_user_chain(
                chain,
                &a.uid,
                &a.user_host,
                a.user.authenticated_roots(),
                &a.user,
            )
            .is_ok()
        },
    },
    Stage {
        name: "user chain increment",
        seed: ("user-chain-noop", |a| a.user_noop.clone()),
        run: |a, chain| {
            verify_user_chain_increment(
                chain,
                &a.user_state,
                &a.uid,
                &a.user_host,
                a.user.authenticated_roots(),
                &a.user,
            )
            .is_ok()
        },
    },
    Stage {
        name: "non-self user chain increment",
        seed: ("user-chain-noop", |a| a.user_noop.clone()),
        run: |a, chain| {
            verify_non_self_user_chain_increment(
                chain,
                &a.non_self_user_state,
                &a.uid,
                &a.user_host,
                a.user.authenticated_roots(),
                &a.user,
            )
            .is_ok()
        },
    },
    Stage {
        name: "team chain",
        seed: ("team-chain", |_| TEAM_CHAIN.to_vec()),
        run: |a, chain| {
            let _ = team_chain_root_epochs(chain);
            let _ = team_evidence_root_epochs(chain);
            verify_team_chain(
                chain,
                &a.team,
                &a.team_host,
                a.team_advance.authenticated_roots(),
                &a.team_advance,
            )
            .is_ok()
        },
    },
    Stage {
        name: "team chain increment",
        seed: ("team-chain-noop", |a| a.team_noop.clone()),
        run: |a, chain| {
            verify_team_chain_increment(
                chain,
                &a.team_state,
                &a.team,
                &a.team_host,
                a.team_advance.authenticated_roots(),
                &a.team_advance,
            )
            .is_ok()
        },
    },
    Stage {
        name: "Merkle advance root",
        seed: ("merkle-root-998", |_| USER_ROOT.to_vec()),
        run: |a, root| {
            verify_merkle_advance(a.public.snapshot.merkle_root(), root, USER_HISTORY, &a.tail)
                .is_ok()
        },
    },
    Stage {
        name: "Merkle advance history",
        seed: ("merkle-historical-response", |_| USER_HISTORY.to_vec()),
        run: |a, history| {
            verify_merkle_advance(a.public.snapshot.merkle_root(), USER_ROOT, history, &a.tail)
                .is_ok()
        },
    },
    Stage {
        name: "signed Merkle advance",
        seed: ("signed-merkle-root", |_| SIGNED_ROOT.to_vec()),
        run: |a, signed| {
            verify_signed_merkle_advance(
                a.public.snapshot.merkle_root(),
                signed,
                &a.empty_history,
                a.public.snapshot.chain_bytes(),
                &a.tail,
            )
            .is_ok()
        },
    },
];

/// Runs the selected stage and reports whether verification accepted the input.
pub fn run(input: &[u8]) -> Option<bool> {
    let (&selector, rest) = input.split_first()?;
    let stage = &STAGES[usize::from(selector) % STAGES.len()];
    Some((stage.run)(anchors(), rest))
}
