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

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for the standalone Cargo boundary check" >&2
  exit 1
fi

package_args=()
parallel_test_args=()
for package in "${packages[@]}"; do
  package_args+=("-p" "$package")
  if [[ "$package" != foks-client-app ]]; then
    parallel_test_args+=("-p" "$package")
  fi
done

repository_root=$(git rev-parse --show-toplevel)
metadata=$(mktemp)
boundary_paths=$(mktemp)
trap 'rm -f "$metadata" "$boundary_paths"' EXIT
cargo metadata --offline --locked --no-deps --format-version 1 >"$metadata"

jq -r --arg root "$repository_root/" --argjson selected "$(printf '%s\n' "${packages[@]}" | jq -R . | jq -s .)" '
  .packages[]
  | select(.name as $name | $selected | index($name))
  | . as $package
  | [
      "\($package.name)\tmanifest\t\($package.manifest_path | ltrimstr($root))",
      (
        $package.dependencies[]
        | select(.path != null)
        | "\($package.name)\t\(.name)\t\(.path | ltrimstr($root))"
      )
    ]
  | .[]
' "$metadata" >"$boundary_paths"

while IFS=$'\t' read -r package relation path; do
  case "$path" in
    crates/foks-*/Cargo.toml|apps/desktop/src-tauri/Cargo.toml|crates/foks-*) ;;
    *)
      echo "$package reaches $relation outside the standalone FOKS boundary: $path" >&2
      exit 1
      ;;
  esac
done <"$boundary_paths"

cargo fmt "${package_args[@]}" -- --check
cargo build --offline --locked -p foks-agent
cargo test --offline --locked "${parallel_test_args[@]}"
# Native credential state in the application tests must be accessed serially.
cargo test --offline --locked -p foks-client-app -- --test-threads=1
cargo clippy --offline --locked "${package_args[@]}" --all-targets -- -D warnings
