use super::*;
use crate::restricted_storage::import::list::connection::{FixedExpected, FIXED_CHECKS};
fn memory() -> Arc<Request> {
    crate::net::memory::Pools::new(Default::default())
        .admitted_request(None)
        .unwrap()
}
#[test]
fn compiled_400_byte_record_and_program_artifact_bind_the_actual_inventory() {
    let profile = Profile::new(&memory()).unwrap();
    assert_eq!(&profile.record[..4], b"FKNP");
    assert_eq!(
        u16::from_le_bytes(profile.record[4..6].try_into().unwrap()),
        1
    );
    assert_eq!(
        u16::from_le_bytes(profile.record[6..8].try_into().unwrap()),
        400
    );
    let components: Vec<_> = profile.record[32..352].chunks_exact(32).collect();
    assert_eq!(components.len(), 10);
    assert_eq!(components[2], profile.contract.hash());
    assert_eq!(components[3], scalar_hash().unwrap());
    assert_eq!(
        components[4],
        hash(&foks_metadata_core::page::catalog_artifact().unwrap())
    );
    assert_eq!(components[7], hash(CONNECTION_POLICY));
    let mut digest = Sha256::new();
    digest.update(b"foks-native-personal-list-profile-v1\0");
    digest.update(profile.record);
    assert_eq!(profile.digest, <[u8; 32]>::from(digest.finalize()));
    assert_eq!(
        profile.record[352..],
        [16_384u64, 1_048_576, 65_536, 4096, 4096, 2048]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(components[0], component(objects::FOKS).unwrap().hash());
    assert_eq!(components[1], component(objects::AUTHORITY).unwrap().hash());
    assert_eq!(
        components[5],
        hash(include_bytes!(
            "../../../../foks-schema-contract/src/storage/generated.rs"
        ))
    );
    assert_eq!(components[6], super::super::super::policy::fingerprint());
    // Reconstruct the source-bearing components independently: exactly these
    // length-delimited bytes, rather than a label or a caller-supplied hash.
    for (index, domain, parts) in [
        (
            8,
            b"foks-native-list-entry-contract-v1\0".as_slice(),
            vec![
                ENTRY_POLICY,
                include_bytes!("../operation.rs").as_slice(),
                include_bytes!("../output.rs").as_slice(),
                include_bytes!("../../../../foks-metadata-core/src/personal/native_list.rs")
                    .as_slice(),
            ],
        ),
        (
            9,
            b"foks-native-list-certificate-import-v1\0".as_slice(),
            vec![
                CERTIFICATE_POLICY,
                include_bytes!("../certificate.rs").as_slice(),
                include_bytes!("../../restricted_storage/import/certificates.rs").as_slice(),
            ],
        ),
    ] {
        let mut expected = Sha256::new();
        expected.update(domain);
        for bytes in parts {
            expected.update((bytes.len() as u64).to_le_bytes());
            expected.update(bytes);
        }
        assert_eq!(components[index], <[u8; 32]>::from(expected.finalize()));
    }
    let mut catalog = Sha256::new();
    catalog.update(b"foks-native-personal-list-catalog-v1\0");
    catalog.update(components[3]);
    catalog.update(components[4]);
    assert_eq!(profile.catalog_digest, <[u8; 32]>::from(catalog.finalize()));
    let policy: serde_json::Value = serde_json::from_slice(CONNECTION_POLICY).unwrap();
    assert_eq!(
        policy["sqlite_engine"]["qualified_version"],
        rusqlite::version()
    );
    assert_eq!(
        policy["programs"].as_array().unwrap().len(),
        FIXED_CHECKS.len()
    );
    for (actual, compiled) in policy["programs"]
        .as_array()
        .unwrap()
        .iter()
        .zip(FIXED_CHECKS)
    {
        assert_eq!(actual["id"], format!("connection/{}", compiled.name));
        assert_eq!(actual["sql"], compiled.sql);
        assert_eq!(
            actual["targets"],
            if compiled.native_only {
                serde_json::json!(["native"])
            } else {
                serde_json::json!(["native", "celld"])
            }
        );
        match compiled.expected {
            FixedExpected::Integer(value) => assert_eq!(
                actual["expected"],
                serde_json::json!({"kind":"integer","value":value})
            ),
            FixedExpected::Text(value) => assert_eq!(
                actual["expected"],
                serde_json::json!({"kind":"text","value":value})
            ),
        }
    }
    // Pin the observed Rust 1.95 native layout, not an assumed enum tag word.
    // The allocation proof remains proportional to the actual target layout.
    let path_segment_bytes = std::mem::size_of::<foks_snowpack::PathSegment>();
    let path_segment_alignment = std::mem::align_of::<foks_snowpack::PathSegment>();
    assert_eq!(path_segment_bytes, 24);
    assert_eq!(path_segment_alignment, 8);
    let display_bytes = 23 + 5 + 65 * (1 + 3 * 31) + 2 + 39;
    assert_eq!(display_bytes, 6179);
    let diagnostic_bytes =
        (128 + 65) * path_segment_bytes + 2 * 65 * 31 + (3 * display_bytes + 8) + (9 * 31 + 8);
    assert_eq!(diagnostic_bytes, 27_494);
    assert_eq!(
        super::super::operation::diagnostic_bytes(),
        diagnostic_bytes
    );
    let entry: serde_json::Value = serde_json::from_slice(ENTRY_POLICY).unwrap();
    assert_eq!(entry["snowpack"]["depth_path_segments"], 65);
    assert_eq!(entry["snowpack"]["variant_tag_bytes"], 31);
    assert_eq!(entry["snowpack"]["maximum_display_bytes"], display_bytes);
    assert_eq!(
        entry["snowpack"]["formula"],
        "193*sizeof(PathSegment)+4030+18545+287"
    );
    assert_eq!(
        entry["snowpack"]["native_64_bit_path_segment_bytes"],
        path_segment_bytes
    );
    assert_eq!(
        entry["snowpack"]["native_64_bit_path_segment_alignment"],
        path_segment_alignment
    );
    assert_eq!(entry["snowpack"]["native_64_bit_bytes"], diagnostic_bytes);
    let ingress_diagnostic_bytes = diagnostic_bytes + 3 * (20 - 5);
    assert_eq!(ingress_diagnostic_bytes, 27_539);
    assert_eq!(
        entry["snowpack"]["ingress_native_64_bit_bytes"],
        ingress_diagnostic_bytes
    );
    assert_eq!(entry["snowpack"]["ingress_offset_digits"], 20);
    assert_eq!(
        super::super::operation::ingress_diagnostic_bytes(),
        ingress_diagnostic_bytes
    );
    assert_eq!(foks_metadata_core::MAX_SQL_CALLS, MAX_SQL);
}
fn observed(connection: &rusqlite::Connection) -> Vec<[String; 4]> {
    connection.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE substr(name,1,7)!='sqlite_' COLLATE NOCASE ORDER BY type COLLATE BINARY,name COLLATE BINARY,tbl_name COLLATE BINARY").unwrap()
        .query_map([],|row|Ok([row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?])).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap()
}
#[test]
fn trusted_static_objects_reproduce_current_source_schema_without_celld_objects() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("schema.sqlite");
    let database = foks_server_db::Database::open(&path, Default::default()).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let actual = observed(&connection);
    assert_eq!(actual.len(), objects::FOKS.len());
    for (actual, compiled) in actual.iter().zip(objects::FOKS) {
        assert_eq!(
            actual.each_ref().map(String::as_str),
            [compiled.kind, compiled.name, compiled.table, compiled.sql]
        );
    }
    drop(connection);
    drop(database);
    let authority = rusqlite::Connection::open_in_memory().unwrap();
    authority
        .execute_batch(foks_metadata_core::authority::SCHEMA)
        .unwrap();
    let actual = observed(&authority);
    assert_eq!(actual.len(), objects::AUTHORITY.len());
    for (actual, compiled) in actual.iter().zip(objects::AUTHORITY) {
        assert_eq!(
            actual.each_ref().map(String::as_str),
            [compiled.kind, compiled.name, compiled.table, compiled.sql]
        );
    }
    let objects = objects::FOKS
        .iter()
        .chain(objects::AUTHORITY)
        .collect::<Vec<_>>();
    assert!(objects.len() <= 1024);
    assert!(objects.iter().map(|o| o.sql.len()).sum::<usize>() <= 2 * 1024 * 1024);
    for object in objects {
        assert!(
            object.kind.len() <= 8
                && object.name.len() <= 256
                && object.table.len() <= 256
                && object.sql.len() <= 65_536
        );
    }
    let profile = Profile::new(&memory()).unwrap();
    assert_eq!(
        profile.record[8..32],
        [1u32, 1, 59, 1, 1, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
}
#[test]
fn failed_original_owner_cannot_construct_profile() {
    let owner = memory();
    foks_metadata_core::owned_list::ListRetainedOwner::fail(owner.as_ref());
    assert!(matches!(Profile::new(&owner), Err(Error::Bounds)));
}
