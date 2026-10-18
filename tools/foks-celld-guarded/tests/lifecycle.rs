//! Actual binary, one finite scenario per fresh native process. No simulated cfg.
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn drain(mut stream: impl Read + Send + 'static) -> thread::JoinHandle<(Vec<u8>, bool)> {
    thread::spawn(move || {
        let mut bytes = Vec::with_capacity(65_536);
        let mut chunk = [0; 4096];
        let mut exceeded = false;
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => return (bytes, exceeded),
                Ok(n) => {
                    let available = 65_536usize.saturating_sub(bytes.len());
                    bytes.extend_from_slice(&chunk[..n.min(available)]);
                    exceeded |= n > available;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => panic!("child output read: {error}"),
            }
        }
    })
}
fn run(name: &str) -> serde_json::Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_foks-celld-guarded"))
        .args(["lifecycle", name])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn actual guarded binary");
    // This child deliberately inherits the root-owned process group. The
    // harness does not create sessions or detach any descendant.
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let (finished, notice) = mpsc::channel();
    let deadline = Instant::now() + Duration::from_secs(120);
    // A bounded monitor polls only child exit, not a lifecycle readiness oracle.
    let monitor = thread::spawn(move || loop {
        if let Some(status) = child.try_wait().expect("observe child exit") {
            let _ = finished.send(status);
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let status = child.wait().expect("reap timed-out child");
            let _ = finished.send(status);
            return;
        }
        thread::park_timeout(Duration::from_millis(10));
    });
    let status = notice
        .recv_timeout(Duration::from_secs(125))
        .expect("owned child did not terminate");
    monitor.join().unwrap();
    let (out, out_large) = stdout.join().unwrap();
    let (err, err_large) = stderr.join().unwrap();
    assert!(
        !out_large && !err_large,
        "{name}: bounded child diagnostics exceeded"
    );
    assert!(
        status.success(),
        "{name}: status={status}; stderr={}",
        String::from_utf8_lossy(&err)
    );
    assert!(out.len() <= 65_536 && out.last() == Some(&b'\n'));
    let value: serde_json::Value =
        serde_json::from_slice(&out).expect("bounded actual report JSON");
    // Captured in the parent-owned qualification command evidence.
    std::io::stdout().lock().write_all(&out).unwrap();
    value
}
fn total(report: &serde_json::Value, name: &str) -> u64 {
    report["event_totals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row[0] == name)
        .unwrap()[1]
        .as_u64()
        .unwrap()
}
#[test]
fn actual_closed_lifecycle_matrix() {
    assert_eq!(celld::LifecycleScenario::ALL.len(), 28);
    for scenario in celld::LifecycleScenario::ALL {
        let name = scenario.name();
        let result = run(name);
        let report = &result["lifecycle"];
        assert_eq!(result["actual_shared_request_exercised"], true);
        assert_eq!(result["process_owner_retained_until_exit"], true);
        assert_eq!(report["claim"], "a2-physical-lifetimes-local-fixture");
        assert_eq!(report["scenario"], name);
        assert_eq!(report["slots"], 2);
        assert_eq!(report["accessible_bytes_per_slot"], 67_108_864);
        assert!(report["peak_reserved_bytes"].as_u64().unwrap() <= 134_217_728);
        assert!(report["peak_observed_live_workers"].as_u64().unwrap() <= 2);
        assert!(report["peak_observed_live_stores"].as_u64().unwrap() <= 2);
        assert_eq!(report["original_owner_identity_preserved"], true);
        assert_eq!(report["original_deadlines_preserved"], true);
        assert!(
            report["native_scratch_observed_bound"].as_u64().unwrap()
                <= report["native_scratch_charge_bound"].as_u64().unwrap()
        );
        assert!(report["timing_policy"]
            .as_str()
            .unwrap()
            .contains("unmeasured/null"));
        let quarantine = matches!(
            name,
            "panic-build" | "panic-turn" | "panic-payload-drop" | "panic-teardown" | "panic-task"
        );
        assert_eq!(
            result["invocation_owners_retained"],
            serde_json::json!([quarantine, false, false])
        );
        let records = report["final_records"].as_array().unwrap();
        assert_eq!(
            records[0]["state"],
            if quarantine { "quarantined" } else { "free" }
        );
        assert_eq!(records[1]["state"], "free");
        if quarantine {
            assert_eq!(records[0]["memory_reserved"], true);
            assert!(total(report, "quarantined") > 0);
        }
        if matches!(name, "admission" | "saturation-drain") {
            assert_eq!(report["peak_occupied"], 2);
            assert_eq!(report["peak_reserved_bytes"], 134_217_728);
            assert!(total(report, "refused") >= if name == "admission" { 1 } else { 4 });
        }
        if name == "tenant-busy" {
            assert_eq!(total(report, "refused"), 4);
        }
        if name == "admission-refusals" {
            assert_eq!(total(report, "refused"), 4);
        }
        if matches!(
            name,
            "deadline-run"
                | "deadline-retire"
                | "deadline-output"
                | "cancel-after-reply"
                | "shutdown"
        ) {
            assert_eq!(report["cycles"][0]["publication_denied"], true);
            assert_eq!(report["cycles"][0]["diagnostic_bytes_observed"], 0);
        }
        if name == "cold-cycles" {
            assert_eq!(report["trace_mode"], "first-cycle-plus-exact-aggregates");
            assert_eq!(report["detailed_trace_cycles"], 1);
            assert_eq!(report["cycles"].as_array().unwrap().len(), 32);
            for (i, cycle) in report["cycles"].as_array().unwrap().iter().enumerate() {
                assert_eq!(cycle["normal_completion"], true);
                assert_eq!(cycle["fresh_wasm"], true);
                assert_eq!(cycle["counter"], if i < 16 { i + 1 } else { 1 });
                assert!(cycle["instantiate_us"].is_null());
            }
            for event in [
                "admitted",
                "worker-built",
                "driver-joined",
                "slot-consumed",
                "worker-dropped",
                "probe-dropped",
                "memory-released",
                "output-dropped",
                "payloads-dropped",
                "owner-dropped",
                "freed",
            ] {
                assert_eq!(total(report, event), 32, "{name}/{event}");
            }
        }
        // The normal detailed trace is actual post-return proof, not Drop entry.
        if name == "normal" {
            let events = report["events"].as_array().unwrap();
            let at = |name: &str| events.iter().position(|event| event[3] == name).unwrap();
            assert!(at("driver-joined") < at("cell-closed"));
            assert!(at("cell-closed") < at("slot-consumed"));
            assert!(at("slot-consumed") < at("worker-dropped"));
            assert!(at("worker-dropped") < at("probe-dropped"));
            assert!(at("probe-dropped") < at("memory-released"));
            assert!(at("memory-released") < at("output-dropped"));
            assert!(at("output-dropped") < at("payloads-dropped"));
            assert!(at("payloads-dropped") < at("owner-dropped"));
            assert!(at("owner-dropped") < at("freed"));
        }
    }
}
