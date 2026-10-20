#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
cd "$repository_root"

packages=(
    foks-go-interop
    foks-hosted-transport
    foks-merkle-store
    foks-metadata-core
    foks-protocol-metadata
    foks-server-db
    foks-server
    foks-server-testkit
    foks-yubi
)
package_args=()
for package in "${packages[@]}"; do
    package_args+=("-p" "$package")
done

# Workspace package placement, path dependencies, crate layering and the
# foks-server-testkit production-dependency guard (tools/crate-layers.toml).
python3 scripts/check-crate-layers.py

cargo fmt "${package_args[@]}" -- --check
cargo clippy --offline --locked "${package_args[@]}" --all-targets -- -D warnings
cargo test --offline --locked "${package_args[@]}"

# The hosted metadata kernel runs as a wasm32 cdylib; build it as celld loads
# it, without and with its qualification features.
cargo build --offline --locked --target wasm32-unknown-unknown -p foks-metadata-core
cargo build --offline --locked --target wasm32-unknown-unknown -p foks-metadata-core \
    --features read-token-qualification,list-allocation-qualification
