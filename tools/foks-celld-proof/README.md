# Experimental FOKS SQL authority bridge

This is an opt-in local qualification bridge, not a production backend. The
`celld-proof` Cargo feature enables native TCP/mTLS dispatch against disposable
SQLite projections. The Worker owns actual FOKS SQL rows, native constraints and
accounting triggers. Native before/after comparison produces a bounded base-row delta;
authoritative revision/owner checks, delta application, resulting-state check and
receipt insertion share one cell transaction. No mutable whole-database blob is
used as the authority.

Only `kv_nodes`, `kv_dirents`, `kv_dirent_heads` and `kv_locks` are writable base
tables. Namespace counters are trigger outputs and must match the native result.
Every other table is frozen. A schema change requires regenerating and reviewing
the deployment-pinned schema; initialization accepts no request-provided SQL.
Native exports/materialization additionally compare the entire schema to a fresh
database initialized by the current native code. Equivalent legacy DDL is not
implicitly supported by this deliberately strict proof.

Limits: 8 MiB encoded messages/snapshots, 10,000 total snapshot rows, 128 tables,
50 columns/table, 512 schema objects, 10,000 retained attempt receipts and 512 KiB
encrypted key bundle. Exhaustion fails closed. There is no receipt expiry. The
projection does full-history copying; its bandwidth, CPU and reconstruction cost
must be measured before any density inference.

Integer values travel as canonical signed decimal strings and use SQL casts;
cell reads never round them through JavaScript Number. Text and blobs have
distinct base64 tags. Canonical hashes sort object keys, preserve array order and
hash UTF-8 JSON. Snapshot rows are sorted by their ASCII JSON representation.

After direct and independent review, generate actual native-schema vectors in a
new directory and run the focused tests (Node with `node:sqlite` is required):

```sh
cargo test --locked -j 2 -p foks-server-testkit --features celld-proof --lib celld_proof::tests
cargo run --locked -j 2 -p foks-server-testkit --features celld-proof \
  --example celld-proof-vectors -- /tmp/foks-proof-vectors-new
RUSTFLAGS="-C link-arg=--max-memory=67108864" cargo build --locked -j 2 \
  -p foks-metadata-core --target wasm32-unknown-unknown
FOKS_PROOF_VECTORS=/tmp/foks-proof-vectors-new/vectors.json \
FOKS_PROOF_NODE_DECODER_WASM=target/wasm32-unknown-unknown/debug/foks_metadata_core.wasm \
  node --test tools/foks-celld-proof/authority.test.mjs tools/foks-celld-proof/node-decoder.test.mjs
```

These tests use full FOKS schema/constraints but synthetic SQL identity rows,
which are not cryptographically valid FOKS accounts. They qualify typed transfer,
multi-table publication, FK/trigger equivalence, rollback, replay and owner checks.
They do not establish actual Go-client compatibility or celld durability.

For an actual-cell run, copy the reviewed `generated-schema.json` beside
`worker.mjs`, use `nodejs_compat`, bind `FOKS_PROOF` to SQLite class `FoksProof`, and
provide distinct random private `PROOF_TOKEN` and `PROOF_CONTROL_TOKEN` values of
at least 32 characters. Only the controller receives the latter: native adapters
cannot call initialize or claim another adapter epoch. The bridge accepts
authenticated POSTs to `/initialize`, `/claim`, `/snapshot`, `/apply` and `/prove`,
with a bounded `tenant` query parameter. It is loopback/private controller access,
not public FOKS ingress. Do not write the token into committed config or logs.

`/apply` retry is the same epoch/owner/attempt and canonical request. A matching
receipt returns its prior result without charging again; a stale epoch or any
payload change rejects. `/prove` checks current owner/revision/state for a held
read/error response. The caller must retain its operation guard and response
until one of these gated replies succeeds. Application epochs fence native
adapters; celld's platform epochs independently fence cell owners.

Initialization imports rows before installing native triggers, checks foreign
keys and exact reconstructed state, then atomically records the installation and
encrypted key bundle. It is only repeatable for the identical initialization
manifest. The separate wrapping/root secret remains outside the cell. The native fixture restores and decrypts the encrypted keys into a private
directory and verifies every immutable manifest generation before starting mTLS.
It keeps the operator root secret, public probe TLS fixture and routing addresses
outside the cell; these must be counted as surviving controller state.


The native bridge serializes one operation per adapter, reconstructs the bounded
full database, drains its writer, applies an exact base-row delta and obtains an
ownership/revision/state proof before returning the candidate RPC response. This
proof is the observation's linearization point. It does not atomically fence the
later socket write: an adapter paused after proof may release an overlapping,
already-proved response after promotion. A stale adapter paused before apply or
proof cannot publish new SQL or release a newly obtained response. Every accepted
write remains in the authoritative history.

Only preprovisioned personal accounts, roots and small-file namespaces are in
scope. Certificates must already be provisioned and outside their renewal window.
Identity, host-key, membership, directory/root, SSO, background-job and large-file
mutations are unsupported. The allowlist and frozen-table comparison both enforce
this. No production startup or CLI path installs the adapter.

The native snapshot JSON, multiple typed copies and fresh SQLite projection are
additional bounded proof overhead, separate from ordinary request accounting.
Each RPC scans full history up to 8 MiB and 10,000 rows; current tests are tiny.
This bridge is intentionally unsuitable for thousands of production tenants.
The worker also caps its unpruned receipt history at 10,000 writes and fails
closed when full. The measurements must report snapshot traffic and full resident
native/celld processes, not only JavaScript heap usage.

