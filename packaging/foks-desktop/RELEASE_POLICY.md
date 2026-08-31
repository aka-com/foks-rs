# FOKS desktop release policy

FOKS desktop is a Tauri application with a managed `foks-agent`. It is a
separate product and release train from AKA. The desktop owns the agent process
and private state directory; packages do not install a systemd or launchd
service.

## Supported artifacts

- macOS 13 or newer on Apple Silicon: a Developer-ID-signed, Apple-notarized,
  stapled ZIP containing `FOKS.app`. The managed agent is signed first and lives
  at `Contents/Helpers/foks-agent`; the outer bundle is signed second. DMG is not
  a FOKS release format.
- Debian-family Linux on x86-64: `foks-desktop.deb`, containing sibling
  `/usr/bin/foks-desktop` and `/usr/bin/foks-agent`, the polkit action, one
  desktop entry, and AppStream metadata. It depends on WebKitGTK/GTK (added by
  the Tauri bundler), `policykit-1`, and `libpcsclite1`.

Windows, Intel macOS, and Linux arm64 are not release targets. Adding one needs
an explicit native toolchain, packaging job, and runtime/hardware acceptance;
the release workflow does not label an unbuilt architecture as supported.

## Signing, notarization, and provenance

The macOS release target fails when no Developer ID identity is available.
Release CI submits an Apple-metadata-preserving ZIP, waits for notarization,
staples and validates the application, applies Gatekeeper assessment, and only
then creates the downloadable ZIP. Signing and notarization credentials are not
available to pull-request jobs. ZIP—not DMG—is the notarization policy.

The tag workflow creates SHA-256 checksums and GitHub build-provenance
attestations after both platform jobs succeed. The attestation job has OIDC and
attestation permission but no publication permission; the final publication
job has release-content permission but no OIDC or signing secrets. All external
actions are pinned to full commits.

There is no privileged or in-app updater in version 1. macOS updates replace the
notarized application, and Linux updates use the package manager.

## External release gates

The repository can verify source, Cargo, Bazel, bundle layout, Debian package,
and unsigned application assembly locally. A production release additionally
requires all of the following outside an ordinary checkout:

- Apple Developer ID certificate and notarization credentials; final checks run
  on a real Apple Silicon macOS runner with `codesign`, `notarytool`, `stapler`,
  and Gatekeeper.
- GitHub-hosted OIDC attestation and release publication for a
  `foks-desktop-v*` tag.
- A Linux x86-64 runner with WebKitGTK, GTK, PC/SC development libraries and
  Tauri's bundling tools; the generated `.deb` is unpacked and audited in CI.
- Installed-package testing on a graphical Debian system, including the polkit
  unlock prompt and PC/SC hardware enrollment/revocation with a real supported
  security key.

Publisher and homepage metadata are intentionally omitted until the release
owner supplies canonical values. CI must not invent them.

## Crash reports

Crash upload is disabled. Only the default managed private state directory
receives markers; an argument- or environment-selected agent socket never
authorizes a neighboring write. The marker contains only application version,
timestamp, and OS—no panic payload, backtrace, paths, protocol data, account
names, or secrets. Any future upload flow requires explicit opt-in,
preview/redaction, endpoint and retention documentation, and a separate review.
