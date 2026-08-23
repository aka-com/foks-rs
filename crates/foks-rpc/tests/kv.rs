use foks_proto::{
    KvDirectoryPair, KvDirent, KvLargeFileMetadata, KvListResponse, KvNodeId, KvPathVersionVector,
    KvRoot, KvSmallFileBox, KvUploadChunk,
};
use foks_rpc::{
    encode_kv_cache_check_request, encode_kv_file_upload_chunk_request_at,
    encode_kv_file_upload_init_request_at, encode_kv_get_dir_request,
    encode_kv_get_encrypted_chunk_request, encode_kv_get_node_request, encode_kv_get_root_request,
    encode_kv_list_request, encode_kv_lock_acquire_request_at, encode_kv_lock_release_request_at,
    encode_kv_mkdir_request_at, encode_kv_put_request_at, encode_kv_put_root_request_at,
    encode_kv_put_small_file_or_symlink_request_at, encode_kv_select_vhost_request,
    read_void_response, Error, KvAuth, KvListCursor, DEFAULT_MAX_FRAME_LENGTH,
};

const DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{name}")).unwrap()
}

#[test]
fn official_kv_request_frames_match_byte_for_byte() {
    let host = {
        let chain = foks_proto::TeamChain::decode(&fixture("team-chain.snowp")).unwrap();
        chain.links[0].decode_team_group_change().unwrap().host
    };
    let token: [u8; 16] = std::array::from_fn(|index| 0x40 + index as u8);
    let auth = KvAuth::Team(&token);
    let root = foks_proto::KvRoot::decode(&fixture("kv-root.snowp")).unwrap();
    let listing = KvListResponse::decode(&fixture("kv-list.snowp")).unwrap();
    let small = listing.entries[0].value;
    let large = listing.entries[2].value;

    assert_eq!(
        encode_kv_select_vhost_request(&host).unwrap(),
        fixture("kv-select-vhost-request.frame")
    );
    assert_eq!(
        encode_kv_get_root_request(auth).unwrap(),
        fixture("kv-get-root-request.frame")
    );
    assert_eq!(
        encode_kv_get_dir_request(auth, &root.root).unwrap(),
        fixture("kv-get-dir-request.frame")
    );
    assert_eq!(
        encode_kv_list_request(auth, &root.root, KvListCursor::None, 100, true).unwrap(),
        fixture("kv-list-request.frame")
    );
    assert_eq!(
        encode_kv_get_node_request(auth, small).unwrap(),
        fixture("kv-get-small-node-request.frame")
    );
    assert_eq!(
        encode_kv_get_node_request(auth, large).unwrap(),
        fixture("kv-get-large-node-request.frame")
    );
    assert_eq!(
        encode_kv_get_encrypted_chunk_request(auth, large, 0).unwrap(),
        fixture("kv-get-large-chunk-request.frame")
    );
    let versions = KvPathVersionVector::decode(&fixture("kv-path-version-vector.snowp")).unwrap();
    assert_eq!(
        encode_kv_cache_check_request(auth, &versions).unwrap(),
        fixture("kv-cache-check-request.frame")
    );
}

#[test]
fn personal_auth_and_pagination_are_canonical() {
    let directory = [7u8; 16];
    let frame = encode_kv_list_request(
        KvAuth::User,
        &directory,
        KvListCursor::Mac([9u8; 32]),
        25,
        false,
    )
    .unwrap();
    assert!(!frame.is_empty());
    assert!(encode_kv_get_node_request(KvAuth::User, KvNodeId([3u8; 17])).is_ok());
}

#[test]
fn official_void_vhost_response_is_accepted() {
    let fixture = fixture("kv-select-vhost-response.frame");
    let mut response = fixture.as_slice();
    read_void_response(&mut response, DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
    assert!(response.is_empty());
}

#[test]
fn official_kv_mutation_frames_match_byte_for_byte() {
    let token: [u8; 16] = std::array::from_fn(|index| 0x40 + index as u8);
    let auth = KvAuth::Team(&token);
    let versions = KvPathVersionVector::decode(&fixture("kv-path-version-vector.snowp")).unwrap();
    let root = KvRoot::decode(&fixture("kv-root.snowp")).unwrap();
    let directory = KvDirectoryPair::decode(&fixture("kv-root-dir.snowp")).unwrap();
    let dirent = KvDirent::decode(&fixture("kv-write-dirent.snowp")).unwrap();
    let small = KvSmallFileBox::decode(&fixture("kv-small-box.snowp")).unwrap();
    let metadata = KvLargeFileMetadata::decode(&fixture("kv-write-large-metadata.snowp")).unwrap();
    let chunk = KvUploadChunk::decode(&fixture("kv-upload-chunk.snowp")).unwrap();
    let large_id: [u8; 16] = std::array::from_fn(|index| 0x80 + index as u8);
    let small_id = KvNodeId(std::array::from_fn(|index| {
        if index == 0 {
            3
        } else {
            0x3f + index as u8
        }
    }));
    let lock_id: [u8; 16] = std::array::from_fn(|index| 0xf0 + index as u8);

    assert_eq!(
        encode_kv_mkdir_request_at(auth, Some(&versions), &directory.active, 1).unwrap(),
        fixture("kv-mkdir-request.frame")
    );
    assert_eq!(
        encode_kv_put_request_at(auth, Some(&versions), std::slice::from_ref(&dirent), 1).unwrap(),
        fixture("kv-put-request.frame")
    );
    assert_eq!(
        encode_kv_put_root_request_at(auth, &root, 1).unwrap(),
        fixture("kv-put-root-request.frame")
    );
    assert_eq!(
        encode_kv_file_upload_init_request_at(auth, large_id, &metadata, &chunk, 1).unwrap(),
        fixture("kv-file-upload-init-request.frame")
    );
    assert_eq!(
        encode_kv_file_upload_chunk_request_at(auth, large_id, &chunk, 1).unwrap(),
        fixture("kv-file-upload-chunk-request.frame")
    );
    assert_eq!(
        encode_kv_put_small_file_or_symlink_request_at(auth, small_id, &small, 1).unwrap(),
        fixture("kv-put-small-request.frame")
    );
    assert_eq!(
        encode_kv_lock_acquire_request_at(auth, root.root, dirent.id, lock_id, 1000, 1,).unwrap(),
        fixture("kv-lock-acquire-request.frame")
    );
    assert_eq!(
        encode_kv_lock_release_request_at(auth, root.root, dirent.id, lock_id, 1).unwrap(),
        fixture("kv-lock-release-request.frame")
    );
}

#[test]
fn upload_final_checksum_is_preserved_exactly() {
    let mut chunk = KvUploadChunk::decode(&fixture("kv-upload-chunk.snowp")).unwrap();
    chunk
        .final_upload
        .as_mut()
        .expect("official fixture is a final chunk")
        .chunk_sum = [0x5a; 32];

    let encoded = chunk.encode().unwrap();
    assert_eq!(KvUploadChunk::decode(&encoded).unwrap(), chunk);
}

#[test]
fn official_stale_cache_status_preserves_the_version_vector() {
    let expected = KvPathVersionVector::decode(&fixture("kv-path-version-vector.snowp")).unwrap();
    let fixture = fixture("kv-stale-cache-response.frame");
    let mut response = fixture.as_slice();
    let error = read_void_response(&mut response, DEFAULT_MAX_FRAME_LENGTH, 1).unwrap_err();
    assert_eq!(response, []);
    assert!(
        matches!(error, Error::KvStaleCache(ref actual) if actual == &expected),
        "{error:?}"
    );
}
