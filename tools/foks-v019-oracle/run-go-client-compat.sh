#!/bin/sh
set -eu

tool_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repository=$(CDPATH= cd -- "$tool_dir/../.." && pwd -P)

GOCACHE="${TMPDIR:-/tmp}/foks-go-build-cache" \
FOKS_GO_ORACLE_DIR="$tool_dir" \
    cargo test --manifest-path "$repository/Cargo.toml" --offline --locked \
        -p foks-server-testkit --test go_client_live -- --nocapture
