//! Cross-domain protocol regression tests.

use super::{
    check_compatibility_header, check_status, decode_call, encode_call,
    encode_call_with_validated_argument, encode_status_response_at, read_frame, read_response,
    resequence_call, sanitize_server_text, unsigned, Cursor, Error, OAuth2Outcome, RpcStatus,
    StatusDetail, DEFAULT_MAX_FRAME_LENGTH, MAXIMUM_STATUS_DETAIL_BYTES, METHOD_CALL_V2,
    RESPONSE_HEADER,
};
use foks_proto::{KvDirectoryVersion, KvDirentVersion, KvPathVersionVector};

#[test]
fn compatibility_headers_accept_nonzero_versions_only() {
    check_compatibility_header(RESPONSE_HEADER).unwrap();
    let mut newer = RESPONSE_HEADER.to_vec();
    *newer.last_mut().unwrap() = 2;
    check_compatibility_header(&newer).unwrap();

    let mut zero = RESPONSE_HEADER.to_vec();
    *zero.last_mut().unwrap() = 0;
    assert!(matches!(
        check_compatibility_header(&zero),
        Err(Error::Compatibility)
    ));

    let mut unknown_header_format = RESPONSE_HEADER.to_vec();
    unknown_header_format[3] = 2;
    assert!(matches!(
        check_compatibility_header(&unknown_header_format),
        Err(Error::Compatibility)
    ));
}

#[test]
fn status_responses_round_trip_in_the_positional_go_shape() {
    // No-payload status: [Sc, {}].
    let framed = encode_status_response_at(&RpcStatus::RateLimited, 9).unwrap();
    let error = read_response(
        &mut std::io::Cursor::new(&framed),
        DEFAULT_MAX_FRAME_LENGTH,
        9,
    )
    .unwrap_err();
    assert!(matches!(error, Error::RemoteStatus { code: 1012, .. }));

    // Detail-string status: [Sc, {"1": <text>}].
    let framed = encode_status_response_at(&RpcStatus::BadArguments("bad".to_owned()), 9).unwrap();
    let error = read_response(
        &mut std::io::Cursor::new(&framed),
        DEFAULT_MAX_FRAME_LENGTH,
        9,
    )
    .unwrap_err();
    assert!(
        matches!(&error, Error::RemoteStatus { code: 1030, detail } if detail.0.as_deref() == Some("bad"))
    );

    let framed = encode_status_response_at(&RpcStatus::Duplicate("block".to_owned()), 9).unwrap();
    let error = read_response(
        &mut std::io::Cursor::new(&framed),
        DEFAULT_MAX_FRAME_LENGTH,
        9,
    )
    .unwrap_err();
    assert!(
        matches!(&error, Error::RemoteStatus { code: 1001, detail } if detail.0.as_deref() == Some("block"))
    );

    let framed = encode_status_response_at(&RpcStatus::KvRace("dirent".to_owned()), 9).unwrap();
    let error = read_response(
        &mut std::io::Cursor::new(&framed),
        DEFAULT_MAX_FRAME_LENGTH,
        9,
    )
    .unwrap_err();
    assert!(
        matches!(&error, Error::RemoteStatus { code: 8003, detail } if detail.0.as_deref() == Some("dirent"))
    );

    let framed =
        encode_status_response_at(&RpcStatus::MerkleVerify("proof".to_owned()), 9).unwrap();
    let error = read_response(
        &mut std::io::Cursor::new(&framed),
        DEFAULT_MAX_FRAME_LENGTH,
        9,
    )
    .unwrap_err();
    assert!(
        matches!(&error, Error::RemoteStatus { code: 4003, detail } if detail.0.as_deref() == Some("proof"))
    );

    // Stale-cache status: [8012, {"b": [Root, [ [Id, Vers, [[Id, Vers]]] ]]}].
    let pvv = KvPathVersionVector {
        root_version: 7,
        directories: vec![KvDirectoryVersion {
            id: [1; 16],
            version: 3,
            entries: vec![KvDirentVersion {
                id: [2; 16],
                version: 4,
            }],
        }],
    };
    let framed = encode_status_response_at(&RpcStatus::StaleCache(pvv.clone()), 10).unwrap();
    let error = read_response(
        &mut std::io::Cursor::new(&framed),
        DEFAULT_MAX_FRAME_LENGTH,
        10,
    )
    .unwrap_err();
    assert!(matches!(error, Error::KvStaleCache(v) if v == pvv));
}

