#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
cd "$repository_root"

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required for the isolated Cargo dependency check" >&2
    exit 1
fi

packages=(
    foks-go-interop
    foks-merkle-store
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

metadata=$(mktemp)
boundary_paths=$(mktemp)
trap 'rm -f "$metadata" "$boundary_paths"' EXIT HUP INT TERM

cargo metadata --offline --locked --no-deps --format-version 1 >"$metadata"

# The standalone boundary is a property of the Cargo graph, not of a branch's
# changed files. This repository builds two products out of one workspace, so
# a branch that carries FOKS work can legitimately carry other workspace code;
# a whole-branch changed-path allowlist rejected that without saying
# anything about whether FOKS still stands alone. What has to hold is that
# every FOKS package is *defined* inside FOKS-owned directories and reaches
# only FOKS-owned directories for its local sources.
#
# `crates/foks-*` holds the libraries and binaries; `apps/desktop/src-tauri` holds the
# desktop shell package (`foks-desktop-app`). Nothing else may define or be
# reached by a FOKS package.
jq -r --arg root "$repository_root/" '
    .packages[]
    | select(.name | startswith("foks-"))
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

tab=$(printf '\t')
while IFS="$tab" read -r package relation path; do
    case "$path" in
        crates/foks-*/Cargo.toml|apps/desktop/src-tauri/Cargo.toml|crates/foks-*) ;;
        *)
            if [ "$relation" = manifest ]; then
                echo "$package is defined outside the standalone FOKS boundary: $path" >&2
            else
                echo "$package depends on $relation outside the standalone FOKS boundary: $path" >&2
            fi
            exit 1
            ;;
    esac
done <"$boundary_paths"

# Keep the production dependency guard repository-wide as well.
while IFS= read -r package; do
    if [ "$package" != "foks-server-testkit" ] \
        && cargo tree --offline --locked --edges normal --prefix none -p "$package" \
            | sed 's/ .*//' | grep '^foks-server-testkit$' >/dev/null; then
        echo "$package has a production dependency on foks-server-testkit" >&2
        exit 1
    fi
done < <(jq -r '.packages[].name | select(startswith("foks-"))' "$metadata")

cargo fmt "${package_args[@]}" -- --check
cargo clippy --offline --locked "${package_args[@]}" --all-targets -- -D warnings
cargo test --offline --locked "${package_args[@]}"
