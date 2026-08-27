use foks_proto::{
    DecodedProvisionDeviceArgument, DecodedRevokeDeviceArgument, EntityId, Hepk, Role, RoleType,
    SeedChainBox, UserMemberKeys,
};

use crate::{Error, Result};

pub(crate) enum Argument {
    Provision(DecodedProvisionDeviceArgument),
    Revoke(DecodedRevokeDeviceArgument),
}

impl Argument {
    pub(crate) fn link(&self) -> &foks_proto::UserLink {
        match self {
            Self::Provision(argument) => &argument.link,
            Self::Revoke(argument) => &argument.link,
        }
    }
}

pub(crate) struct AddedCredential {
    pub device_id: Vec<u8>,
    pub hepk_fingerprint: [u8; 32],
    pub exact_hepk: Vec<u8>,
    pub exact_name: Vec<u8>,
    pub role: Role,
    pub subkey_id: Option<Vec<u8>>,
}

pub(crate) struct SharedKey {
    pub role: Role,
    pub generation: u64,
    pub verify_key: Vec<u8>,
    pub exact_hepk: Vec<u8>,
}

pub(crate) struct Parcel {
    pub device_id: Vec<u8>,
    pub role: Role,
    pub generation: u64,
    pub exact: Vec<u8>,
}

pub(crate) struct Command {
    pub uid: Vec<u8>,
    pub signer: Vec<u8>,
    pub sequence: u64,
    pub expected_tail_hash: [u8; 32],
    pub link_hash: [u8; 32],
    pub exact_link: Vec<u8>,
    pub next_tree_location: [u8; 32],
    pub added: Option<AddedCredential>,
    pub revoked: Option<Vec<u8>>,
    pub shared_keys: Vec<SharedKey>,
    pub parcels: Vec<Parcel>,
    pub seed_chain: Vec<SeedChainBox>,
    pub expected_root_epoch: u64,
    pub expected_root_hash: [u8; 32],
}

