//! Optimistically concurrent KV namespace and content mutations.

use std::io::Read;
use std::time::Duration;

use foks_client_db::{
    HardStateStore, KvDirectoryProjection, MutationKind, MutationOperation, MutationState,
    SoftStateStore,
};
use foks_crypto::{derive_kv_keys, seal_kv_chunk, seal_kv_dirent_name};
use foks_proto::{
    KvDirectory, KvDirectoryPair, KvDirectoryStatus, KvDirent, KvNodeId, KvNodeType, KvRoot,
    KvSmallFilePlaintext, Role, RoleAndGeneration, SecretSeed,
};
use foks_snowpack::{decode, encode, Value};
use zeroize::Zeroizing;

use super::rpc::KvRequest;
use super::support::{
    bound_dirent, is_kv_stale_cache, kv_directory_reaches, kv_key, kv_version_vector_from_tree,
    random_kv_node_id, read_kv_upload_chunk, read_kv_upload_chunk_with_carry,
    validate_kv_component, validate_kv_symlink,
};
use super::{KvWriteOptions, KvWriteResult, KvWriteSession};
use crate::{now_microseconds, random_bytes, Error, MutationCoordinator, MutationDraft, Result};

const KV_NAMESPACE_REQUEST_HASH_TYPE_ID: u64 = 0x4303_44d4_219a_8b1e;
const KV_ROOT_REQUEST_HASH_TYPE_ID: u64 = 0x1dca_75c8_c2dd_b868;

