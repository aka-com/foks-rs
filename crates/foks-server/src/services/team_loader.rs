use std::sync::Arc;

use foks_proto::{ActivatedTeamView, EntityId, TeamViewChallenge};
use foks_rpc::RpcStatus;

use crate::auth::{team, Principal};
use crate::keys::HostKeyProvider;
use crate::{Entropy, WriterHandle};

const VIEW_LIFETIME_MICROSECONDS: u64 = 6 * 60 * 60 * 1_000_000;

#[allow(clippy::too_many_arguments)]
pub(crate) fn issue_challenge(
    argument: &[u8],
    principal: Option<&Principal>,
    host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
    _writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
    entropy: &dyn Entropy,
) -> Result<Vec<u8>, RpcStatus> {
    if let Some(p) = principal {
        p.require_ordinary_device()?;
    }
    let request = foks_rpc::arguments::decode_team_view_request(argument).map_err(bad_arguments)?;
    if request.host != *host
        || !matches!(
            request.team.entity_type(),
            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
        )
        || !matches!(
            request.member.entity_type(),
            foks_proto::ENTITY_USER
                | foks_proto::ENTITY_NAMED_TEAM
                | foks_proto::ENTITY_AD_HOC_TEAM
        )
    {
        return Err(permission_denied());
    }
    require_view_transport(principal, &request.member_host, host, reader)?;
    let key = crate::keys::load_capability_generation(
        keys,
        reader
            .active_capability_key_generation()
            .map_err(|_| RpcStatus::TransactionRetry)?,
    )
    .map_err(|_| RpcStatus::TransactionRetry)?;
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let mut token = [0; 16];
    entropy
        .fill(&mut token)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let mut challenge = TeamViewChallenge {
        request,
        time: now / 1_000,
        token,
        key_id: key.generation().as_bytes(),
        mac: [0; 32],
    };
    let payload = challenge
        .payload_encoded()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    challenge.mac = foks_crypto::capability_mac(
        key.expose(),
        foks_proto::TEAM_VIEW_CHALLENGE_TYPE_ID,
        &payload,
    );
    challenge.encoded().map_err(|_| RpcStatus::TransactionRetry)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn activate(
    argument: &[u8],
    principal: Option<&Principal>,
    host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
    writer: &WriterHandle,
    keys: &dyn HostKeyProvider,
    clock: &Arc<dyn foks_server_db::Clock>,
) -> Result<Vec<u8>, RpcStatus> {
    if let Some(p) = principal {
        p.require_ordinary_device()?;
    }
    let activation =
        foks_rpc::arguments::decode_activate_team_view(argument).map_err(bad_arguments)?;
    let challenge = &activation.challenge;
    if challenge.request.host != *host {
        return Err(permission_denied());
    }
    require_view_transport(principal, &challenge.request.member_host, host, reader)?;
    let observed_now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let observed_now_millis = observed_now / 1_000;
    if challenge
        .time
        .checked_add(VIEW_LIFETIME_MICROSECONDS / 1_000)
        .is_none_or(|expires_at| expires_at <= observed_now_millis)
        || challenge.time > observed_now_millis
    {
        return Err(RpcStatus::Expired);
    }
    let key = crate::keys::load_capability_generation(keys, challenge.key_id)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let payload = challenge.payload_encoded().map_err(bad_arguments)?;
    foks_crypto::verify_capability_mac(
        key.expose(),
        foks_proto::TEAM_VIEW_CHALLENGE_TYPE_ID,
        &payload,
        &challenge.mac,
    )
    .map_err(|_| permission_denied())?;
    let (role, visibility) = team::role_parts(challenge.request.source_role);
    let authority = reader
        .team_view_authority(
            challenge.request.team.as_bytes(),
            challenge.request.member.as_bytes(),
            challenge.request.member_host.as_bytes(),
            role,
            visibility,
            challenge.request.generation,
        )
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(permission_denied)?;
    let verify_key = EntityId::from_bytes(authority.source_verify_key.clone())
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let exact_challenge = challenge.encoded().map_err(bad_arguments)?;
    foks_crypto::verify_typed(
        &verify_key,
        &activation.signature,
        foks_proto::TEAM_VIEW_CHALLENGE_TYPE_ID,
        &exact_challenge,
    )
    .map_err(|_| permission_denied())?;
    let challenge_hash = team::challenge_hash(&exact_challenge);
    let token_hash = team::token_hash(&challenge.token);
    let key_generation = challenge.key_id;
    let expires_at = challenge
        .time
        .checked_mul(1_000)
        .and_then(|time| time.checked_add(VIEW_LIFETIME_MICROSECONDS))
        .ok_or(RpcStatus::Expired)?;
    const ACTIVATION_TYPE_ID: u64 = 0x6d10_7e4c_464f_4b53;
    let activation_hash = foks_crypto::prefixed_hash(ACTIVATION_TYPE_ID, argument);
    let activated = writer
        .call_with_current_time(Arc::clone(clock), move |database, now| {
            Ok(database.activate_stateless_team_view_challenge(
                &challenge_hash,
                &activation_hash,
                &token_hash,
                &authority,
                &key_generation,
                expires_at,
                now,
            )?)
        })
        .map_err(map_write_error)?
        .ok_or_else(|| {
            RpcStatus::TeamBearerTokenStale("team-view capability was superseded".to_owned())
        })?;
    if activated.team_id != challenge.request.team.as_bytes()
        || activated.member_id != challenge.request.member.as_bytes()
        || activated.member_host_id != challenge.request.member_host.as_bytes()
        || activated.source_role_type != role
        || activated.source_visibility != visibility
        || activated.source_generation != challenge.request.generation
    {
        return Err(permission_denied());
    }
    ActivatedTeamView {
        token: challenge.token,
        team: challenge.request.team.clone(),
    }
    .encoded()
    .map_err(|_| RpcStatus::TransactionRetry)
}

/// Introspects an already-activated team view bearer token and returns its
/// team id. This is the re-establish call a client makes after a dropped
/// connection, so it never re-runs the challenge/activation handshake and only
/// accepts a token that is still active on this host.
pub(crate) fn check_team_view_token(
    argument: &[u8],
    principal: Option<&Principal>,
    host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
    clock: &Arc<dyn foks_server_db::Clock>,
) -> Result<Vec<u8>, RpcStatus> {
    if let Some(p) = principal {
        p.require_ordinary_device()?;
    }
    let request = foks_rpc::arguments::decode_check_team_view(argument).map_err(bad_arguments)?;
    if request.host != *host {
        return Err(permission_denied());
    }
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let authority = reader
        .resolve_team_view_token(&team::token_hash(&request.token), now)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(|| {
            RpcStatus::TeamBearerTokenStale("team-view bearer token is not active".to_owned())
        })?;
    require_view_transport(
        principal,
        &EntityId::from_bytes(authority.member_host_id.clone())
            .map_err(|_| RpcStatus::TransactionRetry)?,
        host,
        reader,
    )?;
    foks_snowpack::encode(&foks_snowpack::Value::Binary(authority.team_id))
        .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn load_chain(
    argument: &[u8],
    principal: Option<&Principal>,
    host: &EntityId,
    reader: &foks_server_db::ReadSnapshot<'_>,
    clock: &dyn foks_server_db::Clock,
) -> Result<Vec<u8>, RpcStatus> {
    let request = foks_rpc::arguments::decode_load_team_chain(argument).map_err(bad_arguments)?;
    if request.host != *host {
        return Err(permission_denied());
    }
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let authority = match &request.authorization {
        foks_rpc::arguments::TeamChainAuthorization::LocalView(token) => {
            let authority = reader
                .resolve_team_view_token(&team::token_hash(token), now)
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or(RpcStatus::Expired)?;
            if authority.team_id != request.team.as_bytes() {
                return Err(permission_denied());
            }
            if authority.member_host_id == host.as_bytes() {
                let p = principal.ok_or_else(permission_denied)?;
                p.require_ordinary_device()?;
                if reader
                    .active_credential_owner(p.uid(), p.device_id())
                    .map_err(|_| RpcStatus::TransactionRetry)?
                    .is_none()
                {
                    return Err(permission_denied());
                }
            }
            Some(authority)
        }
        foks_rpc::arguments::TeamChainAuthorization::RemotePermission(token) => {
            if request.load_removal_key || request.load_remote_view_tokens {
                return Err(permission_denied());
            }
            let hash = crate::services::federation::permission_token_hash(token.expose());
            if !reader
                .remote_team_view_token_is_current(&hash, request.team.as_bytes(), now)
                .map_err(|_| RpcStatus::TransactionRetry)?
            {
                return Err(permission_denied());
            }
            None
        }
        foks_rpc::arguments::TeamChainAuthorization::LocalParentTeam(token) => {
            let principal = principal.ok_or_else(permission_denied)?;
            principal.require_ordinary_device()?;
            if request.load_removal_key || request.load_remote_view_tokens {
                return Err(permission_denied());
            }
            let authority = reader
                .resolve_team_view_token(&team::token_hash(token), now)
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or(RpcStatus::Expired)?;
            let parent = reader
                .team(&authority.team_id)
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or_else(permission_denied)?;
            let minimum = reader
                .team_local_view_permission(&authority.team_id, request.team.as_bytes())
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or_else(permission_denied)?;
            if authority.member_host_id != host.as_bytes()
                || parent.host_id != host.as_bytes()
                || !super::user::role_can_load_members(&authority, minimum.0, minimum.1)
                || reader
                    .active_credential_owner(principal.uid(), principal.device_id())
                    .map_err(|_| RpcStatus::TransactionRetry)?
                    .is_none()
            {
                return Err(permission_denied());
            }
            let target = reader
                .team(request.team.as_bytes())
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or(RpcStatus::TeamNotFound)?;
            // An explicit scoped grant also authorizes pre-admission inspection.
            // Requiring a roster row here would make team invitations circular.
            if target.host_id != host.as_bytes() {
                return Err(permission_denied());
            }
            None
        }
    };
    encode_team_chain(reader, host, &request, authority.as_ref())
}

fn encode_team_chain(
    database: &foks_server_db::ReadSnapshot<'_>,
    host: &EntityId,
    request: &foks_rpc::arguments::LoadTeamChainArgument,
    authority: Option<&foks_server_db::TeamViewAuthoritySnapshot>,
) -> Result<Vec<u8>, RpcStatus> {
    let team_state = database
        .team(request.team.as_bytes())
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::TeamNotFound)?;
    let maximum_start = u64::try_from(team_state.links.len())
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(RpcStatus::TransactionRetry)?;
    let full = request.start == 1 && request.name_cursor.is_none();
    if request.start == 0 || (!full && request.start < 2) || request.start > maximum_start {
        return Err(bad_arguments("team-chain start is out of range"));
    }
    if !full
        && request.name_cursor.as_ref().is_none_or(|(name, sequence)| {
            name.as_slice() != team_state.normalized_name.as_deref().unwrap_or(b"-")
                || *sequence != team_state.team_name_sequence.saturating_add(1)
        })
    {
        return Err(bad_arguments("team-name cursor is stale"));
    }
    let root = database
        .current_root()
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::StaleRoot)?;
    foks_proto::MerkleRoot::decode(&root.exact_root).map_err(|_| RpcStatus::TransactionRetry)?;
    let start_index =
        usize::try_from(request.start - 1).map_err(|_| RpcStatus::TransactionRetry)?;
    let selected = &team_state.links[start_index..];
    let links = selected
        .iter()
        .map(|link| link.exact_link.clone())
        .collect::<Vec<_>>();
    let mut locations = if full {
        Vec::new()
    } else {
        vec![team_state.links[start_index - 1].next_tree_location]
    };
    locations.extend(selected.iter().map(|link| link.next_tree_location));
    let reader = database.node_reader();
    let prove = |key| {
        foks_merkle_store::proof(&reader, root.root_node, key)
            .map_err(|_| RpcStatus::TransactionRetry)
    };
    let name = team_state.normalized_name.as_deref().unwrap_or(b"-");
    let mut paths = if team_state.kind == foks_proto::ENTITY_AD_HOC_TEAM {
        vec![
            prove(
                foks_merkle_store::username_key(name, host, 1)
                    .map_err(|_| RpcStatus::TransactionRetry)?,
            )?,
            prove(
                foks_merkle_store::username_key(name, host, 2)
                    .map_err(|_| RpcStatus::TransactionRetry)?,
            )?,
        ]
    } else if full {
        vec![
            prove(
                foks_merkle_store::username_key(name, host, 1)
                    .map_err(|_| RpcStatus::TransactionRetry)?,
            )?,
            prove(
                foks_merkle_store::username_key(name, host, team_state.team_name_sequence + 1)
                    .map_err(|_| RpcStatus::TransactionRetry)?,
            )?,
        ]
    } else {
        vec![prove(
            foks_merkle_store::username_key(name, host, team_state.team_name_sequence + 1)
                .map_err(|_| RpcStatus::TransactionRetry)?,
        )?]
    };
    for index in start_index..team_state.links.len() {
        let sequence = u64::try_from(index + 1).map_err(|_| RpcStatus::TransactionRetry)?;
        let prior = index
            .checked_sub(1)
            .map(|prior| &team_state.links[prior].next_tree_location);
        paths.push(prove(
            foks_merkle_store::chain_key(3, &request.team, sequence, prior)
                .map_err(|_| RpcStatus::TransactionRetry)?,
        )?);
    }
    paths.push(prove(
        foks_merkle_store::chain_key(
            3,
            &request.team,
            maximum_start,
            team_state.links.last().map(|link| &link.next_tree_location),
        )
        .map_err(|_| RpcStatus::TransactionRetry)?,
    )?);
    let names = if full && team_state.kind == foks_proto::ENTITY_NAMED_TEAM {
        vec![foks_proto::NameCommitmentAndKey {
            name: name.to_vec(),
            sequence: team_state.team_name_sequence,
            commitment_key: team_state
                .team_name_commitment_key
                .ok_or(RpcStatus::TransactionRetry)?,
        }]
    } else {
        Vec::new()
    };
    let parcels = match authority {
        Some(authority) => {
            let effective = team::stored_role(
                authority.effective_role_type,
                authority.effective_visibility,
            )
            .ok_or(RpcStatus::TransactionRetry)?;
            database
                .scoped_team_parcels(
                    request.team.as_bytes(),
                    &authority.member_id,
                    &authority.member_host_id,
                    authority.source_role_type,
                    authority.source_visibility,
                )
                .map_err(|_| RpcStatus::TransactionRetry)?
                .into_iter()
                .map(|exact| {
                    let parcel = foks_proto::PukParcel::decode(&exact)
                        .map_err(|_| RpcStatus::TransactionRetry)?;
                    let already_have = request.have_ptk_generations.iter().any(|known| {
                        known.role == parcel.role && known.generation >= parcel.generation
                    });
                    Ok((parcel.role <= effective && !already_have).then_some(exact))
                })
                .collect::<Result<Vec<_>, RpcStatus>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        }
        None => Vec::new(),
    };
    let removal_key = match (authority, request.load_removal_key) {
        (Some(authority), true) => Some(
            database
                .team_removal_box(
                    request.team.as_bytes(),
                    &authority.member_id,
                    &authority.member_host_id,
                    authority.source_role_type,
                    authority.source_visibility,
                )
                .map_err(|_| RpcStatus::TransactionRetry)?
                .ok_or_else(|| RpcStatus::TeamRemovalKey("removal key not found".to_owned()))?,
        ),
        _ => None,
    };
    let remote_view_tokens = if request.load_remote_view_tokens
        && authority.is_some_and(|authority| {
            team::stored_role(
                authority.effective_role_type,
                authority.effective_visibility,
            )
            .is_some_and(|role| role >= foks_proto::Role::member(0))
        }) {
        database
            .remote_member_view_tokens(request.team.as_bytes())
            .map_err(|_| RpcStatus::TransactionRetry)?
            .into_iter()
            .map(|stored| {
                let party = foks_proto::EntityId::from_bytes(stored.member_party_id)
                    .map_err(|_| RpcStatus::TransactionRetry)?;
                let host = foks_proto::EntityId::from_bytes(stored.member_host_id)
                    .map_err(|_| RpcStatus::TransactionRetry)?;
                Ok(foks_proto::TeamRemoteMemberViewTokenInner {
                    member: foks_proto::FqParty::new(party, host)
                        .map_err(|_| RpcStatus::TransactionRetry)?,
                    ptk_generation: stored.ptk_generation,
                    secret_box: foks_proto::SecretBox::decode(&stored.exact_secret_box)
                        .map_err(|_| RpcStatus::TransactionRetry)?,
                    ptk_role: team::stored_role(stored.ptk_role_type, stored.ptk_visibility)
                        .ok_or(RpcStatus::TransactionRetry)?,
                })
            })
            .collect::<Result<Vec<_>, RpcStatus>>()?
    } else {
        Vec::new()
    };
    let mut hepks = team_state
        .shared_keys
        .iter()
        .map(|key| key.exact_hepk.clone())
        .collect::<Vec<_>>();
    hepks.extend(
        database
            .team_member_hepks(request.team.as_bytes())
            .map_err(|_| RpcStatus::TransactionRetry)?,
    );
    hepks.sort_unstable();
    hepks.dedup();
    foks_proto::TeamChainResponse {
        exact_links: &links,
        locations: &locations,
        team_names: &names,
        exact_root: &root.exact_root,
        paths: &paths,
        team_name_utf8: if team_state.kind == foks_proto::ENTITY_AD_HOC_TEAM {
            b"-"
        } else {
            &team_state.team_name_utf8
        },
        num_team_name_links: if team_state.kind == foks_proto::ENTITY_AD_HOC_TEAM || full {
            2
        } else {
            1
        },
        exact_parcels: &parcels,
        exact_removal_key: removal_key.as_deref(),
        remote_view_tokens: &remote_view_tokens,
        exact_hepks: &hepks,
    }
    .encoded()
    .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn load_remote_view_tokens(
    argument: &[u8],
    principal: &Principal,
    host: &EntityId,
    reader: &foks_server_db::ReadSnapshot<'_>,
    clock: &dyn foks_server_db::Clock,
) -> Result<Vec<u8>, RpcStatus> {
    principal.require_ordinary_device()?;
    let request = foks_rpc::arguments::decode_load_team_remote_view_tokens(argument)
        .map_err(bad_arguments)?;
    if request.team.host != *host {
        return Err(permission_denied());
    }
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let authority = reader
        .resolve_team_view_token(&team::token_hash(&request.token), now)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::Expired)?;
    if authority.team_id != request.team.team.as_bytes()
        || authority.member_host_id != host.as_bytes()
        || reader
            .active_credential_owner(principal.uid(), principal.device_id())
            .map_err(|_| RpcStatus::TransactionRetry)?
            .is_none()
        || team::stored_role(
            authority.effective_role_type,
            authority.effective_visibility,
        )
        .is_none_or(|role| role < foks_proto::Role::member(0))
    {
        return Err(permission_denied());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut tokens = Vec::with_capacity(request.members.len());
    for member in request.members {
        if member.host == *host
            || !seen.insert((
                member.party.as_bytes().to_vec(),
                member.host.as_bytes().to_vec(),
            ))
        {
            return Err(bad_arguments(
                "remote member list contains a local or duplicate party",
            ));
        }
        let Some(stored) = reader
            .remote_member_view_token(
                request.team.team.as_bytes(),
                member.party.as_bytes(),
                member.host.as_bytes(),
            )
            .map_err(|_| RpcStatus::TransactionRetry)?
        else {
            continue;
        };
        let ptk_role = team::stored_role(stored.ptk_role_type, stored.ptk_visibility)
            .ok_or(RpcStatus::TransactionRetry)?;
        tokens.push(foks_proto::TeamRemoteMemberViewTokenInner {
            member,
            ptk_generation: stored.ptk_generation,
            secret_box: foks_proto::SecretBox::decode(&stored.exact_secret_box)
                .map_err(|_| RpcStatus::TransactionRetry)?,
            ptk_role,
        });
    }
    foks_proto::TeamRemoteViewTokenSet { tokens }
        .encoded()
        .map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn load_removal_for_member(
    argument: &[u8],
    principal: Option<&Principal>,
    host: &EntityId,
    reader: &foks_server_db::ReadSnapshot<'_>,
) -> Result<Vec<u8>, RpcStatus> {
    if let Some(principal) = principal {
        principal.require_ordinary_device()?;
    }
    let request =
        foks_rpc::arguments::decode_load_removal_for_member(argument).map_err(bad_arguments)?;
    if request.team.host != *host {
        return Err(permission_denied());
    }
    let stored = reader
        .team_removal(request.team.team.as_bytes(), &request.commitment)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or_else(|| RpcStatus::NotFound("team removal key not found".to_owned()))?;
    let boxed = foks_proto::TeamRemovalBoxData::decode(&stored.exact_box)
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let response = foks_proto::TeamRemovalAndKeyBox {
        key_box: boxed.member_box,
        removal: foks_proto::TeamRemovalProof::decode(&stored.exact_removal)
            .map_err(|_| RpcStatus::TransactionRetry)?,
    };
    if response.removal.payload.team != request.team.team
        || response.removal.payload.host != *host
        || boxed.commitment != request.commitment
        || boxed.metadata.member != response.removal.payload.member
        || boxed.metadata.member_host != response.removal.payload.member_host
        || boxed.metadata.source_role != response.removal.payload.source_role
    {
        return Err(RpcStatus::TransactionRetry);
    }
    response.encoded().map_err(|_| RpcStatus::TransactionRetry)
}

pub(crate) fn load_team_membership_chain(
    argument: &[u8],
    principal: Option<&Principal>,
    host: &EntityId,
    reader: &foks_server_db::ReadSnapshot<'_>,
    clock: &dyn foks_server_db::Clock,
) -> Result<Vec<u8>, RpcStatus> {
    let request =
        foks_rpc::arguments::decode_load_team_membership_chain(argument).map_err(bad_arguments)?;
    if request.team.host != *host {
        return Err(permission_denied());
    }
    let now = clock
        .now_micros()
        .map_err(|_| RpcStatus::TransactionRetry)?;
    let authority = reader
        .resolve_team_view_token(&team::token_hash(&request.token), now)
        .map_err(|_| RpcStatus::TransactionRetry)?
        .ok_or(RpcStatus::Expired)?;
    if authority.team_id != request.team.team.as_bytes() {
        return Err(permission_denied());
    }
    if authority.member_host_id == host.as_bytes() {
        let principal = principal.ok_or_else(permission_denied)?;
        principal.require_ordinary_device()?;
        if reader
            .active_credential_owner(principal.uid(), principal.device_id())
            .map_err(|_| RpcStatus::TransactionRetry)?
            .is_none()
        {
            return Err(permission_denied());
        }
    } else if let Some(principal) = principal {
        principal.require_ordinary_device()?;
    }
    super::generic::load_for_entity(
        reader,
        &request.team.team,
        foks_proto::CHAIN_TYPE_TEAM_MEMBERSHIP,
        request.start,
    )
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    RpcStatus::BadArguments(error.to_string())
}

fn permission_denied() -> RpcStatus {
    RpcStatus::PermissionDenied("team-view authorization failed".to_owned())
}

fn map_write_error(error: crate::Error) -> RpcStatus {
    match error {
        crate::Error::WriterQueue => RpcStatus::RateLimited,
        crate::Error::Database(foks_server_db::Error::QuotaExceeded) => RpcStatus::QuotaExceeded,
        crate::Error::Database(foks_server_db::Error::OperationConflict) => {
            bad_arguments("conflicting team-view activation replay")
        }
        _ => RpcStatus::TransactionRetry,
    }
}

fn require_view_transport(
    principal: Option<&Principal>,
    member_host: &EntityId,
    host: &EntityId,
    reader: &foks_server_db::ReadDatabase,
) -> Result<(), RpcStatus> {
    if let Some(p) = principal {
        p.require_ordinary_device()?;
        if reader
            .active_credential_owner(p.uid(), p.device_id())
            .map_err(|_| RpcStatus::TransactionRetry)?
            .is_none()
        {
            return Err(permission_denied());
        }
    } else if member_host == host {
        return Err(permission_denied());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_quota_is_not_reported_as_transient_rate_limiting() {
        assert_eq!(
            map_write_error(crate::Error::Database(foks_server_db::Error::QuotaExceeded)),
            RpcStatus::QuotaExceeded
        );
        assert_eq!(
            map_write_error(crate::Error::WriterQueue),
            RpcStatus::RateLimited
        );
    }
}

// Public registration connections authenticate a foreign member by its scoped
// roster key. Local accounts retain mTLS/SSO enforcement on the user listener.
