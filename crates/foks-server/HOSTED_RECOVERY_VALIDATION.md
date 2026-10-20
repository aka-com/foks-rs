# Hosted recovery validation and implementation sequence

Status: local experiments executed against implementation `3c60f78e`; actual
host/AZ qualification remains blocked on metadata replication and fencing. Plans and
results in this document remain unstaged. The target is successful recovery in
15 seconds after a specified host/AZ failure, with no lost client-visible
authoritative state. Organizations are capped at 64 members; personal servers
may store files. Protocol, cryptography and Go wire compatibility stay unchanged.

## What can be established with the current implementation

There is no metadata replication, distributed ownership or automatic promotion
implementation in this repository. Moving encrypted chunks out of SQLite did
not add one. Consequently a local experiment can establish restore mechanics,
measure a limited workload, and expose failure cases; it cannot certify zero-loss
host/AZ failover for the current server.

The local harness models nodes with independent server directories/processes and
keeps controller/client material outside the removed server paths. They still
share one OS, physical disk and loopback network. Source removal deliberately
deletes only generated fixture databases, keys, backups and server configuration.
It is not a power-loss test or an AZ network partition. Operator root material
and test TLS configuration are supplied by the surviving controller fixture;
this does not measure a production secret manager.

The complete-snapshot case quiesces application writes before copying a finished
backup to a surviving directory. The copy is durable before the fault. It has no
replication lag because there are no later writes in that positive fixture.
The stale-snapshot case adds an acknowledged insertion after that copy and shows
why this assumption is insufficient for an actively writing hosted server.

The former-owner experiment alternates two independent database copies at the
same stable endpoint. Two pre-pinned observers keep their original hard, soft and
protected client stores. A former owner accepting a divergent write is evidence
of missing fencing. This experiment does not yet model both owners reachable at
once, paused TCP sessions, or queued mutations resuming across promotion.

The simultaneous-restore fixture destroys every source before launching parallel
restores. Its default is four installations; an explicit 1–32 count is supported
for this development-machine fixture. All clocks start before any source is
removed. It is a small correlated-loss simulation, not a 10,000-tenant density
benchmark. The aggregate elapsed measurement conservatively includes fixture
teardown; per-tenant measurements stop after authenticated read/write validation.

## Reproducible local experiments

Build the current server binary, then opt in to the ignored real-process tests:

```sh
cargo build --locked -j 2 -p foks-server --bin foks-server
cargo test --locked -j 2 -p foks-server-testkit --lib recovery::tests
FOKS_SERVER_BIN="$PWD/target/debug/foks-server" \
  FOKS_RECOVERY_EVIDENCE_DIR=/tmp/foks-recovery-new-run \
  cargo test --locked -j 2 -p foks-server-testkit --test recovery_harness \
  -- --ignored --test-threads=1 --nocapture
```

Select a new evidence directory for each run; existing ledger filenames fail
instead of overwriting evidence. `FOKS_RECOVERY_BUDGET_MS` defaults to 15000.
`FOKS_RECOVERY_TENANTS` sets the simultaneous-restore count. The original
three-cycle process-restart test separately accepts `FOKS_RECOVERY_LEDGER` as an
explicit output filename. Logs contain synthetic operation identifiers and
hashes, never actual passwords or private keys.

The ledger requires every recorded acknowledgment to be freshly read and a new
write/read to succeed. Changing the selected recovery owner clears that attempt's
observations and write eligibility while retaining all expectations and the
original fault time. Missing acknowledgments produce a recorded rejection, even
if the replacement is healthy and accepts new writes. Negative experiments pass
their assertions by demonstrating a failed recovery contract; they are not
successful HA trials. Tests use completion-time assertions, not a hard watchdog.

## Local results, 2026-10-14

Direct review and independent subagent review completed before testing. Three
ledger unit tests and all eight real-binary harness tests passed. A separate
16-installation simultaneous-restore trial also passed. Targeted testkit clippy
and formatting passed. The harness implementation is committed as `2bcf7ec0`;
the Markdown reports/plans remain unstaged. These are debug-build functional experiments on macOS
26.5.1 / arm64 with warm OS caches and synthetic small files; not release-profile
capacity benchmarks. Binary fixture settings include eight worker threads per
installation, 128 active connections, 32 pending connections and 16 pending writes.
No remote object store is used by these new volume fixtures.

