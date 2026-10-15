#![cfg(feature = "celld-proof")]
//! Real FOKS clients over native TCP/mTLS, backed by a deployed proof Worker.
use foks_client::{DeviceCredential, KvWriteOptions, PinnedHost};
use foks_proto::Role;
use foks_server_testkit::celld_proof::{
    claim, digest, CellEndpoint, CellServer, FrozenInstallation,
};
use foks_server_testkit::{RecoveryLedger, TestAccountSpec, TestClient, TestEnvironment};
use std::io::Cursor;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn endpoints(label: &str) -> (CellEndpoint, CellEndpoint) {
    let url = std::env::var("FOKS_CELLD_ENDPOINT")
        .expect("set FOKS_CELLD_ENDPOINT to the deployed local Worker");
    let tenant = format!(
        "{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    (
        CellEndpoint::new(
            &url,
            &tenant,
            std::env::var("FOKS_CELLD_CONTROL_TOKEN").expect("controller token"),
        )
        .unwrap(),
        CellEndpoint::new(
            &url,
            &tenant,
            std::env::var("FOKS_CELLD_TOKEN").expect("data token"),
        )
        .unwrap(),
    )
}
fn write(
    client: &TestClient,
    host: &PinnedHost,
    credential: &DeviceCredential,
    root: [u8; 16],
    name: &str,
) -> foks_client::Result<()> {
    let authenticated = client.foks().authenticate_and_pin(host, credential)?;
    let mut protected = client.open_protected_store().unwrap();
    let mut session = client.foks().user_kv_write_session(
        host,
        credential,
        &authenticated.verified,
        &authenticated.puks,
        client.soft_state_path(),
        &mut protected,
    )?;
    session.put_file(
        root,
        name,
        &mut Cursor::new(name.as_bytes()),
        KvWriteOptions {
            read_role: Role::OWNER,
            write_role: Role::OWNER,
            overwrite: false,
            expected_version: None,
        },
    )?;
    Ok(())
}
fn verify(
    client: &TestClient,
    host: &PinnedHost,
    credential: &DeviceCredential,
    expected: usize,
    ledger: &mut RecoveryLedger,
) -> foks_client::Result<()> {
    let authenticated = client.foks().authenticate_and_pin(host, credential)?;
    let trees = client.foks().sync_user_kv(
        host,
        credential,
        &authenticated.verified,
        &authenticated.puks,
        client.soft_state_path(),
    )?;
    assert_eq!(trees.len(), 1);
    assert_eq!(trees[0].entries.len(), expected);
    for entry in &trees[0].entries {
        ledger
            .observed(
                std::str::from_utf8(&entry.name).unwrap(),
                entry.content.as_deref().unwrap(),
            )
            .unwrap();
    }
    Ok(())
}
fn ledger(environment: &TestEnvironment, label: &str) -> RecoveryLedger {
    let parent = std::env::var_os("FOKS_CELLD_EVIDENCE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| environment.root().to_owned());
    std::fs::create_dir_all(&parent).unwrap();
    RecoveryLedger::create(
        parent.join(format!(
            "{label}-{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )),
        Duration::from_secs(15),
    )
    .unwrap()
}

#[path = "celld_proof/density.rs"]
mod density;

fn persist_sample(name: &str, value: &serde_json::Value) {
    if let Some(parent) = std::env::var_os("FOKS_CELLD_EVIDENCE_DIR") {
        let path = std::path::PathBuf::from(parent).join(name);
        let file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .unwrap();
        serde_json::to_writer(&file, value).unwrap();
        use std::io::Write;
        (&file).write_all(b"\n").unwrap();
        file.sync_all().unwrap();
    }
}

#[test]
#[ignore = "requires deployed real celld proof Worker and explicit local credentials"]
fn real_foks_projection_recovers_keys_and_fences_outstanding_former_owner_mutation() {
    let (controller, endpoint) = endpoints("fenced");
    let environment = TestEnvironment::new().unwrap();
    let native = environment.start_server().unwrap();
    let source_paths = native.owned_paths().map(|path| path.to_path_buf());
    let client = TestClient::new(&environment, "cell-controller").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("celluser", 0x81))
        .unwrap();
    let root = account.kv_projection[0].root_directory_id;
    let mut evidence = ledger(&environment, "native-adapter-fencing");
    write(&client, &host, &account.credential, root, "before.txt").unwrap();
    evidence.acknowledged("before.txt", b"before.txt").unwrap();
    native.shutdown().unwrap();
    let frozen = FrozenInstallation::capture(&environment).unwrap();
    frozen.initialize(&controller).unwrap();
    let schema = frozen.schema.clone();
    drop(frozen);
    let (epoch, owner) = claim(&controller, "0").unwrap();
    let old = CellServer::start(
        &environment,
        endpoint.clone(),
        schema.clone(),
        epoch.clone(),
        owner,
    )
    .unwrap();
    for name in ["cell-a.txt", "cell-b.txt"] {
        write(&client, &host, &account.credential, root, name).unwrap();
        evidence.acknowledged(name, name.as_bytes()).unwrap();
    }
    verify(&client, &host, &account.credential, 3, &mut evidence).unwrap();
    let previous = digest(&old.bridge.snapshot().unwrap().snapshot).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let control = controller.clone();
    old.bridge
        .before_next_publish(Box::new(move || {
            sender.send(claim(&control, &epoch)).unwrap();
        }))
        .unwrap();
    evidence
        .fault("promote-cell-adapter-before-former-owner-authoritative-apply")
        .unwrap();
    assert!(write(
        &client,
        &host,
        &account.credential,
        root,
        "must-not-publish.txt"
    )
    .is_err());
    let (epoch, owner) = receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert!(
        client
            .foks()
            .authenticate_and_pin(&host, &account.credential)
            .is_err(),
        "stale adapter released authenticated data"
    );
    eprintln!(
        "former adapter metrics: {}",
        serde_json::to_string(&old.bridge.metrics()).unwrap()
    );
    old.shutdown().unwrap();
    // Recovery has no access to the original DB, keys, backup or logs. Client
    // hard/soft/protected trust state and controller root secret survive.
    for path in source_paths {
        if path.is_file() {
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        } else if path.exists() {
            std::fs::remove_dir_all(path).unwrap();
        }
    }
    let successor = CellServer::start(&environment, endpoint, schema, epoch, owner).unwrap();
    assert_eq!(
        digest(&successor.bridge.snapshot().unwrap().snapshot).unwrap(),
        previous,
        "fenced native candidate changed authority"
    );
    let client = TestClient::new(&environment, "cell-controller").unwrap();
    assert_eq!(client.pinned_host().unwrap().host_id(), host.host_id());
    verify(&client, &host, &account.credential, 3, &mut evidence).unwrap();
    write(&client, &host, &account.credential, root, "after.txt").unwrap();
    evidence.acknowledged("after.txt", b"after.txt").unwrap();
    verify(&client, &host, &account.credential, 4, &mut evidence).unwrap();
    eprintln!(
        "fenced native adapter recovery: {:?}; metrics {}",
        evidence.recovered().unwrap(),
        serde_json::to_string(&successor.bridge.metrics()).unwrap()
    );
    successor.shutdown().unwrap();
}

#[test]
#[ignore = "requires celld controller fault marker orchestration; not an AZ qualification"]
fn real_foks_cell_owner_process_loss_preserves_acknowledged_sql_writes() {
    let fault_directory = std::path::PathBuf::from(
        std::env::var_os("FOKS_CELLD_FAULT_DIR").expect("fault controller directory"),
    );
    let (controller, endpoint) = endpoints("owner-loss");
    let environment = TestEnvironment::new().unwrap();
    let native = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "cell-controller").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("celluser", 0x82))
        .unwrap();
    let root = account.kv_projection[0].root_directory_id;
    native.shutdown().unwrap();
    let frozen = FrozenInstallation::capture(&environment).unwrap();
    frozen.initialize(&controller).unwrap();
    let schema = frozen.schema.clone();
    drop(frozen);
    let (epoch, owner) = claim(&controller, "0").unwrap();
    let server = CellServer::start(&environment, endpoint, schema, epoch, owner).unwrap();
    let mut evidence = ledger(&environment, "cell-owner-loss");
    std::fs::write(
        fault_directory.join("prepared"),
        b"cell imported before fleet writes",
    )
    .unwrap();
    let preparation = Instant::now();
    while !fault_directory.join("fleet-ready").exists() {
        assert!(
            preparation.elapsed() < Duration::from_secs(45),
            "controller fleet startup timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    for name in ["ack-a.txt", "ack-b.txt", "ack-c.txt"] {
        write(&client, &host, &account.credential, root, name).unwrap();
        evidence.acknowledged(name, name.as_bytes()).unwrap();
    }
    verify(&client, &host, &account.credential, 3, &mut evidence).unwrap();
    evidence
        .fault(
            if std::env::var("FOKS_CELLD_PAUSE_RETURN").as_deref() == Ok("1") {
                "controller-pause-celld-owner-and-reroute-native-adapter"
            } else {
                "controller-sigkill-celld-owner-and-reroute-native-adapter"
            },
        )
        .unwrap();
    let started = Instant::now();
    std::fs::write(
        fault_directory.join("ready"),
        b"real FOKS SQL writes acknowledged",
    )
    .unwrap();
    while !fault_directory.join("injected").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "controller did not inject fault in budget"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    loop {
        evidence.begin_recovery_attempt().unwrap();
        if verify(&client, &host, &account.credential, 3, &mut evidence).is_ok() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "real FOKS read recovery exceeded15s"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    write(&client, &host, &account.credential, root, "new-after.txt").unwrap();
    evidence
        .acknowledged("new-after.txt", b"new-after.txt")
        .unwrap();
    verify(&client, &host, &account.credential, 4, &mut evidence).unwrap();
    eprintln!(
        "real FOKS celld owner process recovery: {:?}; metrics {}",
        evidence.recovered().unwrap(),
        serde_json::to_string(&server.bridge.metrics()).unwrap()
    );
    if std::env::var("FOKS_CELLD_PAUSE_RETURN").as_deref() == Ok("1") {
        evidence
            .fault("resume-former-celld-node-and-probe-old-ingress")
            .unwrap();
        let return_started = Instant::now();
        std::fs::write(
            fault_directory.join("successor-verified"),
            b"all ACKs and new write verified",
        )
        .unwrap();
        while !fault_directory.join("former-returned").exists() {
            assert!(return_started.elapsed() < Duration::from_secs(15));
            std::thread::sleep(Duration::from_millis(10));
        }
        evidence.begin_recovery_attempt().unwrap();
        // verify panics on successful-but-wrong content; only transport/protocol
        // errors are admitted here as an unavailable, fenced old ingress.
        let old_ingress_read = verify(&client, &host, &account.credential, 4, &mut evidence);
        persist_sample(
            "former-owner.jsonl",
            &serde_json::json!({"old_ingress_read_succeeded":old_ingress_read.is_ok()}),
        );
        std::fs::write(
            fault_directory.join("former-probed"),
            b"old ingress read completed",
        )
        .unwrap();
        while !fault_directory.join("survivor-rerouted").exists() {
            assert!(return_started.elapsed() < Duration::from_secs(15));
            std::thread::sleep(Duration::from_millis(10));
        }
        evidence.begin_recovery_attempt().unwrap();
        verify(&client, &host, &account.credential, 4, &mut evidence).unwrap();
        write(
            &client,
            &host,
            &account.credential,
            root,
            "after-return.txt",
        )
        .unwrap();
        evidence
            .acknowledged("after-return.txt", b"after-return.txt")
            .unwrap();
        verify(&client, &host, &account.credential, 5, &mut evidence).unwrap();
        eprintln!(
            "former celld owner return recovery: {:?}",
            evidence.recovered().unwrap()
        );
    }
    server.shutdown().unwrap();
}

#[test]
#[ignore = "local bounded multi-tenant baseline and simultaneous owner loss; no AZ qualification"]
fn real_foks_multiple_cells_capacity_baseline() {
    let tenants = std::env::var("FOKS_CELLD_TENANTS")
        .ok()
        .map(|value| value.parse::<usize>().expect("tenant count"))
        .unwrap_or(4);
    let history = std::env::var("FOKS_CELLD_HISTORY_WRITES")
        .ok()
        .map(|value| value.parse::<usize>().expect("history count"))
        .unwrap_or(3);
    assert!((1..=16).contains(&tenants));
    assert!((3..=100).contains(&history));
    let fault_directory = std::path::PathBuf::from(
        std::env::var_os("FOKS_CELLD_FAULT_DIR").expect("fault controller directory"),
    );
    let idle = density::populate(&fault_directory);
    let started = Instant::now();
    let samples = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        std::thread::scope(|scope| {
            // Channels provide a bounded barrier: a failed preparation cannot leave
            // remaining workers blocked forever in Barrier::wait during teardown.
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let (fault_tx, fault_rx) = std::sync::mpsc::channel();
            let mut releases = Vec::new();
            let mut workers = Vec::new();
            for index in 0..tenants {
                let (release_tx, release_rx) = std::sync::mpsc::channel();
                releases.push(release_tx);
                let ready_tx = ready_tx.clone();
                let fault_tx = fault_tx.clone();
                let fault_directory = fault_directory.clone();
                workers.push(scope.spawn(move || {
                let (controller,endpoint)=endpoints(&format!("capacity-{index}"));
                let environment=TestEnvironment::new().unwrap();let native=environment.start_server().unwrap();
                let client=TestClient::new(&environment,"cell-controller").unwrap();let host=client.probe_and_pin().unwrap().pinned;
                let account=client.create_account(&host,&TestAccountSpec::new("capacityuser",0x91)).unwrap();
                let root=account.kv_projection[0].root_directory_id;native.shutdown().unwrap();
                let imported=Instant::now();let frozen=FrozenInstallation::capture(&environment).unwrap();frozen.initialize(&controller).unwrap();
                let schema=frozen.schema.clone();drop(frozen);let (epoch,owner)=claim(&controller,"0").unwrap();
                let server=CellServer::start(&environment,endpoint,schema,epoch,owner).unwrap();let cold_seconds=imported.elapsed().as_secs_f64();
                std::fs::write(fault_directory.join(format!("prepared-{index}")),b"cell imported").unwrap();
                let waiting=Instant::now();while !fault_directory.join("fleet-ready").exists(){assert!(waiting.elapsed()<Duration::from_secs(60));std::thread::sleep(Duration::from_millis(20));}
                let mut evidence=ledger(&environment,&format!("capacity-{index}"));let mut writes=Vec::new();let mut reads=Vec::new();
                for round in 0..history {
                    let name=format!("item-{round}.txt");let start=Instant::now();write(&client,&host,&account.credential,root,&name).unwrap();writes.push(start.elapsed().as_secs_f64());
                    evidence.acknowledged(&name,name.as_bytes()).unwrap();let start=Instant::now();verify(&client,&host,&account.credential,round+1,&mut evidence).unwrap();reads.push(start.elapsed().as_secs_f64());
                    persist_sample(&format!("capacity-{index}-samples.jsonl"), &serde_json::json!({"phase":"preparation","tenant":index,"completed_writes":round+1,"cold_import_seconds":cold_seconds,"authenticated_write_seconds":writes,"authenticated_fresh_read_seconds":reads,"bridge":server.bridge.metrics()}));
                }
                ready_tx.send(index).unwrap();release_rx.recv_timeout(Duration::from_secs(180)).unwrap();
                evidence.fault("simultaneous-celld-owner-process-loss").unwrap();let recovery=Instant::now();fault_tx.send(index).unwrap();
                while !fault_directory.join("injected").exists(){assert!(recovery.elapsed()<Duration::from_secs(15));std::thread::sleep(Duration::from_millis(10));}
                loop {
                    evidence.begin_recovery_attempt().unwrap();
                    if verify(&client,&host,&account.credential,history,&mut evidence).is_ok(){break;}
                    assert!(recovery.elapsed()<Duration::from_secs(15),"tenant {index} exceeded recovery deadline");std::thread::sleep(Duration::from_millis(100));
                }
                write(&client,&host,&account.credential,root,"after.txt").unwrap();evidence.acknowledged("after.txt",b"after.txt").unwrap();
                verify(&client,&host,&account.credential,history+1,&mut evidence).unwrap();let recovered=evidence.recovered();
                persist_sample(&format!("capacity-{index}-samples.jsonl"), &serde_json::json!({"phase":"recovery","tenant":index,"recovery_seconds":recovery.elapsed().as_secs_f64(),"within_budget":recovered.is_ok(),"bridge":server.bridge.metrics()}));
                let recovered=recovered.unwrap();
                let metrics=server.bridge.metrics();server.shutdown().unwrap();
                serde_json::json!({"tenant":index,"history_writes":history,"cold_import_seconds":cold_seconds,"authenticated_write_seconds":writes,"authenticated_fresh_read_seconds":reads,"recovery_seconds":recovered.as_secs_f64(),"bridge":metrics})
            }));
            }
            drop(ready_tx);
            drop(fault_tx);
            let mut ready = std::collections::BTreeSet::new();
            for _ in 0..tenants {
                assert!(ready.insert(ready_rx.recv_timeout(Duration::from_secs(180)).unwrap()));
            }
            for release in releases {
                release.send(()).unwrap();
            }
            let mut faulted = std::collections::BTreeSet::new();
            for _ in 0..tenants {
                assert!(faulted.insert(fault_rx.recv_timeout(Duration::from_secs(5)).unwrap()));
            }
            std::fs::write(
                fault_directory.join("ready"),
                b"every tenant began its recovery clock",
            )
            .unwrap();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        })
    }));
    // Collect independent cell-revisit evidence even if active recovery missed
    // its deadline, then propagate the original failure unchanged.
    density::revisit(idle);
    let samples = match samples {
        Ok(samples) => samples,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    let result = serde_json::json!({"tenants":tenants,"history_writes":history,"aggregate_seconds":started.elapsed().as_secs_f64(),"samples":samples,"scope":"local real FOKS simultaneous cells; full-history projection, not production density or AZ"});
    eprintln!("{result}");
    if let Some(parent) = std::env::var_os("FOKS_CELLD_EVIDENCE_DIR") {
        let path = std::path::PathBuf::from(parent).join("capacity.json");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        serde_json::to_writer_pretty(&file, &result).unwrap();
        file.sync_all().unwrap();
    }
}

