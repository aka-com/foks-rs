use crate::support::Fixture;
use foks_client::{AddLocalTeamMemberRequest, ChatTransport, NamedTeamSecrets, RealtimeConnection};
use foks_proto::*;
use foks_rpc::{RealtimeRequest as Request, RealtimeResponse as Response};
use foks_server_testkit::{TestAccountSpec, TestClient};

struct EmptyFirstInboxPage<'a> {
    connection: &'a mut RealtimeConnection,
    injected: bool,
}
impl ChatTransport for EmptyFirstInboxPage<'_> {
    fn request(&mut self, request: &Request) -> foks_client::Result<Response> {
        let mut response = self.connection.call(request)?;
        if !self.injected && matches!(request, Request::GetChangedThreads(_)) {
            self.injected = true;
            let Response::InboxDelta(delta) = &mut response else {
                panic!()
            };
            delta.channels.clear();
        }
        Ok(response)
    }
}

struct CountChangedThreads<'a> {
    connection: &'a mut RealtimeConnection,
    requests: usize,
}
impl ChatTransport for CountChangedThreads<'_> {
    fn request(&mut self, request: &Request) -> foks_client::Result<Response> {
        if matches!(request, Request::GetChangedThreads(_)) {
            self.requests += 1;
        }
        self.connection.call(request)
    }
}

struct PreviewFault<'a> {
    connection: &'a mut RealtimeConnection,
    corrupt: bool,
    requests: usize,
}
impl ChatTransport for PreviewFault<'_> {
    fn request(&mut self, request: &Request) -> foks_client::Result<Response> {
        self.requests += 1;
        if matches!(request, Request::GetThread(_)) {
            return Err(if self.corrupt {
                foks_client::Error::ChatIntegrity("injected corrupt preview")
            } else {
                foks_client::Error::DeadlineExceeded
            });
        }
        self.connection.call(request)
    }
}

struct FailedRead<'a>(&'a mut RealtimeConnection);
impl ChatTransport for FailedRead<'_> {
    fn request(&mut self, request: &Request) -> foks_client::Result<Response> {
        if matches!(request, Request::ReadThrough(_)) {
            return Err(foks_client::Error::DeadlineExceeded);
        }
        self.0.call(request)
    }
}

#[test]
pub(crate) fn realtime_capabilities() {
    let fixture = Fixture::start("realtime-capabilities");
    let account = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("rtcaps", 0x39))
        .unwrap();
    let mut connection = fixture
        .client
        .foks()
        .realtime_connection(&fixture.probe.pinned, &account.credential)
        .unwrap();
    let host = RtHostId::new(fixture.probe.pinned.host_id().clone()).unwrap();
    assert_eq!(
        connection
            .call(&Request::ChatCapabilities(RtChatCapabilitiesArgument {
                host: host.clone()
            }))
            .unwrap(),
        Response::ChatCapabilities(RtChatCapabilities::basic_only(host))
    );
    let mut other = fixture.probe.pinned.host_id().as_bytes().to_vec();
    other[1] ^= 1;
    let wrong = Request::ChatCapabilities(RtChatCapabilitiesArgument {
        host: RtHostId::new(EntityId::from_bytes(other).unwrap()).unwrap(),
    });
    assert!(matches!(
        connection.call(&wrong),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: foks_rpc::STATUS_PERMISSION_ERROR,
            ..
        }))
    ));
    assert_eq!(
        connection
            .call(&Request::GetInboxVersion(RtGetInboxVersionArgument {
                key: RtInboxKey { app: RtAppId::Chat }
            }))
            .unwrap(),
        Response::InboxVersion(0)
    );
}

#[test]
pub(crate) fn realtime_poll_capacity_is_separate_and_bounded() {
    let environment = foks_server_testkit::TestEnvironment::with_profile(
        foks_server_testkit::TestProfile::RealtimePollCapacity,
    )
    .unwrap();
    let server = environment.start_server().unwrap();
    let client =
        foks_server_testkit::TestClient::new(&environment, "realtime-poll-capacity").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let fixture = Fixture {
        environment,
        server,
        client,
        probe,
    };
    let account = fixture
        .client
        .create_account(
            fixture.host(),
            &TestAccountSpec::new("rtpollcapacity", 0x30),
        )
        .unwrap();
    let mut pollers = Vec::new();
    for _ in 0..4 {
        pollers.push(
            fixture
                .client
                .foks()
                .realtime_connection(&fixture.probe.pinned, &account.credential)
                .unwrap(),
        );
    }
    let mut ordinary = fixture
        .client
        .foks()
        .realtime_connection(&fixture.probe.pinned, &account.credential)
        .unwrap();
    let mut overflow = fixture
        .client
        .foks()
        .realtime_connection(&fixture.probe.pinned, &account.credential)
        .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(5));
    let waiting = pollers
        .into_iter()
        .map(|mut poller| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                poller.call(&Request::PollInbox(RtPollInboxArgument {
                    poll: RtPollInbox {
                        app: RtAppId::Chat,
                        since: 0,
                        timeout_milliseconds: 3_000,
                    },
                }))
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while fixture.server.metrics().active_realtime_polls != 4 {
        assert!(
            std::time::Instant::now() < deadline,
            "all realtime poll permits were not acquired before the readiness deadline"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        ordinary
            .call(&Request::GetInboxVersion(RtGetInboxVersionArgument {
                key: RtInboxKey { app: RtAppId::Chat },
            }))
            .unwrap(),
        Response::InboxVersion(0)
    );
    let overflow_result = overflow.call(&Request::PollInbox(RtPollInboxArgument {
        poll: RtPollInbox {
            app: RtAppId::Chat,
            since: 0,
            timeout_milliseconds: 500,
        },
    }));
    assert!(
        matches!(
            &overflow_result,
            Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: 1012,
                ..
            }))
        ),
        "unexpected overflow result: {overflow_result:?}"
    );
    for waiter in waiting {
        let Response::PollResult(result) = waiter.join().unwrap().unwrap() else {
            panic!()
        };
        assert!(!result.bumped);
    }
    assert_eq!(fixture.server.metrics().active_realtime_polls, 0);
}

