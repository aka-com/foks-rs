//! The forward-tolerant decoding policy (see `src/trailing.rs`): unsigned RPC
//! arguments and results accept fields appended by a later release, retain
//! them where the value is re-encoded, and leave v0.1.9 bytes unchanged;
//! canonical objects keep exact field counts.

use foks_proto::{
    decode_team_inbox, encode_team_inbox, ChangeUsernameArgument, DeviceNagInfo, HostConfig,
    InitOAuth2SessionArgument, KvDirectoryPair, KvDirent, KvEncryptedChunk, KvGetResponse,
    KvListResponse, KvNode, KvPathVersionVector, KvRoot, KvSmallFileBox, KvUploadChunk, KvUsage,
    KvUsageStats, LookupUserResult, MerkleLookupResponse, MerkleMultiLookupResponse,
    PollOAuth2SessionArgument, RealtimeWire, RegServerConfig, RtAppId, RtChannelMetadata,
    RtChannelSet, RtCreateChannelArgument, RtInboxChannel, RtInboxDelta, RtInboxPollResult,
    RtMessage, RtMessageNoncer, RtPollInbox, RtPollInboxArgument, RtThreadQuery, RtThreadRange,
    ServerClientVersionInfo, SsoConfig, SsoLoginArgument, TeamChain, UserChain, UserLink,
};
use foks_snowpack::{decode, encode, Value};

const V019: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9";

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{V019}/{path}")).unwrap()
}

/// Fields a later release might append: one scalar and one structured value.
fn tail() -> Vec<Value> {
    vec![
        Value::Unsigned(7),
        Value::Array(vec![Value::Text(b"future".to_vec()), Value::Bool(true)]),
    ]
}

fn appended(value: Value) -> Value {
    let Value::Array(mut fields) = value else {
        panic!("positional struct");
    };
    fields.extend(tail());
    Value::Array(fields)
}

/// The encoding with fields appended to the outer struct.
fn grown(bytes: &[u8]) -> Vec<u8> {
    encode(&appended(decode(bytes).unwrap())).unwrap()
}

/// The encoding with one field removed from the outer struct.
fn shrunk(bytes: &[u8]) -> Vec<u8> {
    let Value::Array(mut fields) = decode(bytes).unwrap() else {
        panic!("positional struct");
    };
    fields.pop();
    if fields.is_empty() {
        // Snowpack has no empty array; a one-field struct shrinks to nil.
        return encode(&Value::Null).unwrap();
    }
    encode(&Value::Array(fields)).unwrap()
}

/// The encoding with fields appended to the struct at `path`, a list of
/// indices into nested arrays.
fn grow_at(bytes: &[u8], path: &[usize]) -> Vec<u8> {
    fn walk(value: &mut Value, path: &[usize]) {
        match path.split_first() {
            None => *value = appended(std::mem::replace(value, Value::Null)),
            Some((index, rest)) => {
                let Value::Array(fields) = value else {
                    panic!("positional struct on the path");
                };
                walk(&mut fields[*index], rest);
            }
        }
    }
    let mut value = decode(bytes).unwrap();
    walk(&mut value, path);
    encode(&value).unwrap()
}

fn realtime_grown<T: RealtimeWire + PartialEq + std::fmt::Debug>(value: &T) {
    let exact = value.encoded().unwrap();
    assert_eq!(&T::decode(&exact).unwrap(), value);
    assert_eq!(T::decode(&grown(&exact)).unwrap(), *value);
    // Encoding never carries the discarded fields.
    assert_eq!(T::decode(&grown(&exact)).unwrap().encoded().unwrap(), exact);
    assert!(T::decode(&shrunk(&exact)).is_err());
}

fn channel() -> RtChannelMetadata {
    RtChannelMetadata::decode(&fixture("realtime/channel.snowp")).unwrap()
}

