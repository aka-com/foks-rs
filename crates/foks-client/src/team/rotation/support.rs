//! Pure validation, recipient resolution, and durable operation bindings.

use foks_client_db::{TeamMutationKind, TeamMutationOperation};
use foks_crypto::{
    derive_shared_public, prefixed_hash, seal_shared_key_boxes, PukBoxRandomness, SharedKeyBoxInput,
};
use foks_proto::{EntityId, Role, ENTITY_PTK_VERIFY, ENTITY_PUK_VERIFY};
use foks_snowpack::{encode, Value};
use foks_verify::{
    VerifiedSharedKey, VerifiedTeamMemberState, VerifiedTeamState, VerifiedUserState,
};

use super::super::membership::{current_team_private_key, random_box_randomness};
use super::super::AuthenticatedTeamOutcome;
use super::{
    Receiver, RotationBinding, TeamMemberKeyRefresh, TeamMemberSelector, TeamPtkRotationSeed,
    ValidatedRotation, VerifiedMemberParty,
};
use crate::{random_bytes, Error, PinnedHost, Result, TEAM_MUTATION_OPERATION_ID_TYPE_ID};

pub(super) fn unique_target<'a>(
    team: &'a VerifiedTeamState,
    selector: TeamMemberSelector<'_>,
) -> Result<&'a VerifiedTeamMemberState> {
    let mut matches = team.members().iter().filter(|member| {
        member.party == *selector.party
            && member.scoped_host.as_ref() == selector.host
            && member.source_role == selector.source_role
    });
    let member = matches
        .next()
        .ok_or(Error::TeamRequest("target is not a current team member"))?;
    if matches.next().is_some() || member.removal_key_commitment.is_none() {
        return Err(Error::TeamBinding(
            "target membership is ambiguous or lacks a removal commitment",
        ));
    }
    Ok(member)
}

pub(super) fn validate_replacement<'a>(
    target: &VerifiedTeamMemberState,
    destination_role: Role,
    replacement: Option<VerifiedMemberParty<'a>>,
) -> Result<Option<(VerifiedMemberParty<'a>, &'a VerifiedSharedKey)>> {
    if destination_role == Role::NONE {
        if replacement.is_some() {
            return Err(Error::TeamRequest(
                "removal cannot include replacement keys",
            ));
        }
        return Ok(None);
    }
    if destination_role > target.role {
        return Err(Error::TeamRequest(
            "this transition cannot promote a team member",
        ));
    }
    let party = replacement.ok_or(Error::TeamRequest(
        "non-removal transition requires verified replacement keys",
    ))?;
    let expected_host = target.scoped_host.as_ref();
    if party.party() != &target.party || expected_host.is_some_and(|host| party.host() != host) {
        return Err(Error::TeamBinding(
            "replacement state does not identify the target party",
        ));
    }
    let key = party
        .shared_key(target.source_role)
        .ok_or(Error::KeyBinding(
            "replacement party lacks the roster's source-role key",
        ))?;
    if key.generation < target.generation
        || (key.generation == target.generation
            && (key.verify_key != target.verify_key
                || foks_crypto::hepk_fingerprint(&key.hepk)? != target.hepk_fingerprint))
    {
        return Err(Error::TeamBinding(
            "replacement key generation regresses or conflicts with the roster",
        ));
    }
    if destination_role == target.role && key.generation == target.generation {
        return Err(Error::TeamRequest("team-member transition makes no change"));
    }
    Ok(Some((party, key)))
}

