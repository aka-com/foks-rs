use foks_rpc::RpcStatus;
use foks_snowpack::{decode, encode, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::auth::Principal;
use crate::rpc::RouteId;
use crate::WriterHandle;

pub(crate) enum Response {
    Data(Vec<u8>),
    Void,
}

struct LockFields {
    parent: [u8; 16],
    dirent: [u8; 16],
    lock: [u8; 16],
}

#[derive(Clone)]
struct KvAuthority {
    party: Vec<u8>,
    actor: Vec<u8>,
    token_hash: Option<[u8; 32]>,
    maximum_role: foks_proto::Role,
    key_generations: BTreeMap<foks_proto::Role, u64>,
    clock: Arc<dyn foks_server_db::Clock>,
}

trait KvContextRead {
    fn active_credential_owner(
        &self,
        uid: &[u8],
        credential: &[u8],
    ) -> foks_server_db::Result<Option<Vec<u8>>>;
    fn user_authority(
        &self,
        uid: &[u8],
    ) -> foks_server_db::Result<Option<foks_server_db::UserAuthoritySnapshot>>;
    fn resolve_team_view_token(
        &self,
        token_hash: &[u8; 32],
        now: u64,
    ) -> foks_server_db::Result<Option<foks_server_db::TeamViewAuthoritySnapshot>>;
    fn team(&self, team_id: &[u8]) -> foks_server_db::Result<Option<foks_server_db::TeamSnapshot>>;
}

macro_rules! impl_kv_read {
    ($reader:ty) => {
        impl KvContextRead for $reader {
            fn active_credential_owner(
                &self,
                uid: &[u8],
                credential: &[u8],
            ) -> foks_server_db::Result<Option<Vec<u8>>> {
                self.active_credential_owner(uid, credential)
            }

            fn user_authority(
                &self,
                uid: &[u8],
            ) -> foks_server_db::Result<Option<foks_server_db::UserAuthoritySnapshot>> {
                self.user_authority(uid)
            }

            fn resolve_team_view_token(
                &self,
                token_hash: &[u8; 32],
                now: u64,
            ) -> foks_server_db::Result<Option<foks_server_db::TeamViewAuthoritySnapshot>> {
                self.resolve_team_view_token(token_hash, now)
            }

            fn team(
                &self,
                team_id: &[u8],
            ) -> foks_server_db::Result<Option<foks_server_db::TeamSnapshot>> {
                self.team(team_id)
            }
        }
    };
}

impl_kv_read!(foks_server_db::ReadDatabase);
impl_kv_read!(foks_server_db::ReadSnapshot<'_>);

pub(crate) fn dispatch(
    route: RouteId,
    argument: &[u8],
    principal: &Principal,
    reader: crate::read_pool::ReadLease,
    writer: &WriterHandle,
    clock: &Arc<dyn foks_server_db::Clock>,
    chunk_store: Option<&dyn crate::chunks::ChunkStore>,
) -> Result<Response, RpcStatus> {
    if route == RouteId::KvStoreGetEncryptedChunk {
        let chunk = {
            let snapshot = reader.snapshot().map_err(|_| RpcStatus::TransactionRetry)?;
            let authority = resolve_authority(argument, principal, &snapshot, clock)?;
            get_encrypted_chunk(argument, &snapshot, &authority)?
        };
        // Returning a snapshot is not enough: the pooled connection must also
        // become available before a remote store can block this transfer.
        drop(reader);
        let offset = chunk.offset;
        let final_chunk = chunk.final_chunk;
        let ciphertext = crate::chunk_storage::ciphertext(chunk, chunk_store)
            .map_err(|error| crate::status::unmapped(error, "kv_chunk_read"))?;
        return Ok(Response::Data(
            foks_proto::KvEncryptedChunk {
                ciphertext,
                offset,
                final_chunk,
            }
            .encode()
            .map_err(|_| RpcStatus::TransactionRetry)?,
        ));
    }
    if matches!(
        route,
        RouteId::KvStoreGetRoot
            | RouteId::KvStoreGet
            | RouteId::KvStoreGetDir
            | RouteId::KvStoreGetNode
            | RouteId::KvStoreList
            | RouteId::KvStoreCacheCheck
            | RouteId::KvStoreUsage
    ) {
        let snapshot = reader.snapshot().map_err(|_| RpcStatus::TransactionRetry)?;
        let authority = resolve_authority(argument, principal, &snapshot, clock)?;
        return match route {
            RouteId::KvStoreGetRoot => get_root(argument, &snapshot, &authority),
            RouteId::KvStoreGet => get(argument, &snapshot, &authority),
            RouteId::KvStoreGetDir => get_directory(argument, &snapshot, &authority),
            RouteId::KvStoreGetNode => get_node(argument, &snapshot, &authority),
            RouteId::KvStoreList => list(argument, &snapshot, &authority),
            RouteId::KvStoreCacheCheck => cache_check(argument, &snapshot, &authority),
            RouteId::KvStoreUsage => usage(argument, &snapshot, &authority),
            _ => unreachable!("read-only KV routes were matched above"),
        };
    }
    let authority = {
        let snapshot = reader.snapshot().map_err(|_| RpcStatus::TransactionRetry)?;
        resolve_authority(argument, principal, &snapshot, clock)?
    };
    // Upload authority is only a preflight; write_file_chunk rechecks it in the
    // authoritative writer both before publication and before metadata commit.
    if matches!(
        route,
        RouteId::KvStoreFileUploadInit | RouteId::KvStoreFileUploadChunk
    ) {
        drop(reader);
        return if route == RouteId::KvStoreFileUploadInit {
            file_upload_init(argument, principal, writer, &authority, chunk_store)
        } else {
            file_upload_chunk(argument, principal, writer, &authority, chunk_store)
        };
    }
    match route {
        RouteId::KvStoreMkdir => mkdir(argument, principal, reader, writer, &authority),
        RouteId::KvStoreFileUploadInit => {
            file_upload_init(argument, principal, writer, &authority, chunk_store)
        }
        RouteId::KvStoreFileUploadChunk => {
            file_upload_chunk(argument, principal, writer, &authority, chunk_store)
        }
        RouteId::KvStorePutSmallFileOrSymlink => {
            put_small_file_or_symlink(argument, principal, reader, writer, &authority)
        }
        RouteId::KvStorePut => put(argument, principal, reader, writer, &authority),
        RouteId::KvStorePutRoot => put_root(argument, principal, reader, writer, &authority),
        RouteId::KvStoreLockAcquire => {
            lock_acquire(argument, principal, reader, writer, &authority)
        }
        RouteId::KvStoreLockRelease => {
            lock_release(argument, principal, reader, writer, &authority)
        }
        RouteId::KvStoreGetRoot
        | RouteId::KvStoreGet
        | RouteId::KvStoreGetDir
        | RouteId::KvStoreGetNode
        | RouteId::KvStoreGetEncryptedChunk
        | RouteId::KvStoreList
        | RouteId::KvStoreCacheCheck
        | RouteId::KvStoreUsage => unreachable!("read-only KV routes returned above"),
        _ => Err(RpcStatus::Unsupported),
    }
}

fn get_root(
    argument: &[u8],
    reader: &foks_server_db::ReadSnapshot<'_>,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let _fields = fields(argument, 1)?;
    let uid = authority.party.clone();
    let root = reader
        .kv_root(&uid)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    let decoded =
        foks_proto::KvRoot::decode(&root.exact).map_err(|_| RpcStatus::TransactionRetry)?;
    authority.require_read_key(decoded.key)?;
    Ok(Response::Data(root.exact))
}

fn get(
    argument: &[u8],
    reader: &foks_server_db::ReadSnapshot<'_>,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 3)?;
    let precondition = request_precondition(&fields[0])?;
    check_precondition(reader, authority, precondition.as_ref())?;
    let Value::Array(path) = &fields[1] else {
        return Err(bad_arguments("KV get path is not a struct"));
    };
    let [parent, names] = path.as_slice() else {
        return Err(bad_arguments("KV get path has the wrong shape"));
    };
    let parent = fixed_16(parent, "KV get parent")?;
    let names = match names {
        Value::Null => return Err(RpcStatus::KvNoEnt),
        Value::Array(names) if !names.is_empty() && names.len() <= 64 => names,
        Value::Array(_) => return Err(bad_arguments("KV get names are empty or too numerous")),
        _ => return Err(bad_arguments("KV get names are not a list")),
    };
    let Value::Unsigned(follow) = &fields[2] else {
        return Err(bad_arguments("KV follow behavior is not unsigned"));
    };
    if *follow > 2 {
        return Err(bad_arguments("KV follow behavior is unknown"));
    }
    let mut found = None;
    for name in names {
        let Value::Array(name) = name else {
            return Err(bad_arguments("KV get name is not a struct"));
        };
        let [Value::Unsigned(directory_version), name_mac] = name.as_slice() else {
            return Err(bad_arguments("KV get name has the wrong shape"));
        };
        let name_mac = fixed_32(name_mac, "KV get name MAC")?;
        if let Some(stored) = reader
            .kv_dirent_at_name(&authority.party, &parent, *directory_version, &name_mac)
            .map_err(|_| RpcStatus::TransactionRetry)?
        {
            found = Some(stored);
            break;
        }
    }
    let stored = found.ok_or(RpcStatus::KvNoEnt)?;
    let directory = foks_proto::KvDirectory::decode(
        stored
            .exact_directory
            .as_deref()
            .ok_or(RpcStatus::TransactionRetry)?,
    )
    .map_err(|_| RpcStatus::TransactionRetry)?;
    authority.require_read_key(directory.key)?;
    let mut dirent =
        foks_proto::KvDirent::decode(&stored.exact).map_err(|_| RpcStatus::TransactionRetry)?;
    if dirent.parent != parent || dirent.value.0 != stored.node_id {
        return Err(RpcStatus::TransactionRetry);
    }
    // go-foks omits ctime from kvGet and includes it only in kvList.
    dirent
        .set_creation_time(0)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let node_type = dirent
        .value
        .node_type()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let should_follow = match (*follow, node_type) {
        (_, foks_proto::KvNodeType::None) | (0, _) => false,
        (1, foks_proto::KvNodeType::Directory) | (2, _) => true,
        _ => false,
    };
    let node = should_follow
        .then(|| load_node(reader, authority, dirent.value.0))
        .transpose()?;
    let response =
        foks_proto::KvGetResponse::new(dirent, node).map_err(|_| RpcStatus::TransactionRetry)?;
    Ok(Response::Data(response.encoded().to_vec()))
}

fn usage(
    argument: &[u8],
    reader: &foks_server_db::ReadSnapshot<'_>,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let _fields = fields(argument, 1)?;
    let usage = reader
        .kv_usage(&authority.party)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    Ok(Response::Data(
        usage.encode().map_err(|_| RpcStatus::TransactionRetry)?,
    ))
}

fn mkdir(
    argument: &[u8],
    principal: &Principal,
    reader: crate::read_pool::ReadLease,
    writer: &WriterHandle,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    let Value::Array(header) = &fields[0] else {
        return Err(bad_arguments("KV request header is not a struct"));
    };
    let [_auth, precondition] = header.as_slice() else {
        return Err(bad_arguments("KV request header has the wrong shape"));
    };
    let precondition = match precondition {
        Value::Null => None,
        value => Some(foks_proto::KvPathVersionVector::from_value(value).map_err(bad_arguments)?),
    };
    let snapshot = reader.snapshot().map_err(|_| RpcStatus::TransactionRetry)?;
    check_precondition(&snapshot, authority, precondition.as_ref())?;
    let exact = encode(&fields[1]).map_err(bad_arguments)?;
    let directory = foks_proto::KvDirectory::decode(&exact).map_err(bad_arguments)?;
    if directory.version != 1
        || directory.status != foks_proto::KvDirectoryStatus::Active
        || !authority.can_write_key(directory.key)
        || !authority.can_write_role(directory.write_role)
    {
        return Err(bad_arguments(
            "initial KV directory must be active version one",
        ));
    }
    let (key_role, key_visibility) = role_parts(directory.key.role);
    let uid = authority.party.clone();
    let device = principal.device_id().to_vec();
    let write_authority = authority.clone();
    drop(snapshot);
    drop(reader);
    writer
        .call_with_current_time(Arc::clone(&write_authority.clock), move |database, now| {
            if !kv_write_is_current(database, &write_authority, &device, now)? {
                return Err(crate::Error::AuthorizationChanged);
            }
            database.put_kv_directory_with_precondition(
                &foks_server_db::KvDirectoryMutation {
                    uid: &uid,
                    id: &directory.id,
                    version: directory.version,
                    key_role,
                    key_visibility,
                    key_generation: directory.key.generation,
                    status: 0,
                    exact: &exact,
                },
                precondition.as_ref(),
            )?;
            Ok(())
        })
        .map_err(map_write_error)?;
    Ok(Response::Void)
}

fn put_root(
    argument: &[u8],
    principal: &Principal,
    reader: crate::read_pool::ReadLease,
    writer: &WriterHandle,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    let exact = encode(&fields[1]).map_err(bad_arguments)?;
    let root = foks_proto::KvRoot::decode(&exact).map_err(bad_arguments)?;
    if authority.maximum_role < foks_proto::Role::ADMIN {
        return Err(permission_denied());
    }
    if root.version == 0 || root.binding_mac == [0; 32] || !authority.can_write_key(root.key) {
        return Err(bad_arguments(
            "KV root has invalid version, binding, or key metadata",
        ));
    }
    let (key_role, key_visibility) = role_parts(root.key.role);
    let uid = authority.party.clone();
    let device = principal.device_id().to_vec();
    let write_authority = authority.clone();
    drop(reader);
    writer
        .call_with_current_time(Arc::clone(&write_authority.clock), move |database, now| {
            if !kv_write_is_current(database, &write_authority, &device, now)? {
                return Err(crate::Error::AuthorizationChanged);
            }
            let directory_version = database
                .kv_directory(&uid, &root.root)?
                .ok_or(foks_server_db::Error::KvConflict)?
                .version;
            database.put_kv_root(&foks_server_db::KvRootMutation {
                uid: &uid,
                version: root.version,
                directory_id: &root.root,
                directory_version,
                key_role,
                key_visibility,
                key_generation: root.key.generation,
                exact: &exact,
            })?;
            Ok(())
        })
        .map_err(map_write_error)?;
    Ok(Response::Void)
}

fn file_upload_init(
    argument: &[u8],
    principal: &Principal,
    writer: &WriterHandle,
    authority: &KvAuthority,
    chunk_store: Option<&dyn crate::chunks::ChunkStore>,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 4)?;
    let file_id = fixed_16(&fields[1], "KV file ID")?;
    let exact_metadata = encode(&fields[2]).map_err(bad_arguments)?;
    let metadata =
        foks_proto::KvLargeFileMetadata::decode(&exact_metadata).map_err(bad_arguments)?;
    if !authority.can_write_key(metadata.key) || metadata.version != 1 {
        return Err(bad_arguments("KV file uses unsupported metadata"));
    }
    put_file_chunk(
        &fields[3],
        Some(exact_metadata),
        file_id,
        principal,
        writer,
        authority,
        chunk_store,
    )
}

fn file_upload_chunk(
    argument: &[u8],
    principal: &Principal,
    writer: &WriterHandle,
    authority: &KvAuthority,
    chunk_store: Option<&dyn crate::chunks::ChunkStore>,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 3)?;
    put_file_chunk(
        &fields[2],
        None,
        fixed_16(&fields[1], "KV file ID")?,
        principal,
        writer,
        authority,
        chunk_store,
    )
}

