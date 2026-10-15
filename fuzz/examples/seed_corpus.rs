//! Regenerate small, synthetic protocol seeds with the current public encoders,
//! and seed the schema, envelope and verification corpora with the canonical Go
//! v0.1.9 fixtures.
use foks_agent_proto::{KvUploadFrame, KvUploadPayload, Operation, Request, PROTOCOL_VERSION};
use std::{io::Cursor, path::PathBuf};

#[path = "../support/fixtures.rs"]
mod fixtures;
#[path = "../support/schema.rs"]
mod schema;
#[path = "../support/verify.rs"]
mod verify;

fn seed(target: &str, name: &str, bytes: &[u8]) {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join(target);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(name), bytes).unwrap();
}

/// Removes the generated seeds of a corpus (Go fixtures and encoded calls),
/// keeping any other seeds.
fn reset_generated_seeds(target: &str) {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join(target);
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if name.starts_with("go-") || name.starts_with("call-") {
            std::fs::remove_file(path).unwrap();
        }
    }
}

/// Seeds a selector-table corpus with each input through every decoder that
/// accepts it, prefixed with that decoder's selector byte.
fn seed_table(target: &str, table: &'static [schema::Decoder], inputs: &[(String, Vec<u8>)]) {
    for (name, bytes) in inputs {
        for (index, (decoder, decode)) in table.iter().enumerate() {
            if matches!(decode(bytes), schema::Outcome::Rejected) {
                continue;
            }
            let mut seeded = vec![schema::selector(table, index)];
            seeded.extend_from_slice(bytes);
            seed(target, &schema::seed_name(name, decoder), &seeded);
        }
    }
}

fn fixture_entity(path: &str) -> foks_proto::EntityId {
    let bytes = std::fs::read(fixtures::directory().join(path)).unwrap();
    let foks_snowpack::Value::Binary(bytes) = foks_snowpack::decode(&bytes).unwrap() else {
        panic!("{path} is not binary");
    };
    foks_proto::EntityId::from_bytes(bytes).unwrap()
}

/// Server arguments built by the client encoders around the fixture IDs. Few
/// Go fixtures are complete call arguments, so these give the server decoders
/// well-formed starting points.
fn server_arguments() -> Vec<(String, Vec<u8>)> {
    use foks_proto::{
        Role, RtAppId, RtChannelId, RtHostId, RtListChannelsArgument, RtRecentsArgument,
        RtSelectVhostArgument, RtTeamId,
    };
    use foks_rpc::RealtimeRequest;
    let uid = fixture_entity("user/uid.snowp");
    let device = fixture_entity("user/device-id.snowp");
    let host = fixture_entity("foks.app/host-id.snowp");
    let team = fixture_entity("user/team-id.snowp");
    let calls = [
        (
            "get-login-challenge",
            foks_rpc::encode_get_login_challenge_request(&uid),
        ),
        (
            "check-name-exists",
            foks_rpc::encode_check_name_exists_request(b"alice"),
        ),
        (
            "resolve-username",
            foks_rpc::encode_resolve_username_request(b"alice", false),
        ),
        (
            "load-user-chain",
            foks_rpc::encode_load_user_chain_request(uid.as_bytes(), 1),
        ),
        (
            "load-generic-chain",
            foks_rpc::encode_load_generic_chain_request(
                &uid,
                foks_proto::CHAIN_TYPE_USER_SETTINGS,
                1,
            ),
        ),
        (
            "get-uid-lookup-challenge",
            foks_rpc::encode_get_uid_lookup_challenge_request(&device),
        ),
        (
            "get-subkey-box-challenge",
            foks_rpc::encode_get_subkey_box_challenge_request(&device),
        ),
        (
            "get-yubi-management-key",
            foks_rpc::encode_get_yubi_management_key_request(&device),
        ),
        (
            "clear-device-nag",
            foks_rpc::encode_clear_device_nag_request(true),
        ),
        (
            "join-waitlist",
            foks_rpc::encode_join_waitlist_request_at(b"alice@example.com", 1),
        ),
        (
            "merkle-lookup",
            foks_rpc::encode_merkle_lookup_request(Some(&host), [7; 32], true, Some(998), 1),
        ),
        (
            "merkle-multi-lookup",
            foks_rpc::encode_merkle_multi_lookup_request(
                Some(&host),
                &[[7; 32], [8; 32]],
                true,
                None,
                1,
            ),
        ),
        (
            "merkle-check-key-exists",
            foks_rpc::encode_merkle_check_key_exists_request(None, [7; 32], 1),
        ),
        (
            "reserve-team-name",
            foks_rpc::encode_reserve_team_name_request(b"team"),
        ),
        (
            "make-team-bearer-token",
            foks_rpc::encode_make_team_bearer_token_request(&team, Role::ADMIN, 1),
        ),
        (
            "load-team-membership-chain",
            foks_rpc::encode_load_team_membership_chain_request(&team, &host, &[7; 16], 1),
        ),
        (
            "load-team-chain",
            foks_rpc::encode_load_team_chain_request(&team, &host, &[7; 16], 1),
        ),
    ];
    let mut arguments: Vec<_> = calls
        .into_iter()
        .map(|(name, framed)| {
            let call = foks_rpc::read_call(&mut Cursor::new(framed.unwrap()), 1 << 20).unwrap();
            (format!("call-{name}"), call.argument().to_vec())
        })
        .collect();
    for (name, request) in [
        (
            "select-vhost",
            RealtimeRequest::SelectVhost(RtSelectVhostArgument {
                host: RtHostId::new(host.clone()).unwrap(),
            }),
        ),
        (
            "list-channels",
            RealtimeRequest::ListChannels(RtListChannelsArgument {
                team: RtTeamId::new(team.clone()).unwrap(),
                app: RtAppId::Chat,
                last: 0,
            }),
        ),
        (
            "recents",
            RealtimeRequest::Recents(RtRecentsArgument {
                channel: RtChannelId([7; 16]),
                stop_at: 0,
                limit: 20,
            }),
        ),
    ] {
        arguments.push((format!("call-rt-{name}"), request.argument().unwrap()));
    }
    arguments
}

