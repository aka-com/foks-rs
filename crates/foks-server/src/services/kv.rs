use foks_rpc::RpcStatus;
use foks_snowpack::{decode, encode, Value};

use crate::auth::Principal;
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

pub(crate) fn dispatch(
    method: &str,
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    clock: &dyn foks_server_db::Clock,
) -> Result<Response, RpcStatus> {
    match method {
        "getRoot" => get_root(argument, principal, reader),
        "mkdir" => mkdir(argument, principal, reader, writer),
        "fileUploadInit" => file_upload_init(argument, principal, reader, writer, clock),
        "fileUploadChunk" => file_upload_chunk(argument, principal, reader, writer, clock),
        "putSmallFileOrSymlink" => put_small_file_or_symlink(argument, principal, reader, writer),
        "put" => put(argument, principal, reader, writer),
        "putRoot" => put_root(argument, principal, reader, writer),
        "getDir" => get_directory(argument, principal, reader),
        "getNode" => get_node(argument, principal, reader),
        "getEncryptedChunk" => get_encrypted_chunk(argument, principal, reader),
        "list" => list(argument, principal, reader),
        "cacheCheck" => cache_check(argument, principal, reader),
        "lockAcquire" => lock_acquire(argument, principal, reader, writer, clock),
        "lockRelease" => lock_release(argument, principal, reader, writer),
        _ => Err(RpcStatus::Unsupported),
    }
}

fn get_root(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 1)?;
    require_user_auth(&fields[0])?;
    let uid = active_uid(reader, principal)?;
    let root = reader
        .kv_root(&uid)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    foks_proto::KvRoot::decode(&root.exact).map_err(|_| RpcStatus::TransactionRetry)?;
    Ok(Response::Data(root.exact))
}

