use super::*;
use crate::personal::*;
use crate::{Sql, Statement as S, Value as V};
use foks_proto::{EntityId, Role, RoleAndGeneration};
use foks_snowpack::{with_allocation_budget, AllocationBudget};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Budget {
    calls: Mutex<Vec<usize>>,
    refuse: Option<usize>,
}
impl AllocationBudget for Budget {
    fn reserve(&self, bytes: usize) -> bool {
        let mut calls = self.calls.lock().unwrap();
        let index = calls.len();
        calls.push(bytes);
        self.refuse != Some(index)
    }
}
fn budget(refuse: Option<usize>) -> Arc<Budget> {
    Arc::new(Budget {
        calls: Mutex::new(Vec::new()),
        refuse,
    })
}
#[derive(Default)]
struct Mock {
    steps: VecDeque<(S, Option<Vec<V>>)>,
    calls: Vec<(S, Vec<V>)>,
}
impl Mock {
    fn probes(rows: impl IntoIterator<Item = Option<Vec<V>>>) -> Self {
        Self {
            steps: rows
                .into_iter()
                .map(|r| (S::PersonalIssuedCredential, r))
                .collect(),
            ..Self::default()
        }
    }
    fn steps(steps: impl IntoIterator<Item = (S, Option<Vec<V>>)>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
            ..Self::default()
        }
    }
}
impl Sql for Mock {
    fn query(&mut self, statement: S, parameters: &[V]) -> crate::Result<Option<Vec<V>>> {
        let (expected, row) = self.steps.pop_front().expect("unexpected SQL");
        assert_eq!(statement, expected);
        self.calls.push((statement, parameters.to_vec())); // Test observation only.
        Ok(row)
    }
    fn execute(&mut self, _: S, _: &[V]) -> crate::Result<u64> {
        panic!("read-only auth")
    }
}
fn peer(serial: usize, der: usize, expiry: u64) -> PeerCertificate {
    PeerCertificate::from_frontend_parts(vec![0x30; der], vec![7; serial], [4; 32], expiry).unwrap()
}
fn id(kind: u8) -> Vec<u8> {
    let mut v = vec![
        4;
        if kind == foks_proto::ENTITY_YUBI {
            34
        } else {
            33
        }
    ];
    v[0] = kind;
    v
}
fn row() -> Vec<V> {
    vec![
        V::Blob(vec![1; 33]),
        V::Blob(id(foks_proto::ENTITY_DEVICE)),
        V::Blob(id(foks_proto::ENTITY_DEVICE)),
        V::Integer(3),
        V::Integer(0),
    ]
}
fn current(kind: CredentialKind) -> CurrentCredential {
    let (c, o) = match kind {
        CredentialKind::DelegatedSubkey => (foks_proto::ENTITY_SUBKEY, foks_proto::ENTITY_YUBI),
        CredentialKind::BackupKey => (foks_proto::ENTITY_BACKUP_KEY, foks_proto::ENTITY_BACKUP_KEY),
        _ => (foks_proto::ENTITY_DEVICE, foks_proto::ENTITY_DEVICE),
    };
    CurrentCredential {
        uid: [1; 33],
        credential: EntityId::from_bytes(id(c)).unwrap(),
        owner: EntityId::from_bytes(id(o)).unwrap().into_bytes(),
        role: Role::OWNER,
        kind,
        expires_at_micros: 100,
    }
}
fn bounds() -> PersonalError {
    PersonalError::Kernel(crate::Error::Bounds)
}

#[test]
fn exact_serial_der_charge_formula_and_probe_order() {
    for s in [1, 20] {
        for d in [1, 4096] {
            let peer = peer(s, d, 100);
            let n = 21 - s;
            let mut sql = Mock::probes((0..n).map(|_| None));
            let b = budget(None);
            assert_eq!(
                with_allocation_budget(b.clone(), || authenticate_peer(&mut sql, &peer, 1)),
                Err(PersonalError::Unauthenticated)
            );
            let expected: Vec<_> = (s..=20).flat_map(|w| [w, d]).collect();
            assert_eq!(*b.calls.lock().unwrap(), expected);
            assert_eq!(expected.iter().sum::<usize>(), n * d + (20 + s) * n / 2);
            assert!(expected.iter().sum::<usize>() <= 82_130);
            assert_eq!(sql.calls.len(), n);
            for (offset, (_, p)) in sql.calls.iter().enumerate() {
                let mut serial = vec![0; s + offset];
                serial[offset..].fill(7);
                assert_eq!(p, &[V::Blob(serial), V::Blob(vec![0x30; d]), V::Integer(1)]);
            }
            assert!(sql.steps.is_empty());
        }
    }
}

#[test]
fn every_caller_refusal_precedes_its_query_and_never_runs_later_probes() {
    let p = peer(1, 4096, 100);
    for refuse in 0..40 {
        let mut sql = Mock::probes((0..20).map(|_| None));
        let b = budget(Some(refuse));
        assert_eq!(
            with_allocation_budget(b.clone(), || authenticate_peer(&mut sql, &p, 1)),
            Err(bounds())
        );
        assert_eq!(b.calls.lock().unwrap().len(), refuse + 1);
        assert_eq!(sql.calls.len(), refuse / 2);
    }
}