fn put_file_chunk(
    value: &Value,
    exact_metadata: Option<Vec<u8>>,
    file_id: [u8; 16],
    principal: &Principal,
    writer: &WriterHandle,
    authority: &KvAuthority,
    chunk_store: Option<&dyn crate::chunks::ChunkStore>,
) -> Result<Response, RpcStatus> {
    let exact_chunk = encode(value).map_err(bad_arguments)?;
    let chunk = foks_proto::KvUploadChunk::decode(&exact_chunk).map_err(bad_arguments)?;
    if chunk.ciphertext.is_empty() || chunk.ciphertext.len() > 9 * 1024 * 1024 {
        return Err(bad_arguments("KV upload chunk size is out of range"));
    }
    let upload = Arc::new(PendingFileChunk {
        uid: authority.party.clone(),
        file_id,
        exact_metadata,
        chunk,
        exact_chunk,
    });
    let reservation = if let Some(store) = chunk_store {
        let digest = crate::chunks::chunk_digest(&upload.exact_chunk);
        let Some(token) = reserve_file_chunk(
            writer,
            authority,
            principal.device_id(),
            upload.clone(),
            digest,
            store.publication_policy(),
        )?
        else {
            return Ok(Response::Void); // authenticated exact metadata replay
        };
        match token.action {
            foks_server_db::ChunkReservationAction::Publish => {
                let result = store.put_object(&token.object(), &upload.exact_chunk);
                writer
                    .settle_chunk_publication(token.clone(), result.is_ok())
                    .map_err(map_upload_error)?;
                result.map_err(map_upload_error)?;
            }
            foks_server_db::ChunkReservationAction::Verify => {
                let bytes = store
                    .get_object(&token.object())
                    .map_err(map_upload_error)?;
                if bytes != upload.exact_chunk {
                    return Err(crate::status::invariant("kv_upload_verify"));
                }
                writer
                    .confirm_chunk_presence(token.clone())
                    .map_err(map_upload_error)?;
            }
            foks_server_db::ChunkReservationAction::Present => {}
        }
        Some(token)
    } else {
        None
    };
    write_file_chunk(
        writer,
        authority,
        principal.device_id(),
        upload,
        reservation,
    )?;
    Ok(Response::Void)
}

