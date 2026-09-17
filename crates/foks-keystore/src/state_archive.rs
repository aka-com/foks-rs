//! Bounded authenticated state-transfer framing, independent of application policy.
use crate::{Error, Result};
use chacha20poly1305::{
    aead::{Aead as _, KeyInit as _, Payload},
    XChaCha20Poly1305, XNonce,
};
use sha2::{Digest as _, Sha256};
use std::io::{Read, Write};
use zeroize::Zeroizing;

pub const KEY_PREFIX: &str = "FOKS-STATE-TRANSFER-V1:";
pub const MAX_MANIFEST: usize = 8 * 1024 * 1024;
pub const MAX_ENTRIES: u32 = 8192;
pub const CHUNK_BYTES: usize = 1024 * 1024;
pub const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
const HEADER_BYTES: usize = 73;
const FRAME_BYTES: usize = 21;
const TAG_BYTES: usize = 16;
const MAGIC: &[u8; 8] = b"FOKSST01";
const MANIFEST_INDEX: u32 = u32::MAX;
const TRAILER_INDEX: u32 = u32::MAX - 1;
const TRAILER_BYTES: usize = 52;
const KDF_DOMAIN: &[u8] = b"foks-state-transfer-key-v1\0";

/// A generated random key, never a password or account backup phrase.
pub struct StateTransferKey(Zeroizing<[u8; 32]>);
impl std::fmt::Debug for StateTransferKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StateTransferKey([REDACTED])")
    }
}
impl StateTransferKey {
    pub fn generate() -> Result<Self> {
        let mut bytes = Zeroizing::new([0; 32]);
        getrandom::fill(&mut *bytes).map_err(|_| Error::Randomness)?;
        Ok(Self(bytes))
    }
    pub fn parse(input: &str) -> Result<Self> {
        let hex = input.strip_prefix(KEY_PREFIX).ok_or(Error::InvalidFormat)?;
        if hex.len() != 64 {
            return Err(Error::InvalidFormat);
        }
        let mut bytes = Zeroizing::new([0; 32]);
        fn nibble(c: u8) -> Result<u8> {
            match c {
                b'0'..=b'9' => Ok(c - b'0'),
                b'a'..=b'f' => Ok(c - b'a' + 10),
                _ => Err(Error::InvalidFormat),
            }
        }
        for (out, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
            *out = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Ok(Self(bytes))
    }
    /// Only for a protected native output boundary. The returned text zeroizes.
    pub fn expose_encoding(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = Zeroizing::new(String::with_capacity(KEY_PREFIX.len() + 64));
        out.push_str(KEY_PREFIX);
        for byte in self.0.iter() {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 15) as usize] as char);
        }
        out
    }
}

