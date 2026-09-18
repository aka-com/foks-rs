use foks_client::{
    ChatTransport, EncryptedFileMutationStore, NamedTeamSecrets, RealtimeConnection,
};
use foks_client_db::{ChatOperationState as State, ChatSubmission, HardStateStore};
use foks_proto::{RtChannelId, RtChannelTier, SecretSeed};
use foks_rpc::{RealtimeRequest as Request, RealtimeResponse as Response};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};
use std::{cell::Cell, path::PathBuf, rc::Rc};

struct ObservedTransport {
    connection: RealtimeConnection,
    hard: PathBuf,
    checkpoint: Rc<Cell<bool>>,
    requests: usize,
    channels: usize,
    sends: usize,
    drop_reply: bool,
    drop_before_send: bool,
}

impl ChatTransport for ObservedTransport {
    fn request(&mut self, request: &Request) -> foks_client::Result<Response> {
        self.requests += 1;
        if matches!(request, Request::ListChannels(_)) {
            self.channels += 1;
        }
        if let Request::Send(argument) = request {
            assert!(
                self.checkpoint.get(),
                "delivery preceded the application checkpoint"
            );
            let operation = HardStateStore::open(&self.hard)
                .unwrap()
                .chat_operation(&argument.send.metadata.id.0)
                .unwrap()
                .unwrap();
            assert_eq!(operation.state, State::Uncertain);
            self.sends += 1;
            if std::mem::take(&mut self.drop_before_send) {
                return Err(foks_client::Error::DeadlineExceeded);
            }
            let response = self.connection.call(request)?;
            if std::mem::take(&mut self.drop_reply) {
                return Err(foks_client::Error::DeadlineExceeded);
            }
            return Ok(response);
        }
        self.connection.call(request)
    }
}

