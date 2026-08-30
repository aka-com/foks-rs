use std::sync::Arc;

use foks_proto::{
    EntityId, InviteCode, LookupUserResult, PassphraseLoginResult, RegistrationChallenge,
    RegistrationChallengePayload, SecretBox,
};
use foks_rpc::RpcStatus;

use crate::keys::{HostKeyProvider, KeyPurpose};
use crate::{Entropy, WriterHandle};

const CHALLENGE_LIFETIME_MICROSECONDS: u64 = 10 * 60 * 1_000_000;
pub(crate) const USER_VIEWERSHIP: foks_proto::ViewershipMode = foks_proto::ViewershipMode::Open;

pub(crate) fn client_version_info(argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
    foks_rpc::arguments::decode_client_version_info(argument).map_err(bad_arguments)?;
    foks_proto::ServerClientVersionInfo {
        minimum: None,
        newest: None,
        message: Vec::new(),
    }
    .encoded()
    .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn server_config(
    argument: &[u8],
    database: &foks_server_db::ReadDatabase,
) -> Result<Vec<u8>, RpcStatus> {
    foks_rpc::arguments::decode_void(argument).map_err(bad_arguments)?;
    let invite_code_regime = database
        .invite_policy()
        .map_err(|_| RpcStatus::TransactionRetry)?
        .regime
        .protocol_value();
    foks_proto::RegServerConfig {
        sso: None,
        host_type: 4,
        user_viewership: USER_VIEWERSHIP,
        team_viewership: foks_proto::ViewershipMode::Open,
        invite_code_regime,
    }
    .encoded()
    .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn check_name_exists(
    argument: &[u8],
    database: &foks_server_db::ReadDatabase,
) -> Result<(), RpcStatus> {
    let name = foks_rpc::arguments::decode_check_name_exists(argument).map_err(bad_arguments)?;
    require_normalized_name(&name)?;
    if database
        .uid_by_normalized_name(&name)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .is_some()
    {
        Ok(())
    } else {
        Err(RpcStatus::UserNotFound)
    }
}

pub(crate) fn probe_key_exists(
    argument: &[u8],
    database: &foks_server_db::ReadDatabase,
) -> Result<(), RpcStatus> {
    let request = foks_rpc::arguments::decode_probe_key_exists(argument).map_err(bad_arguments)?;
    if database
        .device_self_token_matches(
            request.uid.as_bytes(),
            request.device_id.as_bytes(),
            request.self_token.expose(),
        )
        .map_err(|_| RpcStatus::TransactionRetry)?
    {
        Ok(())
    } else {
        Err(RpcStatus::KeyNotFound("probed key".to_owned()))
    }
}

pub(crate) fn stretch_version(argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
    foks_rpc::arguments::decode_void(argument).map_err(bad_arguments)?;
    foks_snowpack::encode(&foks_proto::StretchVersion::V1.to_value())
        .map_err(|_| RpcStatus::TransactionRetry)
}
pub(crate) fn invite_fingerprint(code: &InviteCode) -> Result<[u8; 32], RpcStatus> {
    crate::invites::invite_fingerprint(code).map_err(bad_arguments)
}

pub(crate) fn invite_kind(code: &InviteCode) -> Result<foks_server_db::InviteKind, RpcStatus> {
    crate::invites::invite_kind(code).map_err(|_| RpcStatus::BadInvite)
}

pub(crate) fn check_invite_code(
    argument: &[u8],
    database: &foks_server_db::ReadDatabase,
    clock: &dyn foks_server_db::Clock,
) -> Result<(), RpcStatus> {
    let code = foks_rpc::arguments::decode_check_invite_code(argument)
        .map_err(|_| RpcStatus::BadInvite)?;
    match code {
        InviteCode::Empty => {
            let policy = database
                .invite_policy()
                .map_err(|_| RpcStatus::TransactionRetry)?;
            if policy.regime == foks_server_db::InviteRegime::Optional {
                Ok(())
            } else {
                Err(RpcStatus::BadInvite)
            }
        }
        InviteCode::Standard(_) | InviteCode::MultiUse(_) => {
            let hash = invite_fingerprint(&code)?;
            let kind = invite_kind(&code)?;
            let now = clock
                .now_micros()
                .map_err(|_| RpcStatus::TransactionRetry)?;
            if database
                .invite_available(&hash, kind, now)
                .map_err(|_| RpcStatus::TransactionRetry)?
            {
                Ok(())
            } else {
                Err(RpcStatus::BadInvite)
            }
        }
        _ => Err(RpcStatus::BadInvite),
    }
}

pub(crate) fn issue_uid_lookup_challenge(
    argument: &[u8],
    host: &EntityId,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    let entity = foks_rpc::arguments::decode_uid_lookup_challenge(argument)
        .map_err(bad_arguments)?
        .require_type(foks_proto::ENTITY_BACKUP_KEY)
        .map_err(bad_arguments)?;
    issue_recovery_challenge(entity, host, writer, keys, clock, entropy)
}

pub(crate) fn issue_subkey_challenge(
    argument: &[u8],
    host: &EntityId,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    let entity = foks_rpc::arguments::decode_subkey_box_challenge(argument)
        .map_err(bad_arguments)?
        .require_type(foks_proto::ENTITY_YUBI)
        .map_err(bad_arguments)?;
    issue_recovery_challenge(entity, host, writer, keys, clock, entropy)
}

fn issue_recovery_challenge(
    entity: EntityId,
    host: &EntityId,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    let key = keys
        .load_or_create(KeyPurpose::Recovery)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let expires_at = now
        .checked_add(CHALLENGE_LIFETIME_MICROSECONDS)
        .ok_or(RpcStatus::TransactionRetry)?;
    let mut random = [0; 16];
    entropy
        .fill(&mut random)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let payload = RegistrationChallengePayload {
        hmac_key_id: key.generation().as_bytes(),
        entity,
        host: host.clone(),
        random,
        time: now / 1_000,
    };
    let exact_payload = payload.encoded().map_err(|_| RpcStatus::TransactionRetry)?;
    let challenge = RegistrationChallenge {
        mac: foks_crypto::capability_mac(
            key.expose(),
            foks_proto::REG_CHALLENGE_PAYLOAD_TYPE_ID,
            &exact_payload,
        ),
        payload,
    };
    let exact = challenge
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let hash = challenge_hash(&exact);
    let entity = challenge.payload.entity.into_bytes();
    let host = host.as_bytes().to_vec();
    let key_generation = key.generation().as_bytes();
    writer
        .call_with_current_time(Arc::clone(clock), move |database, current_time| {
            database.issue_recovery_challenge(
                &hash,
                &entity,
                &host,
                &key_generation,
                expires_at,
                current_time,
            )?;
            Ok(())
        })
        .map_err(map_write_error)?;
    Ok(exact)
}

pub(crate) fn load_subkey_box(
    argument: &[u8],
    host: &EntityId,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
) -> Result<Vec<u8>, RpcStatus> {
    let request = foks_rpc::arguments::decode_load_subkey_box(argument).map_err(bad_arguments)?;
    if request.challenge.payload.entity != request.parent || request.challenge.payload.host != *host
    {
        return Err(permission_denied());
    }
    let key = keys
        .load_or_create(KeyPurpose::Recovery)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    if request.challenge.payload.hmac_key_id != key.generation().as_bytes() {
        return Err(RpcStatus::Expired);
    }
    let payload = request
        .challenge
        .payload
        .encoded()
        .map_err(|_| permission_denied())?;
    foks_crypto::verify_capability_mac(
        key.expose(),
        foks_proto::REG_CHALLENGE_PAYLOAD_TYPE_ID,
        &payload,
        &request.challenge.mac,
    )
    .map_err(|_| permission_denied())?;
    foks_crypto::verify_typed(
        &request.parent,
        &request.signature,
        foks_proto::REG_CHALLENGE_PAYLOAD_TYPE_ID,
        &payload,
    )
    .map_err(|_| permission_denied())?;
    let exact = request
        .challenge
        .encoded()
        .map_err(|_| permission_denied())?;
    let hash = challenge_hash(&exact);
    let parent = request.parent.into_bytes();
    let host = host.as_bytes().to_vec();
    let generation = key.generation().as_bytes();
    match writer
        .call_with_current_time(Arc::clone(clock), move |database, now| {
            Ok(database.consume_subkey_challenge(&hash, &parent, &host, &generation, now)?)
        })
        .map_err(map_write_error)?
    {
        foks_server_db::SubkeyChallengeResult::Found(exact) => Ok(exact),
        foks_server_db::SubkeyChallengeResult::Expired => Err(RpcStatus::Expired),
        foks_server_db::SubkeyChallengeResult::NotFound => Err(RpcStatus::NotFound(
            "YubiKey subkey box not found".to_owned(),
        )),
    }
}

pub(crate) fn issue_login_challenge(
    argument: &[u8],
    host: &EntityId,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    let uid = foks_rpc::arguments::decode_login_challenge(argument)
        .map_err(bad_arguments)?
        .require_type(foks_proto::ENTITY_USER)
        .map_err(bad_arguments)?;
    let key = keys
        .load_or_create(KeyPurpose::Recovery)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let expires_at = now
        .checked_add(CHALLENGE_LIFETIME_MICROSECONDS)
        .ok_or(RpcStatus::TransactionRetry)?;
    let mut random = [0; 16];
    entropy
        .fill(&mut random)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let payload = RegistrationChallengePayload {
        hmac_key_id: key.generation().as_bytes(),
        entity: uid,
        host: host.clone(),
        random,
        time: now / 1_000,
    };
    let exact_payload = payload.encoded().map_err(|_| RpcStatus::TransactionRetry)?;
    let challenge = RegistrationChallenge {
        mac: foks_crypto::capability_mac(
            key.expose(),
            foks_proto::REG_CHALLENGE_PAYLOAD_TYPE_ID,
            &exact_payload,
        ),
        payload,
    };
    let exact = challenge
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let hash = challenge_hash(&exact);
    let uid = challenge.payload.entity.as_bytes().to_vec();
    let host = host.as_bytes().to_vec();
    let generation = key.generation().as_bytes();
    writer
        .call_with_current_time(Arc::clone(clock), move |database, current_time| {
            database.issue_passphrase_challenge(
                &hash,
                &uid,
                &host,
                &generation,
                expires_at,
                current_time,
            )?;
            Ok(())
        })
        .map_err(map_passphrase_write_error)?;
    Ok(exact)
}

pub(crate) fn passphrase_login(
    argument: &[u8],
    host: &EntityId,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
) -> Result<Vec<u8>, RpcStatus> {
    let request = foks_rpc::arguments::decode_passphrase_login(argument).map_err(bad_arguments)?;
    if request.challenge.payload.entity != request.uid || request.challenge.payload.host != *host {
        return Err(RpcStatus::BadPassphrase);
    }
    let key = keys
        .load_or_create(KeyPurpose::Recovery)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    if request.challenge.payload.hmac_key_id != key.generation().as_bytes() {
        return Err(RpcStatus::BadPassphrase);
    }
    let payload = request
        .challenge
        .payload
        .encoded()
        .map_err(|_| RpcStatus::BadPassphrase)?;
    foks_crypto::verify_capability_mac(
        key.expose(),
        foks_proto::REG_CHALLENGE_PAYLOAD_TYPE_ID,
        &payload,
        &request.challenge.mac,
    )
    .map_err(|_| RpcStatus::BadPassphrase)?;
    let uid = request.uid.as_bytes().to_vec();
    let exact = request
        .challenge
        .encoded()
        .map_err(|_| RpcStatus::BadPassphrase)?;
    let hash = challenge_hash(&exact);
    let host_bytes = host.as_bytes().to_vec();
    let generation = key.generation().as_bytes();
    let state = writer
        .call_with_current_time(Arc::clone(clock), {
            let uid = uid.clone();
            move |database, now| Ok(database.passphrase_for_login(&uid, now)?)
        })
        .map_err(map_passphrase_write_error)?;
    let Some(state) = state else {
        consume_failed_login(writer, clock, hash, uid, host_bytes, generation)?;
        return Err(RpcStatus::BadPassphrase);
    };
    let verify_key =
        EntityId::from_bytes(state.verify_key.clone()).map_err(|_| RpcStatus::TransactionRetry)?;
    if foks_crypto::verify_typed(
        &verify_key,
        &request.signature,
        foks_proto::REG_CHALLENGE_PAYLOAD_TYPE_ID,
        &payload,
    )
    .is_err()
    {
        consume_failed_login(writer, clock, hash, uid, host_bytes, generation)?;
        return Err(RpcStatus::BadPassphrase);
    }
    let authenticated = writer
        .call_with_current_time(Arc::clone(clock), move |database, now| {
            Ok(database.consume_passphrase_challenge(
                &hash,
                &uid,
                &host_bytes,
                &generation,
                verify_key.as_bytes(),
                now,
            )?)
        })
        .map_err(map_passphrase_write_error)?
        .ok_or(RpcStatus::BadPassphrase)?;
    PassphraseLoginResult {
        generation: authenticated.generation,
        skmwk_box: SecretBox::decode(&authenticated.exact_skmwk_box)
            .map_err(|_| RpcStatus::TransactionRetry)?,
        passphrase_box: foks_proto::PpePassphraseBox::decode(&authenticated.exact_passphrase_box)
            .map_err(|_| RpcStatus::TransactionRetry)?,
    }
    .encoded()
    .map_err(|_| RpcStatus::TransactionRetry)
}

fn consume_failed_login(
    writer: &WriterHandle,
    clock: &Arc<dyn foks_server_db::Clock>,
    hash: [u8; 32],
    uid: Vec<u8>,
    host: Vec<u8>,
    generation: [u8; 16],
) -> Result<(), RpcStatus> {
    writer
        .call_with_current_time(Arc::clone(clock), move |database, now| {
            database.consume_failed_passphrase_challenge(&hash, &uid, &host, &generation, now)?;
            Ok(())
        })
        .map_err(map_passphrase_write_error)
}

pub(crate) fn lookup_uid_by_device(
    argument: &[u8],
    host: &EntityId,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
) -> Result<Vec<u8>, RpcStatus> {
    let request =
        foks_rpc::arguments::decode_lookup_uid_by_device(argument).map_err(bad_arguments)?;
    if request.entity.entity_type() != foks_proto::ENTITY_BACKUP_KEY
        || request.challenge.payload.entity != request.entity
        || request.challenge.payload.host != *host
    {
        return Err(lookup_failed());
    }
    let key = keys
        .load_or_create(KeyPurpose::Recovery)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    if request.challenge.payload.hmac_key_id != key.generation().as_bytes() {
        return Err(lookup_failed());
    }
    let payload = request
        .challenge
        .payload
        .encoded()
        .map_err(|_| lookup_failed())?;
    foks_crypto::verify_capability_mac(
        key.expose(),
        foks_proto::REG_CHALLENGE_PAYLOAD_TYPE_ID,
        &payload,
        &request.challenge.mac,
    )
    .map_err(|_| lookup_failed())?;
    foks_crypto::verify_typed(
        &request.entity,
        &request.signature,
        foks_proto::REG_CHALLENGE_PAYLOAD_TYPE_ID,
        &payload,
    )
    .map_err(|_| lookup_failed())?;
    let exact = request.challenge.encoded().map_err(|_| lookup_failed())?;
    let hash = challenge_hash(&exact);
    let entity = request.entity.into_bytes();
    let host_bytes = host.as_bytes().to_vec();
    let generation = key.generation().as_bytes();
    let snapshot = writer
        .call_with_current_time(Arc::clone(clock), move |database, now| {
            Ok(database.consume_recovery_challenge(
                &hash,
                &entity,
                &host_bytes,
                &generation,
                now,
            )?)
        })
        .map_err(map_write_error)?
        .ok_or_else(lookup_failed)?;
    let role = stored_role(snapshot.role_type, snapshot.visibility).ok_or_else(lookup_failed)?;
    LookupUserResult {
        uid: EntityId::from_bytes(snapshot.uid).map_err(|_| RpcStatus::TransactionRetry)?,
        host: host.clone(),
        username: snapshot.normalized_name,
        username_utf8: snapshot.username_utf8,
        role,
        yubi_pq_hint: None,
    }
    .encoded()
    .map_err(|_| RpcStatus::TransactionRetry)
}

fn stored_role(kind: u64, visibility: i64) -> Option<foks_proto::Role> {
    match kind {
        1 => i16::try_from(visibility).ok().map(foks_proto::Role::member),
        2 if visibility == 0 => Some(foks_proto::Role::ADMIN),
        3 if visibility == 0 => Some(foks_proto::Role::OWNER),
        _ => None,
    }
}

fn challenge_hash(exact: &[u8]) -> [u8; 32] {
    const TYPE_ID: u64 = 0x72b6_c1f3_464f_4b53;
    foks_crypto::prefixed_hash(TYPE_ID, exact)
}

fn lookup_failed() -> RpcStatus {
    RpcStatus::NotFound("credential lookup failed".to_owned())
}

fn permission_denied() -> RpcStatus {
    RpcStatus::PermissionDenied("YubiKey challenge verification failed".to_owned())
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

fn require_normalized_name(name: &[u8]) -> Result<(), RpcStatus> {
    if foks_verify::normalize_username(name).as_deref() == Some(name) {
        Ok(())
    } else {
        Err(bad_arguments("username is not normalized"))
    }
}

fn map_write_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::RateLimited,
        _ => RpcStatus::TransactionRetry,
    }
}

fn map_passphrase_write_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::WriterQueue
        | crate::Error::Database(foks_server_db::Error::QuotaExceeded)
        | crate::Error::Database(foks_server_db::Error::PassphraseRateLimited) => {
            RpcStatus::RateLimited
        }
        _ => RpcStatus::TransactionRetry,
    }
}
