#!/bin/sh
set -eu
tool_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
export FOKS_RUST_LIVE_DRIVER="$tool_dir/sso-go-driver.sh"
export FOKS_RUST_LIVE_SSO=1
export GOCACHE="${GOCACHE:-${TMPDIR:-/tmp}/foks-go-build-cache}"
exec go test -C "$tool_dir" -run '^TestRustClientHappyPath$' -count=1 -timeout=10m -v
