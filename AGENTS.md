This repository contains a standalone Rust implementation of the FOKS
(federated open key store) protocol, its client and server components, and the
FOKS desktop application.

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

If CLI integration readiness checks report an IPC protocol version mismatch
while Cargo reports the standalone agent as fresh, force a relink with
`cargo rustc --locked -j 2 -p foks-agent --bin foks-agent -- -C metadata=agent-readiness-recheck`
and rerun the failing test. A successful ordinary build alone did not replace
a stale standalone executable during desktop synchronization verification.