struct PendingFileChunk {
    uid: Vec<u8>,
    file_id: [u8; 16],
    exact_metadata: Option<Vec<u8>>,
    chunk: foks_proto::KvUploadChunk,
    exact_chunk: Vec<u8>,
}

fn reserve_file_chunk(
    writer: &WriterHandle,
    authority: &KvAuthority,
    credential: &[u8],
    upload: Arc<PendingFileChunk>,
    digest: [u8; 32],
    policy: foks_server_db::ChunkPublicationPolicy,
) -> Result<Option<foks_server_db::ChunkReservation>, RpcStatus> {
    let device = credential.to_vec();
    let authority = authority.clone();
    let mut attempt = [0; 16];
    crate::Entropy::fill(&crate::OsEntropy, &mut attempt)
        .map_err(|error| crate::status::internal(error, "kv_upload_attempt"))?;
    writer
        .call_with_current_time(Arc::clone(&authority.clock), move |database, now| {
            if !kv_write_is_current(database, &authority, &device, now)? {
                return Err(crate::Error::AuthorizationChanged);
            }
            let mutation = upload.mutation(now);
            database.preflight_kv_file_chunk(&mutation)?;
            if database.kv_file_chunk_exists(&upload.uid, &upload.file_id, upload.chunk.offset)? {
                return Ok(None);
            }
            Ok(Some(database.reserve_chunk_object_with_policy(
                &digest,
                upload.exact_chunk.len() as u64,
                &attempt,
                now,
                policy,
            )?))
        })
        .map_err(map_upload_error)
}
impl PendingFileChunk {
    fn mutation(&self, now: u64) -> foks_server_db::KvFileChunkMutation<'_> {
        foks_server_db::KvFileChunkMutation {
            uid: &self.uid,
            file_id: &self.file_id,
            exact_metadata: self.exact_metadata.as_deref(),
            offset: self.chunk.offset,
            ciphertext: &self.chunk.ciphertext,
            final_size: self.chunk.final_upload.as_ref().map(|v| v.size),
            exact_chunk: &self.exact_chunk,
            now,
        }
    }
}
fn write_file_chunk(
    writer: &WriterHandle,
    authority: &KvAuthority,
    credential: &[u8],
    upload: Arc<PendingFileChunk>,
    reservation: Option<foks_server_db::ChunkReservation>,
) -> Result<(), RpcStatus> {
    let device = credential.to_vec();
    let authority = authority.clone();
    writer
        .call_with_current_time(Arc::clone(&authority.clock), move |database, now| {
            if !kv_write_is_current(database, &authority, &device, now)? {
                return Err(crate::Error::AuthorizationChanged);
            }
            let mutation = upload.mutation(now);
            if let Some(token) = reservation {
                database.validate_chunk_reservation(&token)?;
                database.put_kv_file_chunk_external(&mutation, &token.digest)?;
            } else {
                database.put_kv_file_chunk(&mutation)?;
            }
            Ok(())
        })
        .map_err(map_upload_error)
}