fn mkdir(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    let Value::Array(header) = &fields[0] else {
        return Err(bad_arguments("KV request header is not a struct"));
    };
    let [auth, precondition] = header.as_slice() else {
        return Err(bad_arguments("KV request header has the wrong shape"));
    };
    require_user_auth(auth)?;
    let precondition = match precondition {
        Value::Null => None,
        value => Some(foks_proto::KvPathVersionVector::from_value(value).map_err(bad_arguments)?),
    };
    let exact = encode(&fields[1]).map_err(bad_arguments)?;
    let directory = foks_proto::KvDirectory::decode(&exact).map_err(bad_arguments)?;
    if directory.version != 1
        || directory.status != foks_proto::KvDirectoryStatus::Active
        || directory.key.role != foks_proto::Role::OWNER
        || directory.key.generation != 1
        || directory.write_role != foks_proto::Role::OWNER
    {
        return Err(bad_arguments(
            "initial KV directory must be active version one",
        ));
    }
    let (key_role, key_visibility) = role_parts(directory.key.role);
    let uid = active_uid(reader, principal)?;
    let error_uid = uid.clone();
    let device = *principal.device_id();
    writer
        .call(move |database| {
            if !database.is_active_device(&uid, &device)? {
                return Err(crate::Error::Database(foks_server_db::Error::KvConflict));
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
        .map_err(|error| map_write_error(error, reader, &error_uid))?;
    Ok(Response::Void)
}

fn put_root(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    require_user_auth(&fields[0])?;
    let exact = encode(&fields[1]).map_err(bad_arguments)?;
    let root = foks_proto::KvRoot::decode(&exact).map_err(bad_arguments)?;
    if root.version != 1 || root.key.role != foks_proto::Role::OWNER || root.key.generation != 1 {
        return Err(bad_arguments("initial KV root must be version one"));
    }
    let (key_role, key_visibility) = role_parts(root.key.role);
    let uid = active_uid(reader, principal)?;
    let error_uid = uid.clone();
    let device = *principal.device_id();
    writer
        .call(move |database| {
            if !database.is_active_device(&uid, &device)? {
                return Err(crate::Error::Database(foks_server_db::Error::KvConflict));
            }
            database.put_kv_root(&foks_server_db::KvRootMutation {
                uid: &uid,
                version: root.version,
                directory_id: &root.root,
                directory_version: 1,
                key_role,
                key_visibility,
                key_generation: root.key.generation,
                exact: &exact,
            })?;
            Ok(())
        })
        .map_err(|error| map_write_error(error, reader, &error_uid))?;
    Ok(Response::Void)
}

fn file_upload_init(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    clock: &dyn foks_server_db::Clock,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 4)?;
    require_user_auth(&fields[0])?;
    let file_id = fixed_16(&fields[1], "KV file ID")?;
    let exact_metadata = encode(&fields[2]).map_err(bad_arguments)?;
    let metadata =
        foks_proto::KvLargeFileMetadata::decode(&exact_metadata).map_err(bad_arguments)?;
    if metadata.key.role != foks_proto::Role::OWNER
        || metadata.key.generation != 1
        || metadata.version != 1
    {
        return Err(bad_arguments("KV file uses unsupported metadata"));
    }
    put_file_chunk(
        &fields[3],
        Some(exact_metadata),
        file_id,
        principal,
        reader,
        writer,
        clock,
    )
}

fn file_upload_chunk(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    clock: &dyn foks_server_db::Clock,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 3)?;
    require_user_auth(&fields[0])?;
    put_file_chunk(
        &fields[2],
        None,
        fixed_16(&fields[1], "KV file ID")?,
        principal,
        reader,
        writer,
        clock,
    )
}

fn put_file_chunk(
    value: &Value,
    exact_metadata: Option<Vec<u8>>,
    file_id: [u8; 16],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    clock: &dyn foks_server_db::Clock,
) -> Result<Response, RpcStatus> {
    let exact_chunk = encode(value).map_err(bad_arguments)?;
    let chunk = foks_proto::KvUploadChunk::decode(&exact_chunk).map_err(bad_arguments)?;
    if chunk.ciphertext.is_empty() || chunk.ciphertext.len() > 9 * 1024 * 1024 {
        return Err(bad_arguments("KV upload chunk size is out of range"));
    }
    let final_size = chunk
        .final_upload
        .as_ref()
        .map(|final_upload| final_upload.size);
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let uid = active_uid(reader, principal)?;
    let device = *principal.device_id();
    writer
        .call(move |database| {
            if !database.is_active_device(&uid, &device)? {
                return Err(crate::Error::Database(foks_server_db::Error::KvConflict));
            }
            database.put_kv_file_chunk(&foks_server_db::KvFileChunkMutation {
                uid: &uid,
                file_id: &file_id,
                exact_metadata: exact_metadata.as_deref(),
                offset: chunk.offset,
                ciphertext: &chunk.ciphertext,
                final_size,
                exact_chunk: &exact_chunk,
                now,
            })?;
            Ok(())
        })
        .map_err(map_upload_error)?;
    Ok(Response::Void)
}

fn put_small_file_or_symlink(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 3)?;
    require_user_auth(&fields[0])?;
    let id = fixed_17(&fields[1], "KV node ID")?;
    if !matches!(id[0], 3 | 4) {
        return Err(bad_arguments("KV node is not a small file or symlink"));
    }
    let exact = encode(&fields[2]).map_err(bad_arguments)?;
    if exact.len() > 64 * 1024 {
        return Err(bad_arguments("encoded KV small node exceeds 64 KiB"));
    }
    let boxed = foks_proto::KvSmallFileBox::decode(&exact).map_err(bad_arguments)?;
    if boxed.key.role != foks_proto::Role::OWNER || boxed.key.generation != 1 {
        return Err(bad_arguments("KV node uses an unsupported content key"));
    }
    let uid = active_uid(reader, principal)?;
    let error_uid = uid.clone();
    let device = *principal.device_id();
    writer
        .call(move |database| {
            if !database.is_active_device(&uid, &device)? {
                return Err(crate::Error::Database(foks_server_db::Error::KvConflict));
            }
            database.put_kv_node(&foks_server_db::KvNodeMutation {
                uid: &uid,
                id: &id,
                node_type: u64::from(id[0]),
                exact: &exact,
            })?;
            Ok(())
        })
        .map_err(|error| map_write_error(error, reader, &error_uid))?;
    Ok(Response::Void)
}

