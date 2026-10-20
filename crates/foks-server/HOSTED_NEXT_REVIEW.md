# Follow-up review after hosted hardening

Independent follow-up review, 2026-10-15. Keep this file unstaged.
The completed implementation and test ledger is HOSTED_EXECUTION_PLAN.md. This
review distinguishes production requirements from the local celld proof. See
HOSTED_CELLD_PROOF_RESULTS.md for run-specific measurements and qualification limits.

## Additional celld lifetime findings, 2026-10-16

The guarded-node source review found two prerequisites for the next implementation group. `Generation::is_drained` and retirement inspect ordinary worker pools; guarded activations need their own retirement pin because those workers are deliberately excluded from the pools. An `Arc<Generation>` alone does not preserve lookup eligibility or prevent premature drain classification.

The pinned celld LTX layer also cancels hydration and compaction without establishing that all retained async roots, blocking jobs and result payloads have finished. Canonical stop success or an empty task registry is insufficient evidence for physical activation retirement. The next plan must retain and join per-epoch work, including nested native jobs, before capacity reuse or VFS unregistration. Existing upstream LTX allocations must be measured/classified honestly; charging the size of an opaque pointer does not account for its boxed payload.

There is also a distinct ordinary-path review candidate: `Hydrator::step` in `crates/ltx/src/paged_vfs.rs` repeatedly looks up the VFS by name and dereferences a later lookup without checking for null, while celld's close path can unregister it after cancellation alone. The same review found a possible reclamation race between final file close and unregistration. Source inspection establishes the problematic orderings; neither has yet been reproduced or repaired. The [VFS lifetime candidate plan](HOSTED_CELLD_VFS_LIFETIME_PLAN.md) records the proposed ownership fix, outstanding lookup/open admission proof and deterministic tests. The guarded path must join hydration before unregistering regardless. None of these observations establishes termination of remote PUT attempts or authorizes object deletion.

## Highest priority

1. **A production authoritative metadata path and ownership boundary.** Immutable
   encrypted objects in strongly consistent S3 are durable, but they do not make a
   local SQLite metadata commit durable. A successful mutation reply must follow
   durable publication of its authoritative transaction and replay receipt. The
   celld proof demonstrates this for a deliberately narrow pre-provisioned small-KV
   workload. It is not a production SQL backend. Implement a persistent adapter
   that preserves native transaction, uniqueness, trigger and authorization
   semantics; qualify every mutation, identity update, Merkle checkpoint, key
   rotation and restore path. Do not silently fall back to local authoritative
   writes if cell authority is unavailable.
2. **Fifteen seconds needs a complete deadline and recovery budget.** Native server
   defaults are a 30-second execution timeout and a separate 15-second I/O timeout,
   applied at multiple phases. They do not enforce a 15-second end-to-end request
   bound. The local proof uses a 5-second client timeout and 4-second bridge HTTP
   timeout. Propagate an absolute deadline through admission, routing, activation,
   TLS, storage, retries and response transmission, retaining resource ownership
   after cancellation. Test slow readers, stalled storage, partitioned old owners
   and client retry behavior. A retryable error is not successful recovery.
   Schema-53 baseline observations with Cargo quiet (before the filesystem
   authority-only schema-54 correction): the default lease produced
   singleton passes at 14.554 and 14.930 seconds, four-tenant failures at
   16.468–17.111 seconds and guarded sixteen-tenant failures at 29.782–32.837
   seconds. A 3-second lease gave guarded four-tenant passes at 9.347–9.491
   seconds, but sixteen tenants still failed at 24.739–26.607 seconds. These are
   bounded local experiments, not production SLO guarantees. Clock-discontinuous
   runs are reported separately and excluded from ordinary timing comparisons.
3. **Eliminate whole-history projection before sizing a production fleet.** The
   proof exports full SQL/key snapshots and materializes a disposable SQLite DB
   for requests, then applies typed row changes and checks the resulting digest.
   One tiny fencing trial moved about 1.9 MiB of snapshot JSON over 73 calls.
   Work and memory grow with current namespace size, retained history and client
   full-sync fanout; explicit 8 MiB/10,000-row proof bounds stop
   growth rather than making it scalable. Replace this with persistent storage
   access and incremental transactions, preserving conflict/idempotency checks.
   Use the same acknowledgment and stale-owner tests against the replacement.

