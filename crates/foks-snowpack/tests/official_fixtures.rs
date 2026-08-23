use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, PathBuf};

use foks_snowpack::{decode, encode};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

#[derive(Deserialize)]
struct Manifest {
    format: String,
    foks_version: String,
    canonical_verified: bool,
    signatures_verified: bool,
    files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct ManifestFile {
    file: String,
    bytes: usize,
    sha256: String,
}

#[test]
fn official_go_v019_fixtures_round_trip_byte_exactly() {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foks-v0.1.9/foks.app");
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.format, "foks-v0.1.9-probe-fixtures-v1");
    assert_eq!(manifest.foks_version, "v0.1.9");
    assert!(manifest.canonical_verified);
    assert!(manifest.signatures_verified);
    assert_eq!(manifest.files.len(), 8);

    let mut names = BTreeSet::new();
    for fixture in manifest.files {
        let relative = PathBuf::from(&fixture.file);
        assert_eq!(
            relative.components().count(),
            1,
            "fixture path must be a bare filename"
        );
        assert!(matches!(
            relative.components().next(),
            Some(Component::Normal(_))
        ));
        assert!(names.insert(fixture.file.clone()), "duplicate fixture name");

        let path = directory.join(relative);
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), fixture.bytes, "{} size", path.display());
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            fixture.sha256,
            "{} digest",
            path.display()
        );
        let value = decode(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(
            encode(&value).unwrap(),
            bytes,
            "{} did not round-trip byte-exactly",
            path.display()
        );
    }

    let disk_names = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".snowp"))
        .collect::<BTreeSet<_>>();
    assert_eq!(disk_names, names, "fixture directory and manifest differ");
}
