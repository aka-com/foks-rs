#![forbid(unsafe_code)]

mod hard_state;

use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use crypto_secretbox::{aead::Aead as _, KeyInit as _, XSalsa20Poly1305};
use foks_snowpack::{decode, Value};
use sha2::{Digest as _, Sha256};
use zeroize::{Zeroize as _, Zeroizing};

const MAXIMUM_SECRET_STORE_BYTES: u64 = 16 * 1024 * 1024;
const MAXIMUM_CANDIDATES: usize = 256;
const MAXIMUM_TEXT_BYTES: usize = 4096;
const SECRET_STORE_NAME: &str = "foks-secrets";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SECRET_KEY_BUNDLE_TYPE_ID: u64 = 0x8456_933b_bb8a_54ae;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("insecure secret store path")]
    UnsafePath,
    #[error("secret store exceeds size limit")]
    TooLarge,
    #[error("invalid secret store: {0}")]
    Invalid(&'static str),
    #[error("unsupported secret store version")]
    UnsupportedVersion,
    #[error("secret store modified concurrently during read")]
    Changed,
    #[error("device key export is unsupported for this profile")]
    CopyUnsupported,
    #[error("Keychain credential unavailable")]
    Keychain,
    #[error("failed to decrypt device credential")]
    Decryption,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Snowpack decode error: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageKind {
    Plaintext,
    Passphrase,
    MacosKeychain,
    NoiseFile,
    GenericKeychain,
}

impl StorageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plaintext => "plaintext",
            Self::Passphrase => "passphrase",
            Self::MacosKeychain => "macos-keychain",
            Self::NoiseFile => "noise-file",
            Self::GenericKeychain => "generic-keychain",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub username: Option<String>,
    pub server_hint: Option<String>,
    pub host_id_hex: String,
    pub user_id_hex: String,
    pub device_id_hex: String,
    pub role: String,
    pub storage: StorageKind,
    pub hidden: bool,
    pub provisional: bool,
    pub pairable: bool,
    pub copyable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Installation {
    pub installed: bool,
    pub root: PathBuf,
    pub candidates: Vec<Candidate>,
}

#[derive(Clone)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct MacosKeychainBundle {
    account: String,
    service: String,
    nonce: [u8; 16],
    ciphertext: Vec<u8>,
}

#[derive(Clone)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum StoredBundle {
    Plaintext,
    Passphrase,
    MacosKeychain(MacosKeychainBundle),
    NoiseFile,
    GenericKeychain,
}

impl StoredBundle {
    fn kind(&self) -> StorageKind {
        match self {
            Self::Plaintext => StorageKind::Plaintext,
            Self::Passphrase => StorageKind::Passphrase,
            Self::MacosKeychain(_) => StorageKind::MacosKeychain,
            Self::NoiseFile => StorageKind::NoiseFile,
            Self::GenericKeychain => StorageKind::GenericKeychain,
        }
    }
}

#[derive(Clone)]
pub struct ResolvedCandidate {
    pub summary: Candidate,
    host_id: [u8; 33],
    user_id: [u8; 33],
    device_id: Vec<u8>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    bundle: StoredBundle,
}

impl ResolvedCandidate {
    pub fn host_id(&self) -> &[u8; 33] {
        &self.host_id
    }

    pub fn user_id(&self) -> &[u8; 33] {
        &self.user_id
    }

