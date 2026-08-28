use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use foks_rpc::{Error as RpcError, DEFAULT_MAX_FRAME_LENGTH};
use foks_server_testkit::{TestEnvironment, TestProfile};

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
        "foks_writer_queue_wait_seconds_count",
        "foks_writer_execution_duration_seconds_count",
        "foks_request_duration_seconds_count",
        "foks_handler_duration_seconds_count",
        "foks_database_bytes",
        "foks_wal_bytes",
        "foks_rate_limited_requests_total",
        "foks_backup_successes_total",
        "foks_backup_duration_seconds_count",
    ] {
        assert!(metrics.contains(name));
    }
    assert!(metric(&metrics, "foks_database_bytes") > 0.0);
    assert_eq!(metric(&metrics, "foks_storage_sample_failures_total"), 0.0);
    assert!(!metrics.contains(environment.root().to_string_lossy().as_ref()));
    assert!(get(address, "/missing").starts_with("HTTP/1.1 404 Not Found\r\n"));
    server.shutdown().unwrap();
}

#[test]
fn request_rate_limit_returns_typed_status_and_counts_rejection() {
    let environment = TestEnvironment::with_profile(TestProfile::RateLimited).unwrap();
    let server = environment.start_server().unwrap();
    let mut connection = connect_public(server.addresses().public_services, server.service_roots());
    for _ in 0..2 {
        let request = foks_rpc::encode_load_user_chain_request(&[1; 33], 0).unwrap();
        connection.write_all(&request).unwrap();
        connection.flush().unwrap();
        assert!(matches!(
            foks_rpc::read_void_response(&mut connection, DEFAULT_MAX_FRAME_LENGTH, 0),
            Err(RpcError::RemoteStatus { code: 1020, .. })
        ));
    }
    let request = foks_rpc::encode_load_user_chain_request(&[1; 33], 0).unwrap();
    connection.write_all(&request).unwrap();
    connection.flush().unwrap();
    assert!(matches!(
        foks_rpc::read_void_response(&mut connection, DEFAULT_MAX_FRAME_LENGTH, 0),
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
