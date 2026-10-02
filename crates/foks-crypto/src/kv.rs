//! KV key derivation, authenticated names, files, chunks, and metadata.

use super::*;
pub(super) const SMALL_FILE_PAYLOAD_TYPE_ID: u64 = 0xaeec_688f_3145_fddf;
pub(super) const DIR_KEY_SEED_TYPE_ID: u64 = 0x8aec_e656_6b24_4356;
// Rust extension stored in KvLargeFileMetadata.custom_metadata. Servers treat
// this field as opaque; the encrypted payload carries its own format version.
pub(super) const LARGE_FILE_SIZE_PAYLOAD_TYPE_ID: u64 = 0xb752_f13b_2ad6_7234;

/// Application-specific MAC and box keys derived from one exact PUK/PTK seed.
pub struct KvKeySet {
    mac: Zeroizing<[u8; 32]>,
    pub(super) box_key: Zeroizing<[u8; 32]>,
}

/// Derives the v0.1.9 KV application key and its MAC/secretbox subkeys.
pub fn derive_kv_keys(shared_key_seed: &SecretSeed) -> Result<KvKeySet> {
    let app_key = derive_key(shared_key_seed, 5, None)?;
    let kv_app = typed_hmac(
        app_key.as_slice(),
        APP_KEY_DERIVATION_TYPE_ID,
        &encode(&Value::Array(vec![
            Value::Unsigned(0),
            Value::Variant(Some((b"0".to_vec(), Box::new(Value::Unsigned(0))))),
        ]))?,
    );
    let derivation = |kind| {
        typed_hmac(
            &kv_app,
            KV_KEY_DERIVATION_TYPE_ID,
            &encode(&Value::Array(vec![
                Value::Unsigned(kind),
                Value::Variant(None),
            ]))
            .expect("fixed KV derivation is canonical"),
        )
    };
    Ok(KvKeySet {
        mac: Zeroizing::new(derivation(1)),
        box_key: Zeroizing::new(derivation(2)),
    })
}

impl KvKeySet {
    pub fn bind_root(
        &self,
        party: &KvParty,
        root: [u8; 16],
        version: u64,
        key: RoleAndGeneration,
    ) -> Result<[u8; 32]> {
        let payload = encode(&Value::Array(vec![
            party.to_value(),
            key.to_value(),
            Value::Binary(root.to_vec()),
            Value::Unsigned(version),
        ]))?;
        Ok(typed_hmac(
            self.mac.as_slice(),
            KV_ROOT_BINDING_PAYLOAD_TYPE_ID,
            &payload,
        ))
    }

    pub fn verify_root(&self, root: &KvRoot, party: &KvParty) -> Result<()> {
        let payload = root.binding_payload(party)?;
        verify_mac(
            self.mac.as_slice(),
            KV_ROOT_BINDING_PAYLOAD_TYPE_ID,
            &payload,
            &root.binding_mac,
        )
    }

    pub fn open_directory_seed(&self, directory: &KvDirectory) -> Result<SecretSeed> {
        let plaintext = open_typed_secretbox(
            &self.box_key,
            DIR_KEY_SEED_TYPE_ID,
            &directory.id,
            &directory.seed_ciphertext,
        )?;
        if plaintext.len() < 34 || plaintext[..2] != [0xc4, 32] {
            return Err(Error::KvBinding);
        }
        require_zero_padding(&plaintext, 34)?;
        Ok(SecretSeed::from_slice(&plaintext[2..34])?)
    }

    pub fn open_small_file(
        &self,
        id: KvNodeId,
        boxed: &KvSmallFileBox,
    ) -> Result<KvSmallFilePlaintext> {
        if boxed.key.generation == 0 {
            return Err(Error::KvKeyMismatch);
        }
        let object_id = id.object_id();
        let plaintext = open_typed_secretbox(
            &self.box_key,
            SMALL_FILE_PAYLOAD_TYPE_ID,
            &object_id,
            &boxed.ciphertext,
        )?;
        let (value, consumed) = decode_prefix(&plaintext)?;
        require_zero_padding(&plaintext, consumed)?;
        KvSmallFilePlaintext::decode_value(value).map_err(Into::into)
    }

