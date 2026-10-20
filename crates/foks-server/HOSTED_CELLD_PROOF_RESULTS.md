# Local real-FOKS celld proof results

Local qualification evidence, 2026-10-15. Keep unstaged. Initial experiments
pin FOKS schema 52; the capacity matrix uses schema 53 atop
`57790f38`, before the later filesystem-store authority binding patch/schema 54,
with unmodified celld v0.6.2. These timing baselines exercise no external chunk
store or filesystem GC. The planned guarded capacity matrix
finished, including the failures below. Experiments ran on one developer Mac
against an in-memory Moto S3 emulator. Initial schema-52 experiments partly
overlapped development builds; the schema-53 runtime matrix kept Cargo quiet. They establish neither
physical host/AZ recovery nor managed-S3 latency, durability or availability.
Managed S3 durability, reliability and strong consistency are accepted design
assumptions. These experiments qualify our use of the storage boundary; they do
not attempt to reproduce the provider's replication qualification. Deployment
latency and the application's combined failure paths still need measurement.

## What actually ran

Real Rust and pinned Go FOKS clients connected through native TCP/mTLS. A private
bridge reconstructed disposable native SQLite projections, then published typed
base-row changes to actual constrained SQL tables inside a celld cell. Application
adapter epochs, expected revisions, transaction receipts, whole-state digests and
the cell output gate preceded RPC response release. Encrypted installation keys
were restored from the cell; controller root secrets, probe TLS and addresses
survived outside it. This is a bounded, expensive qualification bridge, not a
production backend or a Wasm port.
Native-adapter replacement and celld-owner loss were separate experiments. The
native bridge remains alive during the celld process fault. A co-placed native
frontend/cell host loss, including new ingress, key restoration and routing within
one recovery budget, remains unqualified.

The workload is pre-provisioned personal identities with small encrypted text
files. External file uploads, organization membership, key/certificate renewal,
production routing and arbitrary identities are outside the proof. Existing native
external-file tests establish separate storage behavior.
“History” in directory and parameter names means accumulated new live files and
receipt history, not retained overwritten versions. Each baseline round
authenticates and writes one new file, then authenticates and
freshly synchronizes the complete namespace. This exercises a demanding repeated
full-sync pattern; it is not a benchmark of one isolated SQL statement or a
long-lived authenticated connection doing only point reads.

The host reports Apple M5, 10 logical CPUs, 24 GiB RAM, arm64 macOS 26.5.1.
It is a shared developer machine; recording load and keeping Cargo quiet does not
make it a controlled production-capacity environment.

## Default bucket-durability observations

“Default” here means the 10-second lease and bucket durability. Laboratory tuning
also sets two Tokio threads, 32 resident cells, 512 MiB RSS/64 MiB V8 limits,
64 MiB local cache, readiness fleet gate zero and rebalancing disabled. It is not
an otherwise untouched production configuration; exact values are in each ledger.

Each recovery gate required all acknowledged values from fresh authenticated reads
plus a new acknowledged write/read within **15 seconds**. The gate was unchanged
on failures. Successful preservation of acknowledged data after the deadline is
not a successful availability result.

| Run | Outcome | Evidence directory under `/private/tmp/` |
| --- | --- | --- |
| Native adapter promotion, original native artifacts removed | Pass, 7.206 s; stale candidate fenced | `foks-real-cell-schema52-fencing-b` |
| Official Go encrypted small-file roundtrip and exact replay | Pass, Go test 1.94 s | `foks-real-cell-schema52-go-a` |
| One celld owner killed, its local volume deleted | Pass, 14.483 s; only 0.517 s margin | `foks-real-cell-schema52-ownerloss-a` |
| Four cells, three new live files each, simultaneous owner loss | **Fail, all four**, 15.954–16.329 s | `foks-real-cell-schema52-four-history3-a` |
| Sixteen cells, three new live files each, simultaneous owner loss | **Fail, all sixteen**, 29.616–34.378 s | `foks-real-cell-schema52-sixteen-history3-a` |
| Four cells, thirty requested new live files each | **Preparation timed out at 180 s**, 27 ACKs per cell; no fault injected | `foks-real-cell-schema52-four-history30-a` |

All four and sixteen recovery ledgers ultimately verified every previous ACK and
the new write, while explicitly recording `within_budget: false`. Their failure
must remain visible. The history-30 experiment says nothing about recovery: it
exhausted the separate workload preparation budget. No acknowledged-data loss
was observed in the completed recovery experiments; this is not a proof for every
failure mode.