fn put_small_file_or_symlink(
    argument: &[u8],
    principal: &Principal,
    reader: crate::read_pool::ReadLease,
    writer: &WriterHandle,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 3)?;
    let id = fixed_17(&fields[1], "KV node ID")?;
    if !matches!(id[0], 3 | 4) {
        return Err(bad_arguments("KV node is not a small file or symlink"));
    }
    let exact = encode(&fields[2]).map_err(bad_arguments)?;
    let boxed = foks_proto::KvSmallFileBox::decode(&exact).map_err(bad_arguments)?;
    // go-foks limits the encrypted small-file payload to 2 KiB of plaintext
    // plus Secretbox's 16-byte authenticator. Matching that boundary keeps a
    // namespace written here portable to an upstream server.
    if boxed.ciphertext.len() > 2_048 + 16 {
        return Err(bad_arguments("KV small node exceeds the Go payload limit"));
    }
    if !authority.can_write_key(boxed.key) {
        return Err(bad_arguments("KV node uses an unsupported content key"));
    }
    let uid = authority.party.clone();
    let device = principal.device_id().to_vec();
    let write_authority = authority.clone();
    drop(reader);
    writer
        .call_with_current_time(Arc::clone(&write_authority.clock), move |database, now| {
            if !kv_write_is_current(database, &write_authority, &device, now)? {
                return Err(crate::Error::AuthorizationChanged);
            }
            database.put_kv_node(&foks_server_db::KvNodeMutation {
                uid: &uid,
                id: &id,
                node_type: u64::from(id[0]),
                exact: &exact,
            })?;
            Ok(())
        })
        .map_err(map_write_error)?;
    Ok(Response::Void)
}

fn put(
    argument: &[u8],
    principal: &Principal,
    reader: crate::read_pool::ReadLease,
    writer: &WriterHandle,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    let precondition = request_precondition(&fields[0])?;
    let snapshot = reader.snapshot().map_err(|_| RpcStatus::TransactionRetry)?;
    check_precondition(&snapshot, authority, precondition.as_ref())?;
    let Value::Array(values) = &fields[1] else {
        return Err(bad_arguments("KV dirents are not a list"));
    };
    if values.is_empty() || values.len() > 64 {
        return Err(bad_arguments(
            "KV put batch must contain 1 through 64 dirents",
        ));
    }
    let dirents = values
        .iter()
        .map(|value| {
            let exact = encode(value).map_err(bad_arguments)?;
            let dirent = foks_proto::KvDirent::decode(&exact).map_err(bad_arguments)?;
            if dirent.version == 0
                || dirent.directory_version == 0
                || !authority.can_write_role(dirent.write_role)
                || dirent.directory_status != foks_proto::KvDirectoryStatus::Active
            {
                return Err(bad_arguments(
                    "KV dirent uses unsupported mutation metadata",
                ));
            }
            Ok(dirent)
        })
        .collect::<Result<Vec<_>, RpcStatus>>()?;
    for dirent in &dirents {
        require_directory_write_access(&reader, authority, &dirent.parent)?;
    }
    let uid = authority.party.clone();
    let device = principal.device_id().to_vec();
    let write_authority = authority.clone();
    drop(snapshot);
    drop(reader);
    writer
        .call_with_current_time(Arc::clone(&write_authority.clock), move |database, now| {
            if !kv_write_is_current(database, &write_authority, &device, now)? {
                return Err(crate::Error::AuthorizationChanged);
            }
            let mut dirents = dirents;
            for dirent in &mut dirents {
                let creation_time = database
                    .kv_dirent(&uid, &dirent.parent, &dirent.id)?
                    .map(|stored| foks_proto::KvDirent::decode(&stored.exact))
                    .transpose()?
                    .filter(|stored| stored.version == dirent.version)
                    .map_or(now, |stored| stored.creation_time);
                dirent.set_creation_time(creation_time)?;
            }
            let exact = dirents
                .iter()
                .map(foks_proto::KvDirent::encode)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mutations = dirents
                .iter()
                .zip(&exact)
                .map(|(dirent, exact)| foks_server_db::KvDirentMutation {
                    parent: &dirent.parent,
                    id: &dirent.id,
                    version: dirent.version,
                    directory_version: dirent.directory_version,
                    node_id: &dirent.value.0,
                    name_mac: &dirent.name_mac,
                    creation_time: dirent.creation_time,
                    exact,
                })
                .collect::<Vec<_>>();
            database.put_kv_dirents(
                &uid,
                precondition.as_ref(),
                write_authority.maximum_role,
                &mutations,
            )?;
            Ok(())
        })
        .map_err(map_write_error)?;
    Ok(Response::Void)
}

