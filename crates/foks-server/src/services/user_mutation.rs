//! User-chain link mutations: device provisioning, device revocation and
//! normalized username changes, each committed with its Merkle publication.

use std::sync::Arc;

use foks_proto::{EntityId, TreeRoot};
use foks_rpc::RpcStatus;

use super::passphrase::OwnedPassphraseMutation;
use super::user::{bad_arguments, permission_denied};
use super::IdentityPorts;
use crate::auth::Principal;
use crate::merkle::decode_stored_root;

/// Commits a provisionDevice (`provision`) or revokeDevice argument.
pub(crate) fn provision_or_revoke(
    ports: &IdentityPorts<'_>,
    argument: &[u8],
    principal: &Principal,
    provision: bool,
) -> Result<(), RpcStatus> {
    let decoded = if provision {
        crate::identity::mutation::Argument::Provision(
            foks_rpc::arguments::decode_provision_device(argument).map_err(bad_arguments)?,
        )
    } else {
        crate::identity::mutation::Argument::Revoke(
            foks_rpc::arguments::decode_revoke_device(argument).map_err(bad_arguments)?,
        )
    };
    commit(ports, argument, principal, decoded)
}

/// Validates a user-chain link against the signer's current authority and
/// commits it with its Merkle publication in one writer transaction.
pub(crate) fn commit(
    ports: &IdentityPorts<'_>,
    argument: &[u8],
    principal: &Principal,
    decoded: crate::identity::mutation::Argument,
) -> Result<(), RpcStatus> {
    const IDEMPOTENCY_RECORD_LIFETIME_MICROSECONDS: u64 = 24 * 60 * 60 * 1_000_000;
    let reader = ports.read_database()?;
    let exact_link = decoded.link().encoded().map_err(bad_arguments)?;
    let submitted_change = decoded
        .link()
        .decode_group_change()
        .map_err(bad_arguments)?;
    let signed_root = submitted_change.root.clone();
    let idempotency_key =
        foks_crypto::prefixed_hash_signable(foks_proto::LINK_OUTER_TYPE_ID, &exact_link)
            .map_err(bad_arguments)?;
    let request_hash = foks_crypto::prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, argument);
    let idempotency_now = ports
        .clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    match reader.idempotency_record(&idempotency_key, &request_hash, idempotency_now) {
        Ok(Some(idempotency_record)) if idempotency_record.response.is_empty() => return Ok(()),
        Ok(Some(_)) => return Err(RpcStatus::TransactionRetry),
        Ok(None) => {}
        Err(foks_server_db::Error::OperationConflict) => {
            return Err(bad_arguments("user mutation retry binding failed"));
        }
        Err(_) => return Err(RpcStatus::TransactionRetry),
    }
    let identity = reader
        .identity_by_active_device(principal.device_id())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(permission_denied)?;
    let uid = identity.uid;
    let writer = ports.writer.ok_or(RpcStatus::Unsupported)?;
    let keys = Arc::clone(ports.key_provider.ok_or(RpcStatus::Unsupported)?);
    let clock = Arc::clone(ports.clock);
    let host = ports.host().map_err(|_| RpcStatus::TransactionRetry)?;
    let hostchain_tail = ports.hostchain_tail.clone();
    let signer = identity.device_id.clone();
    let authenticated_credential = principal.device_id().to_vec();
    let is_rename = matches!(&decoded, crate::identity::mutation::Argument::Rename(..));
    drop(reader);
    let result = writer.call(move |database| {
        // A username change requires a link signed by an active interactive
        // owner device, authenticated with that owner's credential.
        if is_rename {
            database.require_username_change_signer(&uid, &authenticated_credential, &signer)?;
        }
        let now = clock.now_micros()?;
        let idempotency_expires_at = now
            .checked_add(IDEMPOTENCY_RECORD_LIFETIME_MICROSECONDS)
            .ok_or(crate::Error::Signup(
                "user mutation idempotency_record expiry",
            ))?;
        let authority = database
            .user_authority(&uid)?
            .ok_or(crate::Error::Signup("user mutation authority missing"))?;
        if mutation_cites_superseded_user_head(database, &authority, &host, &submitted_change)? {
            return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
        }
        let cited_root = require_cited_root(database, &signed_root)?;
        let command =
            crate::identity::mutation::validate(&authority, &host, &signer, decoded, signed_root)?;
        let authoritative_root = database
            .current_root()?
            .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
        if cited_root.epoch > authoritative_root.epoch {
            return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
        }
        if now / 1_000 < decode_stored_root(&authoritative_root)?.time {
            return Err(crate::Error::Signup("system clock moved backwards"));
        }
        let chain_key = foks_merkle_store::chain_key(
            0,
            &EntityId::from_bytes(command.uid.clone())?,
            command.sequence,
            Some(&authority.next_tree_location),
        )?;
        let mut leaves = vec![(chain_key, command.link_hash)];
        if let Some(name) = &command.username {
            leaves.push((
                foks_merkle_store::username_key(&name.normalized_name, &host, name.name_sequence)?,
                foks_merkle_store::username_leaf(&EntityId::from_bytes(command.uid.clone())?)?,
            ));
        }
        let settings_location = command
            .user_settings
            .as_ref()
            .map(|settings| {
                let state = database
                    .generic_chain(&command.uid, foks_proto::CHAIN_TYPE_USER_SETTINGS)?
                    .ok_or(crate::Error::Signup("user-settings subchain seed missing"))?;
                let location = if settings.sequence == 1 {
                    foks_crypto::subchain_tree_location(
                        &state.location_seed,
                        foks_proto::CHAIN_TYPE_USER_SETTINGS,
                    )?
                } else {
                    let index = usize::try_from(settings.sequence.saturating_sub(2))
                        .map_err(|_| crate::Error::Signup("settings sequence overflow"))?;
                    state
                        .links
                        .get(index)
                        .filter(|link| link.sequence.saturating_add(1) == settings.sequence)
                        .map(|link| link.next_tree_location)
                        .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?
                };
                let key = foks_merkle_store::chain_key(
                    foks_proto::CHAIN_TYPE_USER_SETTINGS,
                    &EntityId::from_bytes(command.uid.clone())?,
                    settings.sequence,
                    Some(&location),
                )?;
                leaves.push((key, settings.link_hash));
                Ok::<_, crate::Error>(location)
            })
            .transpose()?;
        let crate::merkle::PreparedPublication {
            merkle_commit,
            root_epoch,
            root_hash,
            exact_root,
            exact_signed_root,
            back_pointers,
        } = crate::merkle::prepare_publication(
            database,
            &authoritative_root,
            &leaves,
            hostchain_tail,
            now,
            keys.as_ref(),
        )?;
        let added = command.added.as_ref().map(|added| {
            let (role_type, visibility) = crate::identity::mutation::role_parts(added.role);
            foks_server_db::AddedCredential {
                device_id: &added.device_id,
                self_token: &added.self_token,
                hepk_fingerprint: &added.hepk_fingerprint,
                exact_hepk: &added.exact_hepk,
                exact_name: &added.exact_name,
                role_type,
                visibility,
                subkey_id: added.subkey_id.as_deref(),
                exact_subkey_box: added.exact_subkey_box.as_deref(),
                yubi_pq_hint: added.yubi_pq_hint.as_ref().map(|(slot, id)| (*slot, id)),
            }
        });
        let shared_keys = command
            .shared_keys
            .iter()
            .map(|key| {
                let (role_type, visibility) = crate::identity::mutation::role_parts(key.role);
                foks_server_db::SharedKeyMutation {
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
                let (role_type, visibility) = crate::identity::mutation::role_parts(parcel.role);
                foks_server_db::ParcelMutation {
                    device_id: &parcel.device_id,
                    sender_id: &command.signer,
                    role_type,
                    visibility,
                    generation: parcel.generation,
                    exact_parcel: &parcel.exact,
                }
            })
            .collect::<Vec<_>>();
        // Keep exact seed-box bytes alive for the duration of the commit.
        let exact_seed_chain = command
            .seed_chain
            .iter()
            .map(foks_proto::SeedChainBox::encoded)
            .collect::<Result<Vec<_>, _>>()?;
        let seed_chain = command
            .seed_chain
            .iter()
            .zip(&exact_seed_chain)
            .map(|(boxed, exact)| {
                let (role_type, visibility) = crate::identity::mutation::role_parts(boxed.role);
                foks_server_db::SeedChainMutation {
                    role_type,
                    visibility,
                    generation: boxed.generation,
                    exact_box: exact,
                }
            })
            .collect::<Vec<_>>();
        let passphrase = command
            .passphrase
            .as_ref()
            .map(|argument| {
                let current = database
                    .passphrase(&command.uid)?
                    .ok_or(crate::Error::Database(
                        foks_server_db::Error::PassphraseNotFound,
                    ))?;
                let mut owned = OwnedPassphraseMutation::from_argument(argument)?;
                owned.salt = current.salt;
                Ok::<_, crate::Error>(owned)
            })
            .transpose()?;
        let user_settings = command
            .user_settings
            .as_ref()
            .zip(settings_location.as_ref())
            .map(
                |(settings, current_tree_location)| foks_server_db::GenericLinkMutation {
                    entity_id: &command.uid,
                    chain_type: foks_proto::CHAIN_TYPE_USER_SETTINGS,
                    signer_credential_id: &settings.signer,
                    sequence: settings.sequence,
                    previous: settings.previous.as_ref(),
                    link_root_epoch: settings.root.epoch,
                    link_root_hash: &settings.root.hash,
                    current_tree_location,
                    next_tree_location: &settings.next_tree_location,
                    link_hash: &settings.link_hash,
                    exact_link: &settings.exact_link,
                    passphrase_info: Some(foks_server_db::GenericPassphraseInfo {
                        generation: settings.info.generation,
                        salt: settings.info.salt.as_ref(),
                        stretch_version: settings.info.stretch_version,
                    }),
                },
            );
        if command.link_hash != idempotency_key {
            return Err(crate::Error::Signup("user mutation link hash changed"));
        }
        database.commit_user_mutation(&foks_server_db::UserMutation {
            username: command.username.as_ref(),
            uid: &command.uid,
            signer_device_id: &command.signer,
            authenticated_credential_id: &authenticated_credential,
            expected_sequence: command.sequence,
            expected_tail_hash: &command.expected_tail_hash,
            link_hash: &command.link_hash,
            exact_link: &command.exact_link,
            current_tree_location: &authority.next_tree_location,
            next_tree_location: &command.next_tree_location,
            added_credential: added,
            revoked_device_id: command.revoked.as_deref(),
            shared_keys: &shared_keys,
            parcels: &parcels,
            seed_chain: &seed_chain,
            passphrase: passphrase
                .as_ref()
                .map(|passphrase| passphrase.as_database(now)),
            user_settings,
            cited_root_epoch: cited_root.epoch,
            expected_root_epoch: authoritative_root.epoch,
            expected_root_hash: &authoritative_root.root_hash,
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
            idempotency_expires_at,
        })?;
        Ok(())
    });
    result.map_err(map_user_mutation_error)
}