## Zero-loss replication and fencing qualification

Treat managed S3 durability, availability and strong consistency as assumptions.
Qualify our acknowledgment and ownership protocol, rather than reimplementing
provider replication. Required evidence:

- Distinguish the native adapter execution epoch from celld node/cell ownership.
  A passing adapter fencing test alone does not qualify celld stale-node return.
  Production must establish both boundaries and test stop/resume of old nodes.
- One durable, monotonic ownership epoch per tenant; every authoritative commit
  checks it atomically with transaction publication. A local file lock is only
  host-local exclusion. A stale process must fail even if its old disk and network
  return, and reads must not serve unfenced local snapshots as current authority.
- Replies follow durable transaction publication and a recoverable replay receipt.
  An interrupted publication can have succeeded. Retry the same exact identity
  and reconcile state; do not execute a new mutation to guess the old outcome.
- Activation verifies a consistent metadata/log prefix and the exact key bundle,
  then establishes routing and authentication within the recovery budget. Keep
  operator-root/KMS recovery and certificate/key rotation in this budget. A DB
  backup alone does not recover an installation.
- External acknowledgment ledgers cover failures before/after commit and reply,
  lost replies, pauses before/after fencing checks, partitions, stale owner return,
  concurrent activation and correlated recovery. Verify all acknowledged values,
  a new write/read, ordering, quota and absence of divergent accepted successors.
- A final epoch check is a linearization observation, not an atomic fence on a
  future socket write by a paused process. State the overlapping-request semantics
  precisely; do not claim physical suppression of every stale packet.
- Qualify clock discontinuities and host suspension as well as process faults.
  One final local singleton trial logged a roughly 15-minute wall-clock jump on
  both surviving celld nodes while the controller's monotonic experiment clock
  advanced about 27 seconds. Both survivors self-fenced, and post-fault verification
  did not complete. The cause is unverified; preserve this as a separate
  environmental failure, not an ordinary lease-latency sample. Independent fault
  domains, clock monitoring and enough surviving capacity matter for availability.
- A five-minute metadata rewind is not automatically transparent: clients retain
  cryptographic history/checkpoints and local mutation receipts. Verify rollback
  detection, recovery/reconciliation and consistency of keys, metadata and objects.
  A rollback rejection is safe behavior, but it is not successful availability.

The local proof cannot qualify physical-host/AZ placement, managed-S3 latency,
network routing convergence, region disaster recovery, operator/KMS availability
or deployment security. Those need a disposable deployment once available.
The current celld process-loss tests fault after known acknowledgments. Add a
process crash between authoritative commit and native reply: exact-replay tests
and pausing a native candidate before apply do not cover that process boundary.
Native-adapter replacement and celld-owner loss are also separate experiments.
Qualify their combined failure when deployment placement can lose both together,
including frontdoor reconnect, key recovery and routing convergence inside the
same 15-second budget. Keeping the native test adapter alive while killing celld
does not establish that combined recovery time.

## Resource and storage follow-ups

- Add node-wide admission and recovery headroom. Current byte allowances are per
  server and exclude SQLite page caches, allocator fragmentation, TLS/kernel
  buffers and runtime overhead. Tens of thousands of independent runtime/writer/
  maintenance thread sets are not a viable sizing assumption. Measure idle cells,
  active concurrency, history and eviction; then share runtimes and evict idle
  readers/tenants while preserving each tenant's writer/authorization boundary.
  The proof's native test process cohosts adapters; its RSS is not a measurement
  of one separate native OS process per tenant.
- Safe client receipt retirement is a rollout prerequisite. The first resumable
  upload implementation retains at most 1,024 identities per profile for life;
  cancelled/preparation/explicit small-upload identities count too. Design an
  expiring versioned handle with durable rejection watermark, or a monotonic ID
  allocator with a retained high-water mark, before deleting receipts. Deleting
  arbitrary random-ID tombstones would allow unsafe identity reuse. Preserve
  source-free resume, application receipt ordering and changed-destination checks.
- Upload resume replays immutable chunks from the beginning because the unchanged
  server protocol has no resumable cursor query. Measure retransmission and spool
  costs. The 256 MiB default encrypted-spool budget limits practical plaintext
  sizes below the wire maximum; padding and conservative temporary reservations
  matter. Keep small inline-file outcome reporting distinct from large-file resume.
