#![no_main]
use libfuzzer_sys::fuzz_target;
#[path = "../support/verify.rs"]
mod verify;
fuzz_target!(|input: &[u8]| {
    if input.len() > 2_097_152 {
        return;
    }
    let _ = verify::run(input);
});