fn get_node(
    argument: &[u8],
    reader: &foks_server_db::ReadSnapshot<'_>,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    let id = fixed_17(&fields[1], "KV node ID")?;
    let node = load_node(reader, authority, id)?;
    Ok(Response::Data(
        node.encoded().map_err(|_| RpcStatus::TransactionRetry)?,
    ))
}

fn load_node(
    reader: &foks_server_db::ReadSnapshot<'_>,
    authority: &KvAuthority,
    id: [u8; 17],
) -> Result<foks_proto::KvNode, RpcStatus> {
    let object_id: [u8; 16] = id[1..].try_into().expect("KV node ID width was checked");
    match id[0] {
        1 => {
            let stored = reader
                .kv_directory(&authority.party, &object_id)
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or(RpcStatus::KvNoEnt)?;
            let directory = foks_proto::KvDirectory::decode(&stored.exact)
                .map_err(|_| RpcStatus::TransactionRetry)?;
            authority.require_read_key(directory.key)?;
            Ok(foks_proto::KvNode::Directory(
                foks_proto::KvDirectoryPair::from_active(directory)
                    .map_err(|_| RpcStatus::TransactionRetry)?,
            ))
        }
        2 => {
            let stored = reader
                .kv_file(&authority.party, &object_id)
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or(RpcStatus::KvNoEnt)?;
            let metadata = foks_proto::KvLargeFileMetadata::decode(&stored.exact_metadata)
                .map_err(|_| RpcStatus::TransactionRetry)?;
            authority.require_read_key(metadata.key)?;
            Ok(foks_proto::KvNode::File(metadata))
        }
        3 | 4 => {
            let stored = reader
                .kv_node(&authority.party, &id)
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or(RpcStatus::KvNoEnt)?;
            let boxed = foks_proto::KvSmallFileBox::decode(&stored.exact)
                .map_err(|_| RpcStatus::TransactionRetry)?;
            authority.require_read_key(boxed.key)?;
            Ok(if stored.node_type == 3 {
                foks_proto::KvNode::SmallFile(boxed)
            } else {
                foks_proto::KvNode::Symlink(boxed)
            })
        }
        _ => Err(RpcStatus::KvNoEnt),
    }
}

fn get_encrypted_chunk(
    argument: &[u8],
    reader: &foks_server_db::ReadSnapshot<'_>,
    authority: &KvAuthority,
) -> Result<foks_server_db::StoredKvFileChunk, RpcStatus> {
    let fields = fields(argument, 3)?;
    let id = fixed_16(&fields[1], "KV file ID")?;
    let Value::Unsigned(offset) = &fields[2] else {
        return Err(bad_arguments("KV chunk offset is not unsigned"));
    };
    let uid = authority.party.clone();
    let file = reader
        .kv_file(&uid, &id)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    let metadata = foks_proto::KvLargeFileMetadata::decode(&file.exact_metadata)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    authority.require_read_key(metadata.key)?;
    let chunk = reader
        .kv_file_chunk(&uid, &id, *offset)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    Ok(chunk)
}

fn get_directory(
    argument: &[u8],
    reader: &foks_server_db::ReadSnapshot<'_>,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    let id = fixed_16(&fields[1], "KV directory ID")?;
    let uid = authority.party.clone();
    let stored = reader
        .kv_directory(&uid, &id)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    let directory =
        foks_proto::KvDirectory::decode(&stored.exact).map_err(|_| RpcStatus::TransactionRetry)?;
    authority.require_read_key(directory.key)?;
    let pair = foks_proto::KvDirectoryPair::from_active(directory)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    Ok(Response::Data(pair.encoded().to_vec()))
}