pub(super) fn validate_rotation_seeds<'a>(
    team: &'a AuthenticatedTeamOutcome,
    target: &VerifiedTeamMemberState,
    destination_role: Role,
    replacement_generation: Option<u64>,
    supplied: &'a [TeamPtkRotationSeed<'a>],
) -> Result<Vec<ValidatedRotation<'a>>> {
    let current = team.verified.shared_keys();
    let expected_roles = required_rotation_roles(
        current.iter().map(|key| key.role),
        target.role,
        target.generation,
        destination_role,
        replacement_generation,
    );
    let expected = expected_roles
        .iter()
        .map(|role| {
            current
                .iter()
                .find(|key| key.role == *role)
                .expect("required roles come from current keys")
        })
        .collect::<Vec<_>>();
    if supplied.len() != expected.len() {
        return Err(Error::TeamRequest(
            "PTK rotation roles do not match the removal schedule",
        ));
    }
    let current_verify_keys = team
        .verified
        .shared_keys()
        .iter()
        .map(|key| key.verify_key.as_bytes().to_vec())
        .collect::<std::collections::BTreeSet<_>>();
    let mut new_verify_keys = std::collections::BTreeSet::new();
    let mut rotations = Vec::with_capacity(expected.len());
    for (public, supplied) in expected.into_iter().zip(supplied) {
        if supplied.role != public.role {
            return Err(Error::TeamRequest(
                "PTK rotation roles are missing, duplicated, or out of order",
            ));
        }
        let previous = current_team_private_key(team, public)?;
        let verify_key = derive_shared_public(supplied.seed, ENTITY_PTK_VERIFY)?.verify_key;
        if supplied.seed == &previous.seed
            || current_verify_keys.contains(verify_key.as_bytes())
            || !new_verify_keys.insert(verify_key.as_bytes().to_vec())
        {
            return Err(Error::KeyBinding(
                "replacement PTKs must be fresh and distinct",
            ));
        }
        let generation = public
            .generation
            .checked_add(1)
            .ok_or(Error::TeamRequest("PTK generation overflow"))?;
        rotations.push(ValidatedRotation {
            role: public.role,
            generation,
            seed: supplied.seed,
            previous,
            verify_key,
        });
    }
    Ok(rotations)
}

pub(super) fn validate_refresh_rotation_seeds<'a>(
    team: &'a AuthenticatedTeamOutcome,
    changes: &[TeamMemberKeyRefresh<'_>],
    supplied: &'a [TeamPtkRotationSeed<'a>],
) -> Result<Vec<ValidatedRotation<'a>>> {
    let mut expected_roles = std::collections::BTreeSet::new();
    for change in changes {
        let target = unique_target(&team.verified, change.target)?;
        let (_, replacement) =
            validate_replacement(target, change.destination_role, change.replacement)?.ok_or(
                Error::TeamRequest("member-key refresh lacks replacement keys"),
            )?;
        if change.destination_role != target.role
            || replacement.generation <= target.generation
            || replacement.generation != change.replacement_generation
            || replacement.verify_key != *change.replacement_verify_key
            || foks_crypto::hepk_fingerprint(&replacement.hepk)?
                != change.replacement_hepk_fingerprint
        {
            return Err(Error::TeamRequest(
                "member-key refresh must advance the roster generation",
            ));
        }
        expected_roles.extend(
            team.verified
                .shared_keys()
                .iter()
                .filter(|key| key.role <= target.role)
                .map(|key| key.role),
        );
    }
    let expected = expected_roles
        .iter()
        .map(|role| {
            team.verified
                .shared_keys()
                .iter()
                .find(|key| key.role == *role)
                .expect("required refresh role comes from current keys")
        })
        .collect::<Vec<_>>();
    if supplied.len() != expected.len() {
        return Err(Error::TeamRequest(
            "PTK rotation roles do not match the member-refresh schedule",
        ));
    }
    let current_verify_keys = team
        .verified
        .shared_keys()
        .iter()
        .map(|key| key.verify_key.as_bytes().to_vec())
        .collect::<std::collections::BTreeSet<_>>();
    let mut new_verify_keys = std::collections::BTreeSet::new();
    let mut rotations = Vec::with_capacity(expected.len());
    for (public, supplied) in expected.into_iter().zip(supplied) {
        if supplied.role != public.role {
            return Err(Error::TeamRequest(
                "PTK rotation roles are missing, duplicated, or out of order",
            ));
        }
        let previous = current_team_private_key(team, public)?;
        let verify_key = derive_shared_public(supplied.seed, ENTITY_PTK_VERIFY)?.verify_key;
        if supplied.seed == &previous.seed
            || current_verify_keys.contains(verify_key.as_bytes())
            || !new_verify_keys.insert(verify_key.as_bytes().to_vec())
        {
            return Err(Error::KeyBinding(
                "replacement PTKs must be fresh and distinct",
            ));
        }
        rotations.push(ValidatedRotation {
            role: public.role,
            generation: public
                .generation
                .checked_add(1)
                .ok_or(Error::TeamRequest("PTK generation overflow"))?,
            seed: supplied.seed,
            previous,
            verify_key,
        });
    }
    Ok(rotations)
}

