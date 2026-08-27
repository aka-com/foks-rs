//! Opt-in end-to-end workload benchmarks.
//!
//! These tests report observations for the current host. They enforce bounded
//! admission and recovery invariants, not universal latency promises.

use std::io::Write as _;
use std::net::TcpStream;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use foks_rpc::{Error as RpcError, DEFAULT_MAX_FRAME_LENGTH};
use foks_server_testkit::{TestEnvironment, TestProfile};

const SATURATION_CLIENTS: usize = 128;
const SLOW_CLIENTS: usize = 48;
const HEALTHY_REQUESTS: usize = 32;
const WRITER_REJECTIONS: usize = 10_000;

#[test]
#[ignore = "release-mode production workload benchmark"]
fn connection_saturation_reports_throughput_and_tail_latency() {
    let environment = TestEnvironment::with_profile(TestProfile::ProductionBenchmark).unwrap();
    let server = environment.start_server().unwrap();
    let address = server.addresses().public_services;
    let config = client_config(server.service_roots());
    let barrier = Arc::new(Barrier::new(SATURATION_CLIENTS + 1));
    let wall = Instant::now();
    let results = std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(SATURATION_CLIENTS);
        for _ in 0..SATURATION_CLIENTS {
            let config = Arc::clone(&config);
            let barrier = Arc::clone(&barrier);
            workers.push(scope.spawn(move || {
                barrier.wait();
                let started = Instant::now();
                public_round_trip(address, config).map(|()| started.elapsed())
            }));
        }
        barrier.wait();
        workers
            .into_iter()
            .map(|worker| worker.join().expect("benchmark worker did not panic"))
            .collect::<Vec<_>>()
    });
    let elapsed = wall.elapsed();
    let errors = results.iter().filter(|result| result.is_err()).count();
    let latencies = results
        .into_iter()
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    report("connection_saturation", &latencies, elapsed, errors);
    assert_eq!(errors, 0, "all admitted clients must complete");
    assert_eq!(latencies.len(), SATURATION_CLIENTS);
    server.shutdown().unwrap();
}

#[test]
#[ignore = "release-mode production workload benchmark"]
fn slow_clients_do_not_starve_healthy_clients() {
    let environment = TestEnvironment::with_profile(TestProfile::ProductionBenchmark).unwrap();
    let server = environment.start_server().unwrap();
    let address = server.addresses().public_services;
    let config = client_config(server.service_roots());
    let mut slow = Vec::with_capacity(SLOW_CLIENTS);
    for _ in 0..SLOW_CLIENTS {
        let mut connection = connect(address, Arc::clone(&config)).unwrap();
        // Begin a valid 64-byte frame and then withhold the remaining payload.
        connection.write_all(&[0x40, 0x90]).unwrap();
        connection.flush().unwrap();
        slow.push(connection);
    }

    let wall = Instant::now();
    let mut latencies = Vec::with_capacity(HEALTHY_REQUESTS);
    let mut errors = 0;
    for _ in 0..HEALTHY_REQUESTS {
        let started = Instant::now();
        match public_round_trip(address, Arc::clone(&config)) {
            Ok(()) => latencies.push(started.elapsed()),
            Err(_) => errors += 1,
        }
    }
    report("slow_client_isolation", &latencies, wall.elapsed(), errors);
    assert_eq!(errors, 0, "slow peers must not consume the I/O worker");
    assert_eq!(latencies.len(), HEALTHY_REQUESTS);
    for connection in &mut slow {
        connection.sock.set_nonblocking(true).unwrap();
        let mut byte = [0];
        match connection.sock.peek(&mut byte) {
            Ok(0) => panic!("slow connection ended before the healthy workload completed"),
            Ok(_) => {} // TLS may have post-handshake data waiting for the client.
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("slow connection state check failed: {error}"),
        }
    }

    drop(slow);
    public_round_trip(address, config).expect("listener must recover after slow peers disconnect");
    server.shutdown().unwrap();
}

#[test]
#[ignore = "release-mode production workload benchmark"]
fn saturated_writer_queue_rejects_quickly_and_recovers() {
    let environment = TestEnvironment::with_profile(TestProfile::QueuePressure).unwrap();
    let server = environment.start_server().unwrap();
    let writer = server.writer_handle();
    let pressure = server.saturate_writer_queue().unwrap();
    assert_eq!(pressure.metrics().pending, 2);

    let wall = Instant::now();
    let mut latencies = Vec::with_capacity(WRITER_REJECTIONS);
    let mut unexpected = 0;
    for _ in 0..WRITER_REJECTIONS {
        let started = Instant::now();
        let result = writer.call(|_| Ok(()));
        latencies.push(started.elapsed());
        if !matches!(result, Err(foks_server::Error::WriterQueue)) {
            unexpected += 1;
        }
    }
    report(
        "writer_queue_rejection",
        &latencies,
        wall.elapsed(),
        unexpected,
    );
    assert_eq!(unexpected, 0, "a saturated queue must fail closed");

    let metrics = pressure.drain().unwrap();
    assert_eq!(metrics.pending, 0);
    assert!(metrics.rejected >= WRITER_REJECTIONS as u64);
    writer
        .call(|_| Ok(()))
        .expect("writer must accept work after the queue drains");
    server.shutdown().unwrap();
}

fn client_config(roots: rustls::RootCertStore) -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

fn connect(
    address: std::net::SocketAddr,
    config: Arc<rustls::ClientConfig>,
) -> std::io::Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>> {
    let tcp = TcpStream::connect(address)?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(10)))?;
    let connection = rustls::ClientConnection::new(
        config,
        rustls::pki_types::ServerName::try_from("localhost").expect("static server name"),
    )
    .map_err(std::io::Error::other)?;
    Ok(rustls::StreamOwned::new(connection, tcp))
}

fn public_round_trip(
    address: std::net::SocketAddr,
    config: Arc<rustls::ClientConfig>,
) -> Result<(), String> {
    let mut connection = connect(address, config).map_err(|error| error.to_string())?;
    let request =
        foks_rpc::encode_load_user_chain_request(&[1; 33], 0).map_err(|error| error.to_string())?;
    connection
        .write_all(&request)
        .and_then(|()| connection.flush())
        .map_err(|error| error.to_string())?;
    match foks_rpc::read_void_response(&mut connection, DEFAULT_MAX_FRAME_LENGTH, 0) {
        Err(RpcError::RemoteStatus { code: 1020, .. }) => Ok(()),
        result => Err(format!("unexpected public response: {result:?}")),
    }
}

fn report(name: &str, latencies: &[Duration], wall: Duration, errors: usize) {
    let mut sorted = latencies.to_vec();
    sorted.sort_unstable();
    let throughput = if wall.is_zero() {
        0.0
    } else {
        latencies.len() as f64 / wall.as_secs_f64()
    };
    println!(
        "BENCH name={name} samples={} errors={errors} wall_ms={} throughput_per_second={throughput:.1} p50_ns={} p95_ns={} p99_ns={} parallelism={}",
        latencies.len(),
        wall.as_millis(),
        percentile(&sorted, 50).as_nanos(),
        percentile(&sorted, 95).as_nanos(),
        percentile(&sorted, 99).as_nanos(),
        std::thread::available_parallelism().map_or(1, usize::from),
    );
}

fn percentile(sorted: &[Duration], percentage: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let index = (sorted.len() - 1) * percentage / 100;
    sorted[index]
}