- Filesystem collection authority is intentionally host-local. Fresh version-2
  namespaces pin one canonical database path and device/inode under physical
  namespace leases. Existing version-1 namespaces stay report-only; a raw database
  copy cannot inherit version-2 authority. This does not detect same-inode external
  rollback and cannot serve as a replicated ownership fence. Use supported hydrated
  backups/restores; volume replacement, raw database handoff and in-place rollback
  need an explicit recovery design before enabling collection on those paths.
- Remote object collection remains report-only. SDK mutation retries can leave
  an earlier PUT outstanding after a later successful attempt. Establish a
  single-attempt publication contract or durable subattempt tracking before S3
  deletion. Presence/listing/age/timeout does not prove termination. Keep uncertain
  attempts charged. Persist the publication-proof version: switching future PUTs
  to one attempt does not retroactively prove old retrying successes are safe to
  collect. Migrate legacy successes conservatively. Existing histories continue
  protecting referenced objects;
  the offline collector only reclaims unreferenced, eligible incarnations.
- The default 128 unresolved-publication limit is an outage budget that does not
  automatically recover. Positive GET clears uncertain bytes but retains the
  unresolved attempt, because the old PUT might still arrive. Repeated transient
  failures can block new external objects despite free physical space. Provide
  clear diagnostics and a proved operator drain/reconciliation workflow; age-based
  clearance is unsafe.
- Bound long-term tombstone/attempt metadata, startup validation and inventory work.
  Physical deletion does not delete its safety tombstone or successful attempt
  records, and the charged-object limit excludes deleted states. Add separate SQL
  row/metadata budgets and a proved retirement strategy for high-churn tenants.
  Writer activation still scans retained KV trees to validate capacity; measure
  this independently of one-time schema migration. Filesystem inventory
  currently rescans a flat directory per bounded page, which has quadratic total
  enumeration work. Consider sharding keys or a durable inventory index, with
  restart/cursor correctness tests. Schema migration copies payload tables and
  requires temporary disk headroom; run it as planned maintenance, not failover.
- Collection object/byte bounds do not bound SQL scan time. A custom byte cap below
  candidates in the first selected window can repeatedly skip that window; report
  oversized candidates and consider a durable cursor or fair bounded selection.
- Prefer explicit canonical schema definitions over SQL-text rewriting in future
  placement migrations. Preserve old schema fixtures, logical/physical counters,
  mixed inline/external payloads, foreign keys and transactional rollback tests.

## Ranked client, agent and desktop follow-ups

These findings are from the independent client review after item 4 (`1e0b7410`).
They distinguish rollout prerequisites from later improvements; none requires a
change to the remote FOKS wire format.

1. **Rollout prerequisite: define request and bulk-transfer budgets separately.**
   The desktop sets its IPC client to 60 seconds
   (`apps/desktop/src-tauri/src/agent.rs:305`) and launches its managed agent with
   `--request-timeout-seconds 60`
   (`apps/desktop/src-tauri/src/agent/process.rs:221`). Standalone AgentClient and
   agent defaults are 15 seconds (`crates/foks-agent-client/src/lib.rs:80`,
   `crates/foks-agent/src/main.rs:165`). The official IPC upload client already
   uses an absolute deadline across registration and streaming
   (`crates/foks-agent-client/src/lib.rs:151`); do not describe it as unbounded.
   However, agent streaming admission, readiness, each frame and final worker wait
   receive separate timeout periods (`crates/foks-agent/src/main.rs:1430`), and
   pooled remote calls start a new deadline per RPC
   (`crates/foks-client/src/transport.rs:250`). There is no common end-to-end
   15-second guarantee across these layers.

   Resume retransmits every immutable chunk from offset zero
   (`crates/foks-client/src/kv/uploads.rs:619`). If a complete content pass cannot
   fit the effective caller budget, repeated resumptions cannot finish even though
   durable preparation survives. This is a conditional progress failure, not a
   measured throughput ceiling. Consider durable background transfer jobs with
   bounded acceptance/status calls, fair scheduling and explicit throughput
   assumptions. State whether the 15-second SLO covers one RPC, a user-visible
   status response, or completion of an entire file. Test throttled transfers,
   slow IPC producers, blocked storage and cancellation while retaining resource
   ownership until work actually ends.

