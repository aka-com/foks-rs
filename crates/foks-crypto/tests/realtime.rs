use foks_crypto::{derive_realtime_keys, realtime_message_nonce};
use foks_proto::*;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "../foks-snowpack/tests/fixtures/foks-v0.1.9/realtime/{name}"
    ))
    .unwrap()
}
fn seed() -> SecretSeed {
    SecretSeed::new(fixture("seed.bin").try_into().unwrap())
}
fn noncer() -> RtMessageNoncer {
    RtMessageNoncer::decode(&fixture("noncer.snowp")).unwrap()
}

#[test]
fn go_message_and_metadata_ciphertexts_match_both_directions() {
    let keys = derive_realtime_keys(&seed(), RtAppId::Chat).unwrap();
    let n = noncer();
    assert_eq!(
        realtime_message_nonce(&n).unwrap().as_slice(),
        fixture("message-nonce.bin")
    );
    let sealed = keys.seal_basic_message(&n, b"hello").unwrap();
    assert_eq!(sealed.0, fixture("message-ciphertext.bin"));
    assert_eq!(
        keys.open_basic_message(&n, &sealed).unwrap().as_slice(),
        b"hello"
    );
    for (purpose, plain, boxed) in [
        (RtKeyType::ChannelName, "name.snowp", "name-box.snowp"),
        (
            RtKeyType::ChannelDescription,
            "description.snowp",
            "description-box.snowp",
        ),
    ] {
        let text = RtText::decode(&fixture(plain)).unwrap();
        let expected = SecretBox::decode(&fixture(boxed)).unwrap();
        let nonce = fixture("text-nonce.bin").try_into().unwrap();
        assert_eq!(keys.seal_text(purpose, &text, nonce).unwrap(), expected);
        assert_eq!(keys.open_text(purpose, &expected).unwrap(), text);
    }
}

#[test]
fn each_nonce_binding_rejects_tampering() {
    let keys = derive_realtime_keys(&seed(), RtAppId::Chat).unwrap();
    let original = noncer();
    let ciphertext = RtCiphertext(fixture("message-ciphertext.bin"));
    let alter_entity = |entity: &EntityId| {
        let mut b = entity.as_bytes().to_vec();
        b[2] ^= 1;
        EntityId::from_bytes(b).unwrap()
    };
    let mut cases = Vec::new();
    macro_rules! changed {
        ($n:ident, $body:expr) => {{
            let mut $n = original.clone();
            $body;
            cases.push($n);
        }};
    }
    changed!(n, n.channel.0[1] ^= 1);
    changed!(n, n.metadata.id.0[1] ^= 1);
    changed!(n, n.metadata.previous_id.0[1] ^= 1);
    changed!(n, n.metadata.previous_sequence = 1);
    changed!(n, n.metadata.send_time += 1);
    changed!(n, n.team.party = alter_entity(&n.team.party));
    changed!(n, n.team.host = alter_entity(&n.team.host));
    changed!(n, {
        let s = n.sender.as_mut().unwrap();
        s.party = alter_entity(&s.party)
    });
    changed!(n, {
        let s = n.sender.as_mut().unwrap();
        s.host = alter_entity(&s.host)
    });
    changed!(
        n,
        n.metadata.further_user_attribution =
            Some(RtUserId::new(original.sender.as_ref().unwrap().party.clone()).unwrap())
    );
    changed!(n, n.app = RtAppId::Crdt);
    changed!(n, n.metadata.kind = RtMessageType::Edit);
    for n in cases {
        assert!(keys.open_basic_message(&n, &ciphertext).is_err());
    }
    let mut damaged = ciphertext.clone();
    damaged.0[0] ^= 1;
    assert!(keys.open_basic_message(&original, &damaged).is_err());
    assert!(
        derive_realtime_keys(&SecretSeed::new([0; 32]), RtAppId::Chat)
            .unwrap()
            .open_basic_message(&original, &ciphertext)
            .is_err()
    );
    assert!(derive_realtime_keys(&seed(), RtAppId::Crdt)
        .unwrap()
        .open_basic_message(&original, &ciphertext)
        .is_err());
    let name = SecretBox::decode(&fixture("name-box.snowp")).unwrap();
    assert!(keys
        .open_text(RtKeyType::ChannelDescription, &name)
        .is_err());
    assert!(keys.open_text(RtKeyType::Data, &name).is_err());
}

#[test]
fn rejects_bad_context_and_redacts_plaintext() {
    let keys = derive_realtime_keys(&seed(), RtAppId::Chat).unwrap();
    let mut n = noncer();
    n.sender = None;
    assert!(keys.seal_basic_message(&n, b"sensitive text").is_err());
    n = noncer();
    n.metadata.id = RtMessageId([0; 16]);
    assert!(keys.seal_basic_message(&n, b"sensitive text").is_err());
    assert!(
        !format!("{:?}", RtMessageBody::Basic(b"sensitive text".to_vec()))
            .contains("sensitive text")
    );
    assert!(!format!("{:?}", RtText("sensitive text".into())).contains("sensitive text"));
    assert!(keys
        .seal_basic_message(&noncer(), &vec![0; RT_MAX_BODY_BYTES + 1])
        .is_err());
}

#[test]
fn metadata_size_boundaries_and_plaintext_failures_are_safe() {
    let keys = derive_realtime_keys(&seed(), RtAppId::Chat).unwrap();
    for purpose in [RtKeyType::ChannelName, RtKeyType::ChannelDescription] {
        let text = RtText("a".repeat(RT_MAX_BODY_BYTES));
        let boxed = keys.seal_text(purpose, &text, [9; 16]).unwrap();
        assert!(boxed.ciphertext.len() <= RT_MAX_CIPHERTEXT_BYTES);
        assert_eq!(keys.open_text(purpose, &boxed).unwrap(), text);
        assert!(keys
            .seal_text(purpose, &RtText("a".repeat(RT_MAX_BODY_BYTES + 1)), [9; 16])
            .is_err());
    }
    // A bad schema tag or truncated tagged value must not copy plaintext into
    // diagnostics. Invalid UTF-8 and trailing bytes are rejected as well.
    for bytes in [
        vec![
            0x92, 1, 0x81, 0xa6, b's', b'e', b'c', b'r', b'e', b't', 0xc0,
        ],
        vec![
            0x92, 1, 0x81, 0xa6, b's', b'e', b'c', b'r', b'e', b't', 0xc1,
        ],
        vec![0x92, 1, 0x81, 0xa1, b'1', 0xa1, 0xff],
        [fixture("name.snowp"), vec![0]].concat(),
    ] {
        let error = RtText::decode(&bytes).unwrap_err();
        assert!(!format!("{error:?}").contains("secret"));
    }
}