#[test]
fn status_detail_is_sanitized_and_bounded_in_both_encodings() {
    let hostile = format!("a\u{202e}b\x07c\u{200b}d\n{}", "界".repeat(200));
    let positional = foks_snowpack::encode(&foks_snowpack::Value::Array(vec![
        foks_snowpack::Value::Unsigned(1030),
        foks_snowpack::Value::Variant(Some((
            b"1".to_vec(),
            Box::new(foks_snowpack::Value::Text(hostile.as_bytes().to_vec())),
        ))),
    ]))
    .unwrap();
    let mut named = vec![0x82];
    super::encode_text(b"Sc", &mut named);
    super::encode_unsigned(1030, &mut named);
    super::encode_text(b"f1", &mut named);
    super::encode_text(hostile.as_bytes(), &mut named);
    for bytes in [positional, named] {
        let Err(Error::RemoteStatus { code: 1030, detail }) = check_status(&bytes) else {
            panic!("expected a remote status");
        };
        let detail = detail.detail().unwrap();
        assert!(detail.len() <= MAXIMUM_STATUS_DETAIL_BYTES);
        assert!(detail.starts_with("a b c d 界"));
        assert!(!detail.chars().any(char::is_control));
    }

    // Invalid UTF-8 is replaced rather than discarded.
    let mut invalid = vec![0x82];
    super::encode_text(b"Sc", &mut invalid);
    super::encode_unsigned(1030, &mut invalid);
    super::encode_text(b"f1", &mut invalid);
    super::encode_text(b"bad \xff byte", &mut invalid);
    let Err(error @ Error::RemoteStatus { .. }) = check_status(&invalid) else {
        panic!("expected a remote status");
    };
    assert_eq!(
        error.to_string(),
        "FOKS server refused: the server rejected the request as malformed (status 1030): \"bad \u{fffd} byte\""
    );
}

#[test]
fn server_text_keeps_whole_characters_within_both_bounds() {
    assert_eq!(sanitize_server_text("ab界".as_bytes(), 3, 1024), "ab界");
    assert_eq!(sanitize_server_text("ab界".as_bytes(), 2, 1024), "ab");
    assert_eq!(sanitize_server_text("ab界".as_bytes(), 3, 4), "ab");
    assert_eq!(
        sanitize_server_text("\u{feff}x\u{2028}y\u{2066}z\u{00ad}".as_bytes(), 16, 64),
        "x y z"
    );
    // Tag characters can spell hidden ASCII text; annotation controls,
    // U+180E and U+034F are also invisible.
    assert_eq!(
        sanitize_server_text(
            "a\u{e0001}\u{e0068}\u{e0069}b\u{fff9}c\u{fffb}d\u{180e}e\u{034f}f".as_bytes(),
            32,
            64
        ),
        "a   b c d e f"
    );
}

#[test]
fn empty_status_detail_is_omitted_from_the_message() {
    for text in ["", "\n", " \u{200b}\u{202e} ", "\u{e0041}"] {
        let mut bytes = vec![0x82];
        super::encode_text(b"Sc", &mut bytes);
        super::encode_unsigned(1030, &mut bytes);
        super::encode_text(b"f1", &mut bytes);
        super::encode_text(text.as_bytes(), &mut bytes);
        let Err(error) = check_status(&bytes) else {
            panic!("expected a remote status");
        };
        assert!(
            matches!(&error, Error::RemoteStatus { code: 1030, detail } if detail.detail().is_none()),
            "{text:?}"
        );
        assert_eq!(
            error.to_string(),
            "FOKS server refused: the server rejected the request as malformed (status 1030)"
        );
    }
}

#[test]
fn oauth2_status_details_share_one_classification() {
    let detail = |text: &str| StatusDetail::from_server(text.as_bytes());
    for (text, expected) in [
        ("authorization denied", Some(OAuth2Outcome::Denied)),
        ("callback error: access_denied", Some(OAuth2Outcome::Denied)),
        (
            "provider unavailable",
            Some(OAuth2Outcome::ProviderUnavailable),
        ),
        ("authorization denied by policy", None),
        ("email mismatch", None),
    ] {
        assert_eq!(detail(text).oauth2_outcome(), expected, "{text}");
    }
    assert_eq!(StatusDetail::default().oauth2_outcome(), None);
}

