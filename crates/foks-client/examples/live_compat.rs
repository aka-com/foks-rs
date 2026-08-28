use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use foks_client::{
    FoksClient, KvWriteOptions, ProbeTarget, ProtectedMutationStore, ProtectedStoreError,
    SoftwareAccountRequest, SoftwareAccountSecrets,
};
use foks_proto::{InviteCode, Role, SecretSeed};
use rustls::pki_types::CertificateDer;
use zeroize::Zeroizing;

#[derive(Default)]
struct EphemeralProtectedStore(BTreeMap<Vec<u8>, Vec<u8>>);

impl ProtectedMutationStore for EphemeralProtectedStore {
    fn put_if_absent(&mut self, key: &[u8], material: &[u8]) -> Result<(), ProtectedStoreError> {
        match self.0.get(key) {
            Some(existing) if existing != material => Err(ProtectedStoreError::Conflict),
            Some(_) => Ok(()),
            None => {
                self.0.insert(key.to_vec(), material.to_vec());
                Ok(())
            }
        }
    }

    fn get(&mut self, key: &[u8]) -> Result<Zeroizing<Vec<u8>>, ProtectedStoreError> {
        self.0
            .get(key)
            .cloned()
            .map(Zeroizing::new)
            .ok_or(ProtectedStoreError::Missing)
    }

    fn remove(&mut self, key: &[u8]) -> Result<(), ProtectedStoreError> {
        self.0
            .remove(key)
            .map(|_| ())
            .ok_or(ProtectedStoreError::Missing)
    }
}

fn argument(name: &str) -> Result<String, String> {
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == name {
            return arguments
                .next()
                .ok_or_else(|| format!("missing value for {name}"));
        }
    }
    Err(format!("missing required argument {name}"))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let probe = argument("--probe")?;
    let ca_der = PathBuf::from(argument("--ca-der")?);
    let state_directory = PathBuf::from(argument("--state-dir")?);
    let username = argument("--username")?;

    fs::create_dir_all(&state_directory)?;
    let hard_database = state_directory.join("hard.sqlite3");
    let soft_database = state_directory.join("soft.sqlite3");
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(fs::read(ca_der)?))?;

    let mut client = FoksClient::with_roots(roots);
    client.set_timeout(std::time::Duration::from_secs(30));
    let target = ProbeTarget::parse(&probe)?;
    let host = client.probe_and_pin(&target, &hard_database)?.pinned;

    eprintln!("live compatibility: probing registration RPC");
    client.reserve_username(&host, "rustreserve")?;
    eprintln!("live compatibility: probing Merkle RPC");
    client.advance_merkle_root(&host)?;
    eprintln!("live compatibility: creating account");

    let mut self_token = [0x73; 17];
    self_token[0] = 54; // FOKS ID16Type_PermissionToken
                        // The live harness is single-process and ephemeral. Production callers
                        // provide a durable encrypted implementation of this boundary.
    let mut protected = EphemeralProtectedStore::default();
    let created = client.create_software_account(
        &host,
        SoftwareAccountRequest {
            username_utf8: username.clone(),
            device_name: "Rust live compatibility client".to_owned(),
            invite_code: InviteCode::Empty,
            email: format!("{username}@example.invalid"),
            passphrase: None,
        },
        SoftwareAccountSecrets::new(
            SecretSeed::new([0x31; 32]),
            SecretSeed::new([0x52; 32]),
            self_token,
        ),
        &soft_database,
        &mut protected,
    )?;
    if created.authenticated.verified.username() != username.as_bytes() {
        return Err("authenticated username differs from the created account".into());
    }
    if created.authenticated.current_puk().is_none() {
        return Err("created account has no current PUK".into());
    }
    let root = created
        .kv_projection
        .first()
        .ok_or("created account has no personal KV root")?
        .root_directory_id;

    let expected = b"written by the Rust foks-client live compatibility harness";
    let mut session = client.user_kv_write_session(
        &host,
        &created.credential,
        &created.authenticated.verified,
        &created.authenticated.puks,
        &soft_database,
        &mut protected,
    )?;
    session.put_file(
        root,
        "compatibility.txt",
        &mut Cursor::new(expected),
        KvWriteOptions {
            read_role: Role::OWNER,
            write_role: Role::OWNER,
            overwrite: false,
            expected_version: None,
        },
    )?;
    let cached = session.sync()?;
    let file = cached
        .iter()
        .flat_map(|directory| &directory.entries)
        .find(|entry| entry.name == b"compatibility.txt")
        .ok_or("written file is absent from the synchronized SQLite projection")?;
    if file.content.as_deref() != Some(expected.as_slice()) {
        return Err("written file content differs after incremental synchronization".into());
    }

    println!("verified FOKS v0.1.9 live account {username}, user chain, PUK, and KV round trip");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("foks-client live compatibility failure: {error}");
        std::process::exit(1);
    }
}
