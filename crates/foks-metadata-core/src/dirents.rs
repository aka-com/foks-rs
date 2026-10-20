use crate::{integer, unsigned, Error, Result, Sql, Statement as S, Value as V, MAX_BLOB_BYTES};
use foks_proto::{KvDirent, KvPathVersionVector, Role};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub blob_bytes: usize,
    pub dirent_bytes: usize,
    pub namespace_bytes: u64,
    pub namespace_objects: u64,
    pub directories: u64,
    pub dirents: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub parent: [u8; 16],
    pub id: [u8; 16],
    pub version: u64,
    pub directory_version: u64,
    pub node_id: [u8; 17],
    pub name_mac: [u8; 32],
    pub creation_time: u64,
    pub exact: Vec<u8>,
}

fn blob(bytes: &[u8]) -> V {
    V::Blob(bytes.to_vec())
}

fn head(
    sql: &mut impl Sql,
    uid: &[u8],
    parent: &[u8; 16],
    id: &[u8; 16],
) -> Result<Option<(u64, Vec<u8>)>> {
    sql.query(S::Head, &[blob(uid), blob(parent), blob(id)])?
        .map(|row| {
            let [version, length, exact] = row.as_slice() else {
                return Err(Error::Storage);
            };
            let size = unsigned(length)?;
            if size > MAX_BLOB_BYTES as u64 {
                return Err(Error::Bounds);
            }
            let V::Blob(exact) = exact else {
                return Err(Error::Storage);
            };
            if size != exact.len() as u64 {
                return Err(Error::Storage);
            }
            Ok((unsigned(version)?, exact.clone()))
        })
        .transpose()
}

/// Server-side derived encoding, shared with the Wasm path. This runs inside
/// the same transaction as publication, not against a stale local projection.
pub fn normalize_creation_times(
    sql: &mut impl Sql,
    uid: &[u8],
    mutations: &mut [Mutation],
    now: u64,
) -> Result<()> {
    for mutation in mutations {
        let mut dirent =
            KvDirent::decode(&mutation.exact).map_err(|_| Error::Invalid("KV dirent"))?;
        if dirent.parent != mutation.parent
            || dirent.id != mutation.id
            || dirent.version != mutation.version
            || dirent.directory_version != mutation.directory_version
            || dirent.value.0 != mutation.node_id
            || dirent.name_mac != mutation.name_mac
        {
            return Err(Error::Invalid("KV dirent fields"));
        }
        let creation = head(sql, uid, &mutation.parent, &mutation.id)?
            .map(|(_, exact)| {
                KvDirent::decode(&exact).map_err(|_| Error::Invalid("stored KV dirent"))
            })
            .transpose()?
            .filter(|stored| stored.version == mutation.version)
            .map_or(now, |stored| stored.creation_time);
        dirent
            .set_creation_time(creation)
            .map_err(|_| Error::IntegerRange)?;
        mutation.creation_time = creation;
        mutation.exact = dirent
            .encode()
            .map_err(|_| Error::Invalid("KV dirent encoding"))?;
    }
    Ok(())
}

