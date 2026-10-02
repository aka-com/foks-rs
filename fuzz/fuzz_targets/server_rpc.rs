#![no_main]
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|input: &[u8]| {
    const LIMIT: usize = 2 * 1024 * 1024;
    if input.len() > LIMIT {
        return;
    }
    // These are the server's actual call/control parsers, not its client-side
    // probe-response decoder. Exercise bare content and bounded framed input.
    let _ = foks_rpc::decode_call(input);
    let _ = foks_rpc::decode_message(input);
    let _ = foks_rpc::read_call(&mut Cursor::new(input), LIMIT);
    let _ = foks_rpc::read_message(&mut Cursor::new(input), LIMIT);

    let argument = foks_snowpack::encode(&foks_snowpack::Value::Binary(input.to_vec())).unwrap();
    for protocol in [foks_rpc::USER_PROTOCOL_ID, foks_rpc::KEX_PROTOCOL_ID] {
        let framed = foks_rpc::encode_call(protocol, 1, &argument, 7).unwrap();
        let call = foks_rpc::read_call(&mut Cursor::new(&framed), LIMIT + 128).unwrap();
        assert_eq!(call.argument(), argument);
        assert_eq!(call.protocol_id(), protocol);
        assert_eq!(call.sequence(), 7);
        assert!(matches!(
            foks_rpc::read_message(&mut Cursor::new(&framed), LIMIT + 128).unwrap(),
            foks_rpc::InboundMessage::Call(decoded) if decoded == call
        ));
        // Optional Go log tags must be parsed completely, even for nested or
        // malformed values. Input controls the tag, rather than only a blob.
        let mut content = foks_rpc::read_frame(&mut Cursor::new(&framed), LIMIT + 128).unwrap();
        content[0] = 0x96;
        content.extend_from_slice(input);
        let _ = foks_rpc::decode_call(&content);
    }
    for method in [2, 3, 6, 7] {
        let mut control = vec![0x92, method];
        control.extend_from_slice(input);
        let _ = foks_rpc::decode_message(&control);
    }
});
