use std::io::Cursor;

use foks_rpc::{
    decode_probe_response, encode_probe_success_response, read_frame, read_probe_response,
    DEFAULT_MAX_FRAME_LENGTH,
};
use foks_snowpack::{encode, Value};
use quickcheck::{quickcheck, TestResult};

quickcheck! {
    fn successful_rpc_preserves_arbitrary_canonical_binary(bytes: Vec<u8>) -> TestResult {
        if bytes.len() > 64 * 1024 {
            return TestResult::discard();
        }
        let payload = encode(&Value::Binary(bytes)).unwrap();
        let frame = encode_probe_success_response(&payload).unwrap();
        let decoded = read_probe_response(
            &mut Cursor::new(frame),
            DEFAULT_MAX_FRAME_LENGTH,
        ).unwrap();
        TestResult::from_bool(decoded == payload)
    }

    fn arbitrary_untrusted_envelopes_never_panic(bytes: Vec<u8>) -> bool {
        std::panic::catch_unwind(|| {
            let _ = decode_probe_response(&bytes, 0);
            let _ = read_frame(&mut Cursor::new(&bytes), DEFAULT_MAX_FRAME_LENGTH);
        }).is_ok()
    }
}
