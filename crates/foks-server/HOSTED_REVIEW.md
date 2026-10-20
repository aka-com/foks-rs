# Hosted deployment review after storage disaggregation

Reviewed 2026-10-14 at implementation commit `86032486`, with independent
capacity, HA/recovery, and storage reviews. Target: thousands to tens of thousands
of installations, personal files and organizations capped at 64 members using
credentials and notes. Desired successful recovery is within 15 seconds,
including host/AZ failure. These changes do **not** establish that hosted SLA.

## Delivered and validated

| Commit | Change |
| --- | --- |
| `7a825424` | Transactional KV quota/upload counters and reuse of validated reader configuration. |
| `ba06b885` | External recovery observation ledger, real-binary crash/restart and stale-backup experiments. |
| `776f1686` | Storage plan reviewed before implementation. |
| `86032486` | Schema 50, opt-in filesystem/S3-compatible encrypted chunks, mixed storage, self-contained backup hydration, offline resumable conversion. |

Components received a direct review and an independent subagent review before
tests; fixes were re-reviewed. Final validation passed 279 database/server tests
and 34 targeted integration/conformance/recovery tests, including two actual
pinned Go v0.1.9 client runs. Formatting and all-target clippy passed. Four mock
S3 tests were additionally rerun after a test-only lint cleanup. The live S3 test
and four existing benchmarks were not run. This was targeted validation, not a
full desktop/workspace test run.

The latest real-binary local crash cycles recovered authenticated reads of all
acknowledged values plus a new write/read in approximately 261, 286 and 331 ms.
The original volume survived, the workload was small, and failures were process
kills. These measurements say nothing about lost volumes, AZ failover, a cold
object provider, or simultaneous activation of thousands of tenants.

Wire encodings, encryption and padding remain unchanged. The Go gate encrypts,
uploads, publishes, downloads and decrypts a 4 MiB + 17-byte file against both
storage modes, with exact and conflicting retries and usage checks. Quota
charging retains its previous logical policy. Retained file versions are not
pruned, and external objects are not garbage-collected.

## Workload implications

| Workload | Current path and remaining constraint |
| --- | --- |
| Passwords and small notes | Small-file payloads up to 2,040 plaintext bytes (the Go-compatible 2 KiB boundary minus eight bytes of encoding overhead) remain in SQLite. Quota admission no longer scans namespace history. One writer still serializes mutations and durable commits. |
| Larger text notes and personal files | Each chunk uses a short writer preflight, immutable object PUT, then a writer commit with renewed authorization/order/quota checks. Publication into a directory remains a separate protocol operation. |
| Full-size file chunks | A 4 MiB plaintext chunk pads to roughly 8 MiB encrypted. External storage holds one canonical encoding instead of SQLite holding both ciphertext and encoding: roughly 2x rather than 4x plaintext payload storage for full chunks, excluding metadata, WAL and backups. |
| Exact upload retries | Server replay identity and accounting are preserved. A committed external retry currently still makes a conditional PUT and may GET the whole object for verification. |
| Long-lived update histories | Old small nodes and dirent versions remain charged and retained. Usage queries, startup validation and some tree walks still grow with history. |

The single writer is a reasonable architecture to qualify for these small
organizations. There is no measured requests-per-second or tenant-density claim.
The quota VM-step test demonstrates bounded query work, not bounded complete RPC
latency. Dense file serving is gated by memory and transfer admission below.

## Important next review and implementation candidates

**P1 means address or explicitly constrain before hosted launch. P2 means the
next operational/performance work. These findings are not implemented here.**

1. **P1: Replication must govern visibility, not only write acknowledgments.**
   `Writer::Call::run` in [writer.rs](src/writer.rs) executes local mutations
   before returning their result. Independent readers can already observe those
   commits. Adding a replication wait just before the successful reply would
   still expose unreplicated roots through reads or errors. See the item 1 plan
   below. Local `FULL` WAL and a filesystem writer lock do not provide cross-host
   durability or ownership.