    pub fn device_id(&self) -> &[u8] {
        &self.device_id
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn macos_keychain_bundle(&self) -> Option<(&str, &str, &[u8; 16], &[u8])> {
        match &self.bundle {
            StoredBundle::MacosKeychain(bundle) => Some((
                &bundle.service,
                &bundle.account,
                &bundle.nonce,
                &bundle.ciphertext,
            )),
            _ => None,
        }
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn decrypt_device_seed_with_keychain_data(
        &self,
        keychain_data: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>> {
        let Some((service, _, partial_nonce, ciphertext)) = self.macos_keychain_bundle() else {
            return Err(Error::CopyUnsupported);
        };
        if !self.summary.copyable || service != "foks" {
            return Err(Error::CopyUnsupported);
        }
        let encoded_key = Zeroizing::new(keychain_data.to_vec());
        let key = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(encoded_key.as_slice())
                .map_err(|_| Error::Decryption)?,
        );
        let key: Zeroizing<[u8; 32]> =
            Zeroizing::new(key.as_slice().try_into().map_err(|_| Error::Decryption)?);
        let mut nonce = [0u8; 24];
        nonce[..8].copy_from_slice(&SECRET_KEY_BUNDLE_TYPE_ID.to_be_bytes());
        nonce[8..].copy_from_slice(partial_nonce);
        let plaintext = Zeroizing::new(
            XSalsa20Poly1305::new((&*key).into())
                .decrypt((&nonce).into(), ciphertext)
                .map_err(|_| Error::Decryption)?,
        );
        let mut decoded = decode(&plaintext).map_err(|_| Error::Decryption)?;
        let seed = decode_secret_key_bundle(&decoded).map(Zeroizing::new);
        zeroize_value(&mut decoded);
        seed
    }

    #[cfg(target_os = "macos")]
    pub fn copy_device_seed(&self) -> Result<Zeroizing<[u8; 32]>> {
        let Some((service, account, _, _)) = self.macos_keychain_bundle() else {
            return Err(Error::CopyUnsupported);
        };
        if !self.summary.copyable || service != "foks" {
            return Err(Error::CopyUnsupported);
        }
        let data = Zeroizing::new(
            security_framework::passwords::get_generic_password(service, account)
                .map_err(|_| Error::Keychain)?,
        );
        self.decrypt_device_seed_with_keychain_data(&data)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn copy_device_seed(&self) -> Result<Zeroizing<[u8; 32]>> {
        Err(Error::CopyUnsupported)
    }
}

pub fn standard_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    #[cfg(target_os = "macos")]
    return Some(home.join("Library/Application Support/foks"));
    #[cfg(not(target_os = "macos"))]
    return Some(
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("foks"),
    );
}

pub fn discover_standard() -> Result<Installation> {
    let root = standard_root().ok_or(Error::UnsafePath)?;
    discover(&root)
}

pub fn discover(root: &Path) -> Result<Installation> {
    if !root.exists() {
        return Ok(Installation {
            installed: false,
            root: root.to_owned(),
            candidates: Vec::new(),
        });
    }
    validate_directory(root)?;
    let source = root.join(SECRET_STORE_NAME);
    if !source.exists() {
        return Ok(Installation {
            installed: true,
            root: root.to_owned(),
            candidates: Vec::new(),
        });
    }
    let bytes = read_private_file(&source)?;
    let records = decode_store(&bytes)?;
    let mut candidates = records
        .into_iter()
        .map(|record| record.summary)
        .collect::<Vec<_>>();
    hard_state::enrich(root, &mut candidates);
    Ok(Installation {
        installed: true,
        root: root.to_owned(),
        candidates,
    })
}

pub fn resolve(root: &Path, candidate_id: &str) -> Result<ResolvedCandidate> {
    if candidate_id.len() != 64 || !candidate_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Invalid("candidate id"));
    }
    validate_directory(root)?;
    let records = decode_store(&read_private_file(&root.join(SECRET_STORE_NAME))?)?;
    let mut matches = records
        .into_iter()
        .filter(|candidate| candidate.summary.id == candidate_id);
    let candidate = matches
        .next()
        .ok_or(Error::Invalid("candidate disappeared"))?;
    if matches.next().is_some() {
        return Err(Error::Invalid("duplicate candidate id"));
    }
    Ok(candidate)
}

fn read_private_file(path: &Path) -> Result<Vec<u8>> {
    let before = std::fs::symlink_metadata(path)?;
    validate_secret_metadata(&before)?;
    if before.len() > MAXIMUM_SECRET_STORE_BYTES {
        return Err(Error::TooLarge);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    validate_secret_metadata(&file.metadata()?)?;
    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.by_ref()
        .take(MAXIMUM_SECRET_STORE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAXIMUM_SECRET_STORE_BYTES {
        return Err(Error::TooLarge);
    }
    let after = file.metadata()?;
    if !same_file_snapshot(&before, &after) {
        return Err(Error::Changed);
    }
    Ok(bytes)
}

fn validate_directory(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::UnsafePath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(Error::UnsafePath);
        }
    }
    Ok(())
}

