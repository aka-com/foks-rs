use foks_proto::{KvDirectoryVersion, KvDirentVersion, KvPathVersionVector};

#[test]
fn version_vector_equivalence_is_order_independent_but_rejects_duplicates() {
    let entry_a = KvDirentVersion {
        id: [0x11; 16],
        version: 2,
    };
    let entry_b = KvDirentVersion {
        id: [0x22; 16],
        version: 3,
    };
    let left = KvPathVersionVector {
        root_version: 4,
        directories: vec![KvDirectoryVersion {
            id: [0x33; 16],
            version: 5,
            entries: vec![entry_a, entry_b],
        }],
    };
    let reordered = KvPathVersionVector {
        root_version: 4,
        directories: vec![KvDirectoryVersion {
            id: [0x33; 16],
            version: 5,
            entries: vec![entry_b, entry_a],
        }],
    };
    assert!(left.equivalent(&reordered));

    let duplicate = KvPathVersionVector {
        root_version: 4,
        directories: vec![KvDirectoryVersion {
            id: [0x33; 16],
            version: 5,
            entries: vec![entry_a, entry_a],
        }],
    };
    assert!(!left.equivalent(&duplicate));
}