fn put(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    let Value::Array(header) = &fields[0] else {
        return Err(bad_arguments("KV request header is not a struct"));
    };
    let [auth, precondition] = header.as_slice() else {
        return Err(bad_arguments("KV request header has the wrong shape"));
    };
    require_user_auth(auth)?;
    let precondition = match precondition {
        Value::Null => return Err(bad_arguments("KV put requires a cache precondition")),
        value => foks_proto::KvPathVersionVector::from_value(value).map_err(bad_arguments)?,
    };
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
                || dirent.write_role != foks_proto::Role::OWNER
                || dirent.directory_status != foks_proto::KvDirectoryStatus::Active
            {
                return Err(bad_arguments(
                    "KV dirent uses unsupported mutation metadata",
                ));
            }
            Ok((dirent, exact))
        })
        .collect::<Result<Vec<_>, RpcStatus>>()?;
    let uid = active_uid(reader, principal)?;
    let error_uid = uid.clone();
    let device = *principal.device_id();
    writer
        .call(move |database| {
            if !database.is_active_device(&uid, &device)? {
                return Err(crate::Error::Database(foks_server_db::Error::KvConflict));
            }
            let mutations = dirents
                .iter()
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
            database.put_kv_dirents(&uid, &precondition, &mutations)?;
            Ok(())
        })
        .map_err(|error| map_write_error(error, reader, &error_uid))?;
    Ok(Response::Void)
}

fn get_node(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    require_user_auth(&fields[0])?;
    let id = fixed_17(&fields[1], "KV node ID")?;
    if !matches!(id[0], 2..=4) {
        return Err(RpcStatus::KvNoEnt);
    }
    let uid = active_uid(reader, principal)?;
    if id[0] == 2 {
        let object_id: [u8; 16] = id[1..].try_into().expect("KV node ID width was checked");
        let stored = reader
            .kv_file(&uid, &object_id)
            .map_err(|_| RpcStatus::TransactionRetry)?
            .ok_or(RpcStatus::KvNoEnt)?;
        let metadata = foks_proto::KvLargeFileMetadata::decode(&stored.exact_metadata)
            .map_err(|_| RpcStatus::TransactionRetry)?;
        return Ok(Response::Data(
            foks_proto::KvNode::File(metadata)
                .encoded()
                .map_err(|_| RpcStatus::TransactionRetry)?,
        ));
    }
    let stored = reader
        .kv_node(&uid, &id)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    let boxed = foks_proto::KvSmallFileBox::decode(&stored.exact)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let node = if stored.node_type == 3 {
        foks_proto::KvNode::SmallFile(boxed)
    } else {
        foks_proto::KvNode::Symlink(boxed)
    };
    Ok(Response::Data(
        node.encoded().map_err(|_| RpcStatus::TransactionRetry)?,
    ))
}

fn get_encrypted_chunk(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 3)?;
    require_user_auth(&fields[0])?;
    let id = fixed_16(&fields[1], "KV file ID")?;
    let Value::Unsigned(offset) = &fields[2] else {
        return Err(bad_arguments("KV chunk offset is not unsigned"));
    };
    let uid = active_uid(reader, principal)?;
    let chunk = reader
        .kv_file_chunk(&uid, &id, *offset)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    let response = foks_proto::KvEncryptedChunk {
        ciphertext: chunk.ciphertext,
        offset: chunk.offset,
        final_chunk: chunk.final_chunk,
    };
    Ok(Response::Data(
        response.encode().map_err(|_| RpcStatus::TransactionRetry)?,
    ))
}

fn get_directory(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    require_user_auth(&fields[0])?;
    let id = fixed_16(&fields[1], "KV directory ID")?;
    let uid = active_uid(reader, principal)?;
    let stored = reader
        .kv_directory(&uid, &id)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    let directory =
        foks_proto::KvDirectory::decode(&stored.exact).map_err(|_| RpcStatus::TransactionRetry)?;
    let pair = foks_proto::KvDirectoryPair::from_active(directory)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    Ok(Response::Data(pair.encoded().to_vec()))
}

