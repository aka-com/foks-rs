use std::env;
use std::path::Path;

use foks_client::{ProbeTarget, PublicClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args_os().skip(1);
    let target = arguments
        .next()
        .ok_or("usage: probe <hostname[:port]> <hard-state.sqlite3>")?
        .into_string()
        .map_err(|_| "target is not UTF-8")?;
    let database = arguments.next().ok_or("missing hard-state database path")?;
    if arguments.next().is_some() {
        return Err("too many arguments".into());
    }

    let target = ProbeTarget::parse(&target)?;
    let outcome = PublicClient::webpki().probe_and_pin(&target, Path::new(&database))?;
    let host_id = outcome
        .verified
        .snapshot
        .host_id()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    println!(
        "{:?} host={} chain={} merkle={} canonical_name={}",
        outcome.acceptance,
        host_id,
        outcome.verified.snapshot.chain_seqno(),
        outcome.verified.snapshot.merkle_root().epoch(),
        outcome.verified.snapshot.canonical_name(),
    );
    Ok(())
}
