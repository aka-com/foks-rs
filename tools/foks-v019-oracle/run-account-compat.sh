#!/bin/sh
set -eu
tool_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
export FOKS_RUST_LIVE_ACCOUNT=1
export FOKS_RUST_LIVE_DRIVER="$tool_dir/account-go-driver.sh"
export GOCACHE="${GOCACHE:-${TMPDIR:-/tmp}/foks-go-build-cache}"
exec go test -C "$tool_dir" -run '^TestRustClientHappyPath$' -count=1 -timeout=10m -v
