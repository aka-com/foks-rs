# FOKS desktop release policy

The GPUI application and its local agent are FOKS-only artifacts. They are not
linked to, installed over, or configured from any AKA crate or data directory.

## Platforms and toolchains

- macOS 13+ on Apple Silicon and Intel. Builds need stable Rust, Cargo, Clang,
  and the macOS SDK. GPUI uses runtime Metal shader compilation, so Command
  Line Tools are enough to compile; release signing/notarization needs Xcode's
  `codesign`, `notarytool`, and `stapler` plus a Developer ID Application
  certificate.
- Linux x86-64 and arm64 with Wayland or X11. Builds need stable Rust, Cargo,
  Clang/GCC, pkg-config, fontconfig/freetype, libxkbcommon, Wayland, XCB, and a
  Vulkan 1.2-capable driver at runtime.
- Windows is intentionally not built or packaged.

## Signing and publication

macOS releases sign the helper first and the app bundle second with the
hardened runtime and an empty entitlement set. CI notarizes the zip and staples
the accepted ticket before publication. Linux tarballs receive a keyless
Sigstore attestation tied to the release workflow; distribution packages may
add their repository signature without replacing it. Unsigned developer builds
are visibly separate and must never be placed in the release channel.

## Updates

Version 1 has no privileged or in-app updater. The application is updated by a
notarized macOS replacement or the Linux package manager. Release manifests and
checksums are signed/attested by CI. This avoids introducing a second trusted
download-and-execute stack before update rollback and key rotation are designed.

## Crash reports

Crash upload is disabled. When launched with `--state-dir`, a panic writes a
private local marker containing only application version, timestamp, and OS—no
panic payload, backtrace, paths, protocol data, account names, or secrets. A
future upload flow requires explicit opt-in, preview/redaction, endpoint and
retention documentation, and a separate security review.
