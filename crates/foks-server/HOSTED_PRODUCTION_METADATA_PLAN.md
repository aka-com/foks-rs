# Production metadata, deadlines, and receipt retirement

Status: implementation authorized; concrete phase gates and review log are in
[the execution plan](HOSTED_PRODUCTION_EXECUTION_PLAN.md).
Prepared against `9c19e753`, 2026-10-15. Keep this Markdown unstaged.

## Outcome and constraints

Replace the celld proof bridge with a production authoritative metadata adapter;
remove the client's lifetime upload-receipt ceiling safely; enforce one request
budget; qualify recovery and capacity without whole-state reconstruction per RPC.
S3 object deletion remains report-only until a separate termination-proof gate
passes. Metadata production readiness does not depend on enabling remote deletion,
but retained/uncertain object growth must fit an explicit storage/admission budget.

Preserve Go FOKS network messages, cryptographic encodings and historical verifier
rules. Internal adapter messages, local IPC and local/database schemas may change
with explicit version handling. Retain the recently added rename authorization,
unlock publication gate, selected entry identity checks and KEX admission budget.
Target personal servers and organizations of at most 64 members; organization
qualification initially covers credentials and small text, with files separately
qualified for personal servers.

Assume strongly consistent managed S3 is durable and available. Test our publication,
ownership and recovery protocol, not S3's replication implementation. This assumption
does not make an unreplicated SQLite commit durable or cancel an accepted PUT.
Use celld bucket durability for qualification, pinned to an audited source revision;
fleet-only acknowledgment is a separately named experiment, not an equivalent mode.

The target is successful ordinary requests within 15 seconds under the declared
load/fault envelope, and successful recovery within 15 seconds after host/AZ loss.
Bounded retryable failure is required when success is impossible, but does not pass
the recovery test. With authoritative metadata and keys surviving in S3, target
zero acknowledged-write loss. The user's allowance of at most five minutes of lost
writes is a disaster-recovery fallback requiring explicit rewind qualification,
not permission to acknowledge nondurable writes. Whole-file completion and existing
long-poll waits require separate budgets; do not silently shorten their wire semantics.

## Current evidence and gaps

| Boundary | Inspected implementation | Required change |
| --- | --- | --- |
| Native writer | [writer.rs](src/writer.rs) executes closures against local SQLite, under a host-local lock. | Hosted writes and maintenance must use durable cell authority; no local fallback. |
| Proof authority | [authority.mjs](../../tools/foks-celld-proof/authority.mjs) checks adapter ownership/revision, applies four allowed base-table changes and inserts a receipt atomically. It scans snapshots and caps receipts at 10,000. | Persistent targeted access, complete mutation coverage and bounded safe receipt retirement. |
| Proof output | [worker.mjs](../../tools/foks-celld-proof/worker.mjs) calls `storage.sync()`; [bridge.rs](../foks-server-testkit/src/celld_proof/bridge.rs) reconstructs disposable SQLite state and retains candidate output. | Recoverable outcomes independent of that process and audited celld durability/output fencing. |
| Upload ledger | [uploads.rs](../foks-client-db/src/repositories/uploads.rs), `record_upload_intent`, rejects the 1,025th retained identity. Cancellation and explicitly identified inline uploads count. | Versioned identities with irreversible rejection after retirement, plus rolling resource limits. |
| Existing retirement model | [adapter.rs](../foks-client-db/src/repositories/adapter.rs) and [clock.rs](../foks-client-db/src/repositories/adapter/clock.rs) preserve live rows, validate time and commit rejection watermarks. | Reuse those semantics for uploads; include external rollback-checkpoint publication. |
| Deadlines | [config.rs](src/config.rs), [session.rs](src/net/session.rs), agent streaming and client transport use separate phase budgets. | One propagated ordinary-operation budget and explicit bulk/poll classes. |
| S3 collection | [chunks.rs](src/chunks.rs) configures up to two retries. [chunk_collection.rs](src/chunk_collection.rs) permits deletion only for qualified local storage. | Persist publication-proof provenance; keep uncertain remote attempts charged and undeletable. |

