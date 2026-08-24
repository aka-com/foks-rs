#!/bin/sh
set -eu

tool_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository=$(CDPATH= cd -- "$tool_dir/../.." && pwd)

cargo build --manifest-path "$repository/Cargo.toml" -p foks-client --example live_compat
GOCACHE="${TMPDIR:-/tmp}/aka-foks-go-build-cache" \
FOKS_RUST_LIVE_DRIVER="$repository/target/debug/examples/live_compat" \
    go test -C "$tool_dir" -run '^TestRustClientHappyPath$' -count=1 -v
