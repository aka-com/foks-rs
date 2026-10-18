//! Fixed, hooked, bounded reads followed by existing cryptographic validators.
use super::*;
use crate::keys::{HostKeyProvider, KeyGenerationId, KeyGenerationManifest, KeyPurpose};
use foks_proto::{MerkleRoot, ProbeResponse, HOSTCHAIN_LINK_OUTER_TYPE_ID, MERKLE_ROOT_TYPE_ID};
use foks_server_db::{
    CapabilityKeyGeneration, CapabilityKeyGenerationState as CapabilityState, HostKeyGeneration,
    HostKeyGenerationState as HostState, HostRotationOperation, HostRotationPhase, RootSnapshot,
    StoredHostBootstrap,
};
use rusqlite::Row;
const MIB: usize = 1024 * 1024;
fn u(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| unsupported("import key integer range"))
}
fn maybe(row: &Row<'_>, i: usize) -> Result<Option<u64>> {
    row.get::<_, Option<i64>>(i)?.map(u).transpose()
}
fn fixed<const N: usize>(row: &Row<'_>, i: usize) -> Result<[u8; N]> {
    row.get_ref(i)?
        .as_blob()
        .map_err(|_| unsupported("import key blob type"))?
        .try_into()
        .map_err(|_| unsupported("import key blob width"))
}
fn bytes(row: &Row<'_>, i: usize, max: usize) -> Result<Vec<u8>> {
    let value = row
        .get_ref(i)?
        .as_blob()
        .map_err(|_| unsupported("import key blob type"))?;
    if value.len() > max {
        return Err(unsupported("import key blob cap"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(value.len())
        .map_err(|_| ScanError::Allocation)?;
    out.extend_from_slice(value);
    Ok(out)
}
fn text(row: &Row<'_>, i: usize, max: usize) -> Result<String> {
    let value = row
        .get_ref(i)?
        .as_str()
        .map_err(|_| unsupported("import key text type"))?;
    if value.len() > max {
        return Err(unsupported("import key text cap"));
    }
    let mut out = String::new();
    out.try_reserve_exact(value.len())
        .map_err(|_| ScanError::Allocation)?;
    out.push_str(value);
    Ok(out)
}
fn push<T>(values: &mut Vec<T>, value: T) -> Result<()> {
    if values.len() >= 4096 {
        return Err(unsupported("import key row cap"));
    }
    values.try_reserve(1).map_err(|_| ScanError::Allocation)?;
    values.push(value);
    Ok(())
}

pub(super) fn host_generations(db: &scan::Database) -> Result<Vec<HostKeyGeneration>> {
    preflight_generations(db, true)?;
    db.control().statement()?;
    let mut stmt=db.connection().prepare("SELECT generation_id,encrypted_file_name,public_entity_id,state,created_at,activated_hostchain_seqno FROM main.host_key_generations ORDER BY created_at,generation_id")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        db.control().check()?;
        let state = match row.get::<_, i64>(3)? {
            1 => HostState::Staged,
            2 => HostState::Active,
            3 => HostState::Retiring,
            4 => HostState::Revoked,
            _ => return Err(unsupported("import host generation state")),
        };
        push(
            &mut out,
            HostKeyGeneration {
                generation_id: fixed(row, 0)?,
                encrypted_file_name: text(row, 1, 96)?,
                public_entity_id: bytes(row, 2, 33)?,
                state,
                created_at: u(row.get(4)?)?,
                activated_hostchain_seqno: maybe(row, 5)?,
            },
        )?;
    }
    Ok(out)
}
pub(super) fn capabilities(db: &scan::Database) -> Result<Vec<CapabilityKeyGeneration>> {
    preflight_generations(db, false)?;
    db.control().statement()?;
    let mut stmt=db.connection().prepare("SELECT generation_id,encrypted_file_name,state,created_at,retire_after FROM main.capability_key_generations ORDER BY created_at,generation_id")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        db.control().check()?;
        let state = match row.get::<_, i64>(2)? {
            1 => CapabilityState::Active,
            2 => CapabilityState::Retiring,
            3 => CapabilityState::Revoked,
            _ => return Err(unsupported("import capability state")),
        };
        push(
            &mut out,
            CapabilityKeyGeneration {
                generation_id: fixed(row, 0)?,
                encrypted_file_name: text(row, 1, 96)?,
                state,
                created_at: u(row.get(3)?)?,
                retire_after: maybe(row, 4)?,
            },
        )?;
    }
    Ok(out)
}
fn preflight_generations(db: &scan::Database, host: bool) -> Result<()> {
    let sql = if host {
        "SELECT generation_id,encrypted_file_name,public_entity_id FROM main.host_key_generations LIMIT 4097"
    } else {
        "SELECT generation_id,encrypted_file_name FROM main.capability_key_generations LIMIT 4097"
    };
    db.control().statement()?;
    let mut stmt = db.connection().prepare(sql)?;
    let mut rows = stmt.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        db.control().check()?;
        count += 1;
        if count > 4096 {
            return Err(unsupported("key preflight count"));
        }
        fixed::<16>(row, 0)?;
        let name = row
            .get_ref(1)?
            .as_str()
            .map_err(|_| unsupported("key filename type"))?;
        if name.len() > 96 {
            return Err(unsupported("key filename bytes"));
        }
        if host {
            fixed::<33>(row, 2)?;
        }
    }
    Ok(())
}
pub(super) fn rotation(db: &scan::Database) -> Result<Option<HostRotationOperation>> {
    db.control().statement()?;
    let mut stmt=db.connection().prepare("SELECT operation_id,old_generation_id,new_generation_id,phase,add_link_seqno,add_published_at,revoke_link_seqno,created_at,updated_at FROM main.host_rotation_operations WHERE purpose=1 AND phase!=3 LIMIT 2")?;
    let mut rows = stmt.query([])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let result = HostRotationOperation {
        operation_id: fixed(row, 0)?,
        old_generation_id: fixed(row, 1)?,
        new_generation_id: fixed(row, 2)?,
        phase: match row.get::<_, i64>(3)? {
            1 => HostRotationPhase::Staged,
            2 => HostRotationPhase::Published,
            3 => HostRotationPhase::Complete,
            _ => return Err(unsupported("host rotation phase")),
        },
        add_link_seqno: maybe(row, 4)?,
        add_published_at: maybe(row, 5)?,
        revoke_link_seqno: maybe(row, 6)?,
        created_at: u(row.get(7)?)?,
        updated_at: u(row.get(8)?)?,
    };
    if rows.next()?.is_some() {
        return Err(unsupported("host rotation cardinality"));
    }
    Ok(Some(result))
}
pub(super) fn bootstrap(db: &scan::Database) -> Result<StoredHostBootstrap> {
    db.control().statement()?;
    let mut stmt=db.connection().prepare("SELECT host_id,canonical_name,bootstrap_blob,key_manifest_blob FROM main.host_metadata WHERE singleton=1")?;
    let mut rows = stmt.query([])?;
    let row = rows
        .next()?
        .ok_or(unsupported("import bootstrap missing"))?;
    fixed::<33>(row, 0)?;
    // Check all borrowed dimensions before any typed owned collector.
    for (i, maximum) in [(2, MIB), (3, 4096)] {
        if row
            .get_ref(i)?
            .as_blob()
            .map_err(|_| unsupported("bootstrap blob type"))?
            .len()
            > maximum
        {
            return Err(unsupported("bootstrap blob cap"));
        }
    }
    let name = row
        .get_ref(1)?
        .as_str()
        .map_err(|_| unsupported("bootstrap name type"))?;
    if name.len() > 255 {
        return Err(unsupported("bootstrap name cap"));
    }
    let result = StoredHostBootstrap {
        host_id: bytes(row, 0, 33)?,
        canonical_name: text(row, 1, 255)?,
        probe_response: bytes(row, 2, MIB)?,
        key_manifest: bytes(row, 3, 4096)?,
    };
    if !result.key_manifest.is_ascii() {
        return Err(unsupported("bootstrap ASCII manifest"));
    }
    Ok(result)
}
pub(super) fn root(db: &scan::Database, epoch: Option<u64>) -> Result<RootSnapshot> {
    let sql = if epoch.is_some() {
        "SELECT epoch,root_hash,root_node,exact_root,exact_signed_root FROM main.merkle_roots WHERE epoch=?1"
    } else {
        "SELECT r.epoch,r.root_hash,r.root_node,r.exact_root,r.exact_signed_root FROM main.merkle_root_heads h JOIN main.merkle_roots r ON r.epoch=h.epoch AND r.root_hash=h.root_hash WHERE h.singleton=1"
    };
    db.control().statement()?;
    let mut stmt = db.connection().prepare(sql)?;
    let epoch = epoch
        .map(|v| i64::try_from(v).map_err(|_| unsupported("probe epoch range")))
        .transpose()?;
    let mut rows = match epoch {
        Some(epoch) => stmt.query([epoch])?,
        None => stmt.query([])?,
    };
    let row = rows.next()?.ok_or(unsupported("import root missing"))?;
    for i in [3, 4] {
        if row
            .get_ref(i)?
            .as_blob()
            .map_err(|_| unsupported("import root blob"))?
            .len()
            > MIB
        {
            return Err(unsupported("import root blob cap"));
        }
    }
    Ok(RootSnapshot {
        epoch: u(row.get(0)?)?,
        root_hash: fixed(row, 1)?,
        root_node: fixed(row, 2)?,
        exact_root: bytes(row, 3, MIB)?,
        exact_signed_root: bytes(row, 4, MIB)?,
    })
}
fn hostchain_preflight(db: &scan::Database) -> Result<()> {
    db.control().statement()?;
    let mut stmt = db.connection().prepare(
        "SELECT typeof(exact_link),length(exact_link) FROM main.hostchain_links LIMIT 4097",
    )?;
    let mut rows = stmt.query([])?;
    let mut count = 0;
    let mut total = 0u64;
    while let Some(row) = rows.next()? {
        db.control().check()?;
        count += 1;
        if count > 4096 || row.get_ref(0)? != ValueRef::Text(b"blob") {
            return Err(unsupported("hostchain count/type"));
        }
        total = total
            .checked_add(u(row.get(1)?)?)
            .ok_or(ScanError::Allocation)?;
        if total > MIB as u64 {
            return Err(unsupported("hostchain aggregate cap"));
        }
    }
    Ok(())
}
struct CapabilityReader<'a> {
    db: &'a scan::Database,
    error: std::cell::RefCell<Option<ScanError>>,
}
impl crate::keys::CapabilityGenerationReader for CapabilityReader<'_> {
    fn capability_generations(&self) -> crate::Result<Vec<CapabilityKeyGeneration>> {
        capabilities(self.db).map_err(|error| {
            *self.error.borrow_mut() = Some(error);
            crate::Error::Config("bounded import capability read")
        })
    }
}

