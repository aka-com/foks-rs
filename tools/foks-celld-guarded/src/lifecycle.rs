//! Closed native qualification caller; no remote ingress or caller program.
use celld::{LifecycleInput, LifecycleProcessOwner, LifecycleScenario as Scenario};
use foks_hosted_transport::memory::{self, PoolLimits, Pools, Request};
use foks_metadata_core::owned_list::ListRetainedOwner;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    io::{self, Write},
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
struct BoundedJson {
    bytes: Vec<u8>,
}
impl BoundedJson {
    fn new() -> Self {
        Self {
            bytes: Vec::with_capacity(65_536),
        }
    }
    fn finish(mut self) -> Vec<u8> {
        self.bytes.push(b'\n');
        self.bytes
    }
}
impl Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > 65_535)
        {
            return Err(io::Error::other("lifecycle JSON exceeds fixed bound"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn hex(hash: [u8; 32]) -> [u8; 64] {
    let mut output = [0; 64];
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for (pair, byte) in output.chunks_exact_mut(2).zip(hash) {
        pair[0] = DIGITS[usize::from(byte >> 4)];
        pair[1] = DIGITS[usize::from(byte & 15)];
    }
    output
}
fn charge(request: &Arc<Request>, bytes: usize) -> Result<()> {
    if request.reserve_retained(bytes) && !request.failed() {
        Ok(())
    } else {
        Err("actual Request retained admission refused".into())
    }
}
fn scenario_quarantines(scenario: Scenario) -> bool {
    matches!(
        scenario,
        Scenario::PanicBuild
            | Scenario::PanicTurn
            | Scenario::PanicPayloadDrop
            | Scenario::PanicTeardown
            | Scenario::PanicTask
    )
}
fn scenario_deadline(scenario: Scenario, now: Instant) -> Instant {
    match scenario {
        Scenario::DeadlinePreAdmit => now
            .checked_sub(Duration::from_nanos(1))
            .expect("monotonic clock began"),
        Scenario::DeadlineBuild
        | Scenario::DeadlineRun
        | Scenario::DeadlineRetire
        | Scenario::DeadlineOutput => now + Duration::from_secs(8),
        _ => now + Duration::from_secs(90),
    }
}

pub(crate) fn run(scenario: Scenario, input: &str) -> Result<()> {
    // main already initialized V8 before creating these runtime worker threads.
    let layouts = celld::lifecycle_layouts();
    if layouts.serialization_bytes != 65_536
        || layouts.maximum_alignment > 16
        || layouts.largest_task_payload_bytes > 16 * 1024
    {
        return Err("unexpected lifecycle layout".into());
    }
    let pools = Pools::new(PoolLimits::new(
        1024 * 1024,
        64 * 1024 * 1024,
        64 * 1024,
        4,
    )?);
    let owners = [
        pools
            .admitted_request(None)
            .map_err(|_| "actual request admission failed")?,
        pools
            .admitted_request(None)
            .map_err(|_| "actual request admission failed")?,
        pools
            .admitted_request(None)
            .map_err(|_| "actual request admission failed")?,
        pools
            .admitted_request(None)
            .map_err(|_| "actual request admission failed")?,
    ];
    let weak = owners.each_ref().map(Arc::downgrade);
    for (i, owner) in owners.iter().enumerate() {
        for earlier in &owners[..i] {
            if Arc::ptr_eq(owner, earlier) {
                return Err("owner identity reused".into());
            }
        }
        let actual =
            owner.sync(|| memory::current().is_some_and(|current| Arc::ptr_eq(&current, owner)));
        if !actual || memory::current().is_some() {
            return Err("actual Request scope identity failed".into());
        }
    }
    let process_bytes = layouts
        .process_control_bytes
        .checked_add(layouts.report_retained_bytes)
        .and_then(|n| n.checked_add(layouts.serialization_bytes))
        .ok_or("process layout overflow")?;
    charge(&owners[0], process_bytes)?;
    let invocation_bytes = layouts
        .invocation_control_bytes
        .checked_add(layouts.diagnostic_output_bytes)
        .ok_or("invocation layout overflow")?;
    for owner in &owners[1..] {
        charge(owner, invocation_bytes)?;
    }
    let [process, first, second, third] = owners;
    let now = Instant::now();
    let original_deadline = scenario_deadline(scenario, now);
    // Only the scenario's first invocation expires early. Competing admissions
    // have their own original finite deadline and cannot mask Busy with expiry.
    let inputs = [
        LifecycleInput::new(first, original_deadline, Arc::new(AtomicBool::new(false))),
        LifecycleInput::new(
            second,
            now + Duration::from_secs(90),
            Arc::new(AtomicBool::new(false)),
        ),
        LifecycleInput::new(
            third,
            now + Duration::from_secs(90),
            Arc::new(AtomicBool::new(false)),
        ),
    ];
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .enable_all()
        .build()?;
    celld::asyncrt::set_host_handle(runtime.handle().clone());
    let report = runtime.block_on(celld::qualify_lifecycle(
        scenario,
        LifecycleProcessOwner::new(process),
        inputs,
    ))?;
    // A successful report requires actual job joins. Uncertain runtime objects
    // remain explicitly charged; dropping this runtime is not their release.
    drop(runtime);
    let retained = weak.each_ref().map(|owner| owner.upgrade().is_some());
    if !retained[0] {
        return Err("process/report owner released too early".into());
    }
    if retained[1..] != [scenario_quarantines(scenario), false, false] {
        return Err("invocation owner lifetime mismatch".into());
    }
    {
        // Immutable compiled input parsing is explicitly excluded process
        // setup overhead, not a serving or complete heap bound. Drop that
        // temporary tree before allocating the charged serialization buffer.
        let pins: serde_json::Value = serde_json::from_str(input)?;
        let program = report.program();
        for (name, hash, bytes) in [
            ("js", program.js_sha256, program.js_bytes),
            ("wasm", program.wasm_sha256, program.wasm_bytes),
        ] {
            let value = &pins["qualification"]["program"][name];
            let encoded = hex(hash);
            if value["sha256"].as_str() != Some(std::str::from_utf8(&encoded)?)
                || value["bytes"].as_u64() != Some(u64::try_from(bytes)?)
            {
                return Err("actual closed program differs from input pin".into());
            }
        }
    }
    let input_hash = hex(Sha256::digest(input.as_bytes()).into());
    let mut output = BoundedJson::new();
    output.write_all(b"{\"format\":1,\"input_sha256\":\"")?;
    output.write_all(&input_hash)?;
    output.write_all(b"\",\"actual_shared_request_exercised\":true,\"process_owner_retained_until_exit\":true,\"invocation_owners_retained\":")?;
    serde_json::to_writer(&mut output, &retained[1..])?;
    output.write_all(b",\"lifecycle\":")?;
    serde_json::to_writer(&mut output, &report)?;
    output.write_all(b"}")?;
    let encoded = output.finish();
    std::io::stdout().lock().write_all(&encoded)?;
    drop(encoded);
    drop(report);
    drop(pools);
    // Static inventory intentionally retains its independent process Request.
    Ok(())
}
