use std::collections::{BTreeMap, BTreeSet};

use foks_proto::{EntityId, Hepk, PukParcel, Role, SeedChainBox, TeamRemoteMemberViewToken};

use crate::{Error, Result};

pub(crate) struct Parcel {
    pub party_id: Vec<u8>,
    pub sender_id: Vec<u8>,
    pub target_role: Role,
    pub role: Role,
    pub generation: u64,
    pub exact: Vec<u8>,
}

pub(crate) struct RemovalBox {
    pub member_id: Vec<u8>,
    pub member_host_id: Vec<u8>,
    pub source_role: Role,
    pub exact: Vec<u8>,
}

pub(crate) struct RemovalProof {
    pub commitment: [u8; 32],
    pub member_id: Vec<u8>,
    pub member_host_id: Vec<u8>,
    pub source_role: Role,
    pub exact: Vec<u8>,
}

pub(crate) struct Command {
    pub team: EntityId,
    pub sequence: u64,
    pub expected_tail_hash: [u8; 32],
    pub link_hash: [u8; 32],
    pub exact_link: Vec<u8>,
    pub next_tree_location: [u8; 32],
    pub members: Vec<foks_verify::VerifiedTeamMemberState>,
    pub introduced_keys: Vec<foks_verify::VerifiedSharedKey>,
    pub parcels: Vec<Parcel>,
    pub seed_chain: Vec<SeedChainBox>,
    pub removal_boxes: Vec<RemovalBox>,
    pub removal_proofs: Vec<RemovalProof>,
    pub remote_member_view_tokens: Vec<TeamRemoteMemberViewToken>,
    pub local_view_permissions: Vec<(Vec<u8>, Role)>,
}

