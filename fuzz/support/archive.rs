//! Independent synthetic v1 wire encoder. It deliberately authenticates malformed
//! sequences so coverage can cross AEAD verification. Never used by production.
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use foks_keystore::state_archive::{ArchiveReader, StateTransferKey, CHUNK_BYTES};
use sha2::{Digest, Sha256};
use std::io::{self, Write};

pub const SCENARIOS: u8 = 13;
pub fn key() -> StateTransferKey {
    StateTransferKey::parse(&format!("FOKS-STATE-TRANSFER-V1:{}", "07".repeat(32))).unwrap()
}

struct Encoder {
    header: [u8; 73],
    cipher: XChaCha20Poly1305,
    counter: u64,
    bytes: Vec<u8>,
}
impl Encoder {
    fn new() -> Self {
        let mut header = [0x31; 73];
        header[..8].copy_from_slice(b"FOKSST01");
        header[8] = 1;
        let mut hash = Sha256::new();
        hash.update(b"foks-state-transfer-key-v1\0");
        hash.update([7; 32]);
        hash.update(&header[9..41]);
        hash.update(&header[41..57]);
        let derived: [u8; 32] = hash.finalize().into();
        Self {
            header,
            cipher: XChaCha20Poly1305::new_from_slice(&derived).unwrap(),
            counter: 0,
            bytes: header.to_vec(),
        }
    }
    fn frame(&mut self, entry: u32, chunk: u32, data: &[u8], finality: bool) {
        let mut fields = Vec::with_capacity(21);
        fields.extend_from_slice(&self.counter.to_be_bytes());
        fields.extend_from_slice(&entry.to_be_bytes());
        fields.extend_from_slice(&chunk.to_be_bytes());
        fields.extend_from_slice(&(data.len() as u32).to_be_bytes());
        fields.push(u8::from(finality));
        let mut nonce = [0; 24];
        nonce[..16].copy_from_slice(&self.header[57..]);
        nonce[16..].copy_from_slice(&self.counter.to_be_bytes());
        let mut aad = self.header.to_vec();
        aad.extend_from_slice(&fields);
        let sealed = self
            .cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: data,
                    aad: &aad,
                },
            )
            .unwrap();
        self.bytes.extend(fields);
        self.bytes.extend(sealed);
        self.counter += 1;
    }
}

/// Control bytes select corruption, entry count and chunk-boundary expansion;
/// all remaining bytes vary both the manifest and entry plaintext.
pub fn run(input: &[u8]) {
    let mode = input.first().copied().unwrap_or(0) % SCENARIOS;
    let count = 1 + usize::from(input.get(1).copied().unwrap_or(0) % 4);
    let manifest = if input.len() > 3 {
        &input[3..input.len().min(67)]
    } else {
        b"{}"
    };
    let mut body = input.get(3..).unwrap_or_default().to_vec();
    // Compact seeds exercise both sides of a chunk boundary without a corpus
    // full of megabyte-sized redundant files. Most cases remain small/fast.
    if input.get(2).is_some_and(|b| b & 0x80 != 0) {
        let delta = usize::from(input[2] & 3);
        body.resize(CHUNK_BYTES - 1 + delta, input.first().copied().unwrap_or(0));
    }
    let entries: Vec<&[u8]> = (0..count)
        .map(|index| {
            if index + 1 == count {
                body.as_slice()
            } else {
                &body[..body.len().min(index)]
            }
        })
        .collect();
    let mut encoder = Encoder::new();
    encoder.frame(u32::MAX, 0, manifest, true);
    for (index, entry) in entries.iter().enumerate() {
        let chunks: Vec<&[u8]> = if entry.is_empty() {
            vec![&[]]
        } else {
            entry.chunks(CHUNK_BYTES).collect()
        };
        for (chunk, bytes) in chunks.iter().enumerate() {
            let entry_index = if mode == 5 && index == 0 {
                1
            } else {
                index as u32
            };
            let chunk_index = if mode == 6 && index == 0 {
                chunk as u32 + 1
            } else {
                chunk as u32
            };
            let finality = (chunk + 1 == chunks.len()) ^ (mode == 7 && index == 0);
            encoder.frame(entry_index, chunk_index, bytes, finality);
        }
    }
    let mut trailer = Vec::with_capacity(52);
    let mut digest: [u8; 32] = Sha256::digest(manifest).into();
    if mode == 1 {
        digest[0] ^= 1;
    }
    trailer.extend_from_slice(&digest);
    trailer.extend_from_slice(&(count as u32 + u32::from(mode == 2)).to_be_bytes());
    trailer.extend_from_slice(&(encoder.counter + 1 + u64::from(mode == 3)).to_be_bytes());
    let total: u64 = entries.iter().map(|entry| entry.len() as u64).sum();
    trailer.extend_from_slice(&(total + u64::from(mode == 4)).to_be_bytes());
    if mode != 8 {
        encoder.frame(u32::MAX - 1, 0, &trailer, true);
    }
    if mode == 9 {
        encoder.bytes.push(0);
    }
    if mode == 10 {
        encoder.bytes.pop();
    }
    let key = key();
    let mut reader = ArchiveReader::new(encoder.bytes.as_slice(), &key).unwrap();
    assert_eq!(reader.manifest(), manifest);
    if mode == 12 {
        // Finishing before reading the declared entries must not accept a
        // correctly authenticated but premature trailer position.
        assert!(reader.finish().is_err());
        return;
    }
    for (index, entry) in entries.iter().enumerate() {
        let result = if mode == 11 && index + 1 == count && !entry.is_empty() {
            reader.read_entry(entry.len() as u64, &mut FailedSink)
        } else {
            let mut actual = Vec::new();
            let result = reader.read_entry(entry.len() as u64, &mut actual);
            if result.is_ok() {
                assert_eq!(actual, *entry);
            }
            result
        };
        let malformed_entry = (5..=7).contains(&mode) && index == 0;
        let failed_sink = mode == 11 && index + 1 == count && !entry.is_empty();
        if malformed_entry || failed_sink {
            assert!(result.is_err());
            assert!(reader.read_entry(0, &mut io::sink()).is_err());
            assert!(reader.finish().is_err());
            return;
        }
        result.unwrap();
    }
    let result = reader.finish();
    if mode == 0 || mode == 11 {
        assert!(result.is_ok());
    } else {
        assert!(result.is_err());
    }
}

struct FailedSink;
impl Write for FailedSink {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("synthetic failed sink"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