#[test]
fn realtime_rpc_wrappers_accept_appended_fields() {
    let metadata = channel();
    realtime_grown(&RtChannelSet {
        version: 3,
        channels: vec![metadata.clone()],
        mtime: 9,
    });
    let inbox = RtInboxChannel {
        metadata: metadata.clone(),
        inbox_version: 4,
        read_through: 2,
        hidden: false,
        muted: true,
    };
    realtime_grown(&inbox);
    realtime_grown(&RtInboxDelta {
        inbox_version: 4,
        app: RtAppId::Chat,
        channels: vec![inbox],
    });
    realtime_grown(&RtInboxPollResult {
        bumped: true,
        inbox_version: 5,
    });
    realtime_grown(&RtThreadQuery {
        channel: metadata.id,
        ranges: vec![RtThreadRange { start: 1, end: 4 }],
        sequences: vec![7],
    });
    realtime_grown(&RtCreateChannelArgument {
        metadata: metadata.clone(),
        set_version: 1,
    });
    realtime_grown(&RtPollInboxArgument {
        poll: RtPollInbox {
            app: RtAppId::Chat,
            since: 1,
            timeout_milliseconds: 1000,
        },
    });
}

#[test]
fn nested_realtime_wrappers_accept_appended_fields() {
    let inbox = RtInboxChannel {
        metadata: channel(),
        inbox_version: 4,
        read_through: 2,
        hidden: false,
        muted: false,
    };
    let delta = RtInboxDelta {
        inbox_version: 4,
        app: RtAppId::Chat,
        channels: vec![inbox],
    };
    let exact = delta.encoded().unwrap();
    // RtInboxDelta.channels[0] grows; the channel metadata inside it keeps
    // its retained tail separately (P0-2).
    let bytes = grow_at(&exact, &[2, 0]);
    assert_eq!(RtInboxDelta::decode(&bytes).unwrap(), delta);
}

#[test]
fn canonical_realtime_objects_stay_exact() {
    let noncer = fixture("realtime/noncer.snowp");
    RtMessageNoncer::decode(&noncer).unwrap();
    assert!(RtMessageNoncer::decode(&grown(&noncer)).is_err());
    // The metadata inside the noncer is the hashed nonce input.
    assert!(RtMessageNoncer::decode(&grow_at(&noncer, &[0])).is_err());
    let message = fixture("realtime/message.snowp");
    RtMessage::decode(&message).unwrap();
    assert!(RtMessage::decode(&grown(&message)).is_err());
}

#[test]
fn kv_response_wrappers_retain_appended_fields() {
    let list = fixture("user/kv-list.snowp");
    let original = KvListResponse::decode(&list).unwrap();
    assert_eq!(original.encoded(), list);
    let longer = grown(&list);
    let decoded = KvListResponse::decode(&longer).unwrap();
    assert_eq!(decoded.entries, original.entries);
    assert_eq!(decoded.final_page, original.final_page);
    assert_eq!(decoded.extended, original.extended);
    assert_eq!(decoded.encoded(), longer);
    assert!(KvListResponse::decode(&shrunk(&list)).is_err());

    let pair = fixture("user/kv-root-dir.snowp");
    let original = KvDirectoryPair::decode(&pair).unwrap();
    assert_eq!(original.encoded(), pair);
    let longer = grown(&pair);
    let decoded = KvDirectoryPair::decode(&longer).unwrap();
    assert_eq!(decoded.active, original.active);
    assert_eq!(decoded.encoded(), longer);
    // A directory node re-encodes the pair from its retained bytes, so the
    // tail survives a decode and re-serve of the node.
    let node = encode(&Value::Array(vec![
        Value::Unsigned(1),
        Value::Variant(Some((b"4".to_vec(), Box::new(decode(&longer).unwrap())))),
    ]))
    .unwrap();
    assert_eq!(KvNode::decode(&node).unwrap().encoded().unwrap(), node);

    let dirent = KvDirent::decode(&fixture("user/kv-write-dirent.snowp")).unwrap();
    let response = KvGetResponse::new(dirent, None).unwrap();
    let longer = grown(response.encoded());
    let decoded = KvGetResponse::decode(&longer).unwrap();
    assert_eq!(decoded.dirent, response.dirent);
    assert_eq!(decoded.encoded(), longer);
    assert_eq!(
        KvGetResponse::decode(response.encoded()).unwrap().encoded(),
        response.encoded()
    );
}

