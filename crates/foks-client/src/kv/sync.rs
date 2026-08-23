//! Authenticated KV traversal and monotonic soft-state projection.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use foks_client_db::{KvDirectoryProjection, KvLargeFileStage, KvProjectedEntry, SoftStateStore};
use foks_crypto::{derive_subkey_id, open_kv_chunk, open_kv_dirent_name};
use foks_proto::{
    KvDirectoryPair, KvDirectoryStatus, KvEncryptedChunk, KvListResponse, KvNode, KvNodeType,
    KvParty, KvRoot, KvSmallFilePlaintext, SecretSeed,
};
use foks_rpc::{KvAuth, KvListCursor};
use foks_verify::VerifiedUserState;

use super::rpc::KvRequest;
use super::support::{
    kv_key, kv_version_vector, reachable_kv_tree, user_kv_keys, validate_kv_symlink,
};
use super::{KvPrivateKeyRef, KvWriteSession, OwnedKvAuth};
use crate::{
    AuthenticatedTeamOutcome, DeviceCredential, Error, FoksClient, PinnedHost, Result,
    UserPrivateKey, YubiCredential,
};

impl FoksClient {
    /// Verifies and projects the complete reachable personal KV tree. The PUK
    /// remains caller-owned; only encrypted wire objects and decrypted file
    /// projections are written to the separate soft-state database.
    pub fn sync_user_kv(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        soft_database_path: &Path,
    ) -> Result<Vec<KvDirectoryProjection>> {
        if user.uid() != &credential.uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "KV user state does not match the credential and pinned host",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        self.sync_kv_with_fetch(
            host,
            KvParty {
                party: credential.uid.clone(),
                host: host.host_id.clone(),
            },
            KvAuth::User,
            &keys,
            soft_database_path,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Hardware-backed credential variant of [`Self::sync_user_kv`].
    pub fn sync_user_kv_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        soft_database_path: &Path,
    ) -> Result<Vec<KvDirectoryProjection>> {
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        if user.uid() != &credential.uid
            || user.host() != host.host_id()
            || !user.devices().iter().any(|device| {
                device.id == *credential.parent.entity_id()
                    && device.hepk == *credential.parent.hepk()
                    && device.subkey.as_ref() == Some(&subkey)
            })
        {
            return Err(Error::CredentialBinding(
                "KV Yubi credential does not match the supplied user state",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        self.sync_kv_with_fetch(
            host,
            KvParty {
                party: credential.uid.clone(),
                host: host.host_id.clone(),
            },
            KvAuth::User,
            &keys,
            soft_database_path,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Verifies and projects the complete reachable team KV tree under the
    /// short-lived bearer token and PTKs returned by `load_and_pin_team`.
    pub fn sync_team_kv(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &AuthenticatedTeamOutcome,
        soft_database_path: &Path,
    ) -> Result<Vec<KvDirectoryProjection>> {
        if team.verified.host() != host.host_id() {
            return Err(Error::TeamBinding("KV team state belongs to another host"));
        }
        let keys = team
            .ptks
            .iter()
            .map(|key| KvPrivateKeyRef {
                role: key.role,
                generation: key.generation,
                seed: &key.seed,
            })
            .collect::<Vec<_>>();
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        self.sync_kv_with_fetch(
            host,
            KvParty {
                party: team.verified.team().clone(),
                host: host.host_id.clone(),
            },
            KvAuth::Team(&team.view_token),
            &keys,
            soft_database_path,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Hardware-backed credential variant of [`Self::sync_team_kv`].
    pub fn sync_team_kv_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &AuthenticatedTeamOutcome,
        soft_database_path: &Path,
    ) -> Result<Vec<KvDirectoryProjection>> {
        if team.verified.host() != host.host_id() {
            return Err(Error::TeamBinding("KV team state belongs to another host"));
        }
        let keys = team
            .ptks
            .iter()
            .map(|key| KvPrivateKeyRef {
                role: key.role,
                generation: key.generation,
                seed: &key.seed,
            })
            .collect::<Vec<_>>();
        let mut connection = self.kv_connection_with_material(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        self.sync_kv_with_fetch(
            host,
            KvParty {
                party: team.verified.team().clone(),
                host: host.host_id.clone(),
            },
            KvAuth::Team(&team.view_token),
            &keys,
            soft_database_path,
            |auth, request| connection.call(auth, request),
        )
    }

    pub fn user_kv_write_session<'a>(
        &'a self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &'a [UserPrivateKey],
        soft_database_path: &Path,
    ) -> Result<KvWriteSession<'a>> {
        if user.uid() != &credential.uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "KV user state does not match the credential and pinned host",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let connection = self.kv_connection_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
        )?;
        Ok(KvWriteSession {
            client: self,
            host: host.clone(),
            party: KvParty {
                party: credential.uid.clone(),
                host: host.host_id.clone(),
            },
            auth: OwnedKvAuth::User,
            private_keys: keys,
            soft_database_path: soft_database_path.to_owned(),
            connection,
        })
    }

    pub fn user_kv_write_session_yubi<'a>(
        &'a self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        user: &VerifiedUserState,
        puks: &'a [UserPrivateKey],
        soft_database_path: &Path,
    ) -> Result<KvWriteSession<'a>> {
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        if user.uid() != &credential.uid
            || user.host() != host.host_id()
            || !user.devices().iter().any(|device| {
                device.id == *credential.parent.entity_id()
                    && device.hepk == *credential.parent.hepk()
                    && device.subkey.as_ref() == Some(&subkey)
            })
        {
            return Err(Error::CredentialBinding(
                "KV Yubi credential does not match the supplied user state",
            ));
        }
        let keys = user_kv_keys(user, puks)?;
        let connection = self.kv_connection_with_material(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        Ok(KvWriteSession {
            client: self,
            host: host.clone(),
            party: KvParty {
                party: credential.uid.clone(),
                host: host.host_id.clone(),
            },
            auth: OwnedKvAuth::User,
            private_keys: keys,
            soft_database_path: soft_database_path.to_owned(),
            connection,
        })
    }

    pub fn team_kv_write_session<'a>(
        &'a self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        team: &'a AuthenticatedTeamOutcome,
        soft_database_path: &Path,
    ) -> Result<KvWriteSession<'a>> {
        self.team_kv_write_session_with_material(
            host,
            &credential.seed,
            &credential.certificate_chain,
            team,
            soft_database_path,
        )
    }

    pub fn team_kv_write_session_yubi<'a>(
        &'a self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        team: &'a AuthenticatedTeamOutcome,
        soft_database_path: &Path,
    ) -> Result<KvWriteSession<'a>> {
        self.team_kv_write_session_with_material(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
            team,
            soft_database_path,
        )
    }

