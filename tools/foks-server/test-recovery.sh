#!/bin/sh
# Runs the crash-recovery and stale-backup experiments of
# crates/foks-server-testkit/RECOVERY.md against a release foks-server, or
# the binary FOKS_SERVER_BIN names. The tests run serially so that the
# recovery budget (FOKS_RECOVERY_BUDGET_MS, default 15000) measures one
# workload at a time.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$repo_root"
if [ -z "${FOKS_SERVER_BIN:-}" ]; then
    cargo build --offline --locked --release -p foks-server --bin foks-server
    binary="${CARGO_TARGET_DIR:-$repo_root/target}/release/foks-server"
else
    binary=$FOKS_SERVER_BIN
fi
FOKS_SERVER_BIN="$binary" cargo test --offline --locked \
    -p foks-server-testkit --test recovery_harness -- --ignored --test-threads=1