The upload limit and proof transaction-receipt limit are different ledgers with
different identities and retirement rules. Do not solve one by pruning the other.
See [the existing review](HOSTED_NEXT_REVIEW.md) and
[local proof results](HOSTED_CELLD_PROOF_RESULTS.md). Local process-loss passes
and sixteen-tenant timing failures are evidence, not host/AZ or fleet guarantees.

## Recommended implementation sequence

1. **Inline contracts and observability:** settle necessary authority, outcome, deadline
   and retirement contracts inside the following slices. No separate architecture
   decision phase blocks upload work; the metadata feasibility gate precedes its port.
2. **Client upload retirement:** remove the immediate broad-rollout blocker without
   depending on the production adapter. Ship schema/ledger, cleanup, and local API
   integration as separate reviewed changes.
3. **Production metadata vertical slice:** build the persistent repository boundary
   and qualify one complete authenticated KV mutation/read/recovery path.
4. **Complete authoritative coverage and receipt lifecycle:** port all supported
   operations, maintenance, keys, backup/restore and internal transaction receipts.
5. **End-to-end deadline integration:** carry the contract introduced in step 1
   through every layer, then qualify combined recovery and capacity.
6. **Remote object publication proof:** instrument and qualify PUT termination.
   Enable deletion only in a later, separate change if that proof succeeds.

Each implementation slice requires direct review and an independent subagent review
before testing and its own implementation-only commit. Fixes found by validation
receive narrow rereview and affected-test reruns. Markdown stays unstaged. This
plan is documentation; implementation and evidence are tracked in the execution plan.

## 1. Authority and repository design

### Selected architecture

Keep the native TCP/mTLS frontend and Rust protocol/cryptographic validation.
Celld's persistent per-tenant SQLite state is the hosted metadata authority. Replace
disposable full-database projections with bounded, typed repository queries and
transaction commands executed inside the cell. Local SQLite remains the standalone
backend; hosted local caches are optional and never an independent authority.

This is a repository-boundary refactor, not a transparent replacement for arbitrary
`FnOnce(&mut Database)` closures. Define versioned domain commands and deployment-pinned
statement IDs with typed parameters/results. Clients cannot submit SQL or DDL.
Keep transaction execution in the cell; do not hold an open SQLite transaction
across HTTP calls. Avoid a generic client-controlled SQL interpreter and avoid
translating existing SQL by textual rewriting.

Start with a conservative tenant-wide committed revision. Every targeted read in
one validation attempt checks the same `(installation generation, adapter epoch,
owner, revision, schema version)` atomically with its query. A revision change
invalidates the entire candidate. This also covers negative reads and phantoms;
a partial read-set or cached path vector must not authorize an unseen insertion.
Batch queries needed together to bound round trips. Only later replace this global
revision with finer conflict tracking if measurements justify a separate proof.
Initially serialize mutation preparation/publication per tenant through a bounded,
fair scheduler shared with maintenance. Do not hold a SQL transaction while waiting
on native validation or storage. Contended reads/revision conflicts receive bounded
retries within the original deadline, with measured starvation/backpressure behavior.

Rust validates authenticated material and prepares the typed command against that
revision. One short cell SQL transaction checks current owner, revision, current
credential/role and all command preconditions; applies the complete mutation;
executes normal constraints/triggers; increments the revision; and stores the
recoverable result/receipt. On any mismatch, roll back the entire command. Native
and cell implementations must share reviewed semantics and differential fixtures;
duplicating two unchecked SQL implementations is not an acceptable outcome.

The first vertical slice must prove that transaction-dependent return values,
trigger outputs, affected-row assertions and deferred foreign keys can be expressed
without reconstructing the database. If that spike fails, stop and revise this
boundary before porting more operations; do not broaden the proof allowlist and
call it production. Preserve exact i64, BLOB, TEXT and NULL representations.

### Atomic fence and durable output

- Separate celld node ownership from the native adapter ownership epoch. A private
  control-plane CAS advances the latter; ordinary frontends cannot self-promote.
  Bind every operation to tenant/HostID, installation generation, schema, namespace
  and key generation. Reject cross-tenant, stale-owner and wrong-installation calls.
