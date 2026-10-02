//! User registration, device, passphrase, and identity request encoding.

use crate::call::{encode_call, encode_call_with_validated_argument, encode_select_vhost};
use crate::generated::{
    BEACON_LOOKUP_METHOD_POSITION, BEACON_PROTOCOL_ID, KEX_PROTOCOL_ID,
    KEX_RECEIVE_METHOD_POSITION, KEX_SEND_METHOD_POSITION, REG_CHECK_INVITE_CODE_METHOD_POSITION,
    REG_CHECK_NAME_EXISTS_METHOD_POSITION, REG_GET_CLIENT_CERT_CHAIN_METHOD_POSITION,
    REG_GET_LOGIN_CHALLENGE_METHOD_POSITION, REG_GET_SERVER_CONFIG_METHOD_POSITION,
    REG_GET_SUBKEY_BOX_CHALLENGE_METHOD_POSITION, REG_GET_UID_LOOKUP_CHALLENGE_METHOD_POSITION,
    REG_LOAD_SUBKEY_BOX_METHOD_POSITION, REG_LOAD_USER_CHAIN_METHOD_POSITION,
    REG_LOGIN_METHOD_POSITION, REG_LOOKUP_UID_BY_DEVICE_METHOD_POSITION,
    REG_PROBE_KEY_EXISTS_METHOD_POSITION, REG_PROTOCOL_ID, REG_RESERVE_USERNAME_METHOD_POSITION,
    REG_SELECT_VHOST_METHOD_POSITION, REG_SIGNUP_METHOD_POSITION,
    REG_STRETCH_VERSION_METHOD_POSITION, TEAM_ADMIN_PROTOCOL_ID, TEAM_EDIT_METHOD_POSITION,
    USER_CHANGE_PASSPHRASE_METHOD_POSITION, USER_CLEAR_DEVICE_NAG_METHOD_POSITION,
    USER_GET_ALL_YUBI_MANAGEMENT_KEYS_METHOD_POSITION, USER_GET_DEVICE_NAG_METHOD_POSITION,
    USER_GET_PPE_PARCEL_METHOD_POSITION, USER_GET_PUK_FOR_ROLE_METHOD_POSITION,
    USER_GET_SALT_METHOD_POSITION, USER_GET_YUBI_MANAGEMENT_KEY_METHOD_POSITION,
    USER_GRANT_REMOTE_VIEW_PERMISSION_METHOD_POSITION, USER_LOAD_GENERIC_CHAIN_METHOD_POSITION,
    USER_LOAD_USER_CHAIN_METHOD_POSITION, USER_NEXT_PASSPHRASE_GENERATION_METHOD_POSITION,
    USER_PING_METHOD_POSITION, USER_POST_GENERIC_LINK_METHOD_POSITION, USER_PROTOCOL_ID,
    USER_PROVISION_DEVICE_METHOD_POSITION, USER_PUT_YUBI_MANAGEMENT_KEY_METHOD_POSITION,
    USER_RESOLVE_USERNAME_METHOD_POSITION, USER_REVOKE_DEVICE_METHOD_POSITION,
    USER_SET_PASSPHRASE_METHOD_POSITION, USER_STRETCH_VERSION_METHOD_POSITION,
};
use crate::{arguments, Result};
use foks_proto::{
    AddTeamMemberArgument, EntityId, InviteCode, KexReceiveArgument, KexSendArgument,
    PassphraseUpdateArgument, PermissionToken, ProvisionDeviceArgument, RegistrationChallenge,
    RemoteViewPermissionPayload, RevokeDeviceArgument, Role, Signature, SoftwareSignupArgument,
    YubiEncryptedManagementKey, YubiSignupArgument,
};
use foks_snowpack::{decode, encode, Value};

/// Encodes the unauthenticated registration call that obtains an X.509 client
/// certificate chain for an already enrolled device key.
pub fn encode_get_client_cert_chain_request(uid: &[u8], device_id: &[u8]) -> Result<Vec<u8>> {
    encode_get_client_cert_chain_request_at(uid, device_id, 0)
}