#[test]
fn retained_ids_move_without_copy_and_duplicate_match_still_fails() {
    let p = peer(1, 1, 100);
    let first = row();
    let credential = crate::personal::bytes(&first[1]).unwrap().as_ptr();
    let owner = crate::personal::bytes(&first[2]).unwrap().as_ptr();
    let mut sql = Mock::probes(std::iter::once(Some(first)).chain((0..19).map(|_| None)));
    let b = budget(None);
    let got = with_allocation_budget(b.clone(), || authenticate_peer(&mut sql, &p, 1)).unwrap();
    assert_eq!(got.credential_id().as_ptr(), credential);
    assert_eq!(got.owner_id().as_ptr(), owner);
    assert_eq!(sql.calls.len(), 20); // A match never stops duplicate detection.
    assert_eq!(b.calls.lock().unwrap().len(), 40); // No ID-copy admission.
    let mut duplicate = Mock::probes([Some(row()), Some(vec![])]);
    assert_eq!(
        authenticate_peer(&mut duplicate, &p, 1),
        Err(PersonalError::Corrupt)
    );
    assert_eq!(duplicate.calls.len(), 2);
}

#[test]
fn malformed_rows_and_authentication_order_remain_unchanged() {
    let p = peer(20, 1, 100);
    let mut cases = vec![(vec![], PersonalError::Corrupt)];
    for index in [0, 1, 2, 3, 4] {
        let mut r = row();
        r[index] = V::Null;
        cases.push((r, PersonalError::Corrupt));
    }
    let mut r = row();
    r[0] = V::Blob(vec![4; 33]);
    cases.push((r, PersonalError::Corrupt));
    let mut r = row();
    r[1] = V::Blob(vec![4; 32]);
    cases.push((r, PersonalError::Corrupt));
    let mut r = row();
    r[2] = V::Blob(vec![4; 32]);
    cases.push((r, PersonalError::Corrupt));
    let mut r = row();
    let mut key = id(foks_proto::ENTITY_DEVICE);
    key[1] = 9;
    r[1] = V::Blob(key);
    cases.push((r, PersonalError::Unauthenticated));
    let mut r = row();
    let mut key = id(foks_proto::ENTITY_DEVICE);
    key[1] = 9;
    r[1] = V::Blob(key);
    r[2] = V::Null; // Owner corruption precedes the mismatching public key.
    cases.push((r, PersonalError::Corrupt));
    let mut r = row();
    r[3] = V::Integer(99);
    cases.push((r, PersonalError::Corrupt));
    let mut r = row();
    r[1] = V::Blob(id(foks_proto::ENTITY_SUBKEY));
    cases.push((r, PersonalError::Corrupt));
    let mut r = row();
    r[2] = V::Blob(id(foks_proto::ENTITY_YUBI));
    cases.push((r, PersonalError::Corrupt));
    for (r, error) in cases {
        let mut sql = Mock::probes([Some(r)]);
        let b = budget(None);
        assert_eq!(
            with_allocation_budget(b.clone(), || authenticate_peer(&mut sql, &p, 1)),
            Err(error)
        );
        assert_eq!(*b.calls.lock().unwrap(), [20, 1]);
        assert_eq!(sql.calls.len(), 1);
    }
    for (now, error) in [
        (100, PersonalError::Unauthenticated),
        (u64::MAX, PersonalError::Kernel(crate::Error::IntegerRange)),
    ] {
        let mut sql = Mock::default();
        let b = budget(None);
        assert_eq!(
            with_allocation_budget(b.clone(), || authenticate_peer(&mut sql, &p, now)),
            Err(error)
        );
        assert!(b.calls.lock().unwrap().is_empty());
        assert!(sql.calls.is_empty());
    }
    for now in [0, i64::MAX as u64 - 1] {
        let p = peer(20, 1, i64::MAX as u64);
        let mut sql = Mock::probes([Some(row())]);
        assert!(authenticate_peer(&mut sql, &p, now).is_ok());
        assert_eq!(sql.calls[0].1[2], V::Integer(now as i64));
    }
}

#[test]
fn recheck_profile_and_parent_permission_admit_exact_selected_bytes() {
    let c = current(CredentialKind::DelegatedSubkey);
    let steps = [
        (
            S::PersonalCurrentCredential,
            Some(vec![V::Integer(3), V::Integer(0)]),
        ),
        (S::PersonalSsoPolicy, None),
        (S::PersonalAnchor, Some(vec![V::Integer(1)])),
        (S::PersonalRoleGeneration, Some(vec![V::Integer(1)])),
    ];
    let mut sql = Mock::steps(steps);
    let b = budget(None);
    with_allocation_budget(b.clone(), || {
        recheck_current_credential(&mut sql, &c, 1)?;
        require_personal_profile(&mut sql, &c)?;
        require_key(
            &mut sql,
            &c,
            KeyRequirement {
                key: RoleAndGeneration {
                    role: Role::OWNER,
                    generation: 1,
                },
                access: KeyAccess::Read,
            },
        )
    })
    .unwrap();
    assert_eq!(*b.calls.lock().unwrap(), [34, 33, 33, 33, 33]);
    assert_eq!(
        82_130 + b.calls.lock().unwrap().iter().sum::<usize>(),
        82_296
    );
    assert!(sql.steps.is_empty());
    for refuse in 0..3 {
        let mut sql = Mock::default();
        let b = budget(Some(refuse));
        assert_eq!(
            with_allocation_budget(b.clone(), || recheck_current_credential(&mut sql, &c, 1)),
            Err(bounds())
        );
        assert!(sql.calls.is_empty());
        assert_eq!(b.calls.lock().unwrap().len(), refuse + 1);
    }
}

