# foks-rpc

Strict client-side framing for the FOKS v0.1.9 Snowpack RPC protocol.

The RPC envelope is ordinary MessagePack and uses named maps. Authenticated
FOKS objects inside it are canonical Snowpack. This crate keeps those layers
separate and returns exact result bytes for protocol verification. Request
encoders cover discovery, delegated virtual-host selection, registration,
user and team chains, PUK/PTK retrieval, account and device mutations, and KV
reads and writes. Official Go differential fixtures cover the protocol-facing
frames.

Wire protocol IDs, method positions, and exposed upstream status constants are
generated from the checksum-pinned v0.1.9 metadata under
`crates/foks-server/protocol`; this crate has no Go build or runtime dependency.
