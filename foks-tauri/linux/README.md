# Linux packaging for FOKS desktop

The supported Linux bundle is the x86-64 Debian package. Build it from the
repository root; the package script stages the managed agent under the exact
Tauri sidecar name:

```sh
pnpm run foks:bundle:deb
```

`tauri.linux.conf.json` is a release overlay. Keeping `externalBin` out of the
base configuration lets ordinary `cargo check` and desktop development run
without a staged binary; a package build fails if its sidecar is missing.

## Installed boundary

The package name and executable are both `foks-desktop`. The package contains:

```text
/usr/bin/foks-desktop
/usr/bin/foks-agent
/usr/share/applications/foks-desktop.desktop
/usr/share/metainfo/org.foks.Desktop.metainfo.xml
/usr/share/polkit-1/actions/org.foks.desktop.policy
```

Tauri generates the single desktop entry from `foks.desktop.hbs`. No launcher
shim and no systemd user service are installed: the desktop owns managed-agent
launch, advisory locking, connection observation, and the private state
directory.

The explicit dependencies are `policykit-1` and `libpcsclite1`; Tauri adds its
GTK and WebKitGTK runtime dependencies. Building needs
`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libpcsclite-dev`, and `librsvg2-bin`.
PC/SC is required by the security-key operations compiled into the managed
agent.

## Why there is no AppImage

Linux application lock authenticates through the installed
`org.foks.desktop.policy`. An AppImage has no install step for that system
policy, so lock would be unavailable. The portable tarball has the same honest
limitation and is an evaluation artifact; it is also not self-contained and
requires host WebKitGTK 4.1, GTK 3, and PC/SC runtimes. Use the `.deb` for
supported use.

CI verifies package name, both executables, polkit and AppStream files, runtime
dependencies, and the absence of the retired launcher/service. The remaining
release gate is a graphical installed-package run that exercises the real
polkit prompt and PC/SC hardware; it cannot be replaced by archive inspection.
