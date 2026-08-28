use std::path::PathBuf;

use foks_server::host::{
    begin_host_key_rotation, complete_host_key_rotation, load_or_bootstrap, BootstrapEndpoints,
    BootstrapInput, HostKeyRotationObservation,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: host_rotation_fixtures OUTPUT_DIRECTORY")?;
    std::fs::create_dir_all(&output)?;

    let provider = foks_server::keys::MemoryKeyProvider::default();
    let mut database = foks_server_db::Database::open(
        output.join("fixture.sqlite"),
        foks_server_db::Config::default(),
    )?;
    let input = BootstrapInput {
        canonical_name: "localhost".to_owned(),
        endpoints: BootstrapEndpoints {
            probe: "localhost:4430".to_owned(),
            public_services: "localhost:4431".to_owned(),
            authenticated: "localhost:4432".to_owned(),
        },
        ttl_seconds: 60,
        now_microseconds: 1,
    };
    load_or_bootstrap(&mut database, &provider, &input)?;
    let published = begin_host_key_rotation(&mut database, &provider, 2)?;
    std::fs::write(
        output.join("host-add.probe.snowp"),
        database
            .host_bootstrap()?
            .ok_or("missing host bootstrap after addition")?
            .probe_response,
    )?;
    complete_host_key_rotation(
        &mut database,
        &provider,
        published.operation_id,
        HostKeyRotationObservation {
            add_link_seqno: published
                .add_link_seqno
                .ok_or("missing addition sequence")?,
        },
        published
            .observation_not_before
            .ok_or("missing observation deadline")?,
    )?;
    std::fs::write(
        output.join("host-revoke.probe.snowp"),
        database
            .host_bootstrap()?
            .ok_or("missing host bootstrap after revocation")?
            .probe_response,
    )?;
    Ok(())
}