2. **Rollout prerequisite for accepted rewinds: separate historical receipts from
   current server presence.** Finalized upload receipts deliberately return from
   local state before fresh authentication or remote access
   (`crates/foks-client/src/kv/uploads.rs:270`,
   `crates/foks-client-app/src/kv/uploads.rs:249`). Desktop labels this action
   “Verify receipt” (`apps/desktop/src/screens/file-uploads.tsx:114`). It proves an
   earlier publication; it does not prove the file survived a later server rewind.
   Missing inserted entries can become absent without a rollback error, while an
   older surviving dirent version is rejected
   (`crates/foks-client-db/src/soft.rs:1178`, `:1190`).

   Preserve the offline receipt guarantee but describe its scope explicitly. A
   separate current-presence audit needs retained destination/dirent binding;
   cleanup currently deletes the protected manifest containing that information
   (`crates/foks-client/src/kv/uploads.rs:261`). Retain an appropriately protected,
   compact binding if this audit becomes a product feature. Test insertion-loss
   and overwrite-rewind independently with a pre-existing finalized local receipt.
   Never automatically erase rollback anchors or republish to guess an outcome.
   A locally successful receipt lookup alone must not satisfy the hosted recovery
   acceptance check.

3. **Rollout prerequisite: retire receipts without reopening old identities.**
   The permanent 1,024-record ceiling is enforced in
   `crates/foks-client-db/src/repositories/uploads.rs:255`; cancellation and
   explicit small-file identities count. Prefer extending the existing adapter
   approach—validated time, bounded terminal retention and a durable rejection
   watermark—over creating another independent scheme
   (`crates/foks-client-db/src/repositories/adapter/clock.rs:6`, `:20`, `:61`).
   Version the local handles/API, reject new legacy registrations after migration,
   preserve existing live legacy uploads, and retire only safely resolved material.
   A monotonic allocator/high-water mark alternative must handle outstanding lower
   handles, out-of-order completion, profile backup restore and cloning. A restored
   counter must not issue an old identity again; a retirement watermark must not
   discard the only recovery material for a still-live lower handle. A counter by
   itself is not a complete design. Test the 1,025th preparation after retirement,
   replay of retired handles, lost registration replies, clock jumps, restart and
   restored/cloned profiles, including external rollback-checkpoint publication.

4. **Capacity prerequisite: qualify current-namespace sync amplification.**
   Upload publication performs a complete metadata sync
   (`crates/foks-client/src/kv/uploads.rs:658`). Complete traversal includes current
   directories, entries and small-file ciphertext, and can repeat three times on
   stale-cache conflicts (`crates/foks-client/src/kv/sync.rs:858`, `:965`, `:1363`).
   Concurrent organization writes can amplify both traffic and tail latency.
   This client traversal is not a download of every retained file version:
   path-scoped operations already exist (`crates/foks-client/src/kv/write.rs:117`),
   and metadata traversal reuses cached large-file node metadata
   (`crates/foks-client/src/kv/sync.rs:862`). Extend scoped paths where their proofs
   suffice; a partial version vector cannot prove the absence of a new entry.
   Measure flat and wide namespaces, concurrent writers, warm/cold caches, retries,
   RPC counts, bytes and tail latency independently of the proof backend's SQL
   snapshot amplification.

5. **Responsiveness follow-up: remove synchronous cleanup from receipt latency.**
   Cleanup loops over up to 256 chunk removals and the manifest before returning
   (`crates/foks-client/src/kv/uploads.rs:244`). Each removal, including a missing
   record, syncs the protected-store directory
   (`crates/foks-client/src/protected_store.rs:376`). The temporary-file scan's
   250-ms allowance does not bound those preceding operations. Use resumable
   maintenance batches and preserve quota debt until durable erasure completes.
   Test receipt latency under slow filesystem operations and eventual cleanup
   after interruption. Count bounds alone are not a wall-clock guarantee.

