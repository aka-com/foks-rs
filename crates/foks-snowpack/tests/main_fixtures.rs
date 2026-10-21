//! Checks the go-foks main fixture tree (`fixtures/foks-main/<area>/`) against
//! the `SHA256SUMS` file the oracle writes beside each area, so a regenerated
//! or edited fixture fails here as well as in the Go oracle's compare mode.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use sha2::{Digest as _, Sha256};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/foks-main");
/// Files in an area directory that the sums do not cover.
const UNLISTED: [&str; 2] = ["README.md", "SHA256SUMS"];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn check_area(area: &Path) -> usize {
    let sums = fs::read_to_string(area.join("SHA256SUMS"))
        .unwrap_or_else(|error| panic!("{}: SHA256SUMS: {error}", area.display()));
    assert!(
        area.join("README.md").is_file(),
        "{} has no README.md",
        area.display()
    );
    let mut listed = BTreeSet::new();
    let mut previous = String::new();
    for line in sums.lines() {
        let (digest, name) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("{}: malformed line {line:?}", area.display()));
        assert!(
            !name.is_empty() && !name.contains('/') && !UNLISTED.contains(&name),
            "{}: unexpected entry {name:?}",
            area.display()
        );
        assert!(
            name > previous.as_str(),
            "{}: entries are not sorted and unique",
            area.display()
        );
        previous = name.to_owned();
        let bytes = fs::read(area.join(name))
            .unwrap_or_else(|error| panic!("{}/{name}: {error}", area.display()));
        assert_eq!(
            hex(&Sha256::digest(&bytes)),
            digest,
            "{}/{name} does not match SHA256SUMS",
            area.display()
        );
        listed.insert(name.to_owned());
    }
    let present = fs::read_dir(area)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| !UNLISTED.contains(&name.as_str()))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        present,
        listed,
        "{}: files and SHA256SUMS differ",
        area.display()
    );
    listed.len()
}

#[test]
fn every_go_foks_main_fixture_area_matches_its_sums() {
    let mut areas = fs::read_dir(ROOT)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    areas.sort();
    let names = areas
        .iter()
        .map(|area| area.file_name().unwrap().to_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["kv", "realtime", "team"]);
    for area in &areas {
        assert!(check_area(area) > 0, "{} lists no files", area.display());
    }
}
