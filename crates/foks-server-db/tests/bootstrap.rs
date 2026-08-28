mod common;

use foks_server_db::{BootstrapService, Error, HostBootstrap};

#[test]
fn bootstrap_is_atomic_idempotent_and_conflict_safe() {
    let mut database = common::TestDatabase::new();
    let bootstrap = example();
    assert!(database.database.bootstrap_host(&bootstrap).unwrap());
    assert!(!database.database.bootstrap_host(&bootstrap).unwrap());
    assert_eq!(
        database
            .database
            .host_bootstrap()
            .unwrap()
            .unwrap()
            .probe_response,
        bootstrap.probe_response
    );
    assert_eq!(database.database.current_root().unwrap().unwrap().epoch, 1);

    let mut conflict = bootstrap.clone();
    conflict.key_manifest.push(9);
    assert!(matches!(
        database.database.bootstrap_host(&conflict),
        Err(Error::Invalid("conflicting host bootstrap"))
    ));
    assert_eq!(
        database
            .database
            .host_bootstrap()
            .unwrap()
            .unwrap()
            .key_manifest,
        bootstrap.key_manifest
    );
}

#[test]
fn invalid_bootstrap_leaves_no_partial_state() {
    let mut database = common::TestDatabase::new();
    let mut invalid = example();
    invalid.services.pop();
    assert!(matches!(
        database.database.bootstrap_host(&invalid),
        Err(Error::Invalid("host bootstrap"))
    ));
    assert!(database.database.host_bootstrap().unwrap().is_none());
    assert!(database.database.current_root().unwrap().is_none());
}

fn example() -> HostBootstrap {
    HostBootstrap {
        host_id: [vec![2], vec![1; 32]].concat(),
        canonical_name: "localhost".to_owned(),
        probe_response: b"probe".to_vec(),
        key_manifest: b"manifest".to_vec(),
        host_key_generation: [1; 16],
        hostchain_link_hash: [2; 32],
        exact_hostchain_link: b"hostchain".to_vec(),
        services: [1, 2, 5, 10, 12, 16]
            .into_iter()
            .map(|service_type| BootstrapService {
                service_type,
                endpoint: "localhost:4430".to_owned(),
                advertised_blob: b"endpoint".to_vec(),
            })
            .collect(),
        root_hash: [3; 32],
        root_node: [0; 32],
        root_epoch: 1,
        exact_root: b"root".to_vec(),
        exact_signed_root: b"signed-root".to_vec(),
        created_at: 1,
    }
}
