#!/bin/sh
set -eu
tool_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repository=$(CDPATH= cd -- "$tool_dir/../.." && pwd -P)
while [ "$#" -gt 0 ]; do
    case "$1" in
        --probe) export FOKS_TEST_INVITATION_PROBE="$2"; shift 2;;
        --ca-der) export FOKS_TEST_INVITATION_CA="$2"; shift 2;;
        --state-dir) export FOKS_TEST_INVITATION_STATE="$2"; shift 2;;
        --username) export FOKS_TEST_INVITATION_USERNAME="$2"; shift 2;;
        *) echo 'invalid INVITATION driver argument' >&2; exit 2;;
    esac
done
exec cargo test --manifest-path "$repository/Cargo.toml" --locked -p foks-server-testkit --test invitations_live -- --nocapture