pub(crate) fn validate(
    argument: foks_proto::DecodedTeamEditArgument,
    team: &foks_server_db::TeamSnapshot,
    root: &foks_server_db::RootSnapshot,
) -> Result<Command> {
    if team.kind != foks_proto::ENTITY_NAMED_TEAM {
        return Err(Error::Signup("ad-hoc teams are immutable"));
    }
    let team_id = EntityId::from_bytes(team.team_id.clone())?;
    let host = EntityId::from_bytes(team.host_id.clone())?;
    let change = argument.link.decode_team_group_change()?;
    let current_members = team
        .members
        .iter()
        .map(stored_member)
        .collect::<Result<Vec<_>>>()?;
    let mut current_keys = BTreeMap::new();
    for key in &team.shared_keys {
        let role = stored_role(key.role_type, key.visibility)?;
        current_keys.insert(
            role,
            foks_verify::VerifiedSharedKey {
                role,
                generation: key.generation,
                verify_key: EntityId::from_bytes(key.verify_key.clone())?,
                hepk: Hepk::decode(&key.exact_hepk)?,
            },
        );
    }
    let expected_sequence = team
        .links
        .last()
        .and_then(|link| link.sequence.checked_add(1))
        .ok_or(Error::Signup("team head missing"))?;
    let expected_tail = team
        .links
        .last()
        .ok_or(Error::Signup("team head missing"))?;
    let expected_tail_hash = foks_crypto::prefixed_hash_signable(
        foks_proto::LINK_OUTER_TYPE_ID,
        &expected_tail.exact_link,
    )?;
    let verified = foks_verify::verify_team_transition(
        &argument.link,
        &argument.hepks,
        &team_id,
        &host,
        expected_sequence,
        expected_tail_hash,
        foks_proto::TreeRoot {
            epoch: root.epoch,
            hash: root.root_hash,
        },
        argument.next_tree_location,
        &current_members,
        &current_keys.into_values().collect::<Vec<_>>(),
    )?;
    validate_seed_chain_schedule(&verified.introduced_keys, &argument.seed_chain)?;
    let old = current_members
        .iter()
        .map(|member| (member_key(member), member))
        .collect::<BTreeMap<_, _>>();
    let new = verified
        .members
        .iter()
        .map(|member| (member_key(member), member))
        .collect::<BTreeMap<_, _>>();
    let added = new
        .iter()
        .filter(|(key, _)| !old.contains_key(*key))
        .map(|(_, member)| *member)
        .collect::<Vec<_>>();
    let mut expected_boxes = BTreeSet::new();
    if verified.introduced_keys.is_empty() {
        for member in &added {
            for key in verified
                .shared_keys
                .iter()
                .filter(|key| key.role <= member.role)
            {
                expected_boxes.insert((
                    member.party.as_bytes().to_vec(),
                    member
                        .scoped_host
                        .as_ref()
                        .map(|host| host.as_bytes().to_vec()),
                    member.source_role,
                    member.generation,
                    key.role,
                    key.generation,
                ));
            }
        }
    } else {
        for key in &verified.introduced_keys {
            for member in verified
                .members
                .iter()
                .filter(|member| key.role <= member.role)
            {
                expected_boxes.insert((
                    member.party.as_bytes().to_vec(),
                    member
                        .scoped_host
                        .as_ref()
                        .map(|host| host.as_bytes().to_vec()),
                    member.source_role,
                    member.generation,
                    key.role,
                    key.generation,
                ));
            }
        }
    }
    if argument.ptk_boxes.boxes.len() != expected_boxes.len() {
        return Err(Error::Signup("team edit PTK parcel count mismatch"));
    }
    let actor = current_members
        .iter()
        .find(|member| {
            member.party == change.signer_owner.party
                && member.scoped_host.is_none()
                && member.source_role == change.signer_owner.source_role
        })
        .ok_or(Error::Signup("team editor is not a member"))?;
    let mut actor_changes = change.changes.iter().filter(|member| {
        member.party == change.signer_owner.party
            && member.scoped_host.is_none()
            && member.source_role == change.signer_owner.source_role
    });
    let actor_change = actor_changes.next();
    if actor_changes.next().is_some() {
        return Err(Error::Signup("team edit contains duplicate editor changes"));
    }
    if actor_change.is_some_and(|member| member.keys.is_none()) {
        return Err(Error::Signup("team editor cannot remove its parcel sender"));
    }
    let parcel_sender = select_parcel_sender(
        actor,
        actor_change.and_then(|member| member.keys.as_ref()),
        argument.new_key_on_rotate.as_ref(),
    )?;
    let mut actual_boxes = BTreeSet::new();
    let mut parcels = Vec::with_capacity(argument.ptk_boxes.boxes.len());
    for (index, boxed) in argument.ptk_boxes.boxes.iter().enumerate() {
        let target = verified
            .members
            .iter()
            .find(|member| {
                member.party == boxed.target.entity
                    && member.source_role == boxed.target.role
                    && member.generation == boxed.target.generation
                    && member.scoped_host == boxed.target.host
            })
            .ok_or(Error::Signup("team edit PTK parcel target mismatch"))?;
        let tuple = (
            target.party.as_bytes().to_vec(),
            target
                .scoped_host
                .as_ref()
                .map(|host| host.as_bytes().to_vec()),
            target.source_role,
            target.generation,
            boxed.role,
            boxed.generation,
        );
        if !expected_boxes.contains(&tuple) || !actual_boxes.insert(tuple) {
            return Err(Error::Signup("team edit PTK parcel schedule mismatch"));
        }
        let role_seed_chain = argument
            .seed_chain
            .iter()
            .filter(|seed| seed.role == boxed.role)
            .cloned()
            .collect();
        let parcel = PukParcel::from_box_set(
            &argument.ptk_boxes,
            index,
            parcel_sender.clone(),
            role_seed_chain,
        )?;
        parcels.push(Parcel {
            party_id: target.party.as_bytes().to_vec(),
            sender_id: parcel_sender.as_bytes().to_vec(),
            target_role: target.source_role,
            role: parcel.role,
            generation: parcel.generation,
            exact: parcel.encoded()?,
        });
    }
    let removal_proofs = validate_removals(
        &argument.removals,
        &old,
        &new,
        &team_id,
        &host,
        change.signer_owner.party.as_bytes(),
        root,
    )?;
    let expected_local = added
        .iter()
        .filter(|member| {
            member
                .scoped_host
                .as_ref()
                .is_none_or(|scope| scope == &host)
        })
        .map(|member| member.party.as_bytes().to_vec())
        .collect::<BTreeSet<_>>();
    let supplied_local = argument
        .local_permissions_for
        .iter()
        .map(|entity| entity.as_bytes().to_vec())
        .collect::<BTreeSet<_>>();
    if expected_local != supplied_local {
        return Err(Error::Signup("local team permission set mismatch"));
    }
    let remote_member_view_tokens = validate_remote_member_view_tokens(
        &argument.remote_member_view_tokens,
        &added,
        &verified.shared_keys,
        &team_id,
        &host,
    )?;
    let removal_boxes = validate_removal_boxes(&argument.removal_keys, &added, &team_id, &host)?;
    let exact_link = argument.link.encoded()?;
    Ok(Command {
        team: team_id,
        sequence: expected_sequence,
        expected_tail_hash,
        link_hash: verified.link_hash,
        exact_link,
        next_tree_location: argument.next_tree_location,
        members: verified.members,
        introduced_keys: verified.introduced_keys,
        parcels,
        seed_chain: argument.seed_chain,
        removal_boxes,
        removal_proofs,
        remote_member_view_tokens,
        local_view_permissions: supplied_local
            .into_iter()
            .map(|target| (target, Role::member(0)))
            .collect(),
    })
}

