use super::*;

fn fixture(sql: &str) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(sql).unwrap();
    connection
}
fn describe(sql: &str) -> Component {
    introspection::component(&fixture(sql), "foks").unwrap()
}
fn table<'a>(component: &'a Component, name: &str) -> &'a Table {
    component.tables.iter().find(|t| t.name == name).unwrap()
}
fn record<'a>(component: &'a Component, id: &str) -> &'a Record {
    component.records.iter().find(|r| r.id == id).unwrap()
}
fn column<'a>(component: &'a Component, name: &str, field: &str) -> &'a Column {
    table(component, name)
        .columns
        .iter()
        .find(|c| c.name == field)
        .unwrap()
}

#[test]
fn trusted_descriptors_are_reproducible_and_match_reviewed_source_inventory() {
    let first = generate().unwrap();
    assert_eq!(first, generate().unwrap());
    assert_eq!(first.len(), 4);
    for value in first.values() {
        assert!(value.len() <= ARTIFACT_LIMIT);
        assert!(value.ends_with(b"\n"));
        assert!(!value.ends_with(b"\n\n"));
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&first["storage-descriptors-manifest.json"]).unwrap();
    assert_eq!(manifest["source_hashes"]["foks_contract"], FOKS_59_HASH);
    assert_eq!(manifest["foks"]["tables"], 99);
    assert_eq!(manifest["foks"]["index_list_entries"], 189);
    assert_eq!(manifest["foks"]["table_btree_aliases"], 5);
    assert_eq!(manifest["foks"]["physical_records"], 283);
    assert_eq!(manifest["authority"]["tables"], 3);
    assert_eq!(manifest["authority"]["physical_records"], 4);
    assert_eq!(manifest["scope"], "trusted_descriptors_only");
    assert_eq!(manifest["files"].as_object().unwrap().len(), 3);
    for (name, value) in &first {
        if name != "storage-descriptors-manifest.json" {
            assert_eq!(manifest["files"][name], hash(value));
        }
    }
    let foks: serde_json::Value = serde_json::from_slice(&first["foks-records-v1.json"]).unwrap();
    let tables = foks["tables"].as_array().unwrap();
    let non_strict: Vec<_> = tables
        .iter()
        .filter(|t| t["strict"] == false)
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        non_strict,
        ["chunk_storage_binding", "chunk_storage_revision"]
    );
    let engine: serde_json::Value =
        serde_json::from_slice(&first["engine-ownership-v1.json"]).unwrap();
    assert_eq!(engine["objects"].as_array().unwrap().len(), 10);
    assert_eq!(
        engine["scope"],
        "engine_owned_not_application_storage_admission"
    );
    assert!(engine.get("records").is_none());
}