pub(super) fn map_user_mutation_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::Database(foks_server_db::Error::Reservation) => {
            bad_arguments("invalid rename reservation")
        }
        crate::Error::Database(foks_server_db::Error::AuthorizationChanged) => permission_denied(),
        crate::Error::Database(foks_server_db::Error::NameInUse) => RpcStatus::NameInUse,
        crate::Error::Database(foks_server_db::Error::StaleRoot) => {
            RpcStatus::RevokeRace("user chain or Merkle root changed".to_owned())
        }
        crate::Error::Database(foks_server_db::Error::OperationConflict) => {
            bad_arguments("user mutation retry binding failed")
        }
        other => crate::status::shared(&other, "user_mutation")
            .unwrap_or_else(|| bad_arguments("user mutation validation failed")),
    }
}

pub(super) fn require_cited_root(
    database: &foks_server_db::Database,
    cited: &TreeRoot,
) -> crate::Result<foks_server_db::RootSnapshot> {
    let root = database
        .root_at(cited.epoch)?
        .ok_or(crate::Error::Database(foks_server_db::Error::StaleRoot))?;
    decode_stored_root(&root)?;
    if root.root_hash != cited.hash {
        return Err(crate::Error::Database(foks_server_db::Error::StaleRoot));
    }
    Ok(root)
}