fn list(
    argument: &[u8],
    reader: &foks_server_db::ReadSnapshot<'_>,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let foks_rpc::arguments::KvListArguments {
        parent: id,
        cursor,
        number,
        load_small,
    } = foks_rpc::arguments::decode_kv_list_arguments(argument)?;
    let cursor = match cursor {
        foks_rpc::KvListCursor::None => None,
        foks_rpc::KvListCursor::Mac(mac) => Some(foks_server_db::KvListCursor::Mac(mac)),
        foks_rpc::KvListCursor::Time(time) => Some(foks_server_db::KvListCursor::Time(time)),
    };
    let uid = authority.party.clone();
    let parent = reader
        .kv_directory(&uid, &id)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    let parent =
        foks_proto::KvDirectory::decode(&parent.exact).map_err(|_| RpcStatus::TransactionRetry)?;
    authority.require_read_key(parent.key)?;
    let limit =
        usize::try_from(number).map_err(|_| bad_arguments("KV pagination count overflows"))?;
    let stored = reader
        .kv_list(&uid, &id, cursor, limit)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let final_page = stored.len() < limit;
    let mut entries = Vec::with_capacity(stored.len());
    let mut extended = Vec::new();
    for (position, stored) in stored.into_iter().enumerate() {
        let entry =
            foks_proto::KvDirent::decode(&stored.exact).map_err(|_| RpcStatus::TransactionRetry)?;
        if load_small && stored.node_id[0] == 3 {
            let Some(node) = reader
                .kv_node(&uid, &stored.node_id)
                .map_err(|_| RpcStatus::TransactionRetry)?
            else {
                entries.push(entry);
                continue;
            };
            let small_file = foks_proto::KvSmallFileBox::decode(&node.exact)
                .map_err(|_| RpcStatus::TransactionRetry)?;
            // Go's list path uses failOnPermError=false: the dirent remains in
            // the page, while an unreadable or absent optional small-file body
            // is omitted from the extended-entry side table.
            if authority.require_read_key(small_file.key).is_ok() {
                extended.push(foks_proto::KvExtendedDirent {
                    position: u64::try_from(position).map_err(|_| RpcStatus::TransactionRetry)?,
                    small_file,
                });
            }
        }
        entries.push(entry);
    }
    let response = foks_proto::KvListResponse::new(entries, final_page, extended)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    Ok(Response::Data(response.encoded().to_vec()))
}

fn cache_check(
    argument: &[u8],
    reader: &foks_server_db::ReadSnapshot<'_>,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 1)?;
    let Value::Array(inner) = &fields[0] else {
        return Err(bad_arguments("KV cache check is not a struct"));
    };
    let [_auth, versions] = inner.as_slice() else {
        return Err(bad_arguments("KV cache check has the wrong shape"));
    };
    let supplied = match versions {
        Value::Null => None,
        value => Some(foks_proto::KvPathVersionVector::from_value(value).map_err(bad_arguments)?),
    };
    check_precondition(reader, authority, supplied.as_ref())?;
    Ok(Response::Void)
}

fn lock_acquire(
    argument: &[u8],
    principal: &Principal,
    reader: crate::read_pool::ReadLease,
    writer: &WriterHandle,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 3)?;
    let LockFields {
        parent,
        dirent,
        lock,
    } = lock_fields(&fields[1])?;
    let Value::Unsigned(timeout_millis) = &fields[2] else {
        return Err(bad_arguments("KV lock timeout is not unsigned"));
    };
    let duration = timeout_millis
        .checked_mul(1000)
        .ok_or_else(|| bad_arguments("KV lock duration overflows"))?;
    require_directory_write_access(&reader, authority, &parent)?;
    let uid = authority.party.clone();
    let device = principal.device_id().to_vec();
    let write_authority = authority.clone();
    drop(reader);
    writer
        .call_with_current_time(Arc::clone(&write_authority.clock), move |database, now| {
            if !kv_write_is_current(database, &write_authority, &device, now)? {
                return Err(crate::Error::AuthorizationChanged);
            }
            database.acquire_kv_lock(&uid, &parent, &dirent, &lock, now, duration)?;
            Ok(())
        })
        .map_err(map_lock_error)?;
    Ok(Response::Void)
}

fn lock_release(
    argument: &[u8],
    principal: &Principal,
    reader: crate::read_pool::ReadLease,
    writer: &WriterHandle,
    authority: &KvAuthority,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    let LockFields {
        parent,
        dirent,
        lock,
    } = lock_fields(&fields[1])?;
    require_directory_write_access(&reader, authority, &parent)?;
    let uid = authority.party.clone();
    let device = principal.device_id().to_vec();
    let write_authority = authority.clone();
    drop(reader);
    writer
        .call_with_current_time(Arc::clone(&write_authority.clock), move |database, now| {
            if !kv_write_is_current(database, &write_authority, &device, now)? {
                return Err(crate::Error::AuthorizationChanged);
            }
            database.release_kv_lock(&uid, &parent, &dirent, &lock)?;
            Ok(())
        })
        .map_err(map_lock_error)?;
    Ok(Response::Void)
}

fn request_precondition(
    value: &Value,
) -> Result<Option<foks_proto::KvPathVersionVector>, RpcStatus> {
    let Value::Array(header) = value else {
        return Err(bad_arguments("KV request header is not a struct"));
    };
    let [_auth, precondition] = header.as_slice() else {
        return Err(bad_arguments("KV request header has the wrong shape"));
    };
    match precondition {
        Value::Null => Ok(None),
        value => foks_proto::KvPathVersionVector::from_value(value)
            .map(Some)
            .map_err(bad_arguments),
    }
}

fn check_precondition(
    reader: &foks_server_db::ReadSnapshot<'_>,
    authority: &KvAuthority,
    supplied: Option<&foks_proto::KvPathVersionVector>,
) -> Result<(), RpcStatus> {
    let Some(supplied) = supplied else {
        return Ok(());
    };
    // Go checks read access to every cited directory and the current root
    // before exposing any cache-version delta to the caller.
    for directory in &supplied.directories {
        let stored = reader
            .kv_directory(&authority.party, &directory.id)
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or_else(|| RpcStatus::NotFound("cached directory not found".to_owned()))?;
        let stored = foks_proto::KvDirectory::decode(&stored.exact)
            .map_err(|_| RpcStatus::TransactionRetry)?;
        authority.require_read_key(stored.key)?;
    }
    let root = reader
        .kv_root(&authority.party)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(|| RpcStatus::NotFound("cached root not found".to_owned()))?;
    let root = foks_proto::KvRoot::decode(&root.exact).map_err(|_| RpcStatus::TransactionRetry)?;
    authority.require_read_key(root.key)?;
    match reader
        .kv_version_check(&authority.party, supplied)
        .map_err(|_| RpcStatus::TransactionRetry)?
    {
        foks_server_db::KvVersionCheck::Current => {}
        foks_server_db::KvVersionCheck::Stale(delta) => {
            return Err(RpcStatus::StaleCache(delta));
        }
        foks_server_db::KvVersionCheck::Future => {
            return Err(bad_arguments("KV cache version is ahead of the server"));
        }
        foks_server_db::KvVersionCheck::Missing => {
            return Err(RpcStatus::NotFound("cached KV object not found".to_owned()));
        }
    }
    Ok(())
}

