use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use foks_rpc::{Error as RpcError, DEFAULT_MAX_FRAME_LENGTH};
use foks_server_testkit::{oidc::TestOidcProvider, TestEnvironment, TestProfile};

#[test]
fn health_readiness_and_metrics_are_isolated_on_management_http() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let address = server.management_address();
    let health = get(address, "/healthz");
    assert!(health.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(health.ends_with("\r\n\r\nok\n"));
    let readiness = get(address, "/readyz");
    assert!(readiness.starts_with("HTTP/1.1 200 OK\r\n"));
    let metrics = get(address, "/metrics");
    assert!(metrics.starts_with("HTTP/1.1 200 OK\r\n"));
    for name in [
        "foks_requests_started_total",
        "foks_active_connections",
        "foks_writer_pending",
        "foks_writer_expired_total",
        "foks_writer_queue_wait_seconds_count",
        "foks_writer_execution_duration_seconds_count",
        "foks_request_duration_seconds_count",
        "foks_handler_duration_seconds_count",
        "foks_database_bytes",
        "foks_wal_bytes",
        "foks_rate_limited_requests_total",
        "foks_backup_successes_total",
        "foks_backup_duration_seconds_count",
        "foks_maintenance_attempts_total",
        "foks_maintenance_successes_total",
        "foks_maintenance_failures_total",
        "foks_reclaimed_uploads_total",
        "foks_expiry_deleted_rows_total",
        "foks_expiry_empty_passes_total",
        "foks_maintenance_warning",
        "foks_checkpoint_attempts_total",
        "foks_checkpoint_duration_seconds_bucket",
        "foks_checkpoint_frame_counts_available",
        "foks_checkpoint_remaining_frames",
        "foks_accept_errors_total{kind=\"emfile\"}",
        "foks_accept_errors_total{kind=\"other\"}",
        "foks_admission_rejected_total{reason=\"source_connections\"}",
        "foks_admission_rejected_total{reason=\"account_realtime_polls\"}",
        "foks_capacity_limit{resource=\"merkle_epoch\"} 65535\n",
        "foks_capacity_limit{resource=\"database_bytes\"}",
    ] {
        assert!(metrics.contains(name));
    }
    assert!(metric(&metrics, "foks_database_bytes") > 0.0);
    assert_eq!(metric(&metrics, "foks_storage_sample_failures_total"), 0.0);
    // Without OIDC the rollout is never sampled and its gauges are omitted.
    assert!(!metrics.contains("foks_oidc_rollout_mode"));
    assert_eq!(metric(&metrics, "foks_oidc_sample_failures_total"), 0.0);
    assert!(!metrics.contains(environment.root().to_string_lossy().as_ref()));
    server.run_maintenance().unwrap();
    let after = get(address, "/metrics");
    assert_eq!(metric(&after, "foks_maintenance_attempts_total"), 1.0);
    assert_eq!(metric(&after, "foks_maintenance_successes_total"), 1.0);
    assert_eq!(metric(&after, "foks_maintenance_failures_total"), 0.0);
    assert_eq!(metric(&after, "foks_maintenance_warning"), 0.0);
    assert!(metric(&after, "foks_last_maintenance_success_unixtime") > 0.0);
    assert_eq!(metric(&after, "foks_checkpoint_attempts_total"), 1.0);
    assert_eq!(metric(&after, "foks_checkpoint_complete_total"), 1.0);
    assert_eq!(
        metric(&after, "foks_checkpoint_frame_counts_available"),
        1.0
    );
    assert_eq!(metric(&after, "foks_checkpoint_remaining_frames"), 0.0);
    assert_eq!(
        metric(&after, "foks_checkpoint_duration_seconds_count"),
        1.0
    );
    assert!(get(address, "/missing").starts_with("HTTP/1.1 404 Not Found\r\n"));
    server.shutdown().unwrap();
}

