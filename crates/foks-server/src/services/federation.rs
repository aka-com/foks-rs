use std::sync::Arc;

use chacha20poly1305::aead::{Aead, KeyInit as _, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use foks_proto::{
    EntityId, PermissionToken, RoleType, ENTITY_AD_HOC_TEAM, ENTITY_HOST, ENTITY_NAMED_TEAM,
    ENTITY_USER,
};
use foks_rpc::RpcStatus;
use zeroize::Zeroizing;

use crate::auth::Principal;
use crate::keys::{HostKeyProvider, SecretKey};
use crate::{Entropy, WriterHandle};

const PERMISSION_TOKEN_TAG: u8 = 54;
const USER_PERMISSION_TOKEN_AAD: &[u8] = b"foks-federation-user-view-token-v1";
const TEAM_PERMISSION_TOKEN_AAD: &[u8] = b"foks-federation-team-view-token-v1";
// Go v0.1.9 treats remote-view permissions as valid until explicit
// revocation. SQLite stores this field as a signed integer, so its maximum is
// the protocol-compatible stand-in for a non-expiring bearer.
const PERMISSION_EXPIRY_MICROSECONDS: u64 = i64::MAX as u64;

#[allow(clippy::too_many_arguments)]
pub(crate) fn grant_remote_user_view(
    argument: &[u8],
    principal: &Principal,
    local_host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let payload = foks_rpc::arguments::decode_grant_remote_view_permission_for_user(argument)
        .map_err(bad_arguments)?;
    payload
        .viewee
        .clone()
        .require_type(ENTITY_USER)
        .map_err(bad_arguments)?;
    if payload.viewee.as_bytes() != principal.uid()
        || payload.viewer.host == *local_host
        || payload.viewer.host.entity_type() != ENTITY_HOST
    {
        return Err(permission_denied());
    }

    let target = payload.viewee.as_bytes().to_vec();
    let viewer = payload.viewer.party.as_bytes().to_vec();
    let viewer_host = payload.viewer.host.as_bytes().to_vec();
    let now = clock.now_micros().map_err(internal)?;
    let key = active_capability_key(reader, keys)?;
    let current = reader
        .renewable_remote_user_view_permission(&target, &viewer, &viewer_host)
        .map_err(internal)?;
    let grant = prepare_permission_grant(
        USER_PERMISSION_TOKEN_AAD,
        &target,
        &viewer,
        &viewer_host,
        current.as_ref().map(ExistingPermission::user),
        &key,
        keys,
        now,
        entropy,
    )?;
    let credential = principal.device_id().to_vec();
    let outcome = writer
        .call_with_current_time(Arc::clone(clock), move |database, current_time| {
            Ok(database.issue_remote_user_view_permission(
                &target,
                &credential,
                &viewer,
                &viewer_host,
                &grant,
                current_time,
            )?)
        })
        .map_err(map_write_error)?;
    let permission = match outcome {
        foks_server_db::RemoteUserViewPermissionOutcome::Inserted(permission)
        | foks_server_db::RemoteUserViewPermissionOutcome::Existing(permission)
        | foks_server_db::RemoteUserViewPermissionOutcome::Renewed(permission) => permission,
    };
    let decrypt_key = crate::keys::load_capability_generation(keys, permission.key_generation)
        .map_err(internal)?;
    let token = decrypt_user_token(&decrypt_key, &permission)?;
    token.encoded().map_err(internal)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn grant_remote_team_view(
    argument: &[u8],
    principal: &Principal,
    local_host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let request = foks_rpc::arguments::decode_grant_remote_view_permission_for_team(argument)
        .map_err(bad_arguments)?;
    if !matches!(
        request.payload.viewee.entity_type(),
        ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM
    ) || request.payload.viewer.host == *local_host
    {
        return Err(permission_denied());
    }
    if !matches!(
        request.authorization.role.kind(),
        RoleType::Admin | RoleType::Owner
    ) {
        return Err(permission_denied());
    }
    let now = clock.now_micros().map_err(internal)?;
    if !super::protocol_time_is_nowish(request.payload.time, now) {
        return Err(permission_denied());
    }
    let team = reader
        .team(request.payload.viewee.as_bytes())
        .map_err(internal)?
        .ok_or(RpcStatus::TeamNotFound)?;
    if team.host_id != local_host.as_bytes() {
        return Err(permission_denied());
    }
    let (role_type, visibility) = crate::auth::team::role_parts(request.authorization.role);
    let matching = team
        .shared_keys
        .iter()
        .filter(|candidate| {
            candidate.role_type == role_type
                && candidate.visibility == visibility
                && candidate.generation == request.authorization.generation
        })
        .collect::<Vec<_>>();
    let [shared_key] = matching.as_slice() else {
        return Err(permission_denied());
    };
    let latest = team
        .shared_keys
        .iter()
        .filter(|candidate| candidate.role_type == role_type && candidate.visibility == visibility)
        .map(|candidate| candidate.generation)
        .max();
    if latest != Some(request.authorization.generation) {
        return Err(permission_denied());
    }
    let verify_key = EntityId::from_bytes(shared_key.verify_key.clone()).map_err(internal)?;
    foks_crypto::verify_typed(
        &verify_key,
        &request.authorization.signature,
        foks_proto::REMOTE_VIEW_PERMISSION_PAYLOAD_TYPE_ID,
        &request.payload.encoded().map_err(bad_arguments)?,
    )
    .map_err(|_| permission_denied())?;

    let target = request.payload.viewee.as_bytes().to_vec();
    let viewer = request.payload.viewer.party.as_bytes().to_vec();
    let viewer_host = request.payload.viewer.host.as_bytes().to_vec();
    let key = active_capability_key(reader, keys)?;
    let current = reader
        .renewable_remote_team_view_permission(&target, &viewer, &viewer_host)
        .map_err(internal)?;
    let grant = prepare_permission_grant(
        TEAM_PERMISSION_TOKEN_AAD,
        &target,
        &viewer,
        &viewer_host,
        current.as_ref().map(ExistingPermission::team),
        &key,
        keys,
        now,
        entropy,
    )?;
    let authority_verify_key = shared_key.verify_key.clone();
    let credential = principal.device_id().to_vec();
    let outcome = writer
        .call_with_current_time(Arc::clone(clock), move |database, current_time| {
            Ok(database.issue_remote_team_view_permission(
                &target,
                &credential,
                &viewer,
                &viewer_host,
                &foks_server_db::TeamGrantAuthority {
                    role_type,
                    visibility,
                    generation: request.authorization.generation,
                    verify_key: &authority_verify_key,
                },
                &grant,
                current_time,
            )?)
        })
        .map_err(map_write_error)?;
    let permission = match outcome {
        foks_server_db::RemoteTeamViewPermissionOutcome::Inserted(permission)
        | foks_server_db::RemoteTeamViewPermissionOutcome::Existing(permission)
        | foks_server_db::RemoteTeamViewPermissionOutcome::Renewed(permission) => permission,
    };
    let decrypt_key = crate::keys::load_capability_generation(keys, permission.key_generation)
        .map_err(internal)?;
    decrypt_team_token(&decrypt_key, &permission)?
        .encoded()
        .map_err(internal)
}

pub(crate) fn load_remote_user_chain(
    database: &foks_server_db::ReadSnapshot<'_>,
    local_host: &EntityId,
    argument: &[u8],
    now: u64,
) -> Result<Vec<u8>, RpcStatus> {
    let request =
        foks_rpc::arguments::decode_load_user_chain_argument(argument).map_err(bad_arguments)?;
    match &request.authorization {
        foks_rpc::arguments::UserChainAuthorization::RemoteToken(token) => {
            let hash = token_hash(token.expose());
            if !database
                .remote_user_view_token_is_current(&hash, request.uid.as_bytes(), now)
                .map_err(internal)?
            {
                return Err(permission_denied());
            }
        }
        foks_rpc::arguments::UserChainAuthorization::SelfToken(token) => {
            if !database
                .self_token_matches(request.uid.as_bytes(), token.expose())
                .map_err(internal)?
            {
                return Err(permission_denied());
            }
        }
        _ => return Err(permission_denied()),
    }
    super::user::render_user_chain(database, local_host, &request)
}

struct TokenBinding<'a> {
    domain: &'a [u8],
    target: &'a [u8],
    viewer: &'a [u8],
    viewer_host: &'a [u8],
    token_hash: &'a [u8; 32],
    key_generation: &'a [u8; 16],
}

struct ExistingPermission<'a> {
    token_hash: &'a [u8; 32],
    token_nonce: &'a [u8; 24],
    token_ciphertext: &'a [u8],
    key_generation: &'a [u8; 16],
    expires_at: u64,
}

