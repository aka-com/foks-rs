#!/usr/bin/env bash
set -euo pipefail

packages=(
  foks-keystore
  foks-client-db
  foks-crypto
  foks-proto
  foks-rpc
  foks-snowpack
  foks-verify
  foks-local-deadline
  foks-schema-contract
  foks-client
  foks-oidc
  foks-client-app
  foks-compat-artifact
  foks-cli
  foks-mcp
  foks-agent-proto
  foks-agent-client
  foks-agent
  foks-desktop
)

package_args=()
parallel_test_args=()
for package in "${packages[@]}"; do
  package_args+=("-p" "$package")
  if [[ "$package" != foks-client-app ]]; then
    parallel_test_args+=("-p" "$package")
  fi
done

repository_root=$(git rev-parse --show-toplevel)
cd "$repository_root"

# Workspace package placement, path dependencies, crate layering and the
# foks-server-testkit production-dependency guard (tools/crate-layers.toml).
python3 scripts/check-crate-layers.py

cargo fmt "${package_args[@]}" -- --check
cargo build --offline --locked -p foks-agent
cargo test --offline --locked "${parallel_test_args[@]}"
# Native credential state in the application tests must be accessed serially.
cargo test --offline --locked -p foks-client-app -- --test-threads=1
cargo clippy --offline --locked "${package_args[@]}" --all-targets -- -D warnings