pub fn encode_kex_send_request(argument: &KexSendArgument) -> Result<Vec<u8>> {
    encode_call(
        KEX_PROTOCOL_ID,
        KEX_SEND_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_kex_receive_request(argument: &KexReceiveArgument) -> Result<Vec<u8>> {
    encode_call(
        KEX_PROTOCOL_ID,
        KEX_RECEIVE_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_reserve_username_request_at(name: &[u8], sequence: u64) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Text(name.to_vec())]))?;
    encode_call(
        REG_PROTOCOL_ID,
        REG_RESERVE_USERNAME_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_signup_request_at(
    argument: &SoftwareSignupArgument<'_>,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_SIGNUP_METHOD_POSITION,
        &argument.encoded()?,
        sequence,
    )
}

pub fn encode_yubi_signup_request_at(
    argument: &YubiSignupArgument<'_>,
    sequence: u64,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_SIGNUP_METHOD_POSITION,
        &argument.encoded()?,
        sequence,
    )
}

pub fn encode_get_subkey_box_challenge_request(parent: &EntityId) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_SUBKEY_BOX_CHALLENGE_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Binary(
            parent.as_bytes().to_vec(),
        )]))?,
        0,
    )
}

pub fn encode_load_subkey_box_request(
    parent: &EntityId,
    challenge: &RegistrationChallenge,
    signature: &Signature,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_LOAD_SUBKEY_BOX_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(parent.as_bytes().to_vec()),
            decode(&challenge.encoded()?)?,
            signature.to_value(),
        ]))?,
        0,
    )
}

pub fn encode_check_invite_code_request(code: &InviteCode) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_CHECK_INVITE_CODE_METHOD_POSITION,
        &encode(&Value::Array(vec![code.to_value()]))?,
        0,
    )
}

pub fn encode_get_login_challenge_request(uid: &EntityId) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_LOGIN_CHALLENGE_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Binary(uid.as_bytes().to_vec())]))?,
        0,
    )
}

pub fn encode_passphrase_login_request(
    uid: &EntityId,
    challenge: &RegistrationChallenge,
    signature: &Signature,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_LOGIN_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(uid.as_bytes().to_vec()),
            decode(&challenge.encoded()?)?,
            signature.to_value(),
        ]))?,
        0,
    )
}

pub fn encode_registration_stretch_version_request() -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_STRETCH_VERSION_METHOD_POSITION,
        &encode(&Value::Null)?,
        0,
    )
}

pub fn encode_registration_server_config_request() -> Result<Vec<u8>> {
    encode_call_with_validated_argument(
        REG_PROTOCOL_ID,
        REG_GET_SERVER_CONFIG_METHOD_POSITION,
        &[0x90],
        0,
    )
}

pub fn encode_check_name_exists_request(name: &[u8]) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_CHECK_NAME_EXISTS_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Text(name.to_vec())]))?,
        0,
    )
}

pub fn encode_probe_key_exists_request(
    uid: &EntityId,
    device_id: &EntityId,
    self_token: &PermissionToken,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_PROBE_KEY_EXISTS_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(uid.as_bytes().to_vec()),
            Value::Binary(device_id.as_bytes().to_vec()),
            self_token.to_value(),
        ]))?,
        0,
    )
}

pub fn encode_get_client_cert_chain_request_at(
    uid: &[u8],
    device_id: &[u8],
    sequence: u64,
) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        Value::Binary(uid.to_vec()),
        Value::Binary(device_id.to_vec()),
    ]))?;
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_CLIENT_CERT_CHAIN_METHOD_POSITION,
        &argument,
        sequence,
    )
}

pub fn encode_registration_select_vhost_request(host: &EntityId) -> Result<Vec<u8>> {
    encode_select_vhost(REG_PROTOCOL_ID, REG_SELECT_VHOST_METHOD_POSITION, host)
}

/// Encodes an authenticated request for a user's chain, starting at `start`.
pub fn encode_load_user_chain_request(uid: &[u8], start: u64) -> Result<Vec<u8>> {
    encode_load_user_chain_request_from(uid, start, None)
}

pub fn encode_load_user_chain_request_from(
    uid: &[u8],
    start: u64,
    current_name: Option<(&[u8], u64)>,
) -> Result<Vec<u8>> {
    let as_local_user = Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]);
    encode_load_user_chain_with_authorization(uid, start, current_name, as_local_user)
}

/// Encodes the authenticated `OpenVHost` form used to inspect a prospective
/// local member before admitting it to a team. The server still applies the
/// virtual host's public-user-viewership policy.
pub fn encode_load_user_chain_open_host_request(
    uid: &[u8],
    start: u64,
    current_name: Option<(&[u8], u64)>,
) -> Result<Vec<u8>> {
    let open_host = Value::Array(vec![Value::Unsigned(4), Value::Variant(None)]);
    encode_load_user_chain_with_authorization(uid, start, current_name, open_host)
}

