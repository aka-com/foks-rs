#!/bin/sh
set -eu

repository_root=$(git rev-parse --show-toplevel)
cd "$repository_root"

base_ref=${FOKS_BASE_REF:-origin/main}
if ! git rev-parse --verify "$base_ref" >/dev/null 2>&1; then
    echo "FOKS boundary base ref does not exist: $base_ref" >&2
    exit 1
fi

changed_paths=$(mktemp)
package_names=$(mktemp)
trap 'rm -f "$changed_paths" "$package_names"' EXIT HUP INT TERM

git diff --name-only "$base_ref"...HEAD -- >"$changed_paths"
git diff --name-only HEAD -- >>"$changed_paths"
git diff --cached --name-only -- >>"$changed_paths"
git ls-files --others --exclude-standard >>"$changed_paths"
sort -u -o "$changed_paths" "$changed_paths"

while IFS= read -r path; do
    case "$path" in
        ""|crates/foks-*|tools/foks-v019-oracle/*|tools/foks-protocol-sync/*|tools/foks-server/*|.github/workflows/foks-protocol-drift.yml|Cargo.toml|Cargo.lock|rust-project.json|MODULE.bazel|MODULE.bazel.lock|.gitattributes)
            ;;
        *)
            echo "path escapes standalone FOKS boundary: $path" >&2
            exit 1
            ;;
    esac
done <"$changed_paths"

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required for the isolated Cargo dependency check" >&2
    exit 1
fi

cargo metadata --offline --locked --no-deps --format-version 1 \
    | jq -r '.packages[].name | select(startswith("foks-"))' \
    | sort -u >"$package_names"

if [ ! -s "$package_names" ]; then
    echo "no FOKS workspace packages found" >&2
    exit 1
fi

while IFS= read -r package; do
    if cargo tree --offline --locked --prefix none -p "$package" | sed 's/ .*//' | grep '^aka-' >/dev/null; then
        echo "$package has an AKA dependency" >&2
        exit 1
    fi
    if [ "$package" != "foks-server-testkit" ] \
        && cargo tree --offline --locked --prefix none -p "$package" \
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
