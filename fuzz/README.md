# Parser and archive fuzzing

Install a nightly Rust toolchain and `cargo-fuzz`, then run from the repository:

```sh
cargo +nightly fuzz run snowpack -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run rpc -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run server_rpc -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run agent_frames -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run state_archive -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run proto_schema -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run server_arguments -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
cargo +nightly fuzz run verify -- -max_total_time=60 -max_len=2097152 -rss_limit_mb=2048
```

The independent workspace and lockfile keep fuzzing tools out of production
dependencies. Snowpack checks canonical round trips and sensitive decoding;
RPC exercises the client response envelopes (data, void, bare and bare void)
and length-prefixed framing. `server_rpc`
exercises the production call and control-message parsers with bare/framed
inputs, generated valid calls, and malformed optional log tags. `agent_frames`
exercises request correlation IDs, protocol versions, JSON requests, Hello
requests of any version, streamed upload frames and the client's response and
Hello-reply decoders, with bounded canonical round trips. Archive fuzzing combines
raw input (seeded with the authenticated compatibility vector) and generated
authenticated archives, including empty and multi-chunk entries, to reach
entry and trailer validation. An independent test-only encoder also generates
validly authenticated but malformed entry ordering, chunk indexes/finality,
trailer hashes/counts/lengths, truncation, early finish and failing output sinks.
The scenario tests (`cargo test --manifest-path fuzz/Cargo.toml --test
archive_scenarios`) verify these expectations, including chunk boundaries.
All keys and content are synthetic; the deterministic encoder is never linked
into production.

`proto_schema`, `server_arguments` and `verify` use the first input byte as a
selector over a table in `support/`. `proto_schema` reaches every typed
`foks-proto` decoder, including the realtime types through `RealtimeWire`.
Where a type has an encoder, a decoded value that encodes must decode again to
the same encoding; KV types that retain their decoded bytes must return their
input. `server_arguments` reaches every `foks_rpc::arguments` decoder and
`RealtimeRequest::decode_argument` at each realtime method position, whose
re-encoded request must decode to an equal request. `verify` authenticates the
official foks.app probe and Merkle roots once, then replaces one input of
`verify_public_host`, the self and non-self user chain, the team chain, the
Merkle advance root or history, or the signed Merkle root, so mutations reach
chain, proof and signature checks.

The 137 canonical Go v0.1.9 `.snowp` fixtures seed the `snowpack` corpus as
they are, the `rpc` corpus as framed success responses, and the
`proto_schema` and `server_arguments` corpora once per accepting decoder,
behind its selector byte. Calls built by the client encoders around the fixture
IDs add well-formed `server_arguments` seeds, and each `verify` stage is
seeded with the fixture it accepts. Generated seeds are named `go-*` and
`call-*`. The seed tests (`cargo test --manifest-path fuzz/Cargo.toml --test
corpus_seeds`) run every table entry on every fixture, require Go fixtures to
re-encode to their exact bytes, and fail when a seed no longer selects the
decoder in its name, which happens when a table changes order. Regenerate the
seeds after protocol or table changes:

```sh
cargo run --locked --manifest-path fuzz/Cargo.toml --example seed_corpus
```

Pull requests run short campaigns and scheduled jobs run longer campaigns.
Reproduce a failure with `cargo +nightly fuzz run TARGET ARTIFACT`; minimize with
`cargo +nightly fuzz tmin TARGET ARTIFACT`. Turn minimized failures into ordinary
regression tests and commit useful non-secret corpus seeds. Generated artifacts
and build outputs are ignored; CI uploads failing artifacts.
