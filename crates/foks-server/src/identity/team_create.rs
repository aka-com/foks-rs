use foks_proto::{ChangeMetadata, EntityId, Role, TeamRemovalBoxData};
use foks_snowpack::Value;

use crate::{Error, Result};

pub(crate) struct Header {
    pub kind: u8,
    pub normalized_name: Option<Vec<u8>>,
    pub team_name_utf8: Vec<u8>,
    pub name_sequence: u64,
    pub name_commitment_key: Option<[u8; 16]>,
    pub reservation_token: Option<[u8; 17]>,
    pub reservation_expires_at: Option<u64>,
}

pub(crate) struct Member {
    pub party_id: Vec<u8>,
    pub scoped_host_id: Option<Vec<u8>>,
    pub source_role: Role,
    pub role: Role,
    pub generation: u64,
    pub verify_key: Vec<u8>,
    pub hepk_fingerprint: [u8; 32],
    pub removal_key_commitment: Option<[u8; 32]>,
}

pub(crate) struct SharedKey {
    pub role: Role,
    pub generation: u64,
    pub verify_key: Vec<u8>,
    pub exact_hepk: Vec<u8>,
}

pub(crate) struct Parcel {
    pub party_id: Vec<u8>,
    pub sender_id: Vec<u8>,
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

pub(crate) struct Command {
    pub team: EntityId,
    pub link_hash: [u8; 32],
    pub exact_link: Vec<u8>,
    pub next_tree_location: [u8; 32],
    pub header: Header,
    pub members: Vec<Member>,
    pub shared_keys: Vec<SharedKey>,
    pub parcels: Vec<Parcel>,
    pub removal_boxes: Vec<RemovalBox>,
    pub expected_root_epoch: u64,
    pub expected_root_hash: [u8; 32],
}

pub(crate) enum Argument {
    Named(foks_proto::DecodedNamedTeamCreateArgument),
    AdHoc(foks_proto::DecodedAdHocTeamCreateArgument),
}

pub(crate) fn validate(
    argument: Argument,
    authority: &foks_server_db::UserAuthoritySnapshot,
    host: &EntityId,
    principal_owner: &EntityId,
) -> Result<Command> {
    let (link, next, boxes, hepks, membership, membership_next, subchain, header, removals) =
        match argument {
            Argument::Named(argument) => {
                let normalized = foks_verify::normalize_username(&argument.name_utf8)
                    .ok_or(Error::Signup("invalid team name"))?;
                let commitment_wire = foks_snowpack::encode(&Value::Array(vec![
                    Value::Text(normalized.clone()),
                    Value::Unsigned(argument.reservation.sequence),
                ]))?;
                let expected_commitment = foks_crypto::commitment(
                    foks_proto::NAME_COMMITMENT_TYPE_ID,
                    &commitment_wire,
                    &argument.team_name_commitment_key,
                );
                let change = argument.edit.link.decode_team_group_change()?;
                if !matches!(change.metadata.first(), Some(ChangeMetadata::TeamName(value)) if *value == expected_commitment)
                {
                    return Err(Error::Signup("team name commitment mismatch"));
                }
                (
                    argument.edit.link,
                    argument.edit.next_tree_location,
                    argument.edit.ptk_boxes,
                    argument.edit.hepks,
                    argument.membership_link,
                    argument.membership_next_tree_location,
                    argument.subchain_tree_location,
                    Header {
                        kind: foks_proto::ENTITY_NAMED_TEAM,
                        normalized_name: Some(normalized),
                        team_name_utf8: argument.name_utf8,
                        name_sequence: argument.reservation.sequence,
                        name_commitment_key: Some(argument.team_name_commitment_key),
                        reservation_token: Some(argument.reservation.token),
                        reservation_expires_at: Some(argument.reservation.expires_at),
                    },
                    argument.edit.removal_keys,
                )
            }
            Argument::AdHoc(argument) => (
                argument.link,
                argument.next_tree_location,
                argument.ptk_boxes,
                argument.hepks,
                argument.membership_link,
                argument.membership_next_tree_location,
                argument.subchain_tree_location,
                Header {
                    kind: foks_proto::ENTITY_AD_HOC_TEAM,
                    normalized_name: None,
                    team_name_utf8: Vec::new(),
                    name_sequence: 0,
                    name_commitment_key: None,
                    reservation_token: None,
                    reservation_expires_at: None,
                },
                Vec::new(),
            ),
        };
    let change = link.decode_team_group_change()?;
    if change.host != *host
        || change.signer_owner.party.as_bytes() != authority.uid
        || change.team.entity_type() != header.kind
    {
        return Err(Error::Signup("team founder binding mismatch"));
    }
    let eldest_index = usize::from(header.kind == foks_proto::ENTITY_NAMED_TEAM);
    let expected_subchain = location_commitment(&subchain)?;
    if !matches!(change.metadata.get(eldest_index), Some(ChangeMetadata::Eldest { subchain_location_commitment }) if *subchain_location_commitment == expected_subchain)
    {
        return Err(Error::Signup("team subchain commitment mismatch"));
    }
    let founding = foks_verify::verify_team_founding(
        &link,
        &hepks,
        &change.team,
        host,
        foks_proto::TreeRoot {
            epoch: authority.current_root_epoch,
            hash: authority.current_root_hash,
        },
        next,
    )?;
    if founding.members.len() != 1
        || founding.members[0].party.as_bytes() != authority.uid
        || founding.members[0].role != Role::OWNER
        || founding.members[0].source_role != Role::OWNER
    {
        return Err(Error::Signup("team founding roster mismatch"));
    }
    let founder = &founding.members[0];
    let user_key = authority.shared_keys.iter().find(|key| {
        key.role_type == founder.source_role.protocol_value()
            && key.visibility == i64::from(founder.source_role.visibility().unwrap_or_default())
            && key.generation == founder.generation
            && key.verify_key == founder.verify_key.as_bytes()
    });
    if user_key.is_none() {
        return Err(Error::Signup("team founder PUK is not current"));
    }
    let membership_change = membership.decode_approved_membership()?;
    if membership_change.user.as_bytes() != authority.uid
        || membership_change.host != *host
        || membership_change.signer != *principal_owner
        || membership_change.sequence != 1
        || membership_change.previous.is_some()
        || membership_change.root.epoch != authority.current_root_epoch
        || membership_change.root.hash != authority.current_root_hash
        || membership_change.time != 0
        || membership_change.next_location_commitment != location_commitment(&membership_next)?
        || membership_change.team != change.team
        || membership_change.source_role != founder.source_role
        || membership_change.destination_role != founder.role
        || membership_change.team_sequence != 1
        || membership_change.removal_key_commitment != founder.removal_key_commitment
        || membership.signatures().len() != 1
        || foks_crypto::verify_typed(
            principal_owner,
            &membership.signatures()[0],
            foks_proto::LINK_OUTER_V1_TYPE_ID,
            &membership.signing_bytes(0)?,
        )
        .is_err()
    {
        return Err(Error::Signup("team membership signature mismatch"));
    }
    let shared_keys = founding
        .shared_keys
        .iter()
        .map(|key| {
            Ok(SharedKey {
                role: key.role,
                generation: key.generation,
                verify_key: key.verify_key.as_bytes().to_vec(),
                exact_hepk: key.hepk.encoded()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if boxes.boxes.len() != shared_keys.len() {
        return Err(Error::Signup("founding PTK parcel count mismatch"));
    }
    let mut parcels = Vec::with_capacity(boxes.boxes.len());
    for (index, boxed) in boxes.boxes.iter().enumerate() {
        let key = shared_keys
            .iter()
            .find(|key| key.role == boxed.role && key.generation == boxed.generation)
            .ok_or(Error::Signup("founding PTK parcel role mismatch"))?;
        let _ = key;
        if boxed.target.entity.as_bytes() != authority.uid
            || boxed.target.host.is_some()
            || boxed.target.role != Role::OWNER
            || boxed.target.generation != founder.generation
        {
            return Err(Error::Signup("founding PTK parcel target mismatch"));
        }
        let parcel = foks_proto::PukParcel::from_box_set(
            &boxes,
            index,
            founder.verify_key.clone(),
            Vec::new(),
        )?;
        parcels.push(Parcel {
            party_id: authority.uid.clone(),
            sender_id: founder.verify_key.as_bytes().to_vec(),
            role: parcel.role,
            generation: parcel.generation,
            exact: parcel.encoded()?,
        });
    }
    parcels.sort_by_key(|parcel| parcel.role);
    parcels.dedup_by_key(|parcel| parcel.role);
    if parcels.len() != shared_keys.len() {
        return Err(Error::Signup("duplicate founding PTK parcel role"));
    }
    let removal_boxes = validate_removal_boxes(&removals, &change.team, host, founder)?;
    let members = founding
        .members
        .into_iter()
        .map(|member| Member {
            party_id: member.party.into_bytes(),
            scoped_host_id: member.scoped_host.map(EntityId::into_bytes),
            source_role: member.source_role,
            role: member.role,
            generation: member.generation,
            verify_key: member.verify_key.into_bytes(),
            hepk_fingerprint: member.hepk_fingerprint,
            removal_key_commitment: member.removal_key_commitment,
        })
        .collect();
    let exact_link = link.encoded()?;
    Ok(Command {
        team: change.team,
        link_hash: founding.link_hash,
        exact_link,
        next_tree_location: next,
        header,
        members,
        shared_keys,
        parcels,
        removal_boxes,
        expected_root_epoch: authority.current_root_epoch,
        expected_root_hash: authority.current_root_hash,
    })
}

fn validate_removal_boxes(
    boxes: &[TeamRemovalBoxData],
    team: &EntityId,
    host: &EntityId,
    founder: &foks_verify::VerifiedTeamMemberState,
) -> Result<Vec<RemovalBox>> {
    if team.entity_type() == foks_proto::ENTITY_AD_HOC_TEAM {
        if boxes.is_empty() && founder.removal_key_commitment.is_none() {
            return Ok(Vec::new());
        }
        return Err(Error::Signup("ad-hoc team has removal material"));
    }
    let [boxed] = boxes else {
        return Err(Error::Signup("named-team founder removal box mismatch"));
    };
    if Some(boxed.commitment) != founder.removal_key_commitment
        || boxed.metadata.team != *team
        || boxed.metadata.host != *host
        || boxed.metadata.member != founder.party
        || boxed.metadata.member_host != *host
        || boxed.metadata.source_role != founder.source_role
        || boxed.metadata.destination_role != founder.role
        || boxed.metadata.team_sequence != 1
    {
        return Err(Error::Signup("named-team removal binding mismatch"));
    }
    Ok(vec![RemovalBox {
        member_id: founder.party.as_bytes().to_vec(),
        member_host_id: host.as_bytes().to_vec(),
        source_role: founder.source_role,
        exact: boxed.encoded()?,
    }])
}

fn location_commitment(location: &[u8; 32]) -> Result<[u8; 32]> {
    Ok(foks_crypto::prefixed_hash(
        foks_proto::TREE_LOCATION_TYPE_ID,
        &foks_snowpack::encode(&Value::Binary(location.to_vec()))?,
    ))
}
