//! Username reservation, signup and client certificate issuance.

use std::sync::Arc;

use foks_proto::{DecodedSignupArgument, EntityId, InviteCode, UsernameReservation};
use foks_rpc::RpcStatus;
use foks_snowpack::{decode, Value};

use super::merkle_query::validated_root;
use super::passphrase::OwnedPassphraseMutation;
use super::user::bad_arguments;
use super::user_mutation::require_cited_root;
use super::IdentityPorts;
use crate::identity::validate_signup;
use crate::merkle::decode_stored_root;

pub(crate) fn reserve_username(
    ports: &IdentityPorts<'_>,
    argument: &[u8],
) -> Result<Vec<u8>, RpcStatus> {
    const RESERVATION_LIFETIME_MICROSECONDS: u64 = 10 * 60 * 1_000_000;
    let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments(
            "username reservation argument is not a struct",
        ));
    };
    let [Value::Text(name)] = fields.as_slice() else {
        return Err(bad_arguments(
            "username reservation argument has the wrong shape",
        ));
    };
    let normalized = foks_verify::normalize_username(name)
        .filter(|normalized| normalized == name)
        .ok_or_else(|| bad_arguments("username is not normalized"))?;
    let mut token = [0; 17];
    ports
        .entropy
        .fill(&mut token)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let now = ports
        .clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    // The reservation is replayed over the Go `lib.Time` wire type, whose
    // millisecond precision must round-trip exactly into the database.
    let expires_at_millis = (now / 1_000)
        .checked_add(RESERVATION_LIFETIME_MICROSECONDS / 1_000)
        .ok_or(RpcStatus::TransactionRetry)?;
    let expires_at = expires_at_millis
        .checked_mul(1_000)
        .ok_or(RpcStatus::TransactionRetry)?;
    let writer = ports.writer.ok_or(RpcStatus::Unsupported)?;
    let result =
        writer.call_with_current_time(Arc::clone(ports.clock), move |database, current_time| {
            database.reserve_name(&normalized, &token, 1, current_time, expires_at)?;
            Ok(())
        });
    match result {
        Ok(()) => UsernameReservation {
            token,
            sequence: 1,
            expires_at: expires_at_millis,
        }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry),
        Err(error) => Err(map_reservation_error(error)),
    }
}