#[test]
fn kv_read_only_results_accept_appended_fields_at_every_level() {
    let chunk = fixture("user/kv-large-chunk.snowp");
    let original = KvEncryptedChunk::decode(&chunk).unwrap();
    assert_eq!(original.encode().unwrap(), chunk);
    assert_eq!(KvEncryptedChunk::decode(&grown(&chunk)).unwrap(), original);
    assert!(KvEncryptedChunk::decode(&shrunk(&chunk)).is_err());

    let vector = fixture("user/kv-path-version-vector.snowp");
    let original = KvPathVersionVector::decode(&vector).unwrap();
    assert_eq!(original.encode().unwrap(), vector);
    assert!(!original.directories.is_empty());
    for path in [&[][..], &[1, 0], &[1, 0, 2, 0]] {
        if path.len() == 4 && original.directories[0].entries.is_empty() {
            continue;
        }
        assert_eq!(
            KvPathVersionVector::decode(&grow_at(&vector, path)).unwrap(),
            original,
            "{path:?}"
        );
    }

    let usage = KvUsage {
        small: KvUsageStats {
            number: 2,
            bytes: 300,
        },
        large: foks_proto::KvChunkedUsageStats {
            base: KvUsageStats {
                number: 1,
                bytes: 5000,
            },
            chunks: 3,
        },
    };
    let exact = usage.encode().unwrap();
    for path in [&[][..], &[0], &[1], &[1, 0]] {
        assert_eq!(KvUsage::decode(&grow_at(&exact, path)).unwrap(), usage);
    }
}

#[test]
fn canonical_kv_objects_stay_exact() {
    for (path, decode_fn) in [
        (
            "user/kv-write-dirent.snowp",
            (|b: &[u8]| KvDirent::decode(b).map(drop)) as fn(&[u8]) -> foks_proto::Result<()>,
        ),
        ("user/kv-root.snowp", |b| KvRoot::decode(b).map(drop)),
        ("user/kv-small-box.snowp", |b| {
            KvSmallFileBox::decode(b).map(drop)
        }),
        // The server digests the exact upload chunk.
        ("user/kv-upload-chunk.snowp", |b| {
            KvUploadChunk::decode(b).map(drop)
        }),
    ] {
        let exact = fixture(path);
        decode_fn(&exact).unwrap();
        assert!(decode_fn(&grown(&exact)).is_err(), "{path}");
    }
    // A directory inside the pair is stored and compared by the server.
    let pair = fixture("user/kv-root-dir.snowp");
    assert!(KvDirectoryPair::decode(&grow_at(&pair, &[0])).is_err());
}

#[test]
fn chain_and_merkle_responses_accept_appended_fields() {
    let chain = fixture("user/user-chain.snowp");
    let original = UserChain::decode(&chain).unwrap();
    assert_eq!(original.exact_bytes, chain);
    let longer = grown(&chain);
    let decoded = UserChain::decode(&longer).unwrap();
    assert_eq!(decoded.links, original.links);
    assert_eq!(decoded.merkle, original.merkle);
    assert_eq!(decoded.hepks, original.hepks);
    assert_eq!(decoded.exact_bytes, longer);
    assert!(UserChain::decode(&shrunk(&chain)).is_err());

    let team = fixture("user/team-chain.snowp");
    let original = TeamChain::decode(&team).unwrap();
    assert_eq!(original.exact_bytes, team);
    let longer = grown(&team);
    let decoded = TeamChain::decode(&longer).unwrap();
    assert_eq!(decoded.links, original.links);
    assert_eq!(decoded.hepks, original.hepks);
    assert_eq!(decoded.exact_bytes, longer);

    let lookup = fixture("user/merkle-lookup-response.snowp");
    let original = MerkleLookupResponse::decode(&lookup).unwrap();
    assert_eq!(original.encoded().unwrap(), lookup);
    assert_eq!(
        MerkleLookupResponse::decode(&grown(&lookup)).unwrap(),
        original
    );
    let multi = fixture("user/merkle-multi-lookup-response.snowp");
    let original = MerkleMultiLookupResponse::decode(&multi).unwrap();
    assert_eq!(original.encoded().unwrap(), multi);
    assert_eq!(
        MerkleMultiLookupResponse::decode(&grown(&multi)).unwrap(),
        original
    );

    let lookup = fixture("user-mutations/backup-lookup-result.snowp");
    let original = LookupUserResult::decode(&lookup).unwrap();
    assert_eq!(original.encoded().unwrap(), lookup);
    assert_eq!(LookupUserResult::decode(&grown(&lookup)).unwrap(), original);

    let historical = fixture("user/merkle-historical-response.snowp");
    let original = foks_proto::HistoricalMerkleRoots::decode(&historical).unwrap();
    assert_eq!(original.encoded().unwrap(), historical);
    assert_eq!(
        foks_proto::HistoricalMerkleRoots::decode(&grown(&historical)).unwrap(),
        original
    );

    // Signed links stay exact inside a tolerant wrapper.
    let link = fixture("user/user-eldest-link.snowp");
    UserLink::decode(&link).unwrap();
    assert!(UserLink::decode(&grown(&link)).is_err());
}

