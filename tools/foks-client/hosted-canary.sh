#!/bin/sh
set -eu

: "${FOKS_CANARY_STATE_DIR:?set FOKS_CANARY_STATE_DIR}"
: "${FOKS_CANARY_PROFILE:?set FOKS_CANARY_PROFILE}"
: "${FOKS_CAPABILITY_PROFILE:?set FOKS_CAPABILITY_PROFILE}"
: "${FOKS_CANARY_ACCOUNT:?set FOKS_CANARY_ACCOUNT}"
: "${FOKS_CANARY_SIGNING_KEY_FILE:?set FOKS_CANARY_SIGNING_KEY_FILE}"
: "${FOKS_CANARY_OUTPUT:?set FOKS_CANARY_OUTPUT}"
: "${FOKS_CANARY_RUN_NUMBER:?set FOKS_CANARY_RUN_NUMBER}"
: "${FOKS_CANARY_RUN_ATTEMPT:?set FOKS_CANARY_RUN_ATTEMPT}"

root=${FOKS_CANARY_ROOT:-$(git rev-parse --show-toplevel)}
client="$root/target/release/foks-rs"
signer="$root/target/release/foks-compat-artifact"
metadata="$root/crates/foks-server/protocol/upstream-v0.1.9.json"
run_id=${GITHUB_RUN_ID:-manual}-$(date -u +%s)
generated_at=$(date -u +%s)
expires_at=$((generated_at + 172800))
if [ -e "$FOKS_CANARY_OUTPUT" ] || [ -L "$FOKS_CANARY_OUTPUT" ]; then
    echo "canary output already exists" >&2
    exit 1
fi
temporary=$(mktemp -d)
input="$temporary/input"
output="$temporary/output"
candidate="$temporary/signed-artifact"
path="/compat-canary-$run_id"
reason=
outcome=compatible
zero_digest=0000000000000000000000000000000000000000000000000000000000000000
mutation_digest=$zero_digest
read_digest=$zero_digest
trap 'rm -rf "$temporary"' EXIT HUP INT TERM

umask 077
printf 'foks-hosted-canary:%s\n' "$run_id" >"$input"

profile_target() {
    "$client" --state-dir "$FOKS_CANARY_STATE_DIR" --json profile show "$1" \
        | jq -er '.probe | select(type == "string" and length > 0)'
}

target=$(profile_target "$FOKS_CANARY_PROFILE")
capability_target=$(profile_target "$FOKS_CAPABILITY_PROFILE")
if [ "$target" != "$capability_target" ]; then
    echo "canary and capability profiles target different services" >&2
    exit 1
fi
if [ -n "${FOKS_CANARY_EXPECTED_TARGET:-}" ] \
    && [ "$target" != "$FOKS_CANARY_EXPECTED_TARGET" ]; then
    echo "stored canary target does not match the configured expectation" >&2
    exit 1
fi

set -- allocate-generation \
    --target "$target" \
    --run-number "$FOKS_CANARY_RUN_NUMBER" \
    --run-attempt "$FOKS_CANARY_RUN_ATTEMPT" \
    --signing-key-file "$FOKS_CANARY_SIGNING_KEY_FILE"
if [ -n "${FOKS_CANARY_PREVIOUS_ARTIFACT:-}" ]; then
    set -- "$@" --previous-artifact "$FOKS_CANARY_PREVIOUS_ARTIFACT"
fi
generation=$("$signer" "$@")

set +e
"$client" --state-dir "$FOKS_CANARY_STATE_DIR" --json profile probe "$FOKS_CANARY_PROFILE" >"$temporary/probe" \
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

# Every additional grant is backed by the complete onboarding lifecycle. Missing
# operator authorization or an unsupported server requirement revokes the lease.
if [ "$outcome" = compatible ]; then
    if python3 "$root/tools/foks-client/onboarding-canary.py" \
        --client "$client" --target "$target" \
        --expected-host "$(jq -er '.host_id_hex | select(type == "string" and length == 66)' "$temporary/probe")" \
        >"$temporary/capabilities"; then
        # Bind the signed evidence to the exercised capability matrix as well as KV.
        mutation_digest=$(cat "$input" "$temporary/capabilities" | shasum -a 256 | awk '{print $1}')
        read_digest=$(cat "$output" "$temporary/capabilities" | shasum -a 256 | awk '{print $1}')
    else
        outcome=drift
        reason="onboarding lifecycle or its operational prerequisites failed"
        mutation_digest=$zero_digest
        read_digest=$zero_digest
    fi
fi

protocol_digest=$(shasum -a 256 "$metadata" | awk '{print $1}')
set -- sign \
    --generation "$generation" \
    --target "$target" \
    --run-id "$run_id" \
    --generated-at "$generated_at" \
    --expires-at "$expires_at" \
    --protocol-digest "$protocol_digest" \
    --mutation-digest "$mutation_digest" \
    --read-digest "$read_digest" \
    --outcome "$outcome" \
    --signing-key-file "$FOKS_CANARY_SIGNING_KEY_FILE" \
    --output "$candidate"
if [ "$outcome" = compatible ]; then
    set -- "$@" --capability user-sync --capability kv
    while IFS= read -r capability; do
        case "$capability" in
            signup|device-administration|recovery|passphrases) set -- "$@" --capability "$capability" ;;
            *) echo "unexpected onboarding capability" >&2; exit 1 ;;
        esac
    done <"$temporary/capabilities"
else
    set -- "$@" --drift-reason "$reason"
fi
"$signer" "$@"

# Applying drift replaces any prior lease with probe-only policy. Successful
# leases are short-lived, so a stopped scheduler also revokes automatically.
"$client" --state-dir "$FOKS_CANARY_STATE_DIR" profile apply-canary \
    "$FOKS_CAPABILITY_PROFILE" --artifact "$candidate"

# The stable output becomes visible only after both signing and local
# reauthorization have succeeded. A failed apply leaves no stale candidate for
# a later workflow step to publish.
if [ -e "$FOKS_CANARY_OUTPUT" ] || [ -L "$FOKS_CANARY_OUTPUT" ]; then
    echo "canary output appeared during execution" >&2
    exit 1
fi
mv "$candidate" "$FOKS_CANARY_OUTPUT"

[ "$outcome" = compatible ]