fn validate_secret_metadata(metadata: &std::fs::Metadata) -> Result<()> {
    if !metadata.is_file() {
        return Err(Error::UnsafePath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(Error::UnsafePath);
        }
    }
    Ok(())
}

fn same_file_snapshot(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
    }
    #[cfg(not(unix))]
    {
        before.len() == after.len() && before.modified().ok() == after.modified().ok()
    }
}

fn decode_store(bytes: &[u8]) -> Result<Vec<ResolvedCandidate>> {
    let root = decode(bytes)?;
    let outer = exact_array(&root, 2, "secret store")?;
    if unsigned(&outer[0], "secret store version")? != 2 {
        return Err(Error::UnsupportedVersion);
    }
    let payload = tagged(&outer[1], b"2", "secret store version")?;
    let store = exact_array(payload, 2, "secret store v2")?;
    let local_instance_id = fixed::<17>(&store[0], "local instance id")?;
    let rows = list(&store[1], "secret store keys")?;
    if rows.len() > MAXIMUM_CANDIDATES {
        return Err(Error::TooLarge);
    }
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        let candidate = decode_candidate(row, local_instance_id)?;
        let identity = (
            candidate.host_id,
            candidate.user_id,
            candidate.device_id.clone(),
        );
        if !seen.insert(identity) {
            return Err(Error::Invalid("duplicate candidate"));
        }
        candidates.push(candidate);
    }
    candidates.sort_by(|left, right| left.summary.id.cmp(&right.summary.id));
    Ok(candidates)
}

fn decode_candidate(value: &Value, local_instance_id: [u8; 17]) -> Result<ResolvedCandidate> {
    let fields = array_range(value, 8, 9, "secret key record")?;
    let fqur = exact_array(&fields[0], 2, "fully qualified user and role")?;
    let fqu = exact_array(&fqur[0], 2, "fully qualified user")?;
    let user_id = fixed::<33>(&fqu[0], "user id")?;
    let host_id = fixed::<33>(&fqu[1], "host id")?;
    if user_id[0] != 1 || host_id[0] != 2 {
        return Err(Error::Invalid("fully qualified user identity"));
    }
    let role = decode_role(&fqur[1])?;
    let device_id = binary(&fields[1], "device id")?.to_vec();
    if !matches!(device_id.len(), 33 | 34)
        || !device_id
            .first()
            .is_some_and(|kind| matches!(*kind, 4 | 8 | 16 | 19))
    {
        return Err(Error::Invalid("device id"));
    }
    let self_token = binary(&fields[2], "self token")?;
    if self_token.len() != 17 {
        return Err(Error::Invalid("self token length"));
    }
    let bundle = decode_stored_bundle(&fields[3])?;
    let provisional = boolean(&fields[4], "provisional flag")?;
    let _ = integer(&fields[5], "creation time")?;
    let _ = integer(&fields[6], "modification time")?;
    let _ = unsigned(&fields[7], "minor version")?;
    let hidden = fields
        .get(8)
        .map(|value| boolean(value, "hidden flag"))
        .transpose()?
        .unwrap_or(false);
    let id = candidate_id(
        &local_instance_id,
        &host_id,
        &user_id,
        &device_id,
        bundle.kind(),
    );
    let stable = !hidden && !provisional && role != "none";
    let copyable = cfg!(target_os = "macos")
        && stable
        && device_id[0] == 4
        && matches!(bundle, StoredBundle::MacosKeychain(_));
    Ok(ResolvedCandidate {
        summary: Candidate {
            id,
            username: None,
            server_hint: None,
            host_id_hex: hex(&host_id),
            user_id_hex: hex(&user_id),
            device_id_hex: hex(&device_id),
            role,
            storage: bundle.kind(),
            hidden,
            provisional,
            pairable: stable,
            copyable,
        },
        host_id,
        user_id,
        device_id,
        bundle,
    })
}