impl<'a> ExistingPermission<'a> {
    fn user(permission: &'a foks_server_db::RemoteUserViewPermission) -> Self {
        Self {
            token_hash: &permission.token_hash,
            token_nonce: &permission.token_nonce,
            token_ciphertext: &permission.token_ciphertext,
            key_generation: &permission.key_generation,
            expires_at: permission.expires_at,
        }
    }

    fn team(permission: &'a foks_server_db::RemoteTeamViewPermission) -> Self {
        Self {
            token_hash: &permission.token_hash,
            token_nonce: &permission.token_nonce,
            token_ciphertext: &permission.token_ciphertext,
            key_generation: &permission.key_generation,
            expires_at: permission.expires_at,
        }
    }
}

impl TokenBinding<'_> {
    fn aad(&self) -> Vec<u8> {
        let mut aad = Vec::with_capacity(self.domain.len() + 33 * 3 + 32 + 16);
        aad.extend_from_slice(self.domain);
        aad.extend_from_slice(self.target);
        aad.extend_from_slice(self.viewer);
        aad.extend_from_slice(self.viewer_host);
        aad.extend_from_slice(self.token_hash);
        aad.extend_from_slice(self.key_generation);
        aad
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_permission_grant(
    domain: &[u8],
    target: &[u8],
    viewer: &[u8],
    viewer_host: &[u8],
    existing: Option<ExistingPermission<'_>>,
    active_key: &SecretKey,
    keys: &dyn HostKeyProvider,
    _now: u64,
    entropy: &dyn Entropy,
) -> Result<foks_server_db::RemoteUserViewGrant, RpcStatus> {
    let full_expiry = PERMISSION_EXPIRY_MICROSECONDS;
    let Some(existing) = existing else {
        let mut token_bytes = Zeroizing::new([0u8; 17]);
        entropy.fill(&mut token_bytes[1..]).map_err(internal)?;
        token_bytes[0] = PERMISSION_TOKEN_TAG;
        return seal_permission_grant(
            domain,
            target,
            viewer,
            viewer_host,
            active_key,
            &PermissionToken::new(*token_bytes),
            full_expiry,
            entropy,
        );
    };

    let extend = existing.expires_at != full_expiry;
    let migrate = *existing.key_generation != active_key.generation().as_bytes();
    if !extend && !migrate {
        return Ok(foks_server_db::RemoteUserViewGrant {
            token_hash: *existing.token_hash,
            token_nonce: *existing.token_nonce,
            token_ciphertext: existing.token_ciphertext.to_vec(),
            key_generation: *existing.key_generation,
            expires_at: existing.expires_at,
        });
    }

    let prior_key = crate::keys::load_capability_generation(keys, *existing.key_generation)
        .map_err(internal)?;
    let prior_binding = TokenBinding {
        domain,
        target,
        viewer,
        viewer_host,
        token_hash: existing.token_hash,
        key_generation: existing.key_generation,
    };
    let token = decrypt_token_parts(
        &prior_key,
        &prior_binding,
        existing.token_nonce,
        existing.token_ciphertext,
    )?;
    seal_permission_grant(
        domain,
        target,
        viewer,
        viewer_host,
        active_key,
        &token,
        if extend {
            full_expiry
        } else {
            existing.expires_at
        },
        entropy,
    )
}

#[allow(clippy::too_many_arguments)]
fn seal_permission_grant(
    domain: &[u8],
    target: &[u8],
    viewer: &[u8],
    viewer_host: &[u8],
    key: &SecretKey,
    token: &PermissionToken,
    expires_at: u64,
    entropy: &dyn Entropy,
) -> Result<foks_server_db::RemoteUserViewGrant, RpcStatus> {
    let mut nonce = [0u8; 24];
    entropy.fill(&mut nonce).map_err(internal)?;
    let token_hash = token_hash(token.expose());
    let key_generation = key.generation().as_bytes();
    let binding = TokenBinding {
        domain,
        target,
        viewer,
        viewer_host,
        token_hash: &token_hash,
        key_generation: &key_generation,
    };
    Ok(foks_server_db::RemoteUserViewGrant {
        token_hash,
        token_nonce: nonce,
        token_ciphertext: encrypt_token(key, &nonce, &binding, token.expose())?,
        key_generation,
        expires_at,
    })
}

fn encrypt_token(
    key: &SecretKey,
    nonce: &[u8; 24],
    binding: &TokenBinding<'_>,
    token: &[u8; 17],
) -> Result<Vec<u8>, RpcStatus> {
    XChaCha20Poly1305::new(key.expose().into())
        .encrypt(
            &XNonce::from(*nonce),
            Payload {
                msg: token,
                aad: &binding.aad(),
            },
        )
        .map_err(|_| RpcStatus::TransactionRetry)
}

fn decrypt_user_token(
    key: &SecretKey,
    permission: &foks_server_db::RemoteUserViewPermission,
) -> Result<PermissionToken, RpcStatus> {
    let binding = TokenBinding {
        domain: USER_PERMISSION_TOKEN_AAD,
        target: &permission.target_user_id,
        viewer: &permission.viewer_party_id,
        viewer_host: &permission.viewer_host_id,
        token_hash: &permission.token_hash,
        key_generation: &permission.key_generation,
    };
    decrypt_token_parts(
        key,
        &binding,
        &permission.token_nonce,
        &permission.token_ciphertext,
    )
}

fn decrypt_team_token(
    key: &SecretKey,
    permission: &foks_server_db::RemoteTeamViewPermission,
) -> Result<PermissionToken, RpcStatus> {
    let binding = TokenBinding {
        domain: TEAM_PERMISSION_TOKEN_AAD,
        target: &permission.target_team_id,
        viewer: &permission.viewer_party_id,
        viewer_host: &permission.viewer_host_id,
        token_hash: &permission.token_hash,
        key_generation: &permission.key_generation,
    };
    decrypt_token_parts(
        key,
        &binding,
        &permission.token_nonce,
        &permission.token_ciphertext,
    )
}

fn decrypt_token_parts(
    key: &SecretKey,
    binding: &TokenBinding<'_>,
    token_nonce: &[u8; 24],
    token_ciphertext: &[u8],
) -> Result<PermissionToken, RpcStatus> {
    if *binding.key_generation != key.generation().as_bytes() {
        return Err(RpcStatus::TransactionRetry);
    }
    let plaintext = Zeroizing::new(
        XChaCha20Poly1305::new(key.expose().into())
            .decrypt(
                &XNonce::from(*token_nonce),
                Payload {
                    msg: token_ciphertext,
                    aad: &binding.aad(),
                },
            )
            .map_err(|_| RpcStatus::TransactionRetry)?,
    );
    let bytes: [u8; 17] = plaintext
        .as_slice()
        .try_into()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    if bytes[0] != PERMISSION_TOKEN_TAG || token_hash(&bytes) != *binding.token_hash {
        return Err(RpcStatus::TransactionRetry);
    }
    Ok(PermissionToken::new(bytes))
}

fn token_hash(token: &[u8; 17]) -> [u8; 32] {
    foks_crypto::federation_permission_token_hash(&PermissionToken::new(*token))
}

pub(crate) fn permission_token_hash(token: &[u8; 17]) -> [u8; 32] {
    token_hash(token)
}

fn active_capability_key(
    reader: &foks_server_db::ReadDatabase,
    keys: &dyn HostKeyProvider,
) -> Result<SecretKey, RpcStatus> {
    let generation = reader
        .active_capability_key_generation()
        .map_err(internal)?;
    crate::keys::load_capability_generation(keys, generation).map_err(internal)
}

fn map_write_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::AuthorizationChanged
        | crate::Error::Database(foks_server_db::Error::AuthorizationChanged) => {
            permission_denied()
        }
        crate::Error::WriterQueue
        | crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::RateLimited,
        _ => RpcStatus::TransactionRetry,
    }
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

fn permission_denied() -> RpcStatus {
    RpcStatus::PermissionDenied("federation permission denied".to_owned())
}

fn internal(_: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::TransactionRetry
}