fn select_parcel_sender(
    actor: &foks_verify::VerifiedTeamMemberState,
    replacement: Option<&foks_proto::TeamMemberKeys>,
    supplied: Option<&EntityId>,
) -> Result<EntityId> {
    let key_changed = replacement.is_some_and(|keys| {
        keys.generation != actor.generation
            || keys.verify_key != actor.verify_key
            || keys.hepk_fingerprint != actor.hepk_fingerprint
    });
    match (key_changed, replacement, supplied) {
        (true, Some(keys), Some(sender)) if keys.verify_key == *sender => Ok(sender.clone()),
        (true, _, _) => Err(Error::Signup(
            "new team parcel sender does not match the self change",
        )),
        (false, _, None) => Ok(actor.verify_key.clone()),
        (false, _, Some(_)) => Err(Error::Signup(
            "new team parcel sender is present without a self rotation",
        )),
    }
}

fn validate_seed_chain_schedule(
    introduced: &[foks_verify::VerifiedSharedKey],
    supplied: &[SeedChainBox],
) -> Result<()> {
    validate_seed_chain_schedule_pairs(
        introduced.iter().map(|key| (key.role, key.generation)),
        supplied.iter().map(|boxed| (boxed.role, boxed.generation)),
    )
}

fn validate_seed_chain_schedule_pairs(
    introduced: impl IntoIterator<Item = (Role, u64)>,
    supplied: impl IntoIterator<Item = (Role, u64)>,
) -> Result<()> {
    let expected = introduced
        .into_iter()
        .filter_map(|(role, generation)| {
            generation
                .checked_sub(1)
                .filter(|_| generation > 1)
                .map(|previous| (role, previous))
        })
        .collect::<BTreeSet<_>>();
    let supplied = supplied.into_iter().collect::<Vec<_>>();
    let actual = supplied.iter().copied().collect::<BTreeSet<_>>();
    if supplied.len() != actual.len() || actual != expected {
        return Err(Error::Signup("team edit PTK seed-chain schedule mismatch"));
    }
    Ok(())
}

fn validate_removal_boxes(
    boxes: &[foks_proto::TeamRemovalBoxData],
    added: &[&foks_verify::VerifiedTeamMemberState],
    team: &EntityId,
    host: &EntityId,
) -> Result<Vec<RemovalBox>> {
    if boxes.len() != added.len() {
        return Err(Error::Signup("team edit removal-box count mismatch"));
    }
    let mut output = Vec::with_capacity(boxes.len());
    let mut seen = BTreeSet::new();
    for boxed in boxes {
        let member = added
            .iter()
            .find(|member| {
                member.party == boxed.metadata.member
                    && member.source_role == boxed.metadata.source_role
            })
            .ok_or(Error::Signup("team edit removal-box member mismatch"))?;
        if boxed.metadata.team != *team
            || boxed.metadata.host != *host
            || boxed.metadata.member_host != *member.scoped_host.as_ref().unwrap_or(host)
            || boxed.metadata.destination_role != member.role
            || Some(boxed.commitment) != member.removal_key_commitment
            || !seen.insert((member.party.as_bytes().to_vec(), member.source_role))
        {
            return Err(Error::Signup("team edit removal-box binding mismatch"));
        }
        output.push(RemovalBox {
            member_id: member.party.as_bytes().to_vec(),
            member_host_id: member
                .scoped_host
                .as_ref()
                .unwrap_or(host)
                .as_bytes()
                .to_vec(),
            source_role: member.source_role,
            exact: boxed.encoded()?,
        });
    }
    Ok(output)
}

