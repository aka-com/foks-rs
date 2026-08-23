use std::env;
use std::fs;

use foks_verify::verify_public_host;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args_os().skip(1);
    let probe_path = arguments
        .next()
        .ok_or("usage: verify_probe <probe-response.snowp> <lookup-name>")?;
    let lookup_name = arguments
        .next()
        .ok_or("missing lookup name")?
        .into_string()
        .map_err(|_| "lookup name is not UTF-8")?;
    if arguments.next().is_some() {
        return Err("too many arguments".into());
    }

    let probe = fs::read(probe_path)?;
    let verified = verify_public_host(&lookup_name, &probe)?;
    let host_id = verified
        .snapshot
        .host_id()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    println!(
        "verified host={host_id} chain={} merkle={} canonical_name={}",
        verified.snapshot.chain_seqno(),
        verified.snapshot.merkle_root().epoch(),
        verified.snapshot.canonical_name(),
    );
    Ok(())
}
