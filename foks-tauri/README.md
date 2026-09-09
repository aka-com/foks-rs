# `foks-tauri` — the FOKS desktop application

The Tauri app places the native agent, catalog, exact-version reads, app lock,
clipboard, downloads, guarded writes, and streaming file ingress behind the
React product shell.

- Rust package: `foks-desktop-app`; binary: `foks-desktop`.
- Web app: `../foks-ui`, served at `http://127.0.0.1:1421` in development.
- Bundle identifier: `org.foks.desktop`.
- Managed helper: `foks-agent`, discovered beside the packaged application.

## Development

The root pnpm scripts run Tauri from this directory so it selects this
`tauri.conf.json`:

```sh
pnpm run foks:dev          # local server, agent, and desktop application
pnpm run foks:dev:tauri    # Tauri only, using an already-running agent
pnpm run foks:build        # production frontend and desktop executable
```

`dragDropEnabled` remains enabled so Rust receives native file paths without
loading secret file bytes into the renderer. `withGlobalTauri` remains disabled;
the bridge imports the narrow Tauri API explicitly.

## Native packages

The package scripts build and stage `foks-agent` with the target suffix Tauri
expects, then let the Tauri CLI assemble the platform bundle:

```sh
pnpm run foks:bundle:macos
pnpm run foks:bundle:deb
```

The macOS application is written under `target/release/bundle/macos/`.
`scripts/sign-macos-app.sh` signs the managed helper and outer application with
`APPLE_SIGNING_IDENTITY`; if it is unset, the script accepts exactly one
installed Developer ID Application identity. `scripts/notarize-macos-app.sh`
submits, staples, validates, and archives the signed application using
`APPLE_ID`, `APPLE_TEAM_ID`, and `APPLE_APP_PASSWORD`.

Linux packaging requires `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`libpcsclite-dev`, and `librsvg2-bin`. See `linux/README.md` for the installed
layout.

## Checks

```sh
cargo test --locked -p foks-desktop
cargo test --locked -p foks-desktop-app
cargo clippy --locked -p foks-desktop -p foks-desktop-app --all-targets -- -D warnings
cargo fmt -p foks-desktop -p foks-desktop-app -- --check
pnpm run typecheck
pnpm run test:foks-ui
pnpm run acceptance:foks-ui
```

The Tauri package is a workspace member but is excluded from the default
members because it requires the platform webview toolchain.
