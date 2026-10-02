//! Regenerate small, synthetic protocol seeds with the current public encoders.
use foks_agent_proto::{KvUploadFrame, KvUploadPayload, Operation, Request, PROTOCOL_VERSION};
use std::{io::Cursor, path::PathBuf};

fn seed(target: &str, name: &str, bytes: &[u8]) {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("corpus")
        .join(target);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(name), bytes).unwrap();
}

fn main() {
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
