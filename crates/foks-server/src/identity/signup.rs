use foks_proto::{
    ChangeMetadata, DecodedSoftwareSignupArgument, EntityId, InviteCode, MerkleRoot, Role,
    DEVICE_LABEL_TYPE_ID, ENTITY_DEVICE, ENTITY_USER, LINK_OUTER_TYPE_ID, LINK_OUTER_V1_TYPE_ID,
    NAME_COMMITMENT_TYPE_ID, TREE_LOCATION_TYPE_ID,
};
use foks_snowpack::{encode, Value};

use crate::{Error, Result};

pub(crate) struct ValidatedSignup {
    pub normalized_name: Vec<u8>,
    pub username_utf8: Vec<u8>,
    pub username_commitment_key: [u8; 16],
    pub uid: EntityId,
    pub device_id: EntityId,
    pub device_hepk_fingerprint: [u8; 32],
    pub exact_device_hepk: Vec<u8>,
    pub exact_device_name: Vec<u8>,
    pub link_hash: [u8; 32],
    pub exact_link: Vec<u8>,
    pub next_tree_location: [u8; 32],
    pub puk_verify_key: EntityId,
    pub exact_puk_hepk: Vec<u8>,
    pub exact_parcel: Vec<u8>,
    pub leaves: [([u8; 32], [u8; 32]); 2],
}

pub(crate) fn validate_software_signup(
    request: &DecodedSoftwareSignupArgument,
    expected_host: &EntityId,
    current_root: &MerkleRoot,
    current_root_hash: [u8; 32],
) -> Result<ValidatedSignup> {
    if request.invite_code != InviteCode::Empty
        || request.email.len() > 320
        || request.reservation.sequence != 1
    {
        return Err(Error::Signup("unsupported signup policy"));
    }
    let normalized_name = foks_verify::normalize_username(&request.username_utf8)
        .ok_or(Error::Signup("invalid username"))?;
    if request.device_name.normalization_version != 0
        || request.device_name.label.device_type != foks_proto::DeviceType::Computer
        || request.device_name.label.serial != 1
        || foks_verify::normalize_device_name(&request.device_name.display_name).as_deref()
            != Some(request.device_name.label.normalized_name.as_slice())
    {
        return Err(Error::Signup("invalid software device disclosure"));
    }

    let eldest = request.link.decode_eldest()?;
    if eldest.seqno != 1
        || eldest.previous.is_some()
        || eldest.root.epoch != current_root.epoch
        || eldest.root.hash != current_root_hash
        || &eldest.host != expected_host
        || eldest.signer != eldest.member
        || eldest.member != eldest.member_verify_key
        || eldest.member.entity_type() != ENTITY_DEVICE
        || eldest.member_role != Role::OWNER
        || eldest.member_source_role != Role::NONE
        || eldest.member_scoped_host.is_some()
        || eldest.member_subkey.is_some()
        || eldest.puk_generation != 1
        || request.link.signatures().len() != 2
    {
        return Err(Error::Signup("invalid software eldest link"));
    }
    let mut uid_bytes = eldest.puk_verify_key.as_bytes().to_vec();
    uid_bytes[0] = ENTITY_USER;
    let uid = EntityId::from_bytes(uid_bytes)?;
    if eldest.uid != uid {
        return Err(Error::Signup("PUK and UID do not match"));
    }

    foks_crypto::verify_typed(
        &eldest.puk_verify_key,
        &request.link.signatures()[0],
        LINK_OUTER_V1_TYPE_ID,
        &request.link.signing_bytes(0)?,
    )?;
    foks_crypto::verify_typed(
        &eldest.member,
        &request.link.signatures()[1],
        LINK_OUTER_V1_TYPE_ID,
        &request.link.signing_bytes(1)?,
    )?;
    let device_hepk_fingerprint = foks_crypto::hepk_fingerprint(&request.device_hepk)?;
    let puk_hepk_fingerprint = foks_crypto::hepk_fingerprint(&request.puk_hepk)?;
    if eldest.member_hepk_fingerprint != device_hepk_fingerprint
        || eldest.puk_hepk_fingerprint != puk_hepk_fingerprint
        || request.device_hepk.curve25519().is_none()
        || request.puk_hepk.curve25519().is_none()
    {
        return Err(Error::Signup("HEPK binding mismatch"));
    }

    let username_object = encode(&Value::Array(vec![
        Value::Text(normalized_name.clone()),
        Value::Unsigned(request.reservation.sequence),
    ]))?;
    let label = &request.device_name.label;
    let device_label_object = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type.protocol_value()),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
    let expected_metadata = [
        ChangeMetadata::Username(foks_crypto::commitment(
            NAME_COMMITMENT_TYPE_ID,
            &username_object,
            &request.username_commitment_key,
        )),
        ChangeMetadata::DeviceName(foks_crypto::commitment(
            DEVICE_LABEL_TYPE_ID,
            &device_label_object,
            &request.device_name.commitment_key,
        )),
        ChangeMetadata::Eldest {
            subchain_location_commitment: location_commitment(&request.subchain_tree_location)?,
        },
    ];
    if eldest.metadata.as_slice() != expected_metadata
        || eldest.next_tree_location != location_commitment(&request.next_tree_location)?
    {
        return Err(Error::Signup("disclosure commitment mismatch"));
    }

    let boxes = &request.puk_box;
    let valid_box = boxes.boxes.len() == 1
        && boxes.temp_dh_key.is_none()
        && boxes.boxes[0].generation == 1
        && boxes.boxes[0].role == Role::OWNER
        && boxes.boxes[0].target.entity == eldest.member
        && boxes.boxes[0].target.host.is_none()
        && boxes.boxes[0].target.role == Role::NONE
        && boxes.boxes[0].target.generation == 0;
    if !valid_box {
        return Err(Error::Signup("initial PUK box binding mismatch"));
    }

    let exact_link = request.link.encoded()?;
    let link_hash = foks_crypto::prefixed_hash(LINK_OUTER_TYPE_ID, &exact_link);
    let user_key = foks_merkle_store::chain_key(0, &uid, 1, None)?;
    let name_key = foks_merkle_store::username_key(&normalized_name, expected_host, 1)?;
    Ok(ValidatedSignup {
        normalized_name,
        username_utf8: request.username_utf8.clone(),
        username_commitment_key: request.username_commitment_key,
        uid: uid.clone(),
        device_id: eldest.member,
        device_hepk_fingerprint,
        exact_device_hepk: request.device_hepk.encoded()?,
        exact_device_name: request.device_name.encoded()?,
        link_hash,
        exact_link,
        next_tree_location: request.next_tree_location,
        puk_verify_key: eldest.puk_verify_key,
        exact_puk_hepk: request.puk_hepk.encoded()?,
        exact_parcel: request.puk_box.encoded(),
        leaves: [
            (user_key, link_hash),
            (name_key, foks_merkle_store::username_leaf(&uid)?),
        ],
    })
}

