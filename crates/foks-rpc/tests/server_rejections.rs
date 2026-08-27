use std::io::Cursor;

use foks_rpc::{decode_call, encode_call, read_call, Error, DEFAULT_MAX_FRAME_LENGTH};
use foks_snowpack::{encode, Value};

#[test]
fn rejects_every_truncation_without_panicking() {
    let argument = encode(&Value::Array(vec![Value::Unsigned(1)])).unwrap();
    let request = encode_call(1, 2, &argument, 3).unwrap();
    for length in 0..request.len() {
        assert!(read_call(
            &mut Cursor::new(&request[..length]),
            DEFAULT_MAX_FRAME_LENGTH
        )
        .is_err());
    }
}

#[test]
fn rejects_oversized_frames_before_allocating_the_payload() {
    let bytes = [0xcd, 0x10, 0x00];
    assert!(matches!(
        read_call(&mut Cursor::new(bytes), 1024),
        Err(Error::FrameTooLarge {
            received: 4096,
            maximum: 1024
        })
    ));
}

#[test]
fn rejects_noncanonical_argument_and_trailing_values() {
    let argument = encode(&Value::Unsigned(1)).unwrap();
    let request = encode_call(1, 2, &argument, 3).unwrap();
    let content = &request[1..];

    let mut trailing = content.to_vec();
    trailing.push(0xc0);
    assert!(decode_call(&trailing).is_err());

    let mut noncanonical = content.to_vec();
    let argument_offset = noncanonical
        .windows(5)
        .position(|window| window == [0xa4, b'D', b'a', b't', b'a'])
        .unwrap()
        + 5;
    noncanonical.splice(argument_offset..argument_offset + 1, [0xcc, 0x01]);
    assert!(decode_call(&noncanonical).is_err());
}