- Atomically check the fence with mutation publication, never just before queuing.
  All counters, Merkle/chain heads, authorization changes, object attempts, maintenance
  effects and transaction outcomes follow this boundary. No background bypass.
- Gate output on a matching durable outcome after the audited celld sync/output
  contract succeeds. A local SQL commit, HTTP acceptance, output digest alone or
  in-memory pending response is not a recoverable acknowledgment. Store enough
  versioned result data to rebuild the response after process loss, including
  necessary generated/signed material. Do not persist transport secrets or
  unbounded response bodies; large reads use immutable references and bounded pages.
- Treat a lost commit reply as unknown, not failed. The successor queries the
  durable attempt identity under its current authority; it can recover a prior
  outcome without allowing the old owner to submit new writes. A missing/retired
  attempt never grants permission to replay under a fresh identity.
- Reads and data-bearing errors need a current authoritative observation too.
  Define read linearization at the fenced, durability-gated observation. A response
  already proved before promotion may arrive later if its process was paused;
  no final check can atomically suppress every later socket write. Do not promise
  that stronger property. Recheck current credentials before releasing sensitive
  replayed results where required by existing RPC semantics.
- S3 file PUTs happen outside SQL transactions. Journal their reservation/attempt
  first; publish metadata references only after validated object acknowledgment.
  A durable metadata acknowledgment must recover all required object references
  and exact key generations. Failed metadata publication can leave a charged orphan.

### Persistent access and complete coverage

Remove `/snapshot`, whole-table sorting/hashing and disposable database creation
from request handling. Fetch rows/proofs/pages needed by the operation, with byte,
row, query-count and execution bounds. Use explicit transaction/log/checkpoint
positions for authority, not a per-request full-state digest. Full consistency
audits remain offline qualification tools. Celld owns its storage recovery; native
activation fetches only bounded configuration/key metadata and needed working data.
Measure any celld checkpoint/tail hydration rather than assuming it is constant time.

Inventory and port identity/signup/rename/revocation, host and device keys,
certificates, Merkle publication, teams/invitations, directories/roots/locks,
small/large KV, quotas/object ledgers, SSO/admin, realtime state, maintenance and
backup/restore. Include writes hidden in reads, timestamps and caches. A declared
deployment profile may disable a feature explicitly; it must not silently route
unsupported operations into local authoritative SQLite. Porting excludes generic
historical verifier changes and retains the new transactional rename guards.

Version migrations as bounded maintenance jobs before failover. Fence activation
during incompatible upgrades; do not let mixed writers interpret different schemas.
Bootstrap/import once under a control capability, verify canonical schema and
complete state/key/namespace binding, then switch routing atomically. Backout after
accepted cell writes requires an authoritative export and fenced handoff, not an
old local database. Keep backup pins and replay/log retention until all consumers
and supported restore points have advanced.

Do not restore ownership epochs or allocation/rejection watermarks from an older
application snapshot. Restore payload state under retained, non-rolled-back control
authority and advance the installation generation before reopening admission. If
that control authority was also lost, a new namespace/generation is safe only after
the former authority is irreversibly fenced; changing routing alone is insufficient.
Otherwise remain unavailable. Key recovery must include the wrapping/KMS authority
outside the encrypted bundle, exact generation manifests and certificate renewal.

### Internal transaction receipts

Replace the proof's 10,000 lifetime attempts with bounded live intents and retained
terminal outcomes. Use a durable per-installation/per-adapter-epoch sequence allocator;
reserve the intent before execution, recording canonical input and response-version
bindings. Persist the allocation high-water mark even after terminal rows are removed.
Only an existing matching live intent may commit. Missing allocated identities are
retired/unknown, never new; closed epochs cannot admit new commits. Caller-controlled
sequence gaps, integer overflow, changed payloads and cross-epoch reuse fail closed.