    fn team_kv_write_session_with_material<'a>(
        &'a self,
        host: &PinnedHost,
        device_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        team: &'a AuthenticatedTeamOutcome,
        soft_database_path: &Path,
    ) -> Result<KvWriteSession<'a>> {
        if team.verified.host() != host.host_id() {
            return Err(Error::TeamBinding("KV team state belongs to another host"));
        }
        let connection = self.kv_connection_with_material(host, device_seed, certificate_chain)?;
        Ok(KvWriteSession {
            client: self,
            host: host.clone(),
            party: KvParty {
                party: team.verified.team().clone(),
                host: host.host_id.clone(),
            },
            auth: OwnedKvAuth::Team(team.view_token),
            private_keys: team
                .ptks
                .iter()
                .map(|key| KvPrivateKeyRef {
                    role: key.role,
                    generation: key.generation,
                    seed: &key.seed,
                })
                .collect(),
            soft_database_path: soft_database_path.to_owned(),
            connection,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sync_kv_with_fetch<F>(
        &self,
        host: &PinnedHost,
        party: KvParty,
        auth: KvAuth<'_>,
        private_keys: &[KvPrivateKeyRef<'_>],
        soft_database_path: &Path,
        mut fetch: F,
    ) -> Result<Vec<KvDirectoryProjection>>
    where
        F: FnMut(KvAuth<'_>, &KvRequest) -> Result<Vec<u8>>,
    {
        const PAGE_SIZE: u64 = 100;
        const MAX_DIRECTORIES: usize = 4096;
        const MAX_PAGES_PER_DIRECTORY: usize = 4096;
        const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
        const MAX_FILE_CHUNKS: usize = 4096;
        const MAX_ENTRIES: usize = 100_000;
        const MAX_TOTAL_CONTENT_BYTES: usize = 512 * 1024 * 1024;

        for _ in 0..3 {
            let attempt = (|| -> Result<Vec<KvDirectoryProjection>> {
                let mut store = SoftStateStore::open(soft_database_path)?;
                let cached_vector =
                    store.version_vector(host.host_id.as_bytes(), party.party.as_bytes())?;
                let cached_tree = if cached_vector.is_some() {
                    store.tree(host.host_id.as_bytes(), party.party.as_bytes())?
                } else {
                    Vec::new()
                };
                let stale = if let Some(versions) = &cached_vector {
                    match fetch(auth, &KvRequest::CacheCheck(versions.clone())) {
                        Ok(_) => return Ok(cached_tree),
                        Err(Error::Rpc(foks_rpc::Error::KvStaleCache(stale))) => Some(stale),
                        Err(error) => return Err(error),
                    }
                } else {
                    None
                };
                let incremental = stale.as_ref().is_some_and(|stale| {
                    cached_vector
                        .as_ref()
                        .is_some_and(|cached| stale.root_version == cached.root_version)
                });
                let (root_bytes, root, mut queue) = if incremental {
                    let cached_root = cached_tree
                        .first()
                        .ok_or(Error::KvResponse("cached KV tree has no root"))?;
                    let root = KvRoot::decode(&cached_root.root_bytes)?;
                    let stale = stale
                        .as_ref()
                        .expect("incremental state has stale versions");
                    let queue = stale
                        .directories
                        .iter()
                        .map(|directory| directory.id)
                        .collect::<VecDeque<_>>();
                    (cached_root.root_bytes.clone(), root, queue)
                } else {
                    let root_bytes = fetch(auth, &KvRequest::Root)?;
                    let root = KvRoot::decode(&root_bytes)?;
                    let queue = VecDeque::from([root.root]);
                    (root_bytes, root, queue)
                };
                if root.version == 0 {
                    return Err(Error::KvResponse("root version is zero"));
                }
                let root_keys = kv_key(private_keys, root.key.role, root.key.generation)?;
                root_keys.verify_root(&root, &party)?;

                let known_directories = cached_tree
                    .iter()
                    .map(|directory| directory.directory_id)
                    .collect::<BTreeSet<_>>();
                let mut combined = if incremental {
                    cached_tree
                        .into_iter()
                        .map(|directory| (directory.directory_id, directory))
                        .collect::<BTreeMap<_, _>>()
                } else {
                    BTreeMap::new()
                };
                let mut large_files: Vec<KvLargeFileStage> = Vec::new();
                let mut visited = BTreeSet::new();
                let mut projections = Vec::new();
                let mut total_entries = 0usize;
                let mut total_content_bytes = 0usize;
                while let Some(directory_id) = queue.pop_front() {
                    if !visited.insert(directory_id) {
                        continue;
                    }
                    if visited.len() > MAX_DIRECTORIES {
                        return Err(Error::KvResponse("directory traversal limit exceeded"));
                    }
                    let directory_bytes = fetch(auth, &KvRequest::Directory(directory_id))?;
                    let directory = KvDirectoryPair::decode(&directory_bytes)?;
                    if directory.active.id != directory_id
                        || directory.active.status != KvDirectoryStatus::Active
                        || directory.encrypting.as_ref().is_some_and(|encrypting| {
                            encrypting.id != directory_id
                                || encrypting.status != KvDirectoryStatus::Encrypting
                        })
                    {
                        return Err(Error::KvResponse("directory response ID mismatch"));
                    }
                    let mut directory_seeds = BTreeMap::new();
                    for generation in
                        std::iter::once(&directory.active).chain(directory.encrypting.iter())
                    {
                        let keys =
                            kv_key(private_keys, generation.key.role, generation.key.generation)?;
                        let seed = keys.open_directory_seed(generation)?;
                        if directory_seeds.insert(generation.version, seed).is_some() {
                            return Err(Error::KvResponse(
                                "duplicate directory encryption version",
                            ));
                        }
                    }

                    let mut cursor = KvListCursor::None;
                    let mut entries = Vec::new();
                    let mut entry_ids = BTreeSet::new();
                    let mut names = BTreeSet::new();
                    let mut final_page = false;
                    for _ in 0..MAX_PAGES_PER_DIRECTORY {
                        let list_bytes = fetch(
                            auth,
                            &KvRequest::List {
                                directory: directory_id,
                                cursor,
                                number: PAGE_SIZE,
                                load_small_files: true,
                            },
                        )?;
                        let listing = KvListResponse::decode(&list_bytes)?;
                        if !listing.final_page && listing.entries.is_empty() {
                            return Err(Error::KvResponse("non-final KV list page is empty"));
                        }
                        let mut extended = BTreeMap::new();
                        for item in listing.extended {
                            let position = usize::try_from(item.position).map_err(|_| {
                                Error::KvResponse("extended dirent position overflow")
                            })?;
                            if position >= listing.entries.len()
                                || extended.insert(position, item.small_file).is_some()
                            {
                                return Err(Error::KvResponse("invalid extended dirent position"));
                            }
                        }
                        let last_mac = listing.entries.last().map(|entry| entry.name_mac);
                        for (position, entry) in listing.entries.into_iter().enumerate() {
                            total_entries = total_entries
                                .checked_add(1)
                                .ok_or(Error::KvResponse("entry count overflow"))?;
                            if total_entries > MAX_ENTRIES {
                                return Err(Error::KvResponse("entry traversal limit exceeded"));
                            }
                            if entry.parent != directory_id
                                || !entry_ids.insert(entry.id)
                                || entry.value.node_type()? == KvNodeType::None
                            {
                                return Err(Error::KvResponse(
                                    "invalid or duplicate directory entry",
                                ));
                            }
                            let seed = directory_seeds.get(&entry.directory_version).ok_or(
                                Error::KvResponse("dirent uses an unavailable directory key"),
                            )?;
                            let name = open_kv_dirent_name(seed, &entry)?.name;
                            if std::str::from_utf8(&name).is_err()
                                || name.contains(&0)
                                || name.contains(&b'/')
                                || name == b"."
                                || name == b".."
                                || !names.insert(name.clone())
                            {
                                return Err(Error::KvResponse(
                                    "invalid or duplicate plaintext name",
                                ));
                            }
                            let mut projected = KvProjectedEntry {
                                dirent_id: entry.id,
                                node_id: entry.value.0,
                                version: entry.version,
                                directory_version: entry.directory_version,
                                name,
                                write_role_type: entry.write_role.protocol_value(),
                                write_role_visibility: i64::from(
                                    entry.write_role.visibility().unwrap_or(0),
                                ),
                                creation_time: entry.creation_time,
                                dirent_bytes: entry.encoded().to_vec(),
                                node_bytes: None,
                                content: None,
                                symlink: None,
                                large_file_size: None,
                            };
                            match entry.value.node_type()? {
                                KvNodeType::Directory => {
                                    let child = entry.value.object_id();
                                    if !known_directories.contains(&child) {
                                        queue.push_back(child);
                                    }
                                }
                                KvNodeType::SmallFile => {
                                    let (boxed, exact) = match extended.remove(&position) {
                                        Some(boxed) => (boxed, None),
                                        None => {
                                            let bytes = fetch(auth, &KvRequest::Node(entry.value))?;
                                            let KvNode::SmallFile(boxed) = KvNode::decode(&bytes)?
                                            else {
                                                return Err(Error::KvResponse(
                                                    "small-file node type mismatch",
                                                ));
                                            };
                                            (boxed, Some(bytes))
                                        }
                                    };
                                    let keys =
                                        kv_key(private_keys, boxed.key.role, boxed.key.generation)?;
                                    let KvSmallFilePlaintext::File(content) =
                                        keys.open_small_file(entry.value, &boxed)?
                                    else {
                                        return Err(Error::KvResponse(
                                            "small-file plaintext type mismatch",
                                        ));
                                    };
                                    projected.node_bytes = exact;
                                    total_content_bytes = total_content_bytes
                                        .checked_add(content.len())
                                        .ok_or(Error::KvResponse("content size overflow"))?;
                                    projected.content = Some(content);
                                }
                                KvNodeType::Symlink => {
                                    let bytes = fetch(auth, &KvRequest::Node(entry.value))?;
                                    let KvNode::Symlink(boxed) = KvNode::decode(&bytes)? else {
                                        return Err(Error::KvResponse(
                                            "symlink node type mismatch",
                                        ));
                                    };
                                    let keys =
                                        kv_key(private_keys, boxed.key.role, boxed.key.generation)?;
                                    let KvSmallFilePlaintext::Symlink(target) =
                                        keys.open_small_file(entry.value, &boxed)?
                                    else {
                                        return Err(Error::KvResponse(
                                            "symlink plaintext type mismatch",
                                        ));
                                    };
                                    validate_kv_symlink(&target)?;
                                    projected.node_bytes = Some(bytes);
                                    total_content_bytes = total_content_bytes
                                        .checked_add(target.len())
                                        .ok_or(Error::KvResponse("content size overflow"))?;
                                    projected.symlink = Some(target);
                                }
                                KvNodeType::File => {
                                    let bytes = fetch(auth, &KvRequest::Node(entry.value))?;
                                    let KvNode::File(metadata) = KvNode::decode(&bytes)? else {
                                        return Err(Error::KvResponse(
                                            "large-file node type mismatch",
                                        ));
                                    };
                                    let keys = kv_key(
                                        private_keys,
                                        metadata.key.role,
                                        metadata.key.generation,
                                    )?;
                                    let size = if let Some(stage) = large_files
                                        .iter()
                                        .find(|stage| stage.node_id == entry.value.0)
                                    {
                                        stage.size
                                    } else {
                                        let file_seed =
                                            keys.open_file_seed(entry.value, &metadata)?;
                                        let mut stage = store.begin_large_file(
                                            host.host_id.as_bytes(),
                                            party.party.as_bytes(),
                                            entry.value.0,
                                        )?;
                                        let mut offset = 0u64;
                                        let mut complete = false;
                                        for _ in 0..MAX_FILE_CHUNKS {
                                            let chunk_bytes = fetch(
                                                auth,
                                                &KvRequest::Chunk {
                                                    file: entry.value,
                                                    offset,
                                                },
                                            )?;
                                            let chunk = KvEncryptedChunk::decode(&chunk_bytes)?;
                                            let clear = open_kv_chunk(
                                                &file_seed,
                                                entry.value,
                                                offset,
                                                &chunk,
                                            )?;
                                            if clear.is_empty() && !chunk.final_chunk {
                                                return Err(Error::KvResponse(
                                                    "empty non-final file chunk",
                                                ));
                                            }
                                            store.append_large_file(&mut stage, &clear)?;
                                            if stage.size > MAX_FILE_BYTES as u64 {
                                                return Err(Error::KvResponse(
                                                    "file size limit exceeded",
                                                ));
                                            }
                                            offset = stage.size;
                                            if chunk.final_chunk {
                                                complete = true;
                                                break;
                                            }
                                        }
                                        if !complete {
                                            return Err(Error::KvResponse(
                                                "file chunk limit exceeded",
                                            ));
                                        }
                                        store.finish_large_file(&stage)?;
                                        let size = stage.size;
                                        large_files.push(stage);
                                        size
                                    };
                                    projected.node_bytes = Some(bytes);
                                    total_content_bytes = total_content_bytes
                                        .checked_add(usize::try_from(size).map_err(|_| {
                                            Error::KvResponse(
                                                "file size does not fit in memory size",
                                            )
                                        })?)
                                        .ok_or(Error::KvResponse("content size overflow"))?;
                                    projected.large_file_size = Some(size);
                                }
                                KvNodeType::None => unreachable!("rejected above"),
                            }
                            entries.push(projected);
                            if total_content_bytes > MAX_TOTAL_CONTENT_BYTES {
                                return Err(Error::KvResponse("total content limit exceeded"));
                            }
                        }
                        if !extended.is_empty() {
                            return Err(Error::KvResponse(
                                "extended entry does not reference a small file",
                            ));
                        }
                        if listing.final_page {
                            final_page = true;
                            break;
                        }
                        cursor = KvListCursor::Mac(
                            last_mac.ok_or(Error::KvResponse("list page lacks pagination MAC"))?,
                        );
                    }
                    if !final_page {
                        return Err(Error::KvResponse("directory pagination limit exceeded"));
                    }
                    entries.sort_by(|left, right| {
                        left.name
                            .cmp(&right.name)
                            .then(left.dirent_id.cmp(&right.dirent_id))
                    });
                    let projection = KvDirectoryProjection {
                        host_id: host.host_id.as_bytes().to_vec(),
                        party_id: party.party.as_bytes().to_vec(),
                        root_version: root.version,
                        root_directory_id: root.root,
                        root_bytes: root_bytes.clone(),
                        directory_id,
                        directory_version: directory.active.version,
                        directory_bytes,
                        entries,
                    };
                    combined.insert(directory_id, projection.clone());
                    projections.push(projection);
                }
                let combined = reachable_kv_tree(root.root, combined)?;
                let final_versions = kv_version_vector(root.version, &combined);
                fetch(auth, &KvRequest::CacheCheck(final_versions))?;
                store.project_reachable_tree_with_large_files(&projections, &large_files)?;
                Ok(store.tree(host.host_id.as_bytes(), party.party.as_bytes())?)
            })();
            match attempt {
                Err(Error::Rpc(foks_rpc::Error::KvStaleCache(_))) => continue,
                result => return result,
            }
        }
        Err(Error::KvResponse(
            "KV cache changed during three synchronization attempts",
        ))
    }
}
