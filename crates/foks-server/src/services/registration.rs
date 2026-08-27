use foks_proto::{EntityId, LookupUserResult, RegistrationChallenge, RegistrationChallengePayload};
use foks_rpc::RpcStatus;

use crate::keys::{HostKeyProvider, KeyPurpose};
use crate::{Entropy, WriterHandle};

const CHALLENGE_LIFETIME_MICROSECONDS: u64 = 10 * 60 * 1_000_000;

pub(crate) fn issue_uid_lookup_challenge(
    argument: &[u8],
    host: &EntityId,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &dyn foks_server_db::Clock,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    let entity = foks_rpc::arguments::decode_uid_lookup_challenge(argument)
        .map_err(bad_arguments)?
        .require_type(foks_proto::ENTITY_BACKUP_KEY)
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
        .call(move |database| {
            database.issue_recovery_challenge(
                &hash,
                &entity,
                &host,
                &key_generation,
                expires_at,
                now,
            )?;
            Ok(())
        })
        .map_err(map_write_error)?;
    Ok(exact)
}

pub(crate) fn lookup_uid_by_device(
    argument: &[u8],
    host: &EntityId,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &dyn foks_server_db::Clock,
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
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let entity = request.entity.into_bytes();
    let host_bytes = host.as_bytes().to_vec();
    let generation = key.generation().as_bytes();
    let snapshot = writer
        .call(move |database| {
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

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

fn map_write_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::RateLimited,
        _ => RpcStatus::TransactionRetry,
    }
}