Lost reservation replies may leave unused intents; no side effect may precede a
confirmed reservation and its durable execution-state transition. A delayed caller
cannot execute an intent already retired. Retire only definite terminal/unused
intents whose replay contract has ended; unresolved external work or publication
retains its row and admission charge. Out-of-order completion must not erase lower
live sequences. Preserve bounded recovery lookup under the successor epoch.

Specify receipt retention, maximum live intents, terminal rows/bytes and maintenance
budgets before enabling this backend. Preserve existing remote FOKS idempotency
semantics independently: internal receipt expiry must not turn a client retry into
a second mutation. Test exhaustion and recoverable backpressure, not just pruning.
Do not claim cross-reconnection exactly-once execution for a remote RPC that has
no stable replay identity in the unchanged FOKS protocol. Preserve its existing
reconciliation/error semantics instead of guessing identity from equal payloads.

## 2. Safe client upload receipt retirement

### Handle and migration contract

Introduce a separately typed, versioned local upload handle with the hard-state
database ID (profile generation), issuance time and 128 random bits, reusing the existing adapter clock/rejection-watermark semantics.
Reuse validated-clock code with domain-separated upload ledgers/rejection state;
do not silently change the existing adapter handle's retention/admission contract.
Keep its full canonical identity; do not truncate its hash into the existing
16-byte protected-record ID. Allocate a distinct internal ID once and retain its
mapping while live. A new timestamp is a new operation with newly generated private
upload material, not reuse of a prior file ID/seed.

An unseen handle must match the current hard-state database ID; reset or profile
recreation cannot resurrect handles from the deleted generation. Preserve exact known
handles across supported imports. Use canonical `u1-<32hex-generation>-<16hex-seconds>-<32hex-random>`
encoding, with a distinct internal protected-record ID.

Bind registration to profile/HostID/user/party and canonical immutable intent
(destination, selected dirent/version, roles and declared source constraints).
Replay of registration returns the existing matching state before new-admission
checks; changed intent rejects. Preserve registration before source reads, the
one-time source-consumption transition, and source-free resume after preparation.
Lost registration/ready replies must not cause fresh encryption under an old identity.

Migrate existing rows in place as legacy handles. Existing prepared live legacy operations
remain resumable/cancelable, and existing terminal receipts remain readable. Legacy
awaiting-source rows lack immutable intent: cancel/restart with a new handle rather
than silently binding new source to historical unbound intent.
Reject all new unseen legacy registrations. The at-most-1,024 legacy tombstones
can remain as a fixed migration cost outside the new rolling budget; this avoids
guessing issuance times for random historical IDs. Never delete a live legacy row
to make room. New binaries reject incompatible local schema/IPC; allocate a new
IPC version above current 35, without changing FOKS network IDs or `SubmissionHandle`
semantics used by existing adapters.

### Retirement ordering

1. Existing matching rows take precedence over age: pending, blocked, source-dependent
   and ambiguous uploads retain their recovery material. An untrusted clock blocks
   new issuance/pruning, not evidence recording or safe recovery of existing rows.
2. Use validated wall/monotonic samples and persisted clock state. For new handles,
   start with the adapter's 24-hour admission age and five-minute future-skew rule;
   retain terminal outcomes for 30 days. Terminal retention starts at verified
   terminalization, not handle issuance, so a long-running upload gets a full window.
3. Finish namespace/adapter outcome recording and upload terminal state before
   cleanup. Preserve ambiguity and ancillary committed work. `inline-untracked`
   remains explicitly untracked publication, not a success receipt.
4. Move protected-record erasure into resumable, bounded maintenance. Record cleanup
   progress; retain spool-byte debt until records, temporary files and directory
   durability are accounted for. Completion/status must not synchronously perform
   up to 257 file removals/fsyncs. Do not release a reservation while an uncancelled
   cleanup operation still owns work or buffers.
5. Only after all durable owners and cleanup debt are resolved, atomically delete
   eligible terminal rows and advance `reject_issued_before` beyond their issuance
   times. Future use of a deleted handle must return retired/expired, never create
   a new intent. Advancing the watermark must not invalidate still-live older rows.
