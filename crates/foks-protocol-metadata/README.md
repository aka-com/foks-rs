# foks-protocol-metadata

Rust-only validation and deterministic rendering for the server's FOKS v0.1.9
contract. It merges checked upstream metadata with handwritten local policy and
generates the public RPC constants, server route registry, and
`protocol-v1.toml`.

This crate is publish-disabled, is not a default workspace member, and has no
Go or network dependency. Normal Cargo and Bazel builds consume checked Rust
and data files. Go is used only by the explicit extractor and mainline audit in
`tools/foks-protocol-sync`.
