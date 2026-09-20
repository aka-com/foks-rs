# foks-rs

A Rust implementation of FOKS, the federated open key store protocol,
including client and server libraries, command-line tools, a local
agent, and a Tauri desktop application.

The desktop app is a client for the local FOKS agent. The FOKS agent
owns credentials, sessions, protocol verification, resumable
operations, and access to personal and team key-value stores.

## Development

Install the Rust toolchain declared in `rust-toolchain.toml`, the exact Node.js
version in `.node-version`, and the npm version declared in `package.json`, then
install frontend dependencies:

```sh
npm install --global npm@11.12.1
npm ci

npm run dev             # Run with auto-reload, and a local FOKS server and agent
npm run start           # Run with auto-reload, for an existing FOKS agent

npm run bundle:macos    # Create bundled DMG
npm run build:frontend  # Build frontend-only mock for browser testing
```

Tests and checks:

```sh
npm run test:core       # seven core Rust crates + UI tests
npm run test:full       # entire Rust workspace + UI tests
npm run format
npm run lint
npm run typecheck
npm run test:foks-ui       # full UI coverage, including the notification scale case
npm run test:foks-ui:fast  # ordinary UI iteration
npm run test:foks-ui:scale # production-size notification rotation
npm run test:rust:scale   # production-size incremental history
```

`cargo` builds and tests the Rust packages. npm drives Vite, TypeScript,
ESLint, UI tests, and the Tauri CLI.

- The complete desktop and release workflow and platform prerequisites are documented in
  [`apps/desktop/src-tauri/README.md`](apps/desktop/src-tauri/README.md).
- Protocol and server validation live under [`tools/foks-server/`](tools/foks-server/)
- The pinned Go compatibility oracle lives under [`tools/foks-v019-oracle/`](tools/foks-v019-oracle/).

The project is pre-v1. Compatibility with the upstream Go protocol is tested
explicitly; internal storage and application schemas may otherwise change
without migration support.

`npm test` aliases `test:core`. Full tests need native desktop dependencies;
explicitly ignored hardware, hosted, and real-agent tests remain opt-in.
`npm run test:rust:core` and `npm run test:rust:full` run only Rust tests.
The full commands retain the production-size cases; the ordinary agent unit
suite uses a smaller history page to cover the same cursor boundaries quickly.
`test:rust:full` builds the standalone agent first and includes `test:rust:scale`.
Both Rust commands accept build flags, for example `-- --release`.
For a focused UI edit, pass the affected files directly to `npx tsx --test`.

Use `npm run build` for a production executable (embedded frontend),
and `npm run test:foks-desktop:packaged` after building the macOS bundle.

## Benchmarks

The real-process chat benchmark runs desktop TypeScript services against release
Rust agent and server processes, with six incoming messages per second and
concurrent foreground history reads and message writes. Linux x86_64 results with
notifications enabled, comparing `a4d69ad0` with the publication/fanout changes
in `df95f9e9` plus timing fixes:

| Workload | Foreground history p95 before | After |
| --- | ---: | ---: |
| Steady traffic, 70 channels | 44.9 ms | 43.7 ms |
| Message backlog, 70 channels | 39.2 ms | 38.4 ms |
| Steady traffic, 200 channels | 44.7 ms | 45.1 ms |

Values are medians of three trial p95s, each with 15 seconds of warmup, 60 seconds
of measurement and a 10-second drain. They include the service/agent/server path
but exclude desktop rendering. Notification discovery missed the 99% target in
some steady-traffic trials, so these latency results are **not an acceptance pass**.
See the [benchmark guide](scripts/benchmarks/README.md) for reproduction and the
[validation report](docs/inbox-publication-fanout-validation.md) for details.

## Repository layout

- `apps/desktop/src/`: React frontend, including its `main.tsx` entry point.
- `apps/desktop/kit/`: shared desktop UI components and design tokens.
- `apps/desktop/src-tauri/`: native desktop shell and packaging configuration.
- `apps/desktop/tests/`: frontend unit, rendering, and browser acceptance tests.
- `crates/foks-*`: independent protocol, client, server, agent, and support crates.

The root `package.json` and `package-lock.json` own frontend dependencies and
commands. The desktop has no separate npm package; run npm commands from the
repository root. The root Cargo workspace and lockfile cover all Rust crates.
