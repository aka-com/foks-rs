use std::sync::Arc;

use foks_proto::EntityId;
use foks_rpc::RpcStatus;
use foks_snowpack::{decode, Value};

use crate::auth::Principal;
use crate::{net::session::OwnedPassphraseMutation, WriterHandle};

pub(crate) fn set_passphrase(
    database: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    clock: &Arc<dyn foks_server_db::Clock>,
    argument: &[u8],
    principal: &Principal,
) -> Result<(), RpcStatus> {
    authorize_database(database, principal)?;
    principal.require_ordinary_device()?;
    let decoded = foks_rpc::arguments::decode_set_passphrase(argument).map_err(bad_arguments)?;
    let owned = OwnedPassphraseMutation::from_argument(&decoded)
        .map_err(|_| bad_arguments("invalid passphrase boxes"))?;
    let uid = principal.uid().to_vec();
    let credential = principal.device_id().to_vec();
    writer
        .call_with_current_time(Arc::clone(clock), move |database, now| {
            database.set_passphrase(&uid, &credential, owned.as_database(now))?;
            Ok(())
        })
        .map_err(map_passphrase_write_error)
}

pub(crate) fn change_passphrase(
    database: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    clock: &Arc<dyn foks_server_db::Clock>,
    argument: &[u8],
    principal: &Principal,
) -> Result<(), RpcStatus> {
    authorize_database(database, principal)?;
    principal.require_ordinary_device()?;
    let current = database
        .passphrase(principal.uid())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::PassphraseNotFound)?;
    let decoded = foks_rpc::arguments::decode_change_passphrase(argument, current.salt)
        .map_err(bad_arguments)?;
    let owned = OwnedPassphraseMutation::from_argument(&decoded)
        .map_err(|_| bad_arguments("invalid passphrase boxes"))?;
    let uid = principal.uid().to_vec();
    let credential = principal.device_id().to_vec();
    writer
        .call_with_current_time(Arc::clone(clock), move |database, now| {
            database.change_passphrase(&uid, &credential, owned.as_database(now))?;
            Ok(())
        })
        .map_err(map_passphrase_write_error)
}

