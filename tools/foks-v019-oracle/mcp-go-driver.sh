#!/bin/sh
# Driver contract used by the existing unmodified-Go service harness.
set -eu
tool_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repository=$(CDPATH= cd -- "$tool_dir/../.." && pwd -P)
while [ "$#" -gt 0 ]; do
    case "$1" in
        --probe) export FOKS_TEST_MCP_PROBE="$2"; shift 2;;
        --ca-der) export FOKS_TEST_MCP_CA="$2"; shift 2;;
        --state-dir|--username) shift 2;;
        *) echo 'invalid MCP compatibility driver argument' >&2; exit 2;;
    esac
done
: "${FOKS_TEST_MCP_PROBE:?probe is required}"
: "${FOKS_TEST_MCP_CA:?CA is required}"
export FOKS_GO_MCP_ORACLE="$tool_dir"
exec cargo test --manifest-path "$repository/Cargo.toml" --locked -p foks-cli --test mcp_stdio -- --nocapture
