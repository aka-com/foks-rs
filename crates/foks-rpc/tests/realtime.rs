use foks_proto::*;
use foks_rpc::*;
use std::io::Cursor;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "../foks-snowpack/tests/fixtures/foks-v0.1.9/realtime/{name}"
    ))
    .unwrap()
}
fn request(position: u64) -> RealtimeRequest {
    let wire = fixture(&format!("request-{position}.frame"));
    let call = read_call(&mut Cursor::new(&wire), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(call.protocol_id(), REAL_TIME_PROTOCOL_ID);
    RealtimeRequest::decode_argument(position, call.argument()).unwrap()
}
#[test]
fn all_ten_methods_match_generated_go_frames_and_results() {
    for position in [0, 2, 3, 4, 5, 6, 7, 8, 9, 10] {
        let req = request(position);
        assert_eq!(req.position(), position);
        assert_eq!(
            req.encode_at(7).unwrap(),
            fixture(&format!("request-{position}.frame"))
        );
        let response = fixture(&format!("response-{position}.frame"));
        let decoded = if req.is_void() {
            read_void_response(&mut Cursor::new(&response), DEFAULT_MAX_FRAME_LENGTH, 7).unwrap();
            Vec::new()
        } else {
            read_response(&mut Cursor::new(&response), DEFAULT_MAX_FRAME_LENGTH, 7).unwrap()
        };
        if req.is_void() {
            assert!(
                read_void_response(&mut Cursor::new(&response), DEFAULT_MAX_FRAME_LENGTH, 8)
                    .is_err()
            );
        }
        let result = req.decode_result(&decoded).unwrap();
        let encoded = match result {
            RealtimeResponse::Void => continue,
            RealtimeResponse::Channels(v) => v.encoded().unwrap(),
            RealtimeResponse::Sent(v) => v.encoded().unwrap(),
            RealtimeResponse::Thread(v) => v.encoded().unwrap(),
            RealtimeResponse::InboxVersion(v) => v.encoded().unwrap(),
            RealtimeResponse::InboxDelta(v) => v.encoded().unwrap(),
            RealtimeResponse::PollResult(v) => v.encoded().unwrap(),
            RealtimeResponse::Messages(v) => v.encoded().unwrap(),
        };
        assert_eq!(encoded, decoded);
        assert!(read_response(&mut Cursor::new(&response), DEFAULT_MAX_FRAME_LENGTH, 8).is_err());
    }
    assert!(!is_headerless_argument_protocol(REAL_TIME_PROTOCOL_ID));
    assert!(!is_headerless_result_protocol(REAL_TIME_PROTOCOL_ID));
}
#[test]
fn wrong_nesting_fields_and_unions_are_rejected() {
    let req = request(3);
    let value = foks_snowpack::decode(&req.argument().unwrap()).unwrap();
    let foks_snowpack::Value::Array(fields) = value else {
        panic!()
    };
    assert!(
        RealtimeRequest::decode_argument(3, &foks_snowpack::encode(&fields[0]).unwrap()).is_err()
    );
    assert!(RealtimeRequest::decode_argument(1, &[0xc0]).is_err());
    let mut extra = fields.clone();
    extra.push(foks_snowpack::Value::Null);
    assert!(RealtimeRequest::decode_argument(
        3,
        &foks_snowpack::encode(&foks_snowpack::Value::Array(extra)).unwrap()
    )
    .is_err());
    let b = fixture("body.snowp");
    let mut v = foks_snowpack::decode(&b).unwrap();
    if let foks_snowpack::Value::Array(f) = &mut v {
        f[0] = foks_snowpack::Value::Unsigned(2);
    }
    assert!(RtMessageBody::from_value(&v).is_err());
    assert!(RtChannelId::decode(&[0xc4, 1, 0]).is_err());
    assert!(RtChannelIdShort::new(-1).is_err());
    assert!(RealtimeRequest::decode_argument(3, &vec![0; RT_MAX_REQUEST_BYTES + 1]).is_err());
}
#[test]
fn bounds_and_optional_values_preserve_the_wire() {
    let n = RtMessageNoncer::decode(&fixture("noncer.snowp")).unwrap();
    assert_eq!(n.channel.short().get(), 0x0100000000000000);
    assert_eq!(RtText::decode(&fixture("name.snowp")).unwrap().0, "");
    let md = RtChannelMetadata::decode(&fixture("channel.snowp")).unwrap();
    assert!(md.name.key.role.visibility().unwrap() < 0);
    let mut changed = md.clone();
    changed.sequence = u64::MAX;
    assert_eq!(
        RtChannelMetadata::decode(&changed.encoded().unwrap()).unwrap(),
        changed
    );
    let mut q = RtThreadQuery {
        channel: n.channel,
        ranges: vec![],
        sequences: vec![1; RT_MAX_COLLECTION + 1],
    };
    assert!(q.encoded().is_err());
    q.sequences.clear();
    assert_eq!(RtThreadQuery::decode(&q.encoded().unwrap()).unwrap(), q);
    let RealtimeRequest::GetChangedThreads(arg) = request(6) else {
        panic!()
    };
    assert_eq!(arg.query.since, 3);
    let RealtimeRequest::PollInbox(arg) = request(8) else {
        panic!()
    };
    assert_eq!(arg.poll.timeout_milliseconds, 25_000);
}
#[test]
fn realtime_statuses_preserve_void_and_text_details() {
    for (status, code) in [
        (RpcStatus::Realtime("rt".into()), 12001),
        (RpcStatus::RtChannelExists, 12002),
        (RpcStatus::RtRace("race".into()), 12003),
        (RpcStatus::RtAmbiguousChannel("ambiguous".into()), 12004),
        (RpcStatus::RtNotFound("missing".into()), 12005),
        (RpcStatus::RtMessageOrder("order".into()), 12006),
    ] {
        let wire = encode_status_response_at(&status, 7).unwrap();
        assert!(
            matches!(read_response(&mut Cursor::new(wire),DEFAULT_MAX_FRAME_LENGTH,7),Err(foks_rpc::Error::RemoteStatus{code:c,..}) if c==code)
        );
    }
}

#[test]
fn realtime_rejects_bad_compatibility_headers() {
    for position in [0, 2, 3, 4, 5, 6, 7, 8, 9, 10] {
        let mut wire = fixture(&format!("request-{position}.frame"));
        let version = wire.windows(4).position(|w| w == b"Vers").unwrap() + 4;
        wire[version] = 0;
        assert!(read_call(&mut Cursor::new(wire), DEFAULT_MAX_FRAME_LENGTH).is_err());
        let mut response = fixture(&format!("response-{position}.frame"));
        let version = response.windows(4).position(|w| w == b"Vers").unwrap() + 4;
        response[version] = 0;
        if request(position).is_void() {
            assert!(
                read_void_response(&mut Cursor::new(response), DEFAULT_MAX_FRAME_LENGTH, 7)
                    .is_err()
            );
        } else {
            assert!(
                read_response(&mut Cursor::new(response), DEFAULT_MAX_FRAME_LENGTH, 7).is_err()
            );
        }
    }
}

#[test]
fn maximum_basic_body_fits_complete_send_with_maximum_metadata() {
    let n = RtMessageNoncer::decode(&fixture("noncer.snowp")).unwrap();
    let mut n = n;
    n.metadata.previous_sequence = u64::MAX;
    n.metadata.send_time = u64::MAX;
    n.metadata.further_user_attribution =
        Some(RtUserId::new(n.sender.as_ref().unwrap().party.clone()).unwrap());
    let seed = SecretSeed::new(fixture("seed.bin").try_into().unwrap());
    let keys = foks_crypto::derive_realtime_keys(&seed, RtAppId::Chat).unwrap();
    let body = vec![42; RT_MAX_BODY_BYTES];
    let ciphertext = keys.seal_basic_message(&n, &body).unwrap();
    assert_eq!(
        keys.open_basic_message(&n, &ciphertext).unwrap().as_slice(),
        body
    );
    let RealtimeRequest::Send(mut arg) = request(3) else {
        panic!()
    };
    arg.send.metadata = n.metadata;
    arg.send.expected_previous_sequence = u64::MAX;
    arg.send.channel = RtChannelIdShort::new(i64::MAX).unwrap();
    let RtMessageWrapper::Encrypted(boxed) = &mut arg.send.wrapper else {
        panic!()
    };
    boxed.ciphertext = ciphertext;
    boxed.key.generation = u64::MAX;
    let req = RealtimeRequest::Send(arg);
    let frame = req.encode_at(u64::MAX).unwrap();
    assert!(frame.len() <= RT_MAX_REQUEST_BYTES);
    let call = read_call(&mut Cursor::new(frame), DEFAULT_MAX_FRAME_LENGTH).unwrap();
    assert_eq!(
        RealtimeRequest::decode_argument(3, call.argument()).unwrap(),
        req
    );
    assert!(keys
        .seal_basic_message(
            &RtMessageNoncer::decode(&fixture("noncer.snowp")).unwrap(),
            &vec![0; RT_MAX_BODY_BYTES + 1]
        )
        .is_err());
}
