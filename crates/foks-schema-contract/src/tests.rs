use super::*;
use crate::lex::Tokens;
#[path = "test_oracle.rs"]
mod old;

fn tokens(sql: &str) -> Result<Vec<Vec<u8>>, ContractError> {
    Tokens::new(sql)
        .map(|token| token.map(|t| t.owned()))
        .collect()
}
fn table(name: &'static str, sql: &'static str) -> SchemaObject<'static> {
    SchemaObject {
        kind: "table",
        name,
        table: name,
        sql,
    }
}
fn expected() -> Contract<'static> {
    Contract::new(vec![
        ContractObject::borrowed(table("alpha", "CREATE TABLE alpha(v INTEGER)")),
        ContractObject::borrowed(table("omega", "CREATE TABLE omega(v TEXT)")),
    ])
    .unwrap()
}
fn check(
    contract: &Contract<'_>,
    mut objects: Vec<SchemaObject<'_>>,
    policy: MatchPolicy,
) -> Result<(), ContractError> {
    objects.sort_by(|a, b| a.key().cmp(&b.key()));
    let mut verifier = contract.verifier(policy);
    for object in objects {
        verifier.observe(object)?;
    }
    verifier.finish()
}
fn objects() -> Vec<SchemaObject<'static>> {
    vec![
        table("alpha", "CREATE TABLE alpha(v INTEGER)"),
        table("omega", "CREATE TABLE omega(v TEXT)"),
    ]
}

#[test]
fn lexer_matches_frozen_schema59_at_quote_comment_and_token_boundaries() {
    let cases = [
        "",
        " ; --comment",
        ";;",
        "SELECT a bc",
        "SELECT ab c",
        "SELECT 'A B'",
        "SELECT 'ab'",
        "SELECT 'a''b'",
        "SELECT \"0\",\"null\",\"true\"",
        "SELECT `source_hash`,[SOURCE_HASH],\"blocked_reason\"",
        "SELECT 'source_hash'",
        "SELECT \"source_\"\"hash\"",
        "SELECT /*x*/ '/*x*/'; --last",
        "SELECT utfé, '你好'",
        "SELECT 'unterminated",
        "/*unterminated",
        "SELECT 1; /*unterminated",
        "SELECT [unterminated",
        "SELECT `a``b`",
        "SELECT [x]]",
        "SELECT \"chunk_objects\";\n/*done*/",
    ];
    for sql in cases {
        let actual = tokens(sql);
        let prior = old::tokens(sql);
        assert_eq!(actual.is_ok(), prior.is_ok(), "{sql:?}");
        if let (Ok(a), Ok(b)) = (actual, prior) {
            assert_eq!(a, b, "{sql:?}");
        }
    }
    for left in cases {
        for right in cases {
            match (old::tokens(left), old::tokens(right)) {
                (Ok(a), Ok(b)) => assert_eq!(
                    sql_equal(left, right).unwrap(),
                    a == b,
                    "{left:?} vs {right:?}"
                ),
                _ => assert!(sql_equal(left, right).is_err(), "{left:?} vs {right:?}"),
            }
        }
    }
}

