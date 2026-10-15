//! The canonical Go FOKS v0.1.9 Snowpack fixtures, used as corpus seeds.
use std::path::{Path, PathBuf};

pub fn directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/foks-snowpack/tests/fixtures/foks-v0.1.9")
}

/// Every fixture with one of `extensions` as (seed name, bytes), ordered by
/// seed name. The seed name is the relative path without the extension, with
/// `/` replaced by `-`.
fn with_extensions(extensions: &[&str]) -> Vec<(String, Vec<u8>)> {
    fn walk(directory: &Path, extensions: &[&str], found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, extensions, found);
            } else if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extensions.contains(&extension))
            {
                found.push(path);
            }
        }
    }
    let root = directory();
    let mut paths = Vec::new();
    walk(&root, extensions, &mut paths);
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

/// Every `.snowp` fixture, named as by `with_extensions`.
pub fn snowp() -> Vec<(String, Vec<u8>)> {
    with_extensions(&["snowp"])
}

/// The call argument of every `.frame` and `.rpc` fixture that is a framed
/// RPC call, named as by `with_extensions` with a `-call` suffix so that the
/// names stay distinct from `.snowp` fixtures with the same stem. Response
/// frames are skipped.
pub fn call_arguments() -> Vec<(String, Vec<u8>)> {
    with_extensions(&["frame", "rpc"])
        .into_iter()
        .filter_map(|(name, bytes)| {
            let call = foks_rpc::read_call(&mut std::io::Cursor::new(bytes), 1 << 20).ok()?;
            Some((format!("{name}-call"), call.argument().to_vec()))
        })
        .collect()
}