fn seed_fixture_corpora() {
    let fixtures = fixtures::snowp();
    for target in [
        "snowpack",
        "rpc",
        "proto_schema",
        "server_arguments",
        "verify",
    ] {
        reset_generated_seeds(target);
    }
    for (name, bytes) in &fixtures {
        seed("snowpack", name, bytes);
        seed(
            "rpc",
            name,
            &foks_rpc::encode_success_response_at(bytes, 0).unwrap(),
        );
    }
    seed_table("proto_schema", schema::PROTO, &fixtures);
    seed_table("server_arguments", schema::SERVER, &fixtures);
    seed_table("server_arguments", schema::SERVER, &server_arguments());
    for (index, (stage, _)) in verify::STAGES.iter().enumerate() {
        let (fixture, bytes) = verify::SEEDS[index];
        let mut seeded = vec![u8::try_from(index).unwrap()];
        seeded.extend_from_slice(bytes);
        seed(
            "verify",
            &schema::seed_name(&format!("go-{fixture}"), stage),
            &seeded,
        );
    }
}

fn main() {
    seed_fixture_corpora();
    for (name, protocol) in [
        ("user", foks_rpc::USER_PROTOCOL_ID),
        ("kex", foks_rpc::KEX_PROTOCOL_ID),
    ] {
        let argument = foks_snowpack::encode(&foks_snowpack::Value::Array(vec![
            foks_snowpack::Value::Unsigned(1),
        ]))
        .unwrap();
        let framed = foks_rpc::encode_call(protocol, 1, &argument, 7).unwrap();
        seed("server_rpc", &format!("{name}-framed"), &framed);
        let mut content = foks_rpc::read_frame(&mut Cursor::new(framed), 1024).unwrap();
        seed("server_rpc", &format!("{name}-bare"), &content);
        content[0] = 0x96;
        content.extend_from_slice(&[0x81, 0xa1, b'x', 0x91, 0xc0]);
        seed("server_rpc", &format!("{name}-tags"), &content);
    }
    for method in [2, 3, 6, 7] {
        seed(
            "server_rpc",
            &format!("control-{method}"),
            &[0x92, method, 7],
        );
    }
    let request = Request::new(1, Operation::ListProfiles);
    let encoded = foks_agent_proto::encode(&request).unwrap();
    seed("agent_frames", "list-profiles", &encoded);
    seed("agent_frames", "list-profiles-json", &encoded[4..]);
    for (name, payload) in [
        (
            "chunk",
            KvUploadPayload::Chunk {
                offset: 0,
                content: b"synthetic".to_vec(),
            },
        ),
        ("commit", KvUploadPayload::Commit),
    ] {
        let frame = KvUploadFrame {
            version: PROTOCOL_VERSION,
            id: 1,
            payload,
        };
        seed(
            "agent_frames",
            name,
            &foks_agent_proto::encode(&frame).unwrap(),
        );
    }
    for scenario in 0..13 {
        seed(
            "state_archive",
            &format!("scenario-{scenario:02}"),
            &[scenario, 3, 0, 42],
        );
    }
    for (name, size) in [("below", 0x80), ("exact", 0x81), ("above", 0x82)] {
        seed("state_archive", &format!("chunk-{name}"), &[0, 1, size, 42]);
    }
}