fn decode_role(value: &Value) -> Result<String> {
    let fields = exact_array(value, 2, "role")?;
    let kind = unsigned(&fields[0], "role kind")?;
    match kind {
        0 | 2 | 3 => {
            if !matches!(fields[1], Value::Variant(None)) {
                return Err(Error::Invalid("role payload"));
            }
            Ok(match kind {
                0 => "none",
                2 => "admin",
                _ => "owner",
            }
            .to_owned())
        }
        1 => {
            let level = integer(tagged(&fields[1], b"0", "member role")?, "member level")?;
            if !(-16384..=16384).contains(&level) {
                return Err(Error::Invalid("member level"));
            }
            Ok(format!("member({level})"))
        }
        _ => Err(Error::Invalid("role kind")),
    }
}

fn decode_stored_bundle(value: &Value) -> Result<StoredBundle> {
    let fields = exact_array(value, 2, "stored key bundle")?;
    let kind = unsigned(&fields[0], "storage kind")?;
    let payload = tagged(&fields[1], kind.to_string().as_bytes(), "stored key bundle")?;
    match kind {
        0 => {
            decode_secret_key_bundle(payload)?;
            Ok(StoredBundle::Plaintext)
        }
        1 => {
            let fields = exact_array(payload, 4, "passphrase bundle")?;
            let _ = unsigned(&fields[0], "passphrase generation")?;
            let _: [u8; 16] = fixed(&fields[1], "passphrase salt")?;
            let _ = unsigned(&fields[2], "stretch version")?;
            let _ = decode_secret_box(&fields[3])?;
            Ok(StoredBundle::Passphrase)
        }
        2 => {
            let fields = exact_array(payload, 3, "macOS keychain bundle")?;
            let account = text(&fields[0], "keychain account")?;
            let service = text(&fields[1], "keychain service")?;
            if account.is_empty() || account.len() > MAXIMUM_TEXT_BYTES || service != "foks" {
                return Err(Error::Invalid("keychain identity"));
            }
            let (nonce, ciphertext) = decode_secret_box(&fields[2])?;
            Ok(StoredBundle::MacosKeychain(MacosKeychainBundle {
                account,
                service,
                nonce,
                ciphertext,
            }))
        }
        3 => {
            let fields = exact_array(payload, 2, "noise-file bundle")?;
            let filename = text(&fields[0], "noise filename")?;
            if filename.is_empty() {
                return Err(Error::Invalid("noise filename"));
            }
            let _ = decode_secret_box(&fields[1])?;
            Ok(StoredBundle::NoiseFile)
        }
        4 => {
            let fields = exact_array(payload, 3, "generic keychain bundle")?;
            if !matches!(fields[0], Value::Null) {
                return Err(Error::Invalid("generic keychain deprecated field"));
            }
            let service = text(&fields[1], "generic keychain service")?;
            if service.is_empty() {
                return Err(Error::Invalid("generic keychain service"));
            }
            let _ = decode_secret_box(&fields[2])?;
            Ok(StoredBundle::GenericKeychain)
        }
        _ => Err(Error::Invalid("storage kind")),
    }
}

fn decode_secret_key_bundle(value: &Value) -> Result<[u8; 32]> {
    let fields = exact_array(value, 2, "secret key bundle")?;
    if unsigned(&fields[0], "secret key bundle version")? != 1 {
        return Err(Error::UnsupportedVersion);
    }
    fixed(
        tagged(&fields[1], b"1", "secret key bundle")?,
        "device seed",
    )
}

