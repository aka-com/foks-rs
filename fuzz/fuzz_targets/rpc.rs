#![no_main]
use libfuzzer_sys::fuzz_target;

fn decode_content(content: &[u8]) {
    let _ = foks_rpc::decode_probe_response(content, 0);
    let _ = foks_rpc::decode_void_response(content, 0);
    let _ = foks_rpc::decode_bare_response(content, 0);
    let _ = foks_rpc::decode_bare_void_response(content, 0);
}

fuzz_target!(|input: &[u8]| {
    if input.len() > 1_048_576 {
        return;
    }
    decode_content(input);
    // The Go fixture seeds are framed responses; decode a valid frame's
    // content as well, so mutations reach the envelope decoders.
    if let Ok(content) = foks_rpc::read_frame(&mut std::io::Cursor::new(input), 1_048_576) {
        decode_content(&content);
    }
    let _ = foks_rpc::read_probe_response(&mut std::io::Cursor::new(input), 1_048_576);
});