pub(crate) fn passphrase_salt(
    database: &foks_server_db::ReadSnapshot<'_>,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    foks_rpc::arguments::decode_void(argument).map_err(bad_arguments)?;
    authorize(database, principal)?;
    let state = database
        .passphrase(principal.uid())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::PassphraseNotFound)?;
    foks_snowpack::encode(&Value::Binary(state.salt.to_vec()))
        .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn next_passphrase_generation(
    database: &foks_server_db::ReadSnapshot<'_>,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    foks_rpc::arguments::decode_void(argument).map_err(bad_arguments)?;
    authorize(database, principal)?;
    let next = database
        .passphrase(principal.uid())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .map_or(Ok(1), |state| {
            state
                .generation
                .checked_add(1)
                .ok_or(RpcStatus::TransactionRetry)
        })?;
    foks_snowpack::encode(&Value::Unsigned(next)).map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn stretch_version(
    database: &foks_server_db::ReadSnapshot<'_>,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    foks_rpc::arguments::decode_void(argument).map_err(bad_arguments)?;
    authorize(database, principal)?;
    foks_snowpack::encode(&foks_proto::StretchVersion::V1.to_value())
        .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn ppe_parcel(
    database: &foks_server_db::ReadSnapshot<'_>,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    foks_rpc::arguments::decode_void(argument).map_err(bad_arguments)?;
    authorize(database, principal)?;
    let state = database
        .passphrase(principal.uid())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::PassphraseNotFound)?;
    let stretch_version =
        foks_proto::StretchVersion::from_value(&Value::Unsigned(state.stretch_version))
            .map_err(|_| RpcStatus::TransactionRetry)?;
    let verify_key =
        EntityId::from_bytes(state.verify_key).map_err(|_| RpcStatus::TransactionRetry)?;
    foks_proto::PpeParcel {
        skmwk_box: foks_proto::SecretBox::decode(&state.exact_skmwk_box)
            .map_err(|_| RpcStatus::TransactionRetry)?,
        generation: state.generation,
        passphrase_box: foks_proto::PpePassphraseBox::decode(&state.exact_passphrase_box)
            .map_err(|_| RpcStatus::TransactionRetry)?,
        puk_box: state
            .exact_puk_box
            .as_deref()
            .map(foks_proto::PpePukBox::decode)
            .transpose()
            .map_err(|_| RpcStatus::TransactionRetry)?,
        salt: state.salt,
        stretch_version,
        verify_key,
    }
    .encoded()
    .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn put_yubi_management_key(
    writer: &WriterHandle,
    clock: &Arc<dyn foks_server_db::Clock>,
    argument: &[u8],
    principal: &Principal,
) -> Result<(), RpcStatus> {
    let value =
        foks_rpc::arguments::decode_put_yubi_management_key(argument).map_err(bad_arguments)?;
    let exact_box = value
        .secret_box
        .encoded()
        .map_err(|_| bad_arguments("invalid Yubi management-key box"))?;
    let (role_type, visibility) = role_parts(value.role);
    let snapshot = foks_server_db::YubiManagementKeySnapshot {
        parent_id: value.yubi_id.into_bytes(),
        exact_box,
        generation: value.generation,
        role_type,
        visibility,
    };
    let uid = principal.uid().to_vec();
    let credential = principal.device_id().to_vec();
    writer
        .call_with_current_time(Arc::clone(clock), move |database, now| {
            database.put_yubi_management_key(&uid, &credential, &snapshot, now)?;
            Ok(())
        })
        .map_err(map_yubi_write_error)
}

pub(crate) fn get_yubi_management_key(
    database: &foks_server_db::ReadSnapshot<'_>,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    let parent =
        foks_rpc::arguments::decode_get_yubi_management_key(argument).map_err(bad_arguments)?;
    let snapshot = database
        .yubi_management_key_for_credential(
            principal.uid(),
            principal.device_id(),
            parent.as_bytes(),
        )
        .map_err(map_yubi_database_error)?
        .ok_or_else(|| RpcStatus::NotFound("Yubi management key not found".to_owned()))?;
    yubi_management_value(snapshot)?
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn get_all_yubi_management_keys(
    database: &foks_server_db::ReadSnapshot<'_>,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    foks_rpc::arguments::decode_void(argument).map_err(bad_arguments)?;
    let values = database
        .all_yubi_management_keys_for_credential(principal.uid(), principal.device_id())
        .map_err(map_yubi_database_error)?
        .into_iter()
        .map(yubi_management_value)
        .collect::<Result<Vec<_>, _>>()?;
    foks_snowpack::encode(&Value::Array(
        values
            .iter()
            .map(foks_proto::YubiEncryptedManagementKey::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| RpcStatus::TransactionRetry)?,
    ))
    .map_err(|_| RpcStatus::TransactionRetry)
}

fn yubi_management_value(
    snapshot: foks_server_db::YubiManagementKeySnapshot,
) -> Result<foks_proto::YubiEncryptedManagementKey, RpcStatus> {
    Ok(foks_proto::YubiEncryptedManagementKey {
        yubi_id: EntityId::from_bytes(snapshot.parent_id)
            .map_err(|_| RpcStatus::TransactionRetry)?,
        secret_box: foks_proto::SecretBox::decode(&snapshot.exact_box)
            .map_err(|_| RpcStatus::TransactionRetry)?,
        generation: snapshot.generation,
        role: stored_role(snapshot.role_type, snapshot.visibility)
            .ok_or(RpcStatus::TransactionRetry)?,
    })
}

fn stored_role(kind: u64, visibility: i64) -> Option<foks_proto::Role> {
    match kind {
        1 => i16::try_from(visibility).ok().map(foks_proto::Role::member),
        2 if visibility == 0 => Some(foks_proto::Role::ADMIN),
        3 if visibility == 0 => Some(foks_proto::Role::OWNER),
        _ => None,
    }
}

fn authorize(
    database: &foks_server_db::ReadSnapshot<'_>,
    principal: &Principal,
) -> Result<(), RpcStatus> {
    database
        .identity_for_active_device(principal.uid(), principal.device_id())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(permission_denied)?;
    Ok(())
}

fn authorize_database(
    database: &foks_server_db::ReadDatabase,
    principal: &Principal,
) -> Result<(), RpcStatus> {
    database
        .identity_for_active_device(principal.uid(), principal.device_id())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(permission_denied)?;
    Ok(())
}

fn map_passphrase_write_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::AuthorizationChanged => permission_denied(),
        crate::Error::Database(foks_server_db::Error::AuthorizationChanged) => permission_denied(),
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::PassphraseNotFound) => {
            RpcStatus::PassphraseNotFound
        }
        crate::Error::Database(foks_server_db::Error::PassphraseGeneration) => {
            RpcStatus::BadArguments("passphrase generation is stale".to_owned())
        }
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::RateLimited,
        _ => RpcStatus::TransactionRetry,
    }
}

fn map_yubi_write_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::AuthorizationChanged
        | crate::Error::Database(foks_server_db::Error::AuthorizationChanged) => {
            permission_denied()
        }
        crate::Error::WriterQueue
        | crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::Invalid(message)) => bad_arguments(message),
        _ => RpcStatus::TransactionRetry,
    }
}