#[test]
pub(crate) fn realtime_text() {
    let fixture = Fixture::start("realtime-owner");
    let owner = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("rtowner", 0x31))
        .unwrap();
    let client = TestClient::new(&fixture.environment, "realtime-member").unwrap();
    let member_host = client.probe_and_pin().unwrap();
    let member = client
        .create_account(&member_host.pinned, &TestAccountSpec::new("rtmember", 0x32))
        .unwrap();
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([0x51; 32]),
        member: SecretSeed::new([0x52; 32]),
        admin: SecretSeed::new([0x53; 32]),
        owner: SecretSeed::new([0x54; 32]),
        removal_key: SecretSeed::new([0x55; 32]),
        team_name_commitment_key: [0x56; 16],
    };
    let created = fixture
        .client
        .foks()
        .create_single_owner_named_team(fixture.host(), &owner.credential, "rtteam", &secrets)
        .unwrap();
    let removal = SecretSeed::new([0x61; 32]);
    fixture
        .client
        .foks()
        .add_local_user_to_named_team(
            fixture.host(),
            &owner.credential,
            &created.team,
            &AddLocalTeamMemberRequest {
                target_user: &member.authenticated.verified,
                destination_role: Role::member(0),
                removal_key: &removal,
            },
        )
        .unwrap();
    let loaded = client
        .foks()
        .load_and_pin_team(
            &member_host.pinned,
            &member.credential,
            &member.authenticated.verified,
            &member.authenticated.puks,
            &created.team,
        )
        .unwrap();
    let member_key = loaded
        .ptks
        .iter()
        .find(|k| k.role == Role::member(0))
        .unwrap();
    let opener = foks_crypto::derive_realtime_keys(&member_key.seed, RtAppId::Chat).unwrap();
    let name_keys = foks_crypto::derive_realtime_keys(&secrets.member_min, RtAppId::Chat).unwrap();
    let data_keys = foks_crypto::derive_realtime_keys(&secrets.member, RtAppId::Chat).unwrap();
    let md = RtChannelMetadata {
        id: RtChannelId([0x62; 16]),
        team: RtTeamId::new(created.team.clone()).unwrap(),
        app: RtAppId::Chat,
        sequence: 1,
        name: RtBox {
            key: RoleAndGeneration {
                role: Role::member(-0x4000),
                generation: 1,
            },
            boxed: name_keys
                .seal_text(RtKeyType::ChannelName, &RtText("".into()), [1; 16])
                .unwrap(),
        },
        description: Some(RtBox {
            key: RoleAndGeneration {
                role: Role::member(0),
                generation: 1,
            },
            boxed: data_keys
                .seal_text(
                    RtKeyType::ChannelDescription,
                    &RtText("team chat".into()),
                    [2; 16],
                )
                .unwrap(),
        }),
        roles: RtRolePair {
            read: Role::member(0),
            write: Role::member(0),
        },
        last_message: None,
        ctime: 0,
        mtime: 0,
        updated_at: 1,
        tier: RtChannelTier::Bottom,
        unreadable: false,
    };
    let mut writer = fixture
        .client
        .foks()
        .realtime_connection(fixture.host(), &owner.credential)
        .unwrap();
    let mut reader = client
        .foks()
        .realtime_connection(&member_host.pinned, &member.credential)
        .unwrap();
    writer
        .call(&Request::CreateChannel(RtCreateChannelArgument {
            metadata: md.clone(),
            set_version: 1,
        }))
        .unwrap();
    let list = Request::ListChannels(RtListChannelsArgument {
        team: md.team.clone(),
        app: RtAppId::Chat,
        last: 0,
    });
    let Response::Channels(channels) = reader.call(&list).unwrap() else {
        panic!()
    };
    assert_eq!(channels.channels.len(), 1);
    assert_eq!(
        opener
            .open_text(
                RtKeyType::ChannelDescription,
                &channels.channels[0].description.as_ref().unwrap().boxed
            )
            .unwrap()
            .0,
        "team chat"
    );
    let mut noncer = RtMessageNoncer {
        metadata: RtMessageMetadata {
            id: RtMessageId([0x63; 16]),
            previous_id: RtMessageId([0; 16]),
            previous_sequence: 0,
            send_time: 1700000000000,
            kind: RtMessageType::Basic,
            further_user_attribution: None,
        },
        sender: Some(
            FqParty::new(
                owner.credential.uid.clone(),
                fixture.host().host_id().clone(),
            )
            .unwrap(),
        ),
        app: RtAppId::Chat,
        team: FqParty::new(created.team.clone(), fixture.host().host_id().clone()).unwrap(),
        channel: md.id,
    };
    if std::env::var_os("FOKS_GO_ORACLE_DIR").is_some() {
        let mut go_md = md.clone();
        go_md.id = RtChannelId([0x79; 16]);
        go_md.updated_at = 2;
        go_md.name.boxed = name_keys
            .seal_text(RtKeyType::ChannelName, &RtText("gochat".into()), [4; 16])
            .unwrap();
        let mut go_noncer = noncer.clone();
        go_noncer.channel = go_md.id;
        go_noncer.metadata.id = RtMessageId([0x78; 16]);
        go_roundtrip(
            &fixture,
            &owner.credential,
            &RtCreateChannelArgument {
                metadata: go_md,
                set_version: 2,
            },
            &go_noncer,
        );
    }
    let first = Request::Send(RtSendArgument {
        send: RtSend {
            metadata: noncer.metadata.clone(),
            channel: md.id.short(),
            wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                key: RoleAndGeneration {
                    role: Role::member(0),
                    generation: 1,
                },
                ciphertext: data_keys
                    .seal_basic_message(&noncer, b"hello teammate")
                    .unwrap(),
            }),
            expected_previous_sequence: 0,
        },
    });
    let receipt = writer.call(&first).unwrap();
    assert_eq!(writer.call(&first).unwrap(), receipt);
    let recent = Request::Recents(RtRecentsArgument {
        channel: md.id,
        stop_at: 0,
        limit: 100,
    });
    let Response::Messages(messages) = reader.call(&recent).unwrap() else {
        panic!()
    };
    assert_eq!(messages.messages.len(), 1);
    assert_eq!(messages.messages[0].sequence, 1);
    assert_eq!(
        messages.messages[0].sender.as_ref().unwrap().entity(),
        &owner.credential.uid
    );
    let RtMessageWrapper::Encrypted(boxed) = &messages.messages[0].wrapper else {
        panic!()
    };
    assert_eq!(
        opener
            .open_basic_message(&noncer, &boxed.ciphertext)
            .unwrap()
            .as_slice(),
        b"hello teammate"
    );
    let mut member_chat = client
        .foks()
        .chat_session(&member_host.pinned, &member.credential, &created.team)
        .unwrap();
    let soft_path = fixture
        .environment
        .client_path("realtime-member", "chat-soft.sqlite3")
        .unwrap();
    let mut soft = foks_client_db::SoftStateStore::open(&soft_path).unwrap();
    let synced = {
        let mut fallback = EmptyFirstInboxPage {
            connection: &mut reader,
            injected: false,
        };
        let synced = member_chat.sync_inbox(&mut fallback, &mut soft).unwrap();
        assert!(fallback.injected);
        synced
    };
    assert_eq!(
        synced
            .inbox
            .conversations
            .iter()
            .find(|conversation| conversation.channel.metadata.id == md.id)
            .unwrap()
            .unread,
        1
    );
    let mut preview_fault = PreviewFault {
        connection: &mut reader,
        corrupt: false,
        requests: 0,
    };
    let partial_preview = member_chat
        .sync_inbox(&mut preview_fault, &mut soft)
        .unwrap();
    assert!(partial_preview.inbox.previews_incomplete);
    assert_eq!(partial_preview.inbox.conversations[0].unread, 1);
    assert!(
        preview_fault.requests <= 5,
        "one-channel refresh must remain bounded"
    );
    preview_fault.corrupt = true;
    assert!(matches!(
        member_chat.sync_inbox(&mut preview_fault, &mut soft),
        Err(foks_client::Error::ChatIntegrity(_))
    ));
    assert!(member_chat
        .mark_read(&mut FailedRead(&mut reader), &mut soft, md.id, 1)
        .is_err());
    let inbox_scope = foks_client_db::ChatInboxScope {
        host: fixture.host().host_id().as_bytes().to_vec(),
        uid: member.credential.uid.as_bytes().to_vec(),
        app: RtAppId::Chat,
    };
    assert_eq!(
        soft.pending_chat_reads(&inbox_scope).unwrap(),
        vec![(md.id, 1)]
    );
    let partial = member_chat
        .sync_inbox(&mut FailedRead(&mut reader), &mut soft)
        .unwrap();
    assert!(partial.inbox.read_retry_pending);
    assert!(!partial.inbox.channels.is_empty());
    assert!(partial
        .inbox
        .conversations
        .iter()
        .any(|c| c.channel.metadata.id == md.id));
    assert_eq!(
        member_chat
            .retry_pending_reads(&mut reader, &mut soft)
            .unwrap(),
        1
    );
    assert_eq!(
        member_chat
            .inbox_from_store(&soft)
            .unwrap()
            .conversations
            .iter()
            .find(|conversation| conversation.channel.metadata.id == md.id)
            .unwrap()
            .unread,
        0
    );
    let Response::InboxVersion(unchanged_head) = reader
        .call(&Request::GetInboxVersion(RtGetInboxVersionArgument {
            key: RtInboxKey { app: RtAppId::Chat },
        }))
        .unwrap()
    else {
        panic!()
    };
    soft.observe_chat_inbox_head(&inbox_scope, unchanged_head, true)
        .unwrap();
    let mut unchanged_degraded = CountChangedThreads {
        connection: &mut reader,
        requests: 0,
    };
    let resumed = member_chat
        .sync_inbox(&mut unchanged_degraded, &mut soft)
        .unwrap();
    assert_eq!(unchanged_degraded.requests, 0);
    assert!(resumed.inbox.degraded);
    noncer.metadata.previous_id = noncer.metadata.id;
    noncer.metadata.previous_sequence = 1;
    noncer.metadata.id = RtMessageId([0x64; 16]);
    noncer.sender = Some(
        FqParty::new(
            member.credential.uid.clone(),
            fixture.host().host_id().clone(),
        )
        .unwrap(),
    );
    let second = Request::Send(RtSendArgument {
        send: RtSend {
            metadata: noncer.metadata.clone(),
            channel: md.id.short(),
            wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                key: RoleAndGeneration {
                    role: Role::member(0),
                    generation: 1,
                },
                ciphertext: opener.seal_basic_message(&noncer, b"hello back").unwrap(),
            }),
            expected_previous_sequence: 1,
        },
    });
    let Response::Sent(second_receipt) = reader.call(&second).unwrap() else {
        panic!()
    };
    assert_eq!(second_receipt.sequence, 2);
    let history = Request::GetThread(RtGetThreadArgument {
        query: RtThreadQuery {
            channel: md.id,
            ranges: vec![RtThreadRange { start: 2, end: 1 }],
            sequences: vec![2, 900],
        },
    });
    // Reader-side setup and refresh may outlive the server's idle timeout.
    // Fetch history over a fresh authenticated transport.
    writer = fixture
        .client
        .foks()
        .realtime_connection(fixture.host(), &owner.credential)
        .unwrap();
    let Response::Thread(page) = writer.call(&history).unwrap() else {
        panic!()
    };
    let mut bounded = fixture
        .client
        .foks()
        .chat_session(fixture.host(), &owner.credential, &created.team)
        .unwrap();
    bounded.limit_history_bytes(1).unwrap();
    assert!(matches!(
        bounded.read_recent(&mut writer, md.id, 2),
        Err(foks_client::Error::ChatLimit(_))
    ));
    bounded.limit_history_bytes(512 * 1024).unwrap();
    assert_eq!(
        bounded
            .read_recent(&mut writer, md.id, 2)
            .unwrap()
            .messages
            .len(),
        2
    );

    assert_eq!(page.ranges[0].messages.len(), 2);
    assert_eq!(page.sequences.len(), 1);
    let RtMessageWrapper::Encrypted(boxed) = &page.sequences[0].wrapper else {
        panic!()
    };
    assert_eq!(
        data_keys
            .open_basic_message(&noncer, &boxed.ciphertext)
            .unwrap()
            .as_slice(),
        b"hello back"
    );
    let Response::InboxVersion(inbox_version) = reader
        .call(&Request::GetInboxVersion(RtGetInboxVersionArgument {
            key: RtInboxKey { app: RtAppId::Chat },
        }))
        .unwrap()
    else {
        panic!()
    };
    assert!(inbox_version >= 3);
    let Response::InboxDelta(delta) = reader
        .call(&Request::GetChangedThreads(RtGetChangedThreadsArgument {
            query: RtChangedThreads {
                app: RtAppId::Chat,
                since: 0,
                maximum: 100,
            },
        }))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(delta.inbox_version, inbox_version);
    assert_eq!(
        delta
            .channels
            .iter()
            .find(|channel| channel.metadata.id == md.id)
            .unwrap()
            .read_through,
        2
    );
    reader
        .call(&Request::ReadThrough(RtReadThroughArgument {
            read: RtReadThrough {
                channel: md.id,
                sequence: 2,
            },
        }))
        .unwrap();
    let Response::PollResult(poll) = reader
        .call(&Request::PollInbox(RtPollInboxArgument {
            poll: RtPollInbox {
                app: RtAppId::Chat,
                since: 0,
                timeout_milliseconds: 1,
            },
        }))
        .unwrap()
    else {
        panic!()
    };
    assert!(poll.bumped);
    assert_eq!(poll.inbox_version, inbox_version);
    let Response::PollResult(poll) = reader
        .call(&Request::PollInbox(RtPollInboxArgument {
            poll: RtPollInbox {
                app: RtAppId::Chat,
                since: inbox_version,
                timeout_milliseconds: 1,
            },
        }))
        .unwrap()
    else {
        panic!()
    };
    assert!(!poll.bumped);
    assert_eq!(poll.inbox_version, inbox_version);
    let mut poller = client
        .foks()
        .realtime_connection(&member_host.pinned, &member.credential)
        .unwrap();
    let waiting = std::thread::spawn(move || {
        poller
            .call(&Request::PollInbox(RtPollInboxArgument {
                poll: RtPollInbox {
                    app: RtAppId::Chat,
                    since: inbox_version,
                    timeout_milliseconds: 5_000,
                },
            }))
            .unwrap()
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut third_noncer = noncer.clone();
    third_noncer.metadata.previous_id = third_noncer.metadata.id;
    third_noncer.metadata.previous_sequence = 2;
    third_noncer.metadata.id = RtMessageId([0x65; 16]);
    third_noncer.sender = Some(
        FqParty::new(
            owner.credential.uid.clone(),
            fixture.host().host_id().clone(),
        )
        .unwrap(),
    );
    writer
        .call(&Request::Send(RtSendArgument {
            send: RtSend {
                metadata: third_noncer.metadata.clone(),
                channel: md.id.short(),
                wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                    key: RoleAndGeneration {
                        role: Role::member(0),
                        generation: 1,
                    },
                    ciphertext: data_keys
                        .seal_basic_message(&third_noncer, b"wake poller")
                        .unwrap(),
                }),
                expected_previous_sequence: 2,
            },
        }))
        .unwrap();
    let Response::PollResult(poll) = waiting.join().unwrap() else {
        panic!()
    };
    assert!(poll.bumped);
    assert_eq!(poll.inbox_version, inbox_version + 1);
    // A committed member removal must be honored on an already-open RT
    // connection, and exact old-generation replay remains available to owner.
    let rotated_min = SecretSeed::new([0x71; 32]);
    let rotated_member = SecretSeed::new([0x72; 32]);
    let rotations = [
        foks_client::TeamPtkRotationSeed {
            role: Role::member(-0x4000),
            seed: &rotated_min,
        },
        foks_client::TeamPtkRotationSeed {
            role: Role::member(0),
            seed: &rotated_member,
        },
    ];
    assert_eq!(
        member_chat
            .read_thread(&mut reader, md.id, 1, 2)
            .unwrap()
            .messages
            .len(),
        2
    );
    let removal_since = poll.inbox_version;
    let mut removal_poller = client
        .foks()
        .realtime_connection(&member_host.pinned, &member.credential)
        .unwrap();
    let removal_wait = std::thread::spawn(move || {
        removal_poller
            .call(&Request::PollInbox(RtPollInboxArgument {
                poll: RtPollInbox {
                    app: RtAppId::Chat,
                    since: removal_since,
                    timeout_milliseconds: 5_000,
                },
            }))
            .unwrap()
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut protected = fixture.client.open_protected_store().unwrap();
    fixture
        .client
        .foks()
        .remove_team_member_and_rotate_ptks(
            fixture.host(),
            &owner.credential,
            &created.team,
            &foks_client::RemoveTeamMemberRequest {
                target: member.authenticated.verified.uid(),
                rotations: &rotations,
                remaining_users: &[],
            },
            &mut protected,
        )
        .unwrap();
    let Response::PollResult(removal_poll) = removal_wait.join().unwrap() else {
        panic!()
    };
    assert!(removal_poll.bumped);
    assert!(removal_poll.inbox_version > removal_since);
    assert!(matches!(
        reader.call(&recent),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1013,
            ..
        }))
    ));
    assert!(matches!(
        reader.call(&second),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1013,
            ..
        }))
    ));
    assert!(member_chat.read_thread(&mut reader, md.id, 1, 2).is_err());
    let mut owner_chat = fixture
        .client
        .foks()
        .chat_session(fixture.host(), &owner.credential, &created.team)
        .unwrap();
    assert_eq!(
        owner_chat
            .read_thread(&mut writer, md.id, 1, 2)
            .unwrap()
            .messages
            .len(),
        2
    );
    assert_eq!(writer.call(&first).unwrap(), receipt);
    // Restart with exact ciphertext and receipts intact. A new connection must
    // select the host again; no in-memory chat state is required for recovery.
    drop(writer);
    drop(reader);
    fixture.client.foks().clear_connection_pool().unwrap();
    client.foks().clear_connection_pool().unwrap();
    fixture.server.shutdown().unwrap();
    let restarted = fixture.environment.start_server().unwrap();
    let mut writer = fixture
        .client
        .foks()
        .realtime_connection(&fixture.probe.pinned, &owner.credential)
        .unwrap();
    assert_eq!(writer.call(&first).unwrap(), receipt);
    let Response::Messages(messages) = writer.call(&recent).unwrap() else {
        panic!()
    };
    assert_eq!(messages.messages.len(), 3);
    drop(writer);
    drop(restarted);
}

