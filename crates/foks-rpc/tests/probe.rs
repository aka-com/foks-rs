use std::io::Cursor;

use foks_rpc::{
    decode_probe_response, encode_probe_request, encode_probe_success_response, read_frame,
    read_probe_response, Error, DEFAULT_MAX_FRAME_LENGTH,
};

const FIXTURE_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app"
);

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURE_DIR}/{name}")).expect("checked-in fixture")
}

#[test]
fn probe_request_matches_official_go_rpc_encoder() {
    let expected = fixture("probe-request.frame");
    let encoded = encode_probe_request("foks.app", 0, None).unwrap();
    assert_eq!(encoded, expected);
}

#[test]
fn success_response_preserves_exact_probe_bytes() {
    let probe = fixture("probe-response.snowp");
    let response = encode_probe_success_response(&probe).unwrap();
    let decoded =
        read_probe_response(&mut Cursor::new(response), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(decoded, probe);
}

#[test]
fn frame_reader_enforces_limit_before_allocation() {
    let error = read_frame(&mut Cursor::new([0xcd, 0x01, 0x00]), 255).unwrap_err();
    assert!(matches!(
        error,
        Error::FrameTooLarge {
            received: 256,
            maximum: 255
        }
    ));
}

#[test]
fn frame_reader_rejects_noncanonical_lengths() {
    for bytes in [[0xcc, 0x7f].as_slice(), [0xcd, 0x00, 0xff].as_slice()] {
        assert!(matches!(
            read_frame(&mut Cursor::new(bytes), DEFAULT_MAX_FRAME_LENGTH),
            Err(Error::FrameLengthMarker(_))
        ));
    }
}

#[test]
fn response_rejects_wrong_sequence_and_trailing_data() {
    let probe = fixture("probe-response.snowp");
    let framed = encode_probe_success_response(&probe).unwrap();
    let content = read_frame(&mut Cursor::new(framed), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert!(matches!(
        decode_probe_response(&content, 1),
        Err(Error::Sequence {
            expected: 1,
            received: 0
        })
    ));

    let mut trailing = content;
    trailing.push(0xc0);
    assert!(matches!(
        decode_probe_response(&trailing, 0),
        Err(Error::Envelope { .. })
    ));
}

#[test]
fn response_surfaces_typed_foks_status() {
    // [response, seq=0, Status{GENERIC_ERROR: "nope"}, empty result]
    let response = [
        0x94, 0x01, 0x00, 0x92, 0x64, 0x81, 0xa1, b'0', 0xa4, b'n', b'o', b'p', b'e', 0x80,
    ];
    let error = decode_probe_response(&response, 0).unwrap_err();
    assert_eq!(error.to_string(), "FOKS server returned status 100: nope");
}

#[test]
fn probe_request_encodes_incremental_pin_fields() {
    let host_id = [0x02; 33];
    let framed = encode_probe_request("example.test", 256, Some(&host_id)).unwrap();
    let content = read_frame(&mut Cursor::new(framed), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(content[0], 0x95);
    assert!(content
        .windows(3)
        .any(|window| window == [0xcd, 0x01, 0x00]));
    assert!(content
        .windows(35)
        .any(|window| { window[0] == 0xc4 && window[1] == 33 && window[2..] == host_id }));
}

#[test]
fn invalid_hostname_is_rejected() {
    for hostname in ["", "foks\0.app", "fóks.app"] {
        assert!(matches!(
            encode_probe_request(hostname, 0, None),
            Err(Error::Hostname)
        ));
    }
}
