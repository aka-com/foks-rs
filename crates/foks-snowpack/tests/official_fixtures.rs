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

#[derive(Deserialize)]
struct KvManifest {
    format: String,
    foks_version: String,
    official_generated: bool,
    files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct UserManifest {
    format: String,
    foks_version: String,
    official_unboxed: bool,
    files: Vec<ManifestFile>,
    raw_files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct SignupManifest {
    format: String,
    foks_version: String,
    official_built: bool,
    files: Vec<ManifestFile>,
    raw_files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct MutationManifest {
    format: String,
    foks_version: String,
    generator: String,
    official_built: bool,
    server_semantics_verified: bool,
    files: Vec<ManifestFile>,
    raw_files: Vec<ManifestFile>,
}

#[test]
fn official_go_v019_user_mutation_manifest_covers_every_artifact() {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foks-v0.1.9/user-mutations");
    let manifest: MutationManifest =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.format, "foks-v0.1.9-user-mutation-fixtures-v2");
    assert_eq!(manifest.foks_version, "v0.1.9");
    assert_eq!(manifest.generator, "sha256-counter-v1");
    assert!(manifest.official_built);
    assert!(manifest.server_semantics_verified);
    let mut names = BTreeSet::new();
    for fixture in manifest.files.into_iter().chain(manifest.raw_files) {
        assert!(names.insert(fixture.file.clone()));
        let bytes = fs::read(directory.join(&fixture.file)).unwrap();
        assert_eq!(bytes.len(), fixture.bytes, "{} size", fixture.file);
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            fixture.sha256,
            "{} digest",
            fixture.file
        );
        if fixture.file.ends_with(".snowp") {
            assert_eq!(encode(&decode(&bytes).unwrap()).unwrap(), bytes);
        }
    }
    let disk_names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "manifest.json")
        .collect::<BTreeSet<_>>();
    assert_eq!(disk_names, names);
}

#[test]
fn official_go_v019_signup_manifest_covers_every_artifact() {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foks-v0.1.9/signup");
    let manifest: SignupManifest =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.format, "foks-v0.1.9-signup-fixtures-v1");
    assert_eq!(manifest.foks_version, "v0.1.9");
    assert!(manifest.official_built);
    let mut names = BTreeSet::new();
    for fixture in manifest.files.into_iter().chain(manifest.raw_files) {
        assert!(names.insert(fixture.file.clone()));
        let bytes = fs::read(directory.join(&fixture.file)).unwrap();
        assert_eq!(bytes.len(), fixture.bytes, "{} size", fixture.file);
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            fixture.sha256,
            "{} digest",
            fixture.file
        );
        if fixture.file.ends_with(".snowp") {
            assert_eq!(encode(&decode(&bytes).unwrap()).unwrap(), bytes);
        }
    }
    let disk_names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "manifest.json")
        .collect::<BTreeSet<_>>();
    assert_eq!(disk_names, names);
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

#[test]
fn official_go_v019_kv_fixture_manifest_covers_every_kv_artifact() {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foks-v0.1.9/user");
    let manifest: KvManifest =
        serde_json::from_slice(&fs::read(directory.join("kv-manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.format, "foks-v0.1.9-kv-fixtures-v1");
    assert_eq!(manifest.foks_version, "v0.1.9");
    assert!(manifest.official_generated);
    let mut names = BTreeSet::new();
    for fixture in manifest.files {
        assert!(fixture.file.starts_with("kv-"));
        assert!(names.insert(fixture.file.clone()));
        let bytes = fs::read(directory.join(&fixture.file)).unwrap();
        assert_eq!(bytes.len(), fixture.bytes, "{} size", fixture.file);
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            fixture.sha256,
            "{} digest",
            fixture.file
        );
        if fixture.file.ends_with(".snowp") {
            let value = decode(&bytes).unwrap();
            assert_eq!(encode(&value).unwrap(), bytes);
        }
    }
    let disk_names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("kv-") && name != "kv-manifest.json")
        .collect::<BTreeSet<_>>();
    assert_eq!(disk_names, names);
}

#[test]
fn official_go_v019_user_manifest_hashes_remain_exact() {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foks-v0.1.9/user");
    let manifest: UserManifest =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.format, "foks-v0.1.9-user-fixtures-v1");
    assert_eq!(manifest.foks_version, "v0.1.9");
    assert!(manifest.official_unboxed);
    let mut names = BTreeSet::new();
    for fixture in manifest.files.into_iter().chain(manifest.raw_files) {
        assert!(names.insert(fixture.file.clone()));
        let bytes = fs::read(directory.join(&fixture.file)).unwrap();
        assert_eq!(bytes.len(), fixture.bytes, "{} size", fixture.file);
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            fixture.sha256,
            "{} digest",
            fixture.file
        );
        if fixture.file.ends_with(".snowp") {
            let value = decode(&bytes).unwrap();
            assert_eq!(encode(&value).unwrap(), bytes);
        }
    }
    let disk_names = fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.unwrap();
            entry
                .file_type()
                .unwrap()
                .is_file()
                .then(|| entry.file_name().into_string().unwrap())
        })
        .filter(|name| name != "manifest.json" && name != "kv-manifest.json")
        .collect::<BTreeSet<_>>();
    assert_eq!(disk_names, names, "user fixture manifest is incomplete");
}
