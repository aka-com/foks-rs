//! Opt-in isolated benchmark transport. No production state or OS keyring access.
use foks_agent_client::AgentClient;
use foks_agent_proto::{chat::ChatAction, Operation, TeamStoreRef};
use foks_desktop::{AgentError, AgentTransport};
use foks_proto::*;
use foks_rpc::{RealtimeRequest, RealtimeResponse};
use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment, TestProfile};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{self, BufRead, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

struct Agent(Child);
impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn emit(value: Value) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer(&mut out, &value).unwrap();
    writeln!(out).unwrap();
    out.flush().unwrap();
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2));
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn error(e: AgentError) -> Value {
    let code = match &e {
        AgentError::Protocol { code, .. } => serde_json::to_value(code).unwrap(),
        AgentError::Cancelled => json!("cancelled"),
        _ => json!("transport"),
    };
    json!({"code":code,"message":"Benchmark request failed.","retryable":e.transient(),"ambiguous":e.ambiguous(),"fatal":e.fatal()})
}
fn operation(agent: &AgentClient, value: Value) -> Value {
    let op: Operation = serde_json::from_value(value).expect("benchmark operation shape");
    // Setup only: scheduler may briefly hold the fresh profile during startup.
    for _ in 0..100 {
        match AgentTransport::call(agent, op.clone()) {
            Ok(value) => return value,
            Err(AgentError::Protocol {
                code: foks_agent_proto::ErrorCode::ProfileBusy,
                ..
            }) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("benchmark setup failed: {}", error(e)),
        }
    }
    panic!("benchmark setup profile stayed busy")
}
fn chat(agent: &AgentClient, store: &TeamStoreRef, value: Value) -> Value {
    let reply =
        foks_desktop::chat_request(agent, store.clone(), serde_json::from_value(value).unwrap())
            .unwrap();
    serde_json::to_value(reply).unwrap()
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        3,
        "usage: chat_notification_bench <release-agent> <channel-count>"
    );
    let count: usize = args[2].parse().unwrap();
    assert!((1..=200).contains(&count));
    let environment = TestEnvironment::with_profile(TestProfile::ProductionBenchmark).unwrap();
    let _server = environment.start_server().unwrap();
    let sender = TestClient::new(&environment, "benchmark-sender").unwrap();
    let host = sender.probe_and_pin().unwrap().pinned;
    let account = sender
        .create_account(&host, &TestAccountSpec::new("sender", 0x38))
        .unwrap();
    let state = environment.root().join("receiver");
    std::fs::create_dir(&state).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let socket = state.join("agent.sock");
    let mut child = Agent(
        Command::new(Path::new(&args[1]))
            .args([
                "--state-dir",
                state.to_str().unwrap(),
                "--socket",
                socket.to_str().unwrap(),
                "--request-timeout-seconds",
                "35",
                "--scheduler-poll-seconds",
                "3600",
                "--compatibility-poll-seconds",
                "3600",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while !socket.exists() {
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut agent = AgentClient::new(socket);
    agent.set_timeout(Duration::from_secs(40)).unwrap();
    operation(
        &agent,
        json!({"operation":"initialize-state","backend":"private-file"}),
    );
    let certificate = environment.root().join("root.der");
    environment.write_probe_root(&certificate).unwrap();
    operation(
        &agent,
        json!({"operation":"check-and-add-profile","name":"receiver","probe":format!("localhost:{}",environment.addresses().unwrap().probe.port()),"protocol":{"generation":"v019"},"trust":{"kind":"certificate-der","path":certificate}}),
    );
    operation(
        &agent,
        json!({"operation":"create-account","profile":"receiver","alias":"receiver","username":"receiver","device_name":"benchmark","email":"","invite":"","passphrase":null}),
    );
    let team = operation(
        &agent,
        json!({"operation":"create-team","profile":"receiver","account_alias":"receiver","team_alias":"bench","name":"Benchmark","kind":"named"}),
    );
    operation(
        &agent,
        json!({"operation":"add-team-member","profile":"receiver","team_alias":"bench","username":"sender","role":"member","visibility":0}),
    );
    let team_id = team["team_id_hex"].as_str().unwrap();
    let store = TeamStoreRef {
        profile: "receiver".into(),
        account_alias: "receiver".into(),
        team_alias: "bench".into(),
        team_id: team_id.into(),
    };
    eprintln!("benchmark fixture: creating {count} channels");
    let mut channels = Vec::new();
    for i in 0..count {
        let prepared = chat(
            &agent,
            &store,
            json!({"action":"prepare-channel","submission":format!("{:032x}",i+1),"name":format!("channel{i}"),"description":"","admin":false}),
        );
        let op = &prepared["result"]["operation"];
        let confirmed = chat(
            &agent,
            &store,
            json!({"action":"attempt","operation":op["id"]}),
        );
        assert_eq!(confirmed["result"]["operation"]["state"], "confirmed");
        channels.push(op["channel"].as_str().unwrap().to_string());
        chat(
            &agent,
            &store,
            json!({"action":"finalize","operation":op["id"]}),
        );
    }
    eprintln!("benchmark fixture: channels ready");
    let initial = chat(&agent, &store, json!({"action":"channels"}));
    emit(
        json!({"ready":true,"channels":channels,"scope":initial["scope"],"agentPid":child.0.id()}),
    );
    let agent = Arc::new(agent);
    let cancelled = Arc::new(Mutex::new(HashMap::<String, Arc<AtomicBool>>::new()));
    let active = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = std::sync::mpsc::sync_channel::<Value>(16);
    let team_entity = EntityId::from_bytes(unhex(team_id)).unwrap();
    let sender_thread = std::thread::spawn(move || {
        let loaded = sender
            .foks()
            .load_and_pin_team(
                &host,
                &account.credential,
                &account.authenticated.verified,
                &account.authenticated.puks,
                &team_entity,
            )
            .unwrap();
        let key = loaded
            .ptks
            .iter()
            .find(|key| key.role == Role::member(0))
            .unwrap();
        let keys = foks_crypto::derive_realtime_keys(&key.seed, RtAppId::Chat).unwrap();
        let role = RoleAndGeneration {
            role: key.role,
            generation: key.generation,
        };
        let mut connection = None;
        let mut previous = HashMap::<String, (RtMessageId, u64)>::new();
        let mut sequence = 0u128;
        while let Ok(request) = rx.recv() {
            // Open lazily: setup/baselining can exceed the server idle timeout.
            let connection = connection.get_or_insert_with(|| {
                sender
                    .foks()
                    .realtime_connection(&host, &account.credential)
                    .unwrap()
            });
            let channel_text = request["channel"].as_str().unwrap();
            let channel = RtChannelId(unhex(channel_text).try_into().unwrap());
            sequence += 1;
            let id = RtMessageId(sequence.to_be_bytes());
            let (previous_id, previous_sequence) = previous
                .get(channel_text)
                .copied()
                .unwrap_or((RtMessageId([0; 16]), 0));
            let metadata = RtMessageMetadata {
                id,
                previous_id,
                previous_sequence,
                send_time: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
                kind: RtMessageType::Basic,
                further_user_attribution: None,
            };
            let noncer = RtMessageNoncer {
                metadata: metadata.clone(),
                channel,
                app: RtAppId::Chat,
                sender: Some(
                    FqParty::new(account.credential.uid.clone(), host.host_id().clone()).unwrap(),
                ),
                team: FqParty::new(team_entity.clone(), host.host_id().clone()).unwrap(),
            };
            // This synthetic writer exercises real authenticated Basic Send. It does
            // not repeatedly redo the UI mutation workflow at the traffic generator.
            let result = connection.call(&RealtimeRequest::Send(RtSendArgument {
                send: RtSend {
                    metadata,
                    channel: channel.short(),
                    expected_previous_sequence: 0,
                    wrapper: RtMessageWrapper::Encrypted(RtMessageBox {
                        key: role,
                        ciphertext: keys
                            .seal_basic_message(&noncer, b"benchmark message")
                            .unwrap(),
                    }),
                },
            }));
            match result {
                Ok(RealtimeResponse::Sent(send_receipt)) => {
                    previous.insert(channel_text.to_owned(), (id, send_receipt.sequence));
                    emit(json!({"id":request["id"],"value":{"messageId":hex(&id.0)}}));
                }
                _ => emit(
                    json!({"id":request["id"],"error":{"code":"sender-failed","message":"Sender failed.","retryable":false,"ambiguous":true,"fatal":false}}),
                ),
            }
        }
    });
    let mut workers = Vec::new();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        assert!(line.len() <= 65536);
        let request: Value = serde_json::from_str(&line).expect("benchmark request shape");
        match request["kind"].as_str().unwrap() {
            "send" => tx.try_send(request).expect("bounded sender admission"),
            "cancel" => {
                if let Some(token) = cancelled
                    .lock()
                    .unwrap()
                    .get(request["view"].as_str().unwrap())
                {
                    token.store(true, Ordering::SeqCst);
                }
                emit(json!({"id":request["id"],"value":null}));
            }
            "chat" => {
                assert!(
                    active.fetch_add(1, Ordering::SeqCst) < 24,
                    "bounded bridge admission"
                );
                let agent = agent.clone();
                let store = store.clone();
                let cancelled = Arc::clone(&cancelled);
                let active = Arc::clone(&active);
                let view = request["view"].as_str().unwrap().to_owned();
                let token = Arc::new(AtomicBool::new(false));
                cancelled
                    .lock()
                    .unwrap()
                    .insert(view.clone(), Arc::clone(&token));
                workers.retain(|worker: &std::thread::JoinHandle<()>| !worker.is_finished());
                workers.push(std::thread::spawn(move || {
                    let action: ChatAction =
                        serde_json::from_value(request["action"].clone()).unwrap();
                    let result =
                        foks_desktop::chat_request_cancellable(&*agent, store, action, &|| {
                            token.load(Ordering::SeqCst)
                        });
                    cancelled.lock().unwrap().remove(&view);
                    active.fetch_sub(1, Ordering::SeqCst);
                    emit(match result {
                        Ok(value) => json!({"id":request["id"],"value":value}),
                        Err(e) => json!({"id":request["id"],"error":error(e)}),
                    });
                }));
            }
            _ => panic!("invalid benchmark command"),
        }
    }
    for token in cancelled.lock().unwrap().values() {
        token.store(true, Ordering::SeqCst);
    }
    drop(tx);
    sender_thread.join().unwrap();
    for worker in workers {
        worker.join().unwrap();
    }
}
