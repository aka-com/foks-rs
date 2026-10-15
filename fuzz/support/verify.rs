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
    team: EntityId,
    team_host: EntityId,
    team_advance: VerifiedMerkleAdvance,
}

fn binary_entity(bytes: &[u8]) -> EntityId {
    let Value::Binary(bytes) = foks_snowpack::decode(bytes).unwrap() else {
        panic!("fixture is not binary");
    };
    EntityId::from_bytes(bytes).unwrap()
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
        Anchors {
            empty_history: encode(&Value::Array(vec![Value::Null, Value::Null])).unwrap(),
            uid: binary_entity(fixture!("user/uid.snowp")),
            user_host: UserChain::decode(USER_CHAIN).unwrap().links[0]
                .decode_eldest()
                .unwrap()
                .host,
            team: binary_entity(fixture!("user/team-id.snowp")),
            team_host: TeamChain::decode(TEAM_CHAIN).unwrap().links[0]
                .decode_team_group_change()
                .unwrap()
                .host,
            public,
            tail,
            user,
            team_advance,
        }
    })
}

pub type Stage = (&'static str, fn(&Anchors, &[u8]) -> bool);

pub static STAGES: &[Stage] = &[
    ("public host", |_, probe| {
        verify_public_host("foks.app", probe).is_ok()
    }),
    ("user chain", |a, chain| {
        let _ = user_chain_root_epochs(chain);
        verify_user_chain(
            chain,
            &a.uid,
            &a.user_host,
            a.user.authenticated_roots(),
            &a.user,
        )
        .is_ok()
    }),
    ("non-self user chain", |a, chain| {
        verify_non_self_user_chain(
            chain,
            &a.uid,
            &a.user_host,
            a.user.authenticated_roots(),
            &a.user,
        )
        .is_ok()
    }),
    ("team chain", |a, chain| {
        let _ = team_chain_root_epochs(chain);
        verify_team_chain(
            chain,
            &a.team,
            &a.team_host,
            a.team_advance.authenticated_roots(),
            &a.team_advance,
        )
        .is_ok()
    }),
    ("Merkle advance root", |a, root| {
        verify_merkle_advance(a.public.snapshot.merkle_root(), root, USER_HISTORY, &a.tail).is_ok()
    }),
    ("Merkle advance history", |a, history| {
        verify_merkle_advance(a.public.snapshot.merkle_root(), USER_ROOT, history, &a.tail).is_ok()
    }),
    ("signed Merkle advance", |a, signed| {
        verify_signed_merkle_advance(
            a.public.snapshot.merkle_root(),
            signed,
            &a.empty_history,
            a.public.snapshot.chain_bytes(),
            &a.tail,
        )
        .is_ok()
    }),
];

/// Runs the selected stage and reports whether verification accepted the input.
pub fn run(input: &[u8]) -> Option<bool> {
    let (&selector, rest) = input.split_first()?;
    let (_, stage) = STAGES[usize::from(selector) % STAGES.len()];
    Some(stage(anchors(), rest))
}

/// The fixture each stage accepts, used to seed the corpus.
pub const SEEDS: &[(&str, &[u8])] = &[
    ("probe-response", PROBE),
    ("user-chain", USER_CHAIN),
    ("user-chain", USER_CHAIN),
    ("team-chain", TEAM_CHAIN),
    ("merkle-root-998", USER_ROOT),
    ("merkle-historical-response", USER_HISTORY),
    ("signed-merkle-root", SIGNED_ROOT),
];