pub(crate) fn signup(ports: &IdentityPorts<'_>, argument: &[u8]) -> Result<(), RpcStatus> {
    const IDEMPOTENCY_RECORD_LIFETIME_MICROSECONDS: u64 = 24 * 60 * 60 * 1_000_000;
    const SIGNUP_REQUEST_HASH_TYPE_ID: u64 = 0x8f4b_8ab7_464f_4b53;

    let request = DecodedSignupArgument::decode(argument).map_err(bad_arguments)?;
    let (invite_hash, invite_kind) = match &request.invite_code {
        InviteCode::Empty => (None, None),
        InviteCode::Standard(_) | InviteCode::MultiUse(_) => (
            Some(crate::services::registration::invite_fingerprint(
                &request.invite_code,
            )?),
            Some(crate::services::registration::invite_kind(
                &request.invite_code,
            )?),
        ),
        _ => return Err(RpcStatus::BadInvite),
    };
    let idempotency_key = request.self_token;
    let request_hash = foks_crypto::prefixed_hash(SIGNUP_REQUEST_HASH_TYPE_ID, argument);
    let reader = ports.read_database()?;
    let idempotency_now = ports
        .clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    match reader.idempotency_record(&idempotency_key, &request_hash, idempotency_now) {
        Ok(Some(idempotency_record)) if idempotency_record.response.is_empty() => return Ok(()),
        Ok(Some(_)) => return Err(RpcStatus::TransactionRetry),
        Ok(None) => {}
        Err(foks_server_db::Error::OperationConflict) => {
            return Err(bad_arguments("signup retry binding failed"));
        }
        Err(_) => return Err(RpcStatus::TransactionRetry),
    }
    let signed_root = request.link.decode_eldest().map_err(bad_arguments)?.root;
    let cited = reader
        .root_at(signed_root.epoch)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::StaleRoot)?;
    let cited_root = validated_root(&cited)?;
    if cited.root_hash != signed_root.hash {
        return Err(RpcStatus::StaleRoot);
    }
    let host = EntityId::from_bytes(ports.host_id.to_vec()).map_err(bad_arguments)?;
    let validated = validate_signup(&request, &host, &cited_root, cited.root_hash)
        .map_err(|_| bad_arguments("software signup validation failed"))?;
    let sso_binding = match ports.sso {
        Some(service) => Some(
            service
                .prepare_signup(
                    &request.sso,
                    &validated.uid,
                    &validated.device_id,
                    &request.username_utf8,
                    &request.email,
                    &request.reservation,
                )
                .map_err(crate::sso::status)?,
        ),
        None if request.sso == foks_proto::RegSsoArgs::None => None,
        None => return Err(RpcStatus::OAuth2("OIDC is not configured".into())),
    };
    let passphrase = request
        .passphrase
        .as_ref()
        .map(OwnedPassphraseMutation::from_argument)
        .transpose()
        .map_err(|_| bad_arguments("invalid signup passphrase material"))?;
    let writer = ports.writer.ok_or(RpcStatus::Unsupported)?;
    let keys = ports.key_provider.ok_or(RpcStatus::Unsupported)?;
    let keys = Arc::clone(keys);
    let clock = Arc::clone(ports.clock);
    let hostchain_tail = ports.hostchain_tail.clone();
    let reservation = request.reservation;
    // lib.Time is milliseconds on the Go wire, while the database stores
    // reservation deadlines in microseconds alongside its clock values.
    let reservation_expires_at = reservation
        .expires_at
        .checked_mul(1_000)
        .ok_or_else(|| bad_arguments("reservation expiry overflows server clock units"))?;
    let result = writer.call(move |database| {
        let now = clock.now_micros()?;
        let idempotency_expires_at = now
            .checked_add(IDEMPOTENCY_RECORD_LIFETIME_MICROSECONDS)
            .ok_or(crate::Error::Signup("idempotency_record expiry overflow"))?;
        let authoritative = database
            .current_root()?
            .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
        let cited = require_cited_root(database, &signed_root)?;
        if cited.epoch > authoritative.epoch {
            return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
        }
        let authoritative_root = decode_stored_root(&authoritative)?;
        if now / 1_000 < authoritative_root.time {
            return Err(crate::Error::Signup("system clock moved backwards"));
        }
        let crate::merkle::PreparedPublication {
            merkle_commit,
            root_epoch,
            root_hash,
            exact_root,
            exact_signed_root,
            back_pointers,
        } = crate::merkle::prepare_publication(
            database,
            &authoritative,
            &validated.leaves,
            hostchain_tail,
            now,
            keys.as_ref(),
        )?;
        let mutation = foks_server_db::IdentityMutation {
            normalized_name: &validated.normalized_name,
            reservation_token: &reservation.token,
            reservation_sequence: reservation.sequence,
            reservation_expires_at,
            username_utf8: &validated.username_utf8,
            username_commitment_key: &validated.username_commitment_key,
            uid: validated.uid.as_bytes(),
            device_id: validated.device_id.as_bytes(),
            device_hepk_fingerprint: &validated.device_hepk_fingerprint,
            self_token: &request.self_token,
            exact_device_hepk: &validated.exact_device_hepk,
            exact_device_name: &validated.exact_device_name,
            subkey_id: validated.subkey_id.as_ref().map(EntityId::as_bytes),
            exact_subkey_box: validated.exact_subkey_box.as_deref(),
            yubi_pq_hint: validated
                .yubi_pq_hint
                .as_ref()
                .map(|(slot, id)| (*slot, id)),
            link_hash: &validated.link_hash,
            exact_link: &validated.exact_link,
            tree_location: &validated.next_tree_location,
            subchain_tree_location_seed: &validated.subchain_tree_location_seed,
            shared_role_type: foks_proto::Role::OWNER.protocol_value(),
            shared_visibility: 0,
            shared_generation: 1,
            shared_verify_key: validated.puk_verify_key.as_bytes(),
            exact_shared_hepk: &validated.exact_puk_hepk,
            exact_parcel: &validated.exact_parcel,
            expected_root_hash: Some(authoritative.root_hash),
            merkle_commit: &merkle_commit,
            merkle_leaves: &validated.leaves,
            root_epoch,
            root_hash: &root_hash,
            exact_root: &exact_root,
            exact_signed_root: &exact_signed_root,
            back_pointers: &back_pointers,
            idempotency_key: &idempotency_key,
            request_hash: &request_hash,
            response: &[],
            invite: match (&invite_hash, invite_kind) {
                (Some(hash), Some(kind)) => foks_server_db::InviteConsumption::Code {
                    code_hash: hash,
                    kind,
                },
                (None, None) => foks_server_db::InviteConsumption::Empty,
                _ => return Err(crate::Error::Signup("invalid invite binding")),
            },
            passphrase: passphrase
                .as_ref()
                .map(|passphrase| passphrase.as_database(now)),
            now,
            idempotency_expires_at,
        };
        database.commit_identity_with_sso(&mutation, sso_binding.as_ref())?;
        Ok(())
    });
    result.map_err(map_signup_error)
}

