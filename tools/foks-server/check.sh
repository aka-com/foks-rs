#!/bin/sh
set -eu

repository_root=$(git rev-parse --show-toplevel)
cd "$repository_root"

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required for the isolated Cargo dependency check" >&2
    exit 1
fi

metadata=$(mktemp)
package_names=$(mktemp)
boundary_paths=$(mktemp)
trap 'rm -f "$metadata" "$package_names" "$boundary_paths"' EXIT HUP INT TERM

cargo metadata --offline --locked --no-deps --format-version 1 >"$metadata"

jq -r '.packages[].name | select(startswith("foks-"))' "$metadata" \
    | sort -u >"$package_names"

if [ ! -s "$package_names" ]; then
    echo "no FOKS workspace packages found" >&2
    exit 1
fi

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

# MCP process tests use the packaged sibling agent executable.
cargo build --offline --locked -p foks-agent

# The desktop package's Tauri build validates its configured sidecar path even
# for checks and tests. Reuse the debug agent above instead of building the
# release binary used for bundles.
host_target=$(rustc -vV | sed -n 's/^host: //p')
if [ -z "$host_target" ]; then
    echo "could not determine the Rust host target" >&2
    exit 1
fi
mkdir -p apps/desktop/src-tauri/binaries
cp target/debug/foks-agent "apps/desktop/src-tauri/binaries/foks-agent-$host_target"

while IFS= read -r package; do
    if [ "$package" != "foks-server-testkit" ] \
        && cargo tree --offline --locked --edges normal --prefix none -p "$package" \
            | sed 's/ .*//' | grep '^foks-server-testkit$' >/dev/null; then
        echo "$package has a production dependency on foks-server-testkit" >&2
        exit 1
    fi
done <"$package_names"

while IFS= read -r package; do
    cargo fmt --check --package "$package"
done <"$package_names"

while IFS= read -r package; do
    cargo clippy --offline --locked --package "$package" --all-targets -- -D warnings
done <"$package_names"

while IFS= read -r package; do
    cargo test --offline --locked --package "$package"
done <"$package_names"
