//! Pegged Reply, Reaction and Edit messages as go-foks main clients send them
//! (bf9ace5). The Rust server accepts only Basic sends, so the transport
//! rewrites stored Basic messages into pegged ones on the way to the reader,
//! sealed exactly as Go's sealMessage seals them.
use foks_client::{
    ChatContent, ChatPeggedAction, ChatPreviewContent, ChatTransport, EncryptedFileMutationStore,
    NamedTeamSecrets, RealtimeConnection,
};
use foks_client_db::{ChatOperationState as State, SoftStateStore};
use foks_crypto::RealtimeKeys;
use foks_proto::*;
use foks_rpc::{RealtimeRequest as Request, RealtimeResponse as Response};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};
use std::collections::BTreeMap;

struct PeggedRewrite {
    connection: RealtimeConnection,
    keys: RealtimeKeys,
    sender: FqParty,
    team: FqParty,
    channel: RtChannelId,
    /// Sequence -> (type, body, target) to present instead of the stored message.
    pegged: BTreeMap<u64, (RtMessageType, Vec<u8>, RtMessageId)>,
}
impl PeggedRewrite {
    fn rewrite(&self, message: &mut RtMessage) {
        let Some((kind, body, target)) = self.pegged.get(&message.sequence) else {
            return;
        };
        message.metadata.kind = *kind;
        let noncer = RtMessageNoncer {
            metadata: message.metadata.clone(),
            sender: Some(self.sender.clone()),
            app: RtAppId::Chat,
            team: self.team.clone(),
            channel: self.channel,
        };
        let RtMessageWrapper::Encrypted(boxed) = &mut message.wrapper else {
            panic!("encrypted message")
        };
        boxed.ciphertext = self
            .keys
            .seal_message(
                &noncer,
                &RtMessageBody::Pegged {
                    kind: *kind,
                    body: body.clone(),
                    reply_to: *target,
                },
            )
            .unwrap();
    }
}
impl ChatTransport for PeggedRewrite {
    fn request(&mut self, request: &Request) -> foks_client::Result<Response> {
        let mut response = self.connection.call(request)?;
        match &mut response {
            Response::Thread(page) => {
                for list in &mut page.ranges {
                    list.messages.iter_mut().for_each(|m| self.rewrite(m));
                }
                page.sequences.iter_mut().for_each(|m| self.rewrite(m));
            }
            Response::Messages(list) => list.messages.iter_mut().for_each(|m| self.rewrite(m)),
            _ => {}
        }
        Ok(response)
    }
}

