# Parser and archive fuzzing

Install a nightly Rust toolchain and `cargo-fuzz`, then run from the repository:

```sh
cargo +nightly fuzz run snowpack -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run rpc -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run server_rpc -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run agent_frames -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run state_archive -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
```

The independent workspace and lockfile keep fuzzing tools out of production
dependencies. Snowpack checks canonical round trips and sensitive decoding;
RPC exercises response envelopes and length-prefixed framing. `server_rpc`
exercises the production call and control-message parsers with bare/framed
inputs, generated valid calls, and malformed optional log tags. `agent_frames`
exercises request correlation IDs, protocol versions, JSON requests and streamed
upload frames, with bounded canonical round trips. Archive fuzzing combines
raw input (seeded with the authenticated compatibility vector) and generated
authenticated archives, including empty and multi-chunk entries, to reach
entry and trailer validation. An independent test-only encoder also generates
validly authenticated but malformed entry ordering, chunk indexes/finality,
trailer hashes/counts/lengths, truncation, early finish and failing output sinks.
The scenario tests (`cargo test --manifest-path fuzz/Cargo.toml --test
archive_scenarios`) verify these expectations, including chunk boundaries.
All keys and content are synthetic; the deterministic encoder is never linked
into production. Regenerate the small structured seeds after protocol changes:

```sh
cargo run --locked --manifest-path fuzz/Cargo.toml --example seed_corpus
```

Pull requests run short campaigns and scheduled jobs run longer campaigns.
Reproduce a failure with `cargo +nightly fuzz run TARGET ARTIFACT`; minimize with
`cargo +nightly fuzz tmin TARGET ARTIFACT`. Turn minimized failures into ordinary
regression tests and commit useful non-secret corpus seeds. Generated artifacts
and build outputs are ignored; CI uploads failing artifacts.
