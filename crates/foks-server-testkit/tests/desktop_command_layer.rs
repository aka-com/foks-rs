#![cfg(unix)]

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use foks_agent_client::AgentClient;
use foks_agent_proto::{Operation, ProfileProtocol, ProfileTrust, ResponseResult};
use foks_client_app::{Capability, ProfileRegistry, ProtocolPolicy};
use foks_compat_artifact::{CanaryArtifact, Outcome, SignedCanaryArtifact};
use foks_desktop::{
    CatalogFailureScope, CatalogLoadToken, CatalogSnapshot, CatalogStoreReadState, CatalogStoreRef,
    KvItemValue,
};
use foks_server_testkit::TestEnvironment;

const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const CANARY_PUBLIC_KEY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const CANARY_SIGNING_SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];

struct AgentProcess {
    child: Child,
}

impl AgentProcess {
    fn assert_running(&mut self) {
        assert!(
            self.child
                .try_wait()
                .expect("poll persistent foks-agent")
                .is_none(),
            "persistent foks-agent exited before the process-reentry flow completed"
        );
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct BackendRunner {
    binary: PathBuf,
    socket: PathBuf,
    process_ids: Vec<u32>,
}

impl BackendRunner {
    fn success<I, S>(&mut self, arguments: I) -> serde_json::Value
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.run(arguments);
        assert!(
            output.status.success(),
            "backend command failed with status {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("backend success must be one JSON value")
    }

    fn failure<I, S>(&mut self, arguments: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.run(arguments);
        assert!(
            !output.status.success(),
            "backend command unexpectedly succeeded"
        );
        output
    }

    fn run<I, S>(&mut self, arguments: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut child = Command::new(&self.binary)
            .arg("--agent-socket")
            .arg(&self.socket)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start foks-desktop-backend");
        self.process_ids.push(child.id());
        let deadline = Instant::now() + PROCESS_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return child.wait_with_output().expect("collect backend output"),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("foks-desktop-backend exceeded its 30 second deadline");
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("poll foks-desktop-backend: {error}");
                }
            }
        }
    }

    fn assert_every_invocation_was_a_fresh_process(&self) {
        let unique = self.process_ids.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), self.process_ids.len());
    }
}

fn required_binary(variable: &str) -> PathBuf {
    let configured = std::env::var_os(variable).unwrap_or_else(|| {
        panic!("{variable} is required; run scripts/test-foks-desktop-real-agent.sh")
    });
    let path = std::fs::canonicalize(configured).expect("canonicalize test binary");
    assert!(path.is_file(), "configured test binary is not a file");
    assert_ne!(
        std::fs::metadata(&path)
            .expect("read test binary metadata")
            .permissions()
            .mode()
            & 0o111,
        0,
        "configured test binary is not executable"
    );
    path
}

