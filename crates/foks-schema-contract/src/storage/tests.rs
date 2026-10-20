use super::*;

fn table(name: &str) -> TableId {
    HOSTED_SMALL_V1.table(Component::Foks, name).unwrap()
}
fn valid(table: TableId) -> Vec<Dimension> {
    table
        .columns()
        .map(|column| match column.storage_type() {
            StorageType::Integer => Dimension::Integer,
            StorageType::Real => Dimension::Real,
            StorageType::Blob => Dimension::Blob { bytes: 0 },
            StorageType::Text => Dimension::TextUtf8 { bytes: 0 },
        })
        .collect()
}
fn field(table: TableId, name: &str) -> usize {
    table.columns().position(|c| c.name() == name).unwrap()
}
#[test]
fn pinned_inventory_excludes_unadmitted_owned_tables_and_validates_every_layout() {
    assert_eq!(HOSTED_SMALL_V1.name(), "hosted-metadata-small-v1");
    assert_eq!(HOSTED_SMALL_V1.required_encoding(), "UTF-8");
    assert_eq!(HOSTED_SMALL_V1.source_format(), 1);
    assert_eq!(HOSTED_SMALL_V1.source_pins().len(), 4);
    assert_eq!(
        HOSTED_SMALL_V1.descriptor(Component::Foks).tables().len(),
        99
    );
    assert_eq!(
        HOSTED_SMALL_V1.descriptor(Component::Foks).records().len(),
        283
    );
    assert_eq!(
        HOSTED_SMALL_V1
            .descriptor(Component::Authority)
            .tables()
            .len(),
        3
    );
    for name in [
        "sqlite_sequence",
        "sqlite_schema",
        "sqlite_stat1",
        "_cf_KV",
        "unknown",
    ] {
        assert!(HOSTED_SMALL_V1.table(Component::Foks, name).is_none());
    }
    for (c, expected_aliases) in [(Component::Foks, 5), (Component::Authority, 2)] {
        let mut aliases = 0;
        let component = HOSTED_SMALL_V1.descriptor(c);
        for table in component.tables() {
            assert!(table.columns().len() <= 32);
            for record in std::iter::once(table.record()).chain(table.indexes().map(|i| i.record()))
            {
                assert!(record.operands().len() <= 32);
                assert_eq!(record.table(), table.name());
            }
            let result = check_utf8_row_dimensions(table, &valid(table)).unwrap();
            assert_eq!(
                usize::from(result.physical_records_checked),
                1 + table.indexes().filter(|i| !i.table_btree_alias()).count()
            );
            aliases += table.indexes().filter(|i| i.table_btree_alias()).count();
        }
        assert_eq!(aliases, expected_aliases);
    }
}
#[test]
fn exact_count_type_null_and_field_precedence() {
    let t = table("kv_nodes");
    let mut values = valid(t);
    assert!(matches!(
        check_utf8_row_dimensions(t, &values[..4]),
        Err(Violation::Count { expected: 5, .. })
    ));
    values[0] = Dimension::Null;
    values[3] = Dimension::Blob { bytes: u64::MAX };
    assert_eq!(
        check_utf8_row_dimensions(t, &values),
        Err(Violation::Null { column: 0 })
    );
    values[0] = Dimension::TextUtf8 { bytes: 0 };
    assert!(matches!(
        check_utf8_row_dimensions(t, &values),
        Err(Violation::Type {
            column: 0,
            expected: StorageType::Blob,
            observed: ValueType::Text
        })
    ));
    values[0] = Dimension::Blob { bytes: 0 };
    assert_eq!(
        check_utf8_row_dimensions(t, &values),
        Err(Violation::FieldBytes {
            column: 3,
            observed: u64::MAX,
            limit: 65536
        })
    );
    values[3] = Dimension::Blob { bytes: 65536 };
    values[4] = Dimension::Null;
    assert!(check_utf8_row_dimensions(t, &values).is_ok());
    values[4] = Dimension::Real;
    assert!(matches!(
        check_utf8_row_dimensions(t, &values),
        Err(Violation::Type { column: 4, .. })
    ));
}
#[test]
fn every_byte_policy_has_minus_exact_plus_boundaries() {
    for c in [Component::Foks, Component::Authority] {
        for t in HOSTED_SMALL_V1.descriptor(c).tables() {
            for column in t.columns() {
                let Some(limit) = column.maximum_bytes() else {
                    continue;
                };
                let mut dimensions = valid(t);
                for n in [limit - 1, limit, limit + 1] {
                    dimensions[usize::from(column.ordinal())] = match column.storage_type() {
                        StorageType::Blob => Dimension::Blob { bytes: n },
                        StorageType::Text => Dimension::TextUtf8 { bytes: n },
                        _ => panic!("only byte fields have caps"),
                    };
                    let result = check_utf8_row_dimensions(t, &dimensions);
                    if n > limit {
                        assert!(
                            matches!(result, Err(Violation::FieldBytes { column: ordinal, .. }) if ordinal == column.ordinal())
                        );
                    } else {
                        assert!(result.is_ok(), "{} {}: {result:?}", t.name(), column.name());
                    }
                }
            }
        }
    }
}
#[test]
fn complete_table_bound_rejects_two_valid_maxima_and_checks_exact_threshold() {
    let t = table("kv_directories");
    let mut values = valid(t);
    let empty = check_utf8_row_dimensions(t, &values)
        .unwrap()
        .table_record_bytes;
    let cap = HOSTED_SMALL_V1.descriptor(Component::Foks).record_limit();
    values[0] = Dimension::Blob { bytes: 1_048_576 };
    for delta in [-1i64, 0, 1] {
        let bytes = (cap - empty - 1_048_576).checked_add_signed(delta).unwrap();
        values[1] = Dimension::Blob { bytes };
        let result = check_utf8_row_dimensions(t, &values);
        if delta <= 0 {
            assert_eq!(
                result.unwrap().table_record_bytes,
                cap.checked_add_signed(delta).unwrap()
            );
        } else {
            assert_eq!(
                result,
                Err(Violation::RecordBytes {
                    record: t.record(),
                    observed: cap + 1,
                    limit: cap
                })
            );
        }
    }
    values[1] = Dimension::Blob { bytes: 1_048_576 };
    assert!(matches!(
        check_utf8_row_dimensions(t, &values),
        Err(Violation::RecordBytes { .. })
    ));
}
fn operand(source: data::Source) -> data::Operand {
    data::Operand {
        source,
        descending: false,
        collation: "BINARY",
        key: true,
    }
}
#[test]
fn record_formula_covers_null_numeric_repeated_suffixes_and_all_expressions() {
    let dimensions = [
        Dimension::Null,
        Dimension::Integer,
        Dimension::Real,
        Dimension::Blob { bytes: 33 },
        Dimension::TextUtf8 { bytes: 7 },
    ];
    let columns: Vec<_> = (0..5)
        .map(|ordinal| operand(data::Source::Column { ordinal }))
        .collect();
    assert_eq!(
        record_bound(&columns, &dimensions),
        Some(9 + 45 + 8 + 8 + 33 + 7)
    );
    let values = [
        operand(data::Source::Column { ordinal: 3 }),
        operand(data::Source::Column { ordinal: 3 }),
        operand(data::Source::Rowid),
    ];
    assert_eq!(record_bound(&values, &dimensions), Some(9 + 27 + 66 + 8));
    for bytes in [0, 1, 2, 16, 17, 100] {
        assert_eq!(
            record_bound(
                &[operand(data::Source::NodeFileSlice {
                    ordinal: 0,
                    maximum_bytes: 16
                })],
                &[Dimension::Blob { bytes }]
            ),
            Some(18 + bytes.saturating_sub(1).min(16))
        );
    }
    assert_eq!(
        record_bound(
            &[operand(data::Source::ScopedHostOrEmpty {
                ordinal: 0,
                maximum_bytes: 33
            })],
            &[Dimension::Null]
        ),
        Some(18)
    );
    assert_eq!(
        record_bound(
            &[operand(data::Source::ScopedHostOrEmpty {
                ordinal: 3,
                maximum_bytes: 33
            })],
            &dimensions
        ),
        Some(51)
    );
    assert_eq!(
        record_bound(
            &[operand(data::Source::IntegerOne { maximum_bytes: 8 })],
            &[]
        ),
        Some(26)
    );
    assert_eq!(
        record_bound(
            &[operand(data::Source::Column { ordinal: 0 })],
            &[Dimension::Blob { bytes: u64::MAX }]
        ),
        None
    );
    assert_eq!(
        record_bound(&[operand(data::Source::Column { ordinal: 1 })], &[]),
        None
    );
}
#[test]
fn borrowed_utf8_dimensions_include_multibyte_nul_and_do_not_normalize_affinity() {
    let text = "é\0🚀";
    assert_eq!(
        BorrowedValue::TextUtf8(text).dimension(),
        Dimension::TextUtf8 { bytes: 7 }
    );
    assert_eq!(BorrowedValue::Real(f64::NAN).dimension(), Dimension::Real);
    let t = table("kv_nodes");
    let values = [
        BorrowedValue::Blob(b"uid"),
        BorrowedValue::Blob(b"node"),
        BorrowedValue::Integer(i64::MIN),
        BorrowedValue::Blob(b"bytes"),
        BorrowedValue::Null,
    ];
    assert_eq!(
        check_utf8_row(t, &values),
        check_utf8_row_dimensions(t, &values.map(BorrowedValue::dimension))
    );
    assert!(matches!(
        check_utf8_row(t, &values[..4]),
        Err(Violation::Count { .. })
    ));
    let nullable = data::Column {
        ordinal: 0,
        name: "nullable_pk",
        declared_type: "TEXT",
        storage_type: data::StorageType::Text,
        nullable: true,
        explicit_not_null: false,
        primary_key_ordinal: 1,
        integer_rowid_alias: false,
        maximum_bytes: Some(1_048_576),
        policy_source: "test",
    };
    assert!(validate_fields(&[nullable], &[Dimension::Null]).is_ok());
    let alias = data::Column {
        storage_type: data::StorageType::Integer,
        nullable: false,
        integer_rowid_alias: true,
        ..nullable
    };
    assert_eq!(
        validate_fields(&[alias], &[Dimension::Null]),
        Err(Violation::Null { column: 0 })
    );
    let real = data::Column {
        storage_type: data::StorageType::Real,
        maximum_bytes: None,
        ..nullable
    };
    assert!(validate_fields(&[real], &[Dimension::Real]).is_ok());
    assert!(matches!(
        validate_fields(&[real], &[Dimension::Integer]),
        Err(Violation::Type { .. })
    ));
}
#[test]
fn partial_index_and_expression_suffixes_remain_in_complete_measurement() {
    for name in ["kv_dirents", "team_members", "sso_policy"] {
        let t = table(name);
        let mut values = valid(t);
        if name == "team_members" {
            values[field(t, "scoped_host_id")] = Dimension::Null;
        }
        let result = check_utf8_row_dimensions(t, &values).unwrap();
        assert_eq!(
            usize::from(result.physical_records_checked),
            1 + t.indexes().filter(|i| !i.table_btree_alias()).count()
        );
        for index in t.indexes() {
            let bound = checked_record(index.record(), &values).unwrap();
            if !index.table_btree_alias() {
                assert!(bound <= result.maximum_index_bytes);
            }
        }
    }
}
