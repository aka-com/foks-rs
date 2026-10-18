use foks_metadata_core::{page::*, page_catalog::PageStatement, Error, Value};

const INTEGER: Column = Column {
    kind: PageKind::Integer,
    maximum_bytes: 8,
    nullable: false,
};
const BLOB: Column = Column {
    kind: PageKind::Blob,
    maximum_bytes: 64,
    nullable: true,
};
const TEXT: Column = Column {
    kind: PageKind::Text,
    maximum_bytes: 64,
    nullable: false,
};
fn spec() -> PageSpec {
    PageSpec {
        id: 99,
        sql: "fixture",
        parameters: &[INTEGER, BLOB, TEXT],
        columns: &[INTEGER, BLOB, TEXT],
        maximum_rows: 64,
        maximum_bytes: MAX_PAGE_BYTES as u32,
        integer_ranges: &[],
        limit_parameter: None,
    }
}
fn row(n: i64) -> [Cell<'static>; 3] {
    [
        Cell::Integer(n),
        Cell::Blob(b"exact\0bytes"),
        Cell::Text("λ"),
    ]
}

#[test]
fn exact_integers_and_null_empty_distinctions_roundtrip() {
    for n in [
        i64::MIN,
        -9_007_199_254_740_993,
        -1,
        0,
        9_007_199_254_740_993,
        i64::MAX,
    ] {
        let mut encoded = PageEncoder::new(spec(), 64).unwrap();
        encoded.push(&row(n)).unwrap();
        encoded
            .push(&[Cell::Integer(n), Cell::Null, Cell::Text("")])
            .unwrap();
        encoded
            .push(&[Cell::Integer(n), Cell::Blob(b""), Cell::Text("")])
            .unwrap();
        let page = encoded.finish().unwrap();
        let rows = page
            .rows()
            .map(|r| r.cells().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(rows[0], row(n));
        assert_eq!(rows[1][1], Cell::Null);
        assert_eq!(rows[2][1], Cell::Blob(b""));
        assert_eq!(&page.as_bytes()[25..33], &n.to_le_bytes());
    }
}

#[test]
fn all_row_boundaries_and_no_partial_page_on_overflow() {
    for count in [0, 1, 63, 64] {
        let mut encoded = PageEncoder::new(spec(), 64).unwrap();
        for n in 0..count {
            encoded.push(&row(n)).unwrap();
        }
        let page = encoded.finish().unwrap();
        assert_eq!(page.len(), count as usize);
        assert_eq!(page.rows().count(), count as usize);
    }
    let mut encoded = PageEncoder::new(spec(), 1).unwrap();
    encoded.push(&row(1)).unwrap();
    assert!(matches!(encoded.push(&row(2)), Err(Error::Bounds)));
    assert_eq!(encoded.finish().unwrap().len(), 1);
}

#[test]
fn rejects_every_truncation_header_change_and_trailing_bytes() {
    let mut encoded = PageEncoder::new(spec(), 1).unwrap();
    encoded.push(&row(1)).unwrap();
    let bytes = encoded.finish().unwrap().into_bytes();
    for end in 0..bytes.len() {
        assert!(
            PageBuffer::decode(bytes[..end].to_vec(), &spec(), EnvelopeKind::Rows).is_err(),
            "{end}"
        );
    }
    for index in [0, 4, 6, 8, 12, 14, 16, 20, 24] {
        let mut bad = bytes.clone();
        bad[index] = 255;
        assert!(
            PageBuffer::decode(bad, &spec(), EnvelopeKind::Rows).is_err(),
            "{index}"
        );
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(PageBuffer::decode(extra, &spec(), EnvelopeKind::Rows).is_err());
    // Even a self-consistent declared body length may not contain extra cells.
    let mut extra = bytes;
    extra.push(0);
    let n = (extra.len() - HEADER_BYTES) as u32;
    extra[16..20].copy_from_slice(&n.to_le_bytes());
    assert!(PageBuffer::decode(extra, &spec(), EnvelopeKind::Rows).is_err());
}

#[test]
fn malformed_utf8_unknown_null_and_oversized_claims_fail_before_copy() {
    let mut encoded = PageEncoder::new(spec(), 1).unwrap();
    encoded
        .push(&[Cell::Integer(0), Cell::Null, Cell::Text("a")])
        .unwrap();
    let bytes = encoded.finish().unwrap().into_bytes();
    let mut bad = bytes.clone();
    *bad.last_mut().unwrap() = 255;
    assert!(PageBuffer::decode(bad, &spec(), EnvelopeKind::Rows).is_err());
    let mut bad = bytes.clone();
    bad[34] = 0;
    assert!(PageBuffer::decode(bad, &spec(), EnvelopeKind::Rows).is_err());
    let mut bad = bytes;
    bad[35..39].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(PageBuffer::decode(bad, &spec(), EnvelopeKind::Rows).is_err());
}

#[test]
fn maximum_envelope_is_exact_and_impossible_contracts_fail_activation() {
    let huge_columns = Box::leak(Box::new([Column {
        kind: PageKind::Blob,
        maximum_bytes: (MAX_PAGE_BYTES - HEADER_BYTES - 5) as u32,
        nullable: false,
    }]));
    let huge = PageSpec {
        columns: huge_columns,
        maximum_rows: 64,
        ..spec()
    };
    assert_eq!(huge.effective_rows().unwrap(), 1);
    let payload = vec![42; MAX_PAGE_BYTES - HEADER_BYTES - 5];
    let mut encoded = PageEncoder::new(huge, 1).unwrap();
    encoded.push(&[Cell::Blob(&payload)]).unwrap();
    assert_eq!(encoded.finish().unwrap().as_bytes().len(), MAX_PAGE_BYTES);
    let impossible = PageSpec {
        maximum_bytes: (MAX_PAGE_BYTES - 1) as u32,
        ..huge
    };
    assert!(impossible.validate().is_err());
    assert!(PageSpec {
        maximum_rows: 0,
        ..spec()
    }
    .validate()
    .is_err());
    assert!(PageSpec {
        maximum_bytes: u32::MAX,
        ..spec()
    }
    .validate()
    .is_err());
    assert!(PageSpec {
        columns: &[Column {
            kind: PageKind::Blob,
            maximum_bytes: u32::MAX,
            nullable: false
        }],
        ..spec()
    }
    .validate()
    .is_err());
}

#[test]
fn parameter_semantics_limit_and_binary_identity_are_enforced() {
    let statement = PageStatement::ProbeTypes;
    let s = statement.spec();
    let values = [
        Value::Integer(64),
        Value::Integer(i64::MIN),
        Value::Blob(vec![1, 2]),
        Value::Text("λ".into()),
        Value::Null,
        Value::Integer(64),
    ];
    let bytes = encode_parameters(&s, &values).unwrap();
    let (actual, decoded) = decode_parameters(bytes.clone()).unwrap();
    assert_eq!(actual, statement);
    assert_eq!(decoded, values);
    for (index, value) in [(0, -1), (0, 65), (5, 0), (5, 65)] {
        let mut bad = values.clone();
        bad[index] = Value::Integer(value);
        assert!(encode_parameters(&s, &bad).is_err());
    }
    let mut bad = bytes;
    bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_parameters(bad).is_err());
}

#[test]
fn catalog_artifact_binds_all_contracts_and_exact_range_integers() {
    let artifact = catalog_artifact().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&artifact).unwrap();
    assert_eq!(value["descriptor"]["abi_version"], 1);
    assert_eq!(
        value["descriptor"]["statements"].as_array().unwrap().len(),
        PageStatement::ALL.len()
    );
    assert_eq!(value["sha256"].as_str().unwrap().len(), 64);
    for s in PageStatement::ALL {
        let spec = s.spec();
        assert!(spec.effective_rows().unwrap() > 0);
        assert!(spec.maximum_row_bytes().unwrap() + HEADER_BYTES <= spec.maximum_bytes as usize);
    }
    assert!(value["descriptor"]["statements"][0]["integer_ranges"][0]["minimum"].is_string());
}