fn mutation_cites_superseded_user_head(
    database: &foks_server_db::Database,
    authority: &foks_server_db::UserAuthoritySnapshot,
    host: &EntityId,
    change: &foks_proto::UserGroupChange,
) -> crate::Result<bool> {
    if change.uid.as_bytes() != authority.uid
        || &change.host != host
        || change.seqno > authority.chain_sequence
    {
        return Ok(false);
    }
    let Some(previous_sequence) = change.seqno.checked_sub(1) else {
        return Ok(false);
    };
    if change.previous.is_none() {
        return Ok(false);
    }
    let chain = database
        .user_chain(&authority.uid)?
        .ok_or(crate::Error::Signup("user mutation chain missing"))?;
    let Some(historical_head) = chain
        .links
        .iter()
        .find(|link| link.sequence == previous_sequence)
    else {
        return Ok(false);
    };
    change_cites_historical_user_head(authority, host, change, historical_head)
}

fn change_cites_historical_user_head(
    authority: &foks_server_db::UserAuthoritySnapshot,
    host: &EntityId,
    change: &foks_proto::UserGroupChange,
    historical_head: &foks_server_db::UserChainLinkSnapshot,
) -> crate::Result<bool> {
    let historical_hash = foks_crypto::prefixed_hash_signable(
        foks_proto::LINK_OUTER_TYPE_ID,
        &historical_head.exact_link,
    )?;
    Ok(change.uid.as_bytes() == authority.uid
        && &change.host == host
        && change.seqno <= authority.chain_sequence
        && historical_head.sequence.checked_add(1) == Some(change.seqno)
        && change.previous == Some(historical_hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_mutation_validation_failures_are_bad_arguments() {
        assert_eq!(
            map_user_mutation_error(crate::Error::Signup("invalid user link")),
            bad_arguments("user mutation validation failed")
        );
    }

    #[test]
    fn only_an_exact_historical_user_head_is_a_retryable_race() {
        let uid = EntityId::from_bytes(
            std::iter::once(foks_proto::ENTITY_USER)
                .chain([0x31; 32])
                .collect(),
        )
        .unwrap();
        let host = EntityId::from_bytes(
            std::iter::once(foks_proto::ENTITY_HOST)
                .chain([0x41; 32])
                .collect(),
        )
        .unwrap();
        let historical = foks_server_db::UserChainLinkSnapshot {
            sequence: 2,
            exact_link: vec![0x91, 0x01],
            root_epoch: 2,
            next_tree_location: [0x51; 32],
        };
        let historical_hash = foks_crypto::prefixed_hash_signable(
            foks_proto::LINK_OUTER_TYPE_ID,
            &historical.exact_link,
        )
        .unwrap();
        let authority = foks_server_db::UserAuthoritySnapshot {
            uid: uid.as_bytes().to_vec(),
            chain_sequence: 3,
            chain_tail_hash: [0x61; 32],
            next_tree_location: [0x71; 32],
            current_root_epoch: 3,
            current_root_hash: [0x81; 32],
            devices: Vec::new(),
            shared_keys: Vec::new(),
            stale_shared_key_roles: Vec::new(),
        };
        let mut change = foks_proto::UserGroupChange {
            seqno: 3,
            previous: Some(historical_hash),
            root: TreeRoot {
                epoch: 2,
                hash: [0x91; 32],
            },
            time: 1,
            next_location_commitment: [0xa1; 32],
            uid,
            host: host.clone(),
            signer: EntityId::from_bytes(
                std::iter::once(foks_proto::ENTITY_DEVICE)
                    .chain([0xb1; 32])
                    .collect(),
            )
            .unwrap(),
            changes: Vec::new(),
            shared_keys: Vec::new(),
            metadata: Vec::new(),
        };

        assert!(
            change_cites_historical_user_head(&authority, &host, &change, &historical).unwrap()
        );
        change.previous = Some([0xc1; 32]);
        assert!(
            !change_cites_historical_user_head(&authority, &host, &change, &historical).unwrap()
        );
        change.previous = Some(historical_hash);
        change.seqno = 4;
        assert!(
            !change_cites_historical_user_head(&authority, &host, &change, &historical).unwrap()
        );
    }
}
