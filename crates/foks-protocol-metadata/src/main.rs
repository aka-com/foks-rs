use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use foks_protocol_metadata::{
    merge, parse_artifact, parse_policy, render_contract, render_digest_manifest,
    render_protocol_ids, render_routes, render_status_codes,
};

struct Paths {
    upstream: PathBuf,
    policy: PathBuf,
    protocol_ids: PathBuf,
    status_codes: PathBuf,
    routes: PathBuf,
    contract: PathBuf,
    digest: PathBuf,
}

fn main() {
    if let Err(error) = run(env::args().skip(1)) {
        eprintln!("foks-protocol-metadata: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: impl Iterator<Item = String>) -> Result<(), String> {
    let mut arguments = arguments;
    let command = arguments.next().ok_or_else(|| {
        "usage: foks-protocol-metadata <check|write> [--name path ...]".to_owned()
    })?;
    if !matches!(command.as_str(), "check" | "write") {
        return Err(format!("unknown command {command:?}"));
    }
    let mut flags = BTreeMap::new();
    while let Some(flag) = arguments.next() {
        if !flag.starts_with("--") {
            return Err(format!("expected flag, found {flag:?}"));
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("{flag} requires a path"))?;
        if flags.insert(flag.clone(), PathBuf::from(value)).is_some() {
            return Err(format!("duplicate flag {flag}"));
        }
    }
    let paths = Paths {
        upstream: take(&mut flags, "--upstream")?,
        policy: take(&mut flags, "--policy")?,
        protocol_ids: take(&mut flags, "--protocol-ids")?,
        status_codes: take(&mut flags, "--status-codes")?,
        routes: take(&mut flags, "--routes")?,
        contract: take(&mut flags, "--contract")?,
        digest: take(&mut flags, "--digest")?,
    };
    if let Some(flag) = flags.keys().next() {
        return Err(format!("unknown flag {flag}"));
    }
    generate(command == "write", &paths)
}

fn take(flags: &mut BTreeMap<String, PathBuf>, name: &str) -> Result<PathBuf, String> {
    flags
        .remove(name)
        .ok_or_else(|| format!("{name} is required"))
}

fn generate(write: bool, paths: &Paths) -> Result<(), String> {
    let artifact_bytes = fs::read(&paths.upstream)
        .map_err(|error| format!("read {}: {error}", paths.upstream.display()))?;
    let artifact_text = std::str::from_utf8(&artifact_bytes)
        .map_err(|error| format!("{} is not UTF-8: {error}", paths.upstream.display()))?;
    let policy_text = fs::read_to_string(&paths.policy)
        .map_err(|error| format!("read {}: {error}", paths.policy.display()))?;
    let artifact = parse_artifact(artifact_text).map_err(|error| error.to_string())?;
    let policy = parse_policy(&policy_text).map_err(|error| error.to_string())?;
    let merged = merge(&artifact, &policy).map_err(|error| error.to_string())?;
    let outputs = [
        (&paths.protocol_ids, render_protocol_ids(&merged)),
        (&paths.status_codes, render_status_codes(&merged)),
        (&paths.routes, render_routes(&merged)),
        (&paths.contract, render_contract(&merged)),
        (
            &paths.digest,
            render_digest_manifest(&artifact_bytes, &artifact),
        ),
    ];
    for (path, expected) in outputs {
        if write {
            write_output(path, &expected)?;
        } else {
            let actual = fs::read_to_string(path)
                .map_err(|error| format!("read generated {}: {error}", path.display()))?;
            if actual != expected {
                return Err(format!(
                    "{} is stale; run tools/foks-server/generate-protocol.sh --write",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn write_output(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    fs::write(path, contents).map_err(|error| format!("write {}: {error}", path.display()))
}