fn location_commitment(location: &[u8; 32]) -> Result<[u8; 32]> {
    Ok(foks_crypto::prefixed_hash(
        TREE_LOCATION_TYPE_ID,
        &encode(&Value::Binary(location.to_vec()))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNUP: &[u8] = include_bytes!(
        "../../../foks-snowpack/tests/fixtures/foks-v0.1.9/signup/signup-request.frame"
    );

    fn request() -> DecodedSoftwareSignupArgument {
        let call = foks_rpc::read_call(
            &mut std::io::Cursor::new(SIGNUP),
            foks_rpc::DEFAULT_MAX_FRAME_LENGTH,
        )
        .unwrap();
        DecodedSoftwareSignupArgument::decode(call.argument()).unwrap()
    }

    #[test]
    fn official_signup_validates_and_tampering_fails_closed() {
        let mut request = request();
        let eldest = request.link.decode_eldest().unwrap();
        let root = MerkleRoot {
            epoch: eldest.root.epoch,
            time: 1,
            back_pointers: [0; 32],
            root_node: [0; 32],
            hostchain: foks_proto::HostchainTail {
                seqno: 1,
                hash: [0; 32],
            },
        };
        validate_software_signup(&request, &eldest.host, &root, eldest.root.hash).unwrap();

        request.next_tree_location[0] ^= 1;
        assert!(validate_software_signup(&request, &eldest.host, &root, eldest.root.hash).is_err());
        request.next_tree_location[0] ^= 1;

        let mut link = request.link.encoded().unwrap();
        *link.last_mut().unwrap() ^= 1;
        request.link = foks_proto::UserLink::decode(&link).unwrap();
        assert!(validate_software_signup(&request, &eldest.host, &root, eldest.root.hash).is_err());
    }
}