#[test]
fn combined_submit_reuses_verified_work_and_preserves_durable_boundaries() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "combined-submit").unwrap();
    let probe = client.probe_and_pin().unwrap();
    let owner = client
        .create_account(&probe.pinned, &TestAccountSpec::new("submitowner", 0x71))
        .unwrap();
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([0x51; 32]),
        member: SecretSeed::new([0x52; 32]),
        admin: SecretSeed::new([0x53; 32]),
        owner: SecretSeed::new([0x54; 32]),
        removal_key: SecretSeed::new([0x55; 32]),
        team_name_commitment_key: [0x56; 16],
    };
    let team = client
        .foks()
        .create_single_owner_named_team(&probe.pinned, &owner.credential, "submittest", &secrets)
        .unwrap();
    let mut chat = client
        .foks()
        .chat_session(&probe.pinned, &owner.credential, &team.team)
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut protected =
        EncryptedFileMutationStore::open(directory.path(), zeroize::Zeroizing::new([9; 32]))
            .unwrap();
    let mut connection = chat.connection().unwrap();
    let mut channels = Vec::new();
    for name in ["combined", "separate"] {
        let prepared = chat
            .prepare_channel(
                &mut connection,
                &mut protected,
                name,
                "test channel",
                RtChannelTier::Bottom,
            )
            .unwrap();
        let confirmed = chat
            .attempt_operation(&mut connection, &mut protected, &prepared.id)
            .unwrap();
        assert_eq!(confirmed.state, State::Confirmed);
        channels.push(RtChannelId(confirmed.scope.channel));
    }
    let checkpoint = Rc::new(Cell::new(false));
    let mut rpc = ObservedTransport {
        connection,
        hard: client.hard_state_path().to_owned(),
        checkpoint: checkpoint.clone(),
        requests: 0,
        channels: 0,
        sends: 0,
        drop_reply: false,
        drop_before_send: false,
    };
    let submission = ChatSubmission {
        id: [0x61; 16],
        input_mac: [0x61; 32],
    };
    let initial_files = std::fs::read_dir(directory.path()).unwrap().count();
    let requests_before = server.metrics().requests_started;
    let submitted = chat
        .submit_message::<foks_client::Error>(
            &mut rpc,
            &mut protected,
            channels[0],
            "combined message",
            &submission,
            || {
                let pending = HardStateStore::open(client.hard_state_path())
                    .unwrap()
                    .chat_pending(
                        probe.pinned.host_id().as_bytes(),
                        owner.credential.uid.as_bytes(),
                        team.team.as_bytes(),
                    )
                    .unwrap();
                assert_eq!(pending.len(), 1);
                assert_eq!(pending[0].state, State::Prepared);
                assert!(std::fs::read_dir(directory.path()).unwrap().count() > initial_files);
                checkpoint.set(true);
                Ok(())
            },
        )
        .unwrap();
    let combined_requests = server.metrics().requests_started - requests_before;
    assert_eq!(submitted.state, State::Confirmed);
    assert_eq!(rpc.channels, 1);
    assert_eq!(rpc.sends, 1);
    let requests_before = server.metrics().requests_started;
    let before_channels = rpc.channels;
    let prepared = chat
        .prepare_send(&mut rpc, &mut protected, channels[1], "separate message")
        .unwrap();
    chat.attempt_operation(&mut rpc, &mut protected, &prepared.id)
        .unwrap();
    let separate_requests = server.metrics().requests_started - requests_before;
    assert_eq!(rpc.channels - before_channels, 2);
    assert!(
        combined_requests < separate_requests,
        "combined {combined_requests} requests, separate {separate_requests} requests"
    );
    let requests = rpc.requests;
    assert_eq!(
        chat.submit_message::<foks_client::Error>(
            &mut rpc,
            &mut protected,
            channels[0],
            "combined message",
            &submission,
            || panic!("duplicate checkpoint")
        )
        .unwrap(),
        submitted
    );
    assert_eq!(rpc.requests, requests);

    let failed = ChatSubmission {
        id: [0x62; 16],
        input_mac: [0x62; 32],
    };
    checkpoint.set(false);
    let sends = rpc.sends;
    assert!(matches!(
        chat.submit_message(
            &mut rpc,
            &mut protected,
            channels[0],
            "checkpoint failed",
            &failed,
            || Err::<(), _>(foks_client::Error::DeadlineExceeded)
        ),
        Err(foks_client::Error::DeadlineExceeded)
    ));
    assert_eq!(rpc.sends, sends);
    let pending = chat.submitted_operation(Some(&failed)).unwrap().unwrap();
    assert_eq!(pending.state, State::Prepared);
    let requests = rpc.requests;
    assert_eq!(
        chat.submit_message::<foks_client::Error>(
            &mut rpc,
            &mut protected,
            channels[0],
            "checkpoint failed",
            &failed,
            || panic!("duplicate checkpoint")
        )
        .unwrap(),
        pending
    );
    assert_eq!(rpc.requests, requests);
    chat.cancel_prepared_operation(&mut protected, &pending.id)
        .unwrap();

    let lost = ChatSubmission {
        id: [0x63; 16],
        input_mac: [0x63; 32],
    };
    rpc.drop_reply = true;
    assert!(chat
        .submit_message::<foks_client::Error>(
            &mut rpc,
            &mut protected,
            channels[0],
            "lost reply",
            &lost,
            || {
                checkpoint.set(true);
                Ok(())
            }
        )
        .is_err());
    let pending = chat.submitted_operation(Some(&lost)).unwrap().unwrap();
    assert_eq!(pending.state, State::Uncertain);
    let sends = rpc.sends;
    let requests = rpc.requests;
    drop(chat);
    drop(protected);
    let mut chat = client
        .foks()
        .chat_session(&probe.pinned, &owner.credential, &team.team)
        .unwrap();
    let mut protected =
        EncryptedFileMutationStore::open(directory.path(), zeroize::Zeroizing::new([9; 32]))
            .unwrap();
    assert_eq!(
        chat.submit_message::<foks_client::Error>(
            &mut rpc,
            &mut protected,
            channels[0],
            "lost reply",
            &lost,
            || panic!("uncertain checkpoint")
        )
        .unwrap(),
        pending
    );
    assert_eq!(rpc.requests, requests);
    assert_eq!(
        chat.attempt_operation(&mut rpc, &mut protected, &pending.id)
            .unwrap()
            .state,
        State::Confirmed
    );
    assert_eq!(rpc.sends, sends, "uncertain delivery was resubmitted");
    let absent = ChatSubmission {
        id: [0x64; 16],
        input_mac: [0x64; 32],
    };
    rpc.drop_before_send = true;
    assert!(chat
        .submit_message::<foks_client::Error>(
            &mut rpc,
            &mut protected,
            channels[0],
            "transmission uncertain",
            &absent,
            || Ok(())
        )
        .is_err());
    let pending = chat.submitted_operation(Some(&absent)).unwrap().unwrap();
    assert_eq!(pending.state, State::Uncertain);
    let sends = rpc.sends;
    let requests = rpc.requests;
    assert_eq!(
        chat.submit_message::<foks_client::Error>(
            &mut rpc,
            &mut protected,
            channels[0],
            "transmission uncertain",
            &absent,
            || panic!("uncertain checkpoint")
        )
        .unwrap(),
        pending
    );
    assert_eq!(rpc.requests, requests);
    assert_eq!(
        chat.attempt_operation(&mut rpc, &mut protected, &pending.id)
            .unwrap()
            .state,
        State::Uncertain
    );
    assert_eq!(rpc.sends, sends, "absent uncertain message was resent");
}