| Experiment | Observed result | Meaning |
| --- | --- | --- |
| Process kill/restart, original volume survives | 240, 293, 325 ms | Three small local recovery cycles pass. |
| Source files destroyed, complete surviving snapshot | 341 ms | Restore/activate/authenticated read and new write/read pass. |
| Four source installations destroyed, parallel restore | 853 ms aggregate; 781–826 ms per tenant | All four complete within the common 15-second budget. |
| Sixteen source installations destroyed, parallel restore | 3.501 s aggregate; 2.842–3.432 s per tenant | All sixteen complete within the common budget; no density extrapolation. |
| Source destroyed, snapshot predates an acknowledged insertion | Recovery rejected: 2 of 3 acknowledged values present | A healthy replacement and new writes do not repair the lost tail. |
| Former owner reused after replacement served writes | Both copies accept divergent writes; selected owner has 3 of 4 acknowledgments | Distributed ownership fencing is absent. |

All positive volume trials copied a complete quiescent snapshot before failure.
They do not establish zero-loss replication during active writes. Negative trials
pass the test suite precisely because the ledger rejects the recovery contract.
Neither an actual AZ nor a simultaneously reachable partitioned owner was tested.

Evidence from this run is retained in
`/tmp/foks-host-recovery-4ox3la7d/*.jsonl`, with command output in
`/tmp/foks-host-recovery-tests.log`, `/tmp/foks-host-recovery-16.log`,
`/tmp/foks-host-recovery-unit.log` and `/tmp/foks-host-recovery-clippy.log`.
These temporary paths are local artifacts, not a durable CI archive. Server binary
SHA-256: `594a270c9d75da643537971a677a8a02e5aed736aaed7a42385d99628707411c`.

## Separate celld platform experiment

A three-process celld 0.6.2 smoke test with an isolated local S3 emulator recovered
eight toy cells after killing their owner. It retained all 40 acknowledged values
and completed eight new writes plus cross-node reads in **10.068 seconds** from
the fault. The controller retained 296 retry errors; the last was at 9.638 seconds.
The default lease was 10 seconds. Readiness gating and rebalancing were disabled
for the fixture, and the object emulator remained alive. No FOKS code ran in
celld, no former owner resumed, and no source files or AZ were removed.

This is evidence of one limited platform recovery path, with roughly five seconds
of margin in this workload. It is not a native FOKS, production-S3, disk-loss or AZ
qualification. See [celld experiment](HOSTED_CELLD_EXPERIMENT.md) for exact tuning,
artifacts, initial-controller correction and remaining fault cases. Integrating
FOKS with the platform's durable database and output/ownership boundary was the
prerequisite for repeating the actual FOKS observation contract there. The newer
[real FOKS proof results](HOSTED_CELLD_PROOF_RESULTS.md) now exercise that boundary
for a bounded small-file workload. They supersede this toy experiment's coverage,
while leaving physical host/AZ and production-backend qualification open.

See the [native-versus-celld assessment](HOSTED_NATIVE_VS_CELLD.md) for a direct
architecture comparison and the source/log explanation of the transient HTTP
errors. The native restore and celld failover times measure different work.

## Work needed before the actual 15-second host/AZ gate

1. **Select and integrate a replicated metadata backend.** A platform choice is
   an implementation dependency, not just a deployment flag. Prototype a
   physical SQLite/replicated-storage boundary for the native server, or a cell
   adapter that actually routes mutations through the platform's durable store.
   Do not run an ordinary local SQLite file beside a replicated counter and call
   the server replicated. Rust writer closures cannot simply be serialized into
   a deterministic command log.
2. **Define durable visibility and ownership.** Carry an internal tenant/epoch/
   commit-position receipt. The authoritative storage system must reject a stale
   epoch at commit, including work queued or paused before lease loss. A lease
   precheck or process-local file lock is insufficient. Gate reads, errors and
   successful outputs on replicated state, not only write acknowledgments.