HTTP failures have distinct causes. The adapter promotion test's three HTTP 500s
were expected stale-epoch rejection (two identical publication attempts and one
stale snapshot). The one-owner test's 76 HTTP 500s followed fault injection during
recovery; its final failure was about 8.766 s after the kill. The four-cell and
sixteen-cell runs had 290 and 563 `/snapshot` HTTP 500s respectively during owner
recovery. These are genuine temporary unavailability, not expected stale-adapter
test rejection. The Go and history-30 preparation runs had no HTTP failures.

Peak sampled RSS in KiB was respectively native-test / initial celld owner /
surviving celld owner: four-cell 63,424 / 145,216 / 113,328; sixteen-cell
143,920 / 206,496 / 148,624. The native test process cohosts all adapters. These
are not per-tenant OS-process numbers, controlled capacity limits or an estimate
for 10,000 tenants. Sampling was every 0.5 seconds and can miss peaks.

## Qualification limits

The default celld lease is 10,000 ms, renewed at one third of its lifetime.
Separately labeled 3,000 ms trials retain identical bucket durability,
conditional publication, epoch checks and 15-second assertions. The shorter lease
increases renewal frequency about 3.3 times and sensitivity to scheduling/storage
latency. It does not remove projection or full-client-sync costs. These local
samples do not measure managed-S3 request cost or latency.

The approved resumed-former-node, 64-idle-cell and schema-54 requalification trials completed.
The 128-idle-cell and larger-namespace trials exhausted their preparation budgets;
those failures were retained, without extending the deadlines. Per-round durable
observations preserve partial results. Selected-cell coldness still requires
separate evidence: aggregate residency does not establish it.

The test client timeout is 5 s and bridge HTTP timeout 4 s. Native session defaults
remain a 30 s execution timeout and separate 15 s I/O timeouts per phase. A passing
recovery experiment does not establish a 15 s end-to-end worst-case request bound.
The final authority proof linearizes a response observation; it does not atomically
prevent a process paused afterward from releasing an overlapping response later.

Each directory contains the exact controller script, binary hashes, configuration,
test and celld logs, RSS samples, and external JSONL ACK/observation ledgers.
Private worker configuration contains fixture-only capabilities and should not
be published. The ledger-analysis files retain failed-run elapsed measurements.

## Schema-53 runs with Cargo quiet

Schema 53's focused Rust tests (4), native-schema Node/SQLite tests (6), and
feature-enabled clippy with warnings denied passed. A real Go repeat passed in
1.88 seconds with no HTTP failures. The following are additional observations,
not replacements for earlier failures:

| Run suffix under `/private/tmp/foks-real-cell-schema53-` | Result |
| --- | --- |
| `fencing-ttl3-a` | Pass, adapter promotion 7.548 s; three expected stale-epoch 500s |
| `ownerloss-default-b` | Pass, 14.554 s; 72 recovery `/snapshot` 500s |
| `ownerloss-default-c` | Guarded pass, 14.930 s; only about 70 ms margin; maximum elapsed-clock drift 0.328 ms |
| `pause-return-ttl3-a` | Pass, paused owner recovery 9.606 s; resumed former-node phase 5.576 s |
| `four-default-a` | **Fail, all four**, 16.468–17.111 s; all ACKs/new writes eventually verified; 286 recovery `/snapshot` 500s |
| `four-ttl3-b` | Guarded pass, all four 9.347–9.491 s; maximum elapsed-clock drift 0.342 ms |
| `sixteen-default-b` | Guarded **fail, all sixteen**, 29.782–32.837 s; all ACKs/new writes eventually verified; maximum drift 1.084 ms |
| `sixteen-ttl3-b` | Guarded **fail, all sixteen**, 24.739–26.607 s; all ACKs/new writes eventually verified; maximum drift 1.016 ms |
| `idle64-four-ttl3-a` | Guarded pass, 64 idle cells plus four active cells; active recovery 10.405–11.093 s; maximum drift 1.960 ms |
| `idle128-four-ttl3-a` | Guarded **preparation failure**, 120 identities completed before 300 s deadline; no active fault; maximum drift 3.284 ms |
| `four-history30-ttl3-a` | Guarded **preparation failure**, completed rounds 25/24/25/25 before 180 s deadline; no fault; maximum drift 2.058 ms |

The pause-return trial kept the former node's disk, resumed it after successor
verification, and routed a fresh read through its old ingress. That read failed
closed; the controller required exit 3 and a lease `SELF-FENCE` log before routing
back to the survivor and verifying another write/read. One of its two HTTP 502s
was initial recovery unavailability; the other was the intentionally fenced old
ingress. It did not attempt an ambiguous mutation through that old ingress.