#[test]
fn pegged_messages_render_as_placeholders_without_breaking_continuity() {
    let environment = TestEnvironment::new().unwrap();
    let _server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "pegged-reader").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let owner = client
        .create_account(&probe.pinned, &TestAccountSpec::new("peggedowner", 0x73))
        .unwrap();
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([0x31; 32]),
        member: SecretSeed::new([0x32; 32]),
        admin: SecretSeed::new([0x33; 32]),
        owner: SecretSeed::new([0x34; 32]),
        removal_key: SecretSeed::new([0x35; 32]),
        team_name_commitment_key: [0x36; 16],
    };
    let team = client
        .foks()
        .create_single_owner_named_team(&probe.pinned, &owner.credential, "peggedtest", &secrets)
        .unwrap();
    let mut chat = client
        .foks()
        .chat_session(&probe.pinned, &owner.credential, &team.team)
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut protected =
        EncryptedFileMutationStore::open(directory.path(), zeroize::Zeroizing::new([7; 32]))
            .unwrap();
    let mut connection = chat.connection().unwrap();
    let prepared = chat
        .prepare_channel(
            &mut connection,
            &mut protected,
            "pegged",
            "test channel",
            RtChannelTier::Bottom,
        )
        .unwrap();
    let channel = RtChannelId(
        chat.attempt_operation(&mut connection, &mut protected, &prepared.id)
            .unwrap()
            .scope
            .channel,
    );
    let root = chat
        .prepare_send(&mut connection, &mut protected, channel, "root")
        .unwrap();
    chat.attempt_operation(&mut connection, &mut protected, &root.id)
        .unwrap();
    let root_id = RtMessageId(root.id);
    let channels = chat.list_channels(&mut connection).unwrap();
    let md = channels.channels[0].metadata.clone();
    let seed = if md.roles.read == Role::member(0) {
        &secrets.member
    } else {
        &secrets.member_min
    };
    let keys = foks_crypto::derive_realtime_keys(seed, RtAppId::Chat).unwrap();
    let sender =
        FqParty::new(owner.credential.uid.clone(), probe.pinned.host_id().clone()).unwrap();
    let team_party = FqParty::new(team.team.clone(), probe.pinned.host_id().clone()).unwrap();
    // Four more stored messages, each naming its predecessor.
    let mut previous = (1, root_id);
    for tag in 2..=5u8 {
        let metadata = RtMessageMetadata {
            id: RtMessageId([0x50 + tag; 16]),
            previous_id: previous.1,
            previous_sequence: previous.0,
            send_time: 1_700_000_000_000 + tag as u64,
            kind: RtMessageType::Basic,
            further_user_attribution: None,
        };
        let noncer = RtMessageNoncer {
            metadata: metadata.clone(),
            sender: Some(sender.clone()),
            app: RtAppId::Chat,
            team: team_party.clone(),
            channel,
        };
        let Response::Sent(receipt) = connection
            .call(&Request::Send(RtSendArgument {
                send: RtSend {
                    metadata: metadata.clone(),
                    channel: channel.short(),
                    wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                        key: RoleAndGeneration {
                            role: md.roles.read,
                            generation: 1,
                        },
                        ciphertext: keys.seal_basic_message(&noncer, b"stored").unwrap(),
                    }),
                    expected_previous_sequence: 0,
                },
            }))
            .unwrap()
        else {
            panic!("send receipt")
        };
        assert_eq!(receipt.sequence, tag as u64);
        previous = (receipt.sequence, metadata.id);
    }
    let mut rpc = PeggedRewrite {
        connection,
        keys,
        sender,
        team: team_party,
        channel,
        pegged: BTreeMap::from([
            (2, (RtMessageType::Reply, b"a reply".to_vec(), root_id)),
            (
                3,
                (
                    RtMessageType::Reaction,
                    "{\"emoji\":\"👍\"}".as_bytes().to_vec(),
                    root_id,
                ),
            ),
            (4, (RtMessageType::Edit, b"root (edited)".to_vec(), root_id)),
            (5, (RtMessageType::Edit, Vec::new(), root_id)),
        ]),
    };

    let history = chat.read_recent(&mut rpc, channel, 10).unwrap();
    assert!(history.missing_predecessors.is_empty());
    let shown: Vec<_> = history
        .messages
        .iter()
        .map(|m| {
            (
                m.message.sequence,
                match &m.content {
                    ChatContent::Text(text) => format!("text:{}", text.as_str()),
                    ChatContent::Pegged { action, target } => {
                        assert_eq!(*target, root_id);
                        format!("{action:?}")
                    }
                    ChatContent::Unsupported(kind) => format!("unsupported:{kind:?}"),
                },
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            (5, "Delete".to_owned()),
            (4, "Edit".to_owned()),
            (3, "Reaction".to_owned()),
            (2, "Reply".to_owned()),
            (1, "text:root".to_owned()),
        ]
    );
    assert!(history
        .messages
        .iter()
        .all(|m| m.content.is_authenticated()));

    let mut soft = SoftStateStore::open(client.soft_state_path()).unwrap();
    let synced = chat.sync_inbox(&mut rpc, &mut soft).unwrap();
    let conversation = synced
        .inbox
        .conversations
        .iter()
        .find(|c| c.channel.metadata.id == channel)
        .unwrap();
    assert_eq!(
        conversation.preview.as_ref().unwrap().content,
        ChatPreviewContent::Pegged {
            action: ChatPeggedAction::Delete,
            target: root_id,
        }
    );

    // A pegged latest message is a verified predecessor for a new send.
    let next = chat
        .prepare_send(&mut rpc, &mut protected, channel, "after pegged")
        .unwrap();
    let confirmed = chat
        .attempt_operation(&mut rpc, &mut protected, &next.id)
        .unwrap();
    assert_eq!(confirmed.state, State::Confirmed);
    let latest = chat.read_recent(&mut rpc, channel, 1).unwrap();
    assert_eq!(latest.messages[0].message.sequence, 6);
    assert_eq!(latest.messages[0].message.metadata.previous_sequence, 5);
    assert_eq!(
        latest.messages[0].message.metadata.previous_id,
        RtMessageId([0x55; 16])
    );
}
