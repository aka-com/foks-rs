use super::*;
use crate::authority::{Caller, Number, Resolver};
use crate::personal::{authenticate_peer, PersonalError};
use crate::{Sql, Statement as S, Value as V};
use foks_snowpack::{with_allocation_budget, AllocationBudget};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

// Deliberately independent old serializer: never reuse the new micros module.
#[derive(Serialize)]
struct LegacyPeer<'a> {
    exact_der: &'a [u8],
    serial_magnitude: &'a [u8],
    public_key: &'a [u8; 32],
    #[serde(serialize_with = "legacy_expiry")]
    expires_at_micros: u64,
}
fn legacy_expiry<S: serde::Serializer>(n: &u64, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&n.to_string())
}
fn legacy_peer(peer: &PeerCertificate) -> LegacyPeer<'_> {
    LegacyPeer {
        exact_der: peer.exact_der(),
        serial_magnitude: peer.serial_magnitude(),
        public_key: peer.public_key(),
        expires_at_micros: peer.expires_at_micros(),
    }
}
fn make_peer(der: usize, serial: usize, expiry: u64) -> PeerCertificate {
    PeerCertificate::from_frontend_parts(vec![255; der], vec![255; serial], [255; 32], expiry)
        .unwrap()
}
#[test]
fn peer_hash_preserves_old_bytes_and_exact_maximum_without_new_temporaries() {
    for der in [1, 4096] {
        for serial in [1, 20] {
            for expiry in [0, 9, 10, i64::MAX as u64] {
                let peer = make_peer(der, serial, expiry);
                let old =
                    serde_json::to_vec(&("foks-personal-peer-v1", legacy_peer(&peer))).unwrap();
                assert_eq!(
                    old,
                    serde_json::to_vec(&("foks-personal-peer-v1", &peer)).unwrap()
                );
                assert!(old.len() <= PEER_BYTES);
                assert_eq!(
                    peer_context_digest(&peer).unwrap(),
                    <[u8; 32]>::from(Sha256::digest(&old))
                );
                assert_eq!(
                    digest(&("foks-personal-peer-v1", &peer), old.len()).unwrap(),
                    peer_context_digest(&peer).unwrap()
                );
                assert_eq!(
                    digest(&("foks-personal-peer-v1", &peer), old.len() - 1),
                    Err(crate::Error::Bounds.into())
                );
                if der == 4096 && serial == 20 && expiry == i64::MAX as u64 {
                    assert_eq!(old.len(), 16_711);
                    assert_eq!(
                        <[u8; 32]>::from(Sha256::digest(&old)),
                        [
                            68, 58, 61, 23, 250, 179, 69, 82, 66, 63, 26, 142, 43, 167, 151, 29,
                            132, 219, 158, 197, 102, 217, 93, 54, 143, 230, 229, 163, 225, 85, 245,
                            69
                        ]
                    );
                }
            }
        }
    }
}
#[test]
fn writer_overflow_never_hashes_more_bytes_or_exposes_a_prefix() {
    for overflow in [false, true] {
        let mut writer = HashWriter::new(if overflow { usize::MAX } else { 2 });
        writer.write_all(b"a").unwrap();
        if overflow {
            writer.bytes = usize::MAX;
        }
        let before = writer.hash.clone().finalize();
        writer.write_all(b"bc").unwrap();
        assert!(writer.failed);
        writer.write_all(b"later").unwrap();
        assert_eq!(writer.hash.clone().finalize(), before);
        assert_eq!(writer.finish(), Err(crate::Error::Bounds.into()));
    }
    assert_eq!(
        digest(&[1u8, 2], 5).unwrap(),
        <[u8; 32]>::from(Sha256::digest(b"[1,2]"))
    );
}
#[test]
fn every_role_encoding_matches_the_unmodified_protocol_encoder() {
    for role in [Role::NONE, Role::ADMIN, Role::OWNER]
        .into_iter()
        .chain((i16::MIN..=i16::MAX).map(Role::member))
    {
        let (bytes, length) = role_bytes(role);
        assert!((3..=8).contains(&length));
        assert_eq!(&bytes[..length], role.encoded().unwrap());
    }
}
#[derive(Default)]
struct Budget {
    calls: Mutex<Vec<usize>>,
    refuse: Option<usize>,
}
impl AllocationBudget for Budget {
    fn reserve(&self, n: usize) -> bool {
        let mut calls = self.calls.lock().unwrap();
        let at = calls.len();
        calls.push(n);
        self.refuse != Some(at)
    }
}
fn budget(refuse: Option<usize>) -> Arc<Budget> {
    Arc::new(Budget {
        calls: Mutex::new(vec![]),
        refuse,
    })
}
#[derive(Default)]
struct Mock {
    rows: VecDeque<(S, Option<Vec<V>>)>,
    calls: Vec<S>,
}
impl Sql for Mock {
    fn query(&mut self, s: S, _: &[V]) -> crate::Result<Option<Vec<V>>> {
        self.calls.push(s);
        let (expected, row) = self.rows.pop_front().expect("unexpected query");
        assert_eq!(s, expected);
        Ok(row)
    }
    fn execute(&mut self, _: S, _: &[V]) -> crate::Result<u64> {
        panic!("resolver never writes")
    }
}
fn credential_row(role: Role, delegated: bool) -> Vec<V> {
    let mut uid = vec![255; 33];
    uid[0] = foks_proto::ENTITY_USER;
    let mut credential = vec![255; 33];
    credential[0] = if delegated {
        foks_proto::ENTITY_SUBKEY
    } else {
        foks_proto::ENTITY_DEVICE
    };
    let mut owner = vec![255; if delegated { 34 } else { 33 }];
    owner[0] = if delegated {
        foks_proto::ENTITY_YUBI
    } else {
        foks_proto::ENTITY_DEVICE
    };
    vec![
        V::Blob(uid),
        V::Blob(credential),
        V::Blob(owner),
        V::Integer(role.protocol_value() as i64),
        V::Integer(role.visibility().map_or(0, i64::from)),
    ]
}
fn mock(role: Role, delegated: bool) -> Mock {
    Mock {
        rows: [
            (
                S::PersonalIssuedCredential,
                Some(credential_row(role, delegated)),
            ),
            (S::PersonalSsoPolicy, None),
            (S::PersonalAnchor, Some(vec![V::Integer(1)])),
        ]
        .into(),
        calls: vec![],
    }
}
#[test]
fn principal_hash_is_stable_across_renewal_but_context_binds_exact_peer() {
    for delegated in [false, true] {
        let old_peer = make_peer(1, 20, 10000);
        let renewed = make_peer(2, 20, 20000);
        let mut sql = mock(Role::OWNER, delegated);
        let current = authenticate_peer(&mut sql, &old_peer, 1).unwrap();
        let legacy = serde_json::to_vec(&(
            "foks-personal-principal-v1",
            current.uid().as_slice(),
            current.credential_id(),
            current.owner_id(),
        ))
        .unwrap();
        assert!(legacy.len() <= PRINCIPAL_BYTES);
        assert_eq!(
            principal_digest(&current).unwrap(),
            <[u8; 32]>::from(Sha256::digest(&legacy))
        );
        let first = principal_digest(&current).unwrap();
        let mut sql = mock(Role::OWNER, delegated);
        let current = authenticate_peer(&mut sql, &renewed, 1).unwrap();
        assert_eq!(principal_digest(&current).unwrap(), first);
        assert_ne!(
            peer_context_digest(&old_peer).unwrap(),
            peer_context_digest(&renewed).unwrap()
        );
        if delegated {
            assert_eq!(
                <[u8; 32]>::from(Sha256::digest(&legacy)),
                [
                    127, 49, 248, 122, 107, 190, 1, 144, 133, 38, 80, 222, 255, 93, 219, 143, 41,
                    180, 79, 106, 84, 252, 39, 23, 220, 98, 39, 39, 224, 218, 1, 64
                ]
            );
        }
    }
    let max = serde_json::to_vec(&(
        "foks-personal-principal-v1",
        [255u8; 33].as_slice(),
        [255u8; 33].as_slice(),
        [255u8; 34].as_slice(),
    ))
    .unwrap();
    assert_eq!(max.len(), PRINCIPAL_BYTES);
    assert_eq!(PRINCIPAL_BYTES, 436);
}
#[test]
fn resolver_copy_order_and_failure_leave_no_partial_current_or_late_query() {
    let peer = make_peer(1, 20, 10000);
    for role_value in [Role::OWNER, Role::member(i16::MIN), Role::member(0)] {
        let role_length = role_bytes(role_value).1;
        // S3a serial+DER+anchor precede the new namespace and role copies.
        for refuse in [None, Some(3), Some(4)] {
            let b = budget(refuse);
            let mut sql = mock(role_value, false);
            let mut resolver = super::super::PersonalResolver::new(&peer, Number(1)).unwrap();
            let result = with_allocation_budget(b.clone(), || {
                resolver.resolve(&mut sql, &Caller::Frontend { principal: [0; 32] })
            });
            let expected = [20, 1, 33, 33, role_length];
            assert_eq!(
                *b.calls.lock().unwrap(),
                expected[..refuse.map_or(5, |n| n + 1)]
            );
            assert_eq!(
                sql.calls,
                [
                    S::PersonalIssuedCredential,
                    S::PersonalSsoPolicy,
                    S::PersonalAnchor
                ]
            );
            match refuse {
                None => {
                    let p = result.unwrap();
                    assert_eq!(p.namespace.len(), 33);
                    assert_eq!(p.role, role_value.encoded().unwrap());
                    assert!(resolver.current.is_some());
                }
                Some(_) => {
                    assert!(matches!(
                        result,
                        Err(PersonalError::Kernel(crate::Error::Bounds))
                    ));
                    assert!(resolver.current.is_none());
                }
            }
        }
    }
}
#[test]
fn resolver_short_circuits_and_hashing_add_no_heap_admission() {
    let peer = make_peer(1, 20, 1000);
    let b = budget(Some(0));
    let mut resolver = with_allocation_budget(b.clone(), || {
        super::super::PersonalResolver::new(&peer, Number(1))
    })
    .unwrap();
    assert!(b.calls.lock().unwrap().is_empty());
    let mut sql = Mock::default();
    assert!(matches!(
        resolver.resolve(&mut sql, &Caller::Controller {}),
        Err(PersonalError::Permission)
    ));
    assert!(matches!(
        resolver.resolve(&mut sql, &Caller::Frontend { principal: [0; 32] }),
        Err(PersonalError::Unauthenticated)
    ));
    assert!(sql.calls.is_empty());
    let peer = make_peer(1, 20, 10000);
    let mut resolver = super::super::PersonalResolver::new(&peer, Number(1)).unwrap();
    let mut sql = mock(Role::OWNER, false);
    sql.rows[1].1 = Some(vec![V::Integer(1)]);
    let b = budget(None);
    let result = with_allocation_budget(b.clone(), || {
        resolver.resolve(&mut sql, &Caller::Frontend { principal: [0; 32] })
    });
    assert!(matches!(result, Err(PersonalError::UnsupportedProfile)));
    assert_eq!(*b.calls.lock().unwrap(), [20, 1]);
    assert_eq!(
        sql.calls,
        [S::PersonalIssuedCredential, S::PersonalSsoPolicy]
    );
    assert!(resolver.current.is_none());
}