impl KvWriteSession<'_> {
    const MAX_NAMESPACE_ATTEMPTS: usize = 3;
    pub(crate) const MAX_UPLOAD_CHUNK: usize = super::MAX_KV_UPLOAD_CHUNK;
    const MAX_UPLOAD_BYTES: u64 = super::MAX_KV_FILE_BYTES;
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
    /// directory can be orphaned if `kvPutRoot` loses a race, matching the
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
        self.put_root_journaled(KvRoot::new(id, 1, key, binding_mac)?)
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
            let request = KvRequest::Put {
                precondition: kv_version_vector_from_tree(&tree),
                dirents: dirents.clone(),
            };
            let operation = self.prepare_namespace_mutation(&request, &dirents)?;
            MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                .begin_submission(&operation.operation_id)?;
            let submission = self.call(request);
            if let Err(error) = submission {
                if is_kv_stale_cache(&error) {
                    MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                        .rejected(&operation.operation_id)?;
                    if attempt + 1 < Self::MAX_NAMESPACE_ATTEMPTS {
                        tree = self.sync()?;
                        continue;
                    }
                    return Err(error);
                }
                if matches!(error, Error::Rpc(foks_rpc::Error::RemoteStatus { .. })) {
                    MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                        .rejected(&operation.operation_id)?;
                    return Err(error);
                }
                MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                    .submission_unknown(&operation.operation_id)?;
                return match self.reconcile_namespace_mutation(&operation, &dirents) {
                    Ok(tree) => Ok((dirents, tree)),
                    Err(_) => Err(error),
                };
            }
            let projected = self.reconcile_namespace_mutation(&operation, &dirents)?;
            return Ok((dirents, projected));
        }
        Err(Error::KvResponse("KV namespace retry limit exhausted"))
    }

    /// Recovers one durable namespace outbox entry. Prepared operations submit
    /// their sequence-1 frame on a fresh connection; ambiguous operations only
    /// synchronize and compare authenticated dirents.
    pub fn resume_namespace_mutation(
        &mut self,
        operation_id: [u8; 16],
    ) -> Result<Vec<KvDirectoryProjection>> {
        let operation = HardStateStore::open(&self.host.database_path)?
            .mutation(&operation_id)?
            .ok_or(Error::OperationBinding(
                "KV namespace mutation is not recorded",
            ))?;
        if operation.kind != MutationKind::KvNamespace
            || operation.host_id != self.host.host_id.as_bytes()
            || operation.scope_id != self.party.party.as_bytes()
        {
            return Err(Error::OperationBinding(
                "KV namespace mutation belongs to another party",
            ));
        }
        let material =
            MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                .load_bound_material(&operation)?;
        let (precondition, dirents) = decode_namespace_material(&material)?;
        if foks_crypto::prefixed_hash(KV_NAMESPACE_REQUEST_HASH_TYPE_ID, &material)
            != operation.request_hash
        {
            return Err(Error::OperationBinding(
                "KV namespace outbox request fingerprint changed",
            ));
        }
        validate_namespace_operation_binding(&operation, &precondition, &dirents)?;
        match operation.state {
            MutationState::Prepared => {
                MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                    .begin_submission(&operation_id)?;
                if let Err(error) = self.call(KvRequest::Put {
                    precondition,
                    dirents: dirents.clone(),
                }) {
                    if is_kv_stale_cache(&error)
                        || matches!(error, Error::Rpc(foks_rpc::Error::RemoteStatus { .. }))
                    {
                        MutationCoordinator::new(
                            &self.host.database_path,
                            &mut *self.protected_store,
                        )
                        .rejected(&operation_id)?;
                        return Err(error);
                    }
                    MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                        .submission_unknown(&operation_id)?;
                }
            }
            MutationState::Submitting | MutationState::SubmissionUnknown => {}
            MutationState::Verified | MutationState::Rejected => {
                return Err(Error::OperationBinding("KV namespace mutation is terminal"));
            }
        }
        self.reconcile_namespace_mutation(&operation, &dirents)
    }

    /// Recovers creation of a party's first root. As with namespace writes,
    /// only `Prepared` may submit; ambiguous states are resolved by loading
    /// and authenticating the server root.
    pub fn resume_root_mutation(
        &mut self,
        operation_id: [u8; 16],
    ) -> Result<Vec<KvDirectoryProjection>> {
        let operation = HardStateStore::open(&self.host.database_path)?
            .mutation(&operation_id)?
            .ok_or(Error::OperationBinding("KV root mutation is not recorded"))?;
        if operation.kind != MutationKind::KvRoot
            || operation.host_id != self.host.host_id.as_bytes()
            || operation.scope_id != self.party.party.as_bytes()
        {
            return Err(Error::OperationBinding(
                "KV root mutation belongs to another party",
            ));
        }
        let material =
            MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                .load_bound_material(&operation)?;
        if foks_crypto::prefixed_hash(KV_ROOT_REQUEST_HASH_TYPE_ID, &material)
            != operation.request_hash
        {
            return Err(Error::OperationBinding(
                "KV root outbox fingerprint changed",
            ));
        }
        let root = KvRoot::decode(&material)?;
        if operation.subject_id != root.root || operation.expected_version != Some(root.version) {
            return Err(Error::OperationBinding(
                "KV root outbox public binding differs from protected material",
            ));
        }
        match operation.state {
            MutationState::Prepared => {
                MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                    .begin_submission(&operation_id)?;
                if let Err(error) = self.call(KvRequest::PutRoot(root.clone())) {
                    if matches!(error, Error::Rpc(foks_rpc::Error::RemoteStatus { .. })) {
                        MutationCoordinator::new(
                            &self.host.database_path,
                            &mut *self.protected_store,
                        )
                        .rejected(&operation_id)?;
                        return Err(error);
                    }
                    MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                        .submission_unknown(&operation_id)?;
                }
            }
            MutationState::Submitting | MutationState::SubmissionUnknown => {}
            MutationState::Verified | MutationState::Rejected => {
                return Err(Error::OperationBinding("KV root mutation is terminal"));
            }
        }
        self.reconcile_root_mutation(&operation, &root)
    }

    fn put_root_journaled(&mut self, root: KvRoot) -> Result<Vec<KvDirectoryProjection>> {
        let operation_id = random_bytes()?;
        let material = Zeroizing::new(root.encoded().to_vec());
        let operation =
            MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                .prepare(
                    MutationDraft {
                        operation_id,
                        kind: MutationKind::KvRoot,
                        host_id: self.host.host_id.as_bytes().to_vec(),
                        scope_id: self.party.party.as_bytes().to_vec(),
                        subject_id: root.root.to_vec(),
                        expected_version: Some(root.version),
                        request_hash: foks_crypto::prefixed_hash(
                            KV_ROOT_REQUEST_HASH_TYPE_ID,
                            &material,
                        ),
                    },
                    material,
                )?;
        MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
            .begin_submission(&operation_id)?;
        if let Err(error) = self.call(KvRequest::PutRoot(root.clone())) {
            if matches!(error, Error::Rpc(foks_rpc::Error::RemoteStatus { .. })) {
                MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                    .rejected(&operation_id)?;
                return Err(error);
            }
            MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
                .submission_unknown(&operation_id)?;
            return match self.reconcile_root_mutation(&operation, &root) {
                Ok(tree) => Ok(tree),
                Err(_) => Err(error),
            };
        }
        self.reconcile_root_mutation(&operation, &root)
    }

    fn reconcile_root_mutation(
        &mut self,
        operation: &MutationOperation,
        root: &KvRoot,
    ) -> Result<Vec<KvDirectoryProjection>> {
        let tree = self.sync()?;
        let projected = tree.first().ok_or(Error::TransitionNotObserved(
            "KV root was not reflected by synchronization",
        ))?;
        if projected.root_directory_id != root.root
            || projected.root_version != root.version
            || projected.root_bytes != root.encoded()
        {
            return Err(Error::TransitionNotObserved(
                "KV root differs from the durable outbox",
            ));
        }
        MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
            .verified(&operation.operation_id)?;
        Ok(tree)
    }

    fn prepare_namespace_mutation(
        &mut self,
        request: &KvRequest,
        dirents: &[KvDirent],
    ) -> Result<MutationOperation> {
        let precondition = match request {
            KvRequest::Put { precondition, .. } => precondition,
            _ => {
                return Err(Error::KvResponse(
                    "only kvPut can enter the namespace outbox",
                ));
            }
        };
        let material = encode_namespace_material(precondition, dirents)?;
        let operation_id = random_bytes()?;
        let subject_id = namespace_subject(dirents);
        let expected_version = Some(precondition.root_version);
        MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store).prepare(
            MutationDraft {
                operation_id,
                kind: MutationKind::KvNamespace,
                host_id: self.host.host_id.as_bytes().to_vec(),
                scope_id: self.party.party.as_bytes().to_vec(),
                subject_id,
                expected_version,
                request_hash: foks_crypto::prefixed_hash(
                    KV_NAMESPACE_REQUEST_HASH_TYPE_ID,
                    &material,
                ),
            },
            material,
        )
    }

    fn reconcile_namespace_mutation(
        &mut self,
        operation: &MutationOperation,
        dirents: &[KvDirent],
    ) -> Result<Vec<KvDirectoryProjection>> {
        let affected = dirents
            .iter()
            .map(|dirent| dirent.parent)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        SoftStateStore::open(&self.soft_database_path)?.invalidate_directories(
            self.host.host_id.as_bytes(),
            self.party.party.as_bytes(),
            &affected,
        )?;
        let tree = self.sync()?;
        for expected in dirents {
            let projected = tree
                .iter()
                .find(|directory| directory.directory_id == expected.parent)
                .and_then(|directory| {
                    directory
                        .entries
                        .iter()
                        .find(|entry| entry.dirent_id == expected.id)
                });
            if expected.value.node_type()? == KvNodeType::None {
                if projected.is_some() {
                    return Err(Error::TransitionNotObserved(
                        "KV tombstone was not reflected by synchronization",
                    ));
                }
            } else if projected.is_none_or(|entry| {
                entry.version != expected.version || entry.node_id != expected.value.0
            }) {
                return Err(Error::TransitionNotObserved(
                    "KV dirent mutation was not reflected by synchronization",
                ));
            }
        }
        MutationCoordinator::new(&self.host.database_path, &mut *self.protected_store)
            .verified(&operation.operation_id)?;
        Ok(tree)
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

fn encode_namespace_material(
    precondition: &foks_proto::KvPathVersionVector,
    dirents: &[KvDirent],
) -> Result<Zeroizing<Vec<u8>>> {
    Ok(Zeroizing::new(encode(&Value::Array(vec![
        precondition.to_value(),
        Value::Array(
            dirents
                .iter()
                .map(|dirent| Value::Binary(dirent.encoded().to_vec()))
                .collect(),
        ),
    ]))?))
}

fn decode_namespace_material(
    material: &[u8],
) -> Result<(foks_proto::KvPathVersionVector, Vec<KvDirent>)> {
    let Value::Array(fields) = decode(material)? else {
        return Err(Error::OperationBinding(
            "KV namespace outbox material is malformed",
        ));
    };
    let [precondition, Value::Array(encoded_dirents)] = fields.as_slice() else {
        return Err(Error::OperationBinding(
            "KV namespace outbox material has the wrong shape",
        ));
    };
    if encoded_dirents.is_empty() || encoded_dirents.len() > 2 {
        return Err(Error::OperationBinding(
            "KV namespace outbox material has invalid limits",
        ));
    }
    let dirents = encoded_dirents
        .iter()
        .map(|value| match value {
            Value::Binary(bytes) => KvDirent::decode(bytes).map_err(Error::from),
            _ => Err(Error::OperationBinding(
                "KV namespace outbox dirent is malformed",
            )),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((
        foks_proto::KvPathVersionVector::from_value(precondition)?,
        dirents,
    ))
}

fn namespace_subject(dirents: &[KvDirent]) -> Vec<u8> {
    let parents = dirents
        .iter()
        .map(|dirent| dirent.parent)
        .collect::<std::collections::BTreeSet<_>>();
    if parents.len() == 1 {
        parents.iter().next().expect("one parent").to_vec()
    } else {
        Vec::new()
    }
}

fn validate_namespace_operation_binding(
    operation: &MutationOperation,
    precondition: &foks_proto::KvPathVersionVector,
    dirents: &[KvDirent],
) -> Result<()> {
    if operation.subject_id != namespace_subject(dirents)
        || operation.expected_version != Some(precondition.root_version)
    {
        return Err(Error::OperationBinding(
            "KV namespace outbox public binding differs from protected material",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod outbox_tests {
    use super::*;

    #[test]
    fn namespace_outbox_preserves_exact_request_and_dirents() {
        let dirent = KvDirent::decode(include_bytes!(
            "../../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/kv-write-dirent.snowp"
        ))
        .unwrap();
        let precondition = foks_proto::KvPathVersionVector {
            root_version: 3,
            directories: Vec::new(),
        };
        let material =
            encode_namespace_material(&precondition, std::slice::from_ref(&dirent)).unwrap();
        let (decoded_precondition, decoded_dirents) = decode_namespace_material(&material).unwrap();
        assert_eq!(decoded_precondition, precondition);
        assert_eq!(decoded_dirents.len(), 1);
        assert_eq!(decoded_dirents[0].encoded(), dirent.encoded());

        let operation = MutationOperation {
            operation_id: [1; 16],
            kind: MutationKind::KvNamespace,
            host_id: vec![2; 32],
            scope_id: vec![3; 33],
            subject_id: dirent.parent.to_vec(),
            expected_version: Some(precondition.root_version),
            request_hash: foks_crypto::prefixed_hash(KV_NAMESPACE_REQUEST_HASH_TYPE_ID, &material),
            material_ref: vec![4; 16],
            material_hash: [5; 32],
            state: MutationState::Prepared,
            attempt_count: 0,
            created_at: 6,
            updated_at: 6,
        };
        validate_namespace_operation_binding(&operation, &precondition, &decoded_dirents).unwrap();

        let mut changed = operation.clone();
        changed.expected_version = Some(precondition.root_version + 1);
        assert!(
            validate_namespace_operation_binding(&changed, &precondition, &decoded_dirents)
                .is_err()
        );
        changed = operation;
        changed.subject_id[0] ^= 1;
        assert!(
            validate_namespace_operation_binding(&changed, &precondition, &decoded_dirents)
                .is_err()
        );

        assert!(decode_namespace_material(&[0xc0]).is_err());
    }
}
