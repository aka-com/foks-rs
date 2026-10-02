# Parser and archive fuzzing

Install a nightly Rust toolchain and `cargo-fuzz`, then run from the repository:

```sh
cargo +nightly fuzz run snowpack -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run rpc -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run state_archive -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
```

The independent workspace and lockfile keep fuzzing tools out of production
dependencies. Snowpack checks canonical round trips and sensitive decoding;
RPC exercises envelope and length-prefixed framing. Archive fuzzing combines
raw input (seeded with the authenticated compatibility vector) and generated
authenticated archives, including empty and multi-chunk entries, to reach
entry and trailer validation. All keys and content are synthetic.

Pull requests run short campaigns and scheduled jobs run longer campaigns.
Reproduce a failure with `cargo +nightly fuzz run TARGET ARTIFACT`; minimize with
`cargo +nightly fuzz tmin TARGET ARTIFACT`. Turn minimized failures into ordinary
regression tests and commit useful non-secret corpus seeds. Generated artifacts
and build outputs are ignored; CI uploads failing artifacts.
