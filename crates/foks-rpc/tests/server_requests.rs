use std::io::{Cursor, Read};

use foks_rpc::{
    encode_call, read_call, DEFAULT_MAX_FRAME_LENGTH, PROBE_METHOD_POSITION, PROBE_PROTOCOL_ID,
};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../foks-snowpack/tests/fixtures/foks-v0.1.9")
            .join(path),
    )
    .unwrap()
}

#[test]
fn official_probe_call_decodes_and_round_trips_exactly() {
    let bytes = fixture("foks.app/probe-request.frame");
    let call = read_call(&mut Cursor::new(&bytes), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(call.sequence(), 0);
    assert_eq!(call.protocol_id(), PROBE_PROTOCOL_ID);
    assert_eq!(call.method_position(), PROBE_METHOD_POSITION);
    assert_eq!(
        encode_call(
            call.protocol_id(),
            call.method_position(),
            call.argument(),
            call.sequence(),
        )
        .unwrap(),
        bytes
    );
}

#[test]
fn call_decoding_is_independent_of_reader_chunk_boundaries() {
    let bytes = fixture("signup/reserve-request.frame");
    for chunk_size in 1..=bytes.len() {
        let mut reader = ChunkedReader::new(&bytes, chunk_size);
        let call = read_call(&mut reader, DEFAULT_MAX_FRAME_LENGTH).unwrap();
        assert_eq!(call.sequence(), 1);
        assert_eq!(reader.remaining(), 0);
    }
}

struct ChunkedReader<'a> {
    inner: Cursor<&'a [u8]>,
    chunk_size: usize,
}

impl<'a> ChunkedReader<'a> {
    fn new(bytes: &'a [u8], chunk_size: usize) -> Self {
        Self {
            inner: Cursor::new(bytes),
            chunk_size,
        }
    }

    fn remaining(&self) -> usize {
        self.inner.get_ref().len() - self.inner.position() as usize
    }
}

impl Read for ChunkedReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(self.chunk_size);
        self.inner.read(&mut output[..length])
    }
}