pub(crate) fn client_certificate_chain(
    ports: &IdentityPorts<'_>,
    argument: &[u8],
) -> Result<Vec<u8>, RpcStatus> {
    const CERTIFICATE_LIFETIME_MICROSECONDS: u64 = 7 * 24 * 60 * 60 * 1_000_000;
    const RENEWAL_WINDOW_MICROSECONDS: u64 = 24 * 60 * 60 * 1_000_000;
    const CLOCK_SKEW_MICROSECONDS: u64 = 5 * 60 * 1_000_000;

    let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
        return Err(bad_arguments("certificate argument is not a struct"));
    };
    let [Value::Binary(uid), Value::Binary(device_id)] = fields.as_slice() else {
        return Err(bad_arguments("certificate argument has the wrong shape"));
    };
    let uid = EntityId::from_bytes(uid.clone())
        .and_then(|uid| uid.require_type(foks_proto::ENTITY_USER))
        .map_err(bad_arguments)?;
    let device = EntityId::from_bytes(device_id.clone()).map_err(bad_arguments)?;
    if !matches!(
        device.entity_type(),
        foks_proto::ENTITY_DEVICE
            | foks_proto::ENTITY_BACKUP_KEY
            | foks_proto::ENTITY_SUBKEY
            | foks_proto::ENTITY_BOT_TOKEN_KEY
    ) {
        return Err(bad_arguments("unsupported certificate credential kind"));
    }
    if let Some(sso) = ports.sso {
        sso.ensure_access(uid.as_bytes(), device.as_bytes())
            .map_err(crate::sso::access_status)?;
    }
    let writer = ports.writer.ok_or(RpcStatus::Unsupported)?;
    let keys = Arc::clone(ports.key_provider.ok_or(RpcStatus::Unsupported)?);
    let clock = Arc::clone(ports.clock);
    let entropy = Arc::clone(ports.entropy);
    let canonical_name = ports.canonical_name.to_owned();
    let result = writer.call(move |database| {
        let Some(owner) = database.active_credential_owner(uid.as_bytes(), device.as_bytes())?
        else {
            return Ok(None);
        };
        let now = clock.now_micros()?;
        database.sso_require_access(uid.as_bytes(), now / 1000)?;
        if let Some(certificate) =
            database.certificate_for_device(uid.as_bytes(), device.as_bytes())?
        {
            let renewal_at = now
                .checked_add(RENEWAL_WINDOW_MICROSECONDS)
                .ok_or(crate::Error::Signup("certificate renewal overflow"))?;
            if certificate.not_before <= now && certificate.not_after > renewal_at {
                return Ok(Some(certificate.exact_certificate));
            }
        }
        let not_before = now.saturating_sub(CLOCK_SKEW_MICROSECONDS);
        let not_after = now
            .checked_add(CERTIFICATE_LIFETIME_MICROSECONDS)
            .ok_or(crate::Error::Signup("certificate expiry overflow"))?;
        let mut serial = [0; 20];
        entropy.fill(&mut serial)?;
        serial[0] &= 0x7f;
        if serial.iter().all(|byte| *byte == 0) {
            serial[19] = 1;
        }
        let public_key: [u8; 32] = device.as_bytes()[1..]
            .try_into()
            .map_err(|_| crate::Error::Signup("device public key width"))?;
        let certificate = crate::pki::issue_bounded_device_certificate(
            keys.as_ref(),
            &canonical_name,
            public_key,
            &serial,
            not_before,
            not_after,
        )?;
        database.record_certificate(
            &serial,
            uid.as_bytes(),
            &owner,
            device.as_bytes(),
            not_before,
            not_after,
            &certificate,
        )?;
        Ok(Some(certificate))
    });
    match result {
        Ok(Some(certificate)) => {
            foks_snowpack::encode(&Value::Array(vec![Value::Binary(certificate)]))
                .map_err(|_| RpcStatus::TransactionRetry)
        }
        Ok(None) => Err(RpcStatus::NotFound("active device not found".to_owned())),
        Err(error) => Err(map_certificate_error(error)),
    }
}

pub(super) fn map_reservation_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::Database(foks_server_db::Error::NameInUse) => RpcStatus::NameInUse,
        other => crate::status::unmapped(other, "username_reservation"),
    }
}

pub(super) fn map_signup_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::Database(foks_server_db::Error::Duplicate("SSO provider identity")) => {
            RpcStatus::OAuth2("provider identity is already linked to another account".into())
        }
        crate::Error::Database(foks_server_db::Error::NameInUse) => RpcStatus::NameInUse,
        crate::Error::Database(foks_server_db::Error::StaleRoot) => RpcStatus::StaleRoot,
        crate::Error::Database(foks_server_db::Error::BadInvite) => RpcStatus::BadInvite,
        crate::Error::Database(
            foks_server_db::Error::Reservation | foks_server_db::Error::OperationConflict,
        ) => bad_arguments("signup reservation or retry binding failed"),
        other => crate::status::unmapped(other, "signup"),
    }
}

pub(super) fn map_certificate_error(error: crate::Error) -> RpcStatus {
    crate::status::unmapped(error, "client_certificate")
}
