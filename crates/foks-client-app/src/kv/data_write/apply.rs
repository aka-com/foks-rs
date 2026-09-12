//! Bounded namespace mutations within one already-prepared adapter intent.
use super::*;

pub(super) fn apply_write<R: Read>(
    session: &mut foks_client::KvWriteSession<'_>,
    spec: &DataWriteSpec,
    body: &mut R,
    team: bool,
    remote_possible: &mut bool,
) -> Result<Option<String>> {
    let (read_role, write_role) = if team {
        (Role::member(-0x4000), Role::ADMIN)
    } else {
        (Role::OWNER, Role::OWNER)
    };
    let mut tree = session.sync()?;
    if tree.is_empty() && matches!(spec.kind, DataWriteKind::Put | DataWriteKind::Mkdir) {
        *remote_possible = true;
        tree = session.initialize_root(read_role, write_role)?;
    }
    if tree
        .iter()
        .map(|directory| directory.entries.len())
        .sum::<usize>()
        > 1000
    {
        return Err(Error::InvalidAccount("namespace exceeds adapter limit"));
    }
    let (parent_path, name) = split_parent(&spec.path)?;
    if spec.mkdir_p && resolve_directory(&tree, &parent_path).is_err() {
        if tree
            .iter()
            .map(|directory| directory.entries.len())
            .sum::<usize>()
            + path_components(&parent_path)?.len()
            > 1000
        {
            return Err(Error::InvalidKvPath(
                "mkdir-p would exceed adapter namespace limit",
            ));
        }
        *remote_possible = true;
    }
    let (parent, tree) = resolve_write_parent(
        session,
        tree,
        &parent_path,
        spec.mkdir_p,
        KvRoleSummary::from_role(read_role)?,
        KvRoleSummary::from_role(write_role)?,
    )?;
    let directory = tree
        .iter()
        .find(|directory| directory.directory_id == parent)
        .ok_or(Error::InvalidKvPath("missing parent"))?;
    let pair = foks_proto::KvDirectoryPair::decode(&directory.directory_bytes)?;
    let decoded = pair.encrypting.as_ref().unwrap_or(&pair.active);
    let existing = directory
        .entries
        .iter()
        .find(|entry| entry.name == name.as_bytes());
    let options = KvWriteOptions {
        read_role: decoded.key.role,
        write_role: decoded.write_role,
        overwrite: spec.overwrite,
        expected_version: if spec.overwrite {
            Some(existing.map_or(0, |entry| entry.version))
        } else {
            None
        },
    };
    if matches!(spec.kind, DataWriteKind::Put | DataWriteKind::Mkdir)
        && existing.is_some()
        && !spec.overwrite
        && !(spec.kind == DataWriteKind::Mkdir
            && spec.mkdir_p
            && existing.is_some_and(|entry| entry.node_id[0] == KvNodeType::Directory as u8))
    {
        return Err(Error::KvConflict);
    }
    session.mark_adapter_completion()?;
    match spec.kind {
        DataWriteKind::Put => {
            *remote_possible = true;
            Ok(Some(hex(&session
                .put_file(parent, &name, body, options)?
                .node_id
                .0)))
        }
        DataWriteKind::Mkdir => {
            if spec.mkdir_p
                && existing.is_some_and(|entry| entry.node_id[0] == KvNodeType::Directory as u8)
            {
                return Ok(existing.map(|entry| hex(&entry.node_id)));
            }
            *remote_possible = true;
            Ok(Some(hex(&session.mkdir(parent, &name, options)?.node_id.0)))
        }
        DataWriteKind::Remove => {
            let existing = existing.ok_or(Error::InvalidKvPath("entry does not exist"))?;
            if existing.node_id[0] == KvNodeType::Directory as u8 && !spec.recursive {
                return Err(Error::InvalidKvPath(
                    "recursive is required for directory removal",
                ));
            }
            *remote_possible = true;
            session.unlink(
                parent,
                &name,
                Some(existing.version),
                projected_write_role(existing)?,
                spec.recursive,
            )?;
            Ok(None)
        }
        DataWriteKind::Move => {
            let existing = existing.ok_or(Error::InvalidKvPath("move source does not exist"))?;
            let (destination, destination_name) = split_parent(
                spec.destination
                    .as_deref()
                    .ok_or(Error::InvalidKvPath("missing destination"))?,
            )?;
            let destination = resolve_directory(&tree, &destination)?;
            if tree
                .iter()
                .find(|dir| dir.directory_id == destination)
                .is_some_and(|dir| {
                    dir.entries
                        .iter()
                        .any(|entry| entry.name == destination_name.as_bytes())
                })
            {
                return Err(Error::KvConflict);
            }
            *remote_possible = true;
            let node = session.move_entry_checked(
                parent,
                &name,
                Some(existing.version),
                destination,
                &destination_name,
                options,
            )?;
            Ok(Some(hex(&node.node_id.0)))
        }
    }
}
