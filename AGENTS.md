This repository contains a standalone Rust implementation of the FOKS
(federated open key store) protocol, its client and server components, and the
FOKS desktop application.

The project has not shipped a first version. Protocol and schema changes should
be explicit and tested, but do not need product migration or compatibility
handling unless an external protocol such as go-foks interoperability requires
it.

For commit messages, include a short description followed by paragraph(s). Use
multiple `-m` arguments instead of embedded `\n` escapes when committing from
the command line.

Before running `cargo test --locked --workspace`, build the standalone agent
with `cargo build --locked -p foks-agent`. CLI integration tests launch the
sibling `target/debug/foks-agent` binary, which `cargo test` can leave stale
when the hard-state schema changes. Use limited Cargo job concurrency and run
Rust and UI suites separately when disk space is constrained.

Run native desktop unit tests serially with
`cargo test --locked -j 2 -p foks-desktop-app --lib -- --test-threads=1`.
A parallel run can intermittently fail the first-run receipt-lock release
assertion; rerun that test in isolation and the native suite serially before
attributing the failure to a code change.