struct Framing {
    header: [u8; HEADER_BYTES],
    cipher: XChaCha20Poly1305,
    counter: u64,
}
impl Framing {
    fn new(key: &StateTransferKey, header: [u8; HEADER_BYTES]) -> Result<Self> {
        if &header[..8] != MAGIC || header[8] != 1 {
            return Err(Error::InvalidFormat);
        }
        let mut hash = Sha256::new();
        hash.update(KDF_DOMAIN);
        hash.update(key.0.as_slice());
        hash.update(&header[9..41]);
        hash.update(&header[41..57]);
        let derived = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
        let cipher = XChaCha20Poly1305::new_from_slice(derived.as_slice())
            .map_err(|_| Error::InvalidFormat)?;
        Ok(Self {
            header,
            cipher,
            counter: 0,
        })
    }
    fn nonce(&self) -> [u8; 24] {
        let mut nonce = [0; 24];
        nonce[..16].copy_from_slice(&self.header[57..]);
        nonce[16..].copy_from_slice(&self.counter.to_be_bytes());
        nonce
    }
    fn fields(&self, entry: u32, chunk: u32, length: u32, finality: bool) -> [u8; FRAME_BYTES] {
        let mut fields = [0; FRAME_BYTES];
        fields[..8].copy_from_slice(&self.counter.to_be_bytes());
        fields[8..12].copy_from_slice(&entry.to_be_bytes());
        fields[12..16].copy_from_slice(&chunk.to_be_bytes());
        fields[16..20].copy_from_slice(&length.to_be_bytes());
        fields[20] = u8::from(finality);
        fields
    }
    fn aad(&self, fields: &[u8; FRAME_BYTES]) -> [u8; HEADER_BYTES + FRAME_BYTES] {
        let mut aad = [0; HEADER_BYTES + FRAME_BYTES];
        aad[..HEADER_BYTES].copy_from_slice(&self.header);
        aad[HEADER_BYTES..].copy_from_slice(fields);
        aad
    }
    fn write(
        &mut self,
        output: &mut impl Write,
        entry: u32,
        chunk: u32,
        bytes: &[u8],
        finality: bool,
    ) -> Result<()> {
        let next = self.counter.checked_add(1).ok_or(Error::TooLarge)?;
        let length = u32::try_from(bytes.len()).map_err(|_| Error::TooLarge)?;
        let fields = self.fields(entry, chunk, length, finality);
        let ciphertext = self
            .cipher
            .encrypt(
                &XNonce::from(self.nonce()),
                Payload {
                    msg: bytes,
                    aad: &self.aad(&fields),
                },
            )
            .map_err(|_| Error::Authentication)?;
        output.write_all(&fields)?;
        output.write_all(&ciphertext)?;
        self.counter = next;
        Ok(())
    }
    fn read(
        &mut self,
        input: &mut impl Read,
        entry: u32,
        chunk: u32,
        length: Option<usize>,
        maximum: usize,
        finality: bool,
    ) -> Result<Zeroizing<Vec<u8>>> {
        let next = self.counter.checked_add(1).ok_or(Error::TooLarge)?;
        let mut fields = [0; FRAME_BYTES];
        input.read_exact(&mut fields)?;
        let declared = u32::from_be_bytes(
            fields[16..20]
                .try_into()
                .map_err(|_| Error::InvalidFormat)?,
        ) as usize;
        if declared > maximum
            || length.is_some_and(|expected| expected != declared)
            || fields != self.fields(entry, chunk, declared as u32, finality)
        {
            return Err(Error::InvalidFormat);
        }
        let mut ciphertext = vec![0; declared.checked_add(TAG_BYTES).ok_or(Error::TooLarge)?];
        input.read_exact(&mut ciphertext)?;
        let plaintext = Zeroizing::new(
            self.cipher
                .decrypt(
                    &XNonce::from(self.nonce()),
                    Payload {
                        msg: &ciphertext,
                        aad: &self.aad(&fields),
                    },
                )
                .map_err(|_| Error::Authentication)?,
        );
        if plaintext.len() != declared {
            return Err(Error::InvalidFormat);
        }
        self.counter = next;
        Ok(plaintext)
    }
}

