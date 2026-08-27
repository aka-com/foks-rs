#!/bin/sh
set -eu

count=${1:-5}
case "$count" in
    *[!0-9]*|'') echo "repeat count must be a positive integer" >&2; exit 2 ;;
esac
if [ "$count" -lt 1 ]; then
    echo "repeat count must be a positive integer" >&2
    exit 2
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd -P)
cd "$repo_root"
cargo build --offline --locked --release -p foks-server --bin foks-server
binary="$repo_root/target/release/foks-server"

index=1
while [ "$index" -le "$count" ]; do
    echo "FOKS small-team repeat $index/$count (seed=$index)" >&2
    FOKS_TEST_SEED="$index" FOKS_SERVER_BIN="$binary" "$script_dir/test-small-team.sh"
    index=$((index + 1))
done
