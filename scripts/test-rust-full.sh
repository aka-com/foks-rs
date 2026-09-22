#!/usr/bin/env bash
set -euo pipefail

# Keep the standalone executable used by CLI integration tests current.
# Arguments are Cargo build options, for example --release.
cargo build --locked -j 2 -p foks-agent "$@"
cargo test --locked -j 2 --workspace --exclude foks-desktop-app "$@"
# Native account-operation-record lock tests share process resources and must run serially.
cargo test --locked -j 2 -p foks-desktop-app "$@" -- --test-threads=1
bash scripts/test-rust-scale.sh "$@"