pub struct ArchiveWriter<W: Write> {
    output: W,
    framing: Framing,
    manifest_digest: [u8; 32],
    entries: u32,
    total: u64,
    failed: bool,
}
impl<W: Write> ArchiveWriter<W> {
    pub fn new(output: W, key: &StateTransferKey, manifest: &[u8]) -> Result<Self> {
        let mut header = [0; HEADER_BYTES];
        header[..8].copy_from_slice(MAGIC);
        header[8] = 1;
        getrandom::fill(&mut header[9..]).map_err(|_| Error::Randomness)?;
        Self::with_header(output, key, manifest, header)
    }
    fn with_header(
        mut output: W,
        key: &StateTransferKey,
        manifest: &[u8],
        header: [u8; HEADER_BYTES],
    ) -> Result<Self> {
        if manifest.is_empty() || manifest.len() > MAX_MANIFEST {
            return Err(Error::TooLarge);
        }
        let mut framing = Framing::new(key, header)?;
        output.write_all(&header)?;
        framing.write(&mut output, MANIFEST_INDEX, 0, manifest, true)?;
        Ok(Self {
            output,
            framing,
            manifest_digest: Sha256::digest(manifest).into(),
            entries: 0,
            total: 0,
            failed: false,
        })
    }
    pub fn archive_id(&self) -> [u8; 16] {
        {
            let mut id = [0; 16];
            id.copy_from_slice(&self.framing.header[41..57]);
            id
        }
    }
    /// Exactly one ordered entry. Any failure poisons this writer permanently.
    pub fn write_entry(&mut self, length: u64, input: &mut impl Read) -> Result<()> {
        if self.failed {
            return Err(Error::InvalidFormat);
        }
        self.failed = true;
        let total = check_entry(self.entries, self.total, length)?;
        let mut remaining = length;
        let mut chunk = 0;
        let mut bytes = Zeroizing::new(vec![0; CHUNK_BYTES]);
        loop {
            let n = remaining.min(CHUNK_BYTES as u64) as usize;
            input.read_exact(&mut bytes[..n])?;
            remaining -= n as u64;
            self.framing.write(
                &mut self.output,
                self.entries,
                chunk,
                &bytes[..n],
                remaining == 0,
            )?;
            if remaining == 0 {
                break;
            }
            chunk = chunk.checked_add(1).ok_or(Error::TooLarge)?;
        }
        let mut extra = Zeroizing::new([0; 1]);
        if input.read(&mut *extra)? != 0 {
            return Err(Error::InvalidFormat);
        }
        self.entries += 1;
        self.total = total;
        self.failed = false;
        Ok(())
    }
    pub fn finish(mut self) -> Result<W> {
        if self.failed {
            return Err(Error::InvalidFormat);
        }
        let frames = self.framing.counter.checked_add(1).ok_or(Error::TooLarge)?;
        let trailer = trailer(self.manifest_digest, self.entries, frames, self.total);
        self.framing
            .write(&mut self.output, TRAILER_INDEX, 0, &trailer, true)?;
        self.output.flush()?;
        Ok(self.output)
    }
}
fn check_entry(entries: u32, total: u64, length: u64) -> Result<u64> {
    let total = total.checked_add(length).ok_or(Error::TooLarge)?;
    if entries >= MAX_ENTRIES || length > MAX_FILE || total > MAX_TOTAL {
        Err(Error::TooLarge)
    } else {
        Ok(total)
    }
}
fn trailer(digest: [u8; 32], entries: u32, frames: u64, total: u64) -> [u8; TRAILER_BYTES] {
    let mut out = [0; TRAILER_BYTES];
    out[..32].copy_from_slice(&digest);
    out[32..36].copy_from_slice(&entries.to_be_bytes());
    out[36..44].copy_from_slice(&frames.to_be_bytes());
    out[44..].copy_from_slice(&total.to_be_bytes());
    out
}