2. **P1: Budget response bytes through transmission.**
   [session.rs](src/net/session.rs), `read_call_body_async`, accounts for twice
   the incoming frame size. A tiny chunk request can allocate an 8–9 MiB object,
   decoded ciphertext and encoded response. Execution permits end before socket
   transmission finishes, so slow receivers can accumulate responses. Add byte
   reservations for known object/response sizes, retain them through transmission,
   and impose per-tenant and fleet-wide ceilings. The 64 MiB request budget is
   not a total process-memory budget.

3. **P1: Release database leases during object I/O.**
   [handlers/kv.rs](src/net/session/handlers/kv.rs) retains its `ReadLease` while
   dispatching the entire operation. The download snapshot now ends before GET,
   but the lease itself remains occupied; uploads also retain one during PUT.
   Thirty-two delayed object operations can exhaust the default reader pool and
   obstruct credential reads. Split preparation, external transfer and response
   phases, and give bulk transfer separate bounded admission.

4. **P1: Persist and verify storage namespace identity.**
   [standalone.rs](src/standalone.rs) rejects a missing backend when external rows
   exist, but does not reject a different bucket/prefix. A mistaken configuration
   can admit new writes while old objects become inaccessible, splitting placement.
   Bind SQLite to an immutable storage namespace marker, verify it before serving,
   and make relocation explicit. Until then the configured store/prefix must be
   operationally immutable. This is entirely an internal storage change.

5. **P1: Account for physical objects and design safe reclamation.**
   [services/kv.rs](src/services/kv.rs) can leave an orphan after a durable PUT
   followed by failed commit admission. [Upload maintenance](../foks-server-db/src/maintenance/uploads.rs)
   eventually removes abandoned metadata and releases logical quota, while all
   external objects remain. Repeated abandoned uploads can therefore grow physical
   storage beyond quota. Add durable pending/live/deleting records, bounded
   reservations, physical usage metrics and ownership-aware reclamation. Even an
   offline collector must exclude standalone backup processes as well as servers.
   Coordinate publication, reads and in-progress backups; do not use bucket TTL
   deletion. Historical-version pruning can be a separate configurable,
   wire-compatible policy, with cached Go references explicitly tested.

6. **P1: Qualify tenant activation and process density.**
   [listener.rs](src/net/listener.rs) creates a runtime with four default workers,
   while writer, maintenance, management and optional backup work add threads.
   Maintenance wakes every minute. The S3 runtime is shared within one process,
   not across 10,000 standalone processes. Measure a live-process baseline and
   compare it with shared runtimes/scheduling or activation on demand. Keep
   sufficient warm capacity to recover the active tenants of a failed host/AZ.

7. **P2: Persist resumable client chunk uploads.**
   [put_file](../foks-client/src/kv/write.rs) generates a new file ID and seed on
   each call, and uploads before journaling the namespace mutation. Calling it
   again after an ambiguous response starts a new upload. The server supports
   exact replay, but the client needs durable upload identity, secret material,
   progress and reproducible chunk bytes to use that property across restart.
   This can preserve the wire format; bound local storage and protect those secrets.

8. **P2: Reduce history, startup and backup costs.**
   [kv_usage](../foks-server-db/src/kv.rs) still decodes retained small nodes and
   aggregates uploads. Cold database open still validates tree capacity; idle
   reader pools retain their high-water connection count. Consider maintained
   usage counters, bounded validation/activation and reader eviction. Conversion
   frees SQLite pages but does not shrink the file. Provide explicit offline
   compaction. Backup hydration recreates duplicated inline payloads and may
   exceed the configured database limit or temporary disk despite a tiny live
   metadata database. Estimate hydrated size, budget fleet backup I/O, and consider
   a future operator-only metadata/object export format. Local key-directory locks
   currently coordinate rotation and backups; preserve that exclusion in the
   distributed ownership design.

