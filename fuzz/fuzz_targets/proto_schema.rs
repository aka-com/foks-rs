#![no_main]
use libfuzzer_sys::fuzz_target;
#[path = "../support/schema.rs"]
mod schema;
fuzz_target!(|input: &[u8]| {
    if input.len() > 2_097_152 {
        return;
    }
    // The first byte selects a typed decoder; its encoding checks panic on
    // a violation and rejected inputs are expected.
    if let Some(((_, decode), bytes)) = schema::select(schema::PROTO, input) {
        decode(bytes);
    }
});