#[test]
fn profile_key_and_expiry_failures_keep_the_original_short_circuits() {
    let c = current(CredentialKind::SoftwareDevice);
    let b = budget(Some(0));
    let mut sql = Mock::default();
    assert_eq!(
        with_allocation_budget(b.clone(), || recheck_current_credential(&mut sql, &c, 100)),
        Err(PersonalError::Unauthenticated)
    );
    assert_eq!(
        with_allocation_budget(b.clone(), || require_personal_profile(
            &mut sql,
            &current(CredentialKind::BackupKey)
        )),
        Err(PersonalError::Permission)
    );
    let key = KeyRequirement {
        key: RoleAndGeneration {
            role: Role::NONE,
            generation: 1,
        },
        access: KeyAccess::Read,
    };
    assert_eq!(
        with_allocation_budget(b.clone(), || require_key(&mut sql, &c, key)),
        Err(PersonalError::Permission)
    );
    assert!(b.calls.lock().unwrap().is_empty());
    assert!(sql.calls.is_empty());
    let mut sql = Mock::steps([(S::PersonalSsoPolicy, Some(vec![V::Integer(1)]))]);
    assert_eq!(
        with_allocation_budget(b.clone(), || require_personal_profile(&mut sql, &c)),
        Err(PersonalError::UnsupportedProfile)
    );
    assert!(b.calls.lock().unwrap().is_empty());
    let mut sql = Mock::steps([(S::PersonalSsoPolicy, None)]);
    assert_eq!(
        with_allocation_budget(b.clone(), || require_personal_profile(&mut sql, &c)),
        Err(bounds())
    );
    assert_eq!(sql.calls.len(), 1);
    assert_eq!(*b.calls.lock().unwrap(), [33]);
    let mut sql = Mock::steps([(S::PersonalSsoPolicy, None), (S::PersonalAnchor, None)]);
    assert_eq!(
        require_personal_profile(&mut sql, &c),
        Err(PersonalError::Permission)
    );
    for row in [None, Some(vec![V::Integer(0)])] {
        let mut sql = Mock::steps([(S::PersonalRoleGeneration, row)]);
        assert_eq!(
            require_key(
                &mut sql,
                &c,
                KeyRequirement {
                    key: RoleAndGeneration {
                        role: Role::OWNER,
                        generation: 1
                    },
                    access: KeyAccess::Read
                }
            ),
            Err(PersonalError::Permission)
        );
    }
    let mut sql = Mock::default();
    let b = budget(Some(0));
    assert_eq!(
        with_allocation_budget(b.clone(), || current_key_generation(
            &mut sql,
            c.uid(),
            Role::OWNER
        )),
        Err(bounds())
    );
    assert!(sql.calls.is_empty());
    assert_eq!(*b.calls.lock().unwrap(), [33]);
    for (row, error) in [
        (None, PersonalError::Permission),
        (
            Some(vec![V::Integer(2), V::Integer(0)]),
            PersonalError::StalePreparation,
        ),
        (Some(vec![V::Null, V::Integer(0)]), PersonalError::Corrupt),
    ] {
        let mut sql = Mock::steps([(S::PersonalCurrentCredential, row)]);
        assert_eq!(recheck_current_credential(&mut sql, &c, 1), Err(error));
    }
    let mut sql = Mock::default();
    let b = budget(Some(0));
    assert_eq!(
        with_allocation_budget(b.clone(), || current_key_generation(
            &mut sql,
            &[0; 33],
            Role::OWNER
        )),
        Err(PersonalError::BadArguments)
    );
    assert!(b.calls.lock().unwrap().is_empty());
}

#[test]
fn physical_allocation_failure_is_fallible_after_admission_not_a_new_domain_error() {
    let b = budget(None);
    assert_eq!(
        with_allocation_budget(b.clone(), || allocate_with(19, |_, _| Err(()))),
        Err(bounds())
    );
    assert_eq!(*b.calls.lock().unwrap(), [19]);
    let b = budget(Some(0));
    assert_eq!(
        with_allocation_budget(b, || allocate_with(19, |_, _| panic!("admission first"))),
        Err(bounds())
    );
    assert!(copy(&[]).unwrap().is_empty());
    assert_eq!(zeroed(3).unwrap(), [0, 0, 0]);
}
