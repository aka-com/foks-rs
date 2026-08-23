# foks-verify

`foks-verify` turns untrusted FOKS v0.1.9 host, Merkle, user, and team evidence
into sealed capabilities that `foks-client-db` can commit atomically.

It verifies:

- host-chain sequence, previous hashes, immutable HostID, changes, revocation,
  and stacked signatures;
- TLS CA certificates against their Ed25519 EntityIDs;
- public-zone signatures through active delegated metadata keys;
- Merkle-root signatures through active delegated Merkle keys; and
- the Merkle root's commitment to the accepted host-chain tail.

The verifier performs no network or database I/O. It also replays persisted
evidence before recreating a sealed capability; parsed SQLite projections are
never accepted independently.

The offline command-line example verifies a captured response:

```sh
cargo run -p foks-verify --example verify_probe -- \
  crates/foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp \
  foks.app
```

It performs no database writes.

The authenticated slice exposes `verify_user_chain` and
`verify_merkle_advance`. It checks the official skip/back-pointer graph from
the signed SQLite pin, stacked signatures, HEPK bindings, tree-location
commitments, every presence proof, the next-link absence proof, device
provisioning and revocation, and PUK generation rotation. The same replay path
handles both an eldest-only chain and later transitions.
