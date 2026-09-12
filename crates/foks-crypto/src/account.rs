//! Username changes share one unsigned transition across software and hardware signers.
use crate::*;

pub struct UsernameChangeInput<'a> {
    pub base: UserMutationBase<'a>,
    pub normalized_name: &'a [u8],
    pub name_sequence: u64,
    pub commitment_key: [u8; 16],
}

pub fn username_change_unsigned(
    input: &UsernameChangeInput<'_>,
    signer: &EntityId,
) -> Result<UnsignedUserLink> {
    if input.name_sequence == 0 || input.base.seqno < 2 || input.normalized_name.is_empty() {
        return Err(Error::DeviceKey);
    }
    let name = encode(&Value::Array(vec![
        Value::Text(input.normalized_name.to_vec()),
        Value::Unsigned(input.name_sequence),
    ]))?;
    UnsignedUserLink::user_group_change(&UserGroupChange {
        seqno: input.base.seqno,
        previous: Some(input.base.previous),
        root: input.base.root.clone(),
        time: input.base.time,
        next_location_commitment: tree_location_commitment(&input.base.next_tree_location)?,
        uid: input.base.uid.clone(),
        host: input.base.host.clone(),
        signer: signer.clone(),
        changes: Vec::new(),
        shared_keys: Vec::new(),
        metadata: vec![ChangeMetadata::Username(commitment(
            NAME_COMMITMENT_TYPE_ID,
            &name,
            &input.commitment_key,
        ))],
    })
    .map_err(Into::into)
}

pub fn make_software_username_change(
    input: &UsernameChangeInput<'_>,
    seed: &SecretSeed,
) -> Result<UserLink> {
    let signer = derive_device_public(seed)?;
    let unsigned = username_change_unsigned(input, &signer.id)?;
    let signature = sign_seed_typed(seed, LINK_OUTER_V1_TYPE_ID, &unsigned.signing_bytes(&[])?)?;
    unsigned.finish(vec![signature]).map_err(Into::into)
}

pub fn make_yubi_username_change(
    input: &UsernameChangeInput<'_>,
    parent: &dyn YubiDevice,
) -> Result<UserLink> {
    let unsigned = username_change_unsigned(input, parent.entity_id())?;
    let signature = sign_yubi_typed(parent, LINK_OUTER_V1_TYPE_ID, &unsigned.signing_bytes(&[])?)?;
    unsigned.finish(vec![signature]).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_rename_reproduces_go() {
        let bytes =
            std::fs::read("../foks-snowpack/tests/fixtures/foks-v0.1.9/account/rename-full.snowp")
                .unwrap();
        let arg = foks_proto::ChangeUsernameArgument::decode(&bytes).unwrap();
        let full = arg.full.unwrap();
        let gc = full.link.decode_group_change().unwrap();
        let mut seed = [0; 32];
        seed[0] = 3;
        let actual = make_software_username_change(
            &UsernameChangeInput {
                base: UserMutationBase {
                    uid: &gc.uid,
                    host: &gc.host,
                    seqno: gc.seqno,
                    previous: gc.previous.unwrap(),
                    root: &gc.root,
                    time: gc.time,
                    next_tree_location: full.next_tree_location,
                },
                normalized_name: b"alice_new",
                name_sequence: 1,
                commitment_key: full.commitment_key,
            },
            &SecretSeed::new(seed),
        )
        .unwrap();
        assert_eq!(actual.encoded().unwrap(), full.link.encoded().unwrap());
    }
}
