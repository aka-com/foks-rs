use super::*;
use foks_proto::{KvDirectoryStatus, KvNodeId, Role, RoleAndGeneration, SecretBox};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

#[derive(Default)]
struct Owner {
    calls: Mutex<Vec<usize>>,
    fail_at: usize,
    failed: AtomicBool,
}
impl ListRetainedOwner for Owner {
    fn reserve_retained(&self, bytes: usize) -> bool {
        let mut calls = self.calls.lock().unwrap();
        calls.push(bytes);
        calls.len() != self.fail_at
    }
    fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }
    fn fail(&self) {
        self.failed.store(true, Ordering::SeqCst);
    }
}
fn owner(fail_at: usize) -> Arc<Owner> {
    Arc::new(Owner {
        fail_at,
        ..Owner::default()
    })
}
fn entry() -> KvDirent {
    KvDirent::new(
        [0; 16],
        [1; 16],
        KvNodeId([3; 17]),
        1,
        1,
        Role::OWNER,
        [2; 32],
        SecretBox {
            nonce: [3; 16],
            ciphertext: vec![4; 32],
        },
        KvDirectoryStatus::Active,
        [5; 32],
        1,
    )
    .unwrap()
}
fn small() -> KvSmallFileBox {
    KvSmallFileBox {
        key: RoleAndGeneration {
            role: Role::OWNER,
            generation: 1,
        },
        ciphertext: vec![6; 32],
    }
}
#[test]
fn empty_and_nonempty_outputs_keep_owner_through_frame_and_final_drop() {
    for count in [0, 1] {
        for framed in [false, true] {
            let owner = owner(0);
            let weak = Arc::downgrade(&owner);
            let mut owned = OwnedListEncoder::new(count, owner.clone()).unwrap();
            let mut old = ListEncoder::new(count).unwrap();
            if count != 0 {
                owned.append(&entry(), Some(&small())).unwrap();
                old.append(&entry(), Some(&small())).unwrap();
            }
            let expected = old.finish(true).unwrap();
            let raw = owned.finish(true).unwrap();
            assert_eq!(raw.as_bytes(), expected);
            assert_eq!(raw.len(), expected.len());
            let charges = owner.calls.lock().unwrap().clone();
            assert_eq!(charges.last().copied(), Some(expected.len()));
            drop(owner);
            assert!(weak.upgrade().is_some());
            if framed {
                let expected = foks_rpc::encode_success_response_at(&expected, 256).unwrap();
                let frame = frame_owned_list(raw, 256).unwrap();
                assert_eq!(frame.as_bytes(), expected);
                assert_eq!(frame.len(), expected.len());
                assert!(weak.upgrade().is_some());
                assert_eq!(*weak.upgrade().unwrap().calls.lock().unwrap(), charges);
                drop(frame);
            } else {
                drop(raw);
            }
            assert!(weak.upgrade().is_none());
        }
    }
}
#[test]
fn growth_and_finish_refusal_latch_only_owned_path() {
    // With a body: entries growth, extended growth, then final output.
    for fail_at in 1..=3 {
        let owner = owner(fail_at);
        let mut owned = OwnedListEncoder::new(1, owner.clone()).unwrap();
        let result = owned.append(&entry(), Some(&small()));
        if fail_at < 3 {
            assert_eq!(result, Err(Error::Bounds));
            assert_eq!(owned.append(&entry(), None), Err(Error::Bounds));
        } else {
            result.unwrap();
        }
        assert!(matches!(owned.finish(true), Err(Error::Bounds)));
        assert!(owner.failed());
        assert_eq!(owner.calls.lock().unwrap().len(), fail_at);
    }
    // A failed append still permits the original public encoder's prefix finish.
    let mut original = ListEncoder::new(0).unwrap();
    assert_eq!(original.append(&entry(), None), Err(Error::Bounds));
    assert_eq!(original.finish(true).unwrap(), vec![0x93, 0xc0, 0xc3, 0xc0]);
}
#[test]
fn frame_errors_latch_owner_but_preserve_original_error() {
    for raw in [vec![0x90], vec![0; foks_rpc::DEFAULT_MAX_FRAME_LENGTH + 1]] {
        let expected =
            foks_rpc::encode_success_response_owned_at(raw.clone(), u64::MAX).unwrap_err();
        let owner = owner(0);
        let value = OwnedListBytes {
            bytes: raw,
            owner: owner.clone(),
        };
        let actual = match frame_owned_list(value, u64::MAX) {
            Ok(_) => panic!("unexpected frame"),
            Err(e) => e,
        };
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        assert!(owner.failed());
    }
}
struct RefuseFrame;
impl foks_rpc::ResponseAllocationBudget for RefuseFrame {
    fn reserve(&self, _: usize, _: bool) -> bool {
        false
    }
}
#[test]
fn frame_refusal_releases_output_and_latches_shared_owner() {
    let owner = owner(0);
    let weak = Arc::downgrade(&owner);
    let raw = OwnedListEncoder::new(0, owner.clone())
        .unwrap()
        .finish(true)
        .unwrap();
    let result = foks_rpc::with_response_budget(Arc::new(RefuseFrame), || frame_owned_list(raw, 0));
    assert!(
        matches!(result, Err(foks_rpc::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert!(owner.failed());
    drop(result);
    drop(owner);
    assert!(weak.upgrade().is_none());
}