#[test]
fn oidc_rollout_gauges_are_exported_when_oidc_is_configured() {
    let provider = TestOidcProvider::start();
    let environment = TestEnvironment::new().unwrap();
    let config = provider.config(environment.client_path("oidc", "secret").unwrap());
    // Starting with OIDC activates the rollout policy, so a sample exists.
    let server = environment.start_oidc_server(config).unwrap();
    let address = server.management_address();
    for _ in 0..2 {
        let metrics = get(address, "/metrics");
        assert!(metrics.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(metric(&metrics, "foks_oidc_rollout_mode") >= 1.0);
        assert_eq!(metric(&metrics, "foks_oidc_provider_blocked"), 0.0);
        assert_eq!(metric(&metrics, "foks_oidc_cohort_accounts"), 0.0);
        assert_eq!(metric(&metrics, "foks_oidc_sample_failures_total"), 0.0);
    }
    server.shutdown().unwrap();
}

#[test]
fn writer_panic_fails_health_and_reports_a_fatal_exit() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let address = server.management_address();
    let (sender, receiver) = std::sync::mpsc::channel();
    server.on_fatal_exit(move |thread| {
        let _ = sender.send(thread);
    });
    assert!(get(address, "/healthz").starts_with("HTTP/1.1 200 OK\r\n"));
    let writer = server.writer_handle();
    assert!(matches!(
        writer.call::<_, ()>(|_| panic!("injected writer panic")),
        Err(foks_server::Error::WriterQueue)
    ));
    // The foks-server binary closes its signal iterator from this observer
    // and exits non-zero.
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        foks_server::CoreThread::Writer
    );
    assert_eq!(server.fatal_thread(), Some(foks_server::CoreThread::Writer));
    let health = get(address, "/healthz");
    assert!(health.starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
    assert!(health.ends_with("\r\n\r\nwriter thread exited\n"));
    assert!(get(address, "/readyz").starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
    assert!(matches!(
        writer.call(|_| -> foks_server::Result<()> { unreachable!("writer ran after a panic") }),
        Err(foks_server::Error::WriterQueue)
    ));
    assert!(matches!(server.shutdown(), Err(foks_server::Error::Thread)));
}

#[test]
fn orderly_shutdown_is_not_a_fatal_exit() {
    let environment = TestEnvironment::with_profile(TestProfile::BackupAutomation).unwrap();
    let server = environment.start_server().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    server.on_fatal_exit(move |thread| {
        let _ = sender.send(thread);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.metrics().backup_successes == 0 {
        assert!(
            Instant::now() < deadline,
            "automatic backup did not complete"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    server.run_maintenance().unwrap();
    assert_eq!(server.fatal_thread(), None);
    server.shutdown().unwrap();
    // Dropping the server drops the unused observer and its sender.
    assert!(matches!(
        receiver.recv_timeout(Duration::from_secs(1)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn partial_checkpoint_is_successful_maintenance_and_metrics_do_not_enter_writer() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let reader = rusqlite::Connection::open_with_flags(
        environment.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    reader.execute_batch("BEGIN; SELECT * FROM names;").unwrap();
    let writer = server.writer_handle();
    writer
        .call(|db| {
            db.reserve_name(b"checkpoint-private-account", &[91; 17], 1, 1, 10_000_000)?;
            Ok(())
        })
        .unwrap();
    let (_, report) = server.run_maintenance().unwrap();
    let accepted = writer.metrics().accepted;
    let text = get(server.management_address(), "/metrics");
    assert_eq!(writer.metrics().accepted, accepted);
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(report.busy, 0);
    assert_eq!(
        report.outcome(),
        foks_server_db::CheckpointOutcome::Deferred
    );
    assert_eq!(metric(&text, "foks_maintenance_successes_total"), 1.0);
    assert_eq!(metric(&text, "foks_checkpoint_deferred_total"), 1.0);
    assert_eq!(
        metric(&text, "foks_expiry_deleted_rows_total{kind=\"names\"}"),
        1.0
    );
    assert_eq!(
        metric(&text, "foks_expiry_empty_passes_total{kind=\"names\"}"),
        0.0
    );
    for kind in foks_server::ExpiryKind::ALL {
        assert!(text.contains(&format!(
            "foks_expiry_deleted_rows_total{{kind=\"{}\"}}",
            kind.label()
        )));
        assert!(text.contains(&format!(
            "foks_expiry_empty_passes_total{{kind=\"{}\"}}",
            kind.label()
        )));
    }
    assert_eq!(metric(&text, "foks_checkpoint_complete_total"), 0.0);
    assert_eq!(metric(&text, "foks_checkpoint_busy_total"), 0.0);
    assert!(metric(&text, "foks_checkpoint_remaining_frames") > 0.0);
    assert!(metric(&text, "foks_last_checkpoint_observation_unixtime") > 0.0);
    assert_eq!(metric(&text, "foks_last_checkpoint_complete_unixtime"), 0.0);
    let buckets: Vec<f64> = text
        .lines()
        .filter(|line| line.starts_with("foks_checkpoint_duration_seconds_bucket{"))
        .map(|line| line.split_whitespace().last().unwrap().parse().unwrap())
        .collect();
    assert_eq!(buckets.len(), 9);
    assert!(buckets.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(
        buckets[8],
        metric(&text, "foks_checkpoint_duration_seconds_count")
    );
    assert!(!text.contains("checkpoint-private-account"));
    assert!(!text.contains(environment.root().to_string_lossy().as_ref()));
    server.run_maintenance().unwrap();
    let text = get(server.management_address(), "/metrics");
    assert_eq!(metric(&text, "foks_checkpoint_complete_total"), 1.0);
    assert_eq!(
        metric(&text, "foks_expiry_deleted_rows_total{kind=\"names\"}"),
        1.0
    );
    assert_eq!(
        metric(&text, "foks_expiry_empty_passes_total{kind=\"names\"}"),
        1.0
    );
    assert_eq!(metric(&text, "foks_checkpoint_remaining_frames"), 0.0);
    server.shutdown().unwrap();
}

#[test]
fn request_rate_limit_returns_typed_status_and_counts_rejection() {
    let environment = TestEnvironment::with_profile(TestProfile::RateLimited).unwrap();
    let server = environment.start_server().unwrap();
    let mut connection = connect_public(server.addresses().public_services, server.service_roots());
    let request = foks_rpc::encode_get_host_id_request_at(0).unwrap();
    connection.write_all(&request).unwrap();
    connection.flush().unwrap();
    foks_rpc::read_response(&mut connection, DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
    // LoadUserChain is a User method, which the public listener refuses.
    let request = foks_rpc::encode_load_user_chain_request(&[1; 33], 0).unwrap();
    connection.write_all(&request).unwrap();
    connection.flush().unwrap();
    assert!(matches!(
        foks_rpc::read_void_response(&mut connection, DEFAULT_MAX_FRAME_LENGTH, 0),
        Err(RpcError::RemoteStatus { code: 1020, .. })
    ));
    // Clients number calls per connection, so the refusal of the third call
    // must carry that call's sequence number, not 0.
    let request = foks_rpc::encode_get_host_id_request_at(2).unwrap();
    connection.write_all(&request).unwrap();
    connection.flush().unwrap();
    assert!(matches!(
        foks_rpc::read_response(&mut connection, DEFAULT_MAX_FRAME_LENGTH, 2),
        Err(RpcError::RemoteStatus { code: 1012, .. })
    ));
    assert_eq!(server.metrics().rate_limited_requests, 1);
    let duration_deadline = Instant::now() + Duration::from_secs(2);
    while server.metrics().request_duration_observations < 3 {
        assert!(
            Instant::now() < duration_deadline,
            "request duration observation did not complete"
        );
        std::thread::yield_now();
    }
    assert_eq!(server.metrics().request_duration_observations, 3);
    assert_eq!(server.metrics().handler_duration_observations, 2);
    let outcome_deadline = Instant::now() + Duration::from_secs(2);
    while server.metrics().responses_completed < 3 {
        assert!(
            Instant::now() < outcome_deadline,
            "response completion was not counted"
        );
        std::thread::yield_now();
    }
    let text = get(server.management_address(), "/metrics");
    let outcomes: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("foks_requests_total{"))
        .collect();
    assert_eq!(
        outcomes,
        [
            "foks_requests_total{family=\"Reg\",status=\"0\"} 1",
            "foks_requests_total{family=\"unrouted\",status=\"1012\"} 1",
            "foks_requests_total{family=\"unrouted\",status=\"1020\"} 1",
        ]
    );
    assert_eq!(metric(&text, "foks_reader_pool_exhausted_total"), 0.0);
    assert_eq!(
        metric(
            &text,
            "foks_admission_rejected_total{reason=\"request_rate\"}"
        ),
        1.0
    );
    let held = (0..20)
        .map(|_| TcpStream::connect(server.addresses().public_services).unwrap())
        .collect::<Vec<_>>();
    let deadline = Instant::now() + Duration::from_secs(2);
    while server.metrics().rate_limited_connections == 0 {
        assert!(
            Instant::now() < deadline,
            "connection rate limit did not fire"
        );
        std::thread::yield_now();
    }
    let text = get(server.management_address(), "/metrics");
    assert!(
        metric(
            &text,
            "foks_admission_rejected_total{reason=\"connection_rate\"}"
        ) >= 1.0
    );
    // Loopback peers are exempt from the per-source connection allowance.
    assert_eq!(
        metric(
            &text,
            "foks_admission_rejected_total{reason=\"source_connections\"}"
        ),
        0.0
    );
    drop(held);
    server.shutdown().unwrap();
}

#[test]
fn automatic_backups_are_complete_and_retained() {
    let environment = TestEnvironment::with_profile(TestProfile::BackupAutomation).unwrap();
    let server = environment.start_server().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.metrics().backup_successes < 3 {
        assert!(
            Instant::now() < deadline,
            "automatic backup did not complete"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.metrics().backup_failures, 0);
    let duration_deadline = Instant::now() + Duration::from_secs(2);
    while server.metrics().backup_duration_observations < server.metrics().backup_successes {
        assert!(
            Instant::now() < duration_deadline,
            "backup duration observation did not complete"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        server.metrics().backup_duration_observations,
        server.metrics().backup_successes
    );
    server.shutdown().unwrap();

    let backup_root = environment.root().join("backup/automatic");
    let mut backups = std::fs::read_dir(&backup_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("backup-"))
        })
        .collect::<Vec<_>>();
    backups.sort();
    assert_eq!(backups.len(), 2);
    for backup in backups {
        assert!(backup.join("foks-server.sqlite").is_file());
        assert!(backup.join("key-manifest.txt").is_file());
        assert!(backup.join("keys/key-encryption.key").is_file());
        assert!(backup.join("keys/host.key").is_file());
    }
}

fn get(address: std::net::SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    stream.flush().unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn metric(response: &str, name: &str) -> f64 {
    response
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name} ")))
        .unwrap_or_else(|| panic!("missing metric {name}"))
        .parse()
        .unwrap()
}

fn connect_public(
    address: std::net::SocketAddr,
    roots: rustls::RootCertStore,
) -> rustls::StreamOwned<rustls::ClientConnection, TcpStream> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let tcp = TcpStream::connect(address).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let connection = rustls::ClientConnection::new(
        config,
        rustls::pki_types::ServerName::try_from("localhost").unwrap(),
    )
    .unwrap();
    rustls::StreamOwned::new(connection, tcp)
}

/// A refusal that ends the session is followed by an orderly close: a peer
/// that pipelined more frames behind the refused call still reads the
/// refusal, then a TLS close_notify rather than a reset.
#[test]
fn a_closing_refusal_reaches_a_pipelining_peer_before_an_orderly_close() {
    let environment = TestEnvironment::with_profile(TestProfile::RateLimited).unwrap();
    let server = environment.start_server().unwrap();
    let mut connection = connect_public(server.addresses().public_services, server.service_roots());
    // Two calls fit the request bucket; the third is refused and the fourth
    // is already on the wire when the server ends the session.
    let mut pipelined = Vec::new();
    for sequence in 0..4 {
        pipelined.extend(foks_rpc::encode_get_host_id_request_at(sequence).unwrap());
    }
    connection.write_all(&pipelined).unwrap();
    connection.flush().unwrap();
    for sequence in 0..2 {
        foks_rpc::read_response(&mut connection, DEFAULT_MAX_FRAME_LENGTH, sequence).unwrap();
    }
    assert!(matches!(
        foks_rpc::read_response(&mut connection, DEFAULT_MAX_FRAME_LENGTH, 2),
        Err(RpcError::RemoteStatus { code: 1012, .. })
    ));
    let mut byte = [0];
    assert_eq!(connection.read(&mut byte).unwrap(), 0);
    server.shutdown().unwrap();
}
