use foks_client::{
    decrypt_yubi_management_key, encrypt_yubi_management_key, NewYubiDeviceSecrets,
    NoPassphraseConfigured, Passphrase, UserPukRotation, YubiAccountRequest, YubiAccountSecrets,
    YubiDeviceProvisionRequest,
};
use foks_client_db::{HardStateStore, MutationKind, MutationState};
use foks_crypto::derive_subkey_id;
use foks_proto::{InviteCode, Role, SecretSeed, YubiCardId, YubiSlotAndPqKeyId};
use foks_server_testkit::TestAccountSpec;
use foks_yubi::{MockYubiProvider, Pin, PivPolicy, SlotId, YubiProvider as _};

use crate::support::Fixture;

const SIGNING_SLOT: u8 = 0x82;
const PQ_SLOT: u8 = 0x83;

#[test]
pub(crate) fn software_owner_provisions_recovers_manages_and_revokes_yubikey() {
    let fixture = Fixture::start("yubikey-lifecycle-client");
    let software = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("yubiprov", 0x31))
        .unwrap();
    let pin = Pin::new("123456").unwrap();
    let provider = MockYubiProvider::with_card("mock-yubikey", 71001, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let prepared = provider
        .prepare(
            &card,
            SlotId::new(SIGNING_SLOT).unwrap(),
            SlotId::new(PQ_SLOT).unwrap(),
            &pin,
            None,
            PivPolicy::Once,
            PivPolicy::Never,
        )
        .unwrap();
    let subkey_seed = SecretSeed::new([0x41; 32]);
    let expected_subkey = derive_subkey_id(&subkey_seed).unwrap();
    let mut protected = fixture.client.open_protected_store().unwrap();
    let provisioned = fixture
        .client
        .foks()
        .provision_yubi_device(
            fixture.host(),
            &software.credential,
            prepared.device.as_ref(),
            YubiDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: "owner key".to_owned(),
                serial: 2,
                pq_hint: YubiSlotAndPqKeyId {
                    slot: u64::from(PQ_SLOT),
                    id: prepared.locator.pq_key_id,
                },
            },
            NewYubiDeviceSecrets::new(subkey_seed, [0x42; 17]),
            &mut protected,
        )
        .unwrap();
    assert_eq!(provisioned.authenticated.verified.chain_seqno(), 2);
    assert_eq!(provisioned.authenticated.verified.devices().len(), 2);
    let operation = HardStateStore::open(fixture.client.hard_state_path())
        .unwrap()
        .latest_mutation_for_binding(
            fixture.host().host_id().as_bytes(),
            MutationKind::DeviceProvision,
            software.credential.uid.as_bytes(),
            prepared.device.entity_id().as_bytes(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(operation.state, MutationState::Verified);
    drop(provisioned);
    let provisioned = fixture
        .client
        .foks()
        .resume_yubi_device_provision(
            fixture.host(),
            &software.credential,
            prepared.device.as_ref(),
            operation.operation_id,
            SecretSeed::new([0x41; 32]),
            Role::OWNER,
            &mut protected,
        )
        .unwrap();

    let owner = provisioned
        .authenticated
        .puks
        .iter()
        .find(|puk| puk.role == Role::OWNER)
        .unwrap();
    let envelope = encrypt_yubi_management_key(
        owner,
        prepared.device.entity_id(),
        YubiCardId {
            name: card.name.as_bytes().to_vec(),
            serial: u64::from(card.serial),
        },
        u64::from(SIGNING_SLOT),
        &[0x55; 24],
    )
    .unwrap();
    fixture
        .client
        .foks()
        .put_yubi_management_key_yubi(fixture.host(), &provisioned.credential, &envelope)
        .unwrap();
    let all_envelopes = fixture
        .client
        .foks()
        .get_all_yubi_management_keys_yubi(fixture.host(), &provisioned.credential)
        .unwrap();
    assert_eq!(all_envelopes.len(), 1);
    assert_eq!(all_envelopes[0].yubi_id, *prepared.device.entity_id());
    let mut future_envelope = envelope.clone();
    future_envelope.generation = 2;
    assert!(fixture
        .client
        .foks()
        .put_yubi_management_key(fixture.host(), &software.credential, &future_envelope,)
        .is_err());
    let loaded = fixture
        .client
        .foks()
        .get_yubi_management_key(
            fixture.host(),
            &software.credential,
            prepared.device.entity_id(),
        )
        .unwrap();
    let recovered_key = decrypt_yubi_management_key(&loaded, &software.authenticated.puks).unwrap();
    assert_eq!(recovered_key.management_key.as_slice(), &[0x55; 24]);
    assert_eq!(recovered_key.card.serial, u64::from(card.serial));

    drop(provisioned);
    let recovered = fixture
        .client
        .foks()
        .recover_yubi_credential(
            fixture.host(),
            software.credential.uid.clone(),
            &expected_subkey,
            prepared.device.as_ref(),
        )
        .unwrap();
    let authenticated = fixture
        .client
        .foks()
        .authenticate_yubi_and_pin(fixture.host(), &recovered)
        .unwrap();
    assert_eq!(authenticated.verified.chain_seqno(), 2);

    let revoked = fixture
        .client
        .foks()
        .revoke_user_credential_with_software_device(
            fixture.host(),
            &software.credential,
            prepared.device.entity_id(),
            &[UserPukRotation {
                role: Role::OWNER,
                previous_generation: 1,
                previous_seed: SecretSeed::new([0x32; 32]),
                new_seed: SecretSeed::new([0x51; 32]),
            }],
            Some(NoPassphraseConfigured),
            &mut protected,
        )
        .unwrap();
    assert_eq!(revoked.verified.chain_seqno(), 3);
    assert_eq!(revoked.verified.devices().len(), 1);
    assert!(fixture
        .client
        .foks()
        .get_yubi_management_key(
            fixture.host(),
            &software.credential,
            prepared.device.entity_id(),
        )
        .is_err());
    assert!(fixture
        .client
        .foks()
        .authenticate_yubi_and_pin(fixture.host(), &recovered)
        .is_err());
}

#[test]
fn yubikey_signup_supports_passphrase_and_delegated_subkey_recovery() {
    let fixture = Fixture::start("yubikey-signup-client");
    let pin = Pin::new("654321").unwrap();
    let provider = MockYubiProvider::with_card("signup-yubikey", 71002, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let prepared = provider
        .prepare(
            &card,
            SlotId::new(SIGNING_SLOT).unwrap(),
            SlotId::new(PQ_SLOT).unwrap(),
            &pin,
            None,
            PivPolicy::Once,
            PivPolicy::Never,
        )
        .unwrap();
    let subkey_seed = SecretSeed::new([0x61; 32]);
    let expected_subkey = derive_subkey_id(&subkey_seed).unwrap();
    let passphrase = Passphrase::new("hardware signup passphrase").unwrap();
    let mut protected = fixture.client.open_protected_store().unwrap();
    let created = fixture
        .client
        .foks()
        .create_yubi_account(
            fixture.host(),
            prepared.device.as_ref(),
            YubiAccountRequest {
                username_utf8: "yubisignup".to_owned(),
                device_name: "signup key".to_owned(),
                invite_code: InviteCode::Empty,
                email: "yubi@example.test".to_owned(),
                passphrase: Some(passphrase),
                pq_hint: YubiSlotAndPqKeyId {
                    slot: u64::from(PQ_SLOT),
                    id: prepared.locator.pq_key_id,
                },
            },
            YubiAccountSecrets::new(subkey_seed, SecretSeed::new([0x62; 32]), [0x63; 17]),
            fixture.client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    assert_eq!(created.authenticated.verified.chain_seqno(), 1);
    assert_eq!(created.authenticated.puks[0].seed.as_slice(), &[0x62; 32]);
    let operation_id = created.operation_id;
    drop(created);
    let created = fixture
        .client
        .foks()
        .resume_yubi_account_with_pending(
            fixture.host(),
            prepared.device.as_ref(),
            operation_id,
            "yubisignup",
            YubiAccountSecrets::new(
                SecretSeed::new([0x61; 32]),
                SecretSeed::new([0x62; 32]),
                [0x63; 17],
            ),
            fixture.client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    let uid = created.credential.uid.clone();
    drop(created);

    let recovered = fixture
        .client
        .foks()
        .recover_yubi_credential(
            fixture.host(),
            uid,
            &expected_subkey,
            prepared.device.as_ref(),
        )
        .unwrap();
    assert_eq!(recovered.subkey_seed.as_slice(), &[0x61; 32]);
    assert_eq!(
        fixture
            .client
            .foks()
            .verify_passphrase_yubi(
                fixture.host(),
                &recovered,
                &Passphrase::new("hardware signup passphrase").unwrap(),
            )
            .unwrap()
            .generation,
        1
    );
}
