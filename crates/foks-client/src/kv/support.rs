//! KV binding, traversal, version-vector, and validation helpers.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;

use foks_client_db::KvDirectoryProjection;
use foks_crypto::{bind_kv_dirent, derive_kv_keys};
use foks_proto::{
    KvDirectoryStatus, KvDirectoryVersion, KvDirent, KvDirentVersion, KvNodeId, KvNodeType,
    KvPathVersionVector, Role, SecretSeed,
};
use foks_verify::VerifiedUserState;

use super::{KvPrivateKeyRef, KvWriteSession};
use crate::{random_bytes, user_key_for_seed, Error, Result, UserPrivateKey};

#[allow(clippy::too_many_arguments)]
pub(crate) fn bound_dirent(
    seed: &SecretSeed,
    parent: [u8; 16],
    id: [u8; 16],
    value: KvNodeId,
    version: u64,
    directory_version: u64,
    write_role: Role,
    name_mac: [u8; 32],
    name_box: foks_proto::SecretBox,
    directory_status: KvDirectoryStatus,
    creation_time: u64,
) -> Result<KvDirent> {
    let unbound = KvDirent::new(
        parent,
        id,
        value,
        version,
        directory_version,
        write_role,
        name_mac,
        name_box.clone(),
        directory_status,
        [0; 32],
        creation_time,
    )?;
    Ok(KvDirent::new(
        parent,
        id,
        value,
        version,
        directory_version,
        write_role,
        name_mac,
        name_box,
        directory_status,
        bind_kv_dirent(seed, &unbound)?,
        creation_time,
    )?)
}

pub(crate) fn kv_version_vector_from_tree(tree: &[KvDirectoryProjection]) -> KvPathVersionVector {
    let map = tree
        .iter()
        .cloned()
        .map(|directory| (directory.directory_id, directory))
        .collect::<BTreeMap<_, _>>();
    kv_version_vector(
        tree.first().map_or(0, |directory| directory.root_version),
        &map,
    )
}

pub(crate) fn is_kv_stale_cache(error: &Error) -> bool {
    matches!(error, Error::Rpc(foks_rpc::Error::KvStaleCache(_)))
}

pub(crate) fn validate_kv_component(name: &[u8]) -> Result<()> {
    if name.is_empty()
        || name.len() > 255
        || std::str::from_utf8(name).is_err()
        || name.contains(&0)
        || name.contains(&b'/')
        || name == b"."
        || name == b".."
    {
        return Err(Error::KvRequest("invalid KV path component"));
    }
    Ok(())
}

pub(crate) fn random_kv_node_id(node_type: KvNodeType) -> Result<KvNodeId> {
    // In go-foks v0.1.9 the object ID is also the deterministic Secretbox
    // nonce suffix. Every new ciphertext therefore gets a new 128-bit ID,
    // including overwrites of an existing path.
    let mut bytes = [0; 17];
    bytes[0] = node_type as u8;
    bytes[1..].copy_from_slice(&random_bytes::<16>()?);
    Ok(KvNodeId(bytes))
}

pub(crate) fn read_kv_upload_chunk<R: Read>(reader: &mut R) -> Result<(Vec<u8>, Vec<u8>, bool)> {
    read_kv_upload_chunk_with_carry(reader, Vec::new())
}

