use super::*;
use crate::{encode_success_response_at, with_response_budget, ResponseAllocationBudget};
use foks_snowpack::{encode, Value};
use std::sync::{Arc, Mutex};

fn same(raw: Vec<u8>, sequence: u64) {
    let old = encode_success_response_at(&raw, sequence);
    let new = encode_success_response_owned_at(raw, sequence);
    match (old, new) {
        (Ok(a), Ok(b)) => assert_eq!(a, b),
        (Err(a), Err(b)) => {
            assert_eq!(std::mem::discriminant(&a), std::mem::discriminant(&b));
            assert_eq!(a.to_string(), b.to_string());
        }
        pair => panic!("owned/borrowed mismatch: {pair:?}"),
    }
}
#[test]
fn exact_sequence_and_frame_boundaries_keep_the_borrowed_oracle() {
    for sequence in [
        0,
        127,
        128,
        255,
        256,
        65535,
        65536,
        u32::MAX as u64,
        u32::MAX as u64 + 1,
        u64::MAX,
    ] {
        // A bin32 header is five bytes. Derive overhead from the old encoder,
        // independently of the new length calculation.
        let empty = encode(&Value::Binary(vec![])).unwrap();
        let framed = encode_success_response_at(&empty, sequence).unwrap();
        let content = crate::read_frame(&mut framed.as_slice(), DEFAULT_MAX_FRAME_LENGTH).unwrap();
        let overhead = content.len() - empty.len();
        for delta in [-1isize, 0, 1] {
            let length = (DEFAULT_MAX_FRAME_LENGTH - overhead)
                .checked_add_signed(delta)
                .unwrap();
            same(
                encode(&Value::Binary(vec![0x5a; length - 5])).unwrap(),
                sequence,
            );
        }
        for value in [
            Value::Null,
            Value::Unsigned(u64::MAX),
            Value::Array(vec![Value::Null]),
            Value::Array(vec![
                Value::Binary(vec![1; 17]),
                Value::Text(b"value".to_vec()),
            ]),
        ] {
            same(encode(&value).unwrap(), sequence);
        }
    }
}
#[test]
fn malformed_and_raw_oversize_keep_error_precedence() {
    for raw in [
        vec![],
        vec![0xc1],
        vec![0x90], // Empty Snowpack arrays are invalid; compare both framing errors.
        vec![0xc0, 0xc0],
        vec![0xc4, 3, 0],
        vec![0xc1; DEFAULT_MAX_FRAME_LENGTH + 1],
    ] {
        same(raw, u64::MAX);
    }
}
#[derive(Default)]
struct Budget {
    calls: Mutex<Vec<(usize, bool)>>,
    allow: bool,
}
impl ResponseAllocationBudget for Budget {
    fn reserve(&self, bytes: usize, control: bool) -> bool {
        self.calls.lock().unwrap().push((bytes, control));
        self.allow
    }
}
#[test]
fn admission_before_validation_and_one_fallible_final_allocation() {
    let refused = Arc::new(Budget::default());
    let error = with_response_budget(refused.clone(), || {
        encode_with_reservation(vec![0xc1], 0, |_, _| panic!("must not allocate"))
    })
    .unwrap_err();
    assert!(matches!(error, Error::Io(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    assert_eq!(*refused.calls.lock().unwrap(), vec![(258, false)]);
    let allowed = Arc::new(Budget {
        allow: true,
        ..Budget::default()
    });
    let raw = encode(&Value::Binary(vec![3; 800])).unwrap();
    let wanted = encode_success_response_at(&raw, u64::MAX).unwrap();
    let mut allocations = 0;
    let got = with_response_budget(allowed.clone(), || {
        encode_with_reservation(raw.clone(), u64::MAX, |buffer, length| {
            allocations += 1;
            assert!(buffer.is_empty());
            assert_eq!(length, wanted.len());
            buffer.try_reserve_exact(length).unwrap();
            Ok(())
        })
    })
    .unwrap();
    assert_eq!(got, wanted);
    assert_eq!(allocations, 1);
    assert_eq!(
        *allowed.calls.lock().unwrap(),
        vec![(2 * (raw.len() + 128), false)]
    );
    let error = encode_with_reservation(raw, 0, |_, _| {
        Err(Error::Io(std::io::ErrorKind::OutOfMemory.into()))
    })
    .unwrap_err();
    assert!(matches!(error, Error::Io(e) if e.kind() == std::io::ErrorKind::OutOfMemory));
    assert!(matches!(
        encode_with_reservation(vec![0xc1], 0, |_, _| panic!(
            "validation must precede allocation"
        )),
        Err(Error::Snowpack(_))
    ));
}