fn go_roundtrip(
    fixture: &Fixture,
    credential: &foks_client::DeviceCredential,
    create: &RtCreateChannelArgument,
    noncer: &RtMessageNoncer,
) {
    let oracle = std::env::var_os("FOKS_GO_ORACLE_DIR").unwrap();
    let key_path = fixture
        .environment
        .client_path("go-realtime", "key.der")
        .unwrap();
    let root = key_path.parent().unwrap();
    let write = |name: &str, bytes: &[u8]| std::fs::write(root.join(name), bytes).unwrap();
    write(
        "key.der",
        &foks_crypto::device_signing_key_pkcs8(&credential.seed).unwrap(),
    );
    for (i, cert) in credential.certificate_chain.iter().enumerate() {
        write(&format!("certificate-{i}.der"), cert);
    }
    write("ca.der", &fixture.host().tls_ca_certificates()[0]);
    write("create.snowp", &create.encoded().unwrap());
    write("noncer.snowp", &noncer.encoded().unwrap());
    let output = std::process::Command::new("go")
        .args(["test", "-C"])
        .arg(oracle)
        .args([
            "-mod=readonly",
            "-run",
            "^TestGoRealtimeAgainstRustServer$",
            "-count=1",
            "-timeout=30s",
            "-v",
        ])
        .env("FOKS_RT_LIVE_DIR", root)
        .env(
            "FOKS_RT_LIVE_ADDRESS",
            fixture.server.addresses().authenticated.to_string(),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Go RT test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn client_chat_recovers_original_operations_after_lost_responses() {
    use foks_client::{ChatContent, ChatTransport, EncryptedFileMutationStore};
    use foks_client_db::ChatOperationState as State;
    let fixture = Fixture::start("chat-recovery");
    let owner = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("chatowner", 0x41))
        .unwrap();
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([0x51; 32]),
        member: SecretSeed::new([0x52; 32]),
        admin: SecretSeed::new([0x53; 32]),
        owner: SecretSeed::new([0x54; 32]),
        removal_key: SecretSeed::new([0x55; 32]),
        team_name_commitment_key: [0x56; 16],
    };
    let created = fixture
        .client
        .foks()
        .create_single_owner_named_team(fixture.host(), &owner.credential, "chatrecover", &secrets)
        .unwrap();
    let cancellation = foks_client::CancellationToken::new();
    cancellation.cancel();
    let offline_client = fixture.client.foks().with_cancellation_token(cancellation);
    let offline = offline_client
        .chat_session(fixture.host(), &owner.credential, &created.team)
        .unwrap();
    assert!(offline.list_pending().unwrap().is_empty());
    assert!(offline.connection().is_err());
    let mut chat = fixture
        .client
        .foks()
        .chat_session(fixture.host(), &owner.credential, &created.team)
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut protected =
        EncryptedFileMutationStore::open(dir.path(), zeroize::Zeroizing::new([9; 32])).unwrap();
    struct Faults {
        conn: foks_client::RealtimeConnection,
        drop_create: bool,
        drop_send: bool,
        channel: RtChannelId,
        sender: FqParty,
        team: FqParty,
        corrupt_time: bool,
        unsupported_kind: bool,
        hide_exact: bool,
        hide_create: bool,
        drop_before_send: bool,
        send_calls: usize,
        reject_next: bool,
    }
    impl ChatTransport for Faults {
        fn request(&mut self, req: &Request) -> foks_client::Result<Response> {
            if let Request::Send(arg) = req {
                self.send_calls += 1;
                if self.reject_next {
                    self.reject_next = false;
                    let mut rejected = arg.clone();
                    rejected.send.expected_previous_sequence = i64::MAX as u64;
                    return self.conn.call(&Request::Send(rejected));
                }
                if self.drop_before_send {
                    self.drop_before_send = false;
                    return Err(foks_client::Error::DeadlineExceeded);
                }
                if self.drop_send {
                    // Advance a busy channel beyond one recovery page before the target.
                    let keys = foks_crypto::derive_realtime_keys(
                        &SecretSeed::new([0x52; 32]),
                        RtAppId::Chat,
                    )
                    .unwrap();
                    for tag in 1..=101u8 {
                        let mut send = arg.send.clone();
                        send.metadata.id = RtMessageId([tag; 16]);
                        let nonce = RtMessageNoncer {
                            metadata: send.metadata.clone(),
                            sender: Some(self.sender.clone()),
                            team: self.team.clone(),
                            channel: self.channel,
                            app: RtAppId::Chat,
                        };
                        let RtMessageWrapper::Encrypted(b) = &mut send.wrapper else {
                            panic!()
                        };
                        let body = if tag <= 9 {
                            vec![b'x'; RT_MAX_BODY_BYTES - 100]
                        } else {
                            b"intervening message".to_vec()
                        };
                        b.ciphertext = keys.seal_basic_message(&nonce, &body).unwrap();
                        self.conn.call(&Request::Send(RtSendArgument { send }))?;
                    }
                    self.drop_send = false;
                    self.conn.call(req)?;
                    return Err(foks_client::Error::DeadlineExceeded);
                }
            }
            let mut res = self.conn.call(req)?;
            if self.hide_exact {
                if let Response::Thread(page) = &mut res {
                    page.sequences.clear();
                }
            }
            if self.hide_create {
                if let Response::Channels(set) = &mut res {
                    for c in &mut set.channels {
                        c.id = RtChannelId([0xab; 16]);
                    }
                }
            }
            if matches!(req, Request::CreateChannel(_)) && self.drop_create {
                self.drop_create = false;
                return Err(foks_client::Error::DeadlineExceeded);
            }
            if self.unsupported_kind {
                if let Response::Thread(page) = &mut res {
                    if let Some(m) = page.ranges.first_mut().and_then(|r| r.messages.first_mut()) {
                        m.metadata.kind = RtMessageType::Edit;
                    }
                }
            }
            if self.corrupt_time {
                if let Response::Thread(page) = &mut res {
                    if let Some(m) = page.ranges.first_mut().and_then(|r| r.messages.first_mut()) {
                        m.insert_time += 1;
                    }
                }
            }
            Ok(res)
        }
    }
    let mut rpc = Faults {
        conn: chat.connection().unwrap(),
        drop_create: true,
        drop_send: false,
        channel: RtChannelId([0; 16]),
        sender: FqParty {
            host: fixture.host().host_id().clone(),
            party: owner.credential.uid.clone(),
        },
        team: FqParty {
            host: fixture.host().host_id().clone(),
            party: created.team.clone(),
        },
        corrupt_time: false,
        unsupported_kind: false,
        hide_exact: false,
        hide_create: false,
        drop_before_send: false,
        send_calls: 0,
        reject_next: false,
    };
    struct FailAfterPut<'a>(&'a mut EncryptedFileMutationStore);
    impl foks_client::ProtectedMutationStore for FailAfterPut<'_> {
        fn put_if_absent(
            &mut self,
            key: &[u8],
            bytes: &[u8],
        ) -> Result<(), foks_client::ProtectedStoreError> {
            foks_client::ProtectedMutationStore::put_if_absent(self.0, key, bytes)?;
            Err(foks_client::ProtectedStoreError::Backend(
                "crash after protected commit".into(),
            ))
        }
        fn get(
            &mut self,
            key: &[u8],
        ) -> Result<zeroize::Zeroizing<Vec<u8>>, foks_client::ProtectedStoreError> {
            foks_client::ProtectedMutationStore::get(self.0, key)
        }
        fn remove(&mut self, key: &[u8]) -> Result<(), foks_client::ProtectedStoreError> {
            foks_client::ProtectedMutationStore::remove(self.0, key)
        }
    }
    assert!(chat
        .prepare_channel(
            &mut rpc,
            &mut FailAfterPut(&mut protected),
            "orphan",
            "",
            RtChannelTier::Bottom
        )
        .is_err());
    assert!(chat.list_pending().unwrap().is_empty());
    let create = chat
        .prepare_channel(
            &mut rpc,
            &mut protected,
            "  Recovery  ",
            "test channel",
            RtChannelTier::Bottom,
        )
        .unwrap();
    assert_eq!(
        chat.reconcile_operation(&mut rpc, &mut protected, &create.id)
            .unwrap(),
        create
    );
    assert!(chat.list_channels(&mut rpc).unwrap().channels.is_empty());
    assert_eq!(rpc.send_calls, 0);
    rpc.channel = RtChannelId(create.scope.channel);
    let mut wrong_key =
        EncryptedFileMutationStore::open(dir.path(), zeroize::Zeroizing::new([8; 32])).unwrap();
    assert!(chat
        .attempt_operation(&mut rpc, &mut wrong_key, &create.id)
        .is_err());
    assert_eq!(chat.list_pending().unwrap()[0].state, State::Prepared);
    assert!(chat
        .attempt_operation(&mut rpc, &mut protected, &create.id)
        .is_err());
    assert_eq!(chat.list_pending().unwrap()[0].state, State::Uncertain);
    rpc.hide_create = true;
    assert_eq!(
        chat.reconcile_operation(&mut rpc, &mut protected, &create.id)
            .unwrap()
            .state,
        State::Uncertain
    );
    rpc.hide_create = false;
    assert_eq!(
        chat.reconcile_operation(&mut rpc, &mut protected, &create.id)
            .unwrap()
            .state,
        State::Confirmed
    );
    let list = chat.list_channels(&mut rpc).unwrap();
    assert_eq!(list.channels[0].name.0, "recovery");
    let channel = rpc.channel;
    let pending = chat.prepare_send(&mut rpc, &mut protected, channel, "original message");
    let pending = pending.unwrap();
    let calls = rpc.send_calls;
    assert_eq!(
        chat.reconcile_operation(&mut rpc, &mut protected, &pending.id)
            .unwrap(),
        pending
    );
    assert_eq!(rpc.send_calls, calls);
    rpc.drop_send = true;
    assert!(chat
        .attempt_operation(&mut rpc, &mut protected, &pending.id)
        .is_err());
    drop(chat);
    drop(protected);
    if fixture.client.soft_state_path().exists() {
        std::fs::remove_file(fixture.client.soft_state_path()).unwrap();
    }
    // Both hard state and protected request are reopened, with no message cache.
    let mut chat = fixture
        .client
        .foks()
        .chat_session(fixture.host(), &owner.credential, &created.team)
        .unwrap();
    let mut protected =
        EncryptedFileMutationStore::open(dir.path(), zeroize::Zeroizing::new([9; 32])).unwrap();
    // A reopened client must not inherit the old process's idle connection.
    rpc.conn = chat.connection().unwrap();
    let first = chat
        .reconcile_operation(&mut rpc, &mut protected, &pending.id)
        .unwrap();
    assert_eq!(first.state, State::Uncertain);
    assert_eq!(first.scan_cursor, 6);
    let calls = rpc.send_calls;
    let second = chat
        .reconcile_operation(&mut rpc, &mut FailRemove(&mut protected), &pending.id)
        .unwrap();
    assert_eq!(second.state, State::Confirmed);
    assert_eq!(rpc.send_calls, calls);
    assert_eq!(chat.list_cleanup_pending().unwrap(), vec![second.clone()]);
    offline
        .finalize_operation(&mut protected, &pending.id)
        .unwrap();
    let receipt = RtSendResult::decode(second.receipt.as_ref().unwrap()).unwrap();
    assert_eq!(receipt.sequence, 102);
    let channel = rpc.channel;
    let history = chat.read_thread(&mut rpc, channel, 102, 100).unwrap();
    assert!(
        matches!(&history.messages[0].content,ChatContent::Text(text) if text.as_str()=="original message")
    );
    assert!(chat.list_pending().unwrap().is_empty());
    let reply = chat
        .prepare_send(&mut rpc, &mut protected, channel, "reply")
        .unwrap();
    assert_eq!(
        chat.attempt_operation(&mut rpc, &mut protected, &reply.id)
            .unwrap()
            .state,
        State::Confirmed
    );
    let fresh_client = TestClient::new(&fixture.environment, "chat-fresh-evidence").unwrap();
    let fresh_host = fresh_client.probe_and_pin().unwrap();
    let mut fresh = fresh_client
        .foks()
        .chat_session(&fresh_host.pinned, &owner.credential, &created.team)
        .unwrap();
    rpc.conn = fresh.connection().unwrap();
    rpc.hide_exact = true;
    assert_eq!(
        fresh
            .read_thread(&mut rpc, channel, 103, 103)
            .unwrap()
            .missing_predecessors,
        vec![102]
    );
    rpc.hide_exact = false;
    assert!(fresh
        .read_thread(&mut rpc, channel, 103, 103)
        .unwrap()
        .missing_predecessors
        .is_empty());
    rpc.unsupported_kind = true;
    assert!(chat.read_thread(&mut rpc, channel, 102, 100).is_err());
    rpc.unsupported_kind = false;
    rpc.corrupt_time = true;
    assert!(chat.read_thread(&mut rpc, channel, 102, 100).is_err());
    rpc.corrupt_time = false;
    let unsent = chat
        .prepare_send(
            &mut rpc,
            &mut protected,
            channel,
            "uncertain before transmission",
        )
        .unwrap();
    rpc.drop_before_send = true;
    assert!(chat
        .attempt_operation(&mut rpc, &mut protected, &unsent.id)
        .is_err());
    let calls = rpc.send_calls;
    assert_eq!(
        chat.reconcile_operation(&mut rpc, &mut protected, &unsent.id)
            .unwrap()
            .state,
        State::Uncertain
    );
    assert_eq!(
        rpc.send_calls, calls,
        "uncertainty must not cause a blind repost"
    );
    assert!(offline
        .cancel_prepared_operation(&mut protected, &unsent.id)
        .is_err());
    let count = std::fs::read_dir(dir.path()).unwrap().count();
    let cancelled = chat
        .prepare_send(&mut rpc, &mut protected, channel, "cancel me")
        .unwrap();
    assert_eq!(
        offline
            .cancel_prepared_operation(&mut protected, &cancelled.id)
            .unwrap()
            .state,
        State::Cancelled
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), count);
    let rejected = chat
        .prepare_send(&mut rpc, &mut protected, channel, "reject me")
        .unwrap();
    rpc.reject_next = true;
    assert!(chat
        .attempt_operation(&mut rpc, &mut protected, &rejected.id)
        .is_err());
    let rejected = offline
        .finalize_operation(&mut protected, &rejected.id)
        .unwrap();
    assert_eq!(rejected.state, State::Rejected);
    assert_eq!(rejected.rejection_code, Some(12006));
    struct FailRemove<'a>(&'a mut EncryptedFileMutationStore);
    impl foks_client::ProtectedMutationStore for FailRemove<'_> {
        fn put_if_absent(
            &mut self,
            k: &[u8],
            v: &[u8],
        ) -> Result<(), foks_client::ProtectedStoreError> {
            foks_client::ProtectedMutationStore::put_if_absent(self.0, k, v)
        }
        fn get(
            &mut self,
            k: &[u8],
        ) -> Result<zeroize::Zeroizing<Vec<u8>>, foks_client::ProtectedStoreError> {
            foks_client::ProtectedMutationStore::get(self.0, k)
        }
        fn remove(&mut self, _: &[u8]) -> Result<(), foks_client::ProtectedStoreError> {
            Err(foks_client::ProtectedStoreError::Backend(
                "cleanup interrupted".into(),
            ))
        }
    }
    let cancel_cleanup = chat
        .prepare_send(&mut rpc, &mut protected, channel, "cancel cleanup retry")
        .unwrap();
    let cancelled = offline
        .cancel_prepared_operation(&mut FailRemove(&mut protected), &cancel_cleanup.id)
        .unwrap();
    assert_eq!(cancelled.state, State::Cancelled);
    assert_eq!(offline.list_cleanup_pending().unwrap(), vec![cancelled]);
    offline
        .finalize_operation(&mut protected, &cancel_cleanup.id)
        .unwrap();
    let submission = foks_client_db::ChatSubmission {
        id: [0x73; 16],
        input_mac: [0x74; 32],
    };
    let cleanup = chat
        .prepare_send_submission(
            &mut rpc,
            &mut protected,
            channel,
            "cleanup retry",
            Some(&submission),
        )
        .unwrap();
    let confirmed = chat
        .attempt_operation(&mut rpc, &mut FailRemove(&mut protected), &cleanup.id)
        .unwrap();
    assert_eq!(confirmed.state, State::Confirmed);
    let calls = rpc.send_calls;
    assert_eq!(
        chat.reconcile_operation(&mut rpc, &mut FailRemove(&mut protected), &cleanup.id)
            .unwrap(),
        confirmed
    );
    assert_eq!(rpc.send_calls, calls);
    assert_eq!(
        chat.list_cleanup_pending().unwrap(),
        vec![confirmed.clone()]
    );
    assert!(chat
        .finalize_operation(&mut FailRemove(&mut protected), &cleanup.id)
        .is_err());
    drop(protected);
    let mut protected =
        EncryptedFileMutationStore::open(dir.path(), zeroize::Zeroizing::new([9; 32])).unwrap();
    assert_eq!(offline.list_cleanup_pending().unwrap(), vec![confirmed]);
    let calls = rpc.send_calls;
    assert_eq!(
        offline
            .finalize_operation(&mut protected, &cleanup.id)
            .unwrap()
            .state,
        State::Confirmed
    );
    assert_eq!(rpc.send_calls, calls);
    assert!(offline.list_cleanup_pending().unwrap().is_empty());
    let replay = offline
        .submitted_operation(Some(&submission))
        .unwrap()
        .unwrap();
    assert_eq!(replay.id, cleanup.id);
    assert_eq!(replay.state, State::Confirmed);
    offline
        .finalize_operation(&mut protected, &cleanup.id)
        .unwrap();
    let stale = chat
        .prepare_channel(
            &mut rpc,
            &mut protected,
            "stale-channel",
            "",
            RtChannelTier::Bottom,
        )
        .unwrap();
    let concurrent = chat
        .prepare_channel(
            &mut rpc,
            &mut protected,
            "concurrent-channel",
            "",
            RtChannelTier::Bottom,
        )
        .unwrap();
    chat.attempt_operation(&mut rpc, &mut protected, &concurrent.id)
        .unwrap();
    assert!(matches!(
        chat.attempt_operation(&mut rpc, &mut protected, &stale.id),
        Err(foks_client::Error::ChatReprepareRequired(_))
    ));
    assert_eq!(
        offline
            .cancel_prepared_operation(&mut protected, &stale.id)
            .unwrap()
            .state,
        State::Cancelled
    );
}