fn validate_removals(
    proofs: &[foks_proto::TeamRemovalAndCommitment],
    old: &BTreeMap<Vec<u8>, &foks_verify::VerifiedTeamMemberState>,
    new: &BTreeMap<Vec<u8>, &foks_verify::VerifiedTeamMemberState>,
    team: &EntityId,
    host: &EntityId,
    actor: &[u8],
    root: &foks_server_db::RootSnapshot,
) -> Result<Vec<RemovalProof>> {
    let lost = old
        .iter()
        .filter(|(key, prior)| new.get(*key).is_none_or(|next| next.role < prior.role))
        .map(|(_, member)| *member)
        .collect::<Vec<_>>();
    if proofs.len() != lost.len() {
        return Err(Error::Signup("team removal proof count mismatch"));
    }
    let mut seen = BTreeSet::new();
    let mut output = Vec::with_capacity(proofs.len());
    for proof in proofs {
        let payload = &proof.removal.payload;
        let member = lost
            .iter()
            .find(|member| {
                member.party == payload.member && member.source_role == payload.source_role
            })
            .ok_or(Error::Signup("team removal proof member mismatch"))?;
        if payload.team != *team
            || payload.host != *host
            || payload.member_host != *member.scoped_host.as_ref().unwrap_or(host)
            || payload.admin.as_bytes() != actor
            || payload.admin_host != *host
            || payload.root.epoch != root.epoch
            || payload.root.hash != root.root_hash
            || Some(proof.commitment) != member.removal_key_commitment
            || !seen.insert((member.party.as_bytes().to_vec(), member.source_role))
        {
            return Err(Error::Signup("team removal proof binding mismatch"));
        }
        output.push(RemovalProof {
            commitment: proof.commitment,
            member_id: member.party.as_bytes().to_vec(),
            member_host_id: member
                .scoped_host
                .as_ref()
                .unwrap_or(host)
                .as_bytes()
                .to_vec(),
            source_role: member.source_role,
            exact: proof.removal.encoded()?,
        });
    }
    Ok(output)
}

fn validate_remote_member_view_tokens(
    tokens: &[TeamRemoteMemberViewToken],
    added: &[&foks_verify::VerifiedTeamMemberState],
    shared_keys: &[foks_verify::VerifiedSharedKey],
    team: &EntityId,
    host: &EntityId,
) -> Result<Vec<TeamRemoteMemberViewToken>> {
    let expected = added
        .iter()
        .filter_map(|member| {
            member.scoped_host.as_ref().and_then(|scope| {
                (scope != host)
                    .then(|| (member.party.as_bytes().to_vec(), scope.as_bytes().to_vec()))
            })
        })
        .collect::<BTreeSet<_>>();
    if tokens.len() != expected.len() {
        return Err(Error::Signup("remote member-view token count mismatch"));
    }
    let mut seen = BTreeSet::new();
    for token in tokens {
        let tuple = (
            token.inner.member.party.as_bytes().to_vec(),
            token.inner.member.host.as_bytes().to_vec(),
        );
        let key = shared_keys.iter().find(|key| {
            key.role == token.inner.ptk_role && key.generation == token.inner.ptk_generation
        });
        if token.team != *team
            || token.inner.member.host == *host
            || token.inner.ptk_role != Role::member(0)
            || key.is_none()
            || !expected.contains(&tuple)
            || !seen.insert(tuple)
        {
            return Err(Error::Signup("remote member-view token binding mismatch"));
        }
    }
    Ok(tokens.to_vec())
}