#[cfg(feature = "native")]
#[test]
fn admission_failure_precedes_bindings_and_sql_prepare_and_never_returns_empty_success() {
    use foks_metadata_core::{native::NativeSql, Sql};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    struct Reject(AtomicUsize);
    impl foks_snowpack::AllocationBudget for Reject {
        fn reserve(&self, bytes: usize) -> bool {
            self.0.fetch_add(bytes, Ordering::Relaxed);
            false
        }
    }
    let mut connection = rusqlite::Connection::open_in_memory().unwrap();
    let tx = connection.transaction().unwrap();
    let mut host = NativeSql::new(&tx).unwrap();
    let params = [
        Value::Blob(vec![1; 33]),
        Value::Blob(vec![2; 16]),
        Value::Integer(0),
        Value::Integer(1),
    ];
    let budget = Arc::new(Reject(AtomicUsize::new(0)));
    let result = foks_snowpack::with_allocation_budget(budget.clone(), || {
        host.query_page(PageStatement::ListInitial, &params)
    });
    assert!(matches!(result, Err(Error::Bounds)));
    assert!(
        budget.0.load(Ordering::Relaxed)
            > PageStatement::ListInitial
                .spec()
                .maximum_row_bytes()
                .unwrap()
    );
    // This database deliberately has no list tables. Reaching prepare returns
    // Storage, proving the rejecting admission ran before SQL preparation.
    assert!(matches!(
        host.query_page(PageStatement::ListInitial, &params),
        Err(Error::Storage)
    ));
    let budget = Arc::new(Reject(AtomicUsize::new(0)));
    assert!(foks_snowpack::with_allocation_budget(budget, || PageEncoder::new(spec(), 1)).is_err());
    tx.rollback().unwrap();
}

#[test]
fn utf8_bom_is_payload_and_page_capacity_is_admitted_once() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    struct Counter(AtomicUsize);
    impl foks_snowpack::AllocationBudget for Counter {
        fn reserve(&self, bytes: usize) -> bool {
            self.0.fetch_add(bytes, Ordering::Relaxed);
            true
        }
    }
    let budget = Arc::new(Counter(AtomicUsize::new(0)));
    let page = foks_snowpack::with_allocation_budget(budget.clone(), || {
        let mut encoder = PageEncoder::new(spec(), 64).unwrap();
        for _ in 0..64 {
            encoder
                .push(&[
                    Cell::Integer(i64::MAX),
                    Cell::Null,
                    Cell::Text("\u{feff}é🙂"),
                ])
                .unwrap();
        }
        encoder.finish().unwrap()
    });
    assert_eq!(
        budget.0.load(Ordering::Relaxed),
        HEADER_BYTES + 64 * spec().maximum_row_bytes().unwrap()
    );
    for row in page.rows() {
        assert_eq!(row.cells().nth(2), Some(Cell::Text("\u{feff}é🙂")));
    }
}
