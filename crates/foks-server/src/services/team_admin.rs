use std::sync::Arc;

use foks_proto::{EntityId, MerkleRoot, SignedBlob, UsernameReservation};
use foks_rpc::RpcStatus;
use foks_server_db::{TeamAdminTokenActivation, TeamAdminTokenBinding, TeamAdminTokenIssue};
use foks_snowpack::Value;

use crate::auth::Principal;
use crate::identity::team_create::{self, Argument};
use crate::keys::{HostKeyProvider, KeyPurpose};
use crate::{Entropy, WriterHandle};

const RESERVATION_LIFETIME_MICROSECONDS: u64 = 10 * 60 * 1_000_000;
const RECEIPT_LIFETIME_MICROSECONDS: u64 = 24 * 60 * 60 * 1_000_000;
const ADMIN_TOKEN_LIFETIME_MICROSECONDS: u64 = 6 * 60 * 60 * 1_000_000;

pub(crate) fn config(
    argument: &[u8],
    principal: &Principal,
    database: &foks_server_db::ReadDatabase,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    foks_rpc::arguments::decode_void(argument).map_err(bad_arguments)?;
    let maximum_roles = u64::try_from(database.maximum_team_role_bands())
        .map_err(|_| RpcStatus::TransactionRetry)?;
    foks_proto::TeamConfig { maximum_roles }
        .encoded()
        .map_err(internal)
}