struct GoChild(std::process::Child);
impl Drop for GoChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            #[cfg(unix)]
            {
                // `go test` launches its own test executable. Both belong to
                // the fresh group below, so panic cleanup reaps that child too.
                let _ = std::process::Command::new("/bin/kill")
                    .args(["-KILL", "--", &format!("-{}", self.0.id())])
                    .status();
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
#[test]
#[ignore = "requires deployed celld proof Worker and prepared pinned Go oracle"]
fn official_go_client_uses_celld_authoritative_sql() {
    let oracle =
        std::env::var_os("FOKS_GO_ORACLE_DIR").expect("prepared pinned Go oracle required");
    let (controller, endpoint) = endpoints("go-wire");
    let environment = TestEnvironment::new().unwrap();
    environment
        .set_invite_regime(foks_server_db::InviteRegime::Optional)
        .unwrap();
    let native = environment.start_server().unwrap();
    let ca = environment.client_path("go-cell", "probe.der").unwrap();
    environment.write_probe_root(&ca).unwrap();
    let ready = environment.client_path("go-cell", "ready").unwrap();
    let continuation = environment.client_path("go-cell", "continue").unwrap();
    let log = environment.client_path("go-cell", "output.log").unwrap();
    let output = std::fs::File::create(&log).unwrap();
    let mut command = std::process::Command::new("go");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = GoChild(
        command
            .arg("test")
            .arg("-C")
            .arg(oracle)
            .args([
                "-mod=readonly",
                "-run",
                "^TestGoClientAgainstCelldProjection$",
                "-count=1",
                "-timeout=120s",
                "-v",
            ])
            .env("FOKS_GO_RUST_PROBE", native.addresses().probe.to_string())
            .env("FOKS_GO_RUST_CA_DER", ca)
            .env("FOKS_GO_CELLD_READY_PATH", &ready)
            .env("FOKS_GO_CELLD_CONTINUE_PATH", &continuation)
            .env("GOPROXY", "off")
            .env("GOSUMDB", "off")
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap(),
    );
    let started = Instant::now();
    while !ready.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "Go exited before provisioning: {}",
            std::fs::read_to_string(&log).unwrap()
        );
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "Go provisioning timeout"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    native.shutdown().unwrap();
    let frozen = FrozenInstallation::capture(&environment).unwrap();
    frozen.initialize(&controller).unwrap();
    let schema = frozen.schema.clone();
    drop(frozen);
    let (epoch, owner) = claim(&controller, "0").unwrap();
    let server = CellServer::start(&environment, endpoint, schema, epoch, owner).unwrap();
    std::fs::write(continuation, b"native projection routes through cell SQL").unwrap();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "Go client timeout"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = std::fs::read_to_string(&log).unwrap();
    assert!(
        status.success() && output.contains("--- PASS: TestGoClientAgainstCelldProjection"),
        "Go compatibility failed or skipped: {output}"
    );
    eprintln!(
        "{output}\nGo cell bridge metrics: {}",
        serde_json::to_string(&server.bridge.metrics()).unwrap()
    );
    server.shutdown().unwrap();
}