pub(super) fn encode_load_user_chain_with_authorization(
    uid: &[u8],
    start: u64,
    current_name: Option<(&[u8], u64)>,
    authorization: Value,
) -> Result<Vec<u8>> {
    let name = current_name.map_or(Value::Null, |(name, next_sequence)| {
        Value::Array(vec![
            Value::Text(name.to_vec()),
            Value::Unsigned(next_sequence),
        ])
    });
    let argument = encode(&Value::Array(vec![Value::Array(vec![
        Value::Binary(uid.to_vec()),
        Value::Unsigned(start),
        name,
        authorization,
    ])]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_LOAD_USER_CHAIN_METHOD_POSITION,
        &argument,
        0,
    )
}

/// Encodes the public registration-service form used to load a remote user
/// after the remote user explicitly grants this caller a bearer permission.
pub fn encode_load_remote_user_chain_request(
    uid: &EntityId,
    start: u64,
    current_name: Option<(&[u8], u64)>,
    token: &PermissionToken,
) -> Result<Vec<u8>> {
    uid.clone().require_type(foks_proto::ENTITY_USER)?;
    if start == 0 {
        return Err(foks_proto::Error::IntegerRange("user-chain start").into());
    }
    let argument = arguments::load_user_chain_argument_value(
        uid,
        start,
        current_name,
        arguments::remote_token_authorization(token),
    );
    encode_call(
        REG_PROTOCOL_ID,
        REG_LOAD_USER_CHAIN_METHOD_POSITION,
        &encode(&argument)?,
        0,
    )
}

pub fn encode_beacon_lookup_request(host: &EntityId) -> Result<Vec<u8>> {
    host.clone().require_type(foks_proto::ENTITY_HOST)?;
    encode_call(
        BEACON_PROTOCOL_ID,
        BEACON_LOOKUP_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Binary(host.as_bytes().to_vec())]))?,
        0,
    )
}

pub fn encode_grant_remote_view_permission_for_user_request(
    payload: &RemoteViewPermissionPayload,
) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_GRANT_REMOTE_VIEW_PERMISSION_METHOD_POSITION,
        &encode(&Value::Array(vec![payload.to_value()]))?,
        0,
    )
}

/// Encodes an authenticated request for the owner-role PUK parcel addressed
/// to `device_id`.
pub fn encode_get_owner_puk_request(device_id: &[u8]) -> Result<Vec<u8>> {
    encode_get_puk_for_role_request(Role::OWNER, device_id)
}

/// Encodes an authenticated request for the current PUK and its historical
/// seed chain at the enrolled device's exact role.
pub fn encode_get_puk_for_role_request(role: Role, device_id: &[u8]) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![
        role.to_value(),
        Value::Binary(device_id.to_vec()),
    ]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_GET_PUK_FOR_ROLE_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_set_passphrase_request(argument: &PassphraseUpdateArgument) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_SET_PASSPHRASE_METHOD_POSITION,
        &argument.encoded_set()?,
        0,
    )
}

pub fn encode_change_passphrase_request(argument: &PassphraseUpdateArgument) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_CHANGE_PASSPHRASE_METHOD_POSITION,
        &argument.encoded_change()?,
        0,
    )
}

pub(super) fn encode_user_void_request(position: u64) -> Result<Vec<u8>> {
    encode_call(USER_PROTOCOL_ID, position, &encode(&Value::Null)?, 0)
}

pub fn encode_get_passphrase_salt_request() -> Result<Vec<u8>> {
    encode_user_void_request(USER_GET_SALT_METHOD_POSITION)
}

pub fn encode_next_passphrase_generation_request() -> Result<Vec<u8>> {
    encode_user_void_request(USER_NEXT_PASSPHRASE_GENERATION_METHOD_POSITION)
}

pub fn encode_user_stretch_version_request() -> Result<Vec<u8>> {
    encode_user_void_request(USER_STRETCH_VERSION_METHOD_POSITION)
}

pub fn encode_put_yubi_management_key_request(
    value: &YubiEncryptedManagementKey,
) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_PUT_YUBI_MANAGEMENT_KEY_METHOD_POSITION,
        &encode(&Value::Array(vec![value.to_value()?]))?,
        0,
    )
}

