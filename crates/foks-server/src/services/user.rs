use foks_proto::EntityId;
use foks_rpc::RpcStatus;
use foks_snowpack::{decode, Value};

use crate::auth::Principal;

pub(crate) fn load_user_chain(
    database: &foks_server_db::ReadDatabase,
    host: &EntityId,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    let Value::Array(outer) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments("user-chain argument is not a struct"));
    };
    let [Value::Array(fields)] = outer.as_slice() else {
        return Err(bad_arguments("user-chain argument has the wrong shape"));
    };
    let [Value::Binary(uid), Value::Unsigned(start), name, local] = fields.as_slice() else {
        return Err(bad_arguments("user-chain cursor has the wrong shape"));
    };
    let uid = EntityId::from_bytes(uid.clone())
        .and_then(|uid| uid.require_type(foks_proto::ENTITY_USER))
        .map_err(bad_arguments)?;
    if !matches!(
        local,
        Value::Array(local)
            if matches!(local.as_slice(), [Value::Unsigned(0), Value::Variant(None)])
    ) {
        return Err(bad_arguments("only local-user chain views are supported"));
    }
    let identity = database
        .identity_for_active_device(uid.as_bytes(), principal.device_id())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(permission_denied)?;
    let incremental = match (*start, name) {
        (1, Value::Null) => false,
        (2, Value::Array(cursor))
            if matches!(
                cursor.as_slice(),
                [Value::Text(current), Value::Unsigned(2)]
                    if current == &identity.normalized_name
            ) =>
        {
            true
        }
        _ => return Err(bad_arguments("unsupported user-chain cursor")),
    };
    let root = database
        .roots_at(&[identity.chain_root_epoch])
        .map_err(|_| RpcStatus::TransactionRetry)?
        .and_then(|mut roots| roots.pop())
        .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
    validate_root(&root)?;
    let chain_one =
        foks_merkle_store::chain_key(0, &uid, 1, None).map_err(|_| RpcStatus::TransactionRetry)?;
    let chain_two = foks_merkle_store::chain_key(0, &uid, 2, Some(&identity.next_tree_location))
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let name_one = foks_merkle_store::username_key(&identity.normalized_name, host, 1)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let name_two = foks_merkle_store::username_key(&identity.normalized_name, host, 2)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let reader = database.node_reader();
    let prove = |key| {
        foks_merkle_store::proof(&reader, root.root_node, key)
            .map_err(|_| RpcStatus::TransactionRetry)
    };
    let (links, locations, usernames, paths, device_names, hepks, name_path_count) = if incremental
    {
        (
            Vec::new(),
            vec![identity.next_tree_location],
            Vec::new(),
            vec![prove(name_two)?, prove(chain_two)?],
            Vec::new(),
            Vec::new(),
            1,
        )
    } else {
        (
            vec![identity.exact_link],
            vec![identity.next_tree_location],
            vec![foks_proto::NameCommitmentAndKey {
                name: identity.normalized_name,
                sequence: identity.username_sequence,
                commitment_key: identity.username_commitment_key,
            }],
            vec![
                prove(name_one)?,
                prove(name_two)?,
                prove(chain_one)?,
                prove(chain_two)?,
            ],
            vec![
                foks_proto::DeviceLabelNameAndCommitmentKey::decode(&identity.exact_device_name)
                    .map_err(|_| RpcStatus::TransactionRetry)?,
            ],
            vec![identity.exact_shared_hepk, identity.exact_device_hepk],
            2,
        )
    };
    foks_proto::UserChainResponse {
        exact_links: &links,
        locations: &locations,
        usernames: &usernames,
        exact_root: &root.exact_root,
        paths: &paths,
        device_names: &device_names,
        username_utf8: &identity.username_utf8,
        num_username_links: name_path_count,
        exact_hepks: &hepks,
    }
    .encoded()
    .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn puk_for_role(
    database: &foks_server_db::ReadDatabase,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments("PUK argument is not a struct"));
    };
    let [role, Value::Binary(device)] = fields.as_slice() else {
        return Err(bad_arguments("PUK argument has the wrong shape"));
    };
    if !matches!(
        role,
        Value::Array(role)
            if matches!(role.as_slice(), [Value::Unsigned(3), Value::Variant(None)])
    ) {
        return Err(bad_arguments("only the owner PUK role is supported"));
    }
    if device.as_slice() != principal.device_id() {
        return Err(permission_denied());
    }
    let identity = database
        .identity_by_active_device(principal.device_id())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(permission_denied)?;
    let set = foks_proto::SharedKeyBoxSet::decode(&identity.exact_parcel)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let sender =
        EntityId::from_bytes(identity.device_id).map_err(|_| RpcStatus::TransactionRetry)?;
    foks_proto::PukParcel::from_box_set(&set, 0, sender, Vec::new())
        .and_then(|parcel| parcel.encoded())
        .map_err(|_| RpcStatus::TransactionRetry)
}

fn validate_root(root: &foks_server_db::RootSnapshot) -> Result<(), RpcStatus> {
    let decoded = foks_proto::MerkleRoot::decode(&root.exact_root)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let hash = foks_crypto::prefixed_hash(foks_proto::MERKLE_ROOT_TYPE_ID, &root.exact_root);
    if decoded.epoch != root.epoch || decoded.root_node != root.root_node || hash != root.root_hash
    {
        return Err(RpcStatus::TransactionRetry);
    }
    Ok(())
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    let mut message = error.to_string();
    if message.len() > 160 {
        let mut end = 160;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    RpcStatus::BadArguments(message)
}

fn permission_denied() -> RpcStatus {
    RpcStatus::PermissionDenied("active device does not authorize this request".to_owned())
}