pub(crate) fn read_kv_upload_chunk_with_carry<R: Read>(
    reader: &mut R,
    mut bytes: Vec<u8>,
) -> Result<(Vec<u8>, Vec<u8>, bool)> {
    while bytes.len() <= KvWriteSession::MAX_UPLOAD_CHUNK {
        let remaining = KvWriteSession::MAX_UPLOAD_CHUNK + 1 - bytes.len();
        let mut buffer = vec![0; remaining.min(64 * 1024)];
        let read = reader.read(&mut buffer).map_err(foks_rpc::Error::Io)?;
        if read == 0 {
            return Ok((bytes, Vec::new(), true));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    let carry = bytes.split_off(KvWriteSession::MAX_UPLOAD_CHUNK);
    Ok((bytes, carry, false))
}

pub(crate) fn kv_directory_reaches(
    tree: &[KvDirectoryProjection],
    root: [u8; 16],
    target: [u8; 16],
) -> Result<bool> {
    let directories = tree
        .iter()
        .map(|directory| (directory.directory_id, directory))
        .collect::<BTreeMap<_, _>>();
    let mut queue = VecDeque::from([root]);
    let mut visited = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        if id == target {
            return Ok(true);
        }
        if !visited.insert(id) {
            continue;
        }
        let directory = directories
            .get(&id)
            .ok_or(Error::KvResponse("move source tree is incomplete"))?;
        for entry in &directory.entries {
            if entry.node_id[0] == KvNodeType::Directory as u8 {
                if !entry.readable {
                    return Err(Error::KvResponse(
                        "move source tree contains an inaccessible directory",
                    ));
                }
                queue.push_back(
                    entry.node_id[1..]
                        .try_into()
                        .expect("projected KV node IDs are fixed width"),
                );
            }
        }
    }
    Ok(false)
}

pub(crate) fn reachable_kv_tree(
    root: [u8; 16],
    directories: BTreeMap<[u8; 16], KvDirectoryProjection>,
) -> Result<BTreeMap<[u8; 16], KvDirectoryProjection>> {
    let mut queue = VecDeque::from([root]);
    let mut reachable = BTreeMap::new();
    while let Some(id) = queue.pop_front() {
        if reachable.contains_key(&id) {
            continue;
        }
        let directory = directories
            .get(&id)
            .ok_or(Error::KvResponse(
                "reachable directory is absent from cache",
            ))?
            .clone();
        for entry in &directory.entries {
            if entry.readable && entry.node_id[0] == 1 {
                queue.push_back(
                    entry.node_id[1..]
                        .try_into()
                        .expect("projected KV node IDs are fixed width"),
                );
            }
        }
        reachable.insert(id, directory);
    }
    Ok(reachable)
}

pub(crate) fn kv_version_vector(
    root_version: u64,
    directories: &BTreeMap<[u8; 16], KvDirectoryProjection>,
) -> KvPathVersionVector {
    KvPathVersionVector {
        root_version,
        directories: directories
            .values()
            .map(|directory| KvDirectoryVersion {
                id: directory.directory_id,
                version: directory.directory_version,
                entries: directory
                    .entries
                    .iter()
                    .map(|entry| KvDirentVersion {
                        id: entry.dirent_id,
                        version: entry.version,
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub(crate) fn kv_key(
    private_keys: &[KvPrivateKeyRef<'_>],
    role: Role,
    generation: u64,
) -> Result<foks_crypto::KvKeySet> {
    let matches = private_keys
        .iter()
        .filter(|key| key.role == role && key.generation == generation)
        .collect::<Vec<_>>();
    let [key] = matches.as_slice() else {
        return Err(Error::KvResponse("missing or duplicate PUK/PTK generation"));
    };
    derive_kv_keys(key.seed).map_err(Into::into)
}

pub(crate) fn user_kv_keys<'a>(
    user: &VerifiedUserState,
    puks: &'a [UserPrivateKey],
) -> Result<Vec<KvPrivateKeyRef<'a>>> {
    let current = puks
        .last()
        .ok_or(Error::KeyBinding("PUK sequence is empty"))?;
    let source = user_key_for_seed(user, &current.seed)?;
    if current.role != source.role || current.generation != source.generation {
        return Err(Error::KeyBinding(
            "current private PUK does not match verified public state",
        ));
    }
    for pair in puks.windows(2) {
        if pair[0].role != source.role
            || pair[1].role != source.role
            || pair[0].generation.checked_add(1) != Some(pair[1].generation)
        {
            return Err(Error::KeyBinding(
                "private PUK history is not one contiguous role generation sequence",
            ));
        }
    }
    Ok(puks
        .iter()
        .map(|key| KvPrivateKeyRef {
            role: key.role,
            generation: key.generation,
            seed: &key.seed,
        })
        .collect())
}

pub(crate) fn validate_kv_symlink(target: &[u8]) -> Result<()> {
    if target.is_empty()
        || target.len() > 4096
        || target.contains(&0)
        || std::str::from_utf8(target).is_err()
    {
        return Err(Error::KvResponse("invalid symlink target"));
    }
    Ok(())
}
