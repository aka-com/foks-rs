#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
cd "$repo_root"

cargo test --offline --locked -p foks-server-testkit --test conformance team_
cargo test --offline --locked -p foks-server-testkit \
    --test team_restart_recovery --test team_concurrency \
    --test team_capacity --test backup_restore

if [ -z "${FOKS_SERVER_BIN:-}" ]; then
    cargo build --offline --locked --release -p foks-server --bin foks-server
    binary="$repo_root/target/release/foks-server"
else
    binary=$FOKS_SERVER_BIN
fi
FOKS_SERVER_BIN="$binary" cargo test --offline --locked \
    -p foks-server-testkit --test binary_release -- --nocapture