    pub fn open_file_seed(
        &self,
        id: KvNodeId,
        metadata: &KvLargeFileMetadata,
    ) -> Result<SecretSeed> {
        let plaintext = open_typed_secretbox(
            &self.box_key,
            KV_FILE_KEY_PAYLOAD_TYPE_ID,
            &metadata.key_seed.nonce,
            &metadata.key_seed.ciphertext,
        )?;
        let (value, seed) = decode_with_redacted_trailing_seed(&plaintext)?;
        let fields = match value {
            Value::Array(fields) if fields.len() == 3 => fields,
            _ => return Err(Error::KvBinding),
        };
        if fields[0] != Value::Binary(id.object_id().to_vec())
            || fields[1] != Value::Unsigned(metadata.version)
        {
            return Err(Error::KvBinding);
        }
        let Value::Binary(redacted) = &fields[2] else {
            return Err(Error::KvBinding);
        };
        if redacted.as_slice() != [0; 32] {
            return Err(Error::KvBinding);
        }
        Ok(seed)
    }

    /// Opens the Rust large-file size extension.
    ///
    /// `Ok(None)` means that the metadata is absent or uses a newer,
    /// authenticated payload version. Authentication and identity-binding
    /// failures remain errors.
    pub fn open_large_file_size(
        &self,
        id: KvNodeId,
        metadata: &KvLargeFileMetadata,
    ) -> Result<Option<u64>> {
        let Some(boxed) = &metadata.custom_metadata else {
            return Ok(None);
        };
        let plaintext = open_typed_secretbox(
            &self.box_key,
            LARGE_FILE_SIZE_PAYLOAD_TYPE_ID,
            &boxed.nonce,
            &boxed.ciphertext,
        )?;
        let (value, consumed) = decode_prefix(&plaintext)?;
        require_zero_padding(&plaintext, consumed)?;
        let Value::Array(fields) = value else {
            return Err(Error::KvBinding);
        };
        let Some(Value::Unsigned(format_version)) = fields.first() else {
            return Err(Error::KvBinding);
        };
        if *format_version != 1 {
            return Ok(None);
        }
        if fields.len() != 4
            || fields[1] != Value::Binary(id.object_id().to_vec())
            || fields[2] != Value::Unsigned(metadata.version)
        {
            return Err(Error::KvBinding);
        }
        let Value::Unsigned(size) = fields[3] else {
            return Err(Error::KvBinding);
        };
        Ok(Some(size))
    }

    pub fn seal_large_file_size(
        &self,
        id: KvNodeId,
        metadata_version: u64,
        size: u64,
        nonce: [u8; 16],
    ) -> Result<SecretBox> {
        let object_id = id.object_id();
        let plaintext = Zeroizing::new(encode_ref(&ValueRef::Array(vec![
            ValueRef::Unsigned(1),
            ValueRef::Binary(&object_id),
            ValueRef::Unsigned(metadata_version),
            ValueRef::Unsigned(size),
        ]))?);
        Ok(SecretBox {
            nonce,
            ciphertext: seal_typed_secretbox(
                &self.box_key,
                LARGE_FILE_SIZE_PAYLOAD_TYPE_ID,
                &nonce,
                &plaintext,
                false,
            )?,
        })
    }

    pub fn seal_directory_seed(
        &self,
        directory_id: [u8; 16],
        seed: &SecretSeed,
    ) -> Result<Vec<u8>> {
        let plaintext = Zeroizing::new(encode_ref(&ValueRef::Binary(seed.as_slice()))?);
        seal_typed_secretbox(
            &self.box_key,
            DIR_KEY_SEED_TYPE_ID,
            &directory_id,
            &plaintext,
            false,
        )
    }

    /// Seals a v0.1.9 small-file or symlink payload.
    ///
    /// `id.object_id()` supplies the deterministic Secretbox nonce suffix and
    /// must be freshly generated for every distinct plaintext under this key.
    /// Callers constructing IDs directly must ensure object ID uniqueness
    /// across file and symlink variants.
    pub fn seal_small_file(
        &self,
        id: KvNodeId,
        key: RoleAndGeneration,
        plaintext: KvSmallFilePlaintext,
    ) -> Result<KvSmallFileBox> {
        let plaintext = match &plaintext {
            KvSmallFilePlaintext::File(bytes) => ValueRef::Array(vec![
                ValueRef::Unsigned(3),
                ValueRef::Variant(Some((b"0", Box::new(ValueRef::Binary(bytes))))),
            ]),
            KvSmallFilePlaintext::Symlink(path) => ValueRef::Array(vec![
                ValueRef::Unsigned(4),
                ValueRef::Variant(Some((b"1", Box::new(ValueRef::Text(path))))),
            ]),
        };
        let plaintext = Zeroizing::new(encode_ref(&plaintext)?);
        Ok(KvSmallFileBox {
            key,
            ciphertext: seal_typed_secretbox(
                &self.box_key,
                SMALL_FILE_PAYLOAD_TYPE_ID,
                &id.object_id(),
                &plaintext,
                true,
            )?,
        })
    }