6. Include clock/watermark/retirement state in hard-state revision tracking and
   external rollback checkpoints, including transactions that return an admission
   error after persisting rejection. Publish the external checkpoint before exposing
   successful retirement/admission; checkpoint failure must fail closed and recover
   consistently after restart. A rolled-back DB must not resurrect retired identities.

Replace the lifetime cap with separate live-count, retained-terminal row/byte and
spool-byte limits. Initial proposed terminal limits are 8,192 rows and 16 MiB per
profile, plus the fixed legacy set; keep current pending/spool defaults unless
capacity tests justify changes. Reserve eventual terminal capacity at registration
so successful remote publication cannot fail to record evidence because another
operation filled the terminal budget. These are rolling limits, not unlimited burst
capacity: 30-day retention requires capacity at least admitted terminal rate times
retention plus headroom. Expose each budget and cleanup/clock blockage separately.
Qualify higher rates before raising defaults; never evict a still-required receipt.

Keep historical success distinct from current server presence. A local finalized
receipt proves prior publication, not survival after a rewind. Preserve a compact
protected destination/dirent/version binding if offering current-presence checks;
perform those checks remotely and label them separately. Never clear rollback
anchors or republish automatically to make a recovery check pass.
A retired local receipt means its historical evidence is no longer retained; it
does not prove that the remote file is absent or authorize a replacement upload.

Profile restore/cloning is a release gate, not solved by a local watermark alone.
Use the existing external checkpoint/ownership mechanism to reject stale or concurrent
copies; test both supported native and private-file credential backends. PrivateFile
currently skips checkpoint publication: introduce a durable authenticated root-level
checkpoint authority outside individual profiles, with an incompatible client-state
envelope version and fail-closed missing/corrupt authority handling. Preserve existing
Native-only archive portability; whole-root plus authority co-rollback is unqualified. If all local
state and its checkpoint can be rolled back/cloned together, do not claim detection:
require a non-rolled-back authority or a supported fresh-profile recovery procedure
that disables old registration. Do not silently reset the watermark.

## 3. End-to-end deadlines and resource ownership

Introduce an operation context at earliest controlled ingress, covering routing,
admission, activation, TLS, frame reading, repository validation, queue wait,
storage, retry, durable outcome lookup and socket transmission. Bound connection
setup separately where the server cannot yet identify an operation. The official
client starts its own budget before connect, so ingress-only timing is not mislabeled
as client end-to-end latency.

Within one process propagate one absolute monotonic deadline. Across processes
use an authenticated absolute UTC deadline with a qualified maximum clock-error
bound, subtract the conservative skew allowance at receipt, and anchor the resulting
remaining budget once to the receiver's monotonic clock. Forwarders cannot increase
the original deadline. Reject an expired budget or untrusted clock; qualify clock
jumps and suspension. Do not transmit raw `Instant` values or assume host monotonic
clocks share an epoch. Fix the clock-error bound during the contract slice; if it
cannot be established, this cross-host deadline design does not qualify.
Clamp caller-supplied budgets. A retry consumes the original deadline, including
backoff, reconnect, activation and reconciliation; it does not receive 15 new seconds.
This envelope is private/internal. The unchanged public FOKS protocol carries no
new deadline field: client and server enforce their own ingress budgets, and the
client's original monotonic deadline remains the end-to-end responsiveness measure.

Before a queued write begins, check remaining budget and current authorization.
Reject work that cannot be admitted, rather than filling unbounded wait queues.
An executing mutation can become durable after the caller times out: preserve its
identity/outcome and reconcile, never claim cancellation undid it. Resource permits,
buffers, DB leases and external-attempt records follow the work's real lifetime.
Use bounded quarantine/drain capacity for uninterruptible I/O; do not create an
unbounded replacement worker on every timeout. Keep object transfers outside DB
reader leases and retain response-memory permits through socket transmission.

Define three explicit classes: ordinary RPCs with the 15-second total target;
existing long polls with their protocol wait plus bounded admission/transmission;
durable bulk jobs with short acceptance/status calls, bounded individual RPCs and
a separately declared progress/throughput policy. Existing KEX limits still apply.
A large transfer cannot promise 15-second completion, and replay from chunk zero
can starve under too-short attempt budgets. Prefer durable background scheduling
over repeated foreground restarts; preserve one exact prepared upload and expose
blocked/unknown outcomes. No remote resumable-cursor extension is assumed.