pub fn encode_get_yubi_management_key_request(parent: &EntityId) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_GET_YUBI_MANAGEMENT_KEY_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Binary(
            parent.as_bytes().to_vec(),
        )]))?,
        0,
    )
}

pub fn encode_get_all_yubi_management_keys_request() -> Result<Vec<u8>> {
    encode_user_void_request(USER_GET_ALL_YUBI_MANAGEMENT_KEYS_METHOD_POSITION)
}

pub fn encode_get_ppe_parcel_request() -> Result<Vec<u8>> {
    encode_user_void_request(USER_GET_PPE_PARCEL_METHOD_POSITION)
}

pub fn encode_load_generic_chain_request(
    entity: &EntityId,
    chain_type: u64,
    start: u64,
) -> Result<Vec<u8>> {
    if start == 0
        || !matches!(
            chain_type,
            foks_proto::CHAIN_TYPE_USER_SETTINGS | foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP
        )
    {
        return Err(foks_proto::Error::IntegerRange("generic-chain request").into());
    }
    let argument = encode(&Value::Array(vec![
        Value::Binary(entity.as_bytes().to_vec()),
        Value::Unsigned(chain_type),
        Value::Unsigned(start),
    ]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_LOAD_GENERIC_CHAIN_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_post_generic_link_request(
    argument: &foks_proto::PostGenericLinkArgument,
) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_POST_GENERIC_LINK_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_provision_device_request(argument: &ProvisionDeviceArgument<'_>) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_PROVISION_DEVICE_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

pub fn encode_get_uid_lookup_challenge_request(entity: &EntityId) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_GET_UID_LOOKUP_CHALLENGE_METHOD_POSITION,
        &encode(&Value::Array(vec![Value::Binary(
            entity.as_bytes().to_vec(),
        )]))?,
        0,
    )
}

pub fn encode_lookup_uid_by_device_request(
    entity: &EntityId,
    challenge: &RegistrationChallenge,
    signature: &Signature,
) -> Result<Vec<u8>> {
    encode_call(
        REG_PROTOCOL_ID,
        REG_LOOKUP_UID_BY_DEVICE_METHOD_POSITION,
        &encode(&Value::Array(vec![
            Value::Binary(entity.as_bytes().to_vec()),
            decode(&challenge.encoded()?)?,
            signature.to_value(),
        ]))?,
        0,
    )
}

pub fn encode_revoke_device_request(argument: &RevokeDeviceArgument<'_>) -> Result<Vec<u8>> {
    encode_call(
        USER_PROTOCOL_ID,
        USER_REVOKE_DEVICE_METHOD_POSITION,
        &argument.encoded()?,
        0,
    )
}

/// Local invitation admission reuses its explicit scoped view grant.
pub fn encode_local_invitation_admission_request(
    argument: &AddTeamMemberArgument<'_>,
) -> Result<Vec<u8>> {
    encode_call(
        TEAM_ADMIN_PROTOCOL_ID,
        TEAM_EDIT_METHOD_POSITION,
        &argument.encoded_local_invitation()?,
        0,
    )
}

pub fn encode_user_ping_request() -> Result<Vec<u8>> {
    encode_call_with_validated_argument(USER_PROTOCOL_ID, USER_PING_METHOD_POSITION, &[0x90], 0)
}

pub fn encode_resolve_username_request(name: &[u8], open_host: bool) -> Result<Vec<u8>> {
    let authorization = Value::Array(vec![
        Value::Unsigned(if open_host { 4 } else { 0 }),
        Value::Variant(None),
    ]);
    let argument = encode(&Value::Array(vec![Value::Array(vec![
        Value::Text(name.to_vec()),
        authorization,
    ])]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_RESOLVE_USERNAME_METHOD_POSITION,
        &argument,
        0,
    )
}

pub fn encode_get_device_nag_request() -> Result<Vec<u8>> {
    encode_call_with_validated_argument(
        USER_PROTOCOL_ID,
        USER_GET_DEVICE_NAG_METHOD_POSITION,
        &[0x90],
        0,
    )
}

pub fn encode_clear_device_nag_request(cleared: bool) -> Result<Vec<u8>> {
    let argument = encode(&Value::Array(vec![Value::Bool(cleared)]))?;
    encode_call(
        USER_PROTOCOL_ID,
        USER_CLEAR_DEVICE_NAG_METHOD_POSITION,
        &argument,
        0,
    )
}