    pub fn seal_file_seed(
        &self,
        id: KvNodeId,
        key: RoleAndGeneration,
        version: u64,
        file_seed: &SecretSeed,
        nonce: [u8; 16],
    ) -> Result<KvLargeFileMetadata> {
        let object_id = id.object_id();
        let plaintext = Zeroizing::new(encode_ref(&ValueRef::Array(vec![
            ValueRef::Binary(&object_id),
            ValueRef::Unsigned(version),
            ValueRef::Binary(file_seed.as_slice()),
        ]))?);
        Ok(KvLargeFileMetadata {
            key,
            key_seed: SecretBox {
                nonce,
                ciphertext: seal_typed_secretbox(
                    &self.box_key,
                    KV_FILE_KEY_PAYLOAD_TYPE_ID,
                    &nonce,
                    &plaintext,
                    false,
                )?,
            },
            version,
            custom_metadata: None,
        })
    }
}

pub fn seal_kv_dirent_name(
    directory_seed: &SecretSeed,
    parent: [u8; 16],
    directory_version: u64,
    name: Vec<u8>,
    nonce: [u8; 16],
) -> Result<([u8; 32], SecretBox)> {
    let keys = derive_seed_kv_keys(directory_seed)?;
    let payload = Zeroizing::new(encode_ref(&ValueRef::Array(vec![
        ValueRef::Binary(&parent),
        ValueRef::Unsigned(directory_version),
        ValueRef::Text(&name),
    ]))?);
    let mac = typed_hmac(
        keys.mac.as_slice(),
        KV_DIRENT_NAME_PAYLOAD_TYPE_ID,
        &payload,
    );
    let ciphertext = seal_typed_secretbox(
        &keys.box_key,
        KV_DIRENT_NAME_PAYLOAD_TYPE_ID,
        &nonce,
        &payload,
        true,
    )?;
    Ok((mac, SecretBox { nonce, ciphertext }))
}

pub fn bind_kv_dirent(directory_seed: &SecretSeed, dirent: &KvDirent) -> Result<[u8; 32]> {
    let keys = derive_seed_kv_keys(directory_seed)?;
    Ok(typed_hmac(
        keys.mac.as_slice(),
        KV_DIRENT_BINDING_PAYLOAD_TYPE_ID,
        &dirent.binding_payload()?,
    ))
}

pub fn seal_kv_chunk(
    file_seed: &SecretSeed,
    file_id: KvNodeId,
    offset: u64,
    final_chunk: bool,
    cleartext: &[u8],
    encrypted_size_before: u64,
) -> Result<KvUploadChunk> {
    let encoded = Zeroizing::new(encode_ref(&ValueRef::Binary(cleartext))?);
    let padded_length = kv_chunk_padded_length(encoded.len())?;
    let mut padded = Zeroizing::new(vec![0; padded_length]);
    padded[..encoded.len()].copy_from_slice(&encoded);
    let nonce_value = encode(&Value::Array(vec![
        Value::Binary(file_id.object_id().to_vec()),
        Value::Unsigned(offset),
        Value::Bool(final_chunk),
    ]))?;
    let hash = prefixed_hash_signable(KV_CHUNK_NONCE_PAYLOAD_TYPE_ID, &nonce_value)?;
    let nonce: [u8; 24] = hash[..24].try_into().expect("slice length is fixed");
    let cipher = XSalsa20Poly1305::new(file_seed.as_bytes().into());
    let ciphertext = cipher
        .encrypt((&nonce).into(), padded.as_ref())
        .map_err(|_| Error::KvEncryption)?;
    let encrypted_size = encrypted_size_before
        .checked_add(u64::try_from(ciphertext.len()).map_err(|_| Error::KvPadding)?)
        .ok_or(Error::KvPadding)?;
    Ok(KvUploadChunk {
        ciphertext,
        offset,
        final_upload: final_chunk.then_some(KvUploadFinal {
            size: encrypted_size,
            chunk_sum: [0; 32],
        }),
    })
}