pub(super) fn validate_existing_import(
    db: &scan::Database,
    provider: &crate::keys::DirectoryKeyProvider,
    package: &capture::Package,
) -> Result<([u8; 33], String)> {
    db.control().check()?;
    let stored = bootstrap(db)?;
    if stored.key_manifest != package.manifest()? {
        return Err(unsupported("import manifest mismatch"));
    }
    hostchain_preflight(db)?;
    let generations = host_generations(db)?;
    let operation = rotation(db)?;
    let capability_error = std::cell::RefCell::new(None);
    let files =
        crate::standalone::host_key_backup_files_from_loaded(generations, operation, || {
            capabilities(db).map_err(|error| {
                *capability_error.borrow_mut() = Some(error);
                crate::Error::Config("bounded import capability read")
            })
        });
    if let Some(error) = capability_error.into_inner() {
        return Err(error);
    }
    let files = files.map_err(server)?;
    let mut expected = std::collections::BTreeSet::new();
    expected.insert(crate::keys::WRAPPING_KEY_FILE);
    for name in [
        "metadata.key",
        "merkle.key",
        "client-ca.key",
        "delegated-tls.key",
        "recovery.key",
    ] {
        expected.insert(name);
    }
    if files.include_genesis {
        expected.insert("host.key");
    }
    if files.include_genesis_capability {
        expected.insert("capability.key");
    }
    for name in &files.generated {
        expected.insert(name.as_str());
    }
    if !expected.into_iter().eq(package.key_names()) {
        return Err(unsupported("import key inventory closure"));
    }
    let manifest = KeyGenerationManifest::decode(&stored.key_manifest).map_err(server)?;
    manifest
        .validate_existing(
            provider,
            files.include_genesis,
            files.include_genesis_capability,
            false,
        )
        .map_err(server)?;
    let generations = host_generations(db)?;
    for generation in &generations {
        db.control().check()?;
        if generation.state == HostState::Revoked {
            continue;
        }
        let key = if generation.encrypted_file_name == "host.key" {
            provider.load_existing(KeyPurpose::Host)
        } else {
            provider.load_generation(
                KeyPurpose::Host,
                KeyGenerationId::from_bytes(generation.generation_id),
            )
        }
        .map_err(server)?;
        let mut entity = [0; 33];
        entity[0] = foks_proto::ENTITY_HOST;
        entity[1..].copy_from_slice(&foks_crypto::ed25519_public_key(key.expose()));
        if key.generation().as_bytes() != generation.generation_id
            || generation.public_entity_id != entity
        {
            return Err(unsupported("import host key mismatch"));
        }
    }
    let reader = CapabilityReader {
        db,
        error: std::cell::RefCell::new(None),
    };
    let valid = crate::keys::validate_capability_key_generations(&reader, provider);
    if let Some(error) = reader.error.into_inner() {
        return Err(error);
    }
    valid.map_err(server)?;
    db.control().check()?;
    let verified = foks_verify::verify_public_host(&stored.canonical_name, &stored.probe_response)
        .map_err(verification)?;
    if verified.snapshot.host_id() != stored.host_id {
        return Err(unsupported("import host identity mismatch"));
    }
    let probe = ProbeResponse::decode(&stored.probe_response).map_err(protocol)?;
    crate::host::validate_generation_ledger_against_probe(&generations, &stored.host_id, &probe)
        .map_err(server)?;
    let wire_root = MerkleRoot::decode(&probe.merkle_root.inner).map_err(protocol)?;
    let probe_root = root(db, Some(wire_root.epoch))?;
    let current = root(db, None)?;
    if probe_root.epoch != wire_root.epoch
        || probe_root.root_node != wire_root.root_node
        || probe_root.exact_root != probe.merkle_root.inner
        || probe_root.exact_signed_root != probe.merkle_root.encoded().map_err(protocol)?
        || probe_root.root_hash
            != foks_crypto::prefixed_hash_signable(MERKLE_ROOT_TYPE_ID, &probe_root.exact_root)
                .map_err(|_| unsupported("import root hash"))?
        || current.epoch < probe_root.epoch
        || wire_root.hostchain.seqno != verified.snapshot.chain_seqno()
        || wire_root.hostchain.hash != verified.snapshot.chain_tail_hash()
    {
        return Err(unsupported("import root/hostchain mismatch"));
    }
    db.control().statement()?;
    let mut stmt = db
        .connection()
        .prepare("SELECT seqno,link_hash,exact_link FROM main.hostchain_links ORDER BY seqno")?;
    let mut rows = stmt.query([])?;
    for (index, link) in probe.hostchain.iter().enumerate() {
        db.control().check()?;
        let row = rows
            .next()?
            .ok_or(unsupported("import hostchain missing row"))?;
        let exact = link.encoded().map_err(protocol)?;
        if u(row.get(0)?)? != index as u64 + 1
            || row.get_ref(2)? != ValueRef::Blob(&exact)
            || fixed::<32>(row, 1)?
                != foks_crypto::prefixed_hash_signable(HOSTCHAIN_LINK_OUTER_TYPE_ID, &exact)
                    .map_err(|_| unsupported("import hostchain hash"))?
        {
            return Err(unsupported("import hostchain mismatch"));
        }
    }
    if rows.next()?.is_some() {
        return Err(unsupported("import hostchain extra row"));
    }
    drop(rows);
    drop(stmt);
    let parts = verified.snapshot.parts();
    let identity = foks_verify::restore_public_host_identity(
        parts.host_id,
        parts.genesis_key,
        parts.chain_seqno,
        parts.chain_tail_hash,
        parts.chain_bytes,
        parts.public_zone_bytes,
    )
    .map_err(verification)?;
    let delegated = provider
        .load_existing(KeyPurpose::DelegatedTls)
        .map_err(server)?;
    let der = crate::pki::host_tls::delegated_tls_ca_der(&delegated, &stored.canonical_name)
        .map_err(server)?;
    if !identity.tls_ca_certificates().iter().any(|v| v == &der) {
        return Err(unsupported("import delegated TLS key"));
    }
    db.control().check()?;
    let host = stored
        .host_id
        .as_slice()
        .try_into()
        .map_err(|_| unsupported("import verified host width"))?;
    Ok((host, stored.canonical_name))
}
