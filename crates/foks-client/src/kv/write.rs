//! Optimistically concurrent KV namespace and content mutations.

use std::io::Read;
use std::time::Duration;

use foks_client_db::KvDirectoryProjection;
use foks_crypto::{derive_kv_keys, seal_kv_chunk, seal_kv_dirent_name};
use foks_proto::{
    KvDirectory, KvDirectoryPair, KvDirectoryStatus, KvDirent, KvNodeId, KvNodeType, KvRoot,
    KvSmallFilePlaintext, Role, RoleAndGeneration, SecretSeed,
};

use super::rpc::KvRequest;
use super::support::{
    bound_dirent, is_kv_stale_cache, kv_directory_reaches, kv_key, kv_version_vector_from_tree,
    random_kv_node_id, read_kv_upload_chunk, read_kv_upload_chunk_with_carry,
    validate_kv_component, validate_kv_symlink,
};
use super::{KvWriteOptions, KvWriteResult, KvWriteSession};
use crate::{now_microseconds, random_bytes, Error, Result};

impl KvWriteSession<'_> {
    const MAX_NAMESPACE_ATTEMPTS: usize = 3;
    pub(crate) const MAX_UPLOAD_CHUNK: usize = 4 * 1024 * 1024;
    const MAX_UPLOAD_BYTES: u64 = 1024 * 1024 * 1024;
    // Upstream reserves eight bytes of Snowpack overhead inside its 2 KiB
    // small-file budget. Larger plaintexts use the chunked-file protocol.
    pub(crate) const SMALL_FILE_BYTES: usize = 2048 - 8;

    pub fn sync(&mut self) -> Result<Vec<KvDirectoryProjection>> {
        let auth = self.auth.borrowed();
        let connection = &mut self.connection;
        self.client.sync_kv_with_fetch(
            &self.host,
            self.party.clone(),
            auth,
            &self.private_keys,
            &self.soft_database_path,
            |auth, request| connection.call(auth, request),
        )
    }

    /// Creates the first root for a party whose KV namespace is absent. The
    /// directory can become unreferenced if `kvPutRoot` loses a race, matching the
    /// upstream two-step protocol; no authenticated namespace state is lost.
    pub fn initialize_root(
        &mut self,
        read_role: Role,
        write_role: Role,
    ) -> Result<Vec<KvDirectoryProjection>> {
        let (seed, key) = self.current_content_key(read_role)?;
        let id = random_bytes()?;
        let directory_seed = SecretSeed::new(random_bytes()?);
        let directory = KvDirectory {
            id,
            version: 1,
            key,
            seed_ciphertext: derive_kv_keys(seed)?.seal_directory_seed(id, &directory_seed)?,
            write_role,
            status: KvDirectoryStatus::Active,
        };
        let binding_mac = derive_kv_keys(seed)?.bind_root(&self.party, id, 1, key)?;
        self.call(KvRequest::Mkdir {
            precondition: None,
            directory,
        })?;
        self.call(KvRequest::PutRoot(KvRoot::new(id, 1, key, binding_mac)?))?;
        self.sync()
    }

    /// Returns the existing namespace or creates it when `kvGetRoot` reports
    /// the exact v0.1.9 `KV_NOENT_ERROR`. Other failures are never converted
    /// into writes.
    pub fn ensure_root(
        &mut self,
        read_role: Role,
        write_role: Role,
    ) -> Result<Vec<KvDirectoryProjection>> {
        match self.sync() {
            Ok(projection) => Ok(projection),
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus { code: 8016, .. })) => {
                self.initialize_root(read_role, write_role)
            }
            Err(error) => Err(error),
        }
    }

    pub fn put_file<R: Read>(
        &mut self,
        parent: [u8; 16],
        name: &str,
        reader: &mut R,
        options: KvWriteOptions,
    ) -> Result<KvWriteResult> {
        validate_kv_component(name.as_bytes())?;
        let tree = self.sync()?;
        let (first, mut carry, first_is_final) = read_kv_upload_chunk(reader)?;
        let first_len = first.len();
        let (content_seed, key) = self.current_content_key(options.read_role)?;
        let (node_id, boxed_small, large_metadata, first_chunk, file_seed) =
            if first_is_final && first.len() <= Self::SMALL_FILE_BYTES {
                let id = random_kv_node_id(KvNodeType::SmallFile)?;
                let boxed = derive_kv_keys(content_seed)?.seal_small_file(
                    id,
                    key,
                    KvSmallFilePlaintext::File(first),
                )?;
                (id, Some(boxed), None, None, None)
            } else {
                let id = random_kv_node_id(KvNodeType::File)?;
                let file_seed = SecretSeed::new(random_bytes()?);
                let metadata = derive_kv_keys(content_seed)?.seal_file_seed(
                    id,
                    key,
                    1,
                    &file_seed,
                    random_bytes()?,
                )?;
                let chunk = seal_kv_chunk(&file_seed, id, 0, first_is_final, &first, 0)?;
                (id, None, Some(metadata), Some(chunk), Some(file_seed))
            };

        if let Some(boxed) = boxed_small {
            self.call(KvRequest::PutSmall { id: node_id, boxed })?;
        } else {
            let metadata = large_metadata.expect("large-file metadata was constructed");
            let mut chunk = first_chunk.expect("large-file first chunk was constructed");
            let file_seed = file_seed.expect("large-file seed was constructed");
            let mut clear_offset = u64::try_from(first_len)
                .map_err(|_| Error::KvResponse("upload offset overflow"))?;
            let mut encrypted_size = u64::try_from(chunk.ciphertext.len())
                .map_err(|_| Error::KvResponse("upload size overflow"))?;
            self.call(KvRequest::UploadInit {
                file_id: node_id.object_id(),
                metadata,
                chunk: chunk.clone(),
            })?;
            while chunk.final_upload.is_none() {
                let (clear, next_carry, final_chunk) =
                    read_kv_upload_chunk_with_carry(reader, std::mem::take(&mut carry))?;
                carry = next_carry;
                let next_offset = clear_offset
                    .checked_add(
                        u64::try_from(clear.len())
                            .map_err(|_| Error::KvResponse("upload offset overflow"))?,
                    )
                    .ok_or(Error::KvResponse("upload offset overflow"))?;
                if next_offset > Self::MAX_UPLOAD_BYTES
                    || (next_offset == Self::MAX_UPLOAD_BYTES && !final_chunk)
                {
                    return Err(Error::KvResponse("upload exceeds FOKS file size limit"));
                }
                chunk = seal_kv_chunk(
                    &file_seed,
                    node_id,
                    clear_offset,
                    final_chunk,
                    &clear,
                    encrypted_size,
                )?;
                clear_offset = next_offset;
                encrypted_size = encrypted_size
                    .checked_add(
                        u64::try_from(chunk.ciphertext.len())
                            .map_err(|_| Error::KvResponse("upload size overflow"))?,
                    )
                    .ok_or(Error::KvResponse("upload size overflow"))?;
                self.call(KvRequest::UploadChunk {
                    file_id: node_id.object_id(),
                    chunk: chunk.clone(),
                })?;
            }
            if clear_offset > Self::MAX_UPLOAD_BYTES || !carry.is_empty() {
                return Err(Error::KvResponse("upload exceeds FOKS file size limit"));
            }
        }

        self.link_uploaded_node(tree, parent, name.as_bytes(), node_id, options)
    }

    pub fn put_symlink(
        &mut self,
        parent: [u8; 16],
        name: &str,
        target: &str,
        options: KvWriteOptions,
    ) -> Result<KvWriteResult> {
        validate_kv_component(name.as_bytes())?;
        validate_kv_symlink(target.as_bytes())?;
        let tree = self.sync()?;
        let (seed, key) = self.current_content_key(options.read_role)?;
        let id = random_kv_node_id(KvNodeType::Symlink)?;
        let boxed = derive_kv_keys(seed)?.seal_small_file(
            id,
            key,
            KvSmallFilePlaintext::Symlink(target.as_bytes().to_vec()),
        )?;
        self.call(KvRequest::PutSmall { id, boxed })?;
        self.link_uploaded_node(tree, parent, name.as_bytes(), id, options)
    }

    pub fn mkdir(
        &mut self,
        parent: [u8; 16],
        name: &str,
        options: KvWriteOptions,
    ) -> Result<KvWriteResult> {
        validate_kv_component(name.as_bytes())?;
        let tree = self.sync()?;
        let (seed, key) = self.current_content_key(options.read_role)?;
        let id = random_bytes()?;
        let directory_seed = SecretSeed::new(random_bytes()?);
        let directory = KvDirectory {
            id,
            version: 1,
            key,
            seed_ciphertext: derive_kv_keys(seed)?.seal_directory_seed(id, &directory_seed)?,
            write_role: options.write_role,
            status: KvDirectoryStatus::Active,
        };
        let mut current_tree = tree;
        for attempt in 0..Self::MAX_NAMESPACE_ATTEMPTS {
            match self.call(KvRequest::Mkdir {
                precondition: Some(kv_version_vector_from_tree(&current_tree)),
                directory: directory.clone(),
            }) {
                Ok(_) => break,
                Err(error)
                    if is_kv_stale_cache(&error) && attempt + 1 < Self::MAX_NAMESPACE_ATTEMPTS =>
                {
                    current_tree = self.sync()?;
                }
                Err(error) => return Err(error),
            }
        }
        let mut node = [0; 17];
        node[0] = KvNodeType::Directory as u8;
        node[1..].copy_from_slice(&id);
        self.link_uploaded_node(
            current_tree,
            parent,
            name.as_bytes(),
            KvNodeId(node),
            options,
        )
    }

    pub fn unlink(
        &mut self,
        parent: [u8; 16],
        name: &str,
        expected_version: Option<u64>,
        write_role: Role,
        recursive: bool,
    ) -> Result<Vec<KvDirectoryProjection>> {
        validate_kv_component(name.as_bytes())?;
        let tree = self.sync()?;
        let projection = tree
            .iter()
            .find(|directory| directory.directory_id == parent)
            .ok_or(Error::KvResponse(
                "parent directory is not in the verified cache",
            ))?;
        let entry = projection
            .entries
            .iter()
            .find(|entry| entry.name == name.as_bytes())
            .ok_or(Error::KvResponse("cannot unlink a missing KV entry"))?;
        if entry.node_id[0] == KvNodeType::Directory as u8 && !recursive {
            return Err(Error::KvResponse(
                "unlinking a directory requires recursive mode",
            ));
        }
        let options = KvWriteOptions {
            read_role: Role::NONE,
            write_role,
            overwrite: true,
            expected_version,
        };
        let (_, tree) = self.mutate_namespace(tree, |session, tree| {
            Ok(vec![session.prepare_dirent(
                tree,
                parent,
                name.as_bytes(),
                KvNodeId([0; 17]),
                options,
                true,
            )?])
        })?;
        Ok(tree)
    }

    pub fn move_entry(
        &mut self,
        source_parent: [u8; 16],
        source_name: &str,
        destination_parent: [u8; 16],
        destination_name: &str,
        options: KvWriteOptions,
    ) -> Result<KvWriteResult> {
        validate_kv_component(source_name.as_bytes())?;
        validate_kv_component(destination_name.as_bytes())?;
        if source_parent == destination_parent && source_name == destination_name {
            return Err(Error::KvResponse("source and destination are identical"));
        }
        let tree = self.sync()?;
        let (dirents, tree) = self.mutate_namespace(tree, |session, tree| {
            let source_directory = tree
                .iter()
                .find(|directory| directory.directory_id == source_parent)
                .ok_or(Error::KvResponse(
                    "source directory is not in the verified cache",
                ))?;
            let source_entry = source_directory
                .entries
                .iter()
                .find(|entry| entry.name == source_name.as_bytes())
                .ok_or(Error::KvResponse("move source does not exist"))?;
            let source = KvDirent::decode(&source_entry.dirent_bytes)?;
            if source.value.node_type()? == KvNodeType::None {
                return Err(Error::KvResponse("move source is tombstoned"));
            }
            if source.value.node_type()? == KvNodeType::Directory
                && kv_directory_reaches(tree, source.value.object_id(), destination_parent)?
            {
                return Err(Error::KvResponse("cannot move a directory into itself"));
            }
            let source_tombstone = session.prepare_dirent(
                tree,
                source_parent,
                source_name.as_bytes(),
                KvNodeId([0; 17]),
                KvWriteOptions {
                    expected_version: Some(source.version),
                    ..options
                },
                true,
            )?;
            let destination = session.prepare_dirent(
                tree,
                destination_parent,
                destination_name.as_bytes(),
                source.value,
                options,
                false,
            )?;
            Ok(vec![source_tombstone, destination])
        })?;
        let destination = &dirents[1];
        Ok(KvWriteResult {
            node_id: destination.value,
            dirent_version: destination.version,
            tree,
        })
    }

    pub fn acquire_lock(
        &mut self,
        parent: [u8; 16],
        dirent: [u8; 16],
        timeout: Duration,
    ) -> Result<[u8; 16]> {
        let timeout_millis = u64::try_from(timeout.as_millis())
            .map_err(|_| Error::KvResponse("KV lock timeout overflow"))?;
        let lock_id = random_bytes()?;
        self.call(KvRequest::LockAcquire {
            parent,
            dirent,
            lock_id,
            timeout_millis,
        })?;
        Ok(lock_id)
    }

    pub fn release_lock(
        &mut self,
        parent: [u8; 16],
        dirent: [u8; 16],
        lock_id: [u8; 16],
    ) -> Result<()> {
        self.call(KvRequest::LockRelease {
            parent,
            dirent,
            lock_id,
        })?;
        Ok(())
    }

    fn link_uploaded_node(
        &mut self,
        tree: Vec<KvDirectoryProjection>,
        parent: [u8; 16],
        name: &[u8],
        node_id: KvNodeId,
        options: KvWriteOptions,
    ) -> Result<KvWriteResult> {
        let (dirents, tree) = self.mutate_namespace(tree, |session, tree| {
            Ok(vec![session.prepare_dirent(
                tree, parent, name, node_id, options, false,
            )?])
        })?;
        Ok(KvWriteResult {
            node_id,
            dirent_version: dirents[0].version,
            tree,
        })
    }

    fn mutate_namespace<F>(
        &mut self,
        mut tree: Vec<KvDirectoryProjection>,
        prepare: F,
    ) -> Result<(Vec<KvDirent>, Vec<KvDirectoryProjection>)>
    where
        F: Fn(&Self, &[KvDirectoryProjection]) -> Result<Vec<KvDirent>>,
    {
        for attempt in 0..Self::MAX_NAMESPACE_ATTEMPTS {
            let dirents = prepare(self, &tree)?;
            match self.call(KvRequest::Put {
                precondition: kv_version_vector_from_tree(&tree),
                dirents: dirents.clone(),
            }) {
                Ok(_) => return Ok((dirents, self.sync()?)),
                Err(error)
                    if is_kv_stale_cache(&error) && attempt + 1 < Self::MAX_NAMESPACE_ATTEMPTS =>
                {
                    tree = self.sync()?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(Error::KvResponse("KV namespace retry limit exhausted"))
    }

    fn call(&mut self, request: KvRequest) -> Result<Vec<u8>> {
        self.connection.call(self.auth.borrowed(), &request)
    }

    fn current_content_key(&self, role: Role) -> Result<(&SecretSeed, RoleAndGeneration)> {
        let generation = self
            .private_keys
            .iter()
            .filter(|key| key.role == role)
            .map(|key| key.generation)
            .max()
            .ok_or(Error::KvResponse(
                "no private key for requested KV read role",
            ))?;
        let matches = self
            .private_keys
            .iter()
            .filter(|key| key.role == role && key.generation == generation)
            .collect::<Vec<_>>();
        let [key] = matches.as_slice() else {
            return Err(Error::KvResponse("duplicate current KV private key"));
        };
        Ok((key.seed, RoleAndGeneration { role, generation }))
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_dirent(
        &self,
        tree: &[KvDirectoryProjection],
        parent: [u8; 16],
        name: &[u8],
        node_id: KvNodeId,
        options: KvWriteOptions,
        unlink: bool,
    ) -> Result<KvDirent> {
        let projection = tree
            .iter()
            .find(|directory| directory.directory_id == parent)
            .ok_or(Error::KvResponse(
                "parent directory is not in the verified cache",
            ))?;
        let pair = KvDirectoryPair::decode(&projection.directory_bytes)?;
        let existing = projection.entries.iter().find(|entry| entry.name == name);
        if unlink && existing.is_none() {
            return Err(Error::KvResponse("cannot unlink a missing KV entry"));
        }
        if let Some(expected) = options.expected_version {
            let actual = existing.map_or(0, |entry| entry.version);
            if actual != expected {
                return Err(Error::KvResponse("KV dirent version precondition failed"));
            }
        }
        if let Some(entry) = existing {
            let old = KvDirent::decode(&entry.dirent_bytes)?;
            let old_type = old.value.node_type()?;
            let new_type = node_id.node_type()?;
            if !unlink && old_type != KvNodeType::None {
                if !options.overwrite {
                    return Err(Error::KvResponse("KV entry already exists"));
                }
                if !matches!(old_type, KvNodeType::File | KvNodeType::SmallFile)
                    || !matches!(
                        new_type,
                        KvNodeType::File | KvNodeType::SmallFile | KvNodeType::Symlink
                    )
                {
                    return Err(Error::KvResponse("incompatible KV overwrite"));
                }
            }
            let seed = self.directory_seed(&pair, old.directory_version)?;
            return bound_dirent(
                &seed,
                old.parent,
                old.id,
                node_id,
                old.version
                    .checked_add(1)
                    .ok_or(Error::KvResponse("KV dirent version overflow"))?,
                old.directory_version,
                options.write_role,
                old.name_mac,
                old.name_box,
                old.directory_status,
                old.creation_time,
            );
        }
        let directory = pair.encrypting.as_ref().unwrap_or(&pair.active);
        let seed = self.directory_seed(&pair, directory.version)?;
        let (name_mac, name_box) = seal_kv_dirent_name(
            &seed,
            parent,
            directory.version,
            name.to_vec(),
            random_bytes()?,
        )?;
        bound_dirent(
            &seed,
            parent,
            random_bytes()?,
            node_id,
            1,
            directory.version,
            options.write_role,
            name_mac,
            name_box,
            directory.status,
            now_microseconds()?,
        )
    }

    fn directory_seed(&self, pair: &KvDirectoryPair, version: u64) -> Result<SecretSeed> {
        let directory = std::iter::once(&pair.active)
            .chain(pair.encrypting.iter())
            .find(|directory| directory.version == version)
            .ok_or(Error::KvResponse("KV directory generation is unavailable"))?;
        kv_key(
            &self.private_keys,
            directory.key.role,
            directory.key.generation,
        )?
        .open_directory_seed(directory)
        .map_err(Into::into)
    }
}
