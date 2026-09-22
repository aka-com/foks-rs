use super::*;

mod data;
mod data_stat;
pub use data_stat::*;
mod data_write;
pub use data::*;
pub use data_write::*;

fn record_completed_chunk_size(
    soft_database: &Path,
    host_id: &[u8],
    party_id: &[u8],
    node_id: &[u8; 17],
    offset: u64,
    content_len: usize,
    eof: bool,
) -> Result<()> {
    if !eof {
        return Ok(());
    }
    let size = offset
        .checked_add(
            u64::try_from(content_len)
                .map_err(|_| Error::InvalidAccount("large-file size overflow"))?,
        )
        .ok_or(Error::InvalidAccount("large-file size overflow"))?;
    foks_client_db::SoftStateStore::open(soft_database)?
        .record_large_file_size(host_id, party_id, node_id, size)?;
    Ok(())
}

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

    /// Returns the live, authenticated metadata needed by the desktop
    /// catalog. File payloads are deliberately neither downloaded nor staged.
    pub fn list_kv_metadata(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<KvCatalogReport> {
        self.profile.require(Capability::Kv)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self.authenticated_user(&host, &loaded.credential)?;
        let directories = self.client.list_user_kv_metadata(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
        )?;
        KvCatalogReport::from_tree(&directories)
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

    pub fn read_kv_entry(
        &self,
        alias: &str,
        path: &str,
        version: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<KvReadReport> {
        self.profile.require(Capability::Kv)?;
        let (loaded, authenticated, directories) = self.resolved_user_path(alias, path, vault)?;
        let entry = checked_entry(&directories, path, version)?;
        let host = self.pinned_host()?;
        let fetched = self.client.read_user_kv_node(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            KvNodeId(entry.node_id),
        )?;
        read_report_from_fetched(&directories, path, version, fetched)
    }

    /// Reads one range of one large file.
    ///
    /// A download issues this once per delivered chunk, and resolving the
    /// path costs a request per directory on it every time. A session with a
    /// node cache resolves the path once and then reads under the node it
    /// remembered, submitting the version vector that walk cited so the
    /// server refuses the read if anything on the path moved. That is one
    /// request in place of the walk, not zero: a remembered `(path, version)`
    /// pair is not by itself evidence about a node, because a dirent version
    /// restarts at 1 after an unlink and a re-create on a fresh dirent
    /// identifier.
    pub fn read_kv_chunk(
        &self,
        alias: &str,
        path: &str,
        version: u64,
        offset: u64,
        length: usize,
        vault: &mut AccountVault<'_>,
    ) -> Result<KvChunkReport> {
        self.profile.require(Capability::Kv)?;
        let components = parent_components(path)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self.authenticated_user(&host, &loaded.credential)?;
        let cache = self.kv_node_cache_entry(
            &host,
            &loaded.credential,
            &loaded.credential.uid,
            path,
            version,
        )?;
        if let Some((cache, key)) = &cache {
            if let Some(remembered) = cache.get(key) {
                match self.client.read_user_kv_chunk_if_current(
                    &host,
                    &loaded.credential,
                    &authenticated.verified,
                    &authenticated.puks,
                    KvNodeId(remembered.node_id),
                    offset,
                    length,
                    &remembered.versions,
                ) {
                    Ok(Some(chunk)) => {
                        record_completed_chunk_size(
                            &self.paths.soft_database,
                            host.host_id().as_bytes(),
                            loaded.credential.uid.as_bytes(),
                            &remembered.node_id,
                            offset,
                            chunk.content.len(),
                            chunk.eof,
                        )?;
                        return Ok(KvChunkReport {
                            path: path.to_owned(),
                            version,
                            offset,
                            eof: chunk.eof,
                            content: chunk.content,
                        });
                    }
                    // The server refused the vector. The remembered node is
                    // not the node at this path any more, or may not be;
                    // either way the path is walked again below, which is
                    // what decides whether this version still resolves.
                    Ok(None) => cache.invalidate(key),
                    Err(error) => {
                        cache.invalidate(key);
                        return Err(error.into());
                    }
                }
            }
        }
        let directories = self.client.resolve_user_kv_path(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
            &components,
        )?;
        let entry = checked_entry(&directories, path, version)?;
        if KvNodeId(entry.node_id).node_type()? != KvNodeType::File {
            return Err(Error::InvalidAccount("KV chunk path is not a large file"));
        }
        let chunk = self.client.read_user_kv_chunk(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            KvNodeId(entry.node_id),
            offset,
            length,
        )?;
        record_completed_chunk_size(
            &self.paths.soft_database,
            &directories[0].host_id,
            &directories[0].party_id,
            &entry.node_id,
            offset,
            chunk.content.len(),
            chunk.eof,
        )?;
        if let Some((cache, key)) = cache {
            cache.put(
                key,
                crate::KvNodeCacheEntry {
                    node_id: entry.node_id,
                    versions: foks_client::kv_path_version_vector(&directories),
                },
            );
        }
        Ok(KvChunkReport {
            path: path.to_owned(),
            version,
            offset,
            eof: chunk.eof,
            content: chunk.content,
        })
    }

    pub fn read_team_kv_entry(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        version: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<KvReadReport> {
        let (account, team, directories) =
            self.resolved_team_path(account_alias, team_alias, team_id_hex, path, vault)?;
        let entry = checked_entry(&directories, path, version)?;
        let host = self.pinned_host()?;
        let fetched = self.client.read_team_kv_node(
            &host,
            &account.credential,
            &team,
            KvNodeId(entry.node_id),
        )?;
        read_report_from_fetched(&directories, path, version, fetched)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn read_team_kv_chunk(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        version: u64,
        offset: u64,
        length: usize,
        vault: &mut AccountVault<'_>,
    ) -> Result<KvChunkReport> {
        let components = parent_components(path)?;
        let (account, team, host) =
            self.authenticated_team_store(account_alias, team_alias, team_id_hex, vault)?;
        let cache = self.kv_node_cache_entry(
            &host,
            &account.credential,
            team.verified.team(),
            path,
            version,
        )?;
        if let Some((cache, key)) = &cache {
            if let Some(remembered) = cache.get(key) {
                // As on [`Self::read_kv_chunk`]: the remembered node is used
                // only if the server still accepts the walk's version vector.
                match self.client.read_team_kv_chunk_if_current(
                    &host,
                    &account.credential,
                    &team,
                    KvNodeId(remembered.node_id),
                    offset,
                    length,
                    &remembered.versions,
                ) {
                    Ok(Some(chunk)) => {
                        record_completed_chunk_size(
                            &self.paths.soft_database,
                            host.host_id().as_bytes(),
                            team.verified.team().as_bytes(),
                            &remembered.node_id,
                            offset,
                            chunk.content.len(),
                            chunk.eof,
                        )?;
                        return Ok(KvChunkReport {
                            path: path.to_owned(),
                            version,
                            offset,
                            eof: chunk.eof,
                            content: chunk.content,
                        });
                    }
                    Ok(None) => cache.invalidate(key),
                    Err(error) => {
                        cache.invalidate(key);
                        return Err(error.into());
                    }
                }
            }
        }
        let directories = self.client.resolve_team_kv_path(
            &host,
            &account.credential,
            &team,
            &self.paths.soft_database,
            &components,
        )?;
        let entry = checked_entry(&directories, path, version)?;
        if KvNodeId(entry.node_id).node_type()? != KvNodeType::File {
            return Err(Error::InvalidAccount("KV chunk path is not a large file"));
        }
        let chunk = self.client.read_team_kv_chunk(
            &host,
            &account.credential,
            &team,
            KvNodeId(entry.node_id),
            offset,
            length,
        )?;
        record_completed_chunk_size(
            &self.paths.soft_database,
            &directories[0].host_id,
            &directories[0].party_id,
            &entry.node_id,
            offset,
            chunk.content.len(),
            chunk.eof,
        )?;
        if let Some((cache, key)) = cache {
            cache.put(
                key,
                crate::KvNodeCacheEntry {
                    node_id: entry.node_id,
                    versions: foks_client::kv_path_version_vector(&directories),
                },
            );
        }
        Ok(KvChunkReport {
            path: path.to_owned(),
            version,
            offset,
            eof: chunk.eof,
            content: chunk.content,
        })
    }

    fn authenticated_team_store(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<(
        LoadedAccount,
        AuthenticatedTeamOutcome,
        foks_client::PinnedHost,
    )> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Kv)?;
        let stored = vault.team(team_alias)?;
        if !stored.active
            || stored.account_alias != account_alias
            || hex(&stored.team_id) != team_id_hex
        {
            return Err(Error::InvalidAccount(
                "team store reference is not active or bound",
            ));
        }
        let account = vault.account(account_alias)?;
        let host = self.pinned_host()?;
        let user = self.authenticated_user(&host, &account.credential)?;
        let team_id = EntityId::from_bytes(stored.team_id.clone())?;
        let team = self.load_team_for_read(&host, &account.credential, &user, &team_id)?;
        Ok((account, team, host))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_kv_file_checked<R: Read>(
        &self,
        alias: &str,
        path: &str,
        reader: &mut R,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.put_kv_file_checked_with_size(
            alias,
            path,
            reader,
            None,
            precondition,
            read_role,
            write_role,
            mkdir_p,
            vault,
            master_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_kv_file_checked_with_size<R: Read>(
        &self,
        alias: &str,
        path: &str,
        reader: &mut R,
        expected_size: Option<u64>,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.put_kv_node_checked(
            alias,
            path,
            precondition,
            read_role,
            write_role,
            mkdir_p,
            vault,
            master_key,
            |session, parent, name, options| {
                session.put_file_with_size(parent, name, reader, expected_size, options)
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_team_kv_file_checked<R: Read>(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        reader: &mut R,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.put_team_kv_file_checked_with_size(
            account_alias,
            team_alias,
            team_id_hex,
            path,
            reader,
            None,
            precondition,
            read_role,
            write_role,
            mkdir_p,
            vault,
            master_key,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_team_kv_file_checked_with_size<R: Read>(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        reader: &mut R,
        expected_size: Option<u64>,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.put_team_kv_node_checked(
            account_alias,
            team_alias,
            team_id_hex,
            path,
            precondition,
            read_role,
            write_role,
            mkdir_p,
            vault,
            master_key,
            |session, parent, name, options| {
                session.put_file_with_size(parent, name, reader, expected_size, options)
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_kv_symlink_checked(
        &self,
        alias: &str,
        path: &str,
        target: &str,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.put_kv_node_checked(
            alias,
            path,
            precondition,
            read_role,
            write_role,
            mkdir_p,
            vault,
            master_key,
            |session, parent, name, options| session.put_symlink(parent, name, target, options),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_team_kv_symlink_checked(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        target: &str,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        self.put_team_kv_node_checked(
            account_alias,
            team_alias,
            team_id_hex,
            path,
            precondition,
            read_role,
            write_role,
            mkdir_p,
            vault,
            master_key,
            |session, parent, name, options| session.put_symlink(parent, name, target, options),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn put_kv_node_checked(
        &self,
        alias: &str,
        path: &str,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
        put: impl FnOnce(
            &mut foks_client::KvWriteSession<'_>,
            [u8; 16],
            &str,
            KvWriteOptions,
        ) -> foks_client::Result<foks_client::KvWriteResult>,
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
        let (parent, tree) = resolve_write_parent(
            &mut session,
            &parent_path,
            mkdir_p,
            read_role,
            write_role,
            matches!(precondition, KvMutationPrecondition::Create)
                .then_some((Role::OWNER, Role::OWNER)),
        )?;
        verify_precondition(&tree, path, precondition)?;
        let result = put(
            &mut session,
            parent,
            &name,
            KvWriteOptions {
                read_role: read_role.to_role(),
                write_role: write_role.to_role(),
                overwrite: matches!(precondition, KvMutationPrecondition::ExactVersion(_)),
                expected_version: precondition.version(),
            },
        )
        .map_err(checked_write_error)?;
        Ok(KvWriteReport::from_path(
            path,
            result.dirent_version,
            &result.path,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn put_team_kv_node_checked(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
        put: impl FnOnce(
            &mut foks_client::KvWriteSession<'_>,
            [u8; 16],
            &str,
            KvWriteOptions,
        ) -> foks_client::Result<foks_client::KvWriteResult>,
    ) -> Result<KvWriteReport> {
        let (parent_path, name) = split_parent(path)?;
        self.with_team_kv_write_session(
            account_alias,
            team_alias,
            team_id_hex,
            vault,
            master_key,
            |session| {
                let (parent, tree) = resolve_write_parent(
                    session,
                    &parent_path,
                    mkdir_p,
                    read_role,
                    write_role,
                    None,
                )?;
                verify_precondition(&tree, path, precondition)?;
                let result = put(
                    session,
                    parent,
                    &name,
                    KvWriteOptions {
                        read_role: read_role.to_role(),
                        write_role: write_role.to_role(),
                        overwrite: matches!(precondition, KvMutationPrecondition::ExactVersion(_)),
                        expected_version: precondition.version(),
                    },
                )
                .map_err(checked_write_error)?;
                Ok(KvWriteReport::from_path(
                    path,
                    result.dirent_version,
                    &result.path,
                ))
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_team_kv_write_session<T>(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
        run: impl FnOnce(&mut foks_client::KvWriteSession<'_>) -> Result<T>,
    ) -> Result<T> {
        self.profile.require(Capability::Teams)?;
        self.profile.require(Capability::Kv)?;
        let stored = vault.team(team_alias)?;
        if !stored.active
            || stored.account_alias != account_alias
            || hex(&stored.team_id) != team_id_hex
        {
            return Err(Error::InvalidAccount(
                "team store reference is not active or bound",
            ));
        }
        let account = vault.account(account_alias)?;
        let host = self.pinned_host()?;
        let user = self
            .client
            .authenticate_and_pin(&host, &account.credential)?;
        let team_id = EntityId::from_bytes(stored.team_id.clone())?;
        let team = self.client.load_and_pin_team(
            &host,
            &account.credential,
            &user.verified,
            &user.puks,
            &team_id,
        )?;
        let mut mutations = EncryptedFileMutationStore::open(
            &self.paths.protected_mutations,
            derive_mutation_key(master_key),
        )?;
        let mut session = self.client.team_kv_write_session(
            &host,
            &account.credential,
            &team,
            &self.paths.soft_database,
            &mut mutations,
        )?;
        run(&mut session)
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
        let (loaded, authenticated, directories) = self.resolved_user_path(alias, path, vault)?;
        let entry = resolve_entry(&directories, path)?;
        match KvNodeId(entry.node_id).node_type()? {
            KvNodeType::SmallFile => {
                let host = self.pinned_host()?;
                let foks_client::KvFetchedNode::SmallFile(content) =
                    self.client.read_user_kv_node(
                        &host,
                        &loaded.credential,
                        &authenticated.verified,
                        &authenticated.puks,
                        KvNodeId(entry.node_id),
                    )?
                else {
                    return Err(Error::InvalidAccount("small-file node type changed"));
                };
                writer.write_all(&content)?;
                u64::try_from(content.len())
                    .map_err(|_| Error::InvalidAccount("small-file size overflow"))
            }
            KvNodeType::File => {
                let host = self.pinned_host()?;
                let mut offset = 0u64;
                loop {
                    let chunk = self.client.read_user_kv_chunk(
                        &host,
                        &loaded.credential,
                        &authenticated.verified,
                        &authenticated.puks,
                        KvNodeId(entry.node_id),
                        offset,
                        128 * 1024,
                    )?;
                    writer.write_all(&chunk.content)?;
                    offset = offset
                        .checked_add(chunk.content.len() as u64)
                        .ok_or(Error::InvalidAccount("large-file size overflow"))?;
                    if chunk.eof {
                        let mut soft =
                            foks_client_db::SoftStateStore::open(&self.paths.soft_database)?;
                        soft.record_large_file_size(
                            &directories[0].host_id,
                            &directories[0].party_id,
                            &entry.node_id,
                            offset,
                        )?;
                        return Ok(offset);
                    }
                    if chunk.content.is_empty() {
                        return Err(Error::InvalidAccount("large-file read made no progress"));
                    }
                }
            }
            _ => Err(Error::InvalidAccount("KV path is not a file")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn put_kv_file<R: Read>(
        &self,
        alias: &str,
        path: &str,
        reader: &mut R,
        overwrite: bool,
        mkdir_p: bool,
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
        let (parent, _tree) = resolve_write_parent(
            &mut session,
            &parent_path,
            mkdir_p,
            KvRoleSummary::Owner,
            KvRoleSummary::Owner,
            Some((Role::OWNER, Role::OWNER)),
        )?;
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
        Ok(KvWriteReport::from_path(
            path,
            result.dirent_version,
            &result.path,
        ))
    }

    pub fn mkdir_kv(
        &self,
        alias: &str,
        path: &str,
        mkdir_p: bool,
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
        let (parent, _tree) = resolve_write_parent(
            &mut session,
            &parent_path,
            mkdir_p,
            KvRoleSummary::Owner,
            KvRoleSummary::Owner,
            Some((Role::OWNER, Role::OWNER)),
        )?;
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
        Ok(KvWriteReport::from_path(
            path,
            result.dirent_version,
            &result.path,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mkdir_kv_checked(
        &self,
        alias: &str,
        path: &str,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        if precondition != KvMutationPrecondition::Create {
            return Err(Error::InvalidConfig(
                "directory creation requires a create precondition",
            ));
        }
        self.put_kv_node_checked(
            alias,
            path,
            precondition,
            read_role,
            write_role,
            mkdir_p,
            vault,
            master_key,
            |session, parent, name, options| session.mkdir(parent, name, options),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mkdir_team_kv_checked(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        precondition: KvMutationPrecondition,
        read_role: KvRoleSummary,
        write_role: KvRoleSummary,
        mkdir_p: bool,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        if precondition != KvMutationPrecondition::Create {
            return Err(Error::InvalidConfig(
                "directory creation requires a create precondition",
            ));
        }
        self.put_team_kv_node_checked(
            account_alias,
            team_alias,
            team_id_hex,
            path,
            precondition,
            read_role,
            write_role,
            mkdir_p,
            vault,
            master_key,
            |session, parent, name, options| session.mkdir(parent, name, options),
        )
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
        let tree = session.resolve_path(&path_components(&parent_path)?)?;
        let parent = resolve_directory(&tree, &parent_path)?;
        let existing = resolve_entry(&tree, path)?;
        let version = existing
            .version
            .checked_add(1)
            .ok_or(Error::InvalidAccount("KV version overflow"))?;
        let tree = session
            .unlink(
                parent,
                &name,
                Some(existing.version),
                Role::OWNER,
                recursive,
            )
            .map_err(checked_write_error)?;
        Ok(KvWriteReport::from_path(path, version, &tree))
    }

    pub fn remove_kv_checked(
        &self,
        alias: &str,
        path: &str,
        recursive: bool,
        expected_version: u64,
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
        let tree = session.resolve_path(&path_components(&parent_path)?)?;
        let parent = resolve_directory(&tree, &parent_path)?;
        let existing = checked_entry(&tree, path, expected_version)?;
        let write_role = projected_write_role(existing)?;
        let next_version = expected_version
            .checked_add(1)
            .ok_or(Error::InvalidAccount("KV version overflow"))?;
        let tree = session
            .unlink(parent, &name, Some(expected_version), write_role, recursive)
            .map_err(checked_write_error)?;
        Ok(KvWriteReport::from_path(path, next_version, &tree))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn remove_team_kv_checked(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        recursive: bool,
        expected_version: u64,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<KvWriteReport> {
        let (parent_path, name) = split_parent(path)?;
        self.with_team_kv_write_session(
            account_alias,
            team_alias,
            team_id_hex,
            vault,
            master_key,
            |session| {
                let tree = session.resolve_path(&path_components(&parent_path)?)?;
                let parent = resolve_directory(&tree, &parent_path)?;
                let existing = checked_entry(&tree, path, expected_version)?;
                let write_role = projected_write_role(existing)?;
                let next_version = expected_version
                    .checked_add(1)
                    .ok_or(Error::InvalidAccount("KV version overflow"))?;
                let tree = session
                    .unlink(parent, &name, Some(expected_version), write_role, recursive)
                    .map_err(checked_write_error)?;
                Ok(KvWriteReport::from_path(path, next_version, &tree))
            },
        )
    }

    pub(super) fn authenticated_tree(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<(
        LoadedAccount,
        std::sync::Arc<foks_client::AuthenticatedUserOutcome>,
        Vec<KvDirectoryProjection>,
    )> {
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self.authenticated_user(&host, &loaded.credential)?;
        let directories = self.client.sync_user_kv(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
        )?;
        Ok((loaded, authenticated, directories))
    }

    /// Authenticates only the directories `path` walks through, instead of
    /// every directory in the store. The projections it returns read exactly
    /// like a complete traversal restricted to that path, so the ordinary
    /// path helpers report the same absent, unreadable and conflicting
    /// outcomes they report against a whole-namespace projection.
    fn resolved_user_path(
        &self,
        alias: &str,
        path: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<(
        LoadedAccount,
        std::sync::Arc<foks_client::AuthenticatedUserOutcome>,
        Vec<KvDirectoryProjection>,
    )> {
        self.profile.require(Capability::Kv)?;
        let components = parent_components(path)?;
        let loaded = vault.account(alias)?;
        let host = self.pinned_host()?;
        let authenticated = self.authenticated_user(&host, &loaded.credential)?;
        let directories = self.client.resolve_user_kv_path(
            &host,
            &loaded.credential,
            &authenticated.verified,
            &authenticated.puks,
            &self.paths.soft_database,
            &components,
        )?;
        Ok((loaded, authenticated, directories))
    }

    /// Team variant of [`Self::resolved_user_path`].
    fn resolved_team_path(
        &self,
        account_alias: &str,
        team_alias: &str,
        team_id_hex: &str,
        path: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<(
        LoadedAccount,
        AuthenticatedTeamOutcome,
        Vec<KvDirectoryProjection>,
    )> {
        let components = parent_components(path)?;
        let (account, team, host) =
            self.authenticated_team_store(account_alias, team_alias, team_id_hex, vault)?;
        let directories = self.client.resolve_team_kv_path(
            &host,
            &account.credential,
            &team,
            &self.paths.soft_database,
            &components,
        )?;
        Ok((account, team, directories))
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

/// The outcome of one KV mutation. A write reads only the directories along
/// its own path, so the counts describe that path and not the whole store.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvWriteReport {
    pub path: String,
    pub version: u64,
    pub path_directories: usize,
    pub path_entries: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvMutationPrecondition {
    Create,
    ExactVersion(u64),
}

impl KvMutationPrecondition {
    fn version(self) -> Option<u64> {
        match self {
            Self::Create => None,
            Self::ExactVersion(version) => Some(version),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KvRoleSummary {
    Member { visibility: i16 },
    Admin,
    Owner,
}

impl KvRoleSummary {
    fn from_role(role: Role) -> Result<Self> {
        match role.kind() {
            foks_proto::RoleType::Member => Ok(Self::Member {
                visibility: role.visibility().unwrap_or(0),
            }),
            foks_proto::RoleType::Admin => Ok(Self::Admin),
            foks_proto::RoleType::Owner => Ok(Self::Owner),
            foks_proto::RoleType::None => Err(Error::InvalidAccount("KV entry has no access role")),
        }
    }

    fn to_role(self) -> Role {
        match self {
            Self::Member { visibility } => Role::member(visibility),
            Self::Admin => Role::ADMIN,
            Self::Owner => Role::OWNER,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvReadReport {
    pub path: String,
    pub node_type: String,
    pub version: u64,
    pub size: Option<u64>,
    pub read_role: KvRoleSummary,
    pub write_role: KvRoleSummary,
    /// Base64 on the wire, matching `foks_agent_proto::data::DataEntry`, which
    /// is what the adapter reader parses this into. The two shapes share one
    /// adapter so neither side can move without the other.
    #[serde(with = "foks_agent_proto::base64_bytes::optional")]
    pub content: Option<Vec<u8>>,
    pub symlink_target: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvChunkReport {
    pub path: String,
    pub version: u64,
    pub offset: u64,
    /// As on [`KvReadReport::content`]; the reader is
    /// `foks_agent_proto::data::DataChunk`.
    #[serde(with = "foks_agent_proto::base64_bytes")]
    pub content: Vec<u8>,
    pub eof: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvCatalogEntry {
    pub path: String,
    pub node_type: String,
    pub version: u64,
    pub size: Option<u64>,
    pub read_role: KvRoleSummary,
    pub write_role: KvRoleSummary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct KvCatalogReport {
    pub snapshot_version: u64,
    pub entries: Vec<KvCatalogEntry>,
}

impl KvCatalogReport {
    pub(super) fn from_tree(tree: &[KvDirectoryProjection]) -> Result<Self> {
        // Zero denotes the not-yet-created namespace; real roots start at one.
        let snapshot_version = tree.first().map_or(0, |root| root.root_version);
        Ok(Self {
            snapshot_version,
            entries: flatten_catalog_tree(tree)?,
        })
    }
}

impl KvWriteReport {
    fn from_path(path: &str, version: u64, directories: &[KvDirectoryProjection]) -> Self {
        Self {
            path: path.to_owned(),
            version,
            path_directories: directories.len(),
            path_entries: directories
                .iter()
                .map(|directory| directory.entries.len())
                .sum(),
        }
    }
}

fn checked_entry<'a>(
    tree: &'a [KvDirectoryProjection],
    path: &str,
    version: u64,
) -> Result<&'a foks_client_db::KvProjectedEntry> {
    let entry = optional_entry(tree, path)?.ok_or(Error::KvConflict)?;
    if !entry.readable {
        return Err(Error::InvalidKvPath(
            "KV entry is not readable by this account",
        ));
    }
    if entry.version != version {
        return Err(Error::KvConflict);
    }
    Ok(entry)
}

fn verify_precondition(
    tree: &[KvDirectoryProjection],
    path: &str,
    precondition: KvMutationPrecondition,
) -> Result<()> {
    match precondition {
        KvMutationPrecondition::Create => optional_entry(tree, path)?
            .is_none()
            .then_some(())
            .ok_or(Error::KvConflict),
        KvMutationPrecondition::ExactVersion(version) => {
            checked_entry(tree, path, version).map(|_| ())
        }
    }
}

fn checked_write_error(error: foks_client::Error) -> Error {
    match error {
        foks_client::Error::KvResponse(
            "KV dirent version precondition failed" | "KV entry already exists",
        ) => Error::KvConflict,
        error => Error::Client(error),
    }
}

fn projected_write_role(entry: &foks_client_db::KvProjectedEntry) -> Result<Role> {
    match (entry.write_role_type, entry.write_role_visibility) {
        (1, visibility) => Ok(Role::member(i16::try_from(visibility).map_err(|_| {
            Error::InvalidAccount("KV write-role visibility is out of range")
        })?)),
        (2, 0) => Ok(Role::ADMIN),
        (3, 0) => Ok(Role::OWNER),
        _ => Err(Error::InvalidAccount("KV write role is invalid")),
    }
}

/// Builds the report for one entry from the directories along its path. A
/// directory's own read role is not in its parent's projection, so it comes
/// from the node the read already fetched and authenticated.
fn read_report_from_fetched(
    tree: &[KvDirectoryProjection],
    path: &str,
    version: u64,
    fetched: foks_client::KvFetchedNode,
) -> Result<KvReadReport> {
    let entry = checked_entry(tree, path, version)?;
    let node_type = KvNodeId(entry.node_id).node_type()?;
    let write_role = projected_write_role(entry)?;
    let (content, symlink_target, size, read_role) = match (node_type, fetched) {
        (KvNodeType::SmallFile, foks_client::KvFetchedNode::SmallFile(content)) => {
            let size = Some(content.len() as u64);
            (Some(content), None, size, projected_read_role(entry)?)
        }
        (KvNodeType::Symlink, foks_client::KvFetchedNode::Symlink(target)) => {
            let target = std::str::from_utf8(&target)
                .map_err(|_| Error::InvalidAccount("symlink target is not UTF-8"))?
                .to_owned();
            let size = Some(target.len() as u64);
            (None, Some(target), size, projected_read_role(entry)?)
        }
        (KvNodeType::File, foks_client::KvFetchedNode::LargeFile { size }) => {
            (None, None, size, projected_read_role(entry)?)
        }
        (KvNodeType::Directory, foks_client::KvFetchedNode::Directory { read_role }) => {
            (None, None, None, read_role)
        }
        (KvNodeType::None, _) => return Err(Error::InvalidAccount("KV entry is a tombstone")),
        _ => return Err(Error::InvalidAccount("KV node type changed during read")),
    };
    Ok(KvReadReport {
        path: path.to_owned(),
        node_type: node_type_name(node_type)?,
        version,
        size,
        read_role: KvRoleSummary::from_role(read_role)?,
        write_role: KvRoleSummary::from_role(write_role)?,
        content,
        symlink_target,
    })
}

fn node_type_name(node_type: KvNodeType) -> Result<String> {
    Ok(match node_type {
        KvNodeType::None => return Err(Error::InvalidAccount("KV entry is a tombstone")),
        KvNodeType::Directory => "directory",
        KvNodeType::File => "file",
        KvNodeType::SmallFile => "small-file",
        KvNodeType::Symlink => "symlink",
    }
    .to_owned())
}

/// The read role of one non-directory entry, taken from the authenticated
/// node metadata its traversal retained. A directory keeps its read role in
/// its own generation, not in the dirent that names it.
fn projected_read_role(entry: &foks_client_db::KvProjectedEntry) -> Result<Role> {
    let encoded = entry
        .node_bytes
        .as_deref()
        .ok_or(Error::InvalidAccount("catalog node metadata is absent"))?;
    match (
        KvNodeId(entry.node_id).node_type()?,
        foks_proto::KvNode::decode(encoded)?,
    ) {
        (KvNodeType::File, foks_proto::KvNode::File(metadata)) => Ok(metadata.key.role),
        (KvNodeType::SmallFile, foks_proto::KvNode::SmallFile(boxed))
        | (KvNodeType::Symlink, foks_proto::KvNode::Symlink(boxed)) => Ok(boxed.key.role),
        _ => Err(Error::InvalidAccount("catalog file type changed")),
    }
}
/// The directory components a path walks through, excluding the final name.
/// Resolving them reads every directory a lookup of `path` needs and no
/// others.
fn parent_components(path: &str) -> Result<Vec<Vec<u8>>> {
    let mut components = path_components(path)?;
    components.pop();
    Ok(components)
}

fn path_components(path: &str) -> Result<Vec<Vec<u8>>> {
    if !path.starts_with('/') || path.len() > 4096 {
        return Err(Error::InvalidKvPath(
            "path must be absolute and at most 4096 bytes",
        ));
    }
    if path == "/" {
        return Ok(Vec::new());
    }
    let components = path[1..]
        .split('/')
        .map(decode_path_component)
        .collect::<Result<Vec<_>>>()?;
    if components.iter().any(|component| {
        component.is_empty()
            || component.as_slice() == b"."
            || component.as_slice() == b".."
            || component.len() > 255
    }) {
        return Err(Error::InvalidKvPath(
            "path component is empty, relative, or exceeds 255 bytes",
        ));
    }
    Ok(components)
}

fn decode_path_component(component: &str) -> Result<Vec<u8>> {
    let bytes = component.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset] != b'%' {
            output.push(bytes[offset]);
            offset += 1;
            continue;
        }
        let pair = bytes
            .get(offset + 1..offset + 3)
            .ok_or(Error::InvalidKvPath(
                "path has an incomplete percent escape",
            ))?;
        let high = hex_nibble(pair[0])
            .ok_or(Error::InvalidKvPath("path has an invalid percent escape"))?;
        let low = hex_nibble(pair[1])
            .ok_or(Error::InvalidKvPath("path has an invalid percent escape"))?;
        output.push((high << 4) | low);
        offset += 3;
    }
    Ok(output)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn split_parent(path: &str) -> Result<(String, String)> {
    let components = path_components(path)?;
    let (name, parents) = components.split_last().ok_or(Error::InvalidKvPath(
        "the KV root cannot be mutated as an entry",
    ))?;
    let parent = if parents.is_empty() {
        "/".to_owned()
    } else {
        path.rsplit_once('/')
            .map(|(parent, _)| parent.to_owned())
            .ok_or(Error::InvalidKvPath("path has no parent"))?
    };
    let name = String::from_utf8(name.clone()).map_err(|_| {
        Error::InvalidKvPath("entry name is not UTF-8 and cannot be mutated through this API")
    })?;
    Ok((parent, name))
}

fn resolve_directory(tree: &[KvDirectoryProjection], path: &str) -> Result<[u8; 16]> {
    let components = path_components(path)?;
    let mut current = root_directory(tree)?;
    for component in components {
        current = directory_child(tree, current, &component)?
            .ok_or(Error::InvalidKvPath("directory component does not exist"))?;
    }
    Ok(current)
}

/// Resolves the parent directory for a KV write path, creating intermediate
/// directories if `mkdir_p` is enabled.
///
/// Only the directories the path walks through are fetched, and the session
/// retains them as the precondition scope for the write that follows. When
/// `create_root` names a pair of roles, a store that has never been written
/// to has its root created first. When `mkdir_p` is true, missing parent
/// directories are created within the current write session, inheriting the
/// item's read and write roles to ensure consistent access control across the
/// directory hierarchy.
fn resolve_write_parent(
    session: &mut foks_client::KvWriteSession<'_>,
    path: &str,
    mkdir_p: bool,
    read_role: KvRoleSummary,
    write_role: KvRoleSummary,
    create_root: Option<(Role, Role)>,
) -> Result<([u8; 16], Vec<KvDirectoryProjection>)> {
    let components = path_components(path)?;
    let mut tree = session.resolve_path(&components)?;
    if tree.is_empty() {
        if let Some((root_read_role, root_write_role)) = create_root {
            session.initialize_root(root_read_role, root_write_role)?;
            tree = session.resolve_path(&components)?;
        }
    }
    if !mkdir_p {
        let parent = resolve_directory(&tree, path)?;
        return Ok((parent, tree));
    }
    let mut current = root_directory(&tree)?;
    let mut creating = false;
    for (depth, component) in components.iter().enumerate() {
        if !creating {
            if let Some(child) = directory_child(&tree, current, component)? {
                current = child;
                continue;
            }
        }
        let name = String::from_utf8(component.clone()).map_err(|_| {
            Error::InvalidKvPath("a missing parent directory name is not UTF-8 and cannot be created through this API")
        })?;
        // Each new directory is created under the prefix that names it, so
        // the mkdir cites the path it walked rather than an unrelated one.
        session.resolve_path(&components[..depth])?;
        let result = session.mkdir(
            current,
            &name,
            KvWriteOptions {
                read_role: read_role.to_role(),
                write_role: write_role.to_role(),
                overwrite: false,
                expected_version: None,
            },
        )?;
        current = result.node_id.object_id();
        creating = true;
    }
    if creating {
        tree = session.resolve_path(&components)?;
        current = resolve_directory(&tree, path)?;
    }
    Ok((current, tree))
}

/// Adapter variant of [`resolve_write_parent`] that walks a complete
/// projection the caller already fetched, because the adapter also enforces a
/// whole-namespace entry limit and resolves symlinks anywhere in the store.
fn resolve_write_parent_in_tree(
    session: &mut foks_client::KvWriteSession<'_>,
    tree: Vec<KvDirectoryProjection>,
    path: &str,
    mkdir_p: bool,
    read_role: KvRoleSummary,
    write_role: KvRoleSummary,
) -> Result<([u8; 16], Vec<KvDirectoryProjection>)> {
    if !mkdir_p {
        let parent = resolve_directory(&tree, path)?;
        return Ok((parent, tree));
    }
    let components = path_components(path)?;
    let mut tree = tree;
    let mut current = root_directory(&tree)?;
    let mut creating = false;
    for component in &components {
        if !creating {
            if let Some(child) = directory_child(&tree, current, component)? {
                current = child;
                continue;
            }
        }
        let name = String::from_utf8(component.clone()).map_err(|_| {
            Error::InvalidKvPath("a missing parent directory name is not UTF-8 and cannot be created through this API")
        })?;
        let result = session.mkdir(
            current,
            &name,
            KvWriteOptions {
                read_role: read_role.to_role(),
                write_role: write_role.to_role(),
                overwrite: false,
                expected_version: None,
            },
        )?;
        current = result.node_id.object_id();
        creating = true;
    }
    if creating {
        tree = session.sync()?;
        current = resolve_directory(&tree, path)?;
    }
    Ok((current, tree))
}

fn root_directory(tree: &[KvDirectoryProjection]) -> Result<[u8; 16]> {
    Ok(tree
        .first()
        .ok_or(Error::InvalidKvPath("KV projection has no root"))?
        .root_directory_id)
}

fn directory_child(
    tree: &[KvDirectoryProjection],
    parent: [u8; 16],
    component: &[u8],
) -> Result<Option<[u8; 16]>> {
    let directory = tree
        .iter()
        .find(|directory| directory.directory_id == parent)
        .ok_or(Error::InvalidKvPath(
            "parent directory is absent from the projection",
        ))?;
    let Some(entry) = directory
        .entries
        .iter()
        .find(|entry| entry.name == component)
    else {
        return Ok(None);
    };
    let node = KvNodeId(entry.node_id);
    if node.node_type()? != KvNodeType::Directory {
        return Err(Error::InvalidKvPath("path component is not a directory"));
    }
    Ok(Some(node.object_id()))
}

fn resolve_entry<'a>(
    tree: &'a [KvDirectoryProjection],
    path: &str,
) -> Result<&'a foks_client_db::KvProjectedEntry> {
    optional_entry(tree, path)?.ok_or(Error::InvalidKvPath("KV entry does not exist"))
}

fn optional_entry<'a>(
    tree: &'a [KvDirectoryProjection],
    path: &str,
) -> Result<Option<&'a foks_client_db::KvProjectedEntry>> {
    let mut components = path_components(path)?;
    let name = components
        .pop()
        .ok_or(Error::InvalidKvPath("the KV root is not a directory entry"))?;
    let root = tree
        .first()
        .ok_or(Error::InvalidKvPath("KV projection has no root"))?
        .root_directory_id;
    let mut parent = root;
    for component in components {
        let directory = tree
            .iter()
            .find(|directory| directory.directory_id == parent)
            .ok_or(Error::InvalidKvPath(
                "parent directory is absent from the projection",
            ))?;
        let entry = directory
            .entries
            .iter()
            .find(|entry| entry.name == component)
            .ok_or(Error::InvalidKvPath("directory component does not exist"))?;
        let node = KvNodeId(entry.node_id);
        if node.node_type()? != KvNodeType::Directory {
            return Err(Error::InvalidKvPath("path component is not a directory"));
        }
        parent = node.object_id();
    }
    Ok(tree
        .iter()
        .find(|directory| directory.directory_id == parent)
        .and_then(|directory| directory.entries.iter().find(|entry| entry.name == name)))
}

fn flatten_tree(tree: &[KvDirectoryProjection]) -> Result<Vec<KvEntrySummary>> {
    use std::collections::{BTreeSet, VecDeque};

    if tree.is_empty() {
        return Ok(Vec::new());
    }
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
            if !entry.readable {
                continue;
            }
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

fn flatten_catalog_tree(tree: &[KvDirectoryProjection]) -> Result<Vec<KvCatalogEntry>> {
    use std::collections::{BTreeSet, VecDeque};

    if tree.is_empty() {
        return Ok(Vec::new());
    }
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
            if !entry.readable {
                continue;
            }
            let name = display_component(&entry.name);
            let path = format!("{parent_path}/{name}");
            let node_id = KvNodeId(entry.node_id);
            let node_type = node_id.node_type()?;
            let read_role = match node_type {
                KvNodeType::Directory => {
                    let child = tree
                        .iter()
                        .find(|child| child.directory_id == node_id.object_id())
                        .ok_or(Error::InvalidKvPath(
                            "KV projection omits a reachable directory",
                        ))?;
                    foks_proto::KvDirectoryPair::decode(&child.directory_bytes)?
                        .active
                        .key
                        .role
                }
                KvNodeType::None => {
                    return Err(Error::InvalidAccount("catalog contains a tombstone"));
                }
                _ => projected_read_role(entry)?,
            };
            let write_role = projected_write_role(entry)?;
            output.push(KvCatalogEntry {
                path: path.clone(),
                node_type: node_type_name(node_type)?,
                version: entry.version,
                // FOKS does not expose plaintext size in node metadata. The
                // catalog leaves it absent instead of downloading content.
                size: None,
                read_role: KvRoleSummary::from_role(read_role)?,
                write_role: KvRoleSummary::from_role(write_role)?,
            });
            if node_type == KvNodeType::Directory {
                pending.push_back((node_id.object_id(), path));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn projected_entry(name: &[u8]) -> foks_client_db::KvProjectedEntry {
        foks_client_db::KvProjectedEntry {
            readable: true,
            dirent_id: [1; 16],
            node_id: [2; 17],
            version: 1,
            directory_version: 1,
            name: name.to_vec(),
            write_role_type: 3,
            write_role_visibility: 0,
            creation_time: 0,
            dirent_bytes: Vec::new(),
            node_bytes: None,
            content: None,
            symlink: None,
            large_file_size: None,
        }
    }

    #[test]
    fn displayed_components_resolve_back_to_their_exact_foks_name_bytes() {
        let entries = [b"hello world".as_slice(), b"slash/name", &[0xff, 0]]
            .into_iter()
            .map(projected_entry)
            .collect();
        let tree = [KvDirectoryProjection {
            host_id: Vec::new(),
            party_id: Vec::new(),
            root_version: 1,
            root_directory_id: [7; 16],
            root_bytes: Vec::new(),
            directory_id: [7; 16],
            directory_version: 1,
            directory_bytes: Vec::new(),
            entries,
        }];
        for path in ["/hello%20world", "/slash%2Fname", "/%FF%00"] {
            assert!(optional_entry(&tree, path).unwrap().is_some(), "{path}");
        }
        assert!(matches!(
            checked_entry(&tree, "/missing", 1),
            Err(Error::KvConflict)
        ));
    }

    #[test]
    fn restricted_entries_are_hidden_but_not_treated_as_absent() {
        let mut denied = projected_entry(b"restricted");
        denied.readable = false;
        let tree = [KvDirectoryProjection {
            host_id: Vec::new(),
            party_id: Vec::new(),
            root_version: 1,
            root_directory_id: [7; 16],
            root_bytes: Vec::new(),
            directory_id: [7; 16],
            directory_version: 1,
            directory_bytes: Vec::new(),
            entries: vec![denied],
        }];
        assert!(flatten_catalog_tree(&tree).unwrap().is_empty());
        assert!(flatten_tree(&tree).unwrap().is_empty());
        assert!(matches!(
            verify_precondition(&tree, "/restricted", KvMutationPrecondition::Create),
            Err(Error::KvConflict)
        ));
        assert!(checked_entry(&tree, "/restricted", 1).is_err());
        let mut malformed = tree.clone();
        malformed[0].entries[0].readable = true;
        assert!(flatten_catalog_tree(&malformed).is_err());
    }

    #[test]
    fn exact_legacy_large_file_read_reports_no_size_and_no_content() {
        let fixture = |name: &str| {
            std::fs::read(format!(
                "../foks-snowpack/tests/fixtures/foks-v0.1.9/user/{name}"
            ))
            .unwrap()
        };
        let listing = foks_proto::KvListResponse::decode(&fixture("kv-list.snowp")).unwrap();
        let node_id = listing
            .entries
            .iter()
            .find(|entry| entry.value.node_type().unwrap() == KvNodeType::File)
            .unwrap()
            .value;
        let tree = [KvDirectoryProjection {
            host_id: Vec::new(),
            party_id: Vec::new(),
            root_version: 9,
            root_directory_id: [7; 16],
            root_bytes: Vec::new(),
            directory_id: [7; 16],
            directory_version: 1,
            directory_bytes: Vec::new(),
            entries: vec![foks_client_db::KvProjectedEntry {
                readable: true,
                dirent_id: [1; 16],
                node_id: node_id.0,
                version: 4,
                directory_version: 1,
                name: b"archive.bin".to_vec(),
                write_role_type: 3,
                write_role_visibility: 0,
                creation_time: 0,
                dirent_bytes: Vec::new(),
                node_bytes: Some(fixture("kv-large-node.snowp")),
                content: None,
                symlink: None,
                large_file_size: None,
            }],
        }];
        let report = read_report_from_fetched(
            &tree,
            "/archive.bin",
            4,
            foks_client::KvFetchedNode::LargeFile { size: None },
        )
        .unwrap();
        // The official Go fixture has no custom metadata, and node reads do
        // not fetch content to compensate.
        assert_eq!(report.size, None);
        assert!(report.content.is_none());
    }
}
