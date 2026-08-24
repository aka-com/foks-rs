use foks_crypto::derive_shared_public;
use foks_proto::{Role, SecretSeed, UserLink, ENTITY_PTK_VERIFY};

use super::{
    required_rotation_roles, rotation_operation_id, validate_rotation_change, RotationBinding,
};

const DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{name}")).unwrap()
}

#[test]
fn reconciliation_is_bound_to_the_exact_official_rotation() {
    let link = UserLink::decode(&fixture("remove-member-link.snowp")).unwrap();
    let change = link.decode_team_group_change().unwrap();
    let rotations = [
        "remove-member-ptk-member-min-seed.bin",
        "remove-member-ptk-member-seed.bin",
    ];
    let introduced = change
        .shared_keys
        .iter()
        .zip(rotations)
        .map(|(key, name)| {
            let seed = SecretSeed::new(fixture(name).try_into().unwrap());
            (
                key.role,
                key.generation,
                derive_shared_public(&seed, ENTITY_PTK_VERIFY)
                    .unwrap()
                    .verify_key,
            )
        })
        .collect();
    let binding = RotationBinding {
        target: change.changes[0].party.clone(),
        target_host: None,
        target_source_role: Role::OWNER,
        removal_key_commitment: [7; 32],
        destination_role: Role::NONE,
        replacement: None,
        expected_seqno: change.seqno,
        introduced,
    };
    validate_rotation_change(&change, &binding).unwrap();
    let operation =
        rotation_operation_id(&change.signer_owner.party, &change.team, &binding).unwrap();

    let mut wrong = binding;
    wrong.removal_key_commitment[0] ^= 1;
    assert_ne!(
        operation,
        rotation_operation_id(&change.signer_owner.party, &change.team, &wrong).unwrap()
    );
    wrong.introduced.pop();
    assert!(validate_rotation_change(&change, &wrong).is_err());
}

#[test]
fn removal_demotion_and_generation_advance_have_exact_key_floods() {
    let roles = [
        Role::member(-0x4000),
        Role::member(0),
        Role::ADMIN,
        Role::OWNER,
    ];
    assert_eq!(
        required_rotation_roles(roles, Role::OWNER, 1, Role::NONE, None),
        roles
    );
    assert_eq!(
        required_rotation_roles(roles, Role::OWNER, 1, Role::ADMIN, Some(1)),
        [Role::OWNER]
    );
    assert_eq!(
        required_rotation_roles(roles, Role::member(0), 3, Role::member(0), Some(4)),
        [Role::member(-0x4000), Role::member(0)]
    );
}
