#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$repo_root"
if [ -z "${FOKS_SERVER_BIN:-}" ]; then
    cargo build --release -p foks-server --bin foks-server
    binary="$repo_root/target/release/foks-server"
else
    binary=$FOKS_SERVER_BIN
fi
FOKS_SERVER_BIN="$binary" cargo test -p foks-server-testkit --test binary_release --offline -- --nocapture