#[test]
fn decode_call_accepts_optional_log_tags_element() {
    let argument =
        foks_snowpack::encode(&foks_snowpack::Value::Binary(b"payload".to_vec())).unwrap();
    let framed = encode_call(17, 23, &argument, 42).unwrap();
    let content = read_frame(&mut std::io::Cursor::new(&framed), 4096).unwrap();
    assert_eq!(content[0], 0x95);

    // go-snowpack-rpc emits a six-element call with a trailing log-tags
    // element (here an empty map); the server must accept and ignore it.
    let mut tagged = content.clone();
    tagged[0] = 0x96;
    tagged.push(0x80);
    let decoded = decode_call(&tagged).unwrap();
    assert_eq!(decoded.protocol_id(), 17);
    assert_eq!(decoded.method_position(), 23);
    assert_eq!(decoded.sequence(), 42);
    assert_eq!(decoded.argument(), argument.as_slice());

    // The plain five-element call still decodes; a stray seventh element does not.
    assert!(decode_call(&content).is_ok());
    let mut over = tagged;
    over.push(0x80);
    assert!(matches!(decode_call(&over), Err(Error::Envelope { .. })));
}

#[test]
fn decode_call_accepts_empty_array_niladic_argument() {
    // Go encodes a niladic argument as an empty array (0x90); the server must
    // accept it for any method (here User.getSalt @3), not just getHostConfig.
    // Build the call directly since encode_call's canonical validator (rightly)
    // forbids empty arrays in general Rust-emitted encodings.
    let framed = encode_call_with_validated_argument(0x823f_0899, 3, &[0x90], 7).unwrap();
    let content = read_frame(&mut std::io::Cursor::new(&framed), 4096).unwrap();
    let decoded = decode_call(&content).unwrap();
    assert_eq!(decoded.protocol_id(), 0x823f_0899);
    assert_eq!(decoded.method_position(), 3);
    assert_eq!(decoded.argument(), [0x90]);
    super::arguments::decode_void(decoded.argument()).unwrap();

    // A malformed (truncated) argument is still rejected by the
    // canonical validator, so the empty-array carve-out does not weaken it.
    let bad = encode_call_with_validated_argument(0x823f_0899, 3, &[0x91], 7).unwrap();
    let bad_content = read_frame(&mut std::io::Cursor::new(&bad), 4096).unwrap();
    assert!(decode_call(&bad_content).is_err());
}

#[test]
fn successful_named_status_must_not_hide_a_payload() {
    // {"Sc": 0, "f11": nil}
    let malformed = [0x82, 0xa2, b'S', b'c', 0x00, 0xa3, b'f', b'1', b'1', 0xc0];
    assert!(matches!(
        check_status(&malformed),
        Err(Error::Envelope { .. })
    ));

    // A concrete zero status without a union payload remains accepted,
    // though the canonical RPC success representation is nil.
    let zero = [0x81, 0xa2, b'S', b'c', 0x00];
    check_status(&zero).unwrap();
}

#[test]
fn resequencing_preserves_the_exact_protocol_argument() {
    let argument = foks_snowpack::encode(&foks_snowpack::Value::Binary(
        b"signed protocol payload".to_vec(),
    ))
    .unwrap();
    let request = encode_call(17, 23, &argument, 0).unwrap();
    let rewritten = resequence_call(&request, 300, 1024).unwrap();
    let content = read_frame(&mut std::io::Cursor::new(&rewritten), 1024).unwrap();
    let mut cursor = Cursor::new(&content);
    assert_eq!(cursor.byte().unwrap(), 0x95);
    assert_eq!(unsigned(cursor.value().unwrap()).unwrap(), METHOD_CALL_V2);
    assert_eq!(unsigned(cursor.value().unwrap()).unwrap(), 300);
    assert!(content
        .windows(argument.len())
        .any(|window| window == argument.as_slice()));

    let mut trailing = request;
    trailing.push(0);
    assert!(matches!(
        resequence_call(&trailing, 1, 1024),
        Err(Error::Envelope { .. })
    ));
}

#[test]
fn stale_cache_array_lengths_are_bounded_before_allocation() {
    // array32(u32::MAX) with no following elements.
    let huge = [0xdd, 0xff, 0xff, 0xff, 0xff];
    assert!(matches!(
        super::named_directory_versions(&huge),
        Err(Error::CollectionTooLarge { .. })
    ));
    assert!(matches!(
        super::named_dirent_versions(&huge),
        Err(Error::CollectionTooLarge { .. })
    ));

    let mut total = foks_proto::MAXIMUM_KV_DIRENTS;
    assert!(matches!(
        super::named_dirent_versions_counted(&[0x91, 0x80], &mut total),
        Err(Error::CollectionTooLarge { .. })
    ));
}