Recovery must budget detection, celld lease/fence, promotion, key availability,
storage activation, frontdoor convergence, authentication and successful read/write.
Treat a 3-second lease as a measured candidate only: include renewals, scheduling
delays and false self-fencing. Phase budgets are measured allocations within 15
seconds, not independent timeouts that sum to more. Do not classify a bounded 503
or closed connection as successful failover.

## 4. S3 publication proof and reclamation gate

Keep remote collection report-only and conservatively charged throughout steps
1–5. A GET/LIST observation, object age, SDK timeout, task abort, owner promotion,
credential revocation or successful later retry does not prove an earlier PUT
cannot still complete. Killing the local worker is not proof either.

Add durable, versioned publication provenance to each object incarnation and each
physical PUT subattempt. Disable hidden mutation retries for a new qualified path,
or surface every subattempt in the ledger before dispatch. Preserve immutable,
non-reused keys and explicit unknown outcomes. Single-attempt configuration alone
does not settle a lost response; only provider-qualified terminal evidence can
make that particular attempt eligible for collection. Legacy retrying successes
retain their unqualified provenance; new settings do not retroactively certify them.

Before any future DELETE, prove atomically in authoritative metadata: current
collector ownership, zero live/history/backup/read pins, no unfinished publication
or subattempt, and an exclusive deletion intent for the exact incarnation. Prevent
new reference acquisition after that intent. Then execute bounded deletion outside
SQL and finalize accounting by exact intent/generation after confirmed outcome.
Ambiguous deletion retains its charge and intent. Metadata fencing alone cannot
prevent a previously admitted S3 PUT from resurrecting an object.

Qualification must hold an early PUT at the provider emulator, allow a later retry
to finish, remove references, run collection, and finally release the early PUT.
Collection must remain inhibited until every attempt has terminal evidence. Also
cover a single PUT with a lost reply, process death, stale-owner return, delayed
DELETE, metadata rollback, restored backup pins, and legacy provenance. Use real
provider semantics as a later gate; a fake's cancellation behavior proves nothing
about S3. If termination cannot be established, retain report-only mode and explicit
unresolved-capacity diagnostics. A safe operator drain may leave objects charged;
never offer an age-based force-clear as a correctness-preserving repair.

Specifically exercise exhaustion of the current 128 unresolved-attempt allowance
under repeated transient failures, even when byte quota is free. Positive GET can
settle presence without proving the outstanding PUT ended. Account for this outage
budget and terminal attempt/tombstone metadata separately from upload receipts;
plan a proved drain and bounded inventory/maintenance work before general file use.

## Validation and release gates

| Gate | Required evidence |
| --- | --- |
| Repository equivalence | Differential native/cell tests for exact SQL types, constraints, triggers, authorization, negative reads, all stateful routes and maintenance; fault within each multi-table command leaves all-or-nothing state. |
| Durable ownership | Two frontend candidates and separate celld owners; pause before/after fence, commit, sync and reply; concurrent promotion and stale return; changed-input replay rejection; no divergent acknowledged successors. |
| Lost outcome | Actual process death after authoritative commit but before native reply; successor recovers identical outcome without the original process, candidate response or local DB. |
| Receipt lifecycle | More than 1,024 completed uploads over multiple retention cycles, 1,025th preparation after safe retirement, lost registration replies, source-once behavior, live older handles, restart, clock jumps, checkpoint failure, clone/restore and replay after deletion. Bound steady-state rows/bytes and maintenance work. |
| Internal receipt lifecycle | Exceed the proof's 10,000 attempts with bounded retained rows, allocation/reply loss, delayed unused intents, out-of-order completion, promotion, retained unknown work and replay after pruning. |
| Deadlines | Deterministic delays at every queue/I/O/commit/response boundary; slow readers/producers and storage; zero timeout resets, bounded retained resources and precise known/unknown outcomes. |
| End-to-end compatibility | Rust and pinned Go clients against native and hosted backends, including signup, rename, membership, encrypted KV, retry and key rotation. Local IPC/schema downgrade rejection separately. |
| Recovery | External fsynced ACK ledger; destroy both native and celld owner processes/local artifacts together; normal frontdoor routing; recover exact metadata/keys/objects, read every retained ACK and complete a new write/read within 15 seconds per affected tenant. |
| Rewind/DR | Restore keys/metadata/object bindings and retained pins at a declared consistent point; check client rollback anchors, insertion-loss and overwrite-loss with historical receipts. Safe refusal is recorded separately from successful recovery. Never erase pins to pass. |
| Remote GC | All-attempt terminal evidence and late-PUT adversarial tests; until qualified, assert DELETE is never called and uncertain bytes/attempts remain charged. |