pub(crate) fn reserve_name(
    argument: &[u8],
    principal: &Principal,
    writer: &WriterHandle,
    clock: &Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let name = foks_rpc::arguments::decode_team_name_reservation_request(argument)
        .map_err(bad_arguments)?;
    let normalized = foks_verify::normalize_username(&name)
        .filter(|normalized| normalized == &name)
        .ok_or_else(|| bad_arguments("team name is not normalized"))?;
    let now = clock.now_micros().map_err(internal)?;
    // Store a millisecond-aligned value so the Go `lib.Time` response can be
    // replayed losslessly during named-team creation.
    let expires_at_millis = (now / 1_000)
        .checked_add(RESERVATION_LIFETIME_MICROSECONDS / 1_000)
        .ok_or_else(|| internal("team-name expiry overflow"))?;
    let expires_at = expires_at_millis
        .checked_mul(1_000)
        .ok_or_else(|| internal("team-name expiry overflow"))?;
    let mut token = [0_u8; 17];
    entropy.fill(&mut token).map_err(internal)?;
    let reservation = UsernameReservation {
        token,
        sequence: 1,
        expires_at: expires_at_millis,
    };
    writer
        .call_with_current_time(Arc::clone(clock), move |database, current_time| {
            database.reserve_team_name(&normalized, &token, 1, current_time, expires_at)?;
            Ok(())
        })
        .map_err(map_write_error)?;
    reservation.encoded().map_err(internal)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn make_inert_token(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    clock: &Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let request =
        foks_rpc::arguments::decode_make_team_bearer_token(argument).map_err(bad_arguments)?;
    let (role_type, visibility) = crate::auth::team::role_parts(request.role);
    if visibility != 0
        || !matches!(role_type, 2 | 3)
        || request.generation == 0
        || !matches!(
            request.team.entity_type(),
            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
        )
        || reader
            .active_credential_owner(principal.uid(), principal.device_id())
            .map_err(internal)?
            .is_none()
        || reader
            .team(request.team.as_bytes())
            .map_err(internal)?
            .is_none()
    {
        return Err(permission_denied());
    }
    let mut token = [0_u8; 16];
    entropy.fill(&mut token).map_err(internal)?;
    let token_hash = crate::auth::team::admin_token_hash(&token);
    let now = clock.now_micros().map_err(internal)?;
    let expires_at = now
        .checked_add(ADMIN_TOKEN_LIFETIME_MICROSECONDS)
        .ok_or_else(|| internal("team-admin token expiry overflow"))?;
    let team_id = request.team.into_bytes();
    let holder_id = principal.uid().to_vec();
    let ptk_generation = request.generation;
    writer
        .call_with_current_time(Arc::clone(clock), move |database, current_time| {
            database.issue_team_admin_token(TeamAdminTokenIssue {
                token_hash: &token_hash,
                binding: TeamAdminTokenBinding {
                    team_id: &team_id,
                    holder_id: &holder_id,
                    ptk_role_type: role_type,
                    ptk_generation,
                },
                expires_at,
                now: current_time,
            })?;
            Ok(())
        })
        .map_err(map_write_error)?;
    foks_snowpack::encode(&Value::Binary(token.to_vec())).map_err(internal)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn activate_token(
    argument: &[u8],
    principal: &Principal,
    host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    clock: &Arc<dyn foks_server_db::Clock>,
) -> Result<(), RpcStatus> {
    principal.require_ordinary_device()?;
    let activation =
        foks_rpc::arguments::decode_activate_team_bearer_token(argument).map_err(bad_arguments)?;
    let challenge = &activation.challenge;
    if challenge.user.as_bytes() != principal.uid()
        || challenge.user_host != *host
        || challenge.time == 0
        || reader
            .active_credential_owner(principal.uid(), principal.device_id())
            .map_err(internal)?
            .is_none()
    {
        return Err(permission_denied());
    }
    let (role_type, visibility) = crate::auth::team::role_parts(challenge.role);
    if visibility != 0 || !matches!(role_type, 2 | 3) {
        return Err(permission_denied());
    }
    let authority = reader
        .team_admin_authority(
            challenge.team.as_bytes(),
            principal.uid(),
            role_type,
            challenge.generation,
        )
        .map_err(internal)?
        .ok_or_else(permission_denied)?;
    let verify_key = EntityId::from_bytes(authority.ptk_verify_key).map_err(internal)?;
    foks_crypto::verify_blob(
        &verify_key,
        &activation.signature,
        foks_proto::TEAM_BEARER_TOKEN_CHALLENGE_BLOB_TYPE_ID,
        &challenge.encoded_payload().map_err(bad_arguments)?,
    )
    .map_err(|_| permission_denied())?;
    let now = clock.now_micros().map_err(internal)?;
    if !super::protocol_time_is_nowish(challenge.time, now) {
        return Err(permission_denied());
    }
    let token_hash = crate::auth::team::admin_token_hash(&challenge.token);
    const ACTIVATION_TYPE_ID: u64 = 0x6d10_7e50_464f_4b53;
    let activation_hash = foks_crypto::prefixed_hash(ACTIVATION_TYPE_ID, argument);
    let team_id = challenge.team.as_bytes().to_vec();
    let holder_id = principal.uid().to_vec();
    let ptk_generation = challenge.generation;
    writer
        .call_with_current_time(Arc::clone(clock), move |database, current_time| {
            Ok(
                database.activate_team_admin_token(TeamAdminTokenActivation {
                    token_hash: &token_hash,
                    activation_hash: &activation_hash,
                    binding: TeamAdminTokenBinding {
                        team_id: &team_id,
                        holder_id: &holder_id,
                        ptk_role_type: role_type,
                        ptk_generation,
                    },
                    now: current_time,
                })?,
            )
        })
        .map_err(map_write_error)?
        .ok_or(RpcStatus::Expired)?;
    Ok(())
}

/// Introspects an already-activated team admin bearer token and returns its
/// team id. The token must still be active and held by the authenticated user,
/// matching the binding `makeInertTeamBearerToken`/`activateTeamBearerToken`
/// established.
pub(crate) fn check_team_bearer_token(
    argument: &[u8],
    principal: &Principal,
    reader: &foks_server_db::ReadDatabase,
    clock: &Arc<dyn foks_server_db::Clock>,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let token =
        foks_rpc::arguments::decode_check_team_bearer_token(argument).map_err(bad_arguments)?;
    let now = clock.now_micros().map_err(internal)?;
    let authority = reader
        .resolve_team_admin_token(&crate::auth::team::admin_token_hash(&token), now)
        .map_err(internal)?
        .ok_or_else(|| {
            RpcStatus::TeamBearerTokenStale("team bearer token is not active".to_owned())
        })?;
    if authority.holder_id.as_slice() != principal.uid() {
        return Err(permission_denied());
    }
    foks_snowpack::encode(&foks_snowpack::Value::Binary(authority.team_id)).map_err(internal)
}

pub(crate) fn load_removal_box(
    argument: &[u8],
    principal: &Principal,
    host: &EntityId,
    reader: &foks_server_db::ReadSnapshot<'_>,
    clock: &dyn foks_server_db::Clock,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let request =
        foks_rpc::arguments::decode_load_removal_key_box(argument).map_err(bad_arguments)?;
    if request.member_host != *host {
        return Err(permission_denied());
    }
    let now = clock.now_micros().map_err(internal)?;
    let authority = reader
        .resolve_team_admin_token(&crate::auth::team::admin_token_hash(&request.token), now)
        .map_err(internal)?
        .ok_or(RpcStatus::Expired)?;
    if authority.holder_id.as_slice() != principal.uid()
        || reader
            .active_credential_owner(principal.uid(), principal.device_id())
            .map_err(internal)?
            .is_none()
    {
        return Err(permission_denied());
    }
    let (source_role_type, source_visibility) = crate::auth::team::role_parts(request.source_role);
    let exact = reader
        .team_removal_box(
            &authority.team_id,
            request.member.as_bytes(),
            request.member_host.as_bytes(),
            source_role_type,
            source_visibility,
        )
        .map_err(internal)?
        .ok_or(RpcStatus::TeamNoSourceRole)?;
    let outer = foks_proto::TeamRemovalBoxData::decode(&exact).map_err(internal)?;
    if outer.metadata.team.as_bytes() != authority.team_id
        || outer.metadata.member != request.member
        || outer.metadata.member_host != request.member_host
        || outer.metadata.source_role != request.source_role
    {
        return Err(RpcStatus::TeamRemovalKey(
            "stored removal-key binding mismatch".to_owned(),
        ));
    }
    outer.team_box.encoded().map_err(internal)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn post_team_membership_link(
    argument: &[u8],
    principal: &Principal,
    host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    keys: &Arc<dyn HostKeyProvider>,
    clock: &Arc<dyn foks_server_db::Clock>,
    hostchain_tail: &foks_proto::HostchainTail,
) -> Result<(), RpcStatus> {
    principal.require_ordinary_device()?;
    let request =
        foks_rpc::arguments::decode_post_team_membership_link(argument).map_err(bad_arguments)?;
    let now = clock.now_micros().map_err(internal)?;
    let authority = reader
        .resolve_team_admin_token(&crate::auth::team::admin_token_hash(&request.token), now)
        .map_err(internal)?
        .ok_or(RpcStatus::Expired)?;
    if authority.holder_id.as_slice() != principal.uid()
        || authority.ptk_role_type < 2
        || reader
            .active_credential_owner(principal.uid(), principal.device_id())
            .map_err(internal)?
            .is_none()
    {
        return Err(permission_denied());
    }
    let team = EntityId::from_bytes(authority.team_id).map_err(internal)?;
    super::generic::commit_for_entity(
        request.link,
        principal,
        &team,
        Some(&authority.ptk_verify_key),
        host,
        writer,
        keys,
        clock,
        hostchain_tail,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create(
    argument: &[u8],
    named: bool,
    principal: &Principal,
    host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    keys: &Arc<dyn HostKeyProvider>,
    clock: &Arc<dyn foks_server_db::Clock>,
    hostchain_tail: &foks_proto::HostchainTail,
) -> Result<(), RpcStatus> {
    principal.require_ordinary_device()?;
    let decoded = if named {
        Argument::Named(Box::new(
            foks_rpc::arguments::decode_named_team_create(argument).map_err(bad_arguments)?,
        ))
    } else {
        Argument::AdHoc(Box::new(
            foks_rpc::arguments::decode_adhoc_team_create(argument).map_err(bad_arguments)?,
        ))
    };
    let exact_link = match &decoded {
        Argument::Named(argument) => argument.edit.link.encoded(),
        Argument::AdHoc(argument) => argument.link.encoded(),
    }
    .map_err(bad_arguments)?;
    let signed_root = match &decoded {
        Argument::Named(argument) => argument.edit.link.decode_team_group_change(),
        Argument::AdHoc(argument) => argument.link.decode_team_group_change(),
    }
    .map_err(bad_arguments)?
    .root;
    let idempotency_key =
        foks_crypto::prefixed_hash_signable(foks_proto::LINK_OUTER_TYPE_ID, &exact_link)
            .map_err(bad_arguments)?;
    const REQUEST_TYPE_ID: u64 = 0x6d10_7e4d_464f_4b53;
    let request_hash = foks_crypto::prefixed_hash(REQUEST_TYPE_ID, argument);
    let receipt_now = clock.now_micros().map_err(internal)?;
    match reader.request_receipt(&idempotency_key, &request_hash, receipt_now) {
        Ok(Some(receipt)) if receipt.response.is_empty() => return Ok(()),
        Ok(Some(_)) => return Err(RpcStatus::TransactionRetry),
        Ok(None) => {}
        Err(foks_server_db::Error::ReceiptConflict) => {
            return Err(bad_arguments("team creation retry binding failed"));
        }
        Err(_) => return Err(RpcStatus::TransactionRetry),
    }
    let uid = principal.uid().to_vec();
    let credential = principal.device_id().to_vec();
    let host = host.clone();
    let keys = Arc::clone(keys);
    let clock = Arc::clone(clock);
    let hostchain_tail = hostchain_tail.clone();
    writer
        .call(move |database| {
            let now = clock.now_micros()?;
            let receipt_expires_at = now
                .checked_add(RECEIPT_LIFETIME_MICROSECONDS)
                .ok_or(crate::Error::Signup("team receipt expiry overflow"))?;
            let owner = database
                .active_credential_owner(&uid, &credential)?
                .ok_or(crate::Error::Signup("inactive team creator"))?;
            let authority = database
                .user_authority(&uid)?
                .ok_or(crate::Error::Signup("team creator authority missing"))?;
            let membership_chain = database
                .generic_chain(&uid, foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP)?
                .ok_or(crate::Error::Signup(
                    "team membership subchain seed missing",
                ))?;
            let cited_root = require_cited_root(database, &signed_root)?;
            let command = team_create::validate(
                decoded,
                &authority,
                &membership_chain,
                &host,
                &EntityId::from_bytes(owner.clone())?,
                signed_root,
            )?;
            if command.link_hash != idempotency_key {
                return Err(crate::Error::Signup("team creation identity changed"));
            }
            let authoritative = database
                .current_root()?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            let decoded_root = decode_root(&authoritative)?;
            if cited_root.epoch > authoritative.epoch || now / 1_000 < decoded_root.time {
                return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
            }
            let chain_key = foks_merkle_store::chain_key(3, &command.team, 1, None)?;
            let mut leaves = vec![(chain_key, command.link_hash)];
            let membership_location = if command.membership_link.sequence == 1 {
                foks_crypto::subchain_tree_location(
                    &membership_chain.location_seed,
                    foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP,
                )?
            } else {
                membership_chain
                    .links
                    .last()
                    .filter(|link| {
                        link.sequence.saturating_add(1) == command.membership_link.sequence
                    })
                    .map(|link| link.next_tree_location)
                    .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?
            };
            leaves.push((
                foks_merkle_store::chain_key(
                    foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP,
                    &EntityId::from_bytes(command.membership_link.user.clone())?,
                    command.membership_link.sequence,
                    Some(&membership_location),
                )?,
                command.membership_link.link_hash,
            ));
            if let Some(name) = &command.header.normalized_name {
                leaves.push((
                    foks_merkle_store::username_key(name, &host, command.header.name_sequence)?,
                    foks_merkle_store::username_leaf(&command.team)?,
                ));
            }
            let changes = leaves
                .iter()
                .map(|(key, value)| foks_merkle_store::LeafChange::Set {
                    key: *key,
                    value: *value,
                })
                .collect::<Vec<_>>();
            let merkle_commit = foks_merkle_store::prepare(
                &database.node_reader(),
                authoritative.root_node,
                &changes,
            )?;
            let root_epoch = authoritative
                .epoch
                .checked_add(1)
                .ok_or(crate::Error::Signup("Merkle epoch overflow"))?;
            let pointer_epochs = foks_merkle_store::back_pointer_sequence(root_epoch);
            let pointer_roots = database
                .roots_at(&pointer_epochs)?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            for root in &pointer_roots {
                decode_root(root)?;
            }
            let back_pointers = pointer_roots
                .into_iter()
                .map(|root| (root.epoch, root.root_hash))
                .collect::<Vec<_>>();
            let root = MerkleRoot {
                epoch: root_epoch,
                time: now / 1_000,
                back_pointers: foks_merkle_store::back_pointer_hash(root_epoch, &back_pointers)?,
                root_node: merkle_commit.root,
                hostchain: hostchain_tail,
                extensions: Vec::new(),
            };
            let exact_root = root.encoded()?;
            let root_hash =
                foks_crypto::prefixed_hash_signable(foks_proto::MERKLE_ROOT_TYPE_ID, &exact_root)?;
            let merkle_key = keys.load_or_create(KeyPurpose::Merkle)?;
            let exact_signed_root = SignedBlob {
                inner: exact_root.clone(),
                signature: foks_crypto::sign_ed25519_blob(
                    merkle_key.expose(),
                    foks_proto::MERKLE_ROOT_BLOB_TYPE_ID,
                    &exact_root,
                )?,
            }
            .encoded()?;
            let members = command
                .members
                .iter()
                .map(|member| {
                    let (source_type, source_visibility) =
                        crate::auth::team::role_parts(member.source_role);
                    let (role_type, visibility) = crate::auth::team::role_parts(member.role);
                    foks_server_db::TeamMemberMutation {
                        party_id: &member.party_id,
                        scoped_host_id: member.scoped_host_id.as_deref(),
                        source_role_type: source_type,
                        source_visibility,
                        role_type,
                        visibility,
                        generation: member.generation,
                        verify_key: &member.verify_key,
                        hepk_fingerprint: &member.hepk_fingerprint,
                        removal_key_commitment: member.removal_key_commitment.as_ref(),
                    }
                })
                .collect::<Vec<_>>();
            let shared_keys = command
                .shared_keys
                .iter()
                .map(|key| {
                    let (role_type, visibility) = crate::auth::team::role_parts(key.role);
                    foks_server_db::TeamSharedKeyMutation {
                        role_type,
                        visibility,
                        generation: key.generation,
                        verify_key: &key.verify_key,
                        exact_hepk: &key.exact_hepk,
                    }
                })
                .collect::<Vec<_>>();
            let parcels = command
                .parcels
                .iter()
                .map(|parcel| {
                    let (role_type, visibility) = crate::auth::team::role_parts(parcel.role);
                    foks_server_db::TeamParcelMutation {
                        party_id: &parcel.party_id,
                        sender_id: &parcel.sender_id,
                        target_role_type: parcel.target_role.protocol_value(),
                        target_visibility: i64::from(
                            parcel.target_role.visibility().unwrap_or_default(),
                        ),
                        role_type,
                        visibility,
                        generation: parcel.generation,
                        exact_parcel: &parcel.exact,
                    }
                })
                .collect::<Vec<_>>();
            let removal_boxes = command
                .removal_boxes
                .iter()
                .map(|boxed| {
                    let (role_type, visibility) = crate::auth::team::role_parts(boxed.source_role);
                    foks_server_db::TeamRemovalBoxMutation {
                        member_id: &boxed.member_id,
                        member_host_id: &boxed.member_host_id,
                        source_role_type: role_type,
                        source_visibility: visibility,
                        exact_box: &boxed.exact,
                    }
                })
                .collect::<Vec<_>>();
            let local_view_permissions = command
                .local_view_permissions
                .iter()
                .map(|(target, role)| {
                    let (minimum_role_type, minimum_role_visibility) =
                        crate::auth::team::role_parts(*role);
                    foks_server_db::TeamLocalViewPermissionMutation {
                        target_id: target,
                        minimum_role_type,
                        minimum_role_visibility,
                    }
                })
                .collect::<Vec<_>>();
            let (member_load_floor_type, member_load_floor_visibility) =
                crate::auth::team::role_parts(command.header.member_load_floor);
            database.commit_team_mutation(&foks_server_db::TeamMutation {
                team_id: command.team.as_bytes(),
                signer_credential_id: &owner,
                header: Some(foks_server_db::TeamHeader {
                    kind: command.header.kind,
                    host_id: host.as_bytes(),
                    normalized_name: command.header.normalized_name.as_deref(),
                    team_name_utf8: &command.header.team_name_utf8,
                    name_sequence: command.header.name_sequence,
                    name_commitment_key: command.header.name_commitment_key.as_ref(),
                    reservation_token: command.header.reservation_token.as_ref(),
                    reservation_expires_at: command.header.reservation_expires_at,
                    subchain_tree_location_seed: &command.header.subchain_tree_location_seed,
                    member_load_floor_type,
                    member_load_floor_visibility,
                }),
                expected_sequence: 1,
                expected_tail_hash: None,
                link_hash: &command.link_hash,
                exact_link: &command.exact_link,
                next_tree_location: &command.next_tree_location,
                members: &members,
                shared_keys: &shared_keys,
                parcels: &parcels,
                seed_chain: &[],
                removal_boxes: &removal_boxes,
                removal_proofs: &[],
                remote_member_view_tokens: &[],
                local_view_permissions: &local_view_permissions,
                generic_link: Some(foks_server_db::GenericLinkMutation {
                    entity_id: &command.membership_link.user,
                    chain_type: foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP,
                    signer_credential_id: &command.membership_link.signer,
                    sequence: command.membership_link.sequence,
                    previous: command.membership_link.previous.as_ref(),
                    link_root_epoch: command.membership_link.root.epoch,
                    link_root_hash: &command.membership_link.root.hash,
                    current_tree_location: &membership_location,
                    next_tree_location: &command.membership_link.next_tree_location,
                    link_hash: &command.membership_link.link_hash,
                    exact_link: &command.membership_link.exact_link,
                    passphrase_info: None,
                }),
                expected_root_epoch: authoritative.epoch,
                expected_root_hash: &authoritative.root_hash,
                merkle_commit: &merkle_commit,
                merkle_leaves: &leaves,
                root_epoch,
                root_hash: &root_hash,
                exact_root: &exact_root,
                exact_signed_root: &exact_signed_root,
                back_pointers: &back_pointers,
                idempotency_key: &idempotency_key,
                request_hash: &request_hash,
                response: &[],
                now,
                receipt_expires_at,
            })?;
            Ok(())
        })
        .map_err(map_create_error)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn edit(
    argument: &[u8],
    principal: &Principal,
    host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    keys: &Arc<dyn HostKeyProvider>,
    clock: &Arc<dyn foks_server_db::Clock>,
    hostchain_tail: &foks_proto::HostchainTail,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let decoded = foks_rpc::arguments::decode_team_edit(argument).map_err(bad_arguments)?;
    let exact_link = decoded.link.encoded().map_err(bad_arguments)?;
    let idempotency_key =
        foks_crypto::prefixed_hash_signable(foks_proto::LINK_OUTER_TYPE_ID, &exact_link)
            .map_err(bad_arguments)?;
    const REQUEST_TYPE_ID: u64 = 0x6d10_7e4e_464f_4b53;
    let request_hash =
        team_edit_receipt_hash_without_bearer(argument, REQUEST_TYPE_ID).map_err(bad_arguments)?;
    let response = foks_proto::TeamEditResult {
        local_invitees: Vec::new(),
    }
    .encoded()
    .map_err(internal)?;
    let receipt_now = clock.now_micros().map_err(internal)?;
    {
        let change = decoded
            .link
            .decode_team_group_change()
            .map_err(bad_arguments)?;
        let requires_bearer =
            team_edit_transport_requires_bearer(principal.uid(), &change.signer_owner.party);
        match decoded.team_bearer_token.as_ref() {
            Some(token) => {
                let authority = reader
                    .resolve_team_admin_token(
                        &crate::auth::team::admin_token_hash(token),
                        receipt_now,
                    )
                    .map_err(internal)?
                    .ok_or_else(|| {
                        RpcStatus::TeamBearerTokenStale("team bearer token is stale".into())
                    })?;
                if authority.holder_id.as_slice() != principal.uid()
                    || !team_edit_bearer_scope_allowed(
                        &authority.team_id,
                        &change.team,
                        &change.signer_owner.party,
                        false,
                    )
                {
                    return Err(permission_denied());
                }
            }
            None if requires_bearer => {
                // A team PTK can authorize the link, but the authenticated
                // transport still has to prove current control of that actor
                // team. Binding the bearer to the target team is insufficient
                // during a stale-child handoff: the retired child PTK holder
                // could already know the parent's PTKs.
                return Err(permission_denied());
            }
            None => {}
        }
    }
    match reader.request_receipt(&idempotency_key, &request_hash, receipt_now) {
        Ok(Some(receipt)) => return Ok(receipt.response),
        Ok(None) => {}
        Err(foks_server_db::Error::ReceiptConflict) => {
            return Err(bad_arguments("team edit retry binding failed"));
        }
        Err(_) => return Err(RpcStatus::TransactionRetry),
    }
    let uid = principal.uid().to_vec();
    let credential = principal.device_id().to_vec();
    let host = host.clone();
    let keys = Arc::clone(keys);
    let clock = Arc::clone(clock);
    let hostchain_tail = hostchain_tail.clone();
    writer
        .call(move |database| {
            let now = clock.now_micros()?;
            let receipt_expires_at = now
                .checked_add(RECEIPT_LIFETIME_MICROSECONDS)
                .ok_or(crate::Error::Signup("team edit receipt expiry overflow"))?;
            let owner = database
                .active_credential_owner(&uid, &credential)?
                .ok_or(crate::Error::Signup("inactive team editor"))?;
            let root = database
                .current_root()?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            let decoded_root = decode_root(&root)?;
            if now / 1_000 < decoded_root.time {
                return Err(crate::Error::Signup("system clock moved backwards"));
            }
            let change = decoded.link.decode_team_group_change()?;
            let team_bearer_token = decoded.team_bearer_token;
            let team_id = change.team.clone();
            let signer_owner = change.signer_owner.clone();
            let team = database
                .team(team_id.as_bytes())?
                .ok_or(crate::Error::Database(foks_server_db::Error::Invalid(
                    "unknown team",
                )))?;
            if team.host_id != host.as_bytes() {
                return Err(crate::Error::Signup("team belongs to another host"));
            }
            let signed_root = change.root;
            let cited_root = require_cited_root(database, &signed_root)?;
            let command = crate::identity::team_edit::validate(decoded, &team, &cited_root)?;
            let stale_nested_handoff = validate_local_member_keys(
                database,
                &team.members,
                &command.members,
                &command.introduced_keys,
                &host,
                &signer_owner.party,
                signer_owner.source_role,
            )?;
            validate_team_edit_bearer(
                database,
                team_bearer_token.as_ref(),
                &uid,
                &team_id,
                &signer_owner.party,
                stale_nested_handoff,
                now,
            )?;
            if command.link_hash != idempotency_key || command.team != team_id {
                return Err(crate::Error::Signup("team edit identity changed"));
            }
            let prior_location = team
                .links
                .last()
                .map(|link| link.next_tree_location)
                .ok_or(crate::Error::Signup("team head missing"))?;
            let chain_key = foks_merkle_store::chain_key(
                3,
                &command.team,
                command.sequence,
                Some(&prior_location),
            )?;
            let leaves = [(chain_key, command.link_hash)];
            let merkle_commit = foks_merkle_store::prepare(
                &database.node_reader(),
                root.root_node,
                &[foks_merkle_store::LeafChange::Set {
                    key: chain_key,
                    value: command.link_hash,
                }],
            )?;
            let root_epoch = root
                .epoch
                .checked_add(1)
                .ok_or(crate::Error::Signup("Merkle epoch overflow"))?;
            let pointer_epochs = foks_merkle_store::back_pointer_sequence(root_epoch);
            let pointer_roots = database
                .roots_at(&pointer_epochs)?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            for historical in &pointer_roots {
                decode_root(historical)?;
            }
            let back_pointers = pointer_roots
                .into_iter()
                .map(|historical| (historical.epoch, historical.root_hash))
                .collect::<Vec<_>>();
            let next_root = MerkleRoot {
                epoch: root_epoch,
                time: now / 1_000,
                back_pointers: foks_merkle_store::back_pointer_hash(root_epoch, &back_pointers)?,
                root_node: merkle_commit.root,
                hostchain: hostchain_tail,
                extensions: Vec::new(),
            };
            let exact_root = next_root.encoded()?;
            let root_hash =
                foks_crypto::prefixed_hash_signable(foks_proto::MERKLE_ROOT_TYPE_ID, &exact_root)?;
            let merkle_key = keys.load_or_create(KeyPurpose::Merkle)?;
            let exact_signed_root = SignedBlob {
                inner: exact_root.clone(),
                signature: foks_crypto::sign_ed25519_blob(
                    merkle_key.expose(),
                    foks_proto::MERKLE_ROOT_BLOB_TYPE_ID,
                    &exact_root,
                )?,
            }
            .encoded()?;
            let members = command
                .members
                .iter()
                .map(|member| {
                    let (source_role_type, source_visibility) =
                        crate::auth::team::role_parts(member.source_role);
                    let (role_type, visibility) = crate::auth::team::role_parts(member.role);
                    foks_server_db::TeamMemberMutation {
                        party_id: member.party.as_bytes(),
                        scoped_host_id: member.scoped_host.as_ref().map(EntityId::as_bytes),
                        source_role_type,
                        source_visibility,
                        role_type,
                        visibility,
                        generation: member.generation,
                        verify_key: member.verify_key.as_bytes(),
                        hepk_fingerprint: &member.hepk_fingerprint,
                        removal_key_commitment: member.removal_key_commitment.as_ref(),
                    }
                })
                .collect::<Vec<_>>();
            // Keep encoded HEPKs alive independently of the borrowed mutation rows.
            let exact_hepks = command
                .introduced_keys
                .iter()
                .map(|key| key.hepk.encoded())
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let shared_keys = command
                .introduced_keys
                .iter()
                .zip(&exact_hepks)
                .map(|(key, exact)| {
                    let (role_type, visibility) = crate::auth::team::role_parts(key.role);
                    foks_server_db::TeamSharedKeyMutation {
                        role_type,
                        visibility,
                        generation: key.generation,
                        verify_key: key.verify_key.as_bytes(),
                        exact_hepk: exact,
                    }
                })
                .collect::<Vec<_>>();
            let parcels = command
                .parcels
                .iter()
                .map(|parcel| {
                    let (role_type, visibility) = crate::auth::team::role_parts(parcel.role);
                    foks_server_db::TeamParcelMutation {
                        party_id: &parcel.party_id,
                        sender_id: &parcel.sender_id,
                        target_role_type: parcel.target_role.protocol_value(),
                        target_visibility: i64::from(
                            parcel.target_role.visibility().unwrap_or_default(),
                        ),
                        role_type,
                        visibility,
                        generation: parcel.generation,
                        exact_parcel: &parcel.exact,
                    }
                })
                .collect::<Vec<_>>();
            let local_view_permissions = command
                .local_view_permissions
                .iter()
                .map(|(target, role)| {
                    let (minimum_role_type, minimum_role_visibility) =
                        crate::auth::team::role_parts(*role);
                    foks_server_db::TeamLocalViewPermissionMutation {
                        target_id: target,
                        minimum_role_type,
                        minimum_role_visibility,
                    }
                })
                .collect::<Vec<_>>();
            let exact_seed_chain = command
                .seed_chain
                .iter()
                .map(foks_proto::SeedChainBox::encoded)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let seed_chain = command
                .seed_chain
                .iter()
                .zip(&exact_seed_chain)
                .map(|(boxed, exact)| {
                    let (role_type, visibility) = crate::auth::team::role_parts(boxed.role);
                    foks_server_db::TeamSeedChainMutation {
                        role_type,
                        visibility,
                        generation: boxed.generation,
                        exact_box: exact,
                    }
                })
                .collect::<Vec<_>>();
            let removal_boxes = command
                .removal_boxes
                .iter()
                .map(|boxed| {
                    let (source_role_type, source_visibility) =
                        crate::auth::team::role_parts(boxed.source_role);
                    foks_server_db::TeamRemovalBoxMutation {
                        member_id: &boxed.member_id,
                        member_host_id: &boxed.member_host_id,
                        source_role_type,
                        source_visibility,
                        exact_box: &boxed.exact,
                    }
                })
                .collect::<Vec<_>>();
            // A member tuple can be removed and later re-added with a new
            // removal key. Snapshot the box proven by this removal so an old
            // proof never starts resolving through the re-added member's box.
            let exact_removal_proof_boxes = command
                .removal_proofs
                .iter()
                .map(|proof| {
                    let (source_role_type, source_visibility) =
                        crate::auth::team::role_parts(proof.source_role);
                    database
                        .team_removal_box(
                            command.team.as_bytes(),
                            &proof.member_id,
                            &proof.member_host_id,
                            source_role_type,
                            source_visibility,
                        )?
                        .ok_or(crate::Error::Signup("team removal box is absent"))
                })
                .collect::<crate::Result<Vec<_>>>()?;
            let removal_proofs = command
                .removal_proofs
                .iter()
                .zip(&exact_removal_proof_boxes)
                .map(|(proof, exact_box)| {
                    let (source_role_type, source_visibility) =
                        crate::auth::team::role_parts(proof.source_role);
                    foks_server_db::TeamRemovalProofMutation {
                        commitment: &proof.commitment,
                        member_id: &proof.member_id,
                        member_host_id: &proof.member_host_id,
                        source_role_type,
                        source_visibility,
                        exact_box,
                        exact_removal: &proof.exact,
                    }
                })
                .collect::<Vec<_>>();
            let exact_remote_token_boxes = command
                .remote_member_view_tokens
                .iter()
                .map(|token| token.inner.secret_box.encoded())
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let remote_member_view_tokens = command
                .remote_member_view_tokens
                .iter()
                .zip(&exact_remote_token_boxes)
                .map(|(token, exact_secret_box)| {
                    let (ptk_role_type, ptk_visibility) =
                        crate::auth::team::role_parts(token.inner.ptk_role);
                    foks_server_db::TeamRemoteMemberViewTokenMutation {
                        member_party_id: token.inner.member.party.as_bytes(),
                        member_host_id: token.inner.member.host.as_bytes(),
                        ptk_generation: token.inner.ptk_generation,
                        ptk_role_type,
                        ptk_visibility,
                        exact_secret_box,
                        join_request_token: token.join_request.expose(),
                    }
                })
                .collect::<Vec<_>>();
            Ok(
                database.commit_team_mutation(&foks_server_db::TeamMutation {
                    team_id: command.team.as_bytes(),
                    signer_credential_id: &owner,
                    header: None,
                    expected_sequence: command.sequence,
                    expected_tail_hash: Some(&command.expected_tail_hash),
                    link_hash: &command.link_hash,
                    exact_link: &command.exact_link,
                    next_tree_location: &command.next_tree_location,
                    members: &members,
                    shared_keys: &shared_keys,
                    parcels: &parcels,
                    seed_chain: &seed_chain,
                    removal_boxes: &removal_boxes,
                    removal_proofs: &removal_proofs,
                    remote_member_view_tokens: &remote_member_view_tokens,
                    local_view_permissions: &local_view_permissions,
                    generic_link: None,
                    expected_root_epoch: root.epoch,
                    expected_root_hash: &root.root_hash,
                    merkle_commit: &merkle_commit,
                    merkle_leaves: &leaves,
                    root_epoch,
                    root_hash: &root_hash,
                    exact_root: &exact_root,
                    exact_signed_root: &exact_signed_root,
                    back_pointers: &back_pointers,
                    idempotency_key: &idempotency_key,
                    request_hash: &request_hash,
                    response: &response,
                    now,
                    receipt_expires_at,
                })?,
            )
        })
        .map_err(map_edit_error)
}

fn validate_local_member_keys(
    database: &foks_server_db::Database,
    current: &[foks_server_db::TeamMemberSnapshot],
    members: &[foks_verify::VerifiedTeamMemberState],
    introduced_keys: &[foks_verify::VerifiedSharedKey],
    local_host: &EntityId,
    signer_party: &EntityId,
    signer_source_role: foks_proto::Role,
) -> crate::Result<bool> {
    let mut stale_nested_handoff = false;
    if matches!(
        signer_party.entity_type(),
        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
    ) {
        let (source_role_type, source_visibility) =
            crate::auth::team::role_parts(signer_source_role);
        let prior = current
            .iter()
            .find(|member| {
                member.party_id == signer_party.as_bytes()
                    && member.scoped_host_id.is_none()
                    && member.source_role_type == source_role_type
                    && member.source_visibility == source_visibility
            })
            .ok_or(crate::Error::Signup("team editor is not a local member"))?;
        let nested = database
            .team(signer_party.as_bytes())?
            .ok_or(crate::Error::Signup("nested team editor is missing"))?;
        let current_key = nested
            .shared_keys
            .iter()
            .filter(|key| {
                crate::auth::team::stored_role(key.role_type, key.visibility)
                    == Some(signer_source_role)
            })
            .max_by_key(|key| key.generation);
        let current_signer = nested.host_id == local_host.as_bytes()
            && current_key.is_some_and(|key| {
                key.generation == prior.generation
                    && key.verify_key == prior.verify_key
                    && foks_proto::Hepk::decode(&key.exact_hepk)
                        .ok()
                        .and_then(|hepk| foks_crypto::hepk_fingerprint(&hepk).ok())
                        == Some(prior.hepk_fingerprint)
            });
        if !current_signer {
            if !stale_local_team_signer_handoff_is_exact(
                current,
                members,
                introduced_keys,
                &nested,
                local_host,
                signer_party,
                signer_source_role,
                prior,
                current_key,
            ) {
                return Err(crate::Error::Signup(
                    "stale team editor PTK lacks an exact current-key handoff",
                ));
            }
            stale_nested_handoff = true;
        }
    }
    for member in members {
        if member
            .scoped_host
            .as_ref()
            .is_some_and(|host| host != local_host)
        {
            continue;
        }
        let (source_role_type, source_visibility) =
            crate::auth::team::role_parts(member.source_role);
        let (role_type, visibility) = crate::auth::team::role_parts(member.role);
        let unchanged = current.iter().any(|prior| {
            prior.party_id == member.party.as_bytes()
                && prior.scoped_host_id.as_deref()
                    == member.scoped_host.as_ref().map(EntityId::as_bytes)
                && prior.source_role_type == source_role_type
                && prior.source_visibility == source_visibility
                && prior.role_type == role_type
                && prior.visibility == visibility
                && prior.generation == member.generation
                && prior.verify_key == member.verify_key.as_bytes()
                && prior.hepk_fingerprint == member.hepk_fingerprint
                && prior.removal_key_commitment == member.removal_key_commitment
        });
        let receives_introduced_key = introduced_keys.iter().any(|key| key.role <= member.role);
        let is_local_team_signer = member.party == *signer_party
            && member.source_role == signer_source_role
            && matches!(
                member.party.entity_type(),
                foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
            );
        if unchanged && !receives_introduced_key && !is_local_team_signer {
            // Rows outside the introduced PTK readership need not advance in
            // this edit. Every recipient is checked below so a retired PUK
            // can never receive fresh team material. A nested-team signer is
            // always checked so a stale PTK cannot authorize a no-op edit.
            continue;
        }
        let matching = match member.party.entity_type() {
            foks_proto::ENTITY_USER => {
                let authority = database
                    .user_authority(member.party.as_bytes())?
                    .ok_or(crate::Error::Signup("team member user is missing"))?;
                local_user_member_key_matches(
                    &authority,
                    member,
                    !unchanged || receives_introduced_key,
                )?
            }
            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM => {
                let nested = database
                    .team(member.party.as_bytes())?
                    .ok_or(crate::Error::Signup("nested team member is missing"))?;
                let current_generation = nested
                    .shared_keys
                    .iter()
                    .filter(|key| {
                        crate::auth::team::stored_role(key.role_type, key.visibility)
                            == Some(member.source_role)
                    })
                    .map(|key| key.generation)
                    .max();
                nested.host_id == local_host.as_bytes()
                    && current_generation == Some(member.generation)
                    && nested.shared_keys.iter().any(|key| {
                        crate::auth::team::stored_role(key.role_type, key.visibility)
                            == Some(member.source_role)
                            && key.generation == member.generation
                            && key.verify_key == member.verify_key.as_bytes()
                            && foks_proto::Hepk::decode(&key.exact_hepk)
                                .ok()
                                .and_then(|hepk| foks_crypto::hepk_fingerprint(&hepk).ok())
                                == Some(member.hepk_fingerprint)
                    })
            }
            _ => false,
        };
        if !matching {
            return Err(crate::Error::Signup("team member PUK is not current"));
        }
    }
    Ok(stale_nested_handoff)
}

#[allow(clippy::too_many_arguments)]
fn stale_local_team_signer_handoff_is_exact(
    current: &[foks_server_db::TeamMemberSnapshot],
    members: &[foks_verify::VerifiedTeamMemberState],
    introduced_keys: &[foks_verify::VerifiedSharedKey],
    nested: &foks_server_db::TeamSnapshot,
    local_host: &EntityId,
    signer_party: &EntityId,
    signer_source_role: foks_proto::Role,
    prior: &foks_server_db::TeamMemberSnapshot,
    current_key: Option<&foks_server_db::UserSharedKeySnapshot>,
) -> bool {
    let Some(current_key) = current_key.filter(|_| nested.host_id == local_host.as_bytes()) else {
        return false;
    };
    let handoff = members.iter().find(|member| {
        member.party == *signer_party
            && member.scoped_host.is_none()
            && member.source_role == signer_source_role
    });
    let exact_handoff = handoff.is_some_and(|member| {
        current_key.generation > prior.generation
            && member.generation == current_key.generation
            && member.verify_key.as_bytes() == current_key.verify_key
            && foks_proto::Hepk::decode(&current_key.exact_hepk)
                .ok()
                .and_then(|hepk| foks_crypto::hepk_fingerprint(&hepk).ok())
                == Some(member.hepk_fingerprint)
            && crate::auth::team::stored_role(prior.role_type, prior.visibility)
                == Some(member.role)
            && prior.removal_key_commitment == member.removal_key_commitment
    });
    let roster_shape_preserved = current.len() == members.len()
        && current.iter().all(|old| {
            members.iter().any(|member| {
                member.party.as_bytes() == old.party_id
                    && member.scoped_host.as_ref().map(EntityId::as_bytes)
                        == old.scoped_host_id.as_deref()
                    && crate::auth::team::stored_role(old.source_role_type, old.source_visibility)
                        == Some(member.source_role)
                    && crate::auth::team::stored_role(old.role_type, old.visibility)
                        == Some(member.role)
                    && old.removal_key_commitment == member.removal_key_commitment
            })
        });
    !introduced_keys.is_empty() && exact_handoff && roster_shape_preserved
}

fn local_user_member_key_matches(
    authority: &foks_server_db::UserAuthoritySnapshot,
    member: &foks_verify::VerifiedTeamMemberState,
    require_fresh: bool,
) -> crate::Result<bool> {
    let (source_role_type, source_visibility) = crate::auth::team::role_parts(member.source_role);
    if require_fresh
        && authority
            .stale_shared_key_roles
            .contains(&(source_role_type, source_visibility))
    {
        return Err(crate::Error::Signup("team member PUK is not current"));
    }
    let current_generation = authority
        .shared_keys
        .iter()
        .filter(|key| {
            crate::auth::team::stored_role(key.role_type, key.visibility)
                == Some(member.source_role)
        })
        .map(|key| key.generation)
        .max();
    Ok(current_generation == Some(member.generation)
        && authority.shared_keys.iter().any(|key| {
            crate::auth::team::stored_role(key.role_type, key.visibility)
                == Some(member.source_role)
                && key.generation == member.generation
                && key.verify_key == member.verify_key.as_bytes()
                && foks_proto::Hepk::decode(&key.exact_hepk)
                    .ok()
                    .and_then(|hepk| foks_crypto::hepk_fingerprint(&hepk).ok())
                    == Some(member.hepk_fingerprint)
        }))
}

#[cfg(test)]
mod tests {
    use super::local_user_member_key_matches;
    use foks_proto::{Role, SecretSeed, ENTITY_PUK_VERIFY, ENTITY_USER};

    fn entity(kind: u8, tag: u8) -> foks_proto::EntityId {
        let mut bytes = vec![tag; 33];
        bytes[0] = kind;
        foks_proto::EntityId::from_bytes(bytes).unwrap()
    }

    #[test]
    fn same_generation_stale_puk_cannot_receive_a_rotated_ptk() {
        let material =
            foks_crypto::derive_shared_public(&SecretSeed::new([7; 32]), ENTITY_PUK_VERIFY)
                .unwrap();
        let member = foks_verify::VerifiedTeamMemberState {
            party: entity(ENTITY_USER, 1),
            scoped_host: None,
            source_role: Role::OWNER,
            role: Role::OWNER,
            generation: 1,
            verify_key: material.verify_key.clone(),
            hepk_fingerprint: foks_crypto::hepk_fingerprint(&material.hepk).unwrap(),
            removal_key_commitment: Some([8; 32]),
            index_range: None,
        };
        let (role_type, visibility) = crate::auth::team::role_parts(Role::OWNER);
        let authority = foks_server_db::UserAuthoritySnapshot {
            uid: member.party.as_bytes().to_vec(),
            chain_sequence: 2,
            chain_tail_hash: [2; 32],
            next_tree_location: [3; 32],
            current_root_epoch: 4,
            current_root_hash: [5; 32],
            devices: Vec::new(),
            shared_keys: vec![foks_server_db::UserSharedKeySnapshot {
                role_type,
                visibility,
                generation: 1,
                verify_key: material.verify_key.as_bytes().to_vec(),
                exact_hepk: material.hepk.encoded().unwrap(),
            }],
            stale_shared_key_roles: vec![(role_type, visibility)],
        };

        assert!(local_user_member_key_matches(&authority, &member, false).unwrap());
        assert!(matches!(
            local_user_member_key_matches(&authority, &member, true),
            Err(crate::Error::Signup("team member PUK is not current"))
        ));
    }
}

fn map_edit_error(error: crate::Error) -> RpcStatus {
    if let Some(status) = crate::error::merkle_mint_status(&error) {
        return status;
    }
    match error {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::StaleRoot) => {
            RpcStatus::TeamRace("team head or Merkle root changed".to_owned())
        }
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::QuotaExceeded,
        crate::Error::Database(foks_server_db::Error::ReceiptConflict) => {
            bad_arguments("team edit retry binding failed")
        }
        crate::Error::Signup("team member PUK is not current") => {
            RpcStatus::TeamRace("team member PUK changed during edit".to_owned())
        }
        other => RpcStatus::TeamError(format!("team edit validation failed: {other}")),
    }
}

fn decode_root(root: &foks_server_db::RootSnapshot) -> crate::Result<MerkleRoot> {
    let decoded = MerkleRoot::decode(&root.exact_root)?;
    let hash =
        foks_crypto::prefixed_hash_signable(foks_proto::MERKLE_ROOT_TYPE_ID, &root.exact_root)?;
    if decoded.epoch != root.epoch || decoded.root_node != root.root_node || hash != root.root_hash
    {
        return Err(crate::Error::Signup("stored Merkle root binding mismatch"));
    }
    Ok(decoded)
}

fn require_cited_root(
    database: &foks_server_db::Database,
    cited: &foks_proto::TreeRoot,
) -> crate::Result<foks_server_db::RootSnapshot> {
    let root = database
        .root_at(cited.epoch)?
        .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
    decode_root(&root)?;
    if root.root_hash != cited.hash {
        return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
    }
    Ok(root)
}

fn map_create_error(error: crate::Error) -> RpcStatus {
    if let Some(status) = crate::error::merkle_mint_status(&error) {
        return status;
    }
    match error {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::NameInUse) => RpcStatus::NameInUse,
        crate::Error::Database(foks_server_db::Error::Reservation) => RpcStatus::Expired,
        crate::Error::Database(foks_server_db::Error::StaleRoot) => {
            RpcStatus::RevokeRace("team creation chain or Merkle root changed".to_owned())
        }
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::QuotaExceeded,
        crate::Error::Database(foks_server_db::Error::ReceiptConflict) => {
            bad_arguments("team creation retry binding failed")
        }
        other => RpcStatus::TeamError(format!("team creation validation failed: {other}")),
    }
}

fn map_write_error(error: crate::Error) -> RpcStatus {
    if let Some(status) = crate::error::merkle_mint_status(&error) {
        return status;
    }
    match error {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::NameInUse) => RpcStatus::NameInUse,
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::QuotaExceeded,
        _ => RpcStatus::TransactionRetry,
    }
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

fn internal(_: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::TransactionRetry
}

fn permission_denied() -> RpcStatus {
    RpcStatus::PermissionDenied("team-admin authorization failed".to_owned())
}

fn team_edit_transport_requires_bearer(principal: &[u8], signer_owner: &EntityId) -> bool {
    signer_owner.as_bytes() != principal
}

fn team_edit_bearer_scope_allowed(
    authority_team: &[u8],
    target_team: &EntityId,
    _signer_owner: &EntityId,
    _stale_nested_handoff: bool,
) -> bool {
    // Go scopes TeamAdmin bearer tokens to the team being edited, including
    // edits signed by a nested member team's PTK. The actor identity remains
    // independently bound by the signed roster key and authenticated holder.
    authority_team == target_team.as_bytes()
}

fn validate_team_edit_bearer(
    database: &foks_server_db::Database,
    token: Option<&foks_proto::TeamBearerToken>,
    principal: &[u8],
    target_team: &EntityId,
    signer_owner: &EntityId,
    stale_nested_handoff: bool,
    now: u64,
) -> crate::Result<()> {
    let requires_bearer = team_edit_transport_requires_bearer(principal, signer_owner);
    let Some(token) = token else {
        return if requires_bearer {
            Err(crate::Error::Signup(
                "nested team edit lacks transport bearer authority",
            ))
        } else {
            Ok(())
        };
    };
    let authority = database
        .resolve_team_admin_token(&crate::auth::team::admin_token_hash(token), now)?
        .ok_or(crate::Error::Signup("team edit bearer is stale"))?;
    if authority.holder_id.as_slice() != principal
        || !team_edit_bearer_scope_allowed(
            &authority.team_id,
            target_team,
            signer_owner,
            stale_nested_handoff,
        )
    {
        return Err(crate::Error::Signup(
            "team edit bearer does not match its actor and target",
        ));
    }
    Ok(())
}

fn team_edit_receipt_hash_without_bearer(
    argument: &[u8],
    request_type_id: u64,
) -> std::result::Result<[u8; 32], &'static str> {
    let mut decoded =
        foks_snowpack::decode(argument).map_err(|_| "team edit argument is not canonical")?;
    let foks_snowpack::Value::Array(fields) = &mut decoded else {
        return Err("team edit argument is not an array");
    };
    if fields.len() != 5 {
        return Err("team edit argument has the wrong field count");
    }
    fields[3] = Value::Null;
    let canonical =
        foks_snowpack::encode(&decoded).map_err(|_| "team edit argument cannot be encoded")?;
    Ok(foks_crypto::prefixed_hash(request_type_id, &canonical))
}

#[cfg(test)]
mod stale_handoff_tests {
    use super::{
        stale_local_team_signer_handoff_is_exact, team_edit_bearer_scope_allowed,
        team_edit_transport_requires_bearer,
    };

    fn entity(kind: u8, fill: u8) -> foks_proto::EntityId {
        let mut bytes = vec![fill; 33];
        bytes[0] = kind;
        foks_proto::EntityId::from_bytes(bytes).unwrap()
    }

    #[test]
    fn stale_nested_signer_requires_exact_current_key_handoff() {
        let host = entity(foks_proto::ENTITY_HOST, 0x11);
        let signer = entity(foks_proto::ENTITY_NAMED_TEAM, 0x22);
        let old = foks_crypto::derive_shared_public(
            &foks_proto::SecretSeed::new([0x31; 32]),
            foks_proto::ENTITY_PTK_VERIFY,
        )
        .unwrap();
        let new = foks_crypto::derive_shared_public(
            &foks_proto::SecretSeed::new([0x32; 32]),
            foks_proto::ENTITY_PTK_VERIFY,
        )
        .unwrap();
        let parent = foks_crypto::derive_shared_public(
            &foks_proto::SecretSeed::new([0x33; 32]),
            foks_proto::ENTITY_PTK_VERIFY,
        )
        .unwrap();
        let removal = [0x44; 32];
        let prior = foks_server_db::TeamMemberSnapshot {
            party_id: signer.as_bytes().to_vec(),
            scoped_host_id: None,
            source_role_type: 2,
            source_visibility: 0,
            role_type: 3,
            visibility: 0,
            generation: 1,
            verify_key: old.verify_key.as_bytes().to_vec(),
            hepk_fingerprint: foks_crypto::hepk_fingerprint(&old.hepk).unwrap(),
            removal_key_commitment: Some(removal),
        };
        let current = vec![prior.clone()];
        let mut members = vec![foks_verify::VerifiedTeamMemberState {
            party: signer.clone(),
            scoped_host: None,
            source_role: foks_proto::Role::ADMIN,
            role: foks_proto::Role::OWNER,
            generation: 2,
            verify_key: new.verify_key.clone(),
            hepk_fingerprint: foks_crypto::hepk_fingerprint(&new.hepk).unwrap(),
            removal_key_commitment: Some(removal),
            index_range: None,
        }];
        let introduced = vec![foks_verify::VerifiedSharedKey {
            role: foks_proto::Role::OWNER,
            generation: 2,
            verify_key: parent.verify_key,
            hepk: parent.hepk,
        }];
        let nested = foks_server_db::TeamSnapshot {
            team_id: signer.as_bytes().to_vec(),
            kind: foks_proto::ENTITY_NAMED_TEAM,
            host_id: host.as_bytes().to_vec(),
            normalized_name: Some(b"nested".to_vec()),
            team_name_utf8: b"nested".to_vec(),
            team_name_sequence: 1,
            team_name_commitment_key: Some([0x51; 16]),
            member_load_floor_type: 1,
            member_load_floor_visibility: 0,
            links: Vec::new(),
            members: Vec::new(),
            shared_keys: vec![
                foks_server_db::UserSharedKeySnapshot {
                    role_type: 2,
                    visibility: 0,
                    generation: 1,
                    verify_key: old.verify_key.as_bytes().to_vec(),
                    exact_hepk: old.hepk.encoded().unwrap(),
                },
                foks_server_db::UserSharedKeySnapshot {
                    role_type: 2,
                    visibility: 0,
                    generation: 2,
                    verify_key: new.verify_key.as_bytes().to_vec(),
                    exact_hepk: new.hepk.encoded().unwrap(),
                },
            ],
        };
        let current_key = nested.shared_keys.last();
        assert!(stale_local_team_signer_handoff_is_exact(
            &current,
            &members,
            &introduced,
            &nested,
            &host,
            &signer,
            foks_proto::Role::ADMIN,
            &prior,
            current_key,
        ));

        members[0].role = foks_proto::Role::ADMIN;
        assert!(!stale_local_team_signer_handoff_is_exact(
            &current,
            &members,
            &introduced,
            &nested,
            &host,
            &signer,
            foks_proto::Role::ADMIN,
            &prior,
            current_key,
        ));
        members[0].role = foks_proto::Role::OWNER;
        assert!(!stale_local_team_signer_handoff_is_exact(
            &current,
            &members,
            &[],
            &nested,
            &host,
            &signer,
            foks_proto::Role::ADMIN,
            &prior,
            current_key,
        ));
    }

    #[test]
    fn nested_team_signers_require_transport_bearer_authority() {
        let user = entity(foks_proto::ENTITY_USER, 0x11);
        let same_user = entity(foks_proto::ENTITY_USER, 0x11);
        let nested_team = entity(foks_proto::ENTITY_NAMED_TEAM, 0x22);

        assert!(!team_edit_transport_requires_bearer(
            user.as_bytes(),
            &same_user,
        ));
        assert!(team_edit_transport_requires_bearer(
            user.as_bytes(),
            &nested_team,
        ));

        let parent = entity(foks_proto::ENTITY_NAMED_TEAM, 0x44);
        assert!(!team_edit_bearer_scope_allowed(
            nested_team.as_bytes(),
            &parent,
            &nested_team,
            false,
        ));
        assert!(team_edit_bearer_scope_allowed(
            parent.as_bytes(),
            &parent,
            &nested_team,
            false,
        ));
        assert!(team_edit_bearer_scope_allowed(
            parent.as_bytes(),
            &parent,
            &nested_team,
            true,
        ));
        assert!(!team_edit_bearer_scope_allowed(
            nested_team.as_bytes(),
            &parent,
            &nested_team,
            true,
        ));
        assert!(team_edit_bearer_scope_allowed(
            parent.as_bytes(),
            &parent,
            &user,
            false,
        ));
    }
}