pub(super) fn required_rotation_roles(
    current: impl IntoIterator<Item = Role>,
    old_role: Role,
    old_generation: u64,
    destination_role: Role,
    replacement_generation: Option<u64>,
) -> Vec<Role> {
    let source_generation_changed =
        replacement_generation.is_some_and(|generation| generation > old_generation);
    current
        .into_iter()
        .filter(|role| {
            *role <= old_role
                && (destination_role == Role::NONE
                    || source_generation_changed
                    || *role > destination_role)
        })
        .collect()
}

pub(super) fn resolve_remaining_receivers<'a>(
    team: &'a VerifiedTeamState,
    changed: &VerifiedTeamMemberState,
    destination_role: Role,
    replacement: Option<VerifiedMemberParty<'a>>,
    actor: &'a VerifiedUserState,
    supplied: &'a [VerifiedMemberParty<'a>],
    rotated_roles: &[Role],
) -> Result<Vec<Receiver<'a>>> {
    let actor = VerifiedMemberParty::User(actor);
    let mut parties = Vec::with_capacity(supplied.len() + 2);
    let actor_is_changed = changed.party == *actor.party() && changed.scoped_host.is_none();
    if !actor_is_changed {
        parties.push(actor);
    }
    parties.extend_from_slice(supplied);
    if let Some(replacement) = replacement {
        parties.push(replacement);
    }
    let mut remaining = team
        .members()
        .iter()
        .filter(|member| {
            member.party != changed.party
                || member.scoped_host != changed.scoped_host
                || member.source_role != changed.source_role
        })
        .cloned()
        .collect::<Vec<_>>();
    if destination_role != Role::NONE {
        let (_, replacement_key) = validate_replacement(changed, destination_role, replacement)?
            .ok_or(Error::TeamRequest("replacement keys are missing"))?;
        let mut replacement_member = changed.clone();
        replacement_member.role = destination_role;
        replacement_member.generation = replacement_key.generation;
        replacement_member.verify_key = replacement_key.verify_key.clone();
        replacement_member.hepk_fingerprint = foks_crypto::hepk_fingerprint(&replacement_key.hepk)?;
        remaining.push(replacement_member);
    }
    if remaining.len() != parties.len() {
        return Err(Error::TeamRequest(
            "verified party states do not exactly cover the post-transition roster",
        ));
    }
    let mut receivers = Vec::with_capacity(remaining.len());
    for member in &remaining {
        let matching_parties = parties
            .iter()
            .filter(|party| {
                party.party() == &member.party
                    && (member
                        .scoped_host
                        .as_ref()
                        .is_none_or(|host| party.host() == host))
                    && (member.scoped_host.is_some() || party.host() == team.host())
            })
            .copied()
            .collect::<Vec<_>>();
        let [party] = matching_parties.as_slice() else {
            return Err(Error::TeamRequest(
                "remaining party state is missing or duplicated",
            ));
        };
        if rotated_roles.iter().any(|role| *role <= member.role)
            && party.has_stale_shared_key(member.source_role)
        {
            return Err(Error::TeamBinding(
                "team PTK recipient has an unrotated revoked-device PUK",
            ));
        }
        let key = party
            .shared_key(member.source_role)
            .filter(|key| {
                key.generation == member.generation
                    && key.verify_key == member.verify_key
                    && foks_crypto::hepk_fingerprint(&key.hepk).ok()
                        == Some(member.hepk_fingerprint)
            })
            .ok_or(Error::TeamBinding(
                "remaining party roster key is not current",
            ))?;
        receivers.push(Receiver {
            member: member.clone(),
            key,
        });
    }
    receivers.sort_unstable_by(|left, right| {
        left.member
            .party
            .as_bytes()
            .cmp(right.member.party.as_bytes())
            .then(
                left.member
                    .scoped_host
                    .as_ref()
                    .map(EntityId::as_bytes)
                    .cmp(&right.member.scoped_host.as_ref().map(EntityId::as_bytes)),
            )
            .then(left.member.source_role.cmp(&right.member.source_role))
    });
    Ok(receivers)
}

