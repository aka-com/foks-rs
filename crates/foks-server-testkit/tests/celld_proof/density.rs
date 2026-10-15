//! Actual provisioned FOKS identities; controller secrets survive, native data does not.
use super::*;
use foks_server_testkit::celld_proof::Schema;

pub(super) struct IdleCell {
    environment: TestEnvironment,
    endpoint: CellEndpoint,
    schema: Schema,
    epoch: String,
    owner: String,
    content_hash: String,
}

pub(super) fn populate(fault_directory: &std::path::Path) -> Vec<IdleCell> {
    let count = std::env::var("FOKS_CELLD_IDLE_TENANTS")
        .ok()
        .map(|value| value.parse::<usize>().expect("idle tenant count"))
        .unwrap_or(0);
    assert!(count <= 128);
    let start = Instant::now();
    let mut cells = Vec::new();
    for index in 0..count {
        assert!(
            start.elapsed() < Duration::from_secs(300),
            "idle population deadline"
        );
        let (controller, endpoint) = endpoints(&format!("idle-{index}"));
        let environment = TestEnvironment::new().unwrap();
        let native = environment.start_server().unwrap();
        let paths = native.owned_paths().map(|path| path.to_path_buf());
        let client = TestClient::new(&environment, "idle-controller").unwrap();
        let host = client.probe_and_pin().unwrap().pinned;
        let account = client
            .create_account(&host, &TestAccountSpec::new("idleuser", 0x92))
            .unwrap();
        native.shutdown().unwrap();
        let frozen = FrozenInstallation::capture(&environment).unwrap();
        frozen.initialize(&controller).unwrap();
        let schema = frozen.schema.clone();
        drop(frozen);
        let (epoch, owner) = claim(&controller, "0").unwrap();
        let server = CellServer::start(
            &environment,
            endpoint.clone(),
            schema.clone(),
            epoch.clone(),
            owner.clone(),
        )
        .unwrap();
        let mut observations = ledger(&environment, &format!("idle-{index}"));
        verify(&client, &host, &account.credential, 0, &mut observations).unwrap();
        let state = server.bridge.snapshot().unwrap();
        // Ownership/revision may change legitimately during eviction/activation.
        let content_hash = digest(&(&state.snapshot, &state.key_bundle)).unwrap();
        server.shutdown().unwrap();
        for path in paths {
            if path.is_file() {
                std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
            } else if path.exists() {
                std::fs::remove_dir_all(&path).unwrap();
            }
            assert!(!path.exists(), "native source artifact survived");
        }
        cells.push(IdleCell {
            environment,
            endpoint,
            schema,
            epoch,
            owner,
            content_hash,
        });
        persist_sample(
            "idle-population.jsonl",
            &serde_json::json!({"completed":index+1,"elapsed_seconds":start.elapsed().as_secs_f64(),"native_source_artifacts_removed":true}),
        );
    }
    if count != 0 {
        std::fs::write(fault_directory.join("idle-populated"), count.to_string()).unwrap();
        let wait = Instant::now();
        while !fault_directory.join("idle-state-captured").exists() {
            assert!(
                wait.elapsed() < Duration::from_secs(15),
                "idle state capture timeout"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    cells
}

pub(super) fn revisit(cells: Vec<IdleCell>) {
    let count = cells.len();
    for (index, cell) in cells.into_iter().enumerate() {
        if index != 0 && index + 1 != count {
            continue;
        }
        let start = Instant::now();
        let server = CellServer::start(
            &cell.environment,
            cell.endpoint,
            cell.schema,
            cell.epoch,
            cell.owner,
        )
        .unwrap();
        let state = server.bridge.snapshot().unwrap();
        assert_eq!(
            digest(&(&state.snapshot, &state.key_bundle)).unwrap(),
            cell.content_hash
        );
        persist_sample(
            "idle-revisit.jsonl",
            &serde_json::json!({"index":index,"revisit_seconds":start.elapsed().as_secs_f64(),"sql_and_key_content_identical":true,"bridge":server.bridge.metrics()}),
        );
        server.shutdown().unwrap();
    }
}