fn list(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 3)?;
    require_user_auth(&fields[0])?;
    let id = fixed_16(&fields[1], "KV directory ID")?;
    let Value::Array(pagination) = &fields[2] else {
        return Err(bad_arguments("KV pagination is not a struct"));
    };
    let [cursor, Value::Unsigned(number), Value::Bool(load_small)] = pagination.as_slice() else {
        return Err(bad_arguments("KV pagination has the wrong shape"));
    };
    if *number == 0 || *number > 1000 {
        return Err(bad_arguments("KV pagination count is out of range"));
    }
    let after = list_cursor(cursor)?;
    let uid = active_uid(reader, principal)?;
    reader
        .kv_directory(&uid, &id)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)?;
    let limit =
        usize::try_from(*number).map_err(|_| bad_arguments("KV pagination count overflows"))? + 1;
    let mut stored = reader
        .kv_list(&uid, &id, after.as_ref(), limit)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let final_page = stored.len() < limit;
    if !final_page {
        stored.pop();
    }
    let mut entries = Vec::with_capacity(stored.len());
    let mut extended = Vec::new();
    for (position, stored) in stored.into_iter().enumerate() {
        let entry =
            foks_proto::KvDirent::decode(&stored.exact).map_err(|_| RpcStatus::TransactionRetry)?;
        if *load_small && stored.node_id[0] == 3 {
            let node = reader
                .kv_node(&uid, &stored.node_id)
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or(RpcStatus::KvNoEnt)?;
            extended.push(foks_proto::KvExtendedDirent {
                position: u64::try_from(position).map_err(|_| RpcStatus::TransactionRetry)?,
                small_file: foks_proto::KvSmallFileBox::decode(&node.exact)
                    .map_err(|_| RpcStatus::TransactionRetry)?,
            });
        }
        entries.push(entry);
    }
    let response = foks_proto::KvListResponse::new(entries, final_page, extended)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    Ok(Response::Data(response.encoded().to_vec()))
}

fn cache_check(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 1)?;
    let Value::Array(inner) = &fields[0] else {
        return Err(bad_arguments("KV cache check is not a struct"));
    };
    let [auth, versions] = inner.as_slice() else {
        return Err(bad_arguments("KV cache check has the wrong shape"));
    };
    require_user_auth(auth)?;
    let supplied = foks_proto::KvPathVersionVector::from_value(versions).map_err(bad_arguments)?;
    let uid = active_uid(reader, principal)?;
    let current = current_versions(reader, &uid)?;
    if supplied.equivalent(&current) {
        Ok(Response::Void)
    } else {
        Err(RpcStatus::StaleCache(current))
    }
}

fn lock_acquire(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    clock: &dyn foks_server_db::Clock,
) -> Result<Response, RpcStatus> {
    const MAXIMUM_LOCK_MILLIS: u64 = 24 * 60 * 60 * 1000;
    let fields = fields(argument, 3)?;
    require_user_auth(&fields[0])?;
    let LockFields {
        parent,
        dirent,
        lock,
    } = lock_fields(&fields[1])?;
    let Value::Unsigned(timeout_millis) = &fields[2] else {
        return Err(bad_arguments("KV lock timeout is not unsigned"));
    };
    if *timeout_millis == 0 || *timeout_millis > MAXIMUM_LOCK_MILLIS {
        return Err(bad_arguments("KV lock timeout is out of range"));
    }
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let expires_at = timeout_millis
        .checked_mul(1000)
        .and_then(|duration| now.checked_add(duration))
        .ok_or_else(|| bad_arguments("KV lock expiry overflows"))?;
    let uid = active_uid(reader, principal)?;
    let device = *principal.device_id();
    writer
        .call(move |database| {
            if !database.is_active_device(&uid, &device)? {
                return Err(crate::Error::Database(foks_server_db::Error::KvConflict));
            }
            database.acquire_kv_lock(&uid, &parent, &dirent, &lock, now, expires_at)?;
            Ok(())
        })
        .map_err(map_lock_error)?;
    Ok(Response::Void)
}

