//! Per-protocol DataWrap behavior (go-foks v0.1.9 interop #2).
//!
//! go-foks wraps most protocols' arguments and results in an `rpc.DataWrap`
//! `{Data, Header}` map, but places the team protocols (TeamLoader, TeamAdmin,
//! TeamMember, TeamGuest) and Kex bare on the wire. These tests lock in that
//! split on both the call and response paths.

use std::io::Cursor;

use foks_rpc::{
    decode_bare_response, encode_bare_success_response_at, encode_bare_void_success_response_at,
    encode_call, is_headerless_protocol, read_bare_response, read_bare_void_response, read_call,
    BEACON_PROTOCOL_ID, DEFAULT_MAX_FRAME_LENGTH, KV_STORE_PROTOCOL_ID, MERKLE_QUERY_PROTOCOL_ID,
    PROBE_PROTOCOL_ID, REG_PROTOCOL_ID, TEAM_ADMIN_PROTOCOL_ID, TEAM_LOADER_PROTOCOL_ID,
    TEAM_MEMBER_PROTOCOL_ID, USER_PROTOCOL_ID,
};
use foks_snowpack::{encode, Value};

const TEAM_GUEST_PROTOCOL_ID: u64 = 0xf6d7585c;
const KEX_PROTOCOL_ID: u64 = 0xae4df828;

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn headerless_predicate_matches_the_go_v019_protocol_split() {
    for id in [
        TEAM_LOADER_PROTOCOL_ID,
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_MEMBER_PROTOCOL_ID,
        TEAM_GUEST_PROTOCOL_ID,
        KEX_PROTOCOL_ID,
    ] {
        assert!(is_headerless_protocol(id), "{id:#010x} should be bare");
    }
    for id in [
        PROBE_PROTOCOL_ID,
        MERKLE_QUERY_PROTOCOL_ID,
        REG_PROTOCOL_ID,
        USER_PROTOCOL_ID,
        KV_STORE_PROTOCOL_ID,
        BEACON_PROTOCOL_ID,
    ] {
        assert!(!is_headerless_protocol(id), "{id:#010x} should be wrapped");
    }
}

#[test]
fn team_calls_are_bare_and_decode_to_the_exact_argument() {
    let argument = encode(&Value::Array(vec![Value::Text(b"auditteam".to_vec())])).unwrap();
    let frame = encode_call(TEAM_ADMIN_PROTOCOL_ID, 0, &argument, 7).unwrap();

    // A bare call places the argument directly in the payload slot: no DataWrap
    // map keys are present on the wire.
    assert!(!contains(&frame, b"Data"));
    assert!(!contains(&frame, b"Header"));

    let call = read_call(&mut Cursor::new(&frame), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(call.protocol_id(), TEAM_ADMIN_PROTOCOL_ID);
    assert_eq!(call.method_position(), 0);
    assert_eq!(call.sequence(), 7);
    assert_eq!(call.argument(), argument.as_slice());
}

#[test]
fn wrapped_calls_keep_the_datawrap_envelope() {
    let argument = encode(&Value::Array(vec![Value::Binary(vec![1; 16])])).unwrap();
    let frame = encode_call(USER_PROTOCOL_ID, 14, &argument, 0).unwrap();

    // A wrapped call carries the canonical DataWrap map keys.
    assert!(contains(&frame, b"Data"));
    assert!(contains(&frame, b"Header"));

    let call = read_call(&mut Cursor::new(&frame), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(call.protocol_id(), USER_PROTOCOL_ID);
    assert_eq!(call.argument(), argument.as_slice());
}

#[test]
fn bare_success_response_round_trips_without_a_datawrap() {
    let result = encode(&Value::Binary(vec![0x5a; 16])).unwrap();
    let frame = encode_bare_success_response_at(&result, 3).unwrap();

    assert!(!contains(&frame, b"Data"));
    assert!(!contains(&frame, b"Header"));
    assert_eq!(
        read_bare_response(&mut Cursor::new(&frame), DEFAULT_MAX_FRAME_LENGTH, 3).unwrap(),
        result
    );
}

#[test]
fn bare_void_response_is_a_nil_result_slot() {
    // go-foks void handlers on a bare protocol return a nil result, so the
    // response is [RESPONSE(1), seq, nil err, nil res] framed. Lock the exact
    // wire bytes for sequence 5.
    let frame = encode_bare_void_success_response_at(5).unwrap();
    assert_eq!(frame, [0x05, 0x94, 0x01, 0x05, 0xc0, 0xc0]);

    read_bare_void_response(&mut Cursor::new(&frame), DEFAULT_MAX_FRAME_LENGTH, 5).unwrap();
    // The bare success decoder recovers the nil result slot verbatim.
    assert_eq!(decode_bare_response(&frame[1..], 5).unwrap(), [0xc0]);
}