fn private_directory(path: &Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn private_file(path: &Path, contents: &[u8]) {
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn start_agent(binary: &Path, state: &Path, socket: &Path) -> AgentProcess {
    let mut agent = AgentProcess {
        child: Command::new(binary)
            .arg("--state-dir")
            .arg(state)
            .arg("--socket")
            .arg(socket)
            .arg("--request-timeout-seconds")
            .arg("5")
            .arg("--scheduler-poll-seconds")
            .arg("3600")
            .arg("--compatibility-poll-seconds")
            .arg("3600")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start foks-agent"),
    };
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Ok(metadata) = std::fs::symlink_metadata(socket) {
            assert!(metadata.file_type().is_socket());
            break;
        }
        if let Some(status) = agent.child.try_wait().expect("poll foks-agent") {
            panic!("foks-agent exited during startup with {status}");
        }
        assert!(Instant::now() < deadline, "foks-agent startup timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    agent
}

fn words(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn with_file(mut arguments: Vec<OsString>, path: &Path) -> Vec<OsString> {
    arguments.push(path.as_os_str().to_owned());
    arguments
}

fn content(value: &serde_json::Value) -> Vec<u8> {
    value["content"]
        .as_array()
        .expect("read response content")
        .iter()
        .map(|byte| u8::try_from(byte.as_u64().expect("content byte")).expect("content byte range"))
        .collect()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after Unix epoch")
        .as_secs()
}

/// This is intentionally opt-in because Cargo cannot expose sibling-package
/// binaries through CARGO_BIN_EXE. The repository script builds exact sibling
/// executables, passes absolute paths, and runs this one ignored test.
#[test]
#[ignore = "run via scripts/test-foks-desktop-real-agent.sh"]
fn process_reentry_and_real_kv_conflict_against_testkit() {
    let agent_binary = required_binary("FOKS_AGENT_TEST_BINARY");
    let backend_binary = required_binary("FOKS_DESKTOP_BACKEND_TEST_BINARY");
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let address = server.addresses().probe;

    let state = environment.root().join("desktop-agent-state");
    private_directory(&state);
    let socket = state.join("agent.sock");
    let mut agent = start_agent(&agent_binary, &state, &socket);
    let mut backend = BackendRunner {
        binary: backend_binary,
        socket: socket.clone(),
        process_ids: Vec::new(),
    };

    let certificate = environment.root().join("desktop-probe-root.der");
    environment.write_probe_root(&certificate).unwrap();
    let probe = format!("localhost:{}", address.port());

    let initialized = backend.success(words(&[
        "initialize",
        "--credential-backend",
        "private-file",
    ]));
    assert_eq!(initialized["backend"], "private-file");

    let checked = backend.success(with_file(
        words(&["check-profile", "work", &probe, "--certificate-der"]),
        &certificate,
    ));
    assert_eq!(checked["profile"]["name"], "work");
    assert_eq!(checked["probe"]["acceptance"], "inserted");

    let pending = backend.success(words(&["pending", "work"]));
    assert_eq!(pending, serde_json::json!([]));

    let account = backend.success(words(&[
        "create-account",
        "work",
        "personal",
        "--username",
        "sol",
        "--device-name",
        "Sol laptop",
    ]));
    assert_eq!(account["username"], "sol");

    let resumed_accounts = backend.success(words(&["accounts", "work"]));
    assert_eq!(resumed_accounts[0]["alias"], "personal");
    assert_eq!(resumed_accounts[0]["username"], "sol");

    let prepared = backend.success(words(&["backup-prepare", "work", "personal", "paper"]));
    let phrase = prepared["phrase"]
        .as_str()
        .expect("backup prepare returns a phrase");
    assert_eq!(phrase.split_whitespace().count(), 17);
    let phrase_file = environment.root().join("owner-backup-phrase");
    private_file(&phrase_file, phrase.as_bytes());
    drop(prepared);

    let committed = backend.success(with_file(
        words(&[
            "backup-commit",
            "work",
            "personal",
            "paper",
            "--phrase-file",
        ]),
        &phrase_file,
    ));
    assert_eq!(committed["backup_alias"], "paper");
    assert_eq!(committed["account_alias"], "personal");

    let discovered = backend.success(words(&["discover-teams", "work", "personal"]));
    assert_eq!(discovered["account_alias"], "personal");
    assert_eq!(discovered["teams"], serde_json::json!([]));

    let group = backend.success(words(&[
        "team-create",
        "work",
        "personal",
        "engineering",
        "--kind",
        "named",
        "--name",
        "Engineering",
    ]));
    assert_eq!(group["alias"], "engineering");
    let team_id = group["team_id_hex"]
        .as_str()
        .expect("team creation returns an entity id")
        .to_owned();

    let original_file = environment.root().join("original-value");
    let current_file = environment.root().join("current-value");
    let stale_file = environment.root().join("stale-value");
    let group_original_file = environment.root().join("group-original-value");
    let group_current_file = environment.root().join("group-current-value");
    private_file(&original_file, b"initial secret");
    private_file(&current_file, b"current secret");
    private_file(&stale_file, b"must not overwrite");
    private_file(&group_original_file, b"initial shared secret");
    private_file(&group_current_file, b"current shared secret");

    let group_created = backend.success(with_file(
        words(&[
            "kv-create-text",
            "work",
            "personal",
            "--team-alias",
            "engineering",
            "--team-id",
            &team_id,
            "/shared_acceptance_value",
            "--value-file",
        ]),
        &group_original_file,
    ));
    let group_created_version = group_created["version"]
        .as_u64()
        .expect("created group-item version");
    let group_first_read = backend.success(words(&[
        "kv-read",
        "work",
        "personal",
        "--team-alias",
        "engineering",
        "--team-id",
        &team_id,
        "/shared_acceptance_value",
        &group_created_version.to_string(),
    ]));
    assert_eq!(content(&group_first_read), b"initial shared secret");

    let group_edited = backend.success(with_file(
        words(&[
            "kv-edit-text",
            "work",
            "personal",
            "--team-alias",
            "engineering",
            "--team-id",
            &team_id,
            "/shared_acceptance_value",
            &group_created_version.to_string(),
            "--read-role",
            "member:0",
            "--write-role",
            "admin",
            "--value-file",
        ]),
        &group_current_file,
    ));
    let group_current_version = group_edited["version"]
        .as_u64()
        .expect("edited group-item version");
    assert_ne!(group_current_version, group_created_version);
    let group_current_read = backend.success(words(&[
        "kv-read",
        "work",
        "personal",
        "--team-alias",
        "engineering",
        "--team-id",
        &team_id,
        "/shared_acceptance_value",
        &group_current_version.to_string(),
    ]));
    assert_eq!(content(&group_current_read), b"current shared secret");

    let created = backend.success(with_file(
        words(&[
            "kv-create-text",
            "work",
            "personal",
            "/acceptance_value",
            "--value-file",
        ]),
        &original_file,
    ));
    let created_version = created["version"].as_u64().expect("created version");
    let first_read = backend.success(words(&[
        "kv-read",
        "work",
        "personal",
        "/acceptance_value",
        &created_version.to_string(),
    ]));
    assert_eq!(content(&first_read), b"initial secret");

    let edited = backend.success(with_file(
        words(&[
            "kv-edit-text",
            "work",
            "personal",
            "/acceptance_value",
            &created_version.to_string(),
            "--read-role",
            "owner",
            "--write-role",
            "owner",
            "--value-file",
        ]),
        &current_file,
    ));
    let current_version = edited["version"].as_u64().expect("edited version");
    assert_ne!(current_version, created_version);

    let stale = backend.failure(with_file(
        words(&[
            "kv-edit-text",
            "work",
            "personal",
            "/acceptance_value",
            &created_version.to_string(),
            "--read-role",
            "owner",
            "--write-role",
            "owner",
            "--value-file",
        ]),
        &stale_file,
    ));
    assert!(String::from_utf8_lossy(&stale.stderr).contains("Conflict"));
    let after_conflict = backend.success(words(&[
        "kv-read",
        "work",
        "personal",
        "/acceptance_value",
        &current_version.to_string(),
    ]));
    assert_eq!(content(&after_conflict), b"current secret");

    // New paths automatically create parent directories via `mkdir_p`.
    // Subsequent writes reuse the created parent directory.
    let nested = backend.success(with_file(
        words(&[
            "kv-create-text",
            "work",
            "personal",
            "/logins/github.com",
            "--value-file",
        ]),
        &original_file,
    ));
    let nested_version = nested["version"].as_u64().expect("nested create version");
    let nested_read = backend.success(words(&[
        "kv-read",
        "work",
        "personal",
        "/logins/github.com",
        &nested_version.to_string(),
    ]));
    assert_eq!(content(&nested_read), b"initial secret");
    let sibling = backend.success(with_file(
        words(&[
            "kv-create-text",
            "work",
            "personal",
            "/logins/gitlab.com",
            "--value-file",
        ]),
        &current_file,
    ));
    let sibling_version = sibling["version"].as_u64().expect("sibling create version");
    let sibling_read = backend.success(words(&[
        "kv-read",
        "work",
        "personal",
        "/logins/gitlab.com",
        &sibling_version.to_string(),
    ]));
    assert_eq!(content(&sibling_read), b"current secret");

    // Compose an authentic short-lived hosted lease through the same registry
    // API used by production refresh. The external agent reloads this durable
    // profile on every dispatch, so expiry is evaluated in the real process;
    // no clock hook or production-only file format is bypassed.
    let added = AgentClient::new(&socket)
        .call(Operation::AddProfile {
            name: "lapsed".to_owned(),
            probe: probe.clone(),
            protocol: ProfileProtocol::CurrentProbeOnly {
                canary_public_key: CANARY_PUBLIC_KEY.to_owned(),
                lease_url: "https://updates.example.test/foks/canary.json".to_owned(),
            },
            trust: ProfileTrust::CertificateDer {
                path: certificate.to_string_lossy().into_owned(),
            },
        })
        .unwrap();
    assert!(matches!(added.result, ResponseResult::Success { .. }));
    let generated_at = unix_seconds();
    let expires_at = generated_at + 2;
    let signed = SignedCanaryArtifact::sign(
        CanaryArtifact {
            schema_version: foks_compat_artifact::SCHEMA_VERSION,
            generation: 1,
            target: probe.clone(),
            run_id: "desktop-command-layer-gate".to_owned(),
            generated_at,
            expires_at,
            protocol_metadata_sha256: foks_client_app::PINNED_PROTOCOL_METADATA_SHA256.to_owned(),
            mutation_digest: "22".repeat(32),
            read_digest: "33".repeat(32),
            outcome: Outcome::Compatible,
            capabilities: BTreeSet::from(["kv".to_owned()]),
            drift_reason: String::new(),
        },
        &CANARY_SIGNING_SEED,
    )
    .unwrap();
    let mut registry = ProfileRegistry::open(&state).unwrap();
    let validated = registry
        .apply_canary("lapsed", &signed, generated_at)
        .unwrap();
    assert!(matches!(
        &validated.protocol,
        ProtocolPolicy::CurrentValidated { .. }
    ));
    assert!(validated.require_at(Capability::Kv, generated_at).is_ok());
    drop(registry);
    while unix_seconds() < expires_at {
        std::thread::sleep(Duration::from_millis(25));
    }
    let denied = backend.failure(words(&[
        "kv-read",
        "lapsed",
        "personal",
        "/acceptance_value",
        "1",
    ]));
    assert!(String::from_utf8_lossy(&denied.stderr).contains("CapabilityDenied"));
    let denied_write = backend.failure(with_file(
        words(&[
            "kv-create-text",
            "lapsed",
            "personal",
            "/lease_gate_must_precede_store_lookup",
            "--value-file",
        ]),
        &stale_file,
    ));
    assert!(String::from_utf8_lossy(&denied_write.stderr).contains("CapabilityDenied"));

    exercise_remote_catalog(
        &agent_binary,
        &mut backend,
        environment.root(),
        &probe,
        &certificate,
        &phrase_file,
        &team_id,
    );
    exercise_profile_failure_isolation(&mut backend);
    exercise_chat(&socket, &team_id, &probe, &certificate, || {
        agent
            .child
            .kill()
            .expect("stop agent before durable reentry");
        agent.child.wait().expect("wait for stopped agent");
        std::fs::remove_file(&socket).expect("remove stopped agent socket");
        agent = start_agent(&agent_binary, &state, &socket);
    });
    agent.assert_running();
    backend.assert_every_invocation_was_a_fresh_process();
}

fn fresh_profile_catalog(socket: &Path, profile: &str) -> CatalogSnapshot {
    let snapshot = foks_desktop::load_profile_catalog_cancellable(
        Arc::new(AgentClient::new(socket)),
        profile.to_owned(),
        CatalogLoadToken::default(),
    )
    .expect("fresh native profile catalog");
    assert_eq!(snapshot.profiles, vec![profile]);
    assert!(snapshot
        .stores
        .iter()
        .all(|store| store.profile() == profile));
    assert!(snapshot
        .items
        .iter()
        .all(|item| item.store.profile() == profile));
    snapshot
}

fn healthy_profile_catalog(socket: &Path) -> CatalogSnapshot {
    let snapshot = fresh_profile_catalog(socket, "work");
    assert!(snapshot.failures.is_empty(), "{:?}", snapshot.failures);
    assert_eq!(snapshot.full_item_reads, Some(vec!["work".to_owned()]));
    assert_eq!(snapshot.inventory.len(), 1);
    assert!(snapshot.inventory[0].accounts_complete);
    assert!(snapshot.inventory[0].teams_complete);
    assert!(!snapshot.store_reads.is_empty());
    assert!(snapshot
        .store_reads
        .iter()
        .all(|read| read.state == CatalogStoreReadState::Complete));
    snapshot
}

fn assert_catalog_value(
    socket: &Path,
    snapshot: &CatalogSnapshot,
    store: &CatalogStoreRef,
    path: &str,
    version: u64,
    expected: &[u8],
) {
    let entries = snapshot
        .items
        .iter()
        .filter(|item| &item.store == store && item.metadata.path == path)
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1, "one catalog entry for {path}");
    assert_eq!(entries[0].metadata.version, version);
    let read = foks_desktop::read_catalog_item(&AgentClient::new(socket), entries[0])
        .expect("read the freshly cataloged version");
    let KvItemValue::File(bytes) = read.value else {
        panic!("expected file value for {path}");
    };
    assert_eq!(bytes.as_slice(), expected);
}

fn exercise_remote_catalog(
    agent_binary: &Path,
    backend: &mut BackendRunner,
    root: &Path,
    probe: &str,
    certificate: &Path,
    phrase_file: &Path,
    existing_team_id: &str,
) {
    let state = root.join("remote-agent-state");
    private_directory(&state);
    let socket = state.join("agent.sock");
    let mut remote_agent = start_agent(agent_binary, &state, &socket);
    let mut remote = BackendRunner {
        binary: backend.binary.clone(),
        socket,
        process_ids: Vec::new(),
    };
    remote.success(words(&[
        "initialize",
        "--credential-backend",
        "private-file",
    ]));
    remote.success(with_file(
        words(&["check-profile", "remote", probe, "--certificate-der"]),
        certificate,
    ));
    remote.success(with_file(
        words(&[
            "recover-account",
            "remote",
            "recovered",
            "--device-name",
            "Remote laptop",
            "--phrase-file",
        ]),
        phrase_file,
    ));
    let accounts = remote.success(words(&["accounts", "remote"]));
    assert_eq!(accounts[0]["username"], "sol");
    let store = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
        profile: "work".to_owned(),
        account_alias: "personal".to_owned(),
    });
    let path = "/remote_catalog_value";
    let initial = healthy_profile_catalog(&backend.socket);
    assert!(!initial.items.iter().any(|item| item.metadata.path == path));
    let original = root.join("remote-original-value");
    let current = root.join("remote-current-value");
    private_file(&original, b"remote initial secret");
    private_file(&current, b"remote updated secret");
    let created = remote.success(with_file(
        words(&[
            "kv-create-text",
            "remote",
            "recovered",
            path,
            "--value-file",
        ]),
        &original,
    ));
    let created_version = created["version"].as_u64().expect("remote create version");
    assert_catalog_value(
        &backend.socket,
        &healthy_profile_catalog(&backend.socket),
        &store,
        path,
        created_version,
        b"remote initial secret",
    );
    let edited = remote.success(with_file(
        words(&[
            "kv-edit-text",
            "remote",
            "recovered",
            path,
            &created_version.to_string(),
            "--read-role",
            "owner",
            "--write-role",
            "owner",
            "--value-file",
        ]),
        &current,
    ));
    let edited_version = edited["version"].as_u64().expect("remote edit version");
    assert!(edited_version > created_version);
    for _ in 0..2 {
        assert_catalog_value(
            &backend.socket,
            &healthy_profile_catalog(&backend.socket),
            &store,
            path,
            edited_version,
            b"remote updated secret",
        );
    }
    remote.success(words(&[
        "kv-remove",
        "remote",
        "recovered",
        path,
        &edited_version.to_string(),
    ]));
    for _ in 0..2 {
        let deleted = healthy_profile_catalog(&backend.socket);
        assert!(!deleted
            .items
            .iter()
            .any(|item| item.store == store && item.metadata.path == path));
    }
    let before = backend.success(words(&["discover-teams", "work", "personal"]));
    assert_eq!(before["teams"].as_array().unwrap().len(), 1);
    let existing = before["teams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|team| team["team_id_hex"] == existing_team_id)
        .expect("existing team remains discoverable");
    assert_eq!(existing["alias"], "engineering");
    let additional = remote.success(words(&[
        "team-create",
        "remote",
        "recovered",
        "research",
        "--kind",
        "named",
        "--name",
        "Research",
    ]));
    let additional_id = additional["team_id_hex"].as_str().unwrap();
    let mut stable_bindings = None;
    for _ in 0..2 {
        let discovered = backend.success(words(&["discover-teams", "work", "personal"]));
        let teams = discovered["teams"].as_array().unwrap();
        assert_eq!(teams.len(), 2);
        let bindings = teams
            .iter()
            .map(|team| {
                (
                    team["team_id_hex"].as_str().unwrap().to_owned(),
                    team["alias"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(bindings.len(), 2);
        assert!(bindings.contains(&(existing_team_id.to_owned(), "engineering".to_owned())));
        assert!(bindings.iter().any(|(id, _)| id == additional_id));
        assert_eq!(
            bindings
                .iter()
                .map(|(_, alias)| alias)
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        if let Some(previous) = &stable_bindings {
            assert_eq!(&bindings, previous);
        }
        let catalog = healthy_profile_catalog(&backend.socket);
        assert_eq!(catalog.stores.len(), 3);
        let catalog_bindings = catalog
            .stores
            .iter()
            .filter_map(|summary| match summary.store_ref() {
                CatalogStoreRef::Team(team) => {
                    assert_eq!(team.account_alias, "personal");
                    Some((team.team_id, team.team_alias))
                }
                CatalogStoreRef::Account(_) => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(catalog_bindings, bindings);
        stable_bindings = Some(bindings);
    }
    assert_eq!(
        remote.success(words(&["pending", "remote"])),
        serde_json::json!([])
    );
    remote_agent.assert_running();
    remote.assert_every_invocation_was_a_fresh_process();
}

fn exercise_profile_failure_isolation(backend: &mut BackendRunner) {
    let unavailable = TestEnvironment::new().unwrap();
    let server = unavailable.start_server().unwrap();
    let certificate = unavailable.root().join("unavailable-probe-root.der");
    unavailable.write_probe_root(&certificate).unwrap();
    let probe = format!("localhost:{}", server.addresses().probe.port());
    backend.success(with_file(
        words(&["check-profile", "unavailable", &probe, "--certificate-der"]),
        &certificate,
    ));
    backend.success(words(&[
        "create-account",
        "unavailable",
        "isolated",
        "--username",
        "isolated",
        "--device-name",
        "Offline laptop",
    ]));
    let before = fresh_profile_catalog(&backend.socket, "unavailable");
    assert!(before.failures.is_empty(), "{:?}", before.failures);
    let client = AgentClient::new(&backend.socket);
    let observed = client
        .call(Operation::ReconcileProfile {
            profile: "unavailable".to_owned(),
        })
        .unwrap();
    let ResponseResult::Success { value } = observed.result else {
        panic!("live profile reconciliation failed");
    };
    assert_eq!(
        value["identity"]["value"]["status"], "connected",
        "identity observation: {}",
        value["identity"]
    );
    assert_eq!(
        value["compatibility"]["value"]["status"], "not-required",
        "compatibility observation: {}",
        value["compatibility"]
    );
    server.shutdown().unwrap();
    let observed = client
        .call(Operation::ReconcileProfile {
            profile: "unavailable".to_owned(),
        })
        .unwrap();
    let ResponseResult::Success { value } = observed.result else {
        panic!("offline observation failed to return scoped results");
    };
    assert_eq!(value["identity"]["code"], "server-unavailable");
    assert_eq!(value["identity"]["fields"]["profile"], "unavailable");
    assert_eq!(
        value["compatibility"]["value"]["status"], "not-required",
        "compatibility observation: {}",
        value["compatibility"]
    );
    let observed = client
        .call(Operation::ReconcileProfile {
            profile: "work".to_owned(),
        })
        .unwrap();
    let ResponseResult::Success { value } = observed.result else {
        panic!("healthy sibling reconciliation failed");
    };
    assert_eq!(
        value["identity"]["value"]["status"], "connected",
        "identity observation: {}",
        value["identity"]
    );
    let failed = fresh_profile_catalog(&backend.socket, "unavailable");
    let store = CatalogStoreRef::Account(foks_agent_proto::AccountStoreRef {
        profile: "unavailable".to_owned(),
        account_alias: "isolated".to_owned(),
    });
    assert!(failed.failures.iter().any(|failure| {
        matches!(&failure.scope, CatalogFailureScope::Store(failed_store) if failed_store == &store)
    }));
    assert!(failed
        .store_reads
        .iter()
        .any(|read| read.store == store && read.state == CatalogStoreReadState::Failed));
    assert!(failed.full_item_reads.as_ref().is_none_or(Vec::is_empty));
    assert!(failed.items.is_empty());
    let healthy = healthy_profile_catalog(&backend.socket);
    assert!(healthy
        .items
        .iter()
        .any(|item| item.metadata.path == "/acceptance_value"));
    let cancelled = CatalogLoadToken::default();
    cancelled.cancel();
    assert!(matches!(
        foks_desktop::load_profile_catalog_cancellable(
            Arc::new(AgentClient::new(&backend.socket)),
            "work".to_owned(),
            cancelled,
        ),
        Err(foks_desktop::AgentError::Cancelled)
    ));
    healthy_profile_catalog(&backend.socket);
}

#[test]
#[ignore = "child process for the real-agent SubmitMessage integration flow"]
fn chat_submit_client_process() {
    use foks_agent_proto::{chat::ChatAction, TeamStoreRef};

    let Some(request_path) = std::env::var_os("FOKS_CHAT_PROCESS_REQUEST") else {
        return;
    };
    let request_path = PathBuf::from(request_path);
    let (socket, store, action): (PathBuf, TeamStoreRef, ChatAction) =
        serde_json::from_slice(&std::fs::read(&request_path).unwrap()).unwrap();
    let reply = foks_desktop::chat_request(&AgentClient::new(socket), store, action).unwrap();
    if let Some(path) = std::env::var_os("FOKS_CHAT_PROCESS_REPLY") {
        private_file(Path::new(&path), &serde_json::to_vec(&reply).unwrap());
    }
}

fn chat_from_fresh_process(
    socket: &Path,
    store: &foks_agent_proto::TeamStoreRef,
    action: &foks_agent_proto::chat::ChatAction,
    discard_reply: bool,
) -> Option<foks_agent_proto::chat::ChatReply> {
    let directory = socket.parent().unwrap();
    let request_path = directory.join("submit-process-request.json");
    let reply_path = directory.join("submit-process-reply.json");
    assert!(!request_path.exists());
    assert!(!reply_path.exists());
    private_file(
        &request_path,
        &serde_json::to_vec(&(socket, store, action)).unwrap(),
    );
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--ignored", "--exact", "chat_submit_client_process"])
        .env("FOKS_CHAT_PROCESS_REQUEST", &request_path)
        .env_remove("FOKS_CHAT_PROCESS_REPLY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !discard_reply {
        command.env("FOKS_CHAT_PROCESS_REPLY", &reply_path);
    }
    let mut child = AgentProcess {
        child: command.spawn().expect("start fresh chat client process"),
    };
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    let status = loop {
        if let Some(status) = child.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "chat client process timed out");
        std::thread::sleep(Duration::from_millis(10));
    };
    std::fs::remove_file(request_path).unwrap();
    if !status.success() {
        use std::io::Read as _;

        let mut output = String::new();
        child
            .child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        child
            .child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        panic!("chat client process failed: {status}: {output}");
    }
    if discard_reply {
        assert!(!reply_path.exists());
        None
    } else {
        let reply = serde_json::from_slice(&std::fs::read(&reply_path).unwrap()).unwrap();
        std::fs::remove_file(reply_path).unwrap();
        Some(reply)
    }
}

fn exercise_chat_submit(
    socket: &Path,
    owner: &foks_agent_proto::TeamStoreRef,
    channel: &str,
    restart_agent: &mut impl FnMut(),
) {
    use foks_agent_proto::{
        chat::{
            ChatAction as A, ChatContent, ChatOperationKind, ChatReceipt, ChatResult as R,
            ChatState,
        },
        SecretString,
    };

    let chat = |action| {
        foks_desktop::chat_request(&AgentClient::new(socket), owner.clone(), action)
            .unwrap()
            .result
    };
    let operation = |result| {
        let R::Operation { operation } = result else {
            panic!("expected durable chat operation")
        };
        operation
    };
    let history = || {
        let R::History { messages, .. } = chat(A::History {
            channel: channel.into(),
            before: None,
        }) else {
            panic!("expected submit history")
        };
        messages
    };
    let submit = |submission: &str, text: &str| A::SubmitMessage {
        submission: submission.into(),
        channel: channel.into(),
        text: SecretString::new(text),
    };
    let prepare = |submission: &str, text: &str| A::PrepareMessage {
        submission: submission.into(),
        channel: channel.into(),
        text: SecretString::new(text),
    };
    let original_history = history();
    let fresh_id = "a1".repeat(16);
    let fresh_text = "fresh submit survives durable reentry";
    let fresh_action = submit(&fresh_id, fresh_text);
    let fresh = operation(chat(fresh_action.clone()));
    assert_eq!(fresh.state, ChatState::Confirmed);
    assert_eq!(fresh.kind, ChatOperationKind::SendMessage);
    assert_eq!(fresh.channel, channel);
    let after_fresh = history();
    assert_eq!(after_fresh.len(), original_history.len() + 1);
    let fresh_message = after_fresh
        .iter()
        .find(|message| matches!(&message.content, ChatContent::Text { text } if text.expose() == fresh_text))
        .unwrap();
    assert_eq!(
        fresh.receipt,
        Some(ChatReceipt::MessageSent {
            sequence: fresh_message.sequence.clone(),
        })
    );
    assert_eq!(operation(chat(fresh_action.clone())), fresh);
    assert_eq!(operation(chat(prepare(&fresh_id, fresh_text))), fresh);
    assert_eq!(history(), after_fresh);
    assert!(matches!(
        chat(A::OperationBody {
            operation: fresh.id.clone(),
            channel: channel.into(),
        }),
        R::OperationBody { text: None, .. }
    ));
    assert!(!serde_json::to_string(&fresh).unwrap().contains(fresh_text));

    let prepared_id = "a2".repeat(16);
    let prepared_text = "prepared submission requires explicit attempt";
    let prepared = operation(chat(prepare(&prepared_id, prepared_text)));
    assert_eq!(prepared.state, ChatState::Prepared);
    assert_eq!(prepared.receipt, None);
    assert_ne!(prepared.id, fresh.id);
    restart_agent();
    assert_eq!(operation(chat(fresh_action)), fresh);
    assert_eq!(operation(chat(prepare(&fresh_id, fresh_text))), fresh);
    assert_eq!(
        operation(chat(submit(&prepared_id, prepared_text))),
        prepared
    );
    assert_eq!(
        operation(chat(prepare(&prepared_id, prepared_text))),
        prepared
    );
    let R::Pending { operations } = chat(A::Pending) else {
        panic!("expected prepared operation after restart")
    };
    assert_eq!(operations, vec![prepared.clone()]);
    assert!(matches!(
        chat(A::OperationBody {
            operation: prepared.id.clone(),
            channel: channel.into(),
        }),
        R::OperationBody { text: Some(text), .. } if text.expose() == prepared_text
    ));
    assert_eq!(history(), after_fresh);
    let attempted = operation(chat(A::Attempt {
        operation: prepared.id.clone(),
    }));
    assert_eq!(attempted.id, prepared.id);
    assert_eq!(attempted.state, ChatState::Confirmed);
    assert_eq!(
        operation(chat(submit(&prepared_id, prepared_text))),
        attempted
    );
    let after_attempt = history();
    assert_eq!(after_attempt.len(), after_fresh.len() + 1);
    assert_eq!(
        after_attempt
            .iter()
            .filter(|message| matches!(&message.content, ChatContent::Text { text } if text.expose() == prepared_text))
            .count(),
        1
    );

    let alternate = operation(chat(A::PrepareChannel {
        submission: "a3".repeat(16),
        name: SecretString::new("submitmismatch"),
        description: SecretString::new(""),
        admin: false,
    }));
    assert_eq!(
        operation(chat(A::Attempt {
            operation: alternate.id,
        }))
        .state,
        ChatState::Confirmed
    );
    for altered in [
        submit(&fresh_id, "altered submit body"),
        prepare(&fresh_id, "altered prepare body"),
        A::SubmitMessage {
            submission: fresh_id.clone(),
            channel: alternate.channel.clone(),
            text: SecretString::new(fresh_text),
        },
        A::PrepareMessage {
            submission: fresh_id.clone(),
            channel: alternate.channel.clone(),
            text: SecretString::new(fresh_text),
        },
    ] {
        assert!(
            foks_desktop::chat_request(&AgentClient::new(socket), owner.clone(), altered).is_err()
        );
        assert_eq!(
            operation(chat(A::Status {
                operation: fresh.id.clone(),
            })),
            fresh
        );
    }
    let R::History { messages, .. } = chat(A::History {
        channel: alternate.channel,
        before: None,
    }) else {
        panic!("expected alternate channel history")
    };
    assert!(messages.is_empty());
    assert_eq!(history(), after_attempt);

    let cancelled_id = "a4".repeat(16);
    let cancelled_text = "cancelled submission must stay cancelled";
    let cancellable = operation(chat(prepare(&cancelled_id, cancelled_text)));
    let cancelled = operation(chat(A::Cancel {
        operation: cancellable.id,
    }));
    assert_eq!(cancelled.state, ChatState::Cancelled);
    assert_eq!(
        operation(chat(submit(&cancelled_id, cancelled_text))),
        cancelled
    );
    assert_eq!(history(), after_attempt);

    let lost_id = "a5".repeat(16);
    let lost_text = "caller discards successful submit reply";
    let lost_action = submit(&lost_id, lost_text);
    assert!(chat_from_fresh_process(socket, owner, &lost_action, true).is_none());
    restart_agent();
    let recovered = operation(
        chat_from_fresh_process(socket, owner, &lost_action, false)
            .unwrap()
            .result,
    );
    assert_eq!(recovered.state, ChatState::Confirmed);
    assert_ne!(recovered.id, fresh.id);
    assert_ne!(recovered.id, attempted.id);
    assert_eq!(operation(chat(prepare(&lost_id, lost_text))), recovered);
    assert_eq!(
        operation(
            chat_from_fresh_process(socket, owner, &lost_action, false)
                .unwrap()
                .result,
        ),
        recovered
    );
    assert_eq!(
        operation(chat(A::Status {
            operation: recovered.id.clone(),
        })),
        recovered
    );
    assert_eq!(
        operation(chat(submit(&cancelled_id, cancelled_text))),
        cancelled
    );
    let after_recovery = history();
    assert_eq!(after_recovery.len(), after_attempt.len() + 1);
    let lost_messages = after_recovery
        .iter()
        .filter(|message| matches!(&message.content, ChatContent::Text { text } if text.expose() == lost_text))
        .collect::<Vec<_>>();
    assert_eq!(lost_messages.len(), 1);
    assert_eq!(
        recovered.receipt,
        Some(ChatReceipt::MessageSent {
            sequence: lost_messages[0].sequence.clone(),
        })
    );
    let R::Pending { operations } = chat(A::Pending) else {
        panic!("expected no pending submit operations")
    };
    assert!(operations.is_empty());
}

fn exercise_chat(
    socket: &Path,
    team_id: &str,
    probe: &str,
    certificate: &Path,
    mut restart_agent: impl FnMut(),
) {
    use foks_agent_proto::{
        chat::{ChatAction as A, ChatContent, ChatResult as R, ChatState},
        SecretString, TeamRole, TeamStoreRef,
    };
    let client = AgentClient::new(socket);
    let owner = TeamStoreRef {
        profile: "work".into(),
        account_alias: "personal".into(),
        team_alias: "engineering".into(),
        team_id: team_id.into(),
    };
    let call = |operation| match client.call(operation).unwrap().result {
        ResponseResult::Success { value } => value,
        other => panic!("agent operation failed: {other:?}"),
    };
    let chat = |store: &TeamStoreRef, action| {
        foks_desktop::chat_request(&client, store.clone(), action)
            .unwrap()
            .result
    };
    let create = A::PrepareChannel {
        submission: "ab".repeat(16),
        name: SecretString::new(""),
        description: SecretString::new("team discussions"),
        admin: false,
    };
    let R::Operation {
        operation: prepared,
    } = chat(&owner, create.clone())
    else {
        panic!("expected preparation")
    };
    let R::Operation {
        operation: repeated,
    } = chat(&owner, create.clone())
    else {
        panic!("expected preparation")
    };
    assert_eq!(prepared.id, repeated.id);
    let channel = prepared.channel.clone();
    let R::Operation {
        operation: confirmed,
    } = chat(
        &owner,
        A::Attempt {
            operation: prepared.id.clone(),
        },
    )
    else {
        panic!("expected create receipt")
    };
    assert_eq!(confirmed.state, ChatState::Confirmed);
    let R::Channels { channels, .. } = chat(&owner, A::Channels) else {
        panic!("expected channel discovery")
    };
    assert_eq!(
        channels
            .iter()
            .find(|c| c.id == channel)
            .unwrap()
            .description
            .as_ref()
            .unwrap()
            .expose(),
        "team discussions"
    );
    // A fresh local client recovers the same completed submission without resealing.
    let restarted = AgentClient::new(socket);
    let reply = foks_desktop::chat_request(&restarted, owner.clone(), create).unwrap();
    assert!(
        matches!(reply.result, R::Operation { operation } if operation.id == prepared.id && operation.state == ChatState::Confirmed)
    );
    call(Operation::AddProfile {
        name: "chatguest".into(),
        probe: probe.into(),
        protocol: ProfileProtocol::V019,
        trust: ProfileTrust::CertificateDer {
            path: certificate.to_string_lossy().into_owned(),
        },
    });
    call(Operation::Probe {
        profile: "chatguest".into(),
    });
    call(Operation::CreateAccount {
        profile: "chatguest".into(),
        alias: "chatguest".into(),
        username: "chatguest".into(),
        device_name: "Guest laptop".into(),
        email: "".into(),
        invite: SecretString::new(""),
        passphrase: None,
    });
    call(Operation::AddTeamMember {
        profile: "work".into(),
        team_alias: "engineering".into(),
        username: "chatguest".into(),
        role: TeamRole::Member,
        visibility: 0,
    });
    let discovery = call(Operation::DiscoverTeams {
        profile: "chatguest".into(),
        account_alias: "chatguest".into(),
    });
    let alias = discovery["teams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["team_id_hex"] == team_id)
        .unwrap()["alias"]
        .as_str()
        .unwrap()
        .to_owned();
    let guest = TeamStoreRef {
        profile: "chatguest".into(),
        account_alias: "chatguest".into(),
        team_alias: alias,
        ..owner.clone()
    };
    for (index, actor) in [&owner, &guest].into_iter().enumerate() {
        let action = A::PrepareMessage {
            submission: format!("{:032x}", index + 1),
            channel: channel.clone(),
            text: SecretString::new(format!("chat from account {index}")),
        };
        let R::Operation {
            operation: prepared,
        } = chat(actor, action.clone())
        else {
            panic!("expected prepared send")
        };
        let altered = A::PrepareMessage {
            submission: format!("{:032x}", index + 1),
            channel: channel.clone(),
            text: SecretString::new("changed"),
        };
        assert!(foks_desktop::chat_request(&client, actor.clone(), altered).is_err());
        let body_action = A::OperationBody {
            operation: prepared.id.clone(),
            channel: channel.clone(),
        };
        // Body recovery survives a new local connection and does not attempt delivery.
        let body = foks_desktop::chat_request(
            &AgentClient::new(socket),
            actor.clone(),
            body_action.clone(),
        )
        .unwrap();
        assert!(
            matches!(body.result, R::OperationBody { text: Some(text), .. } if text.expose() == format!("chat from account {index}"))
        );
        let other = if index == 0 { &guest } else { &owner };
        assert!(foks_desktop::chat_request(&client, other.clone(), body_action.clone()).is_err());
        let R::Operation { operation: receipt } = chat(
            actor,
            A::Attempt {
                operation: prepared.id.clone(),
            },
        ) else {
            panic!("expected receipt")
        };
        assert_eq!(
            receipt.receipt,
            Some(foks_agent_proto::chat::ChatReceipt::MessageSent {
                sequence: (index + 1).to_string()
            })
        );
        assert!(matches!(
            chat(actor, body_action),
            R::OperationBody { text: None, .. }
        ));
        let R::Operation { operation: replay } = chat(actor, action) else {
            panic!("expected replay")
        };
        assert_eq!(replay, receipt);
        let retry = foks_desktop::chat_request(
            &AgentClient::new(socket),
            actor.clone(),
            A::Attempt {
                operation: receipt.id.clone(),
            },
        )
        .unwrap();
        assert!(matches!(retry.result, R::Operation { operation } if operation == receipt));
    }
    for actor in [&owner, &guest] {
        let R::History { messages, .. } = chat(
            actor,
            A::History {
                channel: channel.clone(),
                before: None,
            },
        ) else {
            panic!("expected history")
        };
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|m|matches!(&m.content,ChatContent::Text {text} if text.expose().starts_with("chat from account"))));
        let R::Pending { operations } = chat(actor, A::Pending) else {
            panic!("expected pending list")
        };
        assert!(operations.is_empty());
    }
    let R::Inbox { conversations, .. } = chat(
        &owner,
        A::SyncInbox {
            blocked_channels: Vec::new(),
        },
    ) else {
        panic!("expected inbox")
    };
    assert_eq!(conversations.len(), 1);
    assert_eq!(conversations[0].unread, "1");
    let R::Read {
        channel: read_channel,
        sequence,
    } = chat(
        &owner,
        A::MarkRead {
            channel: channel.clone(),
            sequence: "2".into(),
        },
    )
    else {
        panic!("expected read receipt")
    };
    assert_eq!(read_channel, channel);
    assert_eq!(sequence, "2");
    let R::Inbox {
        head,
        conversations,
        ..
    } = chat(
        &owner,
        A::SyncInbox {
            blocked_channels: Vec::new(),
        },
    )
    else {
        panic!("expected inbox")
    };
    assert_eq!(conversations[0].unread, "0");
    let poll_socket = socket.to_owned();
    let poll_store = owner.clone();
    let waiting = std::thread::spawn(move || {
        foks_desktop::chat_request(
            &AgentClient::new(poll_socket),
            poll_store,
            A::PollInbox {
                since: head,
                timeout_milliseconds: 5_000,
            },
        )
        .unwrap()
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(matches!(chat(&owner, A::Channels), R::Channels { .. }));
    let R::Operation {
        operation: prepared,
    } = chat(
        &guest,
        A::PrepareMessage {
            submission: "de".repeat(16),
            channel: channel.clone(),
            text: SecretString::new("wake agent poll"),
        },
    )
    else {
        panic!("expected poll-wake preparation")
    };
    let R::Operation { .. } = chat(
        &guest,
        A::Attempt {
            operation: prepared.id,
        },
    ) else {
        panic!("expected poll-wake receipt")
    };
    let R::Poll {
        bumped,
        inbox_version,
    } = waiting.join().unwrap().result
    else {
        panic!("expected poll wake")
    };
    assert!(bumped);
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_flag = cancelled.clone();
    let cancel_socket = socket.to_owned();
    let cancel_store = owner.clone();
    let cancel_since = inbox_version.clone();
    let canceling = std::thread::spawn(move || {
        foks_desktop::chat_request_cancellable(
            &AgentClient::new(cancel_socket),
            cancel_store,
            A::PollInbox {
                since: cancel_since,
                timeout_milliseconds: 5_000,
            },
            &|| cancel_flag.load(std::sync::atomic::Ordering::Acquire),
        )
        .is_err()
    });
    std::thread::sleep(Duration::from_millis(100));
    cancelled.store(true, std::sync::atomic::Ordering::Release);
    assert!(canceling.join().unwrap());
    std::thread::sleep(Duration::from_millis(1_100));
    let R::Poll { bumped, .. } = chat(
        &owner,
        A::PollInbox {
            since: inbox_version,
            timeout_milliseconds: 1,
        },
    ) else {
        panic!("expected reusable poll slot")
    };
    assert!(!bumped);
    let R::Operation {
        operation: prepared,
    } = chat(
        &owner,
        A::PrepareMessage {
            submission: "ef".repeat(16),
            channel: channel.clone(),
            text: SecretString::new("cancel me"),
        },
    )
    else {
        panic!("expected preparation")
    };
    let R::Operation {
        operation: cancelled,
    } = chat(
        &owner,
        A::Cancel {
            operation: prepared.id,
        },
    )
    else {
        panic!("expected cancellation")
    };
    assert_eq!(cancelled.state, ChatState::Cancelled);
    let wrong = TeamStoreRef {
        account_alias: "chatguest".into(),
        ..owner.clone()
    };
    assert!(foks_desktop::chat_request(&client, wrong, A::Channels).is_err());
    exercise_chat_submit(socket, &owner, &channel, &mut restart_agent);
    // Optional bounded manual-desktop fixture. Only synthetic test accounts are
    // used. The operator owns a private control directory; no production agent is
    // contacted. Removing `ready` ends the fixture, touching `send` emits one
    // message from the second test account. Normal CI does not enter this branch.
    if let Some(control) = std::env::var_os("FOKS_DESKTOP_REVIEW_DIR") {
        let control = PathBuf::from(control);
        private_directory(&control);
        private_file(&control.join("ready"), socket.to_string_lossy().as_bytes());
        let deadline = Instant::now() + Duration::from_secs(1800);
        let mut sent = 0u32;
        while control.join("ready").exists() && Instant::now() < deadline {
            if control.join("send").exists() && sent < 20 {
                std::fs::remove_file(control.join("send")).unwrap();
                sent += 1;
                let R::Operation { operation } = chat(
                    &guest,
                    A::PrepareMessage {
                        submission: format!("{:032x}", 1000 + sent),
                        channel: channel.clone(),
                        text: SecretString::new(format!(
                            "Desktop notification test {sent}: **verified Basic text**"
                        )),
                    },
                ) else {
                    panic!("expected manual fixture preparation")
                };
                chat(
                    &guest,
                    A::Attempt {
                        operation: operation.id,
                    },
                );
                private_file(&control.join("sent"), sent.to_string().as_bytes());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