fn require_directory_write_access(
    reader: &foks_server_db::ReadDatabase,
    authority: &KvAuthority,
    directory_id: &[u8; 16],
) -> Result<(), RpcStatus> {
    let stored = reader
        .kv_directory(&authority.party, directory_id)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    let directory =
        foks_proto::KvDirectory::decode(&stored.exact).map_err(|_| RpcStatus::TransactionRetry)?;
    authority.require_read_key(directory.key)?;
    authority.require_write_role(directory.write_role)
}

fn resolve_authority(
    argument: &[u8],
    principal: &Principal,
    reader: &dyn KvContextRead,
    clock: &Arc<dyn foks_server_db::Clock>,
) -> Result<KvAuthority, RpcStatus> {
    // Keep the complete decoded argument alive through authority SQL. Variant
    // classification intentionally follows the original fallible clock read.
    let auth = foks_rpc::arguments::KvAuthorityArgument::decode(argument)?;
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    match auth.classify()? {
        foks_rpc::arguments::KvAuthoritySelection::Personal => {
            if reader
                .active_credential_owner(principal.uid(), principal.device_id())
                .map_err(|_| RpcStatus::TransactionRetry)?
                .is_none()
            {
                return Err(permission_denied());
            }
            let user = reader
                .user_authority(principal.uid())
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or_else(permission_denied)?;
            let maximum_role = user
                .devices
                .iter()
                .find(|device| {
                    device.active
                        && (device.device_id.as_slice() == principal.device_id()
                            || device.subkey_id.as_deref() == Some(principal.device_id()))
                })
                .and_then(|device| {
                    crate::auth::team::stored_role(device.role_type, device.visibility)
                })
                .ok_or_else(permission_denied)?;
            let key_generations = key_generations(&user.shared_keys)?;
            Ok(KvAuthority {
                party: principal.uid().to_vec(),
                actor: principal.uid().to_vec(),
                token_hash: None,
                maximum_role,
                key_generations,
                clock: Arc::clone(clock),
            })
        }
        foks_rpc::arguments::KvAuthoritySelection::Team(token) => {
            let token_hash = crate::auth::team::token_hash(&token);
            let authority = reader
                .resolve_team_view_token(&token_hash, now)
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or(RpcStatus::Expired)?;
            if authority.member_id.as_slice() != principal.uid() {
                return Err(permission_denied());
            }
            let maximum_role = crate::auth::team::stored_role(
                authority.effective_role_type,
                authority.effective_visibility,
            )
            .ok_or(RpcStatus::TransactionRetry)?;
            let team = reader
                .team(&authority.team_id)
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or(RpcStatus::Expired)?;
            let key_generations = key_generations(&team.shared_keys)?;
            Ok(KvAuthority {
                party: authority.team_id,
                actor: authority.member_id,
                token_hash: Some(token_hash),
                maximum_role,
                key_generations,
                clock: Arc::clone(clock),
            })
        }
    }
}

impl KvAuthority {
    fn can_write_role(&self, role: foks_proto::Role) -> bool {
        role <= self.maximum_role
    }

    fn require_write_role(&self, role: foks_proto::Role) -> Result<(), RpcStatus> {
        self.can_write_role(role)
            .then_some(())
            .ok_or_else(permission_denied)
    }

    fn can_write_key(&self, key: foks_proto::RoleAndGeneration) -> bool {
        self.can_write_role(key.role)
            && self.key_generations.get(&key.role) == Some(&key.generation)
    }

    fn require_read_key(&self, key: foks_proto::RoleAndGeneration) -> Result<(), RpcStatus> {
        (key.role <= self.maximum_role
            && key.generation > 0
            && self
                .key_generations
                .get(&key.role)
                .is_some_and(|current| key.generation <= *current))
        .then_some(())
        .ok_or_else(permission_denied)
    }
}

fn key_generations(
    keys: &[foks_server_db::UserSharedKeySnapshot],
) -> Result<BTreeMap<foks_proto::Role, u64>, RpcStatus> {
    let mut generations: BTreeMap<foks_proto::Role, u64> = BTreeMap::new();
    for key in keys {
        let role = crate::auth::team::stored_role(key.role_type, key.visibility)
            .ok_or(RpcStatus::TransactionRetry)?;
        generations
            .entry(role)
            .and_modify(|generation| *generation = (*generation).max(key.generation))
            .or_insert(key.generation);
    }
    Ok(generations)
}

/// Qualification oracle: retain the native whole-history authority computation
/// while comparing the hosted scalar key decisions. Booleans deliberately do
/// not invent route-specific error precedence or replace public dispatch.
#[cfg(feature = "celld-proof")]
pub(crate) fn qualify_personal_key_requirements(
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    clock: &Arc<dyn foks_server_db::Clock>,
    requirements: &[foks_metadata_core::personal::KeyRequirement],
) -> Result<Vec<bool>, RpcStatus> {
    use foks_metadata_core::personal::KeyAccess;

    principal.require_ordinary_device()?;
    let argument = encode(&Value::Array(vec![Value::Array(vec![
        Value::Unsigned(0),
        Value::Variant(None),
    ])]))
    .map_err(|_| RpcStatus::TransactionRetry)?;
    let authority = resolve_authority(&argument, principal, reader, clock)?;
    Ok(requirements
        .iter()
        .map(|requirement| match requirement.access {
            KeyAccess::Read => authority.require_read_key(requirement.key).is_ok(),
            KeyAccess::Write => authority.can_write_key(requirement.key),
        })
        .collect())
}

