#![no_main]
use foks_keystore::state_archive::{ArchiveReader, ArchiveWriter, StateTransferKey};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|input: &[u8]| {
    if input.len() > 2_097_152 {
        return;
    }
    let key =
        StateTransferKey::parse(&format!("FOKS-STATE-TRANSFER-V1:{}", "07".repeat(32))).unwrap();
    // Raw mutation includes the checked-in authenticated two-entry fixture.
    if let Ok(mut reader) = ArchiveReader::new(input, &key) {
        let _ = reader.read_entry(3, &mut std::io::sink());
        let _ = reader.read_entry(0, &mut std::io::sink());
        let _ = reader.finish();
    }
    // Structured generation always crosses authentication into entry/trailer
    // processing, instead of spending the whole campaign rejecting tags.
    let split = if input.first().is_some_and(|b| b & 1 == 1) {
        input.len()
    } else {
        input.len() / 2
    };
    let mut writer = ArchiveWriter::new(Vec::new(), &key, b"fuzz-v1").unwrap();
    for part in [&input[..split], &input[split..]] {
        writer
            .write_entry(part.len() as u64, &mut &part[..])
            .unwrap();
    }
    let bytes = writer.finish().unwrap();
    let mut reader = ArchiveReader::new(bytes.as_slice(), &key).unwrap();
    assert_eq!(reader.manifest(), b"fuzz-v1");
    for expected in [&input[..split], &input[split..]] {
        let mut actual = Vec::new();
        reader
            .read_entry(expected.len() as u64, &mut actual)
            .unwrap();
        assert_eq!(actual, expected);
    }
    reader.finish().unwrap();
    // Valid authentication with an invalid caller-supplied entry length must
    // poison the reader, including on a subsequent attempt to finish.
    let mut reader = ArchiveReader::new(bytes.as_slice(), &key).unwrap();
    assert!(reader
        .read_entry(split as u64 + 1, &mut std::io::sink())
        .is_err());
    assert!(reader.finish().is_err());
});
