use std::sync::Arc;

use foks_proto::{
    EntityId, GenericChainResponse, GenericLinkPayload, LocalTeamListEntry, MerkleRoot,
    PostGenericLinkArgument, Role, SignedBlob,
};
use foks_rpc::RpcStatus;

use crate::auth::Principal;
use crate::keys::{HostKeyProvider, KeyPurpose};
use crate::{net::session::OwnedPassphraseMutation, WriterHandle};

pub(crate) enum PassphraseCompanion {
    Set(OwnedPassphraseMutation),
    Change(OwnedPassphraseMutation),
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn post(
    argument: &[u8],
    principal: &Principal,
    host: &EntityId,
    writer: &WriterHandle,
    keys: &Arc<dyn HostKeyProvider>,
    clock: &Arc<dyn foks_server_db::Clock>,
    hostchain_tail: &foks_proto::HostchainTail,
) -> Result<(), RpcStatus> {
    let argument =
        foks_rpc::arguments::decode_post_generic_link(argument).map_err(bad_arguments)?;
    commit(
        argument,
        principal,
        host,
        writer,
        keys,
        clock,
        hostchain_tail,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn commit(
    argument: PostGenericLinkArgument,
    principal: &Principal,
    host: &EntityId,
    writer: &WriterHandle,
    keys: &Arc<dyn HostKeyProvider>,
    clock: &Arc<dyn foks_server_db::Clock>,
    hostchain_tail: &foks_proto::HostchainTail,
    passphrase: Option<PassphraseCompanion>,
) -> Result<(), RpcStatus> {
    principal.require_ordinary_device()?;
    let entity = EntityId::from_bytes(principal.uid().to_vec()).map_err(internal)?;
    commit_for_entity(
        argument,
        principal,
        &entity,
        None,
        host,
        writer,
        keys,
        clock,
        hostchain_tail,
        passphrase,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_for_entity(
    argument: PostGenericLinkArgument,
    principal: &Principal,
    entity: &EntityId,
    authorized_signer: Option<&[u8]>,
    host: &EntityId,
    writer: &WriterHandle,
    keys: &Arc<dyn HostKeyProvider>,
    clock: &Arc<dyn foks_server_db::Clock>,
    hostchain_tail: &foks_proto::HostchainTail,
    passphrase: Option<PassphraseCompanion>,
) -> Result<(), RpcStatus> {
    commit_for_entity_with_invitation(
        argument,
        principal,
        entity,
        authorized_signer,
        host,
        writer,
        keys,
        clock,
        hostchain_tail,
        passphrase,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_for_entity_with_invitation(
    argument: PostGenericLinkArgument,
    principal: &Principal,
    entity: &EntityId,
    authorized_signer: Option<&[u8]>,
    host: &EntityId,
    writer: &WriterHandle,
    keys: &Arc<dyn HostKeyProvider>,
    clock: &Arc<dyn foks_server_db::Clock>,
    hostchain_tail: &foks_proto::HostchainTail,
    passphrase: Option<PassphraseCompanion>,
    invitation: Option<foks_server_db::LocalInvitationAdmission>,
) -> Result<(), RpcStatus> {
    let decoded = argument.link.decode_generic().map_err(bad_arguments)?;
    if decoded.entity != *entity
        || decoded.host != *host
        || authorized_signer.is_some_and(|signer| decoded.signer.as_bytes() != signer)
    {
        return Err(permission_denied());
    }
    if argument.link.signatures().len() != 1
        || foks_crypto::verify_typed(
            &decoded.signer,
            &argument.link.signatures()[0],
            foks_proto::LINK_OUTER_V1_TYPE_ID,
            &argument.link.signing_bytes(0).map_err(bad_arguments)?,
        )
        .is_err()
    {
        return Err(bad_arguments("invalid generic-link signature"));
    }
    let next_commitment = foks_crypto::prefixed_hash_signable(
        foks_proto::TREE_LOCATION_TYPE_ID,
        &foks_snowpack::encode(&foks_snowpack::Value::Binary(
            argument.next_tree_location.to_vec(),
        ))
        .map_err(bad_arguments)?,
    )
    .map_err(bad_arguments)?;
    if next_commitment != decoded.next_location_commitment {
        return Err(bad_arguments("generic-link location commitment mismatch"));
    }
    let (chain_type, passphrase_info) = match &decoded.payload {
        GenericLinkPayload::UserSettings(info) => (
            foks_proto::CHAIN_TYPE_USER_SETTINGS,
            Some((info.generation, info.salt, info.stretch_version)),
        ),
        GenericLinkPayload::TeamMembership(_) => (foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP, None),
    };
    if entity.entity_type() != foks_proto::ENTITY_USER
        && chain_type != foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP
    {
        return Err(bad_arguments(
            "team generic chains carry membership links only",
        ));
    }
    let exact_link = argument.link.encoded().map_err(bad_arguments)?;
    let link_hash =
        foks_crypto::prefixed_hash_signable(foks_proto::LINK_OUTER_TYPE_ID, &exact_link)
            .map_err(bad_arguments)?;
    let entity_id = entity.as_bytes().to_vec();
    let credential = principal.device_id().to_vec();
    let signer = decoded.signer.into_bytes();
    let sequence = decoded.sequence;
    let previous = decoded.previous;
    let cited_root = decoded.root;
    let next_tree_location = argument.next_tree_location;
    let keys = Arc::clone(keys);
    let clock = Arc::clone(clock);
    let hostchain_tail = hostchain_tail.clone();
    writer
        .call(move |database| {
            let now = clock.now_micros()?;
            let cited = require_cited_root(database, &cited_root)?;
            let authoritative = database
                .current_root()?
                .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
            let authoritative_root = decode_root(&authoritative)?;
            if cited.epoch > authoritative.epoch || now / 1_000 < authoritative_root.time {
                return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
            }
            let state = database
                .generic_chain(&entity_id, chain_type)?
                .ok_or(crate::Error::Signup("generic subchain seed missing"))?;
            let current_tree_location = if sequence == 1 {
                foks_crypto::subchain_tree_location(&state.location_seed, chain_type)?
            } else {
                let index = usize::try_from(sequence.saturating_sub(2))
                    .map_err(|_| crate::Error::Signup("generic chain sequence overflow"))?;
                state
                    .links
                    .get(index)
                    .filter(|link| link.sequence.saturating_add(1) == sequence)
                    .map(|link| link.next_tree_location)
                    .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?
            };
            let merkle_key = foks_merkle_store::chain_key(
                chain_type,
                &EntityId::from_bytes(entity_id.clone())?,
                sequence,
                Some(&current_tree_location),
            )?;
            let leaves = [(merkle_key, link_hash)];
            let merkle_commit = foks_merkle_store::prepare(
                &database.node_reader(),
                authoritative.root_node,
                &[foks_merkle_store::LeafChange::Set {
                    key: merkle_key,
                    value: link_hash,
                }],
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
            let merkle_signer = keys.load_or_create(KeyPurpose::Merkle)?;
            let exact_signed_root = SignedBlob {
                inner: exact_root.clone(),
                signature: foks_crypto::sign_ed25519_blob(
                    merkle_signer.expose(),
                    foks_proto::MERKLE_ROOT_BLOB_TYPE_ID,
                    &exact_root,
                )?,
            }
            .encoded()?;
            let passphrase_info =
                passphrase_info
                    .as_ref()
                    .map(|(generation, salt, stretch_version)| {
                        foks_server_db::GenericPassphraseInfo {
                            generation: *generation,
                            salt: salt.as_ref(),
                            stretch_version: *stretch_version,
                        }
                    });
            let passphrase = passphrase.as_ref().map(|action| match action {
                PassphraseCompanion::Set(value) => foks_server_db::GenericPassphraseAction::Set {
                    credential_id: &credential,
                    mutation: value.as_database(now),
                },
                PassphraseCompanion::Change(value) => {
                    foks_server_db::GenericPassphraseAction::Change {
                        credential_id: &credential,
                        mutation: value.as_database(now),
                    }
                }
            });
            database.commit_generic_mutation(&foks_server_db::GenericMutation {
                invitation: invitation.as_ref(),
                link: foks_server_db::GenericLinkMutation {
                    entity_id: &entity_id,
                    chain_type,
                    signer_credential_id: &signer,
                    sequence,
                    previous: previous.as_ref(),
                    link_root_epoch: cited_root.epoch,
                    link_root_hash: &cited_root.hash,
                    current_tree_location: &current_tree_location,
                    next_tree_location: &next_tree_location,
                    link_hash: &link_hash,
                    exact_link: &exact_link,
                    passphrase_info,
                },
                passphrase,
                expected_root_epoch: authoritative.epoch,
                expected_root_hash: &authoritative.root_hash,
                merkle_commit: &merkle_commit,
                merkle_leaves: &leaves,
                root_epoch,
                root_hash: &root_hash,
                exact_root: &exact_root,
                exact_signed_root: &exact_signed_root,
                back_pointers: &back_pointers,
                now,
            })?;
            Ok(())
        })
        .map_err(map_write_error)
}

pub(crate) fn load(
    database: &foks_server_db::ReadSnapshot<'_>,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    authorize(database, principal)?;
    let request =
        foks_rpc::arguments::decode_load_generic_chain(argument).map_err(bad_arguments)?;
    if request.entity.as_bytes() != principal.uid() {
        return Err(permission_denied());
    }
    load_for_entity(database, &request.entity, request.chain_type, request.start)
}

pub(crate) fn load_for_entity(
    database: &foks_server_db::ReadSnapshot<'_>,
    entity: &EntityId,
    chain_type: u64,
    start: u64,
) -> Result<Vec<u8>, RpcStatus> {
    let chain = database
        .generic_chain(entity.as_bytes(), chain_type)
        .map_err(internal)?
        .ok_or_else(|| RpcStatus::NotFound("generic chain not found".to_owned()))?;
    let maximum_start = u64::try_from(chain.links.len())
        .ok()
        .and_then(|length| length.checked_add(1))
        .ok_or(RpcStatus::TransactionRetry)?;
    if start == 0 || start > maximum_start {
        return Err(bad_arguments("generic-chain start is out of range"));
    }
    let start_index = usize::try_from(start - 1).map_err(internal)?;
    let selected = &chain.links[start_index..];
    let exact_links = selected
        .iter()
        .map(|link| link.exact_link.clone())
        .collect::<Vec<_>>();
    let mut locations = if start == 1 {
        Vec::new()
    } else {
        vec![chain.links[start_index - 1].next_tree_location]
    };
    locations.extend(selected.iter().map(|link| link.next_tree_location));
    let mut current_location = if start == 1 {
        foks_crypto::subchain_tree_location(&chain.location_seed, chain_type).map_err(internal)?
    } else {
        chain.links[start_index - 1].next_tree_location
    };
    let root = database
        .current_root()
        .map_err(internal)?
        .ok_or(RpcStatus::MerkleNoRoot)?;
    decode_root_status(&root)?;
    let reader = database.node_reader();
    let mut paths = Vec::with_capacity(selected.len().saturating_add(1));
    for link in selected {
        let key = foks_merkle_store::chain_key(
            chain_type,
            entity,
            link.sequence,
            Some(&current_location),
        )
        .map_err(internal)?;
        paths.push(foks_merkle_store::proof(&reader, root.root_node, key).map_err(internal)?);
        current_location = link.next_tree_location;
    }
    let next_key =
        foks_merkle_store::chain_key(chain_type, entity, maximum_start, Some(&current_location))
            .map_err(internal)?;
    paths.push(foks_merkle_store::proof(&reader, root.root_node, next_key).map_err(internal)?);
    GenericChainResponse {
        exact_links: &exact_links,
        locations: &locations,
        exact_root: &root.exact_root,
        paths: &paths,
        location_seed: Some(chain.location_seed),
    }
    .encoded()
    .map_err(internal)
}

pub(crate) fn team_list(
    database: &foks_server_db::ReadSnapshot<'_>,
    host: &EntityId,
    argument: &[u8],
    principal: &Principal,
) -> Result<Vec<u8>, RpcStatus> {
    foks_rpc::arguments::decode_void(argument).map_err(bad_arguments)?;
    authorize(database, principal)?;
    let entries = database
        .local_team_list(principal.uid(), host.as_bytes())
        .map_err(internal)?
        .into_iter()
        .map(|entry| {
            Ok(LocalTeamListEntry {
                team: EntityId::from_bytes(entry.team_id).map_err(internal)?,
                source_role: stored_role(entry.source_role_type, entry.source_visibility)?,
                destination_role: stored_role(
                    entry.destination_role_type,
                    entry.destination_visibility,
                )?,
                team_sequence: entry.team_sequence,
                key_generation: entry.key_generation,
            })
        })
        .collect::<Result<Vec<_>, RpcStatus>>()?;
    foks_proto::encode_local_team_list(&entries).map_err(internal)
}

fn stored_role(role_type: u64, visibility: i64) -> Result<Role, RpcStatus> {
    match role_type {
        1 => i16::try_from(visibility)
            .map(Role::member)
            .map_err(internal),
        2 if visibility == 0 => Ok(Role::ADMIN),
        3 if visibility == 0 => Ok(Role::OWNER),
        _ => Err(RpcStatus::TransactionRetry),
    }
}

fn authorize(
    database: &foks_server_db::ReadSnapshot<'_>,
    principal: &Principal,
) -> Result<(), RpcStatus> {
    if database
        .active_credential_owner(principal.uid(), principal.device_id())
        .map_err(internal)?
        .is_some()
    {
        Ok(())
    } else {
        Err(permission_denied())
    }
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

fn decode_root_status(root: &foks_server_db::RootSnapshot) -> Result<MerkleRoot, RpcStatus> {
    decode_root(root).map_err(internal)
}

fn map_write_error(error: crate::Error) -> RpcStatus {
    if let Some(status) = crate::error::mutation_failure_status(&error) {
        return status;
    }
    match error {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::InvitationAlreadyPending) => {
            RpcStatus::TeamInviteAlreadyAccepted
        }
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::QuotaExceeded,
        crate::Error::Database(foks_server_db::Error::StaleRoot) => RpcStatus::StaleRoot,
        crate::Error::Database(foks_server_db::Error::AuthorizationChanged) => permission_denied(),
        crate::Error::Database(foks_server_db::Error::PassphraseNotFound) => {
            RpcStatus::PassphraseNotFound
        }
        crate::Error::Database(foks_server_db::Error::PassphraseGeneration) => {
            RpcStatus::BadArguments("passphrase generation is stale".to_owned())
        }
        _ => bad_arguments("generic mutation validation failed"),
    }
}

fn permission_denied() -> RpcStatus {
    RpcStatus::PermissionDenied("generic chain authorization failed".to_owned())
}

fn bad_arguments(_: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments("invalid generic-chain request".to_owned())
}

fn internal(_: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::TransactionRetry
}
