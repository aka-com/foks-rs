#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|input: &[u8]| {
    if input.len() > 1_048_576 {
        return;
    }
    let _ = foks_rpc::decode_probe_response(input, 0);
    let _ = foks_rpc::read_frame(&mut std::io::Cursor::new(input), 1_048_576);
    let _ = foks_rpc::read_probe_response(&mut std::io::Cursor::new(input), 1_048_576);
});
