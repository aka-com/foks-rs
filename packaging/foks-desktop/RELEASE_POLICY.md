# FOKS desktop release policy

FOKS desktop is a Tauri application with a managed `foks-agent` and an
independent release train.

- macOS releases are arm64 DMGs. `npm run build:release` validates every
  signing and notarization input before staging the agent or building the
  application. The helper and application are signed with hardened runtime and
  a secure timestamp, then a signed DMG is created, notarized, stapled, and
  exposed in the release directory only after validation succeeds.
- Linux releases are x86_64 Debian packages made by
  `npm run bundle:deb`. They install `foks-desktop`, `foks-agent`, the
  polkit action, one desktop entry, and AppStream metadata.
- A tag named `foks-desktop-v<workspace-version>` is required for release.
  CI verifies that the tag and Cargo workspace version match.
- Release jobs publish SHA-256 checksums and GitHub build-provenance
  attestations for both platform artifacts.

The release pipeline must fail if a signing identity, notarization credential,
managed agent, expected metadata file, or expected executable is missing.
Unsigned macOS artifacts must never be published.

The application icons are placeholders until project-owned FOKS artwork is
available. Replace the files under `apps/desktop/src-tauri/icons/` before the first public
release.
