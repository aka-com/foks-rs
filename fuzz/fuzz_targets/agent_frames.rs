#![no_main]
use foks_agent_proto as frame;
use foks_agent_proto::{
    DeadlineEnvelope, KvUploadFrame, KvUploadPayload, Operation, Request, PROTOCOL_VERSION,
};
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

/// A fixed, valid envelope keeps the generated frames deterministic. Decoding
/// checks only the envelope's syntax; clock domains are checked by the agent.
fn deadline() -> DeadlineEnvelope {
    static DEADLINE: OnceLock<DeadlineEnvelope> = OnceLock::new();
    DEADLINE
        .get_or_init(|| {
            serde_json::from_value(serde_json::json!({
                "version": 1,
                "domain": {
                    "clock": "linux_monotonic_v1",
                    "boot": "12345678-1234-1234-1234-123456789abc",
                    "device": "1",
                    "inode": "1"
                },
                "expires_ns": "1000000000"
            }))
            .unwrap()
        })
        .clone()
}

fn inspect(bytes: &[u8]) {
    let _ = frame::request_id(bytes);
    if let Ok(request) = frame::decode_request(bytes) {
        assert_eq!(frame::request_id(bytes), Some(request.id));
        match frame::encode(&request) {
            Ok(canonical) => assert!(frame::decode_request(&canonical).unwrap() == request),
            Err(error) => assert!(matches!(error, frame::Error::TooLarge)),
        }
    }
    if let Ok(upload) = frame::decode_upload_frame(bytes) {
        match frame::encode(&upload) {
            Ok(canonical) => assert_eq!(frame::decode_upload_frame(&canonical).unwrap(), upload),
            Err(error) => assert!(matches!(error, frame::Error::TooLarge)),
        }
    }
    // The agent tries every frame decode_request rejects as a Hello, of any
    // version, so this parser sees unauthenticated input too.
    if let Ok(Some(hello)) = frame::decode_hello(bytes) {
        match frame::encode(&hello) {
            Ok(canonical) => assert_eq!(frame::decode_hello(&canonical).unwrap(), Some(hello)),
            Err(error) => assert!(matches!(error, frame::Error::TooLarge)),
        }
    }
    // Client-side decoders of the frozen refusal envelope and Hello reply.
    let _ = frame::decode_response(bytes);
    if let Ok(response) = frame::decode_hello_response(bytes) {
        let _ = response.reply.require_compatible();
    }
}

fuzz_target!(|input: &[u8]| {
    if input.len() > frame::MAXIMUM_MESSAGE_BYTES + 4 {
        return;
    }
    inspect(input);
    // Also treat mutations as JSON, repairing only the outer frame length.
    // The fuzzer can then reach enum, version, base64 and payload parsing.
    let mut framed = (input.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(input);
    inspect(&framed);

    let request = Request::new(
        u64::from(input.first().copied().unwrap_or(0)),
        Operation::ListProfiles,
        deadline(),
    );
    inspect(&frame::encode(&request).unwrap());
    let upload = KvUploadFrame {
        version: PROTOCOL_VERSION,
        id: request.id,
        payload: KvUploadPayload::Chunk {
            offset: input.len() as u64,
            content: input[..input.len().min(frame::MAXIMUM_KV_PAYLOAD_BYTES)].to_vec(),
        },
    };
    inspect(&frame::encode(&upload).unwrap());
    // JSON enum/field mutations below are authenticated by neither framing nor
    // a server: the decoder must reject unsupported versions on its own.
    let mut wrong_version = request;
    wrong_version.version = PROTOCOL_VERSION.wrapping_add(1);
    assert!(matches!(
        frame::decode_request(&frame::encode(&wrong_version).unwrap()),
        Err(frame::Error::Version)
    ));
});