fn map_yubi_database_error(error: foks_server_db::Error) -> RpcStatus {
    match error {
        foks_server_db::Error::AuthorizationChanged => permission_denied(),
        _ => RpcStatus::TransactionRetry,
    }
}

pub(crate) fn host_config(
    database: &foks_server_db::ReadSnapshot<'_>,
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    database
        .identity_for_active_device(principal.uid(), principal.device_id())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(permission_denied)?;
    let invite_code_regime = database
        .invite_policy()
        .map_err(|_| RpcStatus::TransactionRetry)?
        .regime
        .protocol_value();
    foks_proto::HostConfig {
        meter_users: false,
        meter_vhosts: false,
        meter_per_vhost_disk: false,
        user_viewership: foks_proto::ViewershipMode::Open,
        team_viewership: foks_proto::ViewershipMode::Open,
        host_type: 4,
        invite_code_regime,
    }
    .encoded()
    .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn load_user_chain(
    database: &foks_server_db::ReadSnapshot<'_>,
    host: &EntityId,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    let request =
        foks_rpc::arguments::decode_load_user_chain_argument(argument).map_err(bad_arguments)?;
    if !matches!(
        request.authorization,
        foks_rpc::arguments::UserChainAuthorization::LocalUser
    ) {
        return Err(permission_denied());
    }
    database
        .identity_for_active_device(request.uid.as_bytes(), principal.device_id())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(permission_denied)?;
    render_user_chain(database, host, &request)
}

pub(crate) fn render_user_chain(
    database: &foks_server_db::ReadSnapshot<'_>,
    host: &EntityId,
    request: &foks_rpc::arguments::LoadUserChainArgument,
) -> Result<Vec<u8>, RpcStatus> {
    let uid = &request.uid;
    let start = request.start;
    let chain = database
        .user_chain(uid.as_bytes())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(|| RpcStatus::NotFound("user chain not found".to_owned()))?;
    let maximum_start = u64::try_from(chain.links.len())
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(RpcStatus::TransactionRetry)?;
    let full = start == 1 && request.name_cursor.is_none();
    if !full
        && request.name_cursor.as_ref()
            != Some(&(
                chain.normalized_name.clone(),
                chain.username_sequence.saturating_add(1),
            ))
    {
        return Err(bad_arguments("unsupported user-chain name cursor"));
    }
    if start == 0 || (!full && start < 2) || start > maximum_start {
        return Err(bad_arguments("user-chain start is out of range"));
    }
    let root = database
        .current_root()
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(|| RpcStatus::NotFound("Merkle root not found".to_owned()))?;
    validate_root(&root)?;
    let name_one = foks_merkle_store::username_key(&chain.normalized_name, host, 1)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let name_next = foks_merkle_store::username_key(
        &chain.normalized_name,
        host,
        chain.username_sequence.saturating_add(1),
    )
    .map_err(|_| RpcStatus::TransactionRetry)?;
    let reader = database.node_reader();
    let prove = |key| {
        foks_merkle_store::proof(&reader, root.root_node, key)
            .map_err(|_| RpcStatus::TransactionRetry)
    };
    let start_index =
        usize::try_from(start.saturating_sub(1)).map_err(|_| RpcStatus::TransactionRetry)?;
    let selected = &chain.links[start_index..];
    let links = selected
        .iter()
        .map(|link| link.exact_link.clone())
        .collect::<Vec<_>>();
    let mut locations = if full {
        Vec::new()
    } else {
        vec![chain.links[start_index - 1].next_tree_location]
    };
    locations.extend(selected.iter().map(|link| link.next_tree_location));
    let usernames = if full {
        vec![foks_proto::NameCommitmentAndKey {
            name: chain.normalized_name.clone(),
            sequence: chain.username_sequence,
            commitment_key: chain.username_commitment_key,
        }]
    } else {
        Vec::new()
    };
    let mut paths = if full {
        vec![prove(name_one)?, prove(name_next)?]
    } else {
        vec![prove(name_next)?]
    };
    for index in start_index..chain.links.len() {
        let sequence = u64::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(RpcStatus::TransactionRetry)?;
        let prior = index
            .checked_sub(1)
            .map(|prior| &chain.links[prior].next_tree_location);
        let key = foks_merkle_store::chain_key(0, uid, sequence, prior)
            .map_err(|_| RpcStatus::TransactionRetry)?;
        paths.push(prove(key)?);
    }
    let next_sequence = maximum_start;
    let next_key = foks_merkle_store::chain_key(
        0,
        uid,
        next_sequence,
        chain.links.last().map(|link| &link.next_tree_location),
    )
    .map_err(|_| RpcStatus::TransactionRetry)?;
    paths.push(prove(next_key)?);
    let expected_device_names = selected
        .iter()
        .map(|link| {
            foks_proto::UserLink::decode(&link.exact_link).map_err(|_| RpcStatus::TransactionRetry)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|link| {
            link.decode_group_change()
                .map_err(|_| RpcStatus::TransactionRetry)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flat_map(|change| change.metadata)
        .filter_map(|metadata| match metadata {
            foks_proto::ChangeMetadata::DeviceName(commitment) => Some(commitment),
            _ => None,
        })
        .collect::<Vec<_>>();
    let stored_device_names = chain
        .devices
        .iter()
        .map(|device| {
            let name = foks_proto::DeviceLabelNameAndCommitmentKey::decode(&device.exact_name)
                .map_err(|_| RpcStatus::TransactionRetry)?;
            Ok((device_name_commitment(&name)?, name))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, RpcStatus>>()?;
    let device_names = expected_device_names
        .iter()
        .map(|commitment| {
            stored_device_names
                .get(commitment)
                .cloned()
                .ok_or(RpcStatus::TransactionRetry)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let hepks = chain
        .exact_shared_hepks
        .iter()
        .map(|exact| foks_proto::Hepk::decode(exact).map_err(|_| RpcStatus::TransactionRetry))
        .collect::<Result<Vec<_>, _>>()?;
    foks_proto::UserChainResponse {
        exact_links: &links,
        locations: &locations,
        usernames: &usernames,
        exact_root: &root.exact_root,
        paths: &paths,
        device_names: &device_names,
        username_utf8: &chain.username_utf8,
        num_username_links: if full { 2 } else { 1 },
        exact_hepks: &hepks
            .iter()
            .map(|hepk| hepk.encoded().map_err(|_| RpcStatus::TransactionRetry))
            .collect::<Result<Vec<_>, _>>()?,
    }
    .encoded()
    .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn puk_for_role(
    database: &foks_server_db::ReadSnapshot<'_>,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments("PUK argument is not a struct"));
    };
    let [role, Value::Binary(device)] = fields.as_slice() else {
        return Err(bad_arguments("PUK argument has the wrong shape"));
    };
    let exact_role = foks_snowpack::encode(role).map_err(bad_arguments)?;
    let role = foks_proto::Role::decode(&exact_role).map_err(bad_arguments)?;
    if role == foks_proto::Role::NONE {
        return Err(bad_arguments("PUK role cannot be none"));
    }
    let parent = database
        .active_credential_owner(principal.uid(), principal.device_id())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(permission_denied)?;
    if device.as_slice() != parent {
        return Err(permission_denied());
    }
    let identity = database
        .identity_by_active_device(principal.device_id())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(permission_denied)?;
    let (role_type, visibility) = role_parts(role);
    let material = database
        .puk_material(&identity.uid, &parent, role_type, visibility)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(|| RpcStatus::NotFound("PUK parcel not found".to_owned()))?;
    let set = foks_proto::SharedKeyBoxSet::decode(&material.exact_box_set)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let index = set
        .boxes
        .iter()
        .position(|boxed| boxed.target.entity.as_bytes() == parent && boxed.role == role)
        .ok_or_else(|| RpcStatus::NotFound("PUK parcel target not found".to_owned()))?;
    let sender =
        EntityId::from_bytes(material.sender_id).map_err(|_| RpcStatus::TransactionRetry)?;
    let seed_chain = material
        .exact_seed_chain
        .iter()
        .map(|exact| {
            foks_proto::SeedChainBox::decode(exact).map_err(|_| RpcStatus::TransactionRetry)
        })
        .collect::<Result<Vec<_>, _>>()?;
    foks_proto::PukParcel::from_box_set(&set, index, sender, seed_chain)
        .and_then(|parcel| parcel.encoded())
        .map_err(|_| RpcStatus::TransactionRetry)
}

fn role_parts(role: foks_proto::Role) -> (u64, i64) {
    (
        role.protocol_value(),
        role.visibility().map(i64::from).unwrap_or_default(),
    )
}

fn device_name_commitment(
    name: &foks_proto::DeviceLabelNameAndCommitmentKey,
) -> Result<[u8; 32], RpcStatus> {
    let wire = foks_snowpack::encode(&Value::Array(vec![
        Value::Unsigned(name.label.device_type.protocol_value()),
        Value::Text(name.label.normalized_name.clone()),
        Value::Unsigned(name.label.serial),
    ]))
    .map_err(|_| RpcStatus::TransactionRetry)?;
    Ok(foks_crypto::commitment(
        foks_proto::DEVICE_LABEL_TYPE_ID,
        &wire,
        &name.commitment_key,
    ))
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