#[test]
pub(crate) fn client_chat_multi_team_refresh_budget() {
    struct Count<'a> {
        connection: &'a mut RealtimeConnection,
        calls: usize,
    }
    impl ChatTransport for Count<'_> {
        fn request(&mut self, request: &Request) -> foks_client::Result<Response> {
            self.calls += 1;
            self.connection.call(request)
        }
    }
    let fixture = Fixture::start("chat-refresh-budget");
    let account = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("budgetowner", 0x39))
        .unwrap();
    let mut protected = fixture.client.open_protected_store().unwrap();
    let path = fixture
        .environment
        .client_path("chat-refresh-budget", "inbox.sqlite3")
        .unwrap();
    let mut soft = foks_client_db::SoftStateStore::open(&path).unwrap();
    let mut sessions = Vec::new();
    for index in 0..2u8 {
        let secrets = NamedTeamSecrets {
            member_min: SecretSeed::new([0x51 + index; 32]),
            member: SecretSeed::new([0x61 + index; 32]),
            admin: SecretSeed::new([0x71 + index; 32]),
            owner: SecretSeed::new([0x81 + index; 32]),
            removal_key: SecretSeed::new([0x91 + index; 32]),
            team_name_commitment_key: [0xa1 + index; 16],
        };
        let team = fixture
            .client
            .foks()
            .create_single_owner_named_team(
                fixture.host(),
                &account.credential,
                &format!("budgetteam{index}"),
                &secrets,
            )
            .unwrap();
        let mut chat = fixture
            .client
            .foks()
            .chat_session(fixture.host(), &account.credential, &team.team)
            .unwrap();
        let mut connection = chat.connection().unwrap();
        for channel_index in 0..3 {
            let prepared = chat
                .prepare_channel(
                    &mut connection,
                    &mut protected,
                    &format!("channel{channel_index}"),
                    "",
                    RtChannelTier::Bottom,
                )
                .unwrap();
            let channel = RtChannelId(prepared.scope.channel);
            chat.attempt_operation(&mut connection, &mut protected, &prepared.id)
                .unwrap();
            for _ in 0..2 {
                let message = chat
                    .prepare_send(
                        &mut connection,
                        &mut protected,
                        channel,
                        "bounded preview fixture",
                    )
                    .unwrap();
                chat.attempt_operation(&mut connection, &mut protected, &message.id)
                    .unwrap();
            }
        }
        sessions.push((chat, connection));
    }
    for pass in 0..2 {
        let started = std::time::Instant::now();
        let mut calls = 0;
        for (chat, connection) in &mut sessions {
            // A different team's setup/refresh can exceed the server idle limit.
            // Measure each refresh over a fresh transport, as the product does.
            *connection = chat.connection().unwrap();
            let mut counted = Count {
                connection,
                calls: 0,
            };
            let inbox = chat.sync_inbox(&mut counted, &mut soft).unwrap().inbox;
            assert_eq!(
                inbox
                    .conversations
                    .iter()
                    .filter(|c| c.preview.is_some())
                    .count(),
                3
            );
            assert!(
                counted.calls <= 12,
                "refresh exceeded its RPC budget: {}",
                counted.calls
            );
            calls += counted.calls;
            let foreground = std::time::Instant::now();
            assert_eq!(
                chat.list_channels(counted.connection)
                    .unwrap()
                    .channels
                    .len(),
                3
            );
            eprintln!(
                "chat budget: foreground channel list {:?}",
                foreground.elapsed()
            );
        }
        eprintln!("chat budget: pass {pass}, two teams, six populated previews, {calls} realtime RPCs, {:?}", started.elapsed());
    }
    // Corrupt one channel's encrypted content after otherwise valid RPC decoding.
    // Sibling previews and the other team's account-scoped inbox remain usable.
    struct CorruptChannel<'a> {
        connection: &'a mut RealtimeConnection,
        channel: RtChannelId,
        reads: usize,
    }
    impl ChatTransport for CorruptChannel<'_> {
        fn request(&mut self, request: &Request) -> foks_client::Result<Response> {
            let corrupt = match request {
                Request::GetThread(arg) => arg.query.channel == self.channel,
                Request::Recents(arg) => arg.channel == self.channel,
                _ => false,
            };
            let mut response = self.connection.call(request)?;
            if corrupt {
                self.reads += 1;
                let damage = |message: &mut RtMessage| {
                    if let RtMessageWrapper::Encrypted(boxed) = &mut message.wrapper {
                        *boxed.ciphertext.0.last_mut().unwrap() ^= 1;
                    }
                };
                match &mut response {
                    Response::Messages(page) => page.messages.iter_mut().for_each(damage),
                    Response::Thread(page) => {
                        for range in &mut page.ranges {
                            range.messages.iter_mut().for_each(damage);
                        }
                        page.sequences.iter_mut().for_each(damage);
                    }
                    _ => panic!("unexpected content response"),
                }
            }
            Ok(response)
        }
    }
    let (chat, connection) = &mut sessions[0];
    *connection = chat.connection().unwrap();
    let bad = chat.list_channels(connection).unwrap().channels[0]
        .metadata
        .id;
    let mut corrupt = CorruptChannel {
        connection,
        channel: bad,
        reads: 0,
    };
    assert!(matches!(
        chat.read_recent(&mut corrupt, bad, 10),
        Err(foks_client::Error::ChatChannelIntegrity(_))
    ));
    let inbox = chat.sync_inbox(&mut corrupt, &mut soft).unwrap().inbox;
    assert_eq!(inbox.blocked_channels, vec![bad]);
    assert_eq!(inbox.conversations.len(), 3);
    assert_eq!(
        inbox
            .conversations
            .iter()
            .filter(|c| c.preview.is_some())
            .count(),
        2
    );
    assert!(inbox
        .conversations
        .iter()
        .find(|c| c.channel.metadata.id == bad)
        .unwrap()
        .preview
        .is_none());
    let attempts = corrupt.reads;
    let next = chat
        .sync_inbox_excluding_previews(&mut corrupt, &mut soft, &[bad])
        .unwrap()
        .inbox;
    assert_eq!(
        corrupt.reads, attempts,
        "quarantined previews are not retried"
    );
    assert_eq!(next.blocked_channels, vec![bad]);
    let (other, connection) = &mut sessions[1];
    *connection = other.connection().unwrap();
    assert_eq!(
        other
            .sync_inbox(connection, &mut soft)
            .unwrap()
            .inbox
            .conversations
            .iter()
            .filter(|c| c.preview.is_some())
            .count(),
        3
    );
}