pub(super) fn resolve_refresh_receivers<'a>(
    team: &VerifiedTeamState,
    changes: &[TeamMemberKeyRefresh<'a>],
    actor: VerifiedMemberParty<'a>,
    supplied: &'a [VerifiedMemberParty<'a>],
    rotated_roles: &[Role],
) -> Result<Vec<Receiver<'a>>> {
    let actor_party = actor;
    let changed_keys = changes
        .iter()
        .map(|change| {
            (
                change.target.party.as_bytes().to_vec(),
                change
                    .target
                    .host
                    .map(EntityId::as_bytes)
                    .map(<[u8]>::to_vec),
                change.target.source_role,
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    if changed_keys.len() != changes.len() {
        return Err(Error::TeamRequest(
            "member-key refresh targets are duplicated",
        ));
    }
    let actor_is_changed = changes
        .iter()
        .any(|change| change.target.party == actor_party.party() && change.target.host.is_none());
    let mut parties = Vec::with_capacity(supplied.len() + changes.len() + 1);
    if !actor_is_changed {
        parties.push(actor_party);
    }
    parties.extend_from_slice(supplied);
    parties.extend(changes.iter().filter_map(|change| change.replacement));
    let mut unique_parties = Vec::with_capacity(parties.len());
    for party in parties {
        if !unique_parties
            .iter()
            .any(|known: &VerifiedMemberParty<'_>| {
                known.party() == party.party() && known.host() == party.host()
            })
        {
            unique_parties.push(party);
        }
    }
    let parties = unique_parties;

    let mut remaining = team
        .members()
        .iter()
        .filter(|member| {
            !changed_keys.contains(&(
                member.party.as_bytes().to_vec(),
                member
                    .scoped_host
                    .as_ref()
                    .map(EntityId::as_bytes)
                    .map(<[u8]>::to_vec),
                member.source_role,
            ))
        })
        .cloned()
        .collect::<Vec<_>>();
    for change in changes {
        let target = unique_target(team, change.target)?;
        if change.destination_role != target.role {
            return Err(Error::TeamRequest(
                "member-key refresh cannot change the member role",
            ));
        }
        let replacement_key = change
            .replacement
            .ok_or(Error::TeamRequest(
                "member-key refresh lacks replacement state",
            ))?
            .key_at(
                target.source_role,
                change.replacement_generation,
                change.replacement_verify_key,
                change.replacement_hepk_fingerprint,
            )
            .ok_or(Error::TeamRequest(
                "member-key refresh lacks replacement keys",
            ))?;
        let mut replacement_member = target.clone();
        replacement_member.generation = replacement_key.generation;
        replacement_member.verify_key = replacement_key.verify_key.clone();
        replacement_member.hepk_fingerprint = foks_crypto::hepk_fingerprint(&replacement_key.hepk)?;
        remaining.push(replacement_member);
    }
    let mut receivers = Vec::with_capacity(remaining.len());
    for member in &remaining {
        let matching = parties
            .iter()
            .filter(|party| {
                party.party() == &member.party
                    && member
                        .scoped_host
                        .as_ref()
                        .is_none_or(|host| party.host() == host)
                    && (member.scoped_host.is_some() || party.host() == team.host())
            })
            .copied()
            .collect::<Vec<_>>();
        let [party] = matching.as_slice() else {
            return Err(Error::TeamRequest(
                "refreshed party state is missing or duplicated",
            ));
        };
        if rotated_roles.iter().any(|role| *role <= member.role)
            && party.has_stale_shared_key(member.source_role)
        {
            return Err(Error::TeamBinding(
                "team PTK recipient has an unrotated revoked-device PUK",
            ));
        }
        let is_changed = changed_keys.contains(&(
            member.party.as_bytes().to_vec(),
            member
                .scoped_host
                .as_ref()
                .map(EntityId::as_bytes)
                .map(<[u8]>::to_vec),
            member.source_role,
        ));
        let key = party
            .shared_key(member.source_role)
            .filter(|key| {
                key.generation == member.generation
                    && key.verify_key == member.verify_key
                    && foks_crypto::hepk_fingerprint(&key.hepk).ok()
                        == Some(member.hepk_fingerprint)
            })
            .ok_or(Error::TeamBinding(if is_changed {
                "refreshed party lacks its replacement roster key"
            } else {
                "unchanged roster recipient key is not current"
            }))?;
        receivers.push(Receiver {
            member: member.clone(),
            key,
        });
    }
    receivers.sort_by(|left, right| {
        left.member
            .party
            .as_bytes()
            .cmp(right.member.party.as_bytes())
            .then(
                left.member
                    .scoped_host
                    .as_ref()
                    .map(EntityId::as_bytes)
                    .cmp(&right.member.scoped_host.as_ref().map(EntityId::as_bytes)),
            )
            .then(left.member.source_role.cmp(&right.member.source_role))
    });
    Ok(receivers)
}

pub(super) fn box_rotated_ptks(
    host: &PinnedHost,
    actor: &EntityId,
    actor_seed: &foks_proto::SecretSeed,
    rotations: &[ValidatedRotation<'_>],
    receivers: &[Receiver<'_>],
) -> Result<foks_proto::SharedKeyBoxSet> {
    let mut inputs = Vec::new();
    for rotation in rotations {
        for receiver in receivers
            .iter()
            .filter(|receiver| rotation.role <= receiver.member.role)
        {
            inputs.push(SharedKeyBoxInput {
                seed: rotation.seed,
                generation: rotation.generation,
                role: rotation.role,
                receiver_id: &receiver.member.party,
                receiver_host: receiver.member.scoped_host.as_ref(),
                receiver_hepk: &receiver.key.hepk,
                receiver_role: receiver.member.source_role,
                receiver_generation: receiver.member.generation,
            });
        }
    }
    let randomness = (0..inputs.len())
        .map(|_| random_box_randomness())
        .collect::<Result<Vec<PukBoxRandomness>>>()?;
    let sender_type = match actor.entity_type() {
        foks_proto::ENTITY_USER => ENTITY_PUK_VERIFY,
        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM => ENTITY_PTK_VERIFY,
        _ => return Err(Error::TeamBinding("team editor party type is invalid")),
    };
    let sender = derive_shared_public(actor_seed, sender_type)?;
    Ok(seal_shared_key_boxes(
        host.host_id(),
        actor_seed,
        &sender.hepk,
        random_bytes()?,
        &inputs,
        &randomness,
    )?)
}

pub(super) fn rotation_binding(
    target: TeamMemberSelector<'_>,
    removal_key_commitment: [u8; 32],
    destination_role: Role,
    replacement: Option<(u64, EntityId)>,
    expected_seqno: u64,
    rotations: &[ValidatedRotation<'_>],
) -> Result<RotationBinding> {
    Ok(RotationBinding {
        target: target.party.clone(),
        target_host: target.host.cloned(),
        target_source_role: target.source_role,
        removal_key_commitment,
        destination_role,
        replacement,
        expected_seqno,
        introduced: rotations
            .iter()
            .map(|rotation| {
                (
                    rotation.role,
                    rotation.generation,
                    rotation.verify_key.clone(),
                )
            })
            .collect(),
    })
}

pub(super) fn rotation_operation_id(
    actor: &EntityId,
    team: &EntityId,
    binding: &RotationBinding,
) -> Result<[u8; 16]> {
    let identity = encode(&Value::Array(vec![
        Value::Binary(actor.as_bytes().to_vec()),
        Value::Binary(team.as_bytes().to_vec()),
        Value::Binary(binding.target.as_bytes().to_vec()),
        binding
            .target_host
            .as_ref()
            .map_or(Value::Null, |host| Value::Binary(host.as_bytes().to_vec())),
        binding.target_source_role.to_value(),
        Value::Binary(binding.removal_key_commitment.to_vec()),
        binding.destination_role.to_value(),
        binding
            .replacement
            .as_ref()
            .map_or(Value::Null, |(generation, verify)| {
                Value::Array(vec![
                    Value::Unsigned(*generation),
                    Value::Binary(verify.as_bytes().to_vec()),
                ])
            }),
        Value::Unsigned(binding.expected_seqno),
        Value::Array(
            binding
                .introduced
                .iter()
                .map(|(role, generation, verify)| {
                    Value::Array(vec![
                        role.to_value(),
                        Value::Unsigned(*generation),
                        Value::Binary(verify.as_bytes().to_vec()),
                    ])
                })
                .collect(),
        ),
    ]))?;
    let hash = prefixed_hash(TEAM_MUTATION_OPERATION_ID_TYPE_ID, &identity);
    Ok(hash[..16].try_into().expect("hash prefix has fixed length"))
}

pub(super) fn validate_rotation_transition(
    team: &VerifiedTeamState,
    binding: &RotationBinding,
) -> Result<()> {
    let change = team.group_change_at(binding.expected_seqno)?;
    validate_rotation_change(&change, binding)
}

pub(super) fn validate_rotation_change(
    change: &foks_proto::TeamGroupChange,
    binding: &RotationBinding,
) -> Result<()> {
    let [member] = change.changes.as_slice() else {
        return Err(Error::OperationBinding(
            "prepared PTK rotation sequence contains another roster transition",
        ));
    };
    if member.party != binding.target
        || member.source_role != binding.target_source_role
        || member.scoped_host != binding.target_host
        || member.role != binding.destination_role
        || match (&member.keys, &binding.replacement) {
            (None, None) => false,
            (Some(keys), Some((generation, verify))) => {
                keys.generation != *generation || keys.verify_key != *verify
            }
            _ => true,
        }
        || change.shared_keys.len() != binding.introduced.len()
        || change.shared_keys.iter().zip(&binding.introduced).any(
            |(key, (role, generation, verify))| {
                key.role != *role || key.generation != *generation || key.verify_key != *verify
            },
        )
    {
        return Err(Error::OperationBinding(
            "observed team transition does not match the prepared PTK rotation",
        ));
    }
    Ok(())
}

pub(super) fn validate_rotation_operation(
    operation: &TeamMutationOperation,
    host: &PinnedHost,
    actor: &EntityId,
    team: &EntityId,
    expected_seqno: u64,
) -> Result<()> {
    if operation.kind != TeamMutationKind::PtkRotation
        || operation.host_id != host.host_id().as_bytes()
        || operation.actor_id != actor.as_bytes()
        || operation.team_id != team.as_bytes()
        || operation.expected_seqno != expected_seqno
    {
        return Err(Error::OperationBinding(
            "PTK rotation journal does not match supplied identities",
        ));
    }
    Ok(())
}
