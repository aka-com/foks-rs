//! The canonical Go FOKS v0.1.9 Snowpack fixtures, used as corpus seeds.
use std::path::{Path, PathBuf};

pub fn directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/foks-snowpack/tests/fixtures/foks-v0.1.9")
}

/// Every `.snowp` fixture as (seed name, bytes), ordered by seed name. The
/// seed name is the relative path without the extension, with `/` replaced
/// by `-`.
pub fn snowp() -> Vec<(String, Vec<u8>)> {
    fn walk(directory: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, found);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "snowp")
            {
                found.push(path);
            }
        }
    }
    let root = directory();
    let mut paths = Vec::new();
    walk(&root, &mut paths);
    let mut fixtures: Vec<_> = paths
        .into_iter()
        .map(|path| {
            let relative = path.strip_prefix(&root).unwrap().with_extension("");
            let name = relative
                .components()
                .map(|part| part.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("-");
            (format!("go-{name}"), std::fs::read(&path).unwrap())
        })
        .collect();
    fixtures.sort();
    fixtures
}