fn decode_secret_box(value: &Value) -> Result<([u8; 16], Vec<u8>)> {
    let fields = exact_array(value, 2, "secret box")?;
    if unsigned(&fields[0], "secret box kind")? != 0 {
        return Err(Error::Invalid("secret box kind"));
    }
    let nacl = exact_array(
        tagged(&fields[1], b"0", "secret box")?,
        2,
        "NaCl secret box",
    )?;
    let nonce = fixed(&nacl[0], "secret box nonce")?;
    let ciphertext = binary(&nacl[1], "secret box ciphertext")?.to_vec();
    if ciphertext.len() < 16 || ciphertext.len() > MAXIMUM_SECRET_STORE_BYTES as usize {
        return Err(Error::Invalid("secret box ciphertext length"));
    }
    Ok((nonce, ciphertext))
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn zeroize_value(value: &mut Value) {
    match value {
        Value::Binary(bytes) | Value::Text(bytes) => bytes.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(zeroize_value),
        Value::Variant(Some((tag, payload))) => {
            tag.zeroize();
            zeroize_value(payload);
        }
        Value::Null
        | Value::Bool(_)
        | Value::Unsigned(_)
        | Value::Negative(_)
        | Value::Variant(None) => {}
    }
}

fn candidate_id(
    local_instance_id: &[u8; 17],
    host_id: &[u8; 33],
    user_id: &[u8; 33],
    device_id: &[u8],
    storage: StorageKind,
) -> String {
    let mut hash = Sha256::new();
    hash.update(b"foks-go-profile-candidate-v1");
    hash.update(local_instance_id);
    hash.update(host_id);
    hash.update(user_id);
    hash.update((device_id.len() as u64).to_be_bytes());
    hash.update(device_id);
    hash.update([storage as u8]);
    hex(&hash.finalize())
}

fn exact_array<'a>(value: &'a Value, length: usize, name: &'static str) -> Result<&'a [Value]> {
    array_range(value, length, length, name)
}

fn array_range<'a>(
    value: &'a Value,
    minimum: usize,
    maximum: usize,
    name: &'static str,
) -> Result<&'a [Value]> {
    let Value::Array(fields) = value else {
        return Err(Error::Invalid(name));
    };
    if !(minimum..=maximum).contains(&fields.len()) {
        return Err(Error::Invalid(name));
    }
    Ok(fields)
}

fn list<'a>(value: &'a Value, name: &'static str) -> Result<&'a [Value]> {
    match value {
        Value::Null => Ok(&[]),
        Value::Array(values) => Ok(values),
        _ => Err(Error::Invalid(name)),
    }
}

fn tagged<'a>(value: &'a Value, tag: &[u8], name: &'static str) -> Result<&'a Value> {
    let Value::Variant(Some((actual, payload))) = value else {
        return Err(Error::Invalid(name));
    };
    if actual != tag {
        return Err(Error::Invalid(name));
    }
    Ok(payload)
}

fn binary<'a>(value: &'a Value, name: &'static str) -> Result<&'a [u8]> {
    let Value::Binary(bytes) = value else {
        return Err(Error::Invalid(name));
    };
    Ok(bytes)
}

fn fixed<const N: usize>(value: &Value, name: &'static str) -> Result<[u8; N]> {
    binary(value, name)?
        .try_into()
        .map_err(|_| Error::Invalid(name))
}

fn text(value: &Value, name: &'static str) -> Result<String> {
    let Value::Text(bytes) = value else {
        return Err(Error::Invalid(name));
    };
    if bytes.len() > MAXIMUM_TEXT_BYTES {
        return Err(Error::TooLarge);
    }
    String::from_utf8(bytes.clone()).map_err(|_| Error::Invalid(name))
}

fn unsigned(value: &Value, name: &'static str) -> Result<u64> {
    match value {
        Value::Unsigned(value) => Ok(*value),
        _ => Err(Error::Invalid(name)),
    }
}

fn integer(value: &Value, name: &'static str) -> Result<i64> {
    match value {
        Value::Unsigned(value) => i64::try_from(*value).map_err(|_| Error::Invalid(name)),
        Value::Negative(value) => Ok(*value),
        _ => Err(Error::Invalid(name)),
    }
}

