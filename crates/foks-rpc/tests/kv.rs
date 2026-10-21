use foks_proto::{
    KvDirectoryPair, KvDirectoryVersion, KvDirent, KvLargeFileMetadata, KvListResponse, KvNodeId,
    KvPathVersionVector, KvRoot, KvSmallFileBox, KvUploadChunk,
};
use foks_rpc::{
    decode_optional_response, encode_kv_cache_check_request,
    encode_kv_file_upload_chunk_request_at, encode_kv_file_upload_init_request_at,
    encode_kv_get_dir_request, encode_kv_get_encrypted_chunk_request, encode_kv_get_node_request,
    encode_kv_get_root_request, encode_kv_list_request, encode_kv_lock_acquire_request_at,
    encode_kv_lock_release_request_at, encode_kv_mkdir_request_at, encode_kv_put_request_at,
    encode_kv_put_root_request_at, encode_kv_put_small_file_or_symlink_request_at,
    encode_kv_select_vhost_request, read_optional_response, read_void_response, Error, KvAuth,
    KvListCursor, KvWriteReceipt, DEFAULT_MAX_FRAME_LENGTH,
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
fn empty_cache_version_lists_use_canonical_nulls() {
    let no_directories = KvPathVersionVector {
        root_version: 1,
        directories: Vec::new(),
    };
    assert!(encode_kv_cache_check_request(KvAuth::User, &no_directories).is_ok());

    let empty_directory = KvPathVersionVector {
        root_version: 1,
        directories: vec![KvDirectoryVersion {
            id: [7; 16],
            version: 1,
            entries: Vec::new(),
        }],
    };
    assert!(encode_kv_cache_check_request(KvAuth::User, &empty_directory).is_ok());
    assert_eq!(
        KvPathVersionVector::decode(&empty_directory.encode().unwrap()).unwrap(),
        empty_directory
    );
}

#[test]
fn listed_dirents_bind_the_request_parent() {
    let listing = KvListResponse::decode(&fixture("kv-list.snowp")).unwrap();
    let expected_parent = [0xabu8; 16];
    let mut entry = listing.entries[0].clone();
    entry.parent = [0; 16];
    entry.bind_list_parent(expected_parent).unwrap();
    assert_eq!(entry.parent, expected_parent);
    assert_eq!(
        KvDirent::decode(entry.encoded()).unwrap().parent,
        expected_parent
    );

    entry.parent = [0x42; 16];
    assert!(entry.bind_list_parent(expected_parent).is_err());
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

const MAIN_DIR: &str = "../foks-snowpack/tests/fixtures/foks-main/kv";

fn main_fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{MAIN_DIR}/{name}")).unwrap()
}

fn read_kv_write_receipt(frame: &[u8], sequence: u64) -> foks_rpc::Result<KvWriteReceipt> {
    let mut response = frame;
    let data = read_optional_response(&mut response, DEFAULT_MAX_FRAME_LENGTH, sequence)?;
    assert_eq!(response, []);
    KvWriteReceipt::decode(data.as_deref().unwrap_or_default())
}

#[test]
fn go_main_kv_write_results_decode_both_result_shapes() {
    for method in ["mkdir", "put-small"] {
        assert_eq!(
            read_kv_write_receipt(&main_fixture(&format!("{method}-response-fresh.frame")), 1)
                .unwrap(),
            KvWriteReceipt { was_replay: false }
        );
        assert_eq!(
            read_kv_write_receipt(&main_fixture(&format!("{method}-response-replay.frame")), 1)
                .unwrap(),
            KvWriteReceipt { was_replay: true }
        );
        // The void decoder used for go-foks v0.1.9 refuses the struct result.
        let frame = main_fixture(&format!("{method}-response-fresh.frame"));
        assert!(matches!(
            read_void_response(&mut frame.as_slice(), DEFAULT_MAX_FRAME_LENGTH, 1),
            Err(Error::Envelope {
                expected: "nil void response data",
                ..
            })
        ));
    }
    // A v0.1.9 server omits the result for both methods.
    let void = main_fixture("void-response.frame");
    let mut response = void.as_slice();
    assert_eq!(
        read_optional_response(&mut response, DEFAULT_MAX_FRAME_LENGTH, 1).unwrap(),
        None
    );
    assert_eq!(
        read_kv_write_receipt(&void, 1).unwrap(),
        KvWriteReceipt::default()
    );
    // The pinned v0.1.9 void frame decodes the same way.
    assert_eq!(
        read_kv_write_receipt(&fixture("kv-select-vhost-response.frame"), 0).unwrap(),
        KvWriteReceipt::default()
    );
}

#[test]
fn go_main_kv_reused_object_ids_are_race_statuses() {
    for (name, message) in [
        (
            "mkdir-race-response.frame",
            "dir id reused with different contents",
        ),
        (
            "put-small-race-response.frame",
            "small-file node id reused with different contents",
        ),
    ] {
        let error = read_kv_write_receipt(&main_fixture(name), 1).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::RemoteStatus { code: 8003, detail } if detail.detail() == Some(message)
            ),
            "{error:?}"
        );
    }
}

#[test]
fn kv_write_receipt_accepts_the_optional_struct_and_its_growth() {
    let decode = |bytes: &[u8]| KvWriteReceipt::decode(bytes).map(|receipt| receipt.was_replay);
    assert!(!decode(&[]).unwrap());
    assert!(!decode(&[0xc0]).unwrap());
    assert!(!decode(&[0x91, 0xc2]).unwrap());
    assert!(decode(&[0x91, 0xc3]).unwrap());
    // A later upstream release may append fields to this unsigned result.
    assert!(decode(&[0x92, 0xc3, 0x07]).unwrap());
    assert!(decode(&[0x91, 0x01]).is_err());
    assert!(decode(&[0xc3]).is_err());
    assert!(decode(&[0x91]).is_err());
    assert_eq!(
        KvWriteReceipt { was_replay: false }.encode().unwrap(),
        [0x91, 0xc2]
    );
    assert_eq!(
        KvWriteReceipt { was_replay: true }.encode().unwrap(),
        [0x91, 0xc3]
    );
}

#[test]
fn optional_response_rejects_malformed_data_wrappers() {
    // `{Data: [true]}` without the compatibility header.
    let mut content = vec![0x94, 0x01, 0x01, 0xc0, 0x81];
    content.extend_from_slice(b"\xa4Data\x91\xc3");
    assert!(decode_optional_response(&content, 1).is_err());
    // A data wrapper that is not a map.
    assert!(decode_optional_response(&[0x94, 0x01, 0x01, 0xc0, 0x91, 0xc3], 1).is_err());
    // The wrong sequence.
    let frame = main_fixture("mkdir-response-replay.frame");
    assert!(matches!(
        read_optional_response(&mut frame.as_slice(), DEFAULT_MAX_FRAME_LENGTH, 2),
        Err(Error::Sequence { .. })
    ));
}
