use crate::support::Fixture;
use foks_client::{AddLocalTeamMemberRequest, NamedTeamSecrets};
use foks_proto::*;
use foks_rpc::{RealtimeRequest as Request, RealtimeResponse as Response};
use foks_server_testkit::{TestAccountSpec, TestClient};

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
    let Response::Thread(page) = writer.call(&history).unwrap() else {
        panic!()
    };
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
    assert_eq!(messages.messages.len(), 2);
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