#[test]
fn configuration_and_login_results_accept_appended_fields() {
    let config = fixture("user/reg-server-config-open.snowp");
    let original = RegServerConfig::decode(&config).unwrap();
    assert_eq!(original.encoded().unwrap(), config);
    assert_eq!(RegServerConfig::decode(&grown(&config)).unwrap(), original);
    assert_eq!(
        RegServerConfig::decode(&grow_at(&config, &[2])).unwrap(),
        original
    );

    let host = fixture("user-mutations/host-config-open.snowp");
    let original = HostConfig::decode(&host).unwrap();
    assert_eq!(original.encoded().unwrap(), host);
    for path in [&[][..], &[0], &[1]] {
        assert_eq!(HostConfig::decode(&grow_at(&host, path)).unwrap(), original);
    }

    let nag = fixture("user/user-device-nag-one.snowp");
    let original = DeviceNagInfo::decode(&nag).unwrap();
    assert_eq!(original.encoded().unwrap(), nag);
    assert_eq!(DeviceNagInfo::decode(&grown(&nag)).unwrap(), original);

    let info = ServerClientVersionInfo {
        minimum: Some(foks_proto::SemVer {
            major: 0,
            minor: 1,
            patch: 9,
        }),
        newest: None,
        message: b"upgrade".to_vec(),
    };
    let exact = info.encoded().unwrap();
    assert_eq!(
        ServerClientVersionInfo::decode(&grown(&exact)).unwrap(),
        info
    );
    assert_eq!(
        ServerClientVersionInfo::decode(&grow_at(&exact, &[0])).unwrap(),
        info
    );
}

#[test]
fn sso_arguments_and_results_accept_appended_fields() {
    let config = fixture("sso/config.snowp");
    let original = SsoConfig::decode(&config).unwrap();
    assert_eq!(original.encoded().unwrap(), config);
    assert_eq!(SsoConfig::decode(&grown(&config)).unwrap(), original);
    assert_eq!(
        SsoConfig::decode(&grow_at(&config, &[1])).unwrap(),
        original
    );

    let init = fixture("sso/init-login.snowp");
    let original = InitOAuth2SessionArgument::decode(&init).unwrap();
    assert_eq!(original.encoded().unwrap(), init);
    assert_eq!(
        InitOAuth2SessionArgument::decode(&grown(&init)).unwrap(),
        original
    );
    let poll = fixture("sso/poll.snowp");
    let original = PollOAuth2SessionArgument::decode(&poll).unwrap();
    assert_eq!(original.encoded().unwrap(), poll);
    assert_eq!(
        PollOAuth2SessionArgument::decode(&grown(&poll)).unwrap(),
        original
    );
    let login = fixture("sso/login.snowp");
    let original = SsoLoginArgument::decode(&login).unwrap();
    assert_eq!(original.encoded().unwrap(), login);
    assert_eq!(SsoLoginArgument::decode(&grown(&login)).unwrap(), original);
    // The signed binding inside the login argument stays exact.
    let binding = fixture("sso/signed-binding.snowp");
    foks_proto::OAuth2IdTokenBinding::decode(&binding).unwrap();
    assert!(foks_proto::OAuth2IdTokenBinding::decode(&grown(&binding)).is_err());
}

#[test]
fn invitation_results_and_arguments_accept_appended_fields() {
    let inbox = fixture("invitations/local.inbox");
    let rows = decode_team_inbox(&inbox).unwrap();
    assert_eq!(encode_team_inbox(&rows).unwrap(), inbox);
    assert_eq!(decode_team_inbox(&grown(&inbox)).unwrap(), rows);
    // Each row grows too.
    assert_eq!(decode_team_inbox(&grow_at(&inbox, &[0, 0])).unwrap(), rows);

    let argument = ChangeUsernameArgument {
        username: "alice".to_owned(),
        full: None,
    };
    let exact = argument.encoded().unwrap();
    assert_eq!(
        ChangeUsernameArgument::decode(&grown(&exact)).unwrap(),
        argument
    );
}
