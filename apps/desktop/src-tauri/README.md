# `apps/desktop/src-tauri` — the FOKS desktop application

The Tauri app places the native agent, catalog, exact-version reads, app lock,
clipboard, downloads, guarded writes, and streaming file ingress behind the
React product shell.

- Rust package: `foks-desktop-app`; binary: `foks-desktop`.
- Web app: `..`, served at `http://127.0.0.1:1421` in development.
- Bundle identifier: `org.foks.desktop`.
- Managed helper: `foks-agent`, discovered beside the desktop executable (`Contents/MacOS` on macOS).

## Development

The root npm scripts run Tauri from this directory so it selects this
`tauri.conf.json`:

```sh
npm run dev          # local server, agent, and desktop application
npm run dev:tauri    # Tauri only, using an already-running agent
npm run build        # macOS app/DMG or Linux Debian package
npm run build:release # signed, notarized, stapled macOS release DMG
```

`dragDropEnabled` remains enabled so Rust receives native file paths without
loading secret file bytes into the renderer. `withGlobalTauri` remains disabled;
the bridge imports the narrow Tauri API explicitly.

## Native packages

The package scripts build and stage `foks-agent` with the target suffix Tauri
expects, then let the Tauri CLI assemble the platform bundle:

```sh
npm run bundle:macos
npm run bundle:deb
```

The macOS application and DMG are written under `target/release/bundle/macos/`
and `target/release/bundle/dmg/`, respectively. `scripts/sign-macos-app.sh`
signs the managed helper and outer application with `APPLE_SIGNING_IDENTITY`;
if it is unset, the script accepts exactly one installed Developer ID
Application identity. `scripts/notarize-macos-app.sh` creates a DMG containing
the signed application, signs the disk image, then submits, staples, and
validates it using `APPLE_ID`, `APPLE_TEAM_ID`, and `APPLE_APP_PASSWORD`.

For a distributable macOS release, use `npm run build:release`. It requires
`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_TEAM_ID`, and
`APPLE_APP_PASSWORD` up front, validates the identity and notary credentials,
builds only the application, signs it, and publishes the final DMG under
`target/release/foks-release/` only after notarization, stapling, and Gatekeeper
validation succeed. Files under `target/release/bundle/dmg/` are ordinary
unsigned development bundles.

Linux packaging requires `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`libpcsclite-dev`, and `librsvg2-bin`. See `linux/README.md` for the installed
layout.

## Checks

```sh
cargo test --locked -p foks-desktop
cargo test --locked -p foks-desktop-app
cargo clippy --locked -p foks-desktop -p foks-desktop-app --all-targets -- -D warnings
cargo fmt -p foks-desktop -p foks-desktop-app -- --check
npm run typecheck
npm run test:foks-ui
npm run acceptance:foks-ui
```

The Tauri package is a workspace member but is excluded from the default
members because it requires the platform webview toolchain.

After assembling a macOS bundle, run `npm run test:foks-desktop:packaged`.
This invokes the bundled executable without a webview, checks production mode,
discovers and launches its adjacent agent, and verifies a real AgentStatus reply.
It uses temporary private state and terminates the test agent. It does not test
window rendering, signing, or notarization. For an extracted Linux package, pass
its executable: `npm run test:foks-desktop:packaged -- /path/to/usr/bin/foks-desktop`.

## Command domains

`src/commands/mod.rs` declares the command domains; `src/lib.rs` registers their
handlers with Tauri. Command names and wire DTO shapes stay aligned with
`wire-contract.json` and the frontend bridge.

| Module | Responsibility |
| --- | --- |
| `accounts` | Account inventory, devices, passphrases, and owner backups |
| `enrollment` | First-run setup, recovery, software pairing, and Go CLI handoff |
| `yubikey` | Hardware enrollment, synchronization, and credential lifecycle |
| `vault` | Catalogs, version-bound reads/writes, clipboard, and file transfer |
| `groups` | Group discovery, membership, admission, and federation |
| `servers` | Profiles, trust verification, status, and local reset |
| `application` | Agent connectivity and application information |
| `context` | Shared state, cached authorization facts, and mutation coordination |
| `validation` | Shared input bounds, typed identities, secrets, and window checks |
| `execution` | Transport preflight, mutation execution, and ambiguous outcomes |
| `types` | Small wire types shared across domains |

Domain response decoding stays with its handlers. Shared helpers have visibility
limited to the command layer. Tests live under `src/commands/tests/`, grouped by
domain, with cross-domain wire-contract tests and shared fixtures kept separately.