Four additional runs are **invalid for ordinary timing comparison** because
celld log timestamps show large wall-clock discontinuities while monotonic elapsed
time advanced only seconds: `ownerloss-default-a` about 898–899 seconds,
`four-ttl3-a` about 551 seconds, `sixteen-default-a` about 105 seconds and
`sixteen-ttl3-a` about 822 seconds. The cause is unverified; host suspension or a
clock adjustment are possibilities. The singleton's survivor log jumps from
09:31:46 to 09:46:44 UTC and self-fences. All these cases retained failure evidence;
the latter three failed during preparation, before planned fault injection.
The singleton logged 21 recovery 500s then 100 502s after both survivors fenced.
Its postfault verification was incomplete, so it establishes neither ACK loss nor
successful preservation. The other invalid runs had 4, 16 and 16 preparation
502s respectively after all celld nodes self-fenced.

Subsequent trials add wall/monotonic drift recording and reject timing
qualification if their elapsed clocks differ by more than one second. A temporary
macOS `caffeinate -i` idle-sleep assertion is recorded for those trials; it does
not alter clocks or prevent manual/lid suspension, and the guard remains active.
Earlier schema-52 and unguarded schema-53 runs have no equivalent explicit guard;
their log timestamp audit is supporting evidence, not a controlled SLO guarantee.

In guarded `four-ttl3-b`, the twelve ordinary baseline write samples took
2.993–3.552 seconds and fresh full-sync samples 1.477–1.689 seconds. A complete
write-plus-sync round took 4.598–5.029 seconds. Each tenant's three-round baseline
executed 62 native RPCs through the bridge, serializing about 1.557 MB of SQL
snapshots and 6,321 bytes of typed row deltas, before recovery traffic. These tiny
samples characterize this exact workload only. `requests` counts native RPCs,
each of which performs multiple HTTP requests. `snapshot_bytes` excludes encrypted
key bundles and HTTP/envelope bytes; `delta_bytes` excludes its request envelope.
They are payload lower bounds, not total network or S3 transfer measurements.
The guarded sixteen-cell default baseline's 48 write samples took
8.654–13.999 seconds and full-sync samples 3.832–6.541 seconds. The bridge,
client traversal, celld and single-process emulator all contribute; this does not
isolate celld's own throughput. Reducing the lease improved observed recovery but
did not satisfy the target at sixteen simultaneously active tenants.

Recovery pass/fail and ranges above come from the external ledger's elapsed time,
which includes fault-record fsync. The additional per-round sample's secondary
`recovery_seconds` clock starts after that fsync and is slightly lower; it is not
the authoritative budget measurement. Early controller `content_passed` fields
mean the entire Rust test passed, including its timing assertions, not merely
that acknowledged content was preserved. A false value on a budget miss must not
be interpreted as data loss; subsequent controller versions call it `test_passed`.

## Capacity and namespace-growth observations

All rows below are schema-53 baseline, guarded, Cargo-quiet local samples. Ranges are
individual authenticated write and fresh complete-namespace sync durations, not
percentiles or sustained-throughput claims. The rounds create new live files.

| Active / idle cells; lease | Completed baseline samples | Write seconds | Full sync seconds | Write + sync seconds |
| --- | --- | --- | --- | --- |
| 4 / 0; 3 s | 12 | 2.993–3.552 | 1.477–1.689 | 4.598–5.029 |
| 16 / 0; 10 s | 48 | 8.654–13.999 | 3.832–6.541 | 12.860–20.540 |
| 16 / 0; 3 s | 48 | 10.553–12.860 | 4.283–6.377 | 16.295–18.906 |
| 4 / 64; 3 s | 12 | 3.179–4.067 | 1.525–1.866 | 4.870–5.630 |
| 4 / 0; 3 s, target 30 new files each | 99 complete rounds before timeout | 3.224–9.662 | 1.609–5.461 | 4.848–14.765 |

For the larger namespace, each tenant completed 24–25 full rounds, executing
482–502 native RPCs and serializing 18.074–19.122 MB of SQL snapshots versus
50,606–52,715 bytes of typed deltas. Tenant zero's snapshot payload per round grew
from 526,354 bytes on its first round to 1,046,138 bytes on round 25. This is direct
evidence of the bridge's whole-state serialization amplification. Wall-time
samples also reflect shared-host variance; no isolated causal throughput slope is
claimed. The external ACK ledger can contain a subsequent write whose full sync
was interrupted; the table counts only complete recorded rounds.

The 64-idle-cell population took 145.807 seconds. Its actual pre-active celld state
reported 32 occupied/resident cells, 32 inactive cells and 32 stops, with 64
publishes. Original native database/key/backup/log artifacts were explicitly
removed. First/last identity revisits took 0.156/0.175 seconds and matched SQL and
encrypted-key hashes. These are native-adapter reactivation observations; neither
selected celld cell was independently proven nonresident immediately beforehand.
The attempted 128 population completed 120 identities in 297.343 seconds and hit
the 300-second deadline before active workload or residency capture. This is a
bounded provisioning experiment failure, not evidence of a hard 120-cell limit.

