use foks_server::host::{bootstrap, load_or_bootstrap, BootstrapEndpoints, BootstrapInput};
use foks_server::keys::DirectoryKeyProvider;
use foks_server_db::{Config, Database};

#[test]
fn bootstrap_is_verified_atomic_and_byte_stable_across_restart() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("foks-server.sqlite");
    let key_path = temporary.path().join("keys");
    let input = BootstrapInput {
        canonical_name: "localhost".to_owned(),
        endpoints: BootstrapEndpoints {
            probe: "localhost:4430".to_owned(),
            public_services: "localhost:4431".to_owned(),
            authenticated: "localhost:4432".to_owned(),
        },
        ttl_seconds: 60,
        now_microseconds: 1_700_000_000_000_000,
    };

    let provider = DirectoryKeyProvider::open(&key_path, [0x77; 32]).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    let first = bootstrap(&mut database, &provider, &input).unwrap();
    assert!(first.created);
    let verified = foks_verify::verify_public_host("localhost", &first.probe_response).unwrap();
    assert_eq!(verified.snapshot.host_id(), first.host_id.as_bytes());
    assert_eq!(database.current_root().unwrap().unwrap().epoch, 1);

    drop(database);
    drop(provider);
    let provider = DirectoryKeyProvider::open(&key_path, [0x77; 32]).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    let second = bootstrap(&mut database, &provider, &input).unwrap();
    assert!(!second.created);
    assert_eq!(second.probe_response, first.probe_response);
    assert_eq!(second.delegated_tls_ca, first.delegated_tls_ca);
    assert_eq!(second.key_manifest, first.key_manifest);
    assert_eq!(
        database.host_bootstrap().unwrap().unwrap().probe_response,
        first.probe_response
    );
}

#[test]
fn existing_bootstrap_ignores_a_new_startup_clock_but_not_changed_endpoints() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("foks-server.sqlite");
    let key_path = temporary.path().join("keys");
    let mut input = BootstrapInput {
        canonical_name: "localhost".to_owned(),
        endpoints: BootstrapEndpoints {
            probe: "localhost:4430".to_owned(),
            public_services: "localhost:4431".to_owned(),
            authenticated: "localhost:4432".to_owned(),
        },
        ttl_seconds: 60,
        now_microseconds: 1,
    };
    let provider = DirectoryKeyProvider::open(&key_path, [0x66; 32]).unwrap();
    let mut database = Database::open(&database_path, Config::default()).unwrap();
    let first = load_or_bootstrap(&mut database, &provider, &input).unwrap();
    assert!(first.created);

    input.now_microseconds = u64::MAX;
    let restarted = load_or_bootstrap(&mut database, &provider, &input).unwrap();
    assert!(!restarted.created);
    assert_eq!(restarted.probe_response, first.probe_response);

    input.endpoints.public_services = "localhost:4441".to_owned();
    assert!(load_or_bootstrap(&mut database, &provider, &input).is_err());
}

#[test]
fn bootstrap_rejects_a_different_key_generation() {
    let temporary = tempfile::tempdir().unwrap();
    let mut database = Database::open(
        temporary.path().join("foks-server.sqlite"),
        Config::default(),
    )
    .unwrap();
    let input = BootstrapInput {
        canonical_name: "localhost".to_owned(),
        endpoints: BootstrapEndpoints {
            probe: "localhost:4430".to_owned(),
            public_services: "localhost:4431".to_owned(),
            authenticated: "localhost:4432".to_owned(),
        },
        ttl_seconds: 60,
        now_microseconds: 1,
    };
    let first = foks_server::keys::MemoryKeyProvider::default();
    bootstrap(&mut database, &first, &input).unwrap();
    let different = foks_server::keys::MemoryKeyProvider::default();
    assert!(bootstrap(&mut database, &different, &input).is_err());
    assert!(foks_verify::verify_public_host(
        "localhost",
        &database.host_bootstrap().unwrap().unwrap().probe_response
    )
    .is_ok());
}
