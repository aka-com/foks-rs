#!/bin/sh
set -eu
tool_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repository=$(CDPATH= cd -- "$tool_dir/../.." && pwd -P)
export GOCACHE="${TMPDIR:-/tmp}/foks-go-build-cache"
export FOKS_GO_MCP_ORACLE="$tool_dir"
mode=${1:-all}
case "$mode" in all|rust|go) ;; *) echo 'usage: run-mcp-compat.sh [all|rust|go]' >&2; exit 2;; esac
cargo build --manifest-path "$repository/Cargo.toml" --locked -p foks-agent
if [ "$mode" != go ]; then
    cargo test --manifest-path "$repository/Cargo.toml" --locked -p foks-cli --test mcp_stdio -- --nocapture
fi
if [ "$mode" != rust ]; then
    FOKS_RUST_LIVE_DRIVER="$tool_dir/mcp-go-driver.sh" \
        go test -C "$tool_dir" -run '^TestRustClientHappyPath$' -count=1 -timeout=10m -v
fi