/// Verifies and decrypts one directory entry name under its directory seed.
pub fn open_kv_dirent_name(directory_seed: &SecretSeed, dirent: &KvDirent) -> Result<KvDirentName> {
    let keys = derive_seed_kv_keys(directory_seed)?;
    let plaintext = open_typed_secretbox(
        &keys.box_key,
        KV_DIRENT_NAME_PAYLOAD_TYPE_ID,
        &dirent.name_box.nonce,
        &dirent.name_box.ciphertext,
    )?;
    let (value, consumed) = decode_prefix(&plaintext)?;
    require_zero_padding(&plaintext, consumed)?;
    let canonical = encode(&value)?;
    verify_mac(
        keys.mac.as_slice(),
        KV_DIRENT_NAME_PAYLOAD_TYPE_ID,
        &canonical,
        &dirent.name_mac,
    )?;
    verify_mac(
        keys.mac.as_slice(),
        KV_DIRENT_BINDING_PAYLOAD_TYPE_ID,
        &dirent.binding_payload()?,
        &dirent.binding_mac,
    )?;
    let name = KvDirentName::decode_value(value)?;
    if name.parent != dirent.parent || name.directory_version != dirent.directory_version {
        return Err(Error::KvBinding);
    }
    Ok(name)
}

/// Opens the large-file chunk at `requested_offset`.
///
/// Verifies the decrypted chunk offset against `requested_offset` before
/// returning plaintext to prevent chunk substitution by a malicious peer.
pub fn open_kv_chunk(
    file_seed: &SecretSeed,
    file_id: KvNodeId,
    requested_offset: u64,
    chunk: &KvEncryptedChunk,
) -> Result<Vec<u8>> {
    if chunk.offset != requested_offset {
        return Err(Error::KvBinding);
    }
    let nonce_value = encode(&Value::Array(vec![
        Value::Binary(file_id.object_id().to_vec()),
        Value::Unsigned(requested_offset),
        Value::Bool(chunk.final_chunk),
    ]))?;
    let hash = prefixed_hash_signable(KV_CHUNK_NONCE_PAYLOAD_TYPE_ID, &nonce_value)?;
    let nonce: [u8; 24] = hash[..24].try_into().expect("slice length is fixed");
    let cipher = XSalsa20Poly1305::new(file_seed.as_bytes().into());
    let plaintext = Zeroizing::new(
        cipher
            .decrypt((&nonce).into(), chunk.ciphertext.as_ref())
            .map_err(|_| Error::Decryption)?,
    );
    let (value, consumed) = decode_prefix(&plaintext)?;
    require_zero_padding(&plaintext, consumed)?;
    let Value::Binary(bytes) = value else {
        return Err(Error::KvBinding);
    };
    Ok(bytes)
}

pub(super) fn derive_seed_kv_keys(seed: &SecretSeed) -> Result<KvKeySet> {
    let derivation = |kind| {
        typed_hmac(
            seed.as_slice(),
            KV_KEY_DERIVATION_TYPE_ID,
            &encode(&Value::Array(vec![
                Value::Unsigned(kind),
                Value::Variant(None),
            ]))
            .expect("fixed KV derivation is canonical"),
        )
    };
    Ok(KvKeySet {
        mac: Zeroizing::new(derivation(1)),
        box_key: Zeroizing::new(derivation(2)),
    })
}

pub(super) fn kv_chunk_padded_length(length: usize) -> Result<usize> {
    if length > u32::MAX as usize {
        return Err(Error::KvPadding);
    }
    let mut base = 32usize;
    let mut overhead = 2usize;
    loop {
        if base >= 0x1_0000 {
            overhead = 5;
        } else if base >= 0x100 {
            overhead = 3;
        }
        let total = base.checked_add(overhead).ok_or(Error::KvPadding)?;
        if length <= total {
            return Ok(total);
        }
        base = base.checked_mul(2).ok_or(Error::KvPadding)?;
    }
}

pub(super) fn decode_with_redacted_trailing_seed(plaintext: &[u8]) -> Result<(Value, SecretSeed)> {
    const ENCODED_SEED_LENGTH: usize = 34;
    if plaintext.len() < ENCODED_SEED_LENGTH
        || plaintext[plaintext.len() - ENCODED_SEED_LENGTH..plaintext.len() - 32] != [0xc4, 32]
    {
        return Err(Error::KvBinding);
    }
    let seed_offset = plaintext.len() - 32;
    let mut redacted = Zeroizing::new(plaintext.to_vec());
    let seed = SecretSeed::from_slice(&redacted[seed_offset..])?;
    redacted[seed_offset..].fill(0);
    Ok((decode(&redacted)?, seed))
}

/// Fingerprint of bounded plaintext for a local adapter's durable upload intent.
/// The domain separator is local and never emitted on the FOKS wire.
pub fn kv_adapter_body_hash(body: &[u8]) -> [u8; 32] {
    prefixed_hash(0x5de4_c49e_cabc_9643, body)
}
