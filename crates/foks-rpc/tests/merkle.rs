use foks_proto::{
    EntityId, MerkleExistsResponse, MerkleLookupResponse, MerkleMultiLookupResponse, MerkleRoot,
};
use foks_rpc::{
    encode_get_current_merkle_root_hash_request, encode_merkle_check_key_exists_request,
    encode_merkle_lookup_request, encode_merkle_multi_lookup_request, read_response,
    DEFAULT_MAX_FRAME_LENGTH,
};
use foks_snowpack::Value;

fn user_fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../foks-snowpack/tests/fixtures/foks-v0.1.9/user")
            .join(name),
    )
    .unwrap()
}

fn checked_host() -> EntityId {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/host-id.snowp"),
    )
    .unwrap();
    let Value::Binary(bytes) = foks_snowpack::decode(&bytes).unwrap() else {
        panic!("checked HostID is not binary");
    };
    EntityId::from_bytes(bytes).unwrap()
}

fn checked_key() -> [u8; 32] {
    std::array::from_fn(|index| u8::try_from(index + 1).unwrap())
}

#[test]
fn requests_match_the_official_generated_go_stubs() {
    let host = checked_host();
    let key = checked_key();
    assert_eq!(
        encode_merkle_lookup_request(Some(&host), key, true, Some(996), 0).unwrap(),
        user_fixture("merkle-lookup-request.frame")
    );
    assert_eq!(
        encode_get_current_merkle_root_hash_request(Some(&host), 0).unwrap(),
        user_fixture("merkle-current-root-hash-request.frame")
    );
    assert_eq!(
        encode_merkle_check_key_exists_request(Some(&host), key, 0).unwrap(),
        user_fixture("merkle-check-key-request.frame")
    );
    assert_eq!(
        encode_merkle_multi_lookup_request(Some(&host), &[key, [0; 32]], false, Some(996), 0)
            .unwrap(),
        user_fixture("merkle-multi-lookup-request.frame")
    );
}

#[test]
fn responses_match_official_go_positional_shapes() {
    let current_root = read_response(
        &mut std::io::Cursor::new(user_fixture("merkle-current-root-response.frame")),
        DEFAULT_MAX_FRAME_LENGTH,
        1,
    )
    .unwrap();
    assert_eq!(current_root, user_fixture("merkle-root-998.snowp"));
    MerkleRoot::decode(&current_root).unwrap();

    let lookup = user_fixture("merkle-lookup-response.snowp");
    assert_eq!(
        MerkleLookupResponse::decode(&lookup)
            .unwrap()
            .encoded()
            .unwrap(),
        lookup
    );

    let multiple = user_fixture("merkle-multi-lookup-response.snowp");
    assert_eq!(
        MerkleMultiLookupResponse::decode(&multiple)
            .unwrap()
            .encoded()
            .unwrap(),
        multiple
    );

    let exists = user_fixture("merkle-check-key-response.snowp");
    assert_eq!(
        MerkleExistsResponse::decode(&exists)
            .unwrap()
            .encoded()
            .unwrap(),
        exists
    );

    let root = user_fixture("merkle-current-root-hash-response.snowp");
    assert_eq!(
        foks_proto::TreeRoot::decode(&root)
            .unwrap()
            .encoded()
            .unwrap(),
        root
    );
}