fn kv_write_is_current(
    database: &mut foks_server_db::Database,
    authority: &KvAuthority,
    credential: &[u8],
    now: u64,
) -> crate::Result<bool> {
    if database
        .active_credential_owner(&authority.actor, credential)?
        .is_none()
    {
        return Ok(false);
    }
    let authorized = match authority.token_hash {
        None => {
            let Some(user) = database.user_authority(&authority.actor)? else {
                return Ok(false);
            };
            let current_role = user.devices.iter().find_map(|device| {
                (device.active
                    && (device.device_id.as_slice() == credential
                        || device.subkey_id.as_deref() == Some(credential)))
                .then(|| crate::auth::team::stored_role(device.role_type, device.visibility))
                .flatten()
            });
            authority.party == authority.actor
                && current_role == Some(authority.maximum_role)
                && key_generations(&user.shared_keys)
                    .is_ok_and(|keys| keys == authority.key_generations)
        }
        Some(token_hash) => {
            let token_current = database.team_view_token_is_current(
                &token_hash,
                &authority.party,
                &authority.actor,
                now,
            )?;
            let keys_current = database
                .team(&authority.party)?
                .and_then(|team| key_generations(&team.shared_keys).ok())
                .is_some_and(|keys| keys == authority.key_generations);
            token_current && keys_current
        }
    };
    Ok(authorized && database.ensure_kv_namespace(&authority.party)?)
}

fn fields(argument: &[u8], expected: usize) -> Result<Vec<Value>, RpcStatus> {
    let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments("KV argument is not a struct"));
    };
    if fields.len() != expected {
        return Err(bad_arguments("KV argument has the wrong shape"));
    }
    Ok(fields)
}

fn fixed_16(value: &Value, kind: &'static str) -> Result<[u8; 16], RpcStatus> {
    let Value::Binary(bytes) = value else {
        return Err(bad_arguments(format_args!("{kind} is not binary")));
    };
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| bad_arguments(format_args!("{kind} has the wrong width")))
}

fn fixed_17(value: &Value, kind: &'static str) -> Result<[u8; 17], RpcStatus> {
    let Value::Binary(bytes) = value else {
        return Err(bad_arguments(format_args!("{kind} is not binary")));
    };
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| bad_arguments(format_args!("{kind} has the wrong width")))
}

fn fixed_32(value: &Value, kind: &'static str) -> Result<[u8; 32], RpcStatus> {
    let Value::Binary(bytes) = value else {
        return Err(bad_arguments(format_args!("{kind} is not binary")));
    };
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| bad_arguments(format_args!("{kind} has the wrong width")))
}

fn lock_fields(value: &Value) -> Result<LockFields, RpcStatus> {
    let Value::Array(lock) = value else {
        return Err(bad_arguments("KV lock is not a struct"));
    };
    let [target, lock_id] = lock.as_slice() else {
        return Err(bad_arguments("KV lock has the wrong shape"));
    };
    let Value::Array(target) = target else {
        return Err(bad_arguments("KV lock target is not a struct"));
    };
    let [parent, dirent] = target.as_slice() else {
        return Err(bad_arguments("KV lock target has the wrong shape"));
    };
    Ok(LockFields {
        parent: fixed_16(parent, "KV lock parent")?,
        dirent: fixed_16(dirent, "KV lock dirent")?,
        lock: fixed_16(lock_id, "KV lock ID")?,
    })
}

fn role_parts(role: foks_proto::Role) -> (u64, i64) {
    (
        role.protocol_value(),
        i64::from(role.visibility().unwrap_or(0)),
    )
}

pub(super) fn map_write_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::Database(foks_server_db::Error::KvConflict) => {
            RpcStatus::KvRace("KV mutation lost a version race".to_owned())
        }
        crate::Error::Database(foks_server_db::Error::KvPermission) => RpcStatus::KvPermission {
            operation: 2,
            resource: 1,
        },
        other => crate::status::unmapped(other, "kv_write"),
    }
}

pub(super) fn map_upload_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::Database(foks_server_db::Error::KvConflict) => RpcStatus::KvNoEnt,
        crate::Error::Database(
            foks_server_db::Error::Invalid(_) | foks_server_db::Error::IntegerRange,
        ) => bad_arguments("invalid KV upload transition"),
        // The physical chunk capacity admits no further publication now.
        crate::Error::Database(foks_server_db::Error::ChunkAdmission(_)) => RpcStatus::RateLimited,
        other => crate::status::unmapped(other, "kv_upload"),
    }
}

pub(super) fn map_lock_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::Database(foks_server_db::Error::KvLocked) => RpcStatus::Locked,
        crate::Error::Database(foks_server_db::Error::KvLockTimeout) => RpcStatus::LockTimeout,
        crate::Error::Database(foks_server_db::Error::KvConflict) => RpcStatus::KvNoEnt,
        crate::Error::Database(
            foks_server_db::Error::Invalid(_) | foks_server_db::Error::IntegerRange,
        ) => bad_arguments("invalid KV lock transition"),
        other => crate::status::unmapped(other, "kv_lock"),
    }
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

fn permission_denied() -> RpcStatus {
    RpcStatus::PermissionDenied("active device does not authorize this KV namespace".to_owned())
}

#[cfg(feature = "celld-proof")]
pub(crate) fn qualify_personal_usage(
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    clock: &Arc<dyn foks_server_db::Clock>,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let argument = encode(&Value::Array(vec![Value::Array(vec![
        Value::Unsigned(0),
        Value::Variant(None),
    ])]))
    .map_err(|_| RpcStatus::TransactionRetry)?;
    let snapshot = reader.snapshot().map_err(|_| RpcStatus::TransactionRetry)?;
    let authority = resolve_authority(&argument, principal, &snapshot, clock)?;
    match usage(&argument, &snapshot, &authority)? {
        Response::Data(bytes) => Ok(bytes),
        Response::Void => Err(RpcStatus::TransactionRetry),
    }
}

#[cfg(feature = "celld-proof")]
pub(crate) fn qualify_list(
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    clock: &Arc<dyn foks_server_db::Clock>,
    argument: &[u8],
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let snapshot = reader.snapshot().map_err(|_| RpcStatus::TransactionRetry)?;
    let authority = resolve_authority(argument, principal, &snapshot, clock)?;
    match list(argument, &snapshot, &authority)? {
        Response::Data(bytes) => Ok(bytes),
        Response::Void => Err(RpcStatus::TransactionRetry),
    }
}
