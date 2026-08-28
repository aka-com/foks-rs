use super::*;

impl CheckedProfileSession<'_> {
    pub fn list_kv(&self, alias: &str, vault: &mut AccountVault<'_>) -> Result<KvListReport> {
        self.profile.require(Capability::Kv)?;
        let (_, authenticated, directories) = self.authenticated_tree(alias, vault)?;
        Ok(KvListReport {
            sync: SyncReport::from_tree(
                authenticated.verified.username(),
                authenticated.verified.chain_seqno(),
                &directories,
            ),
            entries: flatten_tree(&directories)?,
        })
    }

    pub fn read_kv_file(
        &self,
        alias: &str,
        path: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<u8>> {
        let mut output = Vec::new();
        self.write_kv_file(alias, path, vault, &mut output)?;
        Ok(output)
    }

    /// Streams one authenticated KV file to the caller without buffering a
    /// large-file projection in application memory.
    pub fn write_kv_file<W: Write>(
        &self,
        alias: &str,
        path: &str,
        vault: &mut AccountVault<'_>,
        writer: &mut W,
    ) -> Result<u64> {
        self.profile.require(Capability::Kv)?;
        let (loaded, _, directories) = self.authenticated_tree(alias, vault)?;
        let entry = resolve_entry(&directories, path)?;
        match KvNodeId(entry.node_id).node_type()? {
            KvNodeType::SmallFile => {
                let content = entry.content.as_ref().ok_or(Error::InvalidAccount(
                    "small-file projection has no content",
                ))?;
                writer.write_all(content)?;
                u64::try_from(content.len())
                    .map_err(|_| Error::InvalidAccount("small-file size overflow"))
            }
            KvNodeType::File => {
                let host = self.pinned_host()?;
                let store = SoftStateStore::open(&self.paths.soft_database)?;
                store
                    .write_large_file(
                        host.host_id().as_bytes(),
                        loaded.credential.uid.as_bytes(),
                        &entry.node_id,
                        writer,
                    )?
                    .ok_or(Error::InvalidAccount("large-file projection is incomplete"))
            }
            _ => Err(Error::InvalidAccount("KV path is not a file")),
        }
    }

    pub fn put_kv_file<R: Read>(
        &self,
        alias: &str,
        path: &str,
        reader: &mut R,
        overwrite: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.profile.require(Capability::Kv)?;
        let (parent_path, name) = split_parent(path)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut session = self.client.user_kv_write_session(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        let tree = session.sync()?;
        let parent = resolve_directory(&tree, &parent_path)?;
        let result = session.put_file(
            parent,
            &name,
            reader,
            KvWriteOptions {
                read_role: Role::OWNER,
                write_role: Role::OWNER,
                overwrite,
                expected_version: None,
            },
        )?;
        Ok(KvWriteReport::from_tree(
            path,
            result.dirent_version,
            &result.tree,
        ))
    }

    pub fn mkdir_kv(
        &self,
        alias: &str,
        path: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.profile.require(Capability::Kv)?;
        let (parent_path, name) = split_parent(path)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut session = self.client.user_kv_write_session(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        let tree = session.sync()?;
        let parent = resolve_directory(&tree, &parent_path)?;
        let result = session.mkdir(
            parent,
            &name,
            KvWriteOptions {
                read_role: Role::OWNER,
                write_role: Role::OWNER,
                overwrite: false,
                expected_version: None,
            },
        )?;
        Ok(KvWriteReport::from_tree(
            path,
            result.dirent_version,
            &result.tree,
        ))
    }

    pub fn remove_kv(
        &self,
        alias: &str,
        path: &str,
        recursive: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.profile.require(Capability::Kv)?;
        let (parent_path, name) = split_parent(path)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut session = self.client.user_kv_write_session(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        let tree = session.sync()?;
        let parent = resolve_directory(&tree, &parent_path)?;
        let existing = resolve_entry(&tree, path)?;
        let version = existing
            .version
            .checked_add(1)
            .ok_or(Error::InvalidAccount("KV version overflow"))?;
        let tree = session.unlink(
            parent,
            &name,
            Some(existing.version),
            Role::OWNER,
            recursive,
        )?;
        Ok(KvWriteReport::from_tree(path, version, &tree))
    }

    pub(super) fn authenticated_tree(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<(
        LoadedAccount,
        foks_client::AuthenticatedUserOutcome,
        Vec<KvDirectoryProjection>,
    )> {
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self
            .client
            .authenticate_and_pin(&host, &loaded.credential)?;
        let directories = self.client.sync_user_kv(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
        )?;
        Ok((loaded, authenticated, directories))
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvEntrySummary {
    pub path: String,
    pub node_type: String,
    pub version: u64,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvListReport {
    pub sync: SyncReport,
    pub entries: Vec<KvEntrySummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvWriteReport {
    pub path: String,
    pub version: u64,
    pub directories: usize,
    pub entries: usize,
}

impl KvWriteReport {
    fn from_tree(path: &str, version: u64, tree: &[KvDirectoryProjection]) -> Self {
        Self {
            path: path.to_owned(),
            version,
            directories: tree.len(),
            entries: tree.iter().map(|directory| directory.entries.len()).sum(),
        }
    }
}
fn path_components(path: &str) -> Result<Vec<&str>> {
    if !path.starts_with('/') || path.len() > 4096 {
        return Err(Error::InvalidKvPath(
            "path must be absolute and at most 4096 bytes",
        ));
    }
    if path == "/" {
        return Ok(Vec::new());
    }
    let components = path[1..].split('/').collect::<Vec<_>>();
    if components.iter().any(|component| {
        component.is_empty() || *component == "." || *component == ".." || component.len() > 255
    }) {
        return Err(Error::InvalidKvPath(
            "path contains an empty, relative, or excessive component",
        ));
    }
    Ok(components)
}

pub(super) fn split_parent(path: &str) -> Result<(String, String)> {
    let components = path_components(path)?;
    let (name, parents) = components.split_last().ok_or(Error::InvalidKvPath(
        "the KV root cannot be mutated as an entry",
    ))?;
    let parent = if parents.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", parents.join("/"))
    };
    Ok((parent, (*name).to_owned()))
}

fn resolve_directory(tree: &[KvDirectoryProjection], path: &str) -> Result<[u8; 16]> {
    let components = path_components(path)?;
    let root = tree
        .first()
        .ok_or(Error::InvalidKvPath("KV projection has no root"))?
        .root_directory_id;
    let mut current = root;
    for component in components {
        let directory = tree
            .iter()
            .find(|directory| directory.directory_id == current)
            .ok_or(Error::InvalidKvPath(
                "parent directory is absent from the projection",
            ))?;
        let entry = directory
            .entries
            .iter()
            .find(|entry| entry.name == component.as_bytes())
            .ok_or(Error::InvalidKvPath("directory component does not exist"))?;
        let node = KvNodeId(entry.node_id);
        if node.node_type()? != KvNodeType::Directory {
            return Err(Error::InvalidKvPath("path component is not a directory"));
        }
        current = node.object_id();
    }
    Ok(current)
}

fn resolve_entry<'a>(
    tree: &'a [KvDirectoryProjection],
    path: &str,
) -> Result<&'a foks_client_db::KvProjectedEntry> {
    let (parent_path, name) = split_parent(path)?;
    let parent = resolve_directory(tree, &parent_path)?;
    tree.iter()
        .find(|directory| directory.directory_id == parent)
        .and_then(|directory| {
            directory
                .entries
                .iter()
                .find(|entry| entry.name == name.as_bytes())
        })
        .ok_or(Error::InvalidKvPath("KV entry does not exist"))
}

fn flatten_tree(tree: &[KvDirectoryProjection]) -> Result<Vec<KvEntrySummary>> {
    use std::collections::{BTreeSet, VecDeque};

    let root = tree
        .first()
        .ok_or(Error::InvalidKvPath("KV projection has no root"))?
        .root_directory_id;
    let mut pending = VecDeque::from([(root, String::new())]);
    let mut visited = BTreeSet::new();
    let mut output = Vec::new();
    while let Some((directory_id, parent_path)) = pending.pop_front() {
        if !visited.insert(directory_id) {
            return Err(Error::InvalidKvPath(
                "KV projection contains a directory cycle",
            ));
        }
        let directory = tree
            .iter()
            .find(|directory| directory.directory_id == directory_id)
            .ok_or(Error::InvalidKvPath(
                "KV projection omits a reachable directory",
            ))?;
        let mut entries = directory.entries.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        for entry in entries {
            let name = display_component(&entry.name);
            let path = format!("{parent_path}/{name}");
            let node_type = KvNodeId(entry.node_id).node_type()?;
            let size = match node_type {
                KvNodeType::SmallFile => entry.content.as_ref().map(|content| content.len() as u64),
                KvNodeType::File => entry.large_file_size,
                KvNodeType::Symlink => entry.symlink.as_ref().map(|target| target.len() as u64),
                _ => None,
            };
            output.push(KvEntrySummary {
                path: path.clone(),
                node_type: match node_type {
                    KvNodeType::None => "none",
                    KvNodeType::Directory => "directory",
                    KvNodeType::File => "file",
                    KvNodeType::SmallFile => "small-file",
                    KvNodeType::Symlink => "symlink",
                }
                .to_owned(),
                version: entry.version,
                size,
            });
            if node_type == KvNodeType::Directory {
                pending.push_back((KvNodeId(entry.node_id).object_id(), path));
            }
        }
    }
    Ok(output)
}

pub(super) fn display_component(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write as _;
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            output.push(char::from(*byte));
        } else {
            write!(&mut output, "%{byte:02X}").expect("writing to a String cannot fail");
        }
        output
    })
}