pub struct ArchiveReader<R: Read> {
    input: R,
    framing: Framing,
    manifest: Zeroizing<Vec<u8>>,
    manifest_digest: [u8; 32],
    entries: u32,
    total: u64,
    failed: bool,
}
impl<R: Read> ArchiveReader<R> {
    pub fn new(mut input: R, key: &StateTransferKey) -> Result<Self> {
        let mut header = [0; HEADER_BYTES];
        input.read_exact(&mut header)?;
        let mut framing = Framing::new(key, header)?;
        let manifest = framing.read(&mut input, MANIFEST_INDEX, 0, None, MAX_MANIFEST, true)?;
        if manifest.is_empty() {
            return Err(Error::InvalidFormat);
        }
        let manifest_digest = Sha256::digest(manifest.as_slice()).into();
        Ok(Self {
            input,
            framing,
            manifest,
            manifest_digest,
            entries: 0,
            total: 0,
            failed: false,
        })
    }
    pub fn manifest(&self) -> &[u8] {
        &self.manifest
    }
    pub fn archive_id(&self) -> [u8; 16] {
        {
            let mut id = [0; 16];
            id.copy_from_slice(&self.framing.header[41..57]);
            id
        }
    }
    /// Caller supplies the authenticated manifest's next length and a private sink.
    /// Plaintext is released only after that chunk's tag verifies; finish is still
    /// mandatory before publishing or otherwise trusting the complete snapshot.
    pub fn read_entry(&mut self, length: u64, output: &mut impl Write) -> Result<()> {
        if self.failed {
            return Err(Error::InvalidFormat);
        }
        self.failed = true;
        let total = check_entry(self.entries, self.total, length)?;
        let mut remaining = length;
        let mut chunk = 0;
        loop {
            let n = remaining.min(CHUNK_BYTES as u64) as usize;
            remaining -= n as u64;
            let bytes = self.framing.read(
                &mut self.input,
                self.entries,
                chunk,
                Some(n),
                CHUNK_BYTES,
                remaining == 0,
            )?;
            output.write_all(&bytes)?;
            if remaining == 0 {
                break;
            }
            chunk = chunk.checked_add(1).ok_or(Error::TooLarge)?;
        }
        self.entries += 1;
        self.total = total;
        self.failed = false;
        Ok(())
    }
    pub fn finish(mut self) -> Result<R> {
        if self.failed {
            return Err(Error::InvalidFormat);
        }
        let frames = self.framing.counter.checked_add(1).ok_or(Error::TooLarge)?;
        let expected = trailer(self.manifest_digest, self.entries, frames, self.total);
        let actual = self.framing.read(
            &mut self.input,
            TRAILER_INDEX,
            0,
            Some(TRAILER_BYTES),
            TRAILER_BYTES,
            true,
        )?;
        if actual.as_slice() != expected {
            return Err(Error::InvalidFormat);
        }
        let mut extra = [0; 1];
        if self.input.read(&mut extra)? != 0 {
            return Err(Error::InvalidFormat);
        }
        Ok(self.input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> StateTransferKey {
        StateTransferKey(Zeroizing::new([7; 32]))
    }
    fn vector() -> Vec<u8> {
        let mut header = [0; HEADER_BYTES];
        header[..8].copy_from_slice(MAGIC);
        header[8] = 1;
        for (i, byte) in header[9..].iter_mut().enumerate() {
            *byte = i as u8;
        }
        let mut writer =
            ArchiveWriter::with_header(Vec::new(), &key(), b"{\"v\":1}", header).unwrap();
        writer.write_entry(3, &mut &b"abc"[..]).unwrap();
        writer.write_entry(0, &mut &b""[..]).unwrap();
        writer.finish().unwrap()
    }
    fn accept(bytes: &[u8], key: &StateTransferKey) -> Result<()> {
        let mut reader = ArchiveReader::new(bytes, key)?;
        if reader.manifest() != b"{\"v\":1}" {
            return Err(Error::InvalidFormat);
        }
        let mut output = Vec::new();
        reader.read_entry(3, &mut output)?;
        if output != b"abc" {
            return Err(Error::InvalidFormat);
        }
        reader.read_entry(0, &mut std::io::sink())?;
        reader.finish()?;
        Ok(())
    }
    #[test]
    fn deterministic_vector_and_empty_entry_round_trip() {
        let bytes = vector();
        accept(&bytes, &key()).unwrap();
        let encoded = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            encoded,
            include_str!("../tests/fixtures/state-archive-v1.hex").trim()
        );
        let hash = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(
            hash,
            "d17738b58962be91ac73ad0ea48b94677387051637be54f30a705e270121669a"
        );
    }
    #[test]
    fn every_truncation_and_byte_tamper_fails() {
        let bytes = vector();
        for end in 0..bytes.len() {
            assert!(accept(&bytes[..end], &key()).is_err(), "truncation {end}");
        }
        for position in 0..bytes.len() {
            let mut changed = bytes.clone();
            changed[position] ^= 1;
            assert!(accept(&changed, &key()).is_err(), "tamper {position}");
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(accept(&extra, &key()).is_err());
        assert!(accept(&bytes, &StateTransferKey(Zeroizing::new([8; 32]))).is_err());
    }
    #[test]
    fn exact_key_domain_and_redacted_debug() {
        let key = StateTransferKey::generate().unwrap();
        let encoded = key.expose_encoding();
        assert_eq!(
            StateTransferKey::parse(&encoded).unwrap().expose_encoding(),
            encoded
        );
        assert!(!format!("{key:?}").contains(&encoded[KEY_PREFIX.len()..]));
        for text in [
            "00".repeat(32),
            format!("{}{}", KEY_PREFIX, "A".repeat(64)),
            format!("{}\n", encoded.as_str()),
            format!("{}0", KEY_PREFIX),
        ] {
            assert!(StateTransferKey::parse(&text).is_err());
        }
    }
    #[test]
    fn failed_streams_cannot_be_finished_or_reused_and_limits_precede_reads() {
        let mut writer = ArchiveWriter::new(Vec::new(), &key(), b"{}").unwrap();
        assert!(writer.write_entry(1, &mut &b"ab"[..]).is_err());
        assert!(writer.write_entry(0, &mut &b""[..]).is_err());
        assert!(writer.finish().is_err());
        let mut writer = ArchiveWriter::new(Vec::new(), &key(), b"{}").unwrap();
        assert!(writer
            .write_entry(MAX_FILE + 1, &mut std::io::empty())
            .is_err());
        let bytes = vector();
        let mut reader = ArchiveReader::new(bytes.as_slice(), &key()).unwrap();
        assert!(reader.read_entry(4, &mut Vec::new()).is_err());
        assert!(reader.finish().is_err());
        assert!(check_entry(MAX_ENTRIES, 0, 0).is_err());
        assert!(check_entry(0, MAX_TOTAL, 1).is_err());
    }
    #[test]
    fn chunk_order_duplication_and_manifest_allocation_bounds_are_checked() {
        let bytes = vector();
        let manifest_end = HEADER_BYTES + FRAME_BYTES + 7 + TAG_BYTES;
        let first_end = manifest_end + FRAME_BYTES + 3 + TAG_BYTES;
        let second_end = first_end + FRAME_BYTES + TAG_BYTES;
        let mut reordered = bytes[..manifest_end].to_vec();
        reordered.extend_from_slice(&bytes[first_end..second_end]);
        reordered.extend_from_slice(&bytes[manifest_end..first_end]);
        reordered.extend_from_slice(&bytes[second_end..]);
        assert!(accept(&reordered, &key()).is_err());
        let mut duplicated = bytes[..first_end].to_vec();
        duplicated.extend_from_slice(&bytes[manifest_end..]);
        assert!(accept(&duplicated, &key()).is_err());
        let mut oversized = bytes.clone();
        oversized[HEADER_BYTES + 16..HEADER_BYTES + 20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(ArchiveReader::new(oversized.as_slice(), &key()).is_err());
        let mut writer = ArchiveWriter::new(Vec::new(), &key(), b"{}").unwrap();
        writer.framing.counter = u64::MAX;
        assert!(writer.write_entry(0, &mut std::io::empty()).is_err());
    }
    #[test]
    fn multiple_chunks_round_trip_with_bounded_buffers() {
        let bytes = vec![0xa5; CHUNK_BYTES + 1];
        let mut writer = ArchiveWriter::new(Vec::new(), &key(), b"{}").unwrap();
        writer
            .write_entry(bytes.len() as u64, &mut bytes.as_slice())
            .unwrap();
        let archive = writer.finish().unwrap();
        let mut reader = ArchiveReader::new(archive.as_slice(), &key()).unwrap();
        let mut output = Vec::new();
        reader.read_entry(bytes.len() as u64, &mut output).unwrap();
        reader.finish().unwrap();
        assert_eq!(bytes, output);
    }
}
