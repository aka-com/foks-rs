#!/usr/bin/env bash
# Builds stand-in native libraries and pkg-config files so the workspace
# compiles and its tests run in a container without pcsclite, D-Bus or the GTK
# and WebKit development packages.
#
# The stubs export every symbol the corresponding -sys crate declares, as
# empty functions, and pkg-config files that claim the versions the crates
# require. They let `cargo build`, `cargo test` and `cargo check` proceed;
# nothing that calls into a smart card, the Secret Service or a window will
# work at run time. The agent's integration tests use the private-file
# credential backend and never reach those calls; the Tauri crate can be
# type-checked and linted but not run.
#
# Usage:
#   source <(scripts/container-native-stubs.sh [directory])
# prints and applies the PKG_CONFIG_PATH and LD_LIBRARY_PATH exports. Apply
# them to cargo commands only: the stub libdbus-1 shadows the system library,
# and a browser started under that LD_LIBRARY_PATH (the acceptance harness's
# Chromium) fails in the dynamic loader.
set -euo pipefail

stubs="${1:-${TMPDIR:-/tmp}/foks-native-stubs}"
mkdir -p "$stubs/include"
# Fetch sources before extracting symbols, including on a fresh Cargo home.
# Keep stdout reserved for the exports that the caller sources.
cargo fetch --locked --manifest-path "$(dirname "${BASH_SOURCE[0]}")/../Cargo.toml" >&2
registry="${CARGO_HOME:-$HOME/.cargo}/registry/src"

stub_library() {
  local crate="$1" library="$2" source
  source="$(ls -d "$registry"/*/"$crate"-*/src/lib.rs | sort -V | tail -1)"
  {
    sed -nE 's/^[[:space:]]*pub fn ([A-Za-z0-9_]+)[[:space:]]*\(.*/void \1(void) {}/p' "$source" | sort -u
    sed -nE 's/^[[:space:]]*pub static ([A-Za-z0-9_]+):.*/char \1[64];/p' "$source" | sort -u
  } > "$stubs/$library.c"
  gcc -shared -fPIC -o "$stubs/lib$library.so" "$stubs/$library.c"
}

pc_file() {
  local name="$1" library="$2" version="$3"
  cat > "$stubs/$name.pc" <<PC
prefix=$stubs
libdir=$stubs
includedir=$stubs/include

Name: $name
Description: stand-in for a container without the native package
Version: $version
Libs: -L\${libdir} -l$library
Cflags: -I\${includedir}
PC
}

# Linked by the agent and the CLI: smart cards and the Linux Secret Service.
stub_library pcsc-sys pcsclite
stub_library libdbus-sys dbus-1
pc_file libpcsclite pcsclite 1.9.9
pc_file dbus-1 dbus-1 1.14.10

# Probed by the Tauri crate's -sys dependencies; enough for `cargo check` and
# `cargo clippy`, never for linking a window.
pc_file gdk-3.0 gdk-3 3.24.41
pc_file gdk-x11-3.0 gdk-3 3.24.41
pc_file gdk-wayland-3.0 gdk-3 3.24.41
pc_file gtk+-3.0 gtk-3 3.24.41
pc_file atk atk-1.0 2.52.0
pc_file pango pango-1.0 1.52.1
pc_file pangocairo pangocairo-1.0 1.52.1
pc_file gdk-pixbuf-2.0 gdk_pixbuf-2.0 2.42.10
pc_file webkit2gtk-4.1 webkit2gtk-4.1 2.44.0
pc_file javascriptcoregtk-4.1 javascriptcoregtk-4.1 2.44.0
pc_file libsoup-3.0 soup-3.0 3.4.4

echo "export PKG_CONFIG_PATH=$stubs\${PKG_CONFIG_PATH:+:\$PKG_CONFIG_PATH}"
echo "export LD_LIBRARY_PATH=$stubs\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
