#![cfg(unix)]
use foks_client_app::{
    derive_vault_key, AccountVault, ClientCredentials, CredentialBackend, KvMutationPrecondition,
    KvRoleSummary, Profile, ProfileRegistry, ProfileSession, ProtocolPolicy, TrustRoot,
};
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::TestEnvironment;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};

struct Process {
    child: Child,
    stdin: Option<ChildStdin>,
    output: Receiver<String>,
}
impl Process {
    fn start(state: &Path, set: &str) -> Self {
        Self::start_mode(state, set, true)
    }
    fn start_mode(state: &Path, set: &str, read_only: bool) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_foks-rs"))
            .args(["--state-dir"])
            .arg(state)
            .args(["mcp", set, "--profile", "local", "--account", "owner"])
            .args(if read_only {
                vec!["--read-only"]
            } else {
                vec![]
            })
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        let mut process = Self {
            child,
            stdin,
            output,
        };
        process.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"independent-process-test","version":"1"}}}));
        let reply = process.receive();
        assert_eq!(reply["result"]["protocolVersion"], "2025-11-25");
        process.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        process
    }
    fn send(&mut self, value: Value) {
        let input = self.stdin.as_mut().unwrap();
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
    }
    fn receive(&self) -> Value {
        let line = self
            .output
            .recv_timeout(Duration::from_secs(30))
            .expect("MCP response timeout/EOF");
        serde_json::from_str(&line).expect("stdout must contain only MCP JSON")
    }
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":arguments}}));
        let result = self.receive();
        assert_eq!(result["id"], 2);
        result
    }
    fn eof(mut self) {
        self.stdin.take();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "MCP did not exit on EOF"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct Agent(Child);
impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn independent_stdio_client_reads_through_real_agent_and_keeps_agent_after_eof() {
    let external_probe = std::env::var("FOKS_TEST_MCP_PROBE").ok();
    let environment = external_probe
        .is_none()
        .then(|| TestEnvironment::new().unwrap());
    let _server = environment
        .as_ref()
        .map(|environment| environment.start_server().unwrap());
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let root = state.join("root.der");
    if let Some(environment) = &environment {
        environment.write_probe_root(&root).unwrap();
    } else {
        std::fs::copy(
            std::env::var("FOKS_TEST_MCP_CA").expect("external probe CA"),
            &root,
        )
        .unwrap();
    }
    ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    registry
        .add(Profile {
            name: "local".into(),
            probe: external_probe.unwrap_or_else(|| {
                format!(
                    "localhost:{}",
                    environment
                        .as_ref()
                        .unwrap()
                        .addresses()
                        .unwrap()
                        .probe
                        .port()
                )
            }),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        })
        .unwrap();
    let session = ProfileSession::open(&registry, "local").unwrap();
    let credentials = ClientCredentials::open(&state).unwrap();
    credentials
        .with_checked_session(&session, |session| {
            session.probe_and_pin()?;
            let master = credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths().credential_store,
                derive_vault_key(&master),
            )?;
            let mut vault = AccountVault::new(&mut store);
            session.create_account(
                "owner",
                "mcpowner",
                "device",
                "owner@example.test",
                "",
                None,
                &mut vault,
                &master,
            )?;
            session.put_kv_file_checked(
                "owner",
                "/hello",
                &mut b"hello MCP\n".as_slice(),
                KvMutationPrecondition::Create,
                KvRoleSummary::Owner,
                KvRoleSummary::Owner,
                false,
                &mut vault,
                &master,
            )?;
            session.put_kv_symlink_checked(
                "owner",
                "/root-alias",
                "/",
                KvMutationPrecondition::Create,
                KvRoleSummary::Owner,
                KvRoleSummary::Owner,
                false,
                &mut vault,
                &master,
            )?;
            session.put_kv_symlink_checked(
                "owner",
                "/hello-alias",
                "/hello",
                KvMutationPrecondition::Create,
                KvRoleSummary::Owner,
                KvRoleSummary::Owner,
                false,
                &mut vault,
                &master,
            )?;
            session.create_named_team("owner", "group", "mcpteam", &mut vault, &master)?;
            Ok::<_, foks_client_app::Error>(())
        })
        .unwrap();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_foks-rs")).with_file_name("foks-agent");
    assert!(
        binary.exists(),
        "build foks-agent before running MCP process tests"
    );
    let mut agent = Agent(
        Command::new(binary)
            .arg("--state-dir")
            .arg(&state)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let client = foks_agent_client::AgentClient::new(state.join("foks-rs.sock"));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while client.call(foks_agent_proto::Operation::Ping).is_err() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut kv = Process::start(&state, "kv");
    kv.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    assert_eq!(kv.receive()["result"]["tools"].as_array().unwrap().len(), 4);
    let listed = kv.call("list", json!({"path":""}));
    assert!(listed["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("hello\tfile\t"));
    let got = kv.call("get", json!({"path":"hello"}));
    assert_eq!(got["result"]["content"][0]["text"], "hello MCP\n");
    assert_eq!(
        kv.call("get", json!({"path":"/hello-alias"}))["result"]["content"][0]["text"],
        "hello MCP\n"
    );
    assert!(
        kv.call("list", json!({"path":"/root-alias"}))["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("hello\tfile\t")
    );
    let usage = kv.call("usage", json!({}));
    assert!(usage["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Num Files:"));
    let stat = kv.call("stat", json!({"path":"/hello"}));
    let metadata: Value =
        serde_json::from_str(stat["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(metadata["V"]["f2"]["Size"], 10);
    let rejected = kv.call("put", json!({"path":"/hello","content":"overwrite"}));
    assert!(rejected.get("error").is_some() || rejected["result"]["isError"] == true);
    kv.eof();
    assert!(agent.0.try_wait().unwrap().is_none());
    let mut team = Process::start(&state, "team");
    let roster = team.call("list", json!({"team":"mcpteam"}));
    assert!(roster["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("mcpowner\t-\to\to\t"));
    let memberships = team.call("list-memberships", json!({}));
    assert_eq!(memberships["result"]["structuredContent"]["complete"], true);
    assert!(memberships["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("mcpteam"));
    team.eof();
    let mut writes = Process::start_mode(&state, "kv", false);
    let issued = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let id = foks_agent_proto::data::SubmissionHandle::new(issued, [0x12; 16]).to_string();
    let body = "x".repeat(300 * 1024);
    let args =
        json!({"path":"/written/file", "content":body, "mkdir_p":true, "fennec_submission_id":id});
    let put = writes.call("put", args.clone());
    assert_eq!(
        put["result"]["structuredContent"]["status"], "committed",
        "{put}"
    );
    writes.eof();
    let mut writes = Process::start_mode(&state, "kv", false);
    let repeated = writes.call("put", args);
    assert_eq!(
        repeated["result"]["structuredContent"]["status"], "committed",
        "{repeated}"
    );
    assert_eq!(
        writes.call("fennec_status", json!({"fennec_submission_id":id}))["result"]
            ["structuredContent"]["status"],
        "committed"
    );
    let moved = writes.call("mv", json!({"src":"/written/file", "dst":"/written/moved"}));
    assert_eq!(
        moved["result"]["structuredContent"]["status"], "committed",
        "{moved}"
    );
    let got = writes.call("get", json!({"path":"/written/moved"}));
    assert!(got["result"]["isError"] != true, "{got}");
    assert!(
        got["result"]["content"][0]["text"].as_str() == Some(body.as_str()),
        "large file content mismatch"
    );
    let duplicate = writes.call(
        "put",
        json!({"path":"/written/moved", "content":"different"}),
    );
    assert_eq!(
        duplicate["result"]["structuredContent"]["status"], "rejected",
        "{duplicate}"
    );
    let team_write = writes.call(
        "put",
        json!({"path":"/team-file", "team":"mcpteam", "content":"team bytes"}),
    );
    assert_eq!(
        team_write["result"]["structuredContent"]["status"], "committed",
        "{team_write}"
    );
    let linked_write = writes.call(
        "put",
        json!({"path":"/root-alias/through-link", "content":"linked"}),
    );
    assert_eq!(
        linked_write["result"]["structuredContent"]["status"], "committed",
        "{linked_write}"
    );
    assert_eq!(
        writes.call("get", json!({"path":"/through-link"}))["result"]["content"][0]["text"],
        "linked"
    );
    assert_eq!(
        writes.call("rm", json!({"path":"/root-alias"}))["result"]["content"][0]["text"],
        "ok"
    );
    let mkdir = writes.call("mkdir", json!({"path":"/new-dir"}));
    assert_eq!(
        mkdir["result"]["structuredContent"]["status"], "committed",
        "{mkdir}"
    );
    let removed = writes.call("rm", json!({"path":"/written", "recursive":true}));
    assert_eq!(
        removed["result"]["structuredContent"]["status"], "committed",
        "{removed}"
    );
    let fresh = foks_agent_proto::data::SubmissionHandle::new(issued, [0x81; 16]).to_string();
    let fresh_status = writes.call("fennec_status", json!({"fennec_submission_id":fresh}));
    assert_eq!(
        fresh_status["result"]["structuredContent"]["status"], "not-recorded",
        "{fresh_status}"
    );
    let stale =
        foks_agent_proto::data::SubmissionHandle::new(issued - 86_401, [0x82; 16]).to_string();
    let expired = writes.call("fennec_status", json!({"fennec_submission_id":stale}));
    assert_eq!(
        expired["result"]["structuredContent"]["status"], "expired",
        "{expired}"
    );
    let expired_write = writes.call(
        "put",
        json!({"path":"/must-not-execute","content":"stale","fennec_submission_id":stale}),
    );
    assert_eq!(
        expired_write["result"]["structuredContent"]["status"], "expired",
        "{expired_write}"
    );
    let legacy=writes.call("put",json!({"path":"/must-not-execute","content":"legacy","fennec_submission_id":"12".repeat(16)}));
    assert_eq!(legacy["result"]["isError"], true, "{legacy}");
    // Local status has reserved worker capacity and contains counters only.
    let retention = foks_agent_client::AgentClient::new(state.join("foks-rs.sock"))
        .call(foks_agent_proto::Operation::RetentionStatus)
        .unwrap();
    let foks_agent_proto::ResponseResult::Success { value } = retention.result else {
        panic!("retention status failed");
    };
    assert!(value["attempts"].is_u64());
    assert!(value["inventory_saturated"].is_u64());
    if let (Some(environment), Some(server)) = (&environment, &_server) {
        // A 300 KiB put is a large file whose only (final) chunk is carried in
        // UploadInit. Drop that response after commit, before namespace linking.
        let hits = environment
            .arm_fault(foks_server_testkit::TestFault::UploadInitAfterCommitBeforeResponse);
        let uncertain_id =
            foks_agent_proto::data::SubmissionHandle::new(issued, [0x91; 16]).to_string();
        let uncertain = writes.call(
            "put",
            json!({
                "path":"/abandoned-upload", "content":"z".repeat(300 * 1024),
                "fennec_submission_id":uncertain_id,
            }),
        );
        assert_eq!(environment.fault_hits(), hits + 1);
        assert_eq!(
            uncertain["result"]["structuredContent"]["status"], "submission-unknown",
            "{uncertain}"
        );
        assert_eq!(server.run_maintenance().unwrap().0.uploads, 0);
        environment.advance_clock(24 * 60 * 60 * 1_000_000 + 1);
        let mut reclaimed = 0;
        for _ in 0..100 {
            let report = server.run_maintenance().unwrap().0;
            reclaimed += report.uploads;
            if !report.upload_cleanup_deferred {
                break;
            }
        }
        assert_eq!(reclaimed, 1);
        assert_eq!(server.metrics().reclaimed_uploads, 1);
        let status = writes.call(
            "fennec_status",
            json!({"fennec_submission_id":uncertain_id}),
        );
        assert_eq!(
            status["result"]["structuredContent"]["status"], "submission-unknown",
            "{status}"
        );
        // Reclamation cannot turn the uncertain intent into permission to retry.
        let replay = writes.call(
            "put",
            json!({
                "path":"/abandoned-upload", "content":"z".repeat(300 * 1024),
                "fennec_submission_id":uncertain_id,
            }),
        );
        assert_eq!(
            replay["result"]["structuredContent"]["status"], "submission-unknown",
            "{replay}"
        );
    }
    writes.eof();
    if let Ok(oracle) = std::env::var("FOKS_GO_MCP_ORACLE") {
        let status = Command::new("go")
            .args([
                "test",
                "-C",
                &oracle,
                "-run",
                "^TestGoSDKAgainstRustMCP$",
                "-count=1",
                "-v",
            ])
            .env("FOKS_MCP_STATE_DIR", &state)
            .env("FOKS_MCP_CLI", env!("CARGO_BIN_EXE_foks-rs"))
            .status()
            .unwrap();
        assert!(status.success(), "independent Go SDK comparison failed");
    }
}
