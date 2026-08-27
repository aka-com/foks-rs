mod common;

use foks_merkle_store::{proof, NodeReader};

#[test]
fn committed_nodes_reopen_and_generate_the_same_proof() {
    let mut database = common::TestDatabase::new();
    database.reserve(1_000_000);
    database.commit(None).unwrap();
    let root = database.database.current_root().unwrap().unwrap();
    let generated = proof(&database.database.node_reader(), root.root_node, [0x10; 32]).unwrap();

    let reopened = foks_server_db::Database::open(&database.path, Default::default()).unwrap();
    assert_eq!(
        proof(&reopened.node_reader(), root.root_node, [0x10; 32]).unwrap(),
        generated
    );
    assert!(reopened
        .node_reader()
        .get_node(&root.root_node)
        .unwrap()
        .is_some());
}
