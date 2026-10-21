#!/bin/sh
set -eu

repository_root=$(git rev-parse --show-toplevel)
mode=check
offline=

while [ "$#" -gt 0 ]; do
    case "$1" in
        --check) mode=check ;;
        --write) mode=write ;;
        --offline) offline=--offline ;;
        *) echo "usage: $0 [--check|--write] [--offline]" >&2; exit 2 ;;
    esac
    shift
done

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/foks-protocol-generate.XXXXXX")
trap 'rm -rf "$temporary_root"' EXIT HUP INT TERM
protocol_go_cache=${FOKS_PROTOCOL_GOCACHE:-"$temporary_root/go-build"}

extract() {
    destination=$1
    structs=$2
    (
        cd "$repository_root/tools/foks-protocol-sync"
        GOCACHE="$protocol_go_cache" go run . pinned \
            --oracle "$repository_root/tools/foks-v019-oracle" \
            ${offline:+"$offline"} \
            --out "$destination" \
            --structs-out "$structs"
    )
}

first="$temporary_root/first.json"
second="$temporary_root/second.json"
first_structs="$temporary_root/first-structs.json"
second_structs="$temporary_root/second-structs.json"
extract "$first" "$first_structs"
extract "$second" "$second_structs"
cmp "$first" "$second"
cmp "$first_structs" "$second_structs"

upstream="$repository_root/crates/foks-server/protocol/upstream-v0.1.9.json"
policy="$repository_root/crates/foks-server/protocol/policy-v1.toml"
protocol_ids="$repository_root/crates/foks-rpc/src/generated/protocol_ids.rs"
status_codes="$repository_root/crates/foks-rpc/src/generated/status_codes.rs"
routes="$repository_root/crates/foks-server/src/rpc/generated/routes.rs"
contract="$repository_root/crates/foks-server/protocol-v1.toml"
digest="$repository_root/crates/foks-server/protocol/upstream-v0.1.9.sha256"
# Positional struct layouts for the drift report. Kept apart from the pinned
# artifact so that its bytes and digest do not depend on them.
structs="$repository_root/crates/foks-server/protocol/upstream-v0.1.9-structs.json"

if [ "$mode" = write ]; then
    cp "$first" "$upstream"
    cp "$first_structs" "$structs"
else
    cmp "$first" "$upstream"
    cmp "$first_structs" "$structs"
fi

cargo run --quiet --manifest-path "$repository_root/Cargo.toml" \
    --package foks-protocol-metadata -- "$mode" \
    --upstream "$upstream" \
    --policy "$policy" \
    --protocol-ids "$protocol_ids" \
    --status-codes "$status_codes" \
    --routes "$routes" \
    --contract "$contract" \
    --digest "$digest"
