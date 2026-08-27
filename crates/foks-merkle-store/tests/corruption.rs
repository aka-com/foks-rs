use foks_merkle_store::{
    apply_commit, hash_node, prepare, proof, Error, LeafChange, MemoryStore, Node, NodeReader,
    NodeWriter, EMPTY_ROOT,
};

#[test]
fn missing_and_hash_mismatched_nodes_fail_closed() {
    let changes = [LeafChange::Set {
        key: [1; 32],
        value: [2; 32],
    }];
    let mut store = MemoryStore::default();
    let commit = prepare(&store, EMPTY_ROOT, &changes).unwrap();
    assert!(matches!(
        proof(&store, commit.root, [1; 32]),
        Err(Error::MissingNode(_))
    ));

    apply_commit(&mut store, &commit).unwrap();
    assert!(store.corrupt(&commit.root, vec![0xc0]));
    assert!(matches!(
        proof(&store, commit.root, [1; 32]),
        Err(Error::InvalidNode) | Err(Error::HashMismatch)
    ));
    assert!(prepare(&store, commit.root, &[]).is_err());
}

#[test]
fn malformed_node_shapes_are_rejected() {
    let root = [9; 32];
    let mut store = MemoryStore::default();
    assert!(!store.corrupt(&root, vec![0x91, 0x00]));
    assert!(matches!(
        proof(&store, root, [0; 32]),
        Err(Error::InvalidNode)
    ));
}

#[test]
fn swapped_children_and_duplicate_commands_fail_closed() {
    let changes = [
        LeafChange::Set {
            key: [0x10; 32],
            value: [1; 32],
        },
        LeafChange::Set {
            key: [0xf0; 32],
            value: [2; 32],
        },
    ];
    let mut store = MemoryStore::default();
    let commit = prepare(&store, EMPTY_ROOT, &changes).unwrap();
    apply_commit(&mut store, &commit).unwrap();
    let encoded = store.get_node(&commit.root).unwrap().unwrap();
    let Node::Interior {
        prefix_bit_start,
        prefix_bit_count,
        prefix,
        left,
        right,
    } = Node::decode(&encoded).unwrap()
    else {
        panic!("two leaves must have an interior root")
    };
    let swapped = Node::Interior {
        prefix_bit_start,
        prefix_bit_count,
        prefix,
        left: right,
        right: left,
    };
    let swapped_hash = hash_node(&swapped).unwrap();
    store
        .put_node(swapped_hash, swapped.encoded().unwrap())
        .unwrap();
    assert!(matches!(
        prepare(&store, swapped_hash, &[]),
        Err(Error::InvalidNode)
    ));

    assert!(matches!(
        prepare(&store, commit.root, &[changes[0], changes[0]]),
        Err(Error::DuplicateKey)
    ));
}
