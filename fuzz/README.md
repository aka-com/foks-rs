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
Where a type has an encoder, including the KV types that convert only to and
from a Snowpack value, a decoded value that encodes must decode again to the
same encoding; KV types that retain their decoded bytes must return their
input. `server_arguments` reaches every `foks_rpc::arguments` decoder and
`RealtimeRequest::decode_argument` at each realtime method position, whose
re-encoded request must decode to an equal request. `verify` authenticates the
official foks.app probe and Merkle roots once, then replaces one input of
`verify_public_host`, the self and non-self user chain, the team chain, the
Merkle advance root or history, or the signed Merkle root, so mutations reach
chain, proof and signature checks. The user and team chain stages also run the
root-epoch discovery for chain responses and persisted evidence. The increment
stages replace the refresh response given to `verify_user_chain_increment`,
`verify_non_self_user_chain_increment` and `verify_team_chain_increment`, whose
prior state is the verified fixture chain.

The 137 canonical Go v0.1.9 `.snowp` fixtures seed the `snowpack` corpus as
they are, the `rpc` corpus as framed success responses, and the
`proto_schema` and `server_arguments` corpora once per accepting decoder,
behind its selector byte. The arguments of the Go `.frame` and `.rpc` request
fixtures seed the same two corpora; these seeds are named `go-*-call`. Calls
built by the client encoders around the fixture IDs add `server_arguments`
seeds for calls that no Go request covers. Each `verify` stage is seeded with
the fixture it accepts; the increment stages are seeded with the refresh
response for a chain that has not advanced, derived from the fixture chain.
Generated seeds are named `go-*` and `call-*`. The seed tests (`cargo test
--manifest-path fuzz/Cargo.toml --test corpus_seeds`) run every table entry on
every fixture and Go call argument, require Go inputs to re-encode to their
exact bytes, and fail when a seed no longer selects the decoder in its name,
which happens when a table changes order. Regenerate the seeds after protocol
or table changes:

```sh
cargo run --locked --manifest-path fuzz/Cargo.toml --example seed_corpus
```

Twelve `server_arguments` selectors have no seed yet. No Go request fixture
covers them, and their client encoders take signatures, registration
challenges or signed links (`decode_post_generic_link`,
`decode_post_team_membership_link`, `decode_passphrase_login`,
`decode_set_passphrase`, `decode_change_passphrase`,
`decode_put_yubi_management_key`, `decode_load_subkey_box` and the user and
team `decode_grant_remote_view_permission_*` decoders), or `foks_rpc` has no
client encoder for them (`decode_check_team_bearer_token`,
`decode_check_team_view` and `decode_load_removal_for_member`). Their campaigns
start from mutations of the other seeds.

Pull requests run short campaigns and scheduled jobs run longer campaigns.
Reproduce a failure with `cargo +nightly fuzz run TARGET ARTIFACT`; minimize with
`cargo +nightly fuzz tmin TARGET ARTIFACT`. Turn minimized failures into ordinary
regression tests and commit useful non-secret corpus seeds. Generated artifacts
and build outputs are ignored; CI uploads failing artifacts.
