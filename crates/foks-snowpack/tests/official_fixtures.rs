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
    files: Vec<ManifestFile>,
    rpc_files: Vec<ManifestFile>,
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
    files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct UserManifest {
    format: String,
    foks_version: String,
    files: Vec<ManifestFile>,
    raw_files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct YubiManifest {
    format: String,
    foks_version: String,
    files: Vec<ManifestFile>,
    raw_files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct SignupManifest {
    format: String,
    foks_version: String,
    go_module_generated: bool,
    files: Vec<ManifestFile>,
    raw_files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct MutationManifest {
    format: String,
    foks_version: String,
    generator: String,
    go_module_generated: bool,
    server_observed: bool,
    files: Vec<ManifestFile>,
    raw_files: Vec<ManifestFile>,
}

#[test]
fn go_v019_user_mutation_manifest_covers_every_generated_artifact() {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foks-v0.1.9/user-mutations");
    let manifest: MutationManifest =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.format, "foks-v0.1.9-user-mutation-fixtures-v3");
    assert_eq!(manifest.foks_version, "v0.1.9");
    assert_eq!(manifest.generator, "sha256-counter-v1");
    assert!(manifest.go_module_generated);
    assert!(!manifest.server_observed);
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
fn go_v019_signup_manifest_covers_every_generated_artifact() {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foks-v0.1.9/signup");
    let manifest: SignupManifest =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.format, "foks-v0.1.9-signup-fixtures-v2");
    assert_eq!(manifest.foks_version, "v0.1.9");
    assert!(manifest.go_module_generated);
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
    assert_eq!(manifest.format, "foks-v0.1.9-probe-fixtures-v2");
    assert_eq!(manifest.foks_version, "v0.1.9");
    assert_eq!(manifest.files.len(), 8);

    let mut names = BTreeSet::new();
    for fixture in &manifest.files {
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

    for fixture in &manifest.rpc_files {
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
    }

    let disk_names = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "manifest.json")
        .collect::<BTreeSet<_>>();
    assert_eq!(disk_names, names, "fixture directory and manifest differ");
}

#[test]
fn official_go_v019_kv_fixture_manifest_covers_every_kv_artifact() {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foks-v0.1.9/user");
    let manifest: KvManifest =
        serde_json::from_slice(&fs::read(directory.join("kv-manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.format, "foks-v0.1.9-kv-fixtures-v2");
    assert_eq!(manifest.foks_version, "v0.1.9");
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
    assert_eq!(manifest.format, "foks-v0.1.9-user-fixtures-v2");
    assert_eq!(manifest.foks_version, "v0.1.9");
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

#[test]
fn official_go_v019_yubi_manifest_hashes_remain_exact() {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/foks-v0.1.9/user/yubi");
    let manifest: YubiManifest =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.format, "foks-v0.1.9-yubi-subkey-fixtures-v1");
    assert_eq!(manifest.foks_version, "v0.1.9");
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
    assert_eq!(disk_names, names, "Yubi fixture manifest is incomplete");
}