#[test]
fn long_tokens_borrow_source_and_hash_without_per_token_buffers() {
    for sql in [
        format!("SELECT '{}'; /*end*/", "Aé ".repeat(16_384)),
        format!("SELECT {};", "AbC_".repeat(16_384)),
    ] {
        let canonical = old::tokens(&sql).unwrap();
        assert_eq!(tokens(&sql).unwrap(), canonical);
        let base = sql.as_ptr() as usize;
        let end = base + sql.len();
        for token in Tokens::new(&sql) {
            let token = token.unwrap();
            let start = token.bytes.as_ptr() as usize;
            assert!(start >= base && start + token.bytes.len() <= end);
        }
        let object = SchemaObject {
            kind: "table",
            name: "large",
            table: "large",
            sql: &sql,
        };
        let new = Contract::new(vec![ContractObject::borrowed(object)]).unwrap();
        assert_eq!(
            *new.hash(),
            old::hash([(object.kind, object.name, object.table, object.sql)]).unwrap()
        );
    }
    // The iterator holds two borrowed token views and offsets, not owned text.
    assert!(std::mem::size_of::<Tokens<'_>>() <= 96);
}

#[test]
fn lexer_errors_are_fused_and_permitted_extensions_validate_their_entire_tail() {
    let mut broken = Tokens::new("SELECT 1; /*bad");
    assert!(broken.by_ref().any(|x| x.is_err()));
    assert!(broken.next().is_none());
    let contract = expected();
    let mut all = objects();
    all.push(table("zeta", "CREATE TABLE zeta(v); /*bad"));
    assert_eq!(
        check(&contract, all, MatchPolicy::NativeRequired),
        Err(ContractError::UnterminatedComment)
    );
}

#[test]
fn contract_hash_and_streaming_pages_match_legacy_without_binding_observed_extensions() {
    let contract = expected();
    let original = objects();
    // Captured from the independent schema59 algorithm before extraction.
    assert_eq!(
        *contract.hash(),
        [
            0xef, 0x86, 0x25, 0xac, 0xfc, 0xe1, 0xdb, 0x89, 0x18, 0xd6, 0xa9, 0xb9, 0x73, 0x28,
            0xa5, 0xfe, 0x08, 0xfa, 0x95, 0x0c, 0x38, 0x37, 0xae, 0xd6, 0x5d, 0xee, 0x6b, 0xcc,
            0x91, 0xd0, 0x6b, 0xaa
        ],
    );
    assert_eq!(
        *contract.hash(),
        old::hash(original.iter().map(|o| (o.kind, o.name, o.table, o.sql))).unwrap()
    );
    let mut verifier = contract.verifier(MatchPolicy::NativeRequired);
    verifier.observe(original[0]).unwrap(); // first host page can now be dropped
    verifier.observe(original[1]).unwrap();
    verifier
        .observe(table("zeta", "CREATE TABLE zeta(v)"))
        .unwrap();
    verifier.finish().unwrap();
    assert_eq!(
        *contract.hash(),
        old::hash(original.iter().map(|o| (o.kind, o.name, o.table, o.sql))).unwrap()
    );
    assert_eq!(
        check(&contract, vec![original[0]], MatchPolicy::NativeRequired),
        Err(ContractError::MissingObject)
    );
    let mut changed = original.clone();
    changed[0].sql = "CREATE TABLE alpha(v TEXT)";
    assert_eq!(
        check(&contract, changed, MatchPolicy::NativeRequired),
        Err(ContractError::ObjectChanged)
    );
}

#[test]
fn explicit_policies_keep_native_extensions_separate_from_exact_hosted_union() {
    let contract = expected();
    for extension in [
        table("sqliteX_extension", "CREATE TABLE sqliteX_extension(v)"),
        SchemaObject {
            kind: "trigger",
            name: "outside",
            table: "unrelated",
            sql: "CREATE TRIGGER outside AFTER INSERT ON unrelated BEGIN SELECT 1; END",
        },
    ] {
        let mut all = objects();
        all.push(extension);
        check(&contract, all.clone(), MatchPolicy::NativeRequired).unwrap();
        assert_eq!(
            check(&contract, all, MatchPolicy::ExactUnion),
            Err(ContractError::ExtraObject)
        );
    }
    for (name, target) in [
        ("sqliteX_hole", "alpha"),
        ("SQLITEZ_hole", "alpha"),
        ("mixed", "ALPHA"),
    ] {
        let extra = SchemaObject {
            kind: "trigger",
            name,
            table: target,
            sql: "SELECT 1",
        };
        let mut all = objects();
        all.push(extra);
        assert_eq!(
            check(&contract, all, MatchPolicy::NativeRequired),
            Err(ContractError::ExtraObject)
        );
    }
    let mut all = objects();
    all.push(table(
        "METADATA_READ_extra",
        "CREATE TABLE METADATA_READ_extra(v)",
    ));
    assert_eq!(
        check(&contract, all, MatchPolicy::NativeRequired),
        Err(ContractError::ExtraObject)
    );
    assert!(is_sqlite_internal_name("SqLiTe_autoindex_x"));
    assert!(!is_sqlite_internal_name("sqliteX_hole"));
    assert!(!is_sqlite_internal_name("sqlite"));
    let mut verifier = contract.verifier(MatchPolicy::NativeRequired);
    assert_eq!(
        verifier.observe(table("sqlite_internal", "SELECT 1")),
        Err(ContractError::InternalObject)
    );
    assert_eq!(verifier.finish(), Err(ContractError::Poisoned));
}

#[test]
fn order_duplicates_missing_objects_and_ignored_errors_cannot_finish_successfully() {
    let contract = expected();
    let all = objects();
    let mut verifier = contract.verifier(MatchPolicy::ExactUnion);
    verifier.observe(all[0]).unwrap();
    assert_eq!(
        verifier.observe(all[0]),
        Err(ContractError::DuplicateObject)
    );
    assert_eq!(verifier.observe(all[1]), Err(ContractError::Poisoned));
    assert_eq!(verifier.finish(), Err(ContractError::Poisoned));
    let mut verifier = contract.verifier(MatchPolicy::NativeRequired);
    verifier.observe(table("aardvark", "SELECT 1")).unwrap();
    assert_eq!(
        verifier.observe(table("AAA", "SELECT 1")),
        Err(ContractError::ObjectOrder)
    );
    assert_eq!(verifier.finish(), Err(ContractError::Poisoned));
    let mut verifier = contract.verifier(MatchPolicy::ExactUnion);
    assert_eq!(verifier.observe(all[1]), Err(ContractError::MissingObject));
    assert_eq!(verifier.finish(), Err(ContractError::Poisoned));
    let mut duplicate = all[0];
    duplicate.table = "different";
    assert_eq!(
        Contract::new(vec![
            ContractObject::borrowed(all[0]),
            ContractObject::borrowed(duplicate)
        ])
        .unwrap_err(),
        ContractError::DuplicateObject
    );
    assert_eq!(
        Contract::new(vec![
            ContractObject::borrowed(all[1]),
            ContractObject::borrowed(all[0])
        ])
        .unwrap_err(),
        ContractError::ObjectOrder
    );
}

#[test]
fn owned_and_borrowed_contracts_have_identical_hash_and_policy() {
    let borrowed = expected();
    let owned = Contract::new(
        objects()
            .iter()
            .map(|o| {
                ContractObject::owned(o.kind.into(), o.name.into(), o.table.into(), o.sql.into())
            })
            .collect(),
    )
    .unwrap();
    assert_eq!(owned.hash(), borrowed.hash());
    check(&owned, objects(), MatchPolicy::ExactUnion).unwrap();
    check(&borrowed, objects(), MatchPolicy::ExactUnion).unwrap();
}

#[test]
fn schema_canonicalization_preserves_literal_contents_and_token_boundaries() {
    assert_eq!(
        tokens("CREATE TABLE chunk_objects (source_hash INTEGER CHECK (source_hash > 0));")
            .unwrap(),
        tokens("create table \"chunk_objects\"(\"source_hash\" integer check(\"source_hash\">0))")
            .unwrap()
    );
    assert_ne!(
        tokens("SELECT 'A B'").unwrap(),
        tokens("SELECT 'ab'").unwrap()
    );
    for literal in ["0", "null", "true", "arbitrary"] {
        assert_ne!(
            tokens(&format!("SELECT \"{literal}\"")).unwrap(),
            tokens(&format!("SELECT {literal}")).unwrap()
        );
    }
    assert_ne!(
        tokens("SELECT a bc").unwrap(),
        tokens("SELECT ab c").unwrap()
    );
    assert_ne!(
        tokens("SELECT 'a''b'").unwrap(),
        tokens("SELECT 'ab'").unwrap()
    );
    assert_eq!(
        tokens("SELECT /* x */ '/* x */'; --ignored").unwrap(),
        tokens("select '/* x */'").unwrap()
    );
    assert!(tokens("SELECT 'unterminated").is_err());
    assert!(tokens("/* unterminated").is_err());
}