fn stored_member(
    member: &foks_server_db::TeamMemberSnapshot,
) -> Result<foks_verify::VerifiedTeamMemberState> {
    Ok(foks_verify::VerifiedTeamMemberState {
        party: EntityId::from_bytes(member.party_id.clone())?,
        scoped_host: member
            .scoped_host_id
            .clone()
            .map(EntityId::from_bytes)
            .transpose()?,
        source_role: stored_role(member.source_role_type, member.source_visibility)?,
        role: stored_role(member.role_type, member.visibility)?,
        generation: member.generation,
        verify_key: EntityId::from_bytes(member.verify_key.clone())?,
        hepk_fingerprint: member.hepk_fingerprint,
        removal_key_commitment: member.removal_key_commitment,
    })
}

fn stored_role(kind: u64, visibility: i64) -> Result<Role> {
    crate::auth::team::stored_role(kind, visibility).ok_or(Error::Signup("stored team role"))
}

fn member_key(member: &foks_verify::VerifiedTeamMemberState) -> Vec<u8> {
    let mut key = member.party.as_bytes().to_vec();
    key.extend_from_slice(
        member
            .scoped_host
            .as_ref()
            .map_or(&[], |host| host.as_bytes()),
    );
    key.extend_from_slice(&member.source_role.protocol_value().to_be_bytes());
    key.extend_from_slice(
        &member
            .source_role
            .visibility()
            .unwrap_or_default()
            .to_be_bytes(),
    );
    key
}

#[cfg(test)]
mod tests {
    use super::{select_parcel_sender, validate_seed_chain_schedule_pairs};
    use foks_proto::{EntityId, Role, TeamMemberKeys, ENTITY_PUK_VERIFY};

    fn puk(tag: u8) -> EntityId {
        let mut bytes = vec![tag; 33];
        bytes[0] = ENTITY_PUK_VERIFY;
        EntityId::from_bytes(bytes).unwrap()
    }

    #[test]
    fn seed_chain_schedule_exactly_matches_rotated_ptks() {
        let introduced = [(Role::OWNER, 3), (Role::member(0), 2)];
        assert!(validate_seed_chain_schedule_pairs(
            introduced,
            [(Role::OWNER, 2), (Role::member(0), 1)]
        )
        .is_ok());

        assert!(validate_seed_chain_schedule_pairs(introduced, [(Role::OWNER, 2)]).is_err());
        assert!(validate_seed_chain_schedule_pairs(
            introduced,
            [(Role::OWNER, 2), (Role::member(0), 1), (Role::ADMIN, 1)]
        )
        .is_err());
        assert!(validate_seed_chain_schedule_pairs(
            introduced,
            [(Role::OWNER, 1), (Role::member(0), 1)]
        )
        .is_err());
        assert!(validate_seed_chain_schedule_pairs(
            introduced,
            [(Role::OWNER, 2), (Role::member(0), 1), (Role::member(0), 1),]
        )
        .is_err());
    }

    #[test]
    fn self_rotation_requires_the_exact_replacement_parcel_sender() {
        let old = puk(1);
        let new = puk(2);
        let actor = foks_verify::VerifiedTeamMemberState {
            party: puk(3),
            scoped_host: None,
            source_role: Role::OWNER,
            role: Role::OWNER,
            generation: 1,
            verify_key: old.clone(),
            hepk_fingerprint: [4; 32],
            removal_key_commitment: Some([5; 32]),
        };
        let replacement = TeamMemberKeys {
            verify_key: new.clone(),
            hepk_fingerprint: [6; 32],
            generation: 2,
            removal_key_commitment: None,
            index_range: None,
        };

        assert_eq!(
            select_parcel_sender(&actor, Some(&replacement), Some(&new)).unwrap(),
            new
        );
        assert!(select_parcel_sender(&actor, Some(&replacement), None).is_err());
        assert!(select_parcel_sender(&actor, Some(&replacement), Some(&old)).is_err());
        assert!(select_parcel_sender(&actor, None, Some(&old)).is_err());
        assert_eq!(select_parcel_sender(&actor, None, None).unwrap(), old);
    }
}