fn lock_release(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
) -> Result<Response, RpcStatus> {
    let fields = fields(argument, 2)?;
    require_user_auth(&fields[0])?;
    let LockFields {
        parent,
        dirent,
        lock,
    } = lock_fields(&fields[1])?;
    let uid = active_uid(reader, principal)?;
    let device = *principal.device_id();
    writer
        .call(move |database| {
            if !database.is_active_device(&uid, &device)? {
                return Err(crate::Error::Database(foks_server_db::Error::KvConflict));
            }
            database.release_kv_lock(&uid, &parent, &dirent, &lock)?;
            Ok(())
        })
        .map_err(map_lock_error)?;
    Ok(Response::Void)
}

fn current_versions(
    reader: &foks_server_db::ReadDatabase,
    uid: &[u8],
) -> Result<foks_proto::KvPathVersionVector, RpcStatus> {
    reader
        .kv_version_vector(uid)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::KvNoEnt)
}

fn active_uid(
    reader: &foks_server_db::ReadDatabase,
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    reader
        .identity_by_active_device(principal.device_id())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .map(|identity| identity.uid)
        .ok_or_else(permission_denied)
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

fn require_user_auth(value: &Value) -> Result<(), RpcStatus> {
    if matches!(
        value,
        Value::Array(auth)
            if matches!(auth.as_slice(), [Value::Unsigned(0), Value::Variant(None)])
    ) {
        Ok(())
    } else {
        Err(bad_arguments(
            "only personal KV authentication is supported",
        ))
    }
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

fn list_cursor(value: &Value) -> Result<Option<[u8; 32]>, RpcStatus> {
    let Value::Array(fields) = value else {
        return Err(bad_arguments("KV list cursor is not a struct"));
    };
    match fields.as_slice() {
        [Value::Unsigned(0), Value::Variant(None)] => Ok(None),
        [Value::Unsigned(1), Value::Variant(Some((tag, value)))] if tag == b"1" => {
            let Value::Binary(bytes) = value.as_ref() else {
                return Err(bad_arguments("KV list MAC cursor is not binary"));
            };
            bytes
                .as_slice()
                .try_into()
                .map(Some)
                .map_err(|_| bad_arguments("KV list MAC cursor has the wrong width"))
        }
        _ => Err(bad_arguments("unsupported KV list cursor")),
    }
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

fn map_write_error(
    error: crate::Error,
    reader: &foks_server_db::ReadDatabase,
    uid: &[u8],
) -> RpcStatus {
    if matches!(&error, crate::Error::Database(error) if error.is_quota()) {
        return RpcStatus::QuotaExceeded;
    }
    match error {
        crate::Error::Database(foks_server_db::Error::KvConflict) => current_versions(reader, uid)
            .map(RpcStatus::StaleCache)
            .unwrap_or(RpcStatus::TransactionRetry),
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        _ => RpcStatus::TransactionRetry,
    }
}

fn map_upload_error(error: crate::Error) -> RpcStatus {
    if matches!(&error, crate::Error::Database(error) if error.is_quota()) {
        return RpcStatus::QuotaExceeded;
    }
    match error {
        crate::Error::Database(foks_server_db::Error::KvConflict) => RpcStatus::KvNoEnt,
        crate::Error::Database(
            foks_server_db::Error::Invalid(_) | foks_server_db::Error::IntegerRange,
        ) => bad_arguments("invalid KV upload transition"),
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        _ => RpcStatus::TransactionRetry,
    }
}

fn map_lock_error(error: crate::Error) -> RpcStatus {
    if matches!(&error, crate::Error::Database(error) if error.is_quota()) {
        return RpcStatus::QuotaExceeded;
    }
    match error {
        crate::Error::Database(foks_server_db::Error::KvLocked) => RpcStatus::Locked,
        crate::Error::Database(foks_server_db::Error::KvConflict) => RpcStatus::KvNoEnt,
        crate::Error::Database(
            foks_server_db::Error::Invalid(_) | foks_server_db::Error::IntegerRange,
        ) => bad_arguments("invalid KV lock transition"),
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        _ => RpcStatus::TransactionRetry,
    }
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

fn permission_denied() -> RpcStatus {
    RpcStatus::PermissionDenied("active device does not authorize this KV namespace".to_owned())
}