#[test]
pub(crate) fn client_chat_inbox_previews_are_cached_by_message_and_key() {
    struct CountThreads<'a> {
        connection: &'a mut RealtimeConnection,
        threads: usize,
    }
    impl ChatTransport for CountThreads<'_> {
        fn request(&mut self, request: &Request) -> foks_client::Result<Response> {
            if matches!(request, Request::GetThread(_)) {
                self.threads += 1;
            }
            self.connection.call(request)
        }
    }
    #[derive(Default)]
    struct Cache {
        entries: Vec<(foks_client::ChatPreviewKey, foks_client::ChatPreview)>,
    }
    impl foks_client::ChatPreviewCache for Cache {
        fn get(&mut self, key: &foks_client::ChatPreviewKey) -> Option<foks_client::ChatPreview> {
            self.entries
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, preview)| preview.clone())
        }
        fn put(&mut self, key: foks_client::ChatPreviewKey, preview: foks_client::ChatPreview) {
            self.entries.push((key, preview));
        }
    }

    let fixture = Fixture::start("chat-preview-cache");
    let account = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("cacheowner", 0x3b))
        .unwrap();
    let mut protected = fixture.client.open_protected_store().unwrap();
    let soft_path = fixture
        .environment
        .client_path("chat-preview-cache", "inbox.sqlite3")
        .unwrap();
    let mut soft = foks_client_db::SoftStateStore::open(&soft_path).unwrap();
    let team = fixture
        .client
        .foks()
        .create_single_owner_named_team(
            fixture.host(),
            &account.credential,
            "cacheteam",
            &NamedTeamSecrets {
                member_min: SecretSeed::new([0x52; 32]),
                member: SecretSeed::new([0x62; 32]),
                admin: SecretSeed::new([0x72; 32]),
                owner: SecretSeed::new([0x82; 32]),
                removal_key: SecretSeed::new([0x92; 32]),
                team_name_commitment_key: [0xa2; 16],
            },
        )
        .unwrap();
    let mut chat = fixture
        .client
        .foks()
        .chat_session(fixture.host(), &account.credential, &team.team)
        .unwrap();
    let mut connection = chat.connection().unwrap();
    let mut channels = Vec::new();
    for index in 0..2 {
        let prepared = chat
            .prepare_channel(
                &mut connection,
                &mut protected,
                &format!("cached{index}"),
                "",
                RtChannelTier::Bottom,
            )
            .unwrap();
        let channel = RtChannelId(prepared.scope.channel);
        chat.attempt_operation(&mut connection, &mut protected, &prepared.id)
            .unwrap();
        let message = chat
            .prepare_send(&mut connection, &mut protected, channel, "first")
            .unwrap();
        chat.attempt_operation(&mut connection, &mut protected, &message.id)
            .unwrap();
        channels.push(channel);
    }

    let mut cache = Cache::default();
    let mut counted = CountThreads {
        connection: &mut connection,
        threads: 0,
    };
    let first = chat
        .sync_inbox_with_preview_cache(&mut counted, &mut soft, &[], &mut cache)
        .unwrap()
        .inbox;
    assert_eq!(counted.threads, 2);
    assert_eq!(
        first
            .conversations
            .iter()
            .filter(|c| c.preview.is_some())
            .count(),
        2
    );
    assert_eq!(cache.entries.len(), 2);

    // Nothing moved: every preview is served from the cache, and the inbox
    // is the one the reads produced.
    counted.threads = 0;
    let repeated = chat
        .sync_inbox_with_preview_cache(&mut counted, &mut soft, &[], &mut cache)
        .unwrap()
        .inbox;
    assert_eq!(counted.threads, 0);
    assert_eq!(repeated.conversations, first.conversations);

    // One channel receives another message. Only that channel's preview is
    // fetched again; the other is still served from the cache.
    let message = chat
        .prepare_send(counted.connection, &mut protected, channels[0], "second")
        .unwrap();
    chat.attempt_operation(counted.connection, &mut protected, &message.id)
        .unwrap();
    counted.threads = 0;
    let moved = chat
        .sync_inbox_with_preview_cache(&mut counted, &mut soft, &[], &mut cache)
        .unwrap()
        .inbox;
    assert_eq!(counted.threads, 1);
    assert_eq!(cache.entries.len(), 3);
    assert_eq!(
        moved
            .conversations
            .iter()
            .filter(|c| c.preview.is_some())
            .count(),
        2
    );

    // A channel the caller blocks is never previewed, so nothing about it is
    // ever cached.
    let mut blocked_cache = Cache::default();
    let blocked = chat
        .sync_inbox_with_preview_cache(&mut counted, &mut soft, &[channels[0]], &mut blocked_cache)
        .unwrap()
        .inbox;
    assert_eq!(blocked.blocked_channels, vec![channels[0]]);
    assert!(blocked_cache
        .entries
        .iter()
        .all(|(key, _)| key.channel != channels[0].0));
    assert!(blocked
        .conversations
        .iter()
        .find(|c| c.channel.metadata.id == channels[0])
        .unwrap()
        .preview
        .is_none());

    // The channels the sync already opened describe the same conversations
    // as reopening each one from stored metadata does.
    let listed = chat.list_channels(counted.connection).unwrap().channels;
    assert_eq!(
        chat.inbox_from_store_with_channels(&soft, &listed).unwrap(),
        chat.inbox_from_store(&soft).unwrap()
    );
}
