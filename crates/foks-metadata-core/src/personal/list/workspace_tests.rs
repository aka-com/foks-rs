use super::*;
use crate::{
    page::{PageBuffer, PageEncoder},
    Statement,
};
use foks_proto::{EntityId, KvDirectoryStatus, KvNodeId, RoleAndGeneration, SecretBox};
use std::collections::VecDeque;

fn credential() -> CurrentCredential {
    let mut id = vec![1; 33];
    id[0] = foks_proto::ENTITY_DEVICE;
    CurrentCredential {
        uid: [1; 33],
        credential: EntityId::from_bytes(id.clone()).unwrap(),
        owner: EntityId::from_bytes(id).unwrap(),
        role: Role::OWNER,
        kind: super::super::CredentialKind::SoftwareDevice,
        expires_at_micros: 100,
    }
}
fn row(page: &mut PageEncoder, n: u8, corrupt: bool, body: bool) {
    let entry = KvDirent::new(
        [0; 16],
        [n; 16],
        KvNodeId([3; 17]),
        1,
        1,
        Role::OWNER,
        [n; 32],
        SecretBox {
            nonce: [3; 16],
            ciphertext: vec![4; 32],
        },
        KvDirectoryStatus::Active,
        [5; 32],
        1,
    )
    .unwrap();
    let small = foks_snowpack::encode(
        &KvSmallFileBox {
            key: RoleAndGeneration {
                role: Role::OWNER,
                generation: 1,
            },
            ciphertext: vec![6; 32],
        }
        .to_value(),
    )
    .unwrap();
    let entry = if corrupt { &[0u8][..] } else { entry.encoded() };
    page.push(&[
        Cell::Blob(&[n; 16]),
        Cell::Blob(&[n; 32]),
        Cell::Integer(1),
        Cell::Blob(&[3; 17]),
        Cell::Text("blob"),
        Cell::Integer(entry.len() as i64),
        Cell::Blob(entry),
        Cell::Integer(i64::from(body)),
        Cell::Text(if body { "blob" } else { "null" }),
        Cell::Integer(if body { small.len() as i64 } else { -1 }),
        if body { Cell::Blob(&small) } else { Cell::Null },
    ])
    .unwrap();
}
fn list_page(count: usize, corrupt_last: bool, body: bool) -> PageBuffer {
    let mut page = PageEncoder::new(P::ListInitial.spec(), 15).unwrap();
    for n in 1..=count {
        row(&mut page, n as u8, corrupt_last && n == count, body);
    }
    page.finish().unwrap()
}
fn role_page() -> PageBuffer {
    let mut page = PageEncoder::new(P::RoleGenerations.spec(), 1).unwrap();
    page.push(&[
        Cell::Integer(0),
        Cell::Integer(1),
        Cell::Text("integer"),
        Cell::Integer(1),
    ])
    .unwrap();
    page.finish().unwrap()
}
struct Mock {
    pages: VecDeque<(P, crate::Result<PageBuffer>)>,
    calls: Vec<P>,
}
impl Sql for Mock {
    fn query(&mut self, _: Statement, _: &[Value]) -> crate::Result<Option<Vec<Value>>> {
        panic!("unexpected scalar")
    }
    fn execute(&mut self, _: Statement, _: &[Value]) -> crate::Result<u64> {
        panic!("unexpected mutation")
    }
    fn query_page(&mut self, statement: P, _: &[Value]) -> crate::Result<PageBuffer> {
        self.calls.push(statement);
        let (expected, result) = self.pages.pop_front().expect("unexpected page");
        assert_eq!(statement, expected);
        result
    }
}
#[derive(Default)]
struct RecordingEncoder {
    appended: usize,
    fail: bool,
}
impl Encoder for RecordingEncoder {
    type Output = usize;
    fn append(&mut self, _: &KvDirent, _: Option<&KvSmallFileBox>) -> crate::Result<()> {
        if self.fail {
            return Err(crate::Error::Bounds);
        }
        self.appended += 1;
        Ok(())
    }
    fn finish(self, _: bool) -> crate::Result<usize> {
        Ok(self.appended)
    }
}
#[test]
fn late_bad_row_keeps_prior_role_and_append_error_precedence() {
    for (role_error, append_error, expected, appended) in [
        (true, false, PageFault::Expired, 0),
        (false, true, PageFault::Bounds, 0),
        (false, false, PageFault::Corrupt, 1),
    ] {
        let mut sql = Mock {
            pages: VecDeque::from([
                (P::ListInitial, Ok(list_page(2, true, true))),
                (
                    P::RoleGenerations,
                    if role_error {
                        Err(crate::Error::Expired)
                    } else {
                        Ok(role_page())
                    },
                ),
            ]),
            calls: vec![],
        };
        let mut encoder = RecordingEncoder {
            fail: append_error,
            ..Default::default()
        };
        let mut work = PageWork::default();
        assert!(
            matches!(process_page(&mut sql, &credential(), &ListRequest::new([0;16], ListCursor::Initial, 15, true), None, 15, &mut encoder, &mut work), Err(e) if e == expected)
        );
        assert_eq!(sql.calls, [P::ListInitial, P::RoleGenerations]);
        assert_eq!(encoder.appended, appended);
        assert!(!work.active.get());
    }
}
#[test]
fn progress_is_copy_and_scope_closes_before_next_page_even_on_error() {
    fn copy_type<T: Copy>() {}
    copy_type::<PageProgress>();
    copy_type::<PageFault>();
    let mut sql = Mock {
        pages: VecDeque::from([
            (P::ListInitial, Ok(list_page(1, false, false))),
            (P::ListMacContinue, Ok(list_page(0, false, false))),
        ]),
        calls: vec![],
    };
    let mut encoder = RecordingEncoder::default();
    let mut work = PageWork::default();
    let request = ListRequest::new([0; 16], ListCursor::Initial, 2, false);
    let progress = process_page(
        &mut sql,
        &credential(),
        &request,
        None,
        1,
        &mut encoder,
        &mut work,
    )
    .unwrap();
    assert_eq!(progress.added, 1);
    assert!(!progress.exhausted);
    assert!(!work.active.get());
    let last = progress.last.unwrap();
    assert_eq!(last.id, [1; 16]);
    let empty = process_page(
        &mut sql,
        &credential(),
        &request,
        Some(last),
        1,
        &mut encoder,
        &mut work,
    )
    .unwrap();
    assert_eq!(empty.added, 0);
    assert!(empty.exhausted);
    assert!(!work.active.get());
    assert_eq!(encoder.appended, 1);
}
#[test]
fn page_scope_rejects_nested_entry_and_restores_after_unwind() {
    let work = PageWork::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = work.enter().unwrap();
        assert!(matches!(work.enter(), Err(PageFault::Storage)));
        assert!(work.active.get());
        panic!("test unwind");
    }));
    assert!(result.is_err());
    assert!(!work.active.get());
    drop(work.enter().unwrap());
    assert!(!work.active.get());
}
#[test]
fn fixed_fault_conversion_preserves_codes_and_consumes_heap_payload() {
    for fault in [
        PageFault::Unauthenticated,
        PageFault::Permission,
        PageFault::Missing,
        PageFault::UnsupportedProfile,
        PageFault::Corrupt,
        PageFault::BadArguments,
        PageFault::StalePreparation,
        PageFault::CachedObjectMissing,
        PageFault::Invalid("fixed"),
        PageFault::IntegerRange,
        PageFault::Conflict,
        PageFault::KernelPermission,
        PageFault::Quota,
        PageFault::Bounds,
        PageFault::Storage,
        PageFault::Expired,
    ] {
        assert_eq!(PageFault::from(PersonalError::from(fault)), fault);
    }
    let stale = PersonalError::StaleCache(foks_proto::KvPathVersionVector {
        root_version: 1,
        directories: Vec::with_capacity(16),
    });
    assert_eq!(PageFault::from(stale), PageFault::Storage);
}
#[test]
fn owned_entry_fails_before_sql_only_when_owner_failed() {
    use crate::list_encoder::owned::ListRetainedOwner;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    struct Owner(AtomicBool);
    impl ListRetainedOwner for Owner {
        fn reserve_retained(&self, _: usize) -> bool {
            true
        }
        fn failed(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
        fn fail(&self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    struct Fault(usize);
    impl Sql for Fault {
        fn query(&mut self, _: Statement, _: &[Value]) -> crate::Result<Option<Vec<Value>>> {
            self.0 += 1;
            Err(crate::Error::Expired)
        }
        fn execute(&mut self, _: Statement, _: &[Value]) -> crate::Result<u64> {
            panic!("mutation")
        }
    }
    for failed in [false, true] {
        let owner = Arc::new(Owner(AtomicBool::new(failed)));
        let mut sql = Fault(0);
        let result = crate::personal::read_personal_list_owned(
            &mut sql,
            &credential(),
            &ListRequest::new([0; 16], ListCursor::Initial, 1, false),
            1,
            1 << 20,
            owner.clone(),
        );
        let expected = if failed {
            crate::Error::Bounds
        } else {
            crate::Error::Expired
        };
        assert!(matches!(result, Err(PersonalError::Kernel(e)) if e == expected));
        assert_eq!(sql.0, usize::from(!failed));
        assert_eq!(owner.failed(), failed);
    }
}

#[test]
fn owned_page_latches_late_errors_after_preserving_role_query_precedence() {
    use crate::list_encoder::owned::{ListRetainedOwner, OwnedListEncoder};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    struct Owner(AtomicBool);
    impl ListRetainedOwner for Owner {
        fn reserve_retained(&self, _: usize) -> bool {
            true
        }
        fn reserve_page_workspace(&self, _: usize) -> bool {
            true
        }
        fn failed(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
        fn fail(&self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    for (role_error, expected) in [(true, PageFault::Expired), (false, PageFault::Corrupt)] {
        let mut sql = Mock {
            pages: VecDeque::from([
                (P::ListInitial, Ok(list_page(2, true, true))),
                (
                    P::RoleGenerations,
                    if role_error {
                        Err(crate::Error::Expired)
                    } else {
                        Ok(role_page())
                    },
                ),
            ]),
            calls: vec![],
        };
        let owner = Arc::new(Owner(AtomicBool::new(false)));
        let mut encoder = OwnedListEncoder::new(15, owner.clone()).unwrap();
        let mut work = PageWork::new(&encoder).unwrap();
        let request = ListRequest::new([0; 16], ListCursor::Initial, 15, true);
        let result = process_page(
            &mut sql,
            &credential(),
            &request,
            None,
            15,
            &mut encoder,
            &mut work,
        );
        assert!(matches!(result,Err(error) if error == expected));
        assert_eq!(sql.calls, [P::ListInitial, P::RoleGenerations]);
        assert!(!work.active.get());
        assert!(owner.failed());
        let before = sql.calls.len();
        assert!(matches!(
            process_page(
                &mut sql,
                &credential(),
                &request,
                None,
                15,
                &mut encoder,
                &mut work
            ),
            Err(PageFault::Bounds)
        ));
        assert_eq!(sql.calls.len(), before);
    }
}