pub(crate) fn validate(
    authority: &foks_server_db::UserAuthoritySnapshot,
    host: &EntityId,
    principal: &[u8],
    argument: Argument,
) -> Result<Command> {
    let (link, next_tree_location, hepks, boxes, seed_chain, exact_name) = match &argument {
        Argument::Provision(argument) => (
            &argument.link,
            argument.next_tree_location,
            &argument.hepks,
            &argument.puk_boxes,
            Vec::new(),
            Some(argument.device_name.encoded()?),
        ),
        Argument::Revoke(argument) => (
            &argument.link,
            argument.next_tree_location,
            &argument.hepks,
            &argument.puk_boxes,
            argument.seed_chain.clone(),
            None,
        ),
    };
    let uid = EntityId::from_bytes(authority.uid.clone())?;
    let devices = authority
        .devices
        .iter()
        .filter(|device| device.active)
        .map(|device| {
            Ok(foks_verify::VerifiedDevice {
                id: EntityId::from_bytes(device.device_id.clone())?,
                role: decode_role(device.role_type, device.visibility)?,
                hepk: Hepk::decode(&device.exact_hepk)?,
                subkey: device
                    .subkey_id
                    .clone()
                    .map(EntityId::from_bytes)
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let shared_keys = latest_shared_keys(&authority.shared_keys)?;
    let expected_sequence = authority
        .chain_sequence
        .checked_add(1)
        .ok_or(Error::Signup("user chain sequence overflow"))?;
    let verified = foks_verify::verify_user_transition(
        link,
        hepks,
        &uid,
        host,
        expected_sequence,
        authority.chain_tail_hash,
        foks_proto::TreeRoot {
            epoch: authority.current_root_epoch,
            hash: authority.current_root_hash,
        },
        next_tree_location,
        &devices,
        &shared_keys,
    )?;
    if verified.change.signer.as_bytes() != principal {
        return Err(Error::Signup(
            "mutation signer does not match TLS principal",
        ));
    }
    let member = verified.change.changes.first();
    let (added, revoked) = match member {
        Some(member) if member.role == Role::NONE => {
            (None, Some(member.entity.as_bytes().to_vec()))
        }
        Some(member) => {
            let UserMemberKeys::User {
                hepk_fingerprint,
                ref subkey,
            } = member.keys
            else {
                return Err(Error::Signup("unsupported provisioned credential"));
            };
            let hepk = hepk_by_fingerprint(hepks, hepk_fingerprint)?;
            let exact_name = exact_name
                .clone()
                .ok_or(Error::Signup("provisioning request has no device name"))?;
            (
                Some(AddedCredential {
                    device_id: member.entity.as_bytes().to_vec(),
                    hepk_fingerprint,
                    exact_hepk: hepk.encoded()?,
                    exact_name,
                    role: member.role,
                    subkey_id: subkey.as_ref().map(|subkey| subkey.as_bytes().to_vec()),
                }),
                None,
            )
        }
        None => (None, None),
    };
    let introduced = verified
        .change
        .shared_keys
        .iter()
        .map(|key| {
            Ok(SharedKey {
                role: key.role,
                generation: key.generation,
                verify_key: key.verify_key.as_bytes().to_vec(),
                exact_hepk: hepk_by_fingerprint(hepks, key.hepk_fingerprint)?.encoded()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let exact_boxes = boxes.encoded();
    let parcels = boxes
        .boxes
        .iter()
        .map(|boxed| Parcel {
            device_id: boxed.target.entity.as_bytes().to_vec(),
            role: boxed.role,
            generation: boxed.generation,
            exact: exact_boxes.clone(),
        })
        .collect();
    let exact_link = link.encoded()?;
    Ok(Command {
        uid: authority.uid.clone(),
        signer: verified.change.signer.as_bytes().to_vec(),
        sequence: expected_sequence,
        expected_tail_hash: authority.chain_tail_hash,
        link_hash: foks_crypto::prefixed_hash(foks_proto::LINK_OUTER_TYPE_ID, &exact_link),
        exact_link,
        next_tree_location,
        added,
        revoked,
        shared_keys: introduced,
        parcels,
        seed_chain,
        expected_root_epoch: authority.current_root_epoch,
        expected_root_hash: authority.current_root_hash,
    })
}

fn latest_shared_keys(
    keys: &[foks_server_db::UserSharedKeySnapshot],
) -> Result<Vec<foks_verify::VerifiedSharedKey>> {
    let mut latest = std::collections::BTreeMap::new();
    for key in keys {
        let role = decode_role(key.role_type, key.visibility)?;
        latest.insert(
            role,
            foks_verify::VerifiedSharedKey {
                role,
                generation: key.generation,
                verify_key: EntityId::from_bytes(key.verify_key.clone())?,
                hepk: Hepk::decode(&key.exact_hepk)?,
            },
        );
    }
    Ok(latest.into_values().collect())
}

fn decode_role(kind: u64, visibility: i64) -> Result<Role> {
    match kind {
        0 => Ok(Role::NONE),
        1 => Ok(Role::member(
            i16::try_from(visibility).map_err(|_| Error::Signup("stored role visibility"))?,
        )),
        2 if visibility == 0 => Ok(Role::ADMIN),
        3 if visibility == 0 => Ok(Role::OWNER),
        _ => Err(Error::Signup("stored role")),
    }
}

pub(crate) fn role_parts(role: Role) -> (u64, i64) {
    (
        role.protocol_value(),
        match role.kind() {
            RoleType::Member => i64::from(role.visibility().unwrap_or_default()),
            _ => 0,
        },
    )
}

fn hepk_by_fingerprint(hepks: &[Hepk], expected: [u8; 32]) -> Result<&Hepk> {
    hepks
        .iter()
        .find(|hepk| foks_crypto::hepk_fingerprint(hepk).ok() == Some(expected))
        .ok_or(Error::Signup("mutation HEPK is missing"))
}
