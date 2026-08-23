# foks-client

Native, non-interactive discovery of FOKS v0.1.9 public hosts. It connects
over WebPKI-authenticated TLS, performs the public probe RPC, verifies the
host chain and signed Merkle state, and atomically advances SQLite hard state.

The client can also obtain an enrolled device certificate without interaction,
derive its exact Ed25519 mTLS key, advance the signed Merkle pin through the
official historical-root protocol, replay device and PUK transitions, fetch
and unbox the current owner PUK, and commit the public user projection
atomically. Merkle advancement always starts from SQLite hard state, so an
untrusted user response can never bless its own root.

See [SECURITY.md](SECURITY.md) for the trust boundaries, invariant ownership,
secret lifecycle, review order, and explicitly unimplemented surfaces.
