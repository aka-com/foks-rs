# FOKS

A standalone Rust implementation of the federated open key store protocol,
including client and server libraries, command-line tools, a local agent, and a
Tauri desktop application.

The desktop is an untrusted presentation client for the local FOKS agent. The
agent owns credentials, sessions, protocol verification, resumable operations,
and access to personal and team key-value stores.

## Development

Install the Rust toolchain declared in `rust-toolchain.toml`, Node.js 22, and
the pnpm version declared in `package.json`, then install frontend dependencies:

```sh
corepack enable
pnpm install
```

Common commands:

```sh
cargo test --workspace
pnpm run lint
pnpm run typecheck
pnpm run test:foks-ui
pnpm run foks:build:frontend
pnpm start
```

`cargo` builds and tests the Rust packages. pnpm drives Vite, TypeScript,
ESLint, UI tests, and the Tauri CLI. `pnpm run foks:bundle:macos` and
`pnpm run foks:bundle:deb` stage the managed agent and create native packages.

The complete desktop and release workflow and platform prerequisites are documented in
[`foks-tauri/README.md`](foks-tauri/README.md). Protocol and server validation
live under [`tools/foks-server/`](tools/foks-server/), while the pinned Go
compatibility oracle lives under
[`tools/foks-v019-oracle/`](tools/foks-v019-oracle/).

The project is pre-v1. Compatibility with the upstream Go protocol is tested
explicitly; internal storage and application schemas may otherwise change
without migration support.