9. **P2: Optimize retries and qualify the real provider.**
   A committed identical retry could skip another PUT if preflight distinguishes
   it from a newly admitted chunk; retain authorization checks and decide whether
   retries should also check object availability. Coalesce same-digest transfers.
   R2 documents a one-write-per-second concurrent same-key limit with 429 responses
   above it, so retry storms need explicit testing. [R2 limits](https://developers.cloudflare.com/r2/platform/limits/)
   R2's atomic conditional writes and read-after-write consistency support this
   publication pattern, but a negative read cannot prove an ambiguous PUT will
   never commit. [R2 consistency](https://developers.cloudflare.com/r2/reference/consistency/)
   Test full chunks, rejected conditional creates, lost replies, throttling,
   credential refresh and failover-location reads. The opt-in live test is only a
   basic roundtrip, not provider durability certification.

## Item 1: zero-loss replication and fencing

Use zero loss of **client-visible authoritative state** as the recovery contract.
The harness proves that stale Merkle roots and returned older dirent versions
can be rejected by existing clients. It also proves that disappearance of a newly
inserted entry can silently look like deletion. Thus a five-minute rewind is
neither uniformly usable nor uniformly detected. Never clear trust anchors to
make recovery pass; revocation and key-state rewind are particularly serious.

Recommended implementation and qualification order:

1. Choose the metadata durability boundary. Prototype synchronous physical
   SQLite replication or an appropriate replicated storage layer. Arbitrary
   Rust writer closures cannot simply become a deterministic replicated command
   log; that alternative requires a broader mutation redesign. Define which
   host/AZ failures the surviving replicas must tolerate before choosing placement.
2. Introduce internal ownership epochs and durable commit receipts containing
   tenant, epoch and replicated position. Fence at the authoritative commit/storage
   boundary. A lease check before a closure is insufficient if the process pauses
   before commit. Cover bootstrap, operator tools, maintenance, conversion and key
   rotation as well as RPCs.
3. Prevent reads, errors and successful responses from exposing commits beyond
   the durable position. Seal the old epoch and recover its acknowledged tail
   before promoting a successor. Keep stale owners from acknowledging divergent
   branches even when their old TCP connections survive.
4. Replicate required encrypted key generations and retain operator-root access
   before publishing their metadata references. Preserve HostID and stable endpoint
   identity. Include key retrieval, routing and TLS reconnect in the recovery budget.
5. Extend the controller-owned ledger harness to kill/partition around commit,
   replication and response; revoke credentials and rotate keys during faults;
   pause an old owner, promote another, then resume old queued and executing work.
   `execute_bounded` currently times out without cancelling `spawn_blocking`, so
   request cancellation must not be mistaken for mutation cancellation or fencing.
6. Test correlated host/AZ loss at realistic history sizes and tenant counts,
   including backup activity and object-store delay. Stop the clock only after
   all expected authenticated reads and a new durable write/read succeed. State
   the quorum/network conditions under which 15 seconds is achievable; loss of
   the required quorum cannot guarantee successful consistent writes on a deadline.

These can be server/hosting changes without protocol extensions. Keep one active
metadata writer per installation until this ownership model is established.

## Platform considerations

The current binary uses native Rust, TCP/mTLS and SQLite. External chunk storage
does not port that execution or transport model into a Worker. A platform adapter
must preserve client authentication and connect mutations and output to the
platform's durability/ownership mechanism.

Cloudflare SQLite Durable Objects currently document 10 GB per object and a
2 MB maximum SQL row/BLOB. Moving chunks out removes the largest known payload
obstacle; other schema sizes, SQL operations and the native adapter still need
qualification. [Durable Object limits](https://developers.cloudflare.com/durable-objects/platform/limits/)

Celld describes epoch fencing and durability gates covering writes, reads,
errors and streamed output. However, its default lease is 10 seconds, large-node
recovery can take minutes, and recovery can require surviving follower data.
Verify cross-AZ placement and recovery bounds rather than assuming 15 seconds.
[Celld guarantees](https://celld.dev/docs/guarantees/)
Its containers have ephemeral disks and are experimental; running this binary
inside one does not itself make SQLite durable. [Celld containers](https://celld.dev/docs/services/containers/)

## Next concrete work

Start the metadata ownership/replication prototype alongside response-byte
admission, reader-lease separation and storage-identity binding. Follow with
physical object accounting/GC, resumable client uploads and fleet activation
qualification. Benchmark small secret writes, note overwrites, bursts of directory
mutations, file uploads, exact retries and slow downloads. Record p50/p95/p99 and
worst observed end-to-end latency, writer queue time, RSS, reader occupancy, WAL
growth, object operations and recovery time under both steady load and failures.
Use 10,000 mostly idle tenants plus concentrated active subsets; do not extrapolate
single-tenant functional tests into a hosted capacity promise.
