use foks_client::{NewSoftwareDeviceSecrets, SoftwareDeviceProvisionRequest};
use foks_proto::{Role, SecretSeed, MERKLE_ROOT_TYPE_ID};
use foks_server_testkit::TestAccountSpec;

use crate::support::Fixture;

#[test]
pub(crate) fn go_client_merkle_queries() {
    let fixture = Fixture::start("go-client-merkle");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("gomerkle", 0x51))
        .unwrap();
    let signup_root = created.authenticated.verified.tree_root();
    let user_key = foks_merkle_store::chain_key(0, &created.credential.uid, 1, None).unwrap();

    let mut protected = fixture.client.open_protected_store().unwrap();
    let provisioned = fixture
        .client
        .foks()
        .provision_software_device(
            fixture.host(),
            &created.credential,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: "second Merkle device".to_owned(),
                serial: 2,
            },
            NewSoftwareDeviceSecrets::new(SecretSeed::new([0x61; 32]), None, [0x62; 17]),
            &mut protected,
        )
        .unwrap();

    let historical = fixture
        .client
        .foks()
        .merkle_lookup(fixture.host(), user_key, false, Some(signup_root.epoch))
        .unwrap();
    assert_eq!(historical.root.epoch, signup_root.epoch);
    assert_eq!(
        foks_crypto::prefixed_hash_signable(
            MERKLE_ROOT_TYPE_ID,
            &historical.root.encoded().unwrap(),
        )
        .unwrap(),
        signup_root.hash
    );
    foks_verify::verify_merkle_path_present(
        &historical.path,
        &user_key,
        &historical.root.root_node,
    )
    .unwrap();

    let missing_key = [0xff; 32];
    let current = fixture
        .client
        .foks()
        .merkle_multi_lookup(fixture.host(), &[user_key, missing_key], true, None)
        .unwrap();
    assert_eq!(current.paths.len(), 2);
    foks_verify::verify_merkle_path_present(&current.paths[0], &user_key, &current.root.root_node)
        .unwrap();
    foks_verify::verify_merkle_path(
        &current.paths[1],
        &missing_key,
        None,
        &current.root.root_node,
    )
    .unwrap();

    assert_eq!(
        fixture
            .client
            .foks()
            .current_merkle_root_hash(fixture.host())
            .unwrap(),
        provisioned.authenticated.verified.tree_root()
    );
    assert_eq!(
        fixture
            .client
            .foks()
            .merkle_key_exists(fixture.host(), user_key)
            .unwrap(),
        foks_proto::MerkleExistsResponse {
            epoch: signup_root.epoch,
            signed: true,
        }
    );
    assert!(matches!(
        fixture
            .client
            .foks()
            .merkle_key_exists(fixture.host(), missing_key),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 4002,
            ..
        }))
    ));
    assert!(matches!(
        fixture
            .client
            .foks()
            .merkle_lookup(fixture.host(), user_key, false, Some(u64::MAX)),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 4001,
            ..
        }))
    ));
}