fn boolean(value: &Value, name: &'static str) -> Result<bool> {
    match value {
        Value::Bool(value) => Ok(*value),
        _ => Err(Error::Invalid(name)),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut encoded, byte| {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
        encoded
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_snowpack::encode;
    use std::os::unix::fs::PermissionsExt as _;

    fn variant(tag: &str, value: Value) -> Value {
        Value::Variant(Some((tag.as_bytes().to_vec(), Box::new(value))))
    }

    fn role_owner() -> Value {
        Value::Array(vec![Value::Unsigned(3), Value::Variant(None)])
    }

    fn secret_box() -> Value {
        Value::Array(vec![
            Value::Unsigned(0),
            variant(
                "0",
                Value::Array(vec![Value::Binary(vec![8; 16]), Value::Binary(vec![9; 48])]),
            ),
        ])
    }

    fn store(hidden: bool, provisional: bool) -> Vec<u8> {
        encode(&Value::Array(vec![
            Value::Unsigned(2),
            variant(
                "2",
                Value::Array(vec![
                    Value::Binary(vec![7; 17]),
                    Value::Array(vec![Value::Array(vec![
                        Value::Array(vec![
                            Value::Array(vec![
                                Value::Binary({
                                    let mut id = vec![1];
                                    id.extend([2; 32]);
                                    id
                                }),
                                Value::Binary({
                                    let mut id = vec![2];
                                    id.extend([3; 32]);
                                    id
                                }),
                            ]),
                            role_owner(),
                        ]),
                        Value::Binary({
                            let mut id = vec![4];
                            id.extend([5; 32]);
                            id
                        }),
                        Value::Binary(vec![6; 17]),
                        Value::Array(vec![
                            Value::Unsigned(2),
                            variant(
                                "2",
                                Value::Array(vec![
                                    Value::Text(b"account-label".to_vec()),
                                    Value::Text(b"foks".to_vec()),
                                    secret_box(),
                                ]),
                            ),
                        ]),
                        Value::Bool(provisional),
                        Value::Unsigned(1),
                        Value::Unsigned(2),
                        Value::Unsigned(0),
                        Value::Bool(hidden),
                    ])]),
                ]),
            ),
        ]))
        .unwrap()
    }

    #[test]
    fn decodes_supported_candidate_without_exposing_secret_fields() {
        let candidates = decode_store(&store(false, false)).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].summary.role, "owner");
        assert_eq!(candidates[0].summary.storage, StorageKind::MacosKeychain);
        assert!(candidates[0].summary.pairable);
        assert_eq!(candidates[0].summary.copyable, cfg!(target_os = "macos"));
        assert_eq!(candidates[0].summary.id.len(), 64);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_keychain_bundle_decrypts_only_the_typed_device_seed() {
        let mut candidate = decode_store(&store(false, false)).unwrap().remove(0);
        let seed = [0x42; 32];
        let plaintext = encode(&Value::Array(vec![
            Value::Unsigned(1),
            variant("1", Value::Binary(seed.to_vec())),
        ]))
        .unwrap();
        let key = [0x24; 32];
        let mut nonce = [0u8; 24];
        nonce[..8].copy_from_slice(&SECRET_KEY_BUNDLE_TYPE_ID.to_be_bytes());
        nonce[8..].copy_from_slice(&[8; 16]);
        let ciphertext = XSalsa20Poly1305::new((&key).into())
            .encrypt((&nonce).into(), plaintext.as_slice())
            .unwrap();
        let StoredBundle::MacosKeychain(bundle) = &mut candidate.bundle else {
            panic!("fixture storage kind changed");
        };
        bundle.ciphertext = ciphertext;
        let encoded = base64::engine::general_purpose::STANDARD.encode(key);
        assert_eq!(
            candidate
                .decrypt_device_seed_with_keychain_data(encoded.as_bytes())
                .unwrap()
                .as_slice(),
            seed
        );
        assert!(candidate
            .decrypt_device_seed_with_keychain_data(b"not-base64")
            .is_err());
        let StoredBundle::MacosKeychain(bundle) = &mut candidate.bundle else {
            panic!("fixture storage kind changed");
        };
        bundle.ciphertext[0] ^= 1;
        assert!(candidate
            .decrypt_device_seed_with_keychain_data(encoded.as_bytes())
            .is_err());
    }

    #[test]
    fn hidden_and_provisional_candidates_are_visible_but_inert() {
        for bytes in [store(true, false), store(false, true)] {
            let candidate = decode_store(&bytes).unwrap().remove(0).summary;
            assert!(!candidate.pairable);
            assert!(!candidate.copyable);
        }
    }

    #[test]
    fn discovery_rejects_unsafe_source_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(SECRET_STORE_NAME);
        std::fs::write(&path, store(false, false)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(discover(directory.path()), Err(Error::UnsafePath)));
    }

    #[test]
    fn discovery_is_empty_when_the_installation_is_absent() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("missing");
        let installation = discover(&root).unwrap();
        assert!(!installation.installed);
        assert!(installation.candidates.is_empty());
    }
}
