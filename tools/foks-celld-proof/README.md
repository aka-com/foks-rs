# celld proof authority

`authority.mjs` is the SQL authority the celld proof Worker runs: it
replays the native delta that `foks-server-testkit`'s `celld_proof` module
produces (`Schema::delta`) and accepts it only when its own database reaches
the delta's `after_hash`. `run-local.py` runs a local qualification against
a celld build and records the SHA-256 of `worker.mjs`, `authority.mjs`,
`node-decoder.mjs` and its other replay inputs as evidence.

State hashes leave out trigger-owned counters that count trigger firings
rather than describe rows (`metadata_read_dependency.revision`): the replay
writes a new `kv_nodes` row in two statements and so fires more triggers
than the native write. `UNHASHED_COUNTERS` in `authority.mjs` and in
`crates/foks-server-testkit/src/celld_proof.rs` must list the same columns;
the authority checks that those counters never move back and that a base
write advances them. No proof covers their values, so an output routed
through the proof must not include them; the read-dependency revision is
the read coordinate of a read token.

Changing `authority.mjs` changes its recorded digest, so lab qualification
evidence recorded before the change no longer describes the current
authority and has to be recorded again.

Tests: `node --test` here runs the artifact-free files and reports the
authority and node-decoder cases as skipped unless `FOKS_PROOF_VECTORS` and
`FOKS_PROOF_NODE_DECODER_WASM` are set. Generate the vectors with `cargo run
-p foks-server-testkit --features celld-proof --example celld-proof-vectors
-- DIR` and set `FOKS_PROOF_VECTORS=DIR/vectors.json`; the decoder is the
`wasm32-unknown-unknown` build of `foks-metadata-core`.