/// Extracted from Database::put_kv_dirents. The caller owns the outer
/// transaction. Even an exact replay must finish COMMIT before publication.
pub fn put_dirents(
    sql: &mut impl Sql,
    uid: &[u8],
    precondition: Option<&KvPathVersionVector>,
    caller_role: Role,
    mutations: &[Mutation],
    limits: &Limits,
) -> Result<()> {
    if uid.len() != 33 || mutations.is_empty() || mutations.len() > 64 {
        return Err(Error::Invalid("KV dirent mutation batch"));
    }
    let mut keys = std::collections::BTreeSet::new();
    for m in mutations {
        if m.version == 0
            || m.directory_version == 0
            || m.exact.is_empty()
            || m.exact.len() > limits.blob_bytes
            || m.exact.len() > limits.dirent_bytes
            || !keys.insert((m.parent, m.id))
        {
            return Err(Error::Invalid("KV dirent mutation"));
        }
        if m.exact.len() > MAX_BLOB_BYTES {
            return Err(Error::Bounds);
        }
    }
    let party = sql
        .query(S::NamespaceParty, &[blob(uid)])?
        .ok_or(Error::Invalid("unknown KV namespace"))?;
    let [kind, host] = party.as_slice() else {
        return Err(Error::Storage);
    };
    if sql.execute(S::NamespaceEnsure, &[blob(uid), kind.clone(), host.clone()])? != 1 {
        return Err(Error::Invalid("unknown KV namespace"));
    }
    let mut existing = Vec::with_capacity(mutations.len());
    for m in mutations {
        existing.push(head(sql, uid, &m.parent, &m.id)?);
    }
    // Deliberately before stale precondition and capacity tests, matching native replay.
    if existing.iter().zip(mutations).all(|(old, m)| {
        old.as_ref()
            .is_some_and(|(version, exact)| *version == m.version && *exact == m.exact)
    }) {
        return Ok(());
    }
    if let Some(condition) = precondition {
        check_versions(sql, uid, condition)?;
    }
    let added = mutations.iter().try_fold(0u64, |n, m| {
        n.checked_add(m.exact.len() as u64)
            .ok_or(Error::IntegerRange)
    })?;
    let usage = sql.query(S::Usage, &[blob(uid)])?.ok_or(Error::Storage)?;
    let [bytes, objects, directories, dirents] = usage.as_slice() else {
        return Err(Error::Storage);
    };
    let sum = |old: &V, delta: u64| unsigned(old)?.checked_add(delta).ok_or(Error::IntegerRange);
    if sum(bytes, added)? > limits.namespace_bytes
        || sum(objects, mutations.len() as u64)? > limits.namespace_objects
        || unsigned(directories)? > limits.directories
        || sum(
            dirents,
            existing.iter().filter(|v| v.is_none()).count() as u64,
        )? > limits.dirents
    {
        return Err(Error::Quota);
    }
    for (m, old) in mutations.iter().zip(existing) {
        if caller_role < Role::OWNER {
            if let Some((_, exact)) = &old {
                let stored =
                    KvDirent::decode(exact).map_err(|_| Error::Invalid("stored KV dirent"))?;
                if caller_role < stored.write_role {
                    return Err(Error::Permission);
                }
            }
        }
        if sql
            .query(
                S::DirectoryExists,
                &[blob(uid), blob(&m.parent), integer(m.directory_version)?],
            )?
            .is_none()
            || !node_exists(sql, uid, &m.node_id)?
        {
            return Err(Error::Conflict);
        }
        let expected = old.map_or(Ok(1), |(version, _)| {
            version.checked_add(1).ok_or(Error::IntegerRange)
        })?;
        if m.version != expected {
            return Err(Error::Conflict);
        }
        if m.node_id[0] != 0
            && sql
                .query(
                    S::NameConflict,
                    &[blob(uid), blob(&m.parent), blob(&m.id), blob(&m.name_mac)],
                )?
                .is_some()
        {
            return Err(Error::Conflict);
        }
        if sql.execute(
            S::InsertDirent,
            &[
                blob(uid),
                blob(&m.parent),
                blob(&m.id),
                integer(m.version)?,
                integer(m.directory_version)?,
                blob(&m.node_id),
                blob(&m.name_mac),
                integer(m.creation_time)?,
                blob(&m.exact),
            ],
        )? != 1
        {
            return Err(Error::Storage);
        }
        if sql.execute(
            S::SetHead,
            &[blob(uid), blob(&m.parent), blob(&m.id), integer(m.version)?],
        )? != 1
        {
            return Err(Error::Storage);
        }
    }
    Ok(())
}

fn node_exists(sql: &mut impl Sql, uid: &[u8], node: &[u8; 17]) -> Result<bool> {
    let result = match node[0] {
        0 => return Ok(node[1..].iter().all(|b| *b == 0)),
        1 => sql.query(S::DirectoryNode, &[blob(uid), blob(&node[1..])])?,
        2 => sql.query(S::FileNode, &[blob(uid), blob(&node[1..])])?,
        3 | 4 => sql.query(
            S::SmallNode,
            &[blob(uid), blob(node), V::Integer(i64::from(node[0]))],
        )?,
        _ => return Ok(false),
    };
    Ok(result.is_some())
}

fn version(sql: &mut impl Sql, statement: S, params: &[V]) -> Result<u64> {
    let row = sql.query(statement, params)?.ok_or(Error::Conflict)?;
    if row.len() != 1 {
        return Err(Error::Storage);
    }
    unsigned(&row[0])
}
fn check_versions(sql: &mut impl Sql, uid: &[u8], supplied: &KvPathVersionVector) -> Result<()> {
    // Feasibility profile bounds; broad hosted coverage must independently size
    // full wire vectors. This is not a new limit on the existing native server.
    if supplied.directories.len() > 256
        || supplied
            .directories
            .iter()
            .map(|d| d.entries.len())
            .sum::<usize>()
            > 1024
    {
        return Err(Error::Bounds);
    }
    let root = version(sql, S::RootVersion, &[blob(uid)])?;
    if root < supplied.root_version {
        return Err(Error::Conflict);
    }
    for d in &supplied.directories {
        if version(sql, S::DirectoryVersion, &[blob(uid), blob(&d.id)])? != d.version {
            return Err(Error::Conflict);
        }
        for entry in &d.entries {
            if version(
                sql,
                S::EntryVersion,
                &[blob(uid), blob(&d.id), blob(&entry.id)],
            )? != entry.version
            {
                return Err(Error::Conflict);
            }
        }
    }
    if supplied.root_version != 0 && root != supplied.root_version {
        return Err(Error::Conflict);
    }
    Ok(())
}
