use std::io::{Cursor, Read};

use foks_proto::{EntityId, Role, RoleAndGeneration, TeamChain};
use foks_rpc::{
    arguments::{
        decode_activate_team_bearer_token, decode_activate_team_view, decode_load_removal_key_box,
        decode_load_team_chain, decode_lookup_uid_by_device, decode_make_team_bearer_token,
        decode_team_name_reservation_request, decode_team_view_request,
        decode_uid_lookup_challenge,
    },
    encode_call, encode_get_host_config_request, encode_load_team_chain_request_with_options,
    read_call, TeamChainLoadOptions, DEFAULT_MAX_FRAME_LENGTH, PROBE_METHOD_POSITION,
    PROBE_PROTOCOL_ID, USER_GET_HOST_CONFIG_METHOD_POSITION, USER_PROTOCOL_ID,
};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../foks-snowpack/tests/fixtures/foks-v0.1.9")
            .join(path),
    )
    .unwrap()
}

fn binary_fixture(path: &str) -> Vec<u8> {
    match foks_snowpack::decode(&fixture(path)).unwrap() {
        foks_snowpack::Value::Binary(bytes) => bytes,
        other => panic!("expected binary fixture, got {other:?}"),
    }
}

#[test]
fn sanctioned_zero_field_user_struct_reaches_server_routing() {
    let frame = encode_get_host_config_request().unwrap();
    let call = read_call(&mut Cursor::new(frame), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(call.protocol_id(), USER_PROTOCOL_ID);
    assert_eq!(call.method_position(), USER_GET_HOST_CONFIG_METHOD_POSITION);
    assert_eq!(call.argument(), [0x90]);
}

#[test]
fn official_probe_call_decodes_and_round_trips_exactly() {
    let bytes = fixture("foks.app/probe-request.frame");
    let call = read_call(&mut Cursor::new(&bytes), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(call.sequence(), 0);
    assert_eq!(call.protocol_id(), PROBE_PROTOCOL_ID);
    assert_eq!(call.method_position(), PROBE_METHOD_POSITION);
    assert_eq!(
        encode_call(
            call.protocol_id(),
            call.method_position(),
            call.argument(),
            call.sequence(),
        )
        .unwrap(),
        bytes
    );
}

#[test]
fn second_slice_server_decoders_accept_official_go_requests() {
    let argument = |name: &str| {
        let frame = fixture(&format!("user-mutations/{name}"));
        read_call(&mut Cursor::new(frame), DEFAULT_MAX_FRAME_LENGTH)
            .unwrap()
            .argument()
            .to_vec()
    };
    decode_uid_lookup_challenge(&argument("backup-lookup-challenge-request.frame")).unwrap();
    decode_lookup_uid_by_device(&argument("backup-lookup-request.frame")).unwrap();
    decode_team_view_request(&fixture_argument("user/team-view-challenge-request.frame")).unwrap();
    decode_activate_team_view(&fixture_argument("user/team-view-activate-request.frame")).unwrap();
    let team_load =
        decode_load_team_chain(&fixture_argument("user/team-load-request.frame")).unwrap();
    assert!(team_load.have_ptk_generations.is_empty());
    assert!(team_load.name_cursor.is_none());
    assert!(!team_load.load_removal_key);
    assert!(!team_load.load_remote_view_tokens);
    assert_eq!(
        decode_team_name_reservation_request(&argument("named-reserve-request.frame")).unwrap(),
        b"auditteam"
    );
    decode_make_team_bearer_token(&argument("team-bearer-make-request.frame")).unwrap();
    decode_activate_team_bearer_token(&argument("team-bearer-activate-request.frame")).unwrap();
    decode_load_removal_key_box(&argument("team-removal-key-load-request.frame")).unwrap();
}

#[test]
fn team_chain_decoder_preserves_every_nondefault_v019_field() {
    let team = EntityId::from_bytes(binary_fixture("user/team-id.snowp")).unwrap();
    let chain = TeamChain::decode(&fixture("user/team-chain.snowp")).unwrap();
    let host = chain.links[0].decode_team_group_change().unwrap().host;
    let generations = [
        RoleAndGeneration {
            role: Role::member(0),
            generation: 3,
        },
        RoleAndGeneration {
            role: Role::ADMIN,
            generation: 2,
        },
    ];
    let frame = encode_load_team_chain_request_with_options(
        &team,
        &host,
        &[0x5a; 16],
        2,
        TeamChainLoadOptions {
            have_ptk_generations: &generations,
            current_name: Some((b"fixtureteam", 2)),
            load_removal_key: true,
            load_remote_view_tokens: true,
        },
    )
    .unwrap();
    let call = read_call(&mut Cursor::new(frame), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    let decoded = decode_load_team_chain(call.argument()).unwrap();
    assert_eq!(decoded.have_ptk_generations, generations);
    assert_eq!(decoded.name_cursor, Some((b"fixtureteam".to_vec(), 2)));
    assert!(decoded.load_removal_key);
    assert!(decoded.load_remote_view_tokens);
}

fn fixture_argument(path: &str) -> Vec<u8> {
    read_call(&mut Cursor::new(fixture(path)), DEFAULT_MAX_FRAME_LENGTH)
        .unwrap()
        .argument()
        .to_vec()
}

#[test]
fn call_decoding_is_independent_of_reader_chunk_boundaries() {
    let bytes = fixture("signup/reserve-request.frame");
    for chunk_size in 1..=bytes.len() {
        let mut reader = ChunkedReader::new(&bytes, chunk_size);
        let call = read_call(&mut reader, DEFAULT_MAX_FRAME_LENGTH).unwrap();
        assert_eq!(call.sequence(), 1);
        assert_eq!(reader.remaining(), 0);
    }
}

struct ChunkedReader<'a> {
    inner: Cursor<&'a [u8]>,
    chunk_size: usize,
}

impl<'a> ChunkedReader<'a> {
    fn new(bytes: &'a [u8], chunk_size: usize) -> Self {
        Self {
            inner: Cursor::new(bytes),
            chunk_size,
        }
    }

    fn remaining(&self) -> usize {
        self.inner.get_ref().len() - self.inner.position() as usize
    }
}

impl Read for ChunkedReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(self.chunk_size);
        self.inner.read(&mut output[..length])
    }
}
