//! Pinned native identity and a closed physical-lifetime qualification fixture.
//! Neither command grants tenant readiness or qualifies a hosted deployment.
use foks_hosted_transport::memory::{self, PoolLimits, Pools, Request};
use foks_metadata_core::owned_list::ListRetainedOwner;
use sha2::{Digest, Sha256};
use std::{error::Error, io::Write, sync::Arc};

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

const INPUT: &str = include_str!("../runtime.lock.json");

#[cfg(feature = "lifecycle")]
mod lifecycle;

enum Command {
    Identity,
    #[cfg(feature = "lifecycle")]
    Lifecycle(celld::LifecycleScenario),
}

fn command() -> Result<Command, Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next().as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("identity") if arguments.next().is_none() => Ok(Command::Identity),
        #[cfg(feature = "lifecycle")]
        Some("lifecycle") => {
            let name = arguments.next().ok_or("missing lifecycle scenario")?;
            let scenario = name
                .to_str()
                .and_then(celld::LifecycleScenario::from_name)
                .ok_or("unknown lifecycle scenario")?;
            if arguments.next().is_some() {
                return Err("unexpected lifecycle argument".into());
            }
            Ok(Command::Lifecycle(scenario))
        }
        _ => Err("usage: foks-celld-guarded identity (or, with its feature, lifecycle <name>)".into()),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // Reject unknown commands before V8 or host runtime initialization.
    let command = command()?;
    celld::runtime::init_v8();
    match command {
        Command::Identity => identity(),
        #[cfg(feature = "lifecycle")]
        Command::Lifecycle(scenario) => lifecycle::run(scenario, INPUT),
    }
}

fn identity() -> Result<(), Box<dyn Error>> {
    let pins: serde_json::Value = serde_json::from_str(INPUT)?;
    let claim = match pins["profile"].as_str() {
        Some("a1-library-identity") => "a1-library-identity-only",
        Some("a2-physical-lifetimes") => "a2-library-identity-only",
        _ => return Err("unsupported identity input profile".into()),
    };
    let connection = rusqlite::Connection::open_in_memory()?;
    let (sqlite_version, sqlite_source_id): (String, String) =
        connection.query_row("SELECT sqlite_version(), sqlite_source_id()", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    let mut options = Vec::<String>::new();
    {
        let mut statement = connection.prepare("PRAGMA compile_options")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let option: String = row.get(0)?;
            if options.len() == 512 || option.len() > 4096 {
                return Err("engine diagnostic exceeds its fixed envelope".into());
            }
            options.push(option);
        }
    }
    options.sort();
    drop(connection);

    let pools = Pools::new(PoolLimits::new(1024 * 1024, 32 * 1024 * 1024, 16384, 2)?);
    let request: Arc<Request> = pools
        .admitted_request(None)
        .map_err(|_| "actual request admission failed")?;
    let owner: Arc<dyn ListRetainedOwner> = request.clone();
    if !owner.reserve_retained(64) || owner.failed() {
        return Err("actual owned-list reservation failed".into());
    }
    let same_owner = request.sync(|| {
        memory::current()
            .map(|current| Arc::ptr_eq(&current, &request))
            .unwrap_or(false)
    });
    if !same_owner || memory::current().is_some() {
        return Err("actual request scope identity failed".into());
    }
    drop(owner);
    drop(request);
    drop(pools);

    let report = serde_json::json!({
        "format": 1,
        "claim": claim,
        "input_sha256": format!("{:x}", Sha256::digest(INPUT.as_bytes())),
        // These are embedded recipe pins; the external receipt checks the
        // compiler actually used and binds the executable hash, not this label.
        "recipe_toolchain": pins["toolchain"],
        "recipe_target": pins["target"],
        "recipe_profile": pins["build"]["profile"],
        "celld_package": pins["upstream"]["version"],
        "v8_package": pins["v8"]["version"],
        "v8_engine": v8::V8::get_version(),
        "sqlite_version": sqlite_version,
        "sqlite_source_id": sqlite_source_id,
        "sqlite_compile_options": options,
        "allocator": "tikv-jemallocator 0.6.1; background_threads,stats",
        "actual_shared_request_exercised": true,
    });
    let mut encoded = serde_json::to_vec(&report)?;
    if encoded.len() > 64 * 1024 {
        return Err("identity report exceeds 64 KiB".into());
    }
    encoded.push(b'\n');
    std::io::stdout().lock().write_all(&encoded)?;
    Ok(())
}
