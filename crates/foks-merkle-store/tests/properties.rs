use foks_merkle_store::{apply_commit, prepare, LeafChange, MemoryStore, EMPTY_ROOT};

#[test]
fn insertion_order_does_not_change_the_root() {
    let changes = (0..64u8)
        .map(|index| {
            let mut key = [0; 32];
            key[0] = index;
            let mut value = [0; 32];
            value[31] = index;
            LeafChange::Set { key, value }
        })
        .collect::<Vec<_>>();
    let expected = prepare(&MemoryStore::default(), EMPTY_ROOT, &changes)
        .unwrap()
        .root;
    for rotation in 0..changes.len() {
        let mut reordered = changes.clone();
        reordered.rotate_left(rotation);
        if rotation % 2 == 1 {
            reordered.reverse();
        }
        assert_eq!(
            prepare(&MemoryStore::default(), EMPTY_ROOT, &reordered)
                .unwrap()
                .root,
            expected
        );
    }
}

#[test]
fn reopen_from_only_immutable_nodes_is_stable() {
    let changes = [
        LeafChange::Set {
            key: [0x11; 32],
            value: [0x22; 32],
        },
        LeafChange::Set {
            key: [0xee; 32],
            value: [0xdd; 32],
        },
    ];
    let mut store = MemoryStore::default();
    let first = prepare(&store, EMPTY_ROOT, &changes).unwrap();
    apply_commit(&mut store, &first).unwrap();
    let reopened = store.clone();
    let second = prepare(&reopened, first.root, &[]).unwrap();
    assert_eq!(second.root, first.root);
    assert!(second.nodes.is_empty());
}
