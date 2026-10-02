#![no_main]
use foks_agent_proto as frame;
use foks_agent_proto::{KvUploadFrame, KvUploadPayload, Operation, Request, PROTOCOL_VERSION};
use libfuzzer_sys::fuzz_target;

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