After building the ignored integration-test executable and generating the schema,
`run-local.py` runs one explicitly selected test against an isolated celld 0.6.2
and Moto installation. `--durability bucket` is the default; `--durability fleet`
is an explicit sensitivity run and is recorded in the controller configuration.
`--lease-ttl-ms 10000` preserves celld's default lease; separately named
`--lease-ttl-ms 3000` trials measure a shorter lease with the same authority and
15-second checks. Renewal runs at TTL/3, so shortening TTL increases renewal
traffic and sensitivity to delayed scheduling/storage. It is not an SLO guarantee.
It requires the existing binary paths and a NEW output
directory; it never builds software or reaches AWS. Example (replace the test
binary hash with Cargo's actual output):

```sh
cargo test --locked -j 2 -p foks-server-testkit --features celld-proof \
  --test celld_proof --no-run
/private/tmp/foks-celld-lab/venv/bin/python tools/foks-celld-proof/run-local.py \
  --celld /private/tmp/foks-celld-lab/bin/celld \
  --moto /private/tmp/foks-celld-lab/venv/bin/moto_server \
  --schema /tmp/foks-proof-vectors-new/generated-schema.json \
  --node-decoder-wasm target/wasm32-unknown-unknown/debug/foks_metadata_core.wasm \
  --esbuild /opt/homebrew/bin/esbuild \
  --test-binary target/debug/deps/celld_proof-ACTUAL_HASH \
  --output /tmp/foks-real-cell-new
```

Select `real_foks_cell_owner_process_loss_preserves_acknowledged_sql_writes` with
`--test-name` for owner SIGKILL, removal of its fixture-owned local artifacts and
rerouting to a surviving celld process. The controller starts two peers before
the acknowledged writes and retains their logs plus the in-memory emulator.
The 15-second ledger spans fault injection, authenticated reads of all prior
ACKs, a newly acknowledged write and its fresh-content read. All remain on one
physical host. Passing establishes this local process-loss experiment only,
not host/AZ loss, real S3 durability, WAN latency or a worst-case guarantee.
For this single-owner test, `--fault-mode pause-return` instead pauses the old
celld node, verifies successor recovery, resumes and probes the old ingress, then
requires its terminal lease self-fence (exit 3) and a further successor write/read.
A successful stale read fails; a fenced transport error is acceptable during the
old-ingress probe. The experiment does not attempt an ambiguous old-ingress write.

Select `real_foks_multiple_cells_capacity_baseline` for four real tenant cells
by default (`FOKS_CELLD_TENANTS=1..16`). Set `FOKS_CELLD_HISTORY_WRITES=3` or `30`
(allowed range3..100) to compare namespace growth from new live files, together
with transaction receipt history; this is not a retained-version benchmark. After all tenants finish
their acknowledged baseline writes, a bounded controller barrier starts every
15-second recovery ledger before one simultaneous owner-process loss. Every
tenant must verify all its ACKs, perform a new write and read it back within its
original deadline. Its JSON reports cold import, authenticated writes/fresh reads,
recovery and full-history transfer overhead; the controller samples native and
celld RSS. This is a bounded baseline, not a density extrapolation.
Per-round timing/bridge samples are synced before the recovery barrier so later
failures retain partial measurements. Ledger and sample fsync overhead is part
of the measured workload; the 180-second preparation bound is separate from the
15-second recovery requirement.

`FOKS_CELLD_IDLE_TENANTS=64` adds sequentially provisioned actual FOKS cells before
the active workload (maximum 128, population deadline 300 seconds). Their native
DB/key artifacts are explicitly removed after authenticated cell reads. The
controller records actual resident/owned/stopped counts with the configured
32-cell resident cap. First/last idle cells are revisited after active recovery,
even when that recovery failed, and their SQL/key-content hashes must match.
Aggregate residency does not prove that either selected revisit was evicted;
revisit timing includes a cold native adapter and a possibly resident celld cell.
The native test process cohosts adapters; its RSS is not a per-tenant process cost.

Client timeout is 5 seconds and bridge HTTP timeout 4 seconds. Native session
defaults still use a 30-second execution timeout and separate 15-second I/O
timeouts. These phase limits do not establish a 15-second end-to-end bound.

The controller records wall and monotonic clocks for every event. A latched
elapsed-clock discrepancy greater than one second invalidates timing qualification
and produces a nonzero exit, even if content assertions passed. Synthetic guard
tests run with `python3 tools/foks-celld-proof/clock_guard_test.py`; they never
change the host clock. `test_passed` and the native test log preserve content
evidence separately from final `qualification_complete.passed`.
On macOS, guarded trials may run under a temporary `caffeinate -i` assertion with
`FOKS_PROOF_IDLE_SLEEP_INHIBITOR=caffeinate-i` recorded in configuration. This
does not prevent manual/lid suspension or disable the clock guard.
Private credentials appear only in the mode-0600 generated deployment config and
child environment; keep the entire mode-0700 output directory private.

Schema58 replay additionally requires the immutable native-built decoder Wasm
artifact. Its pure export uses the shared Rust exact-box decoder; no JavaScript
Snowpack decoder or request-supplied byte count is trusted. Changed small/symlink
nodes are limited to65536 exact bytes and their supplied derived lengths must
match the Rust result before any SQL mutation. Replay writes NULL then stamps
the checked count under the exact row predicate, allowing native triggers to
produce all semantic Usage counters. Full-state equality still includes those
counters, and failed validation or a later SQL error publishes no receipt. The
controller records decoder/source hashes and validates the64MiB memory maximum.
This compatibility addition does not extend the proof's route or HA scope.
