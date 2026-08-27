use std::sync::Arc;

use foks_proto::{EntityId, MerkleRoot, SignedBlob, UsernameReservation};
use foks_rpc::RpcStatus;
use foks_snowpack::Value;

use crate::auth::Principal;
use crate::identity::team_create::{self, Argument};
use crate::keys::{HostKeyProvider, KeyPurpose};
use crate::{Entropy, WriterHandle};

const RESERVATION_LIFETIME_MICROSECONDS: u64 = 10 * 60 * 1_000_000;
const RECEIPT_LIFETIME_MICROSECONDS: u64 = 24 * 60 * 60 * 1_000_000;
const ADMIN_TOKEN_LIFETIME_MICROSECONDS: u64 = 6 * 60 * 60 * 1_000_000;

pub(crate) fn reserve_name(
    argument: &[u8],
    principal: &Principal,
    writer: &WriterHandle,
    clock: &dyn foks_server_db::Clock,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let name = foks_rpc::arguments::decode_team_name_reservation_request(argument)
        .map_err(bad_arguments)?;
    let normalized = foks_verify::normalize_username(&name)
        .filter(|normalized| normalized == &name)
        .ok_or_else(|| bad_arguments("team name is not normalized"))?;
    let now = clock.now_micros().map_err(internal)?;
    let expires_at = now
        .checked_add(RESERVATION_LIFETIME_MICROSECONDS)
        .ok_or_else(|| internal("team-name expiry overflow"))?;
    let mut token = [0_u8; 17];
    entropy.fill(&mut token).map_err(internal)?;
    let reservation = UsernameReservation {
        token,
        sequence: 1,
        expires_at,
    };
    writer
        .call(move |database| {
            database.reserve_team_name(&normalized, &token, 1, now, expires_at)?;
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
    clock: &dyn foks_server_db::Clock,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let request =
        foks_rpc::arguments::decode_make_team_bearer_token(argument).map_err(bad_arguments)?;
    let (role_type, visibility) = crate::auth::team::role_parts(request.role);
    if visibility != 0 || !matches!(role_type, 2 | 3) {
        return Err(permission_denied());
    }
    let authority = reader
        .team_admin_authority(
            request.team.as_bytes(),
            principal.uid(),
            role_type,
            request.generation,
        )
        .map_err(internal)?
        .ok_or_else(permission_denied)?;
    let mut token = [0_u8; 16];
    entropy.fill(&mut token).map_err(internal)?;
    let token_hash = crate::auth::team::admin_token_hash(&token);
    let now = clock.now_micros().map_err(internal)?;
    let expires_at = now
        .checked_add(ADMIN_TOKEN_LIFETIME_MICROSECONDS)
        .ok_or_else(|| internal("team-admin token expiry overflow"))?;
    writer
        .call(move |database| {
            database.issue_team_admin_token(&token_hash, &authority, expires_at, now)?;
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
    clock: &dyn foks_server_db::Clock,
) -> Result<(), RpcStatus> {
    principal.require_ordinary_device()?;
    let activation =
        foks_rpc::arguments::decode_activate_team_bearer_token(argument).map_err(bad_arguments)?;
    let challenge = &activation.challenge;
    if challenge.user.as_bytes() != principal.uid()
        || challenge.user_host != *host
        || challenge.time == 0
    {
        return Err(permission_denied());
    }
    let (role_type, visibility) = crate::auth::team::role_parts(challenge.role);
    if visibility != 0 {
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
    foks_crypto::verify_typed(
        &verify_key,
        &activation.signature,
        foks_proto::TEAM_BEARER_TOKEN_CHALLENGE_BLOB_TYPE_ID,
        &challenge.encoded_blob().map_err(bad_arguments)?,
    )
    .map_err(|_| permission_denied())?;
    let now = clock.now_micros().map_err(internal)?;
    let maximum_future = now.saturating_add(5 * 60 * 1_000_000);
    if challenge.time > maximum_future {
        return Err(permission_denied());
    }
    let token_hash = crate::auth::team::admin_token_hash(&challenge.token);
    const ACTIVATION_TYPE_ID: u64 = 0x6d10_7e50_464f_4b53;
    let activation_hash = foks_crypto::prefixed_hash(ACTIVATION_TYPE_ID, argument);
    let activated = writer
        .call(move |database| {
            Ok(database.activate_team_admin_token(&token_hash, &activation_hash, now)?)
        })
        .map_err(map_write_error)?
        .ok_or(RpcStatus::Expired)?;
    if activated.member_id.as_slice() != principal.uid() {
        return Err(permission_denied());
    }
    Ok(())
}

pub(crate) fn load_removal_box(
    argument: &[u8],
    principal: &Principal,
    host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
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
    if authority.member_id.as_slice() != principal.uid() {
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
        Argument::Named(
            foks_rpc::arguments::decode_named_team_create(argument).map_err(bad_arguments)?,
        )
    } else {
        Argument::AdHoc(
            foks_rpc::arguments::decode_adhoc_team_create(argument).map_err(bad_arguments)?,
        )
    };
    let exact_link = match &decoded {
        Argument::Named(argument) => argument.edit.link.encoded(),
        Argument::AdHoc(argument) => argument.link.encoded(),
    }
    .map_err(bad_arguments)?;
    let idempotency_key = foks_crypto::prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &exact_link);
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
            let command = team_create::validate(
                decoded,
                &authority,
                &host,
                &EntityId::from_bytes(owner.clone())?,
            )?;
            if command.link_hash != idempotency_key {
                return Err(crate::Error::Signup("team creation identity changed"));
            }
            let authoritative = database
                .current_root()?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            let decoded_root = decode_root(&authoritative)?;
            if authoritative.epoch != command.expected_root_epoch
                || authoritative.root_hash != command.expected_root_hash
                || now < decoded_root.time
            {
                return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
            }
            let chain_key = foks_merkle_store::chain_key(3, &command.team, 1, None)?;
            let mut leaves = vec![(chain_key, command.link_hash)];
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
                time: now,
                back_pointers: foks_merkle_store::back_pointer_hash(&back_pointers)?,
                root_node: merkle_commit.root,
                hostchain: hostchain_tail,
            };
            let exact_root = root.encoded()?;
            let root_hash =
                foks_crypto::prefixed_hash(foks_proto::MERKLE_ROOT_TYPE_ID, &exact_root);
            let root_blob = foks_snowpack::encode(&Value::Binary(exact_root.clone()))?;
            let merkle_key = keys.load_or_create(KeyPurpose::Merkle)?;
            let exact_signed_root = SignedBlob {
                inner: exact_root.clone(),
                signature: foks_crypto::sign_ed25519_typed(
                    merkle_key.expose(),
                    foks_proto::MERKLE_ROOT_BLOB_TYPE_ID,
                    &root_blob,
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
                expected_root_epoch: command.expected_root_epoch,
                expected_root_hash: &command.expected_root_hash,
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
    let idempotency_key = foks_crypto::prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &exact_link);
    const REQUEST_TYPE_ID: u64 = 0x6d10_7e4e_464f_4b53;
    let request_hash = foks_crypto::prefixed_hash(REQUEST_TYPE_ID, argument);
    let response = foks_proto::TeamEditResult {
        local_invitees: Vec::new(),
    }
    .encoded()
    .map_err(internal)?;
    let receipt_now = clock.now_micros().map_err(internal)?;
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
            if now < decoded_root.time {
                return Err(crate::Error::Signup("system clock moved backwards"));
            }
            let team_id = decoded.link.decode_team_group_change()?.team;
            let team = database
                .team(team_id.as_bytes())?
                .ok_or(crate::Error::Database(foks_server_db::Error::Invalid(
                    "unknown team",
                )))?;
            if team.host_id != host.as_bytes() {
                return Err(crate::Error::Signup("team belongs to another host"));
            }
            let command = crate::identity::team_edit::validate(decoded, &team, &root, &uid)?;
            validate_local_member_keys(database, &command.members)?;
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
                time: now,
                back_pointers: foks_merkle_store::back_pointer_hash(&back_pointers)?,
                root_node: merkle_commit.root,
                hostchain: hostchain_tail,
            };
            let exact_root = next_root.encoded()?;
            let root_hash =
                foks_crypto::prefixed_hash(foks_proto::MERKLE_ROOT_TYPE_ID, &exact_root);
            let root_blob = foks_snowpack::encode(&Value::Binary(exact_root.clone()))?;
            let merkle_key = keys.load_or_create(KeyPurpose::Merkle)?;
            let exact_signed_root = SignedBlob {
                inner: exact_root.clone(),
                signature: foks_crypto::sign_ed25519_typed(
                    merkle_key.expose(),
                    foks_proto::MERKLE_ROOT_BLOB_TYPE_ID,
                    &root_blob,
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
                        role_type,
                        visibility,
                        generation: parcel.generation,
                        exact_parcel: &parcel.exact,
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
                    expected_root_epoch: command.expected_root_epoch,
                    expected_root_hash: &command.expected_root_hash,
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
    members: &[foks_verify::VerifiedTeamMemberState],
) -> crate::Result<()> {
    for member in members {
        let authority = database
            .user_authority(member.party.as_bytes())?
            .ok_or(crate::Error::Signup("team member user is missing"))?;
        let matching = authority.shared_keys.iter().any(|key| {
            let role = crate::auth::team::stored_role(key.role_type, key.visibility);
            role == Some(member.source_role)
                && key.generation == member.generation
                && key.verify_key == member.verify_key.as_bytes()
                && foks_proto::Hepk::decode(&key.exact_hepk)
                    .ok()
                    .and_then(|hepk| foks_crypto::hepk_fingerprint(&hepk).ok())
                    == Some(member.hepk_fingerprint)
        });
        if !matching {
            return Err(crate::Error::Signup("team member PUK is not current"));
        }
    }
    Ok(())
}

fn map_edit_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::StaleRoot) => {
            RpcStatus::TeamRace("team head or Merkle root changed".to_owned())
        }
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::QuotaExceeded,
        crate::Error::Database(foks_server_db::Error::ReceiptConflict) => {
            bad_arguments("team edit retry binding failed")
        }
        other => RpcStatus::TeamError(format!("team edit validation failed: {other}")),
    }
}

fn decode_root(root: &foks_server_db::RootSnapshot) -> crate::Result<MerkleRoot> {
    let decoded = MerkleRoot::decode(&root.exact_root)?;
    let hash = foks_crypto::prefixed_hash(foks_proto::MERKLE_ROOT_TYPE_ID, &root.exact_root);
    if decoded.epoch != root.epoch || decoded.root_node != root.root_node || hash != root.root_hash
    {
        return Err(crate::Error::Signup("stored Merkle root binding mismatch"));
    }
    Ok(decoded)
}

fn map_create_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::NameInUse) => RpcStatus::NameInUse,
        crate::Error::Database(foks_server_db::Error::Reservation) => RpcStatus::Expired,
        crate::Error::Database(foks_server_db::Error::StaleRoot) => RpcStatus::StaleRoot,
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::QuotaExceeded,
        crate::Error::Database(foks_server_db::Error::ReceiptConflict) => {
            bad_arguments("team creation retry binding failed")
        }
        other => RpcStatus::TeamError(format!("team creation validation failed: {other}")),
    }
}

fn map_write_error(error: crate::Error) -> RpcStatus {
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
