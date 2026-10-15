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
- Release jobs start only after `.github/workflows/foks-dependencies.yml`
  passes for the tagged commit: `cargo deny` checks advisories, licenses, bans
  and sources against `deny.toml` for both Cargo lockfiles, and
  `npm audit --audit-level=high` checks the full npm tree. `deny.toml` fails
  on unmaintained advisories for every crate in the graph, not only direct
  dependencies, so a new advisory against a crate that cannot be updated
  blocks the release until it is added to the `ignore` list with a reason.
- `foks-desktop` and `foks-agent` are built with `cargo auditable`
  (`FOKS_AUDITABLE_BUILD=1`, see `scripts/foks-cargo.sh`), so each shipped
  executable embeds its resolved dependency list.
  `scripts/verify-auditable-binaries.sh` checks both executables in each
  built bundle, and `cargo audit bin <executable>` checks a released binary
  against advisories published after the release.

The release pipeline must fail if a signing identity, notarization credential,
managed agent, expected metadata file, expected executable, or embedded
dependency list is missing.
Unsigned macOS artifacts must never be published.

The application icons are placeholders until project-owned FOKS artwork is
available. Replace the files under `apps/desktop/src-tauri/icons/` before the first public
release.