Run local qualification first against pinned celld and the S3 emulator, preserving
the independent clock-discontinuity guard. Repeat both native-adapter and celld-owner
faults, including their combined loss and frontdoor reconnection. Preserve raw
failures; do not extend deadlines or pre-position unreported surviving state.
No AWS environment exists yet: physical host/AZ, real S3 latency, KMS/certificate
recovery, routing convergence and regional DR remain deployment qualification gates.

Capacity matrix: 1, 4 and 16 simultaneous recoveries first; then 64 if the smaller
matrix passes. Measure idle/resident/evicted tenants separately at 128, 1,024 and
larger supported populations, stepping toward 10,000 without extrapolating tiny runs.
Vary live namespace size, overwritten history, terminal receipt history, object
attempt debt, cache warmth, write contention and small-text versus personal-file
traffic. Include 64-member organization read/write bursts. Report full-process RSS,
CPU, threads, FDs, page/cache bytes, admission/recovery queues, SQL rows visited,
bridge bytes/round trips, S3 calls, receipt growth, cleanup debt and latency tails/max.

For fixed-size keyed operations, doubling unrelated history must not cause linear
snapshot traffic/reconstruction work. Explicitly paginated listing/full-sync work
is measured separately; removing the bridge snapshot does not remove client full-sync
amplification. Reserve node-wide recovery headroom, share runtimes and evict idle
tenant working sets; do not assume one permanent native process/thread set per tenant.
Publish a tested operating envelope and backpressure policy, not an unconditional
15-second guarantee across arbitrary outages or overload.

## Plan review record

Two direct self-review passes were performed before presenting this plan. These
were plan reviews against the source and existing qualification results, not tests
of a new implementation or independent subagent approvals.

1. **Safety and compatibility review:** separated client and authority receipts;
   made current-transaction ownership and authorization explicit; required durable
   recoverable outcomes rather than hashes; preserved overlapping-response semantics;
   kept legacy IDs distinct and live rows ahead of expiry; retained S3 uncertainty.
   Correction incorporated: a single-attempt PUT setting and local process death
   are insufficient termination proofs when the provider response is lost.
   Also required restore to retain non-rolled-back ownership/allocation authority
   and prevented claims of exactly-once replay beyond existing FOKS semantics.
2. **Feasibility and rollout review:** made the repository refactor and vertical-slice
   stop gate explicit; excluded whole-state work from request handling without
   claiming storage activation is constant time; separated bulk/poll budgets from
   ordinary requests; included checkpoint/clone limitations, mixed-version migrations,
   both receipt caps and combined frontend/cell failures. Correction incorporated:
   rolling receipt capacity must cover retained arrival rate, and local qualification
   cannot certify the physical host/AZ target.
   Specified a private, skew-bounded cross-process deadline envelope without adding
   a FOKS wire field, and included unresolved-PUT admission exhaustion explicitly.

Remaining implementation decisions are the measured deadline phase allocation,
final per-operation transaction/query bounds, authoritative receipt-retention
constants and the audited celld revision/API contract. They must be fixed and reviewed
in the contracts/vertical-slice work before deployment. Remote PUT termination and
fully co-rolled-back profile detection remain explicit qualification gates, not
assumed capabilities. No production code or schema was changed for this plan.
