# FOKS desktop release policy

FOKS desktop is a Tauri application with a managed `foks-agent` and an
independent release train.

- macOS releases are arm64 application ZIPs. `pnpm run foks:bundle:macos`
  assembles the application and embeds the agent. The helper and outer bundle
  are signed with hardened runtime and a secure timestamp, then the application
  is notarized and stapled before the final ZIP is created.
- Linux releases are x86_64 Debian packages made by
  `pnpm run foks:bundle:deb`. They install `foks-desktop`, `foks-agent`, the
  polkit action, one desktop entry, and AppStream metadata.
- A tag named `foks-desktop-v<workspace-version>` is required for release.
  CI verifies that the tag and Cargo workspace version match.
- Release jobs publish SHA-256 checksums and GitHub build-provenance
  attestations for both platform artifacts.

The release pipeline must fail if a signing identity, notarization credential,
managed agent, expected metadata file, or expected executable is missing.
Unsigned macOS artifacts must never be published.

The application icons are placeholders until project-owned FOKS artwork is
available. Replace the files under `foks-tauri/icons/` before the first public
release.
