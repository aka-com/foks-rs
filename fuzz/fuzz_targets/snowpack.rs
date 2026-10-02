#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|input: &[u8]| {
    if input.len() > 1_048_576 {
        return;
    }
    if let Ok(value) = foks_snowpack::decode(input) {
        assert_eq!(foks_snowpack::encode(&value).unwrap(), input);
    }
    let _ = foks_snowpack::decode_prefix(input);
    let _ = foks_snowpack::decode_sensitive(input);
    let _ = foks_snowpack::validate_signable(input);
});
