#![no_main]
use libfuzzer_sys::fuzz_target;
#[path = "../support/schema.rs"]
mod schema;
fuzz_target!(|input: &[u8]| {
    if input.len() > 2_097_152 {
        return;
    }
    // The first byte selects a server argument decoder or a realtime method
    // position. decode_call has already unwrapped these bytes on the server.
    if let Some(((_, decode), bytes)) = schema::select(schema::SERVER, input) {
        decode(bytes);
    }
});
