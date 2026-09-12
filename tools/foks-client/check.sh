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
for package in "${packages[@]}"; do
  package_args+=("-p" "$package")
done

cargo fmt "${package_args[@]}" -- --check
cargo build --offline --locked -p foks-agent
cargo test --offline "${package_args[@]}"
cargo clippy --offline "${package_args[@]}" --all-targets -- -D warnings

if cargo tree --offline "${package_args[@]}" | rg -q '(^|[[:space:]])aka-[^[:space:]]+[[:space:]]'; then
  echo "standalone FOKS client graph unexpectedly contains an AKA crate" >&2
  exit 1
fi
