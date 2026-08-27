#!/bin/sh
set -eu

repository_root=$(git rev-parse --show-toplevel)
remote=${FOKS_UPSTREAM_REMOTE:-https://github.com/foks-proj/go-foks.git}
revision=
output_directory=${FOKS_PROTOCOL_DRIFT_OUT:-"$repository_root/target/foks-protocol-drift"}
fail_flag=

while [ "$#" -gt 0 ]; do
    case "$1" in
        --revision) revision=$2; shift ;;
        --out-dir) output_directory=$2; shift ;;
        --fail-on-review) fail_flag=--fail-on-review ;;
        *) echo "usage: $0 [--revision SHA] [--out-dir PATH] [--fail-on-review]" >&2; exit 2 ;;
    esac
    shift
done

case "$output_directory" in
    /*) ;;
    *) output_directory="$repository_root/$output_directory" ;;
esac

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/foks-protocol-drift.XXXXXX")
trap 'rm -rf "$temporary_root"' EXIT HUP INT TERM
protocol_go_cache=${FOKS_PROTOCOL_GOCACHE:-"$temporary_root/go-build"}

if [ -z "$revision" ]; then
    revision=$(git ls-remote --symref "$remote" HEAD | awk '$2 == "HEAD" && $1 != "ref:" { print $1; exit }')
fi
case "$revision" in
    ''|*[!0-9a-f]*) echo "could not resolve an immutable upstream commit: $revision" >&2; exit 1 ;;
esac
if [ "${#revision}" -ne 40 ]; then
    echo "upstream revision must be a full 40-character commit: $revision" >&2
    exit 1
fi

git init --quiet "$temporary_root/upstream"
git -C "$temporary_root/upstream" remote add origin "$remote"
git -C "$temporary_root/upstream" fetch --quiet --depth=1 origin "$revision"
git -C "$temporary_root/upstream" checkout --quiet --detach FETCH_HEAD

candidate="$temporary_root/candidate.json"
(
    cd "$repository_root/tools/foks-protocol-sync"
    GOCACHE="$protocol_go_cache" go run . extract \
        --module-dir "$temporary_root/upstream" \
        --version mainline \
        --commit "$revision" \
        --out "$candidate"
)

mkdir -p "$output_directory"
printf '%s\n' "$revision" >"$output_directory/resolved-commit.txt"
(
    cd "$repository_root/tools/foks-protocol-sync"
    GOCACHE="$protocol_go_cache" go run . diff \
        --baseline "$repository_root/crates/foks-server/protocol/upstream-v0.1.9.json" \
        --candidate "$candidate" \
        --policy "$repository_root/crates/foks-server/protocol/policy-v1.toml" \
        --json "$output_directory/mainline-diff.json" \
        --markdown "$output_directory/mainline-diff.md" \
        ${fail_flag:+"$fail_flag"}
)

echo "FOKS mainline protocol report: $output_directory/mainline-diff.md"