6. **Recovery usability follow-up: connect source failures to durable inventory.**
   Source-read and source-size-change errors return before the upload handle is
   appended to the message (`apps/desktop/src-tauri/src/commands/vault.rs:806`,
   `:821`, `:828`). The intent remains discoverable, but users receive no direct
   recovery link, and repeated failures can occupy the 16 default pending slots.
   Show the handle and destination in upload inventory, distinguish a new-source
   action from source-free resume, and connect completion to an explicit catalog
   refresh. Preserve the current narrow reconciliation exception and ordinary
   stale-mutation protection (`apps/desktop/src-tauri/src/commands/context.rs:1937`).

## Deployment and protocol boundary

The proof retains a native TCP/mTLS frontend and uses a private HTTP bridge to
celld SQL. It does not make the FOKS wire protocol an HTTP Worker API. Production
routing, per-tenant authentication, capability/key isolation, provider credentials,
secret distribution, certificate renewal and control-plane authorization need a
separate review. Existing Go-client compatibility must remain a release gate.
Multiplex unchanged TCP/mTLS by tenant hostname/SNI and verified HostID rather
than assigning one public port per tenant. The proof uses separate ports and
controller-held addresses, so frontdoor routing and its failover convergence are
not qualified by these runs.

## Independent review

Storage, recovery and client subagents reviewed this document. Their additional findings
(irreducible uncertain-attempt admission, SQL metadata growth, startup scan cost,
collector fairness, distinct native/celld fences, cohosted adapter RSS and client
full-sync amplification) are incorporated above. The proof harness also received
an independent audit of timing, ACK checks, artifact removal, resumed-owner fencing
and residency claims. Review any subsequent implementation fixes separately.

## Failed SQL rollback and celld handoff follow-up (2026-10-16)

Retained actual-directory fixture evidence exposes a platform follow-up before
any capacity/recovery qualification. In `/private/tmp/foks-directory-celld-a`,
six cases aborted their seed transaction on a denied fixture `user_version`
assignment; two16MiB cases failed binding a record above the pinned2,200,000-byte
SQL value/row limit. During teardown, four cells logged12 `stop_runtime_retry`
events each with `create handoff snapshot: ltx file corrupted`; later handoff
snapshots and durability barriers succeeded, followed by a node lease watchdog
self-fence at2026-10-16T08:15:24.358843Z. These logs alone establish neither data
loss nor successful failure recovery.

The affected cells correlate, by the controller's sequential fresh-cell startup
order, with `slice_minus`, `slice_exact`, `slice_plus` and `configured_limit`
(roughly512KiB payloads). The `small`, `over64k` and both16MiB bind-rejected cells
had no such retries. Exact cell IDs, first/last retry timestamps and barrier lines
are retained in `/private/tmp/foks-directory-celld-handoff-correlation.json`.
After removing only the forbidden fixture pragma, run-b committed the six
representable seeds and failed later on a strict schema assertion; the same two
large-record binds still failed. Run-b logged no corruption/handoff retry or
self-fence for any of its eight cells, and recorded all eight handoff barriers.
No kernel case had passed yet in either run, so this comparison distinguishes
seed rollback from committed-seed assertion failure, not successful versus failed
production reads. It suggests a rollback/teardown reproduction candidate, not a
proven causal diagnosis.

Pinned source anchors: celld0.6.2 `crates/celld/actor.rs` around4002
(`stop_runtime_retry`) and4141 (`node_lease_watchdog_fence`),
`crates/celld/ltx_repl.rs` around3447 (`eviction_durability_barrier`), and
`crates/ltx/src/error.rs` around36/242 (`ltx file corrupted`). Reproduce controlled
successful/failed SQL transactions across page-size boundaries, capture authoritative
artifacts and ownership transitions, and verify restart contents/ACK behavior
before interpreting the warning or patching the platform. Keep runtime/controller
shutdown effects distinct from host/AZ-loss guarantees. No platform patch is
included in the current work.

Follow-up observation from directory run-d: after the exact pinned platform-schema
fixture correction, six kernel cases passed and the two 16 MiB records still
failed binding. All eight cells recorded final durability barriers; there were
zero `stop_runtime_retry`, LTX-corruption or watchdog self-fence messages in
`/private/tmp/foks-directory-celld-d/celld.log`. This strengthens the distinction
from run-a's seed-transaction rollback failures, but does not resolve their cause
or qualify host/AZ recovery. Retain the rollback/teardown reproduction candidate.