3. **Recover one installation coherently.** Seal the previous epoch and recover
   its durable tail before promotion. Required encrypted key generations must
   survive the same permitted failure before their metadata becomes visible.
   Keep HostID, trust anchors, endpoint identity and operator-root access intact.
   Verify configured chunk namespace identity without full object inventory on
   the activation path. Missing keys or quorum means unavailable, not bootstrap
   a replacement identity or clear client pins.
4. **Add the real deployment adapter to the controller.** Capture topology,
   durability mode, key service, routing, backend version and exact binary. Inject
   process/volume loss, network partitions, stopped/resumed former owners and
   correlated AZ loss without destroying the controller or surviving replicas.
   Declare the fault domains and surviving quorum before each trial. Keep old
   connections and queued work for the stale-owner experiment.
5. **Run failure-boundary and workload tests.** Interrupt before local commit,
   between commit and replication, before/after response, and during read/error
   publication, revocation, key rotation, upload finalization and collection.
   Reconcile ambiguous operations; verify all acknowledged/read-observed state.
   A rollback rejection is a safety signal, not a successful availability result.
6. **Qualify the full time budget under correlated load.** The 15 seconds includes
   detection, fencing, tail recovery, scheduling, key/DB activation, routing, TLS,
   authenticated reads and a new durably acknowledged write/read. Freeze the
   affected active-tenant set before the fault and report its slowest recovery,
   including every failure to recover. Preserve headroom for losing the largest
   supported failure domain. State what healthy network/quorum conditions the
   target assumes; loss of necessary quorum cannot promise consistent writes.

Use a finite declared observation set for the timed qualification. Separately
audit the full larger data history and object reachability, and publish that
duration and result. A small successful probe is not proof that all history
survived. None of the local positive measurements replaces steps 1–6.

## Related change plans and recommended order

| Order | Component | Concrete deliverable and acceptance |
| --- | --- | --- |
| 1, alongside replication prototype | [Storage namespace binding](HOSTED_STORAGE_FOLLOWUP_PLAN.md) | Database/marker identity, fail-closed startup, explicit legacy adoption, backup/restore semantics; wrong prefix cannot accept writes. |
| 2 | [Reader phases and byte ownership](HOSTED_CAPACITY_PLAN.md) | Owned preparation descriptors, no DB lease during object I/O, complete allocation reservations retained by queued/detached work and socket responses; slow transfers do not starve credential traffic. |
| 3 | [Physical object accounting](HOSTED_STORAGE_FOLLOWUP_PLAN.md) | Durable reservations/attempts, conservative uncertain-byte accounting, bounded admission and report-only inventory; failed publication remains charged physically. |
| 4 | [Client upload recovery](HOSTED_UPLOAD_PLAN.md) | Protected bounded chunk spool, stable upload handle, exact replay and atomic namespace association; ambiguous publication reconciles before reupload. |
| 5, after accounting/ownership | [Coordinated reclamation](HOSTED_STORAGE_FOLLOWUP_PLAN.md) | Non-reused object incarnations, read/backup pins, tombstones and provider-qualified handling of ambiguous PUT/DELETE; begin report-only. |
| Continuous baseline; final gate after integration | [Density and simultaneous recovery](HOSTED_CAPACITY_PLAN.md) | Disclosed history/load/active-fraction matrix, real resource measurements, failed-domain headroom and end-to-end fault trials. |

Each implementation slice gets direct review, independent review, focused tests
and its own implementation commit. Plans alone change no runtime behavior. Keep
logical quota reporting unchanged initially; physical storage limits are a
separate control. Historical-version pruning remains a separately tested policy.

## Lower-priority platform alternative

[Rivet assessment](HOSTED_RIVET_ASSESSMENT.md) reviews current deployment options,
maintenance evidence, the announced S3-only 3.0 redesign, and the native FOKS
integration gates. It is a source/API assessment, not a Rivet deployment result.
