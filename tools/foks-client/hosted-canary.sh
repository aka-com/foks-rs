#!/bin/sh
set -eu

: "${FOKS_CANARY_STATE_DIR:?set FOKS_CANARY_STATE_DIR}"
: "${FOKS_CANARY_PROFILE:?set FOKS_CANARY_PROFILE}"
: "${FOKS_CAPABILITY_PROFILE:?set FOKS_CAPABILITY_PROFILE}"
: "${FOKS_CANARY_ACCOUNT:?set FOKS_CANARY_ACCOUNT}"
: "${FOKS_CANARY_SIGNING_KEY_FILE:?set FOKS_CANARY_SIGNING_KEY_FILE}"
: "${FOKS_CANARY_OUTPUT:?set FOKS_CANARY_OUTPUT}"

root=$(git rev-parse --show-toplevel)
client="$root/target/release/foks-rs"
signer="$root/target/release/foks-compat-artifact"
metadata="$root/crates/foks-server/protocol/upstream-v0.1.9.json"
run_id=${GITHUB_RUN_ID:-manual}-$(date -u +%s)
generated_at=$(date -u +%s)
expires_at=$((generated_at + 172800))
temporary=$(mktemp -d)
input="$temporary/input"
output="$temporary/output"
path="/compat-canary-$run_id"
reason=
outcome=compatible
zero_digest=0000000000000000000000000000000000000000000000000000000000000000
mutation_digest=$zero_digest
read_digest=$zero_digest
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

umask 077
printf 'foks-hosted-canary:%s\n' "$run_id" >"$input"

set +e
"$client" --state-dir "$FOKS_CANARY_STATE_DIR" profile probe "$FOKS_CANARY_PROFILE" \
    && "$client" --state-dir "$FOKS_CANARY_STATE_DIR" account sync \
        "$FOKS_CANARY_PROFILE" "$FOKS_CANARY_ACCOUNT" \
    && "$client" --state-dir "$FOKS_CANARY_STATE_DIR" kv put \
        "$FOKS_CANARY_PROFILE" "$FOKS_CANARY_ACCOUNT" "$path" --input "$input" \
    && "$client" --state-dir "$FOKS_CANARY_STATE_DIR" kv get \
        "$FOKS_CANARY_PROFILE" "$FOKS_CANARY_ACCOUNT" "$path" --output "$output" \
    && cmp "$input" "$output"
status=$?
set -e

if [ "$status" -eq 0 ]; then
    mutation_digest=$(shasum -a 256 "$input" | awk '{print $1}')
    read_digest=$(shasum -a 256 "$output" | awk '{print $1}')
else
    outcome=drift
    reason="authenticated mutation/read canary failed"
fi

# A successful write that cannot be removed is drift and would otherwise leak
# one object per scheduled run. Cleanup after an earlier failure stays
# best-effort because the object might never have been created.
if ! "$client" --state-dir "$FOKS_CANARY_STATE_DIR" kv remove \
    "$FOKS_CANARY_PROFILE" "$FOKS_CANARY_ACCOUNT" "$path" >/dev/null 2>&1; then
    if [ "$outcome" = compatible ]; then
        outcome=drift
        reason="authenticated canary cleanup failed"
        mutation_digest=$zero_digest
        read_digest=$zero_digest
    fi
fi

protocol_digest=$(shasum -a 256 "$metadata" | awk '{print $1}')
set -- sign \
    --target "${FOKS_CANARY_TARGET:-foks.pub:443}" \
    --run-id "$run_id" \
    --generated-at "$generated_at" \
    --expires-at "$expires_at" \
    --protocol-digest "$protocol_digest" \
    --mutation-digest "$mutation_digest" \
    --read-digest "$read_digest" \
    --outcome "$outcome" \
    --signing-key-file "$FOKS_CANARY_SIGNING_KEY_FILE" \
    --output "$FOKS_CANARY_OUTPUT"
if [ "$outcome" = compatible ]; then
    set -- "$@" --capability user-sync --capability kv
else
    set -- "$@" --drift-reason "$reason"
fi
"$signer" "$@"

# Applying drift replaces any prior lease with probe-only policy. Successful
# leases are short-lived, so a stopped scheduler also revokes automatically.
"$client" --state-dir "$FOKS_CANARY_STATE_DIR" profile apply-canary \
    "$FOKS_CAPABILITY_PROFILE" --artifact "$FOKS_CANARY_OUTPUT"

[ "$outcome" = compatible ]