#[test]
fn rowid_aliases_desc_exception_and_pk_nullability_follow_actual_metadata() {
    let description=describe("CREATE TABLE alias(id INTEGER PRIMARY KEY, payload BLOB);\nCREATE TABLE descending(id INTEGER PRIMARY KEY DESC, payload BLOB);\nCREATE TABLE nullable(id TEXT PRIMARY KEY, payload BLOB);\nCREATE TABLE composite(a TEXT,b BLOB,PRIMARY KEY(a,b));\nCREATE TABLE strict_pk(id TEXT PRIMARY KEY, payload BLOB) STRICT;\nCREATE TABLE wr(a TEXT,b BLOB,PRIMARY KEY(a,b)) WITHOUT ROWID;");
    assert!(column(&description, "alias", "id").integer_rowid_alias);
    assert!(!column(&description, "alias", "id").nullable);
    assert!(table(&description, "alias").indexes.is_empty());
    assert!(!column(&description, "descending", "id").integer_rowid_alias);
    assert!(column(&description, "descending", "id").nullable);
    let descending = table(&description, "descending");
    assert_eq!(descending.indexes[0].origin, "pk");
    let operands = &record(&description, &descending.indexes[0].record_id).operands;
    assert_eq!(
        operands.iter().map(|o| o.source).collect::<Vec<_>>(),
        [Source::Column { ordinal: 0 }, Source::Rowid]
    );
    assert!(operands[0].descending);
    assert!(column(&description, "nullable", "id").nullable);
    assert!(column(&description, "composite", "a").nullable);
    assert!(column(&description, "composite", "b").nullable);
    assert!(!column(&description, "strict_pk", "id").nullable);
    assert!(!column(&description, "wr", "a").nullable);
    assert!(!column(&description, "wr", "b").nullable);
    // Confirm nullable ordinary PK semantics with SQLite, not just our DTO.
    let c=fixture("CREATE TABLE n(a TEXT,b BLOB,PRIMARY KEY(a,b));INSERT INTO n VALUES(NULL,NULL),(NULL,NULL)");
    assert_eq!(
        c.query_row("SELECT count(*) FROM n", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn without_rowid_table_alias_and_secondary_suffix_have_exact_physical_order() {
    let c=fixture("CREATE TABLE r(a TEXT,b INTEGER,c BLOB,PRIMARY KEY(b DESC,a)) STRICT, WITHOUT ROWID;CREATE INDEX r_c ON r(c,b DESC);");
    let description = introspection::component(&c, "foks").unwrap();
    let t = table(&description, "r");
    assert_eq!(description.records.len(), 2); // table plus secondary, no double-counted PK
    let table_fields = &record(&description, &t.record_id).operands;
    assert_eq!(
        table_fields.iter().map(|o| o.source).collect::<Vec<_>>(),
        [
            Source::Column { ordinal: 1 },
            Source::Column { ordinal: 0 },
            Source::Column { ordinal: 2 }
        ]
    );
    assert_eq!(
        table_fields.iter().map(|o| o.key).collect::<Vec<_>>(),
        [true, true, false]
    );
    assert_eq!(
        table_fields
            .iter()
            .map(|o| o.descending)
            .collect::<Vec<_>>(),
        [true, false, false]
    );
    let pk = t.indexes.iter().find(|i| i.origin == "pk").unwrap();
    assert!(pk.table_btree_alias);
    assert_eq!(pk.record_id, t.record_id);
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM main.sqlite_schema WHERE name=?1",
            [&pk.name],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let rowid = fixture("CREATE TABLE p(a TEXT PRIMARY KEY)");
    let sql: Option<String> = rowid
        .query_row(
            "SELECT sql FROM main.sqlite_schema WHERE type='index'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(sql.is_none()); // A real autoindex row with NULL SQL, unlike the WR alias.

    let secondary = t.indexes.iter().find(|i| i.name == "r_c").unwrap();
    let fields = &record(&description, &secondary.record_id).operands;
    assert_eq!(
        fields.iter().map(|o| o.source).collect::<Vec<_>>(),
        [
            Source::Column { ordinal: 2 },
            Source::Column { ordinal: 1 },
            Source::Column { ordinal: 0 }
        ]
    );
    assert_eq!(
        fields.iter().map(|o| o.key).collect::<Vec<_>>(),
        [true, true, false]
    );
    assert!(
        matches!(&t.locator,Locator::PrimaryKey{operands} if operands.len()==2 && operands[0].descending)
    );
    let actual: Vec<(i64, String, bool, bool)> = c
        .prepare("PRAGMA index_xinfo(r_c)")
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get(1)?,
                r.get(4)?,
                r.get::<_, i64>(3)? == 1,
                r.get::<_, i64>(5)? == 1,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        actual,
        [
            (2, "BINARY".into(), false, true),
            (1, "BINARY".into(), true, true),
            (0, "BINARY".into(), false, false)
        ]
    );
}

#[test]
fn rowid_locator_uses_unshadowed_spelling_and_refuses_complete_shadowing() {
    let d = describe("CREATE TABLE a(_rowid_ BLOB);CREATE TABLE b(_rowid_ BLOB,rowid TEXT);");
    assert_eq!(
        table(&d, "a").locator,
        Locator::Rowid {
            spelling: "rowid".into()
        }
    );
    assert_eq!(
        table(&d, "b").locator,
        Locator::Rowid {
            spelling: "oid".into()
        }
    );
    assert!(introspection::component(
        &fixture("CREATE TABLE x(_rowid_ BLOB,RoWiD TEXT,OID INTEGER)"),
        "foks"
    )
    .unwrap_err()
    .contains("shadowed"));
}

#[test]
fn exact_expression_registry_and_partial_indexes_keep_all_operands() {
    let c=fixture("CREATE TABLE kv_dirents(uid BLOB,node_id BLOB,exact_dirent BLOB);\nCREATE INDEX kv_dirent_file_reference\n    ON kv_dirents(uid, substr(node_id, 2, 16))\n    WHERE substr(node_id, 1, 1) = X'02';\nCREATE TABLE team_members(team_id BLOB,party_id BLOB,scoped_host_id BLOB,source_role_type INTEGER,source_visibility INTEGER);\nCREATE UNIQUE INDEX team_members_identity ON team_members(\n    team_id, party_id, coalesce(scoped_host_id, X''), source_role_type, source_visibility\n);\nCREATE TABLE sso_policy(host BLOB);\nCREATE UNIQUE INDEX sso_single_host_policy ON sso_policy((1));");
    let d = introspection::component(&c, "foks").unwrap();
    policy::require_expressions(&d).unwrap();
    let file = table(&d, "kv_dirents").indexes.first().unwrap();
    assert!(file.partial);
    assert_eq!(file.partial_policy, "check_record_bound_for_every_row");
    assert_eq!(
        record(&d, &file.record_id)
            .operands
            .iter()
            .map(|o| o.source)
            .collect::<Vec<_>>(),
        [
            Source::Column { ordinal: 0 },
            Source::NodeFileSlice {
                ordinal: 1,
                maximum_bytes: 16
            },
            Source::Rowid
        ]
    );
    let host = table(&d, "team_members").indexes.first().unwrap();
    assert_eq!(
        record(&d, &host.record_id).operands[2].source,
        Source::ScopedHostOrEmpty {
            ordinal: 2,
            maximum_bytes: 33
        }
    );
    let one = table(&d, "sso_policy").indexes.first().unwrap();
    assert_eq!(
        record(&d, &one.record_id).operands[0].source,
        Source::IntegerOne { maximum_bytes: 8 }
    );
    // SQL expression semantics are checked independently of a prospective R1 evaluator.
    assert_eq!(c.query_row("SELECT length(coalesce(NULL,X'')),length(coalesce(zeroblob(33),X'')),typeof(1),length(substr(zeroblob(17),2,16))",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?))).unwrap(),(0,33,"integer".into(),16));
    c.execute_batch("DROP INDEX sso_single_host_policy")
        .unwrap();
    assert!(policy::require_expressions(&introspection::component(&c, "foks").unwrap()).is_err());
    c.execute_batch("CREATE UNIQUE INDEX sso_single_host_policy ON sso_policy((2))")
        .unwrap();
    assert!(introspection::component(&c, "foks")
        .unwrap_err()
        .contains("unregistered expression"));
}

#[test]
fn byte_overrides_are_exact_component_table_and_column_matches() {
    let c=fixture("CREATE TABLE kv_nodes(exact_node BLOB);CREATE TABLE merkle_nodes(exact_node BLOB);CREATE TABLE kv_dirents(exact_dirent BLOB);CREATE TABLE team_members(scoped_host_id BLOB);");
    let d = introspection::component(&c, "foks").unwrap();
    policy::require_overrides(&d).unwrap();
    assert_eq!(
        column(&d, "kv_nodes", "exact_node").bytes.maximum_bytes,
        Some(65_536)
    );
    assert_eq!(
        column(&d, "merkle_nodes", "exact_node").bytes.maximum_bytes,
        Some(VALUE_LIMIT)
    );
    assert_eq!(
        column(&d, "kv_dirents", "exact_dirent").bytes.maximum_bytes,
        Some(4096)
    );
    let other = introspection::component(&c, "qualification_authority_v1").unwrap();
    assert_eq!(
        column(&other, "kv_nodes", "exact_node").bytes.maximum_bytes,
        Some(VALUE_LIMIT)
    );
    assert!(
        introspection::component(&fixture("CREATE TABLE kv_nodes(exact_node TEXT)"), "foks")
            .is_err()
    );
    assert!(
        policy::require_overrides(&describe("CREATE TABLE unrelated(exact_node BLOB)")).is_err()
    );
}

#[test]
fn unsupported_layouts_and_dimensions_fail_closed() {
    for sql in [
        "CREATE TABLE t(x NUMERIC)",
        "CREATE TABLE t(x ANY) STRICT",
        "CREATE TABLE t(x INTEGER,y INTEGER GENERATED ALWAYS AS(x+1))",
        "CREATE TABLE t(x TEXT);CREATE INDEX i ON t(x COLLATE NOCASE)",
        "CREATE TABLE t(x BLOB);CREATE INDEX i ON t(length(x))",
        "CREATE VIRTUAL TABLE t USING fts5(x)",
    ] {
        assert!(
            introspection::component(&fixture(sql), "foks").is_err(),
            "{sql}"
        );
    }
    let name = "é".repeat(129);
    assert!(introspection::component(
        &fixture(&format!("CREATE TABLE \"{name}\"(x BLOB)")),
        "foks"
    )
    .is_err());
    let wide = (0..33)
        .map(|n| format!("x{n} BLOB"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(
        introspection::component(&fixture(&format!("CREATE TABLE t({wide})")), "foks").is_err()
    );
    let long = format!("CREATE TABLE t(x TEXT CHECK(x!='{}'))", "x".repeat(65_536));
    assert!(introspection::component(&fixture(&long), "foks").is_err());
    let object = [
        "table".into(),
        "t".into(),
        "t".into(),
        "CREATE TABLE t(x BLOB)".into(),
    ];
    let objects = [object];
    assert!(union(&[&objects, &objects]).is_err());
}

#[test]
fn artifact_bounds_and_create_new_publication_preserve_existing_files() {
    let mut writer = BoundedJson(vec![0; ARTIFACT_LIMIT - 1]);
    assert_eq!(
        std::io::Write::write(&mut writer, b"xx")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::FileTooLarge
    );
    assert_eq!(writer.0.len(), ARTIFACT_LIMIT - 1);
    std::io::Write::write_all(&mut writer, b"x").unwrap();
    assert!(std::io::Write::write(&mut writer, b"x").is_err());
    let temporary = tempfile::tempdir().unwrap();
    let out = temporary.path().join("new");
    let files = generate().unwrap();
    publish_new(&out, files.clone()).unwrap();
    assert!(publish_new(&out, files.clone()).is_err());
    for (name, value) in files {
        assert_eq!(std::fs::read(out.join(name)).unwrap(), value);
    }
}

#[test]
#[ignore = "explicit reviewed descriptor generation; no admission capability"]
fn export_trusted_restricted_descriptors() {
    let path = std::env::var_os("FOKS_RESTRICTED_DESCRIPTOR_OUTPUT")
        .expect("explicit fresh output directory required");
    publish_new(Path::new(&path), generate().unwrap()).unwrap();
}

#[test]
fn sqlite_owned_tables_are_explicit_and_never_imply_snapshot_admission() {
    let c = fixture("CREATE TABLE audit(id INTEGER PRIMARY KEY AUTOINCREMENT, value TEXT UNIQUE)");
    let component = introspection::component(&c, "foks").unwrap();
    let inventory = sqlite_owned(&c, &component).unwrap();
    let objects = inventory["objects"].as_array().unwrap();
    let sequence = objects
        .iter()
        .find(|o| o["name"] == "sqlite_sequence")
        .unwrap();
    assert_eq!(sequence["schema"], "main");
    assert_eq!(
        sequence["classification"]["kind"],
        "sqlite_owned_data_unadmitted"
    );
    let automatic = objects.iter().find(|o| o["kind"] == "index").unwrap();
    let id = automatic["classification"]["record_id"].as_str().unwrap();
    assert!(component.records.iter().any(|r| r.id == id));
    assert_eq!(
        inventory["statistics_tables_present"],
        serde_json::json!([])
    );
    c.execute_batch("ANALYZE").unwrap();
    assert!(sqlite_owned(&c, &component)
        .unwrap_err()
        .contains("unclassified"));
    assert!(introspection::component(&c, "foks").is_err());
    let temp = fixture("CREATE TABLE main.t(x BLOB);CREATE TEMP TABLE shadow(x BLOB)");
    assert!(introspection::component(&temp, "foks")
        .unwrap_err()
        .contains("TEMP"));
    let attached = fixture("ATTACH ':memory:' AS extra;CREATE TABLE main.t(x BLOB)");
    assert!(introspection::component(&attached, "foks")
        .unwrap_err()
        .contains("attached"));
}
