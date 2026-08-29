//! Inbound control-message tolerance (go-foks v0.1.9 interop #3).
//!
//! go-snowpack-rpc may place NOTIFY and CANCEL control messages (method types
//! 2, 3, 6, 7) on an otherwise idle connection and never expects a reply. The
//! server must parse and discard them instead of treating the unexpected
//! method type as a fatal protocol error, while a genuinely unknown method
//! type (or a non-array envelope) stays fatal.

use std::io::Cursor;

use foks_rpc::{
    decode_message, encode_call, read_message, InboundMessage, DEFAULT_MAX_FRAME_LENGTH,
    PROBE_METHOD_POSITION, PROBE_PROTOCOL_ID,
};
use foks_snowpack::{encode, Value};

fn message(method: u64, rest: Vec<Value>) -> Vec<u8> {
    let mut items = vec![Value::Unsigned(method)];
    items.extend(rest);
    encode(&Value::Array(items)).unwrap()
}

#[test]
fn call_v2_is_classified_as_a_call() {
    let argument = encode(&Value::Array(vec![
        Value::Text(b"host".to_vec()),
        Value::Unsigned(0),
        Value::Null,
    ]))
    .unwrap();
    let frame = encode_call(PROBE_PROTOCOL_ID, PROBE_METHOD_POSITION, &argument, 9).unwrap();
    match read_message(&mut Cursor::new(&frame), DEFAULT_MAX_FRAME_LENGTH).unwrap() {
        InboundMessage::Call(call) => {
            assert_eq!(call.protocol_id(), PROBE_PROTOCOL_ID);
            assert_eq!(call.method_position(), PROBE_METHOD_POSITION);
            assert_eq!(call.sequence(), 9);
            assert_eq!(call.argument(), argument.as_slice());
        }
        InboundMessage::Control => panic!("a CALL_V2 was misclassified as a control message"),
    }
}

#[test]
fn notify_and_cancel_are_discardable_control_messages() {
    // NOTIFY [2, name, arg]
    assert_eq!(
        decode_message(&message(
            2,
            vec![Value::Text(b"Svc.event".to_vec()), Value::Null]
        ))
        .unwrap(),
        InboundMessage::Control
    );
    // CANCEL [3, seq, name]
    assert_eq!(
        decode_message(&message(
            3,
            vec![Value::Unsigned(7), Value::Text(b"Svc.method".to_vec())]
        ))
        .unwrap(),
        InboundMessage::Control
    );
    // NOTIFY_V2 [6, puid, pos, arg]
    assert_eq!(
        decode_message(&message(
            6,
            vec![Value::Binary(vec![1; 8]), Value::Unsigned(0), Value::Null]
        ))
        .unwrap(),
        InboundMessage::Control
    );
    // CANCEL_V2 [7, seq, puid, pos]
    assert_eq!(
        decode_message(&message(
            7,
            vec![
                Value::Unsigned(1),
                Value::Binary(vec![2; 8]),
                Value::Unsigned(0)
            ]
        ))
        .unwrap(),
        InboundMessage::Control
    );
}

#[test]
fn known_but_unsupported_and_unknown_method_types_stay_fatal() {
    // 0 = CALL (v1), 1 = RESPONSE, 4 = CALL_COMPRESSED: never sent by go-foks
    // for these routes and not in the discard set, so they remain fatal (a
    // compressed call in particular expects a reply and must not be dropped).
    for method in [0u64, 1, 4, 8, 99] {
        assert!(
            decode_message(&message(method, vec![Value::Unsigned(0)])).is_err(),
            "method type {method} should be fatal"
        );
    }
}

#[test]
fn non_array_envelopes_and_malformed_control_frames_stay_fatal() {
    // A non-array envelope is rejected by the packetizer.
    assert!(decode_message(&encode(&Value::Unsigned(3)).unwrap()).is_err());
    assert!(decode_message(&encode(&Value::Null).unwrap()).is_err());
    // A control frame with trailing garbage past its array is rejected.
    let mut cancel = message(3, vec![Value::Unsigned(7), Value::Text(b"Svc.m".to_vec())]);
    cancel.push(0xc0);
    assert!(decode_message(&cancel).is_err());
}