| Guarded workload | Peak sampled native test KiB | Initial celld owner KiB | Successor celld owner KiB |
| --- | ---: | ---: | ---: |
| Four active, 3 s lease | 65,136 | 127,664 | 110,144 |
| Sixteen active, 10 s lease | 143,984 | 191,616 | 161,472 |
| Sixteen active, 3 s lease | 144,432 | 172,288 | 150,256 |
| 64 idle + four active, 3 s lease | 75,376 | 173,840 | 108,240 |
| Attempted 128 idle population | 65,696 | 179,552 | Not started |

These process samples exclude the emulator and controller, do not isolate
per-cell memory, and do not establish a 10,000-tenant capacity estimate. The
32-resident-cell setting is a cache/residency limit, not a total-tenant limit.

Guarded HTTP failures were exclusively recovery `/snapshot` 500s: singleton 72,
four active 60, sixteen default 518, sixteen tuned 146, and 64-idle/four-active 81.
They all followed the planned owner fault. Both failed preparation experiments
had zero HTTP failures. No malformed-request/adapter-bug failure was identified
in these runs. The earlier intentional stale-epoch rejections and old-ingress
502 remain separately classified above.

The outcome supports continued integration work, not production deployment:
three-second leases met the local target with four active cells, while sixteen
active cells missed it even after tuning. A persistent SQL integration that avoids
full-state reconstruction, complete identity/lifecycle coverage, node-wide
admission, and combined native-frontend/cell failure qualification remain needed.

## Final schema-54 requalification

After filesystem chunk-store authority binding landed in `c6040630`, the isolated
validation tree was advanced to that commit and overlaid with the reviewed opt-in
proof. All authority/proof source files were compared byte-for-byte with the main
worktree. Actual initialized schema-54 vectors were regenerated. Four Rust
projection tests, six native-schema Node/SQLite authority tests, five clock-guard
tests, changed-file formatting, and both default-feature and proof-feature
all-target clippy with warnings denied passed. Cargo's JSON artifact output
selected the rebuilt runtime executable; it was not selected merely by an old
filename.

The following four additional smokes ran with Cargo quiet, `caffeinate -i`, bucket
durability and the same clock guard/deadlines. Each recorded `test_passed: true`,
`timing_valid: true` and final qualification `passed: true`. They do not replace or
reinterpret the schema-53 capacity failures above. The capacity matrix remains a
pre-authority-patch baseline; these workloads never configure an external chunk
store, so the filesystem GC lifecycle itself is covered by its separate tests.

| Run suffix under `/private/tmp/foks-real-cell-schema54-` | Authoritative recovery result | Maximum clock drift | HTTP failures |
| --- | --- | --- | --- |
| `go-a` | Go encrypted roundtrip/replay pass, Go 2.187 s; no recovery fault | 0.0775 ms | None |
| `fencing-ttl3-a` | 7.436 s, all four ACKs verified | 0.1380 ms | Three expected stale-epoch 500s: two apply retries and one snapshot |
| `pause-return-ttl3-a` | Owner recovery 9.730 s; former-node return phase 5.723 s, five cumulative ACKs verified | 0.2231 ms | Two snapshot 502s: initial recovery and intentionally fenced old ingress |
| `four-ttl3-a` | All four 10.422–10.498 s, all sixteen cumulative ACKs verified | 0.2576 ms | Sixty recovery snapshot 500s after owner loss |

Recovery intervals in this table use external ledger `elapsed_ms`; detailed Rust
output has submillisecond durations. The former-node experiment again required
lease self-fence/exit 3, rejected the old-ingress fresh read, and verified a further
survivor write/read. It retains the no-ambiguous-old-node-write limitation.

| Schema-54 smoke | Peak native test KiB | Initial celld owner KiB | Successor celld owner KiB |
| --- | ---: | ---: | ---: |
| Go | 35,072 | 86,848 | Not started |
| Native adapter fencing | 39,904 | 124,224 | Not started |
| Paused owner and return | 39,808 | 98,432 | 89,616 |
| Four tuned cells | 65,552 | 161,920 | 114,592 |

Each schema-54 run's `final-measurements.json` retains source-base commit, SHA-256
for all non-document authority/proof manifest files and Cargo.lock, per-test and
clock qualification fields, authoritative recovery ledger records, HTTP classes
and sampled RSS. `controller.jsonl` additionally records the actual test/celld
binary hashes and host load. The source files remained identical between main
and validation tree after these runs. All earlier scope and capacity limits still
apply; this is local integration requalification, not a production 15-second
host/AZ guarantee.

The exact reviewed and tested proof implementation is committed as `541bc156`,
following storage authority fix `c6040630`. This evidence report and all Markdown
plans remain unstaged.
