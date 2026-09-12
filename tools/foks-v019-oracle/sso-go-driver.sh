#!/bin/sh
set -eu
tool_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repository=$(CDPATH= cd -- "$tool_dir/../.." && pwd -P)
while [ "$#" -gt 0 ]; do
    case "$1" in
        --probe) export FOKS_TEST_SSO_PROBE="$2"; shift 2;;
        --ca-der) export FOKS_TEST_SSO_CA="$2"; shift 2;;
        --state-dir) export FOKS_TEST_SSO_STATE="$2"; shift 2;;
        --username) shift 2;;
        *) echo 'invalid SSO driver argument' >&2; exit 2;;
    esac
done
exec cargo test --manifest-path "$repository/Cargo.toml" --locked -p foks-client --test sso_live -- --nocapture
