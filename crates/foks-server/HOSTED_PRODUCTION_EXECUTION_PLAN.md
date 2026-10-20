# Production metadata execution and verification ledger

Prepared against `9c19e753`, 2026-10-15. Implementation authorized. Keep all Markdown unstaged.

The [design](HOSTED_PRODUCTION_METADATA_PLAN.md) remains the semantic contract.
Resolve decisions within each slice, rather than wait for a separate architecture
phase. Preserve Go FOKS network/crypto compatibility; version local IPC and schema.
Managed strongly consistent S3 is the durable authority assumption, not a claim
that a local SQLite commit or canceled PUT is durable or terminated.

## Commit groups and acceptance gates

### U: Generation-bound upload receipts and protected recovery

Group schema/API changes that must ship together; do not commit a new ledger while
old entry points can create unseen raw IDs or bypass its source claim.

1. Add canonical `UploadHandle`: legacy 32 hex lookup identity, or
   `u1-<32hex-database-id>-<16hex-issued-seconds>-<32hex-random>` (85 characters).
   Full handle identity is distinct from the immutable internal protected-record ID.
   An unseen handle must match the current hard-state database ID. Known exact rows
   take precedence over admission age, pressure and untrusted clocks.
2. Hard schema 41→42 adds handle mappings, scope/immutable intent, validated clock
   and rejection floors, retained child ownership, and cleanup cursors. Ensure the
   older 40 migration explicitly reaches 41 before running 42. Include every new
   table in hard revision tracking, checkpoint inventory and applicable exports.
3. Legacy prepared operations remain resumable/cancelable and terminal rows readable;
   reject unseen legacy IDs. Legacy awaiting-source operations require cancel/restart
   with a new handle because their immutable intent cannot be reconstructed safely.
   Never prune the fixed legacy set. Existing generic orphan children are not guessed
   back into ownership; retain new child associations across retry/cancel.
4. Bind destination, selected identity/version, overwrite/create policy, roles,
   mkdir behavior and source length before reading source. Register then atomically
   claim once. Publish external checkpoint under the existing profile lock before
   sending stream readiness or consuming any bytes. Preserve source-free resume.
5. IPC 35→36 and all agent/client/CLI/desktop consumers carry full handles. A helper
   supplies the validated issuance context; caller mints random bits before registration
   so a lost reply retains the same handle. No truncation into the internal ID.
   Paged summary listing (128) must not hydrate every chunk manifest.
6. Introduce PrivateFile's authenticated root-level checkpoint authority outside
   individual profiles. Client-state envelope 3→4 marks it mandatory: missing/corrupt
   authority fails closed. Serialize root updates, durably write authority before
   publishing the new envelope, and make interrupted migration resumable. Existing
   Migration holds exclusive state-root lifetime reservation to fence old binaries;
   only genuine v3 upgrade/new state may initialize authority, with an external
   recovery marker. Native keeps its existing v3 envelope/archive compatibility.
   Generalize the retained-lock preeffect checkpoint barrier to issuance, registration,
   source claim and any callback escaping the checked session. Existing
   native checkpoint semantics remain intact. Profile/DB-only rollback is detected;
   whole-root plus anchor co-rollback and unsupported clones remain unqualified.
   Preserve Native-only portable archives. Check reset, delete, recreate and import.
7. New live registrations reserve eventual terminal row/byte capacity. Defaults:
   existing live/spool budgets, 8,192 rolling terminal slots/16 MiB plus fixed legacy
   rows, 30-day terminal retention, 24-hour new admission age/5-minute future skew.
   Expose upload-clock preview/CAS reanchor that preserves the rejection floor,
   so a long-offline restart has an explicit repair path.
   Terminal evidence is recorded even with an untrusted clock; retention starts only
   from validated terminal time. Do not release bytes before durable erasure.

Ownership: storage agent owns proto handle and client-db schema/repository/clock;
quota agent owns client/core upload use, app checkpoint authority, IPC and consumers.
Coordinate shared APIs in messages; root integrates and commits. No concurrent Cargo.

Acceptance: genuine schema41 and 40 migration fixtures; old binaries reject schema/IPC;
legacy recovery; changed intent/scope/generation; lost registration/ready replies;
source-once barrier and checkpoint-failure zero-read tests; >1,024 new completions;
rolling row/byte/live pressure; bad clock and committed-error watermark; profile-only
rollback under both credential backends; reset/recreate handles; source-free resume;
existing upload/adapter/portability/IPC/native desktop and renderer regressions.

### UC: Bounded cleanup and receipt retirement

Remove synchronous terminal erasure. Persist cleanup phase/cursor and perform bounded
maintenance under the existing profile ownership/queue: at most eight removals per
pass and a cooperative 50 ms budget (not a hard filesystem latency guarantee).
Unlink/fsync before cursor advancement; include unjournaled next chunk, manifest and
temporary files. A sweep spanning unlocked passes must be bound to the protected-write
generation and restart on intervening writes; inode identity alone is insufficient.
Hold the entire spool debt until complete authoritative sweep and final directory sync. No detached
worker may outlive ownership or release its charge early.

Track every namespace child independently of the current retry pointer. Compact only
definite terminal children with resolved owners and erased protected material. Adapter
outcome ambiguity blocks retirement. In one transaction prune eligible new terminal
receipts and advance the irreversible rejection floor beyond deleted issuance times;
older live receipts still take lookup precedence. Publish external checkpoint before
exposing admission/pruning. Pagination and repeated maintenance remain bounded.

Acceptance: deterministic failure after every erase/cursor/checkpoint boundary;
restart convergence, no early released charge, unresolved owner blocks prune,
out-of-order expiry, exact retired identity cannot register, known old live survives,
repeated >1,024 lifetime cycles and terminal inventory without manifest hydration.

### M0: Shared transaction kernel feasibility

Use one Rust domain transaction kernel compiled native and Wasm, not a second JS
implementation of authorization/encoding. A pinned typed statement catalog and bounded
parameters/results execute synchronously in celld `storage.transactionSync`; await
`storage.sync()` only after commit. No arbitrary SQL or closure over HTTP. Native uses
the same kernel against its already-open transaction. Begin with real PutDirents.

Gate before broad port: actual initialized schema; SQLite triggers/deferred foreign
keys/affected rows; exact replay including creation time and error precedence; exact
signed i64/BLOB/NULL. Celld 0.6.2 SQL currently serializes binds through JSON and returns
large integers as JS Number: use explicit exact decimal text/CAST codecs, never Number
round trips. Bound allocation before copies on both host and Wasm sides. Prove Wasm
host callback/build feasibility and execution interruption/bounded statement plans.
A domain error, host error, trap or deferred COMMIT failure must roll back every effect;
returning an error value from a transaction callback must never commit it. Probe/guard
cell type and length before selecting payload because celld materializes results before
Rust can enforce bounds. Pin Wasm memory maximum and check every copy boundary.
If infeasible, revise this boundary; do not extend the disposable snapshot bridge.

Recovery agent owns kernel/Wasm host and differential fixtures; root and storage agent
join transaction extraction/server integration after U. Inventory dependencies first.

### M1/M2: Authoritative KV slice, then complete coverage

Introduce deadline context with the first envelope. Every query checks installation,
tenant, namespace, key/schema generation, native adapter owner epoch and revision
atomically. Mutation transaction also checks current authorization and all domain
preconditions, applies effects, advances revision and records recoverable outcome.
Celld ownership and native adapter ownership are distinct fences.

Durably reserve a per-epoch intent sequence plus canonical input/version/auth binding
before external effects. Only matching live intents can commit. Retain allocation
high-water after payload retirement; missing allocated identities never execute again.
Keep bounded live/terminal budgets, outcome resolution under successor ownership and
retirement that preserves unknown external work. An output hash alone is insufficient.
Sync gates replies; a paused previously proved reply is an overlapping observation.

M1 covers complete authenticated KV reads/writes, namespace/root/locks, small/file
nodes, quota and object reservation/settlement plus required identity/auth/bootstrap
dependencies. M2 covers remaining identity/signup/rename/revocation, keys/Merkle,
teams/tokens, SSO/admin/realtime, maintenance and backup/restore. Maintain an explicit
route/side-effect inventory. Unsupported hosted routes fail closed; no local authority
fallback. A KV-only milestone is not production coverage. Migrations and bootstrap
are fenced jobs; restore retains non-rolled-back ownership/allocation authority.

Acceptance: same native/Wasm bytes/errors/rows/counters; real Rust and Go TCP/mTLS
flows; stale revision and revocation between prepare/commit; old frontend paused
before commit/prove; lost reply plus frontend loss; exact outcome recovery; receipt
retirement/restart/overflow/exhaustion; targeted SQL work independent of unrelated
history. No snapshot/reconstruction path in ordinary hosted request execution.

### D: End-to-end deadlines and combined local qualification

One absolute monotonic budget per operation. Authenticated private cross-process
UTC deadline has a qualified clock-error margin and is anchored once at receipt;
retries cannot refresh it. Check before queue admission/dequeue, storage/retries,
sync/outcome lookup and transmission. Ordinary target15s; polls preserve existing
wire waits; bulk jobs have bounded calls and separately measured completion policy.
Timeout is not mutation cancellation. Permits/debt follow actual work lifetime.
Caller starts before connect; public wire gains no deadline field.

Real celld bucket-durable local qualification through a stable frontdoor must combine
frontend and cell owner/volume failure, recover all acknowledgments and perform a new
authenticated write/read within15s. Fault process/host simulation cannot certify AWS
AZ/network/KMS behavior. No retryable error counts as successful recovery. Measure
1/4/16 simultaneous recoveries, then64 if prior gates pass; tenant populations128/1024
then10k only with memory/disk/time headroom. Report p50/p95/max, RSS/threads, queue and
SQL rows/bytes/CPU, isolate/Wasm memory, history/receipt/debt sensitivity and failures.

### S: S3 provenance and termination qualification (may proceed independently)

Persist versioned publication provenance before remote PUT. Legacy retrying attempts
are unqualified. Model actual physical attempts, not only a high-level future. A
timeout, abort, GET, retry success or owner loss cannot retire an earlier PUT. Audit
SDK and HTTP retries before claiming a single-attempt policy. Preserve unresolved
attempt charges and the128-attempt outage budget; do not inadvertently turn every
successful ordinary PUT into permanent admission exhaustion.

Test held earlier PUT plus later success, lost reply, process loss, presence without
termination, retry policy, limit exhaustion and report-only collection. Local qualified
filesystem collection remains supported. Remote deletion remains disabled; enabling
it requires separate reviewed proof that every possible PUT is terminal.

## Review and test discipline

For every commit group: author/direct review, independent subagent review, fix findings,
then focused meaningful tests and affected lint. Fixes after testing receive narrow
rereview and rerun. Root controls one Cargo slot (`CARGO_INCREMENTAL=0`, `-j2`) and
checks disk headroom (~2 GiB initially). Build/relink standalone agent before CLI IPC
tests; run native desktop serially. Use barriers rather than sleeps for races.
Commit explicit implementation paths only, with subject and separate body paragraphs.
All Markdown remains unstaged. Do not label unrun or external qualifications passed.

## Plan review log

Direct review: strengthened handle identity with database generation; terminal capacity
reservation; retained child ownership after retry; known-receipt precedence; explicit
PrivateFile rollback authority and interrupted migration; exact i64 codec; separated
cell/native fences; phase boundaries cannot leave raw-ID bypasses. Independent review
passed by recovery and quota agents, with corrections incorporated above: rollback
on all kernel failure paths, pre-materialization SQL guards, exclusive PrivateFile
upgrade fencing, Native envelope compatibility, and retained-lock preeffect barriers.
Implementation may proceed; code review/test results will be appended per group.

### S1 implementation review record

Initial S commit records immutable backend publication policy with admission and
reports unknown history, retrying remote calls and acknowledged remote calls. It
does not invent physical-attempt identities for opaque SDK retries or claim their
termination. Ordinary successful publication settles existing logical admission;
all physical object bytes remain charged. Ambiguous logical attempts still consume
the unresolved budget even after GET confirms presence. No policy grants deletion.
Schema55 preserves legacy history as unknown and tests canonical migrated schema.
Deterministic mock holds backend work after a gateway503 while a retry and GET
succeed, demonstrating why the remote collector remains report-only.

Direct review complete: both online upload and offline externalization journal
policy before PUT; migrated rows are never upgraded; counters retain their existing
meaning; local deletion authority remains a separate namespace lease/fence. Independent review and tests passed; see results below.

### D1 native deadline slice

Before hosted integration, enforce a single native ordinary-RPC budget starting at
receipt of the frame's first byte. Idle wait and initial TLS are separately bounded;
the official client's original budget still starts before connect. Frame remainder,
body read, dispatch, handler, queued writer start, object I/O and response flush all
consume that same deadline. Default ordinary budget becomes15s. Poll methods retain
their protocol wait and a separately bounded output phase; admission/body parsing
still uses the ingress budget. Do not renew budgets at each frame/body/write step.

A scoped absolute monotonic context follows blocking work into the writer queue and
is checked before new work starts. A queued expired mutation cannot execute; an
already running mutation may commit. Accounting for already admitted physical effects
must still settle after expiry under retained work ownership, so settlement runs in
an explicit non-request cleanup context. The outer request closes on deadline while
real work retains its execution/memory permits. Reader checkout checks admission;
S3 whole-operation timeout clamps to remaining budget and never claims PUT termination.
SQL interruption and authenticated cross-host deadlines remain M0/M1/D gates; this
slice must not claim a hard bound on uninterruptible SQL/filesystem work.

Tests: deterministic expired-context admission/dequeue; queue-held operation retains
permits until release; scoped/nested deadline restoration; parsing and transmission
consume one original deadline; polls retain behavior; after-expiry settlement records
physical debt. Existing KEX/TLS/socket-memory and upload/recovery tests remain required.
Direct plan review flags idle-vs-request timing, cleanup exemption, and timeout/outcome
ambiguity explicitly. Independent D1 plan review pending.

S1 committed as `1d704a30` after direct and independent recovery-agent review.
Validation: server-db65passed/1existingbenchmarkignored; server chunk20passed/1liveS3
ignored; external-chunk TLS/recovery/backup/collection integration12passed; all-target
server/server-db Clippy completed (one preexisting unused test import warning).
A synthetic schema53 fixture was corrected to remove55 columns before downgrading;
the correction received narrow independent rereview. Local S3 fault tests do not
qualify real AWS termination or enable remote reclamation.

D1 plan independent storage-agent review passed. Direct code review: preserve the
absolute instant across S3 runtime scheduling and check before first future poll;
queued tasks check at admission and dequeue; only admitted physical accounting clears
request context. Deadline checks do not claim to interrupt running SQL/filesystem work.
Code independent storage-agent review passed after adding the missing ingress deadline check before long-poll dispatch. Unit validation covered all112 active server tests (four existing ignored tests); the synthetic backup predecessor fixture correction passed all six backup tests and was committed separately as `2e9a7745`. Connection validation passed three KEX admission and five lost-reply/recovery tests. Affected all-target server Clippy passed. D1 committed as `fbcf6640`.


### M0 feasibility result

Direct and independent storage-agent review passed. Native differential tests (3),
Node host tests (3) and Wasm-memory parser tests (3) passed; native and Wasm Clippy
passed with warnings denied. Real celld0.6.2 with local bucket durability passed17
semantic vectors, including exact i64/BLOB/NULL, replay/error precedence, host
failure and trap after INSERT, deferred COMMIT rollback and oversized stored BLOB
rejection. Native and Wasm use the same domain kernel. A first run exposed a test
comparator's object-key ordering assumption; reviewed canonical comparison fixes
that without ignoring fields, array order or scalar differences.

This is a feasibility milestone, not an enabled production adapter. The explicit
new-namespace failure vector records an intentional transaction-boundary difference
from existing native behavior. Name-conflict work grows from23 to18023 SQLite VM
steps with0→1000 retained same-name versions; unrelated current heads alone stayed23.
A current-head projection phase is required before claiming history-independent work.
Provider SQL execution still lacks production interruption. Ownership, durable
recoverable outcomes, full authorization/route coverage and recovery/capacity remain.

M0 implementation committed as `2f5f904e`; all Markdown remains unstaged.


### U/UC integration review gate

Client ledger/proto tests passed120 cases before consumer integration. Full consumer
author/direct and independent storage-agent review passed. Findings corrected before
testing: use one process clock origin and preserve injected app clocks; advance a
durable protected-write generation before creating temporaries; synchronize removals
before advancing cleanup cursors; keep ancillary-child cleanup under the upload pass
budget; clear a retired active owner and reset cursors on database-generation change.
PrivateFile v4 authority migration excludes older binaries with the existing exclusive
state-root lease. Both full handles and immutable intent cross IPC36. Full application,
IPC, native desktop and UI validation is underway; no U commit until these gates pass.

### Next reviewed phases

The [current-head plan](HOSTED_CURRENT_HEAD_PLAN.md) passed root and independent
recovery review before implementation. Schema56 preserves historical data and quota
while indexing only live heads; measurements must prove actual bounded SQL work.

The [caller-deadline plan](HOSTED_CLIENT_DEADLINE_PLAN.md) passed root and independent
review. D2 transport code passed direct and independent review, including an added
credential-entry expiry check before key preparation. Validation is queued behind U.

The [authority-envelope plan](HOSTED_METADATA_AUTHORITY_PLAN.md) passed author, root
and independent storage review. It separates immutable execution expiry from a later
outcome observation's fresh deadline; ledger bookkeeping does not invalidate domain
read revisions. The monotonic current epoch rejects execution in all closed epochs,
so safe retirement need not retain an unbounded list of epoch summary rows. The next
private fixture remains disabled as a production backend until real FOKS authority
and route coverage exist.


### Current validation findings

U application rerun passed228 active tests (two existing ignored), after reviewed
corrections preserving reset/inspection access with an unreadable private key while
requiring checkpoint authority for normal work. An interrupted legacy profile
publication exposed an additional migration-recovery gate: root and storage-agent
plan reviews approved encrypted per-profile reset-required quarantine, preserving
healthy siblings and requiring preview-bound explicit reset for the uncertain
profile. That correction and the remaining CLI/native/UI gates precede U commit.

Authority core/host review now restricts durable domain errors to explicit
Conflict/Permission/Quota; infrastructure bounds, deadlines, storage failures and
traps abort the encompassing transaction. Activation separately binds the catalog
hash. The pinned celld harness freezes Date.now/performance.now during synchronous
execution, so between-call checks cannot qualify SQL interruption. Post-sync checks
withhold expired output; this is not a hard bound on sync or ongoing SQL. Fixtures
compare controller-declared anchors and activation labels; actual database-host,
artifact-hash and retrievable-key validation remain production bootstrap gates.

D2 direct and independent recovery-agent reviews passed. Its isolated validation
passed9 transport unit tests,1 actual TCP/mTLS caller-deadline integration and5
existing lost-reply/recovery tests. Source hashes are recorded in
/private/tmp/foks-d2-source-sha256.json. Affected all-target client and testkit Clippy passed with warnings denied. D2 committed as `967c549b`; the commit contains exactly the transport implementation and its TCP/mTLS test. All Markdown remains unstaged.


### Current-head projection committed

Schema56 committed as `295bc757`, exactly20 implementation/test files from the
hash-verified isolated validation snapshot. Main's unfinished authority-kernel
changes and all Markdown remain unstaged. Reviews covered triggers, migration,
query semantics and derived-only legacy-proof replay, plus narrow reviewed fixes
for synthetic predecessor fixtures and an unused test import.

Validation passed181 database tests (one existing benchmark ignored),7 backup-unit
and5 live-backup tests,12 external-file tests,2 transfers,3 pinned-Go flows,6 KV
conformance tests,1 writer-upgrade test,5 proof Rust tests,3 native-kernel tests,
7 Node proof and3 Node host checks. Actual local celld bucket durability matched
all17 schema56 native/Wasm vectors. All-target Clippy passed with warnings denied.
Negative/positive NameConflict costs stayed14/19 VM steps; fixed-result initial,
MAC and time listing stayed30/33/34, and keyed head updates354, through10,000
history/live-head/tombstone dimensions. Evidence:
/private/tmp/foks-current-head-celld/qualification.json and
/private/tmp/foks-current-head-source-hashes.sha256. These are semantic/cost gates,
not an end-to-end capacity or host/AZ recovery claim.

### U continuation gates found during final validation

The final application suite passed229 active tests (two preexisting ignored), and
real upload IPC passed. Agent validation exposed pre-admission hard-database creation:
overview readiness inspection and paired identity checks can create a database before
checkpoint enrollment records whether it was pristine. A contended manifest try-lock
can similarly strand fresh state. Fix admission ordering; do not relax missing-anchor
rejection or adopt unknown existing databases. These corrections need narrow direct
and independent reviews and deterministic contention/restart coverage.

Root and consumer-owner review also found a cleanup throughput blocker. The resident
timer currently selects one profile per60 seconds, alternates adapter/upload passes,
and scans every retained upload handle. With8192 retained handles, an otherwise
actionable new receipt can wait roughly11.4 days per profile for selection. Even
after selection,8-record passes can hold a large completed upload's spool debt for
many minutes. No foreground drain currently compensates. Before committing U, add
indexed actionable selection, independent clock/prune traversal, bounded fair
continuations and a direct-CLI maintenance path. Preserve erasure/fsync-before-debt
release, actual-worker ownership and bounded admission. Quantify progress and
fairness; faster timer polling alone is insufficient.

The detailed personal-authority extraction plan passed root and independent review.
It explicitly separates scalar authorization from the bounded read-bundle interface
needed to preserve4096-entry public listings. The deadline continuation plan also
requires conservative OS-clock sampling: compute wire remaining time from the
original caller deadline after sampling, so descheduling during the sample cannot
extend a receiver's deadline. Platform clock-domain qualification remains required.

### Authority real-celld portability finding

After direct and independent review, native authority tests passed10/10 and the
unchanged shared-kernel differential suite passed3/3. Native/all-target and Wasm
Clippy passed. An earlier negative test caught Serde accepting extra fields on an
internally tagged unit Controller variant; the reviewed empty-struct variant
preserves valid JSON while rejecting extras for both caller variants.

Actual celld root qualification passed27/28 vectors. Execute failed at the raw
SAVEPOINT statement with `SQL error: prepare SQL cursor: not authorized`;
/private/tmp/foks-authority-celld-root-a/qualification.json retains that failure.
Celld deliberately denies application transaction/savepoint SQL. Replace the raw
savepoint path with a reviewed typed host transaction callback that uses public
nested storage.transactionSync on celld and a native savepoint. Preserve one outer
transaction for authority, domain effect and exact outcome, with domain rollback
before recording definite rejection. Do not relax the expected outcomes or use
private privileged SQL. The callback ABI needs explicit reentrancy, memory lifetime
and trap rollback review before implementation and rerunning qualification.

## U continuation: admission and cleanup scheduling correction (review before code)

Validation exposed two production admission defects after PrivateFile gained an
external checkpoint. `requires_import_verification` creates an absent database
before ordinary checked admission; the paired nonblocking duplicate-ID probe does
the same before either pristine check. Fix readiness inspection to read external
readiness for both backends and avoid opening an absent database. Introduce a
fresh-empty initialization helper, called only under exclusive profile ownership:
try/acquire the authority first, require every hard-state artifact absent and no
checkpoint, profile-owned database claim, quarantine, import-readiness or pending
publication, create the fresh empty database, then persist its unique claim and
initial checkpoint before releasing authority or invoking user code. It takes no
DatabaseLock under the manifest. Existing databases use the unchanged database→
manifest ordering. A contended authority leaves no database behind; a failed or
interrupted authority persist after creation remains explicit reset-required and
must never become permission to adopt that database on reopen. Paired initialization
can durably finish the first profile and defer the second safely. Test all those
boundaries with deterministic contention/persist-failure hooks. Root and independent
storage reviews approved this structure before implementation.

Cleanup throughput is an additional rollout blocker in the current U snapshot.
The resident loop visits one profile every60s and alternates upload/adapter work.
For P profiles, a normal upload pass arrives about every120P seconds. Each pass
removes at most8 protected records and spends at most50ms cooperatively. Thus a
1GiB/256-chunk upload needs at least33 passes, roughly66P minutes after selection,
ignoring slower fsyncs and temporary scans. Worse, selection currently revisits all
retained Complete receipts and active uploads:8192 receipts imply approximately
11.4P days per rotation. There is no foreground drain or direct-CLI continuation.
These are cadence calculations, not measured disk throughput.

### Reviewed-interface proposal and ownership split

Storage agent owns client-db only: replace `next_upload_maintenance` with an indexed
`next_upload_cleanup(after_internal_id: Option<[u8;16]>) -> Option<UploadSummary>`
that returns the exact selected internal operation, not a second scope-page lookup.
Eligible states are RemoteVerified and terminal with cleanup phase before Complete.
Index the actionable cleanup table, keyset by internal ID, and skip unresolved live
rows without scanning retained Complete receipts. Keep the small bounded active
set explicit in query-cost tests. Add a separate indexed
`next_upload_unstamped(after_internal_id) -> Option<UploadSummary>` for V1 terminal
rows missing terminal_at, including cleanup Complete. `next_upload_clock_account`
continues to enumerate independent upload clocks; `prune_upload_receipts` is called
from that account cursor even when no cleanup exists. Make terminal stamping a
no-op for already stamped rows so maintenance cannot invalidate a temporary sweep
merely by rewriting its clock. Preserve retention-floor/checkpoint publication and
all child/parent ownership rules. Schema remains42 while U is uncommitted, with
accurate41 migration fixtures. Storage must send final SQL/API details for review
before edits; no auto-start follows this proposal.

Consumer agent owns core/app/agent/CLI: expose a bounded maintenance outcome with
separate `made_progress`, `more_work`, and `blocked`/clock-untrusted facts. The
existing cleanup_deferred boolean alone cannot distinguish resumable progress from
permission errors or unresolved children. Keep one cumulative50ms/8-removal budget
per checked profile turn, and do not reset that budget when switching a domain or
continuing after a phase boundary. No synchronous all-file erasure on completion.
Run upload stamping/account pruning separately from actionable erasure; avoid clock
writes during an active generation-bound temporary sweep. Completed receipts with
untrusted clocks remain retained and actionable for explicit clock repair, with
backoff rather than an immediate loop.

Resident policy proposal: retain one physical maintenance worker/permit and the
existing profile FIFO. Wake at startup and after a completion/cancel that can create
cleanup work. Notifications are coalesced hints only; durable indexed discovery is
the source of truth after restart. While progress/backlog exists, admit at most one
profile turn per250ms globally (four turns/s; at most200ms cooperative work/s before
blocking-call overruns). Rotate profiles after every turn; foreground waiters keep
FIFO priority, and admission contention/error backs off. Idle/error fallback remains
60s, with bounded per-profile exponential backoff only for whole-profile admission
or authority failures. Owner/account clock or dependency backoff must not suppress
healthy work in that same profile, and clock/prune backoff is independent of erasure.
At startup and each discovery wake complete a fair profile round (one turn/250ms)
before idle fallback, even when the first profiles are idle. Completion hints mark
only the affected profile due, preserving other profiles' backoff. An unfinished blocking turn retains its gate/ownership, so timer hints
cannot spawn a second worker. No timeout-based permit release and no detached
cleanup writes.

This cadence gives a theoretical upper erasure rate of16 records/s when upload and
adapter turns alternate, and a1GiB debt release needs at least16.5P seconds after
selection. Actual fsync/scan cost must be measured; this is not a completion promise.
An active owner with no progress must yield to the next indexed owner and back off.
A progressing large owner can retain its phase cursor, but add a bounded burst
limit and owner rotation so another small completed upload cannot wait behind an
unbounded temporary sweep. Temporary scan invalidation under continuous foreground
writes is explicitly a liveness concern: preserve its fail-closed generation check,
measure/reset/yield it, and do not release debt simply because the sweep is slow.
If no bounded cursor policy can complete under the admitted write workload, report
that as an unresolved capacity constraint rather than weakening erasure proof.

Direct CLI proposal: add an explicit `retention maintain --profile ...` local-only
bounded drain command and run one bounded preflight maintenance turn before new
upload admission, outside the upload's checked/source-consumption closure. Retain
state/profile ownership through each physical turn, then release profile admission
between turns; no reacquired authority may authorize a source retry. The explicit
command has a maximum wall budget and reports remaining debt/blocked owners; it
never silently loops forever, returns a success claim when debt remains, or starts
an unowned background thread on process exit. A CLI without a resident agent can
recover spool capacity through that command. Foreground upload success remains
separate from cleanup completion. Root-approved command: `retention maintain --profile NAME --max-seconds 15
--max-turns 256`, defaults15s/256turns and explicit caps60s/1024turns. Incomplete
requested drain exits nonzero with remaining debt. Admission wait consumes the
same turn/caller budget. The wall budget is cooperative between physical operations;
blocking fsync may overrun and must retain ownership until return.

Tests: thousands of retained Complete receipts must not increase VM work for one
pending cleanup selection; terminal timestamps/pruning must progress with no
incomplete cleanup; blocked owner/clock cannot starve another owner/profile; profile
rotation and notification coalescing use fake time, no sleeps. Check a256-chunk
logical fixture's required turns and measured removals under a cumulative budget,
with another profile/foreground waiter interleaved. Test restart discovery, direct
CLI debt drain and budget exhaustion, cancelled blocking maintenance retaining its
permit, temporary-sweep invalidation without false completion, and debt release
only after durable erasure. Run actual small/large filesystem drains to report
measured latency and fsync cost separately from deterministic cadence arithmetic.
This continuation requires root+independent plan review, author implementation
review, independent code review, then focused tests before U commit.

DB selector plan approved by root and independently reviewed by storage: add partial
indexes on cleanup(operation_id) WHERE phase<4 and handles(operation_id) WHERE
handle_version=1 AND terminal_at IS NULL. Use distinct first/continuation SQL
with direct exact-summary decoding and indexed keyed joins. Cleanup selects states
5/6/9/11; unstamped selects6/9/11, including Complete. Unresolved skipped rows are
bounded by max_pending<=128 (default16). Production-SQL VM tests vary retained
Complete/stamped receipts through8192, pending rows through128, and continuation
position. Stamped and legacy terminal calls must preserve metadata revision/token
and clock. Consumer review clarifications: clock failures back off their owner or
account rather than starving healthy work in the profile, and CLI time budgets are
cooperative across blocking physical operations; ownership survives an overrun.

Root also approved account-local prune selection driven by existing
kv_uploads_by_owner(host_id,uid,party_id,operation_id), with keyed handle/cleanup
joins. This excludes unrelated accounts, but scans and sorts matching-account
retained V1 rows (at most configured65536, default8192), plus fixed migrated
legacy rows (at most the old1024 allowance). VM tests report this cost
separately; pruning is not history-independent within an account.

The explicit CLI debt API is `upload_cleanup_debt() -> UploadCleanupDebt` with
reserved_bytes, incomplete (cleanup phase<4, including unresolved owners), and
actionable (phase<4 and state5/6/9/11). It scans bounded retained owner metadata,
never chunks; a missing cleanup ledger is corruption and fails closed. Author
review and quota-agent independent review passed for all three DB implementation
files before tests. Root reviewed the SQL/stamping/pruning slice and debt contract.

DB continuation validation (2026-10-15): exact three-file source snapshot in
`upload-retirement-validation` passed all121 client-db library tests and
client-db all-target Clippy with warnings denied. Focused receipt suite10/10
includes genuine41 migration, true timestamp no-op, missing-ledger debt failure,
and production query cost. At0/100/8192 unrelated retained receipts, VM steps
remain cleanup58/60(first/continuation), unstamped45/47, scoped-prune122 and
absent-account21. At128 unresolved pending rows cleanup costs1466 and unstamped1453;
adding100 matching-account receipts increases pruning122→5622. These are SQLite
VM counts, not elapsed filesystem/hosted responsiveness claims. Initial test
compile needed a checked i64 conversion of a fixed expiry (rusqlite lacks u64
ToSql); author, root and both independent agents reviewed that test-only correction
before rerun. Logs: `/private/tmp/foks-u-maintenance-db-{focused-c,full,clippy}.log`;
manifest/hash record `/private/tmp/foks-u-maintenance-db-{manifest.txt,sha256.txt}`.
Consumer scheduling, fairness and filesystem drain validation remain separate.


### Authority qualification commit

Committed as `f30c70c8` after direct and independent reviews, 14 native tests,
13 Node tests, native/Wasm lint, and all 69 actual-celld vectors across six suites.
The 17 committed blobs match the frozen tested manifest. All Markdown remains
unstaged. This is a qualification fixture, not an enabled persistent adapter or
a successful host/AZ recovery claim.

### Upload retirement commit

Committed as `6c0f8674` after plan review, author/root and independent reviews,
and exact58-file snapshot validation. Final continuation gates passed242 app,
151 agent,14 protected-store,8 CLI,1 real-agent IPC,252 native desktop and1673
UI tests, plus TypeScript, formatting and10-crate all-target Clippy. Earlier
unchanged core/protocol/backend and121 database checks remain recorded in
`/private/tmp/foks-u-final-validation.txt`. Three existing ignored tests remain.
All Markdown is unstaged. Cleanup preserves physical ownership and erasure debt;
continuous protected writes can still delay temporary inventory, and filesystem
budgets are cooperative rather than preemptive. Combined main qualification with
the newer hosted phases remains required.

D3b committed as `74be8a2b` after direct and independent reviews and exact96-file source hash verification against the isolated tested worktree. Final gates:897 passing tests (557 core/app/agent/CLI/OIDC/clock,50 desktop library,9 backend binary,5 backend transcripts,14 MCP,262 serial native desktop), three existing ignored tests; strict all-target Clippy across11 affected packages and whole-workspace formatting passed. Logs and source hashes: `/private/tmp/foks-d3b-final-qualification.json`, `/private/tmp/foks-d3b-union-sha256.json`. All Markdown remains unstaged. Parallel cleanup throughput is unqualified; successful host/AZ15s recovery and authenticated cross-host deadlines are not claimed.

### Read-kernel checkpoint (2026-10-16, through 066a4e64)

Detailed plans/review/evidence remain in
[HOSTED_METADATA_READ_BUNDLE_PLAN.md](HOSTED_METADATA_READ_BUNDLE_PLAN.md).
Implementation-only commits since the last checkpoint:

- `19ac886f`: bounded binary metadata pages; native/Wasm and112 actual celld vectors.
- `f2fc9f60`: bounded public List encoder and private continuation primitives.
- `41f4b045`: exact actual-celld directory fixture; six representable cases pass,
  two native-valid large cases fail at the engine's2.2MB value/row limit.
- `3ef60ab7`: schema59 invalidation coordinate/hooks, connection schema admission
  and private restore incarnation rotation;237 native tests,58 combined metadata
  tests and112 actual-celld vectors passed, plus final verifier regressions.
- `a4f62d45`: schema-cookie capability probe; exact query qualified across eleven
  actual celld transactional DDL phases, with six Node and two Python tests.
- `c8887d76`: strict private continuation decoder;64 native metadata and10 actual
  Wasm tests, native/Wasm lint/build passed; golden wire bytes unchanged.
- `066a4e64`: feature-only native List oracle;15 exact authenticated TCP byte/status
  comparisons and strict server/testkit lint passed.

Current components under the same review-before-test discipline: pure streaming
schema contract extraction, bounded schema catalog queries, and exact activation
factory design. Begin/Next/Finish, bounded frontend assembly and final proof/socket
ownership integration are still pending. They are not enabled production routes.
The default Wasm remains a qualification artifact with raw fixture entry points.

Preserve the native metadata size range by default while the optional restricted
profile question remains unanswered. No existing native-valid record silently
acquires a lower limit. Full-range celld compatibility needs an explicit physical
storage representation; slicing a large SQLite value does not bypass its stored
row limit. Successful15-second host/AZ recovery, full route coverage, production
owner/bootstrap authority and authenticated cross-host deadlines remain open.
S3 reclamation remains report-only pending physical PUT termination proof.

Additional schema foundations committed after the checkpoint: `d9f59046` adds
bounded dependency/cookie/schema-object catalog queries (72 native tests and eight
Node tests over26 native page vectors); `6984a043` extracts the pure streaming
schema verifier (eight pure and210 DB tests, one existing ignored, historical
hash parity, native/Wasm lint/build). Both completed author/direct and independent
reviews before validation. Markdown remains unstaged. Actual celld exact-union
schema enumeration and connection-bound activation are subsequent gates.

Read design refinement: qualify the single-transaction List proposal in
[HOSTED_METADATA_SINGLE_TRANSACTION_LIST_PLAN.md](HOSTED_METADATA_SINGLE_TRANSACTION_LIST_PLAN.md)
before implementing multi-call read orchestration. Continuous writes can invalidate
every multi-call attempt; one transaction avoids that starvation but occupies the
tenant's writable owner. Preserve2048 SQL calls and64MiB Wasm limits. Bounded
seven-role batches address distinct visibility roles without relying on caches or
the64-member cap. The shared helper is not authentication or a serving route.
Both direct and independent architecture reviews passed; exact output/workspace
interfaces, fixed proof overhead and combined allocation/writer-tail tests remain
required before production use.


Read-kernel checkpoint through7730051f: e47fbfd6 adds indexed seven-role
generation batches (77 native tests, eight Node tests and34 actual celld vectors;
VM steps remain386 with1/100/10000 generations).7730051f adds immutable fixed
schema-profile descriptions and trusted-source-only generation (80 native tests,
one explicit generator run, strict native/Wasm lint and64MiB release build).
Both groups passed direct and independent reviews before tests. Generated profiles
are descriptions, not connection admission capabilities or production manifests.
Actual schema enumeration and the shared List domain helper have passed direct
source review; their independent review and final-catalog test gates remain tracked
in the detailed plan. All Markdown remains unstaged.


8ec3a044 commits the bounded shared personal List domain helper and native node-type
projection correction. Direct and independent reviews passed before validation;
fixture-only usage-invalidation corrections were separately rereviewed. Gates:
88 metadata tests (one intentionally ignored exporter), two native service/RPC
oracles, one existing Go-certificate authority flow, strict native/Wasm lint/build,
eight Node tests and37 actual celld page vectors. The4096-distinct-role helper
uses1100 SQL calls. Evidence: /private/tmp/foks-list-domain-qualification.json.
This is exact native domain/private-page qualification, not full hosted List
dispatch, bounded combined workspace, socket publication or failover proof.


ac9d3720 adds actual celld schema enumeration qualification (native exporter,
7 Node tests,3 Python tests,19 celld schema cases and37 page regressions).
6662bcc8 shares native post-authority List argument parsing without changing
authorization/error ordering (106 RPC tests,2 native integration checks including
22 raw authenticated RPC cases, strict lint and Wasm compilation). Each had
plan review, direct/author review and independent source review before tests.
No hosted route or recovery/density guarantee is enabled by these components.

### Connection configuration qualification: real TEMP shadowing found

cb01959f commits only the fixed private probe/controller/tests. Direct and
independent plan/source reviews, including the diagnostic amendment, preceded
validation. Node14 and Python3 passed; actual celld's original schema-cookie probe
passed11 phases. The new configuration gate FAILED: qualified TEMP creation allowed
confirmed table shadowing. This blocks source-only authorizer equivalence for the
connection-affine schema factory; no production readiness is claimed. Detailed
failure evidence and next design options are in
HOSTED_METADATA_CONNECTION_QUALIFICATION_PLAN.md. All Markdown remains unstaged.

### Pure host-context codec qualification

235b8c95 commits the strict structural context decoder after author/root/independent
source reviews. Native metadata100 tests passed with one existing ignored exporter;
11 focused decoder tests also passed under the default, combined optional serde,
and separate float_roundtrip configurations. Strict native all-target and default/
optional-feature Wasm lint/build passed. Both Wasm artifacts retain23 exports and
three imports, with18 initial/1024 maximum memory pages. Evidence and exact source/
artifact hashes: /private/tmp/foks-read-context-qualification.json.

No host export or authority capability was added. Wasm compilation/export checks
are not runtime decoding qualification; the held allowance describes bounded live
parser storage, not zero cumulative allocation traffic. Connection readiness,
combined List workspace, publication and recovery remain separately gated.

### Native scalar allocation admission

9442ef8d commits S1's three-file native scalar admission change after author/root
and independent source reviews. Checked binding/container/payload sizing precedes
owned Rust copies; canonical decimal formatting uses stack storage. Existing SQL,
page behavior, integer error categories and row limits remain unchanged. Nine
focused tests and98 full native metadata tests passed (one explicit exporter
ignored), as did strict native lint and default Wasm compilation. Root's combined
context+S1 native suite passed109 tests with the same ignored exporter. Evidence:
/private/tmp/foks-native-scalar-results.json. No failures or source corrections
were needed. Wasm scalar codec/TLS lifetime and higher-level/JS/engine allocation
terms remain separate S2/S3 gates; this commit does not establish full request RSS
bounds, production connection readiness or recovery guarantees.

### Connection-state gate and full List allocation baseline

fef8f861 commits the fixed TEMP/attachment probe after plan, direct and independent
source reviews. Node24/Python4 pass. Actual celld fails at initial attachment
inspection: pragma_database_list() is denied. The initially empty TEMP query works;
later positive TEMP and ATTACH candidate cases were not reached. The probe does
not equate a post-effect ATTACH cursor exception with absence. Evidence remains in
/private/tmp/foks-connection-state-results.json and the connection qualification plan.

5aa3a7ad commits feature-only List allocation tracing and exact native/Wasm fixtures
after both source reviews. Native117 plus one ignored generator, focused7, the real
certificate exporter and strict native/default+feature Wasm gates passed. Runtime
qualification FAILS honestly:41/42 Node tests pass, with55 matching oracle cases,
one OOM trap and two subsequent cycle cases unreached. A maximum16MiB parent plus
a valid16,750,868-byte List/16,750,904-byte frame traps at the final frame allocation
under the unchanged64MiB maximum. The retired instance's52,559,872-byte memory was
fully wiped; this is its size at disposal, not a claimed64MiB peak measurement.

Evidence: /private/tmp/foks-list-charge-{native,runtime}-qualification.json and
node-a.{log,json}. This commit supplies a reproducible failing feasibility gate;
it does not enable a serving adapter. Review protocol-preserving owned framing
before rerunning the maximum-parent cycles. Scalar S2, page-response lifetime,
complete workspace bounds, connection activation and recovery remain pending.
All Markdown remains unstaged.


### Restricted hosted profile authorized; attachment inspection qualified

User selected an explicitly restricted hosted profile first, instead of requiring
segmented metadata storage to preserve every native-valid record. Native limits
and public wire encoding stay unchanged. The hosted profile must bound individual
values AND complete SQLite rows and explicitly reject unsupported existing data
and writes before effects. Storage will prepare the concrete limits/admission plan
for two reviews. Full-range Wasm framing remains a useful regression, but it is no
longer a prerequisite for this independently restricted first hosted milestone.

C3 replaces the unavailable database-list probe with a fixed table-list presence
query after direct and independent plan/source reviews. Native tests prove detection
of empty in-memory and file-backed attachments, multiple/DETACH transitions and
post-effect cursor failure, against a separate PRAGMA oracle. Node19/Python4 and
actual pinned celld/Moto PASS. Qualified TEMP creation is positively detected;
ATTACH denial is followed by independent absence inspection. Evidence is in
/private/tmp/foks-connection-table-list-results.json. This establishes runtime
inspection prerequisites only, not sealed connection lifetime or serving readiness.
All Markdown remains unstaged.


S2 scalar admission/response ownership is implemented and twice reviewed; final
qualification is recorded in HOSTED_METADATA_SCALAR_ALLOCATION_PLAN.md and
`/private/tmp/foks-scalar-codec-results.json` (eighteen frozen source paths).
Native union132, serde scalar24, strict native/default/trace/token lint, scalar/token/
page/directory Node gates and123 actual local celld cases passed. The unchanged
full-range trace remains43/44 with the known large-parent/near-limit RPC-frame OOM
at64MiB; no production memory/HA qualification is inferred. Preserve that failure
through F1 and keep restricted hosted-profile acceptance separate from native
full-range behavior. Root committed the exact eighteen-file implementation as `5b6f86d6`; the evidence ledger records that commit.


S3a selected List authentication caller admission is source-reviewed and tested
on P1 `5a014c98` and committed as `4842499c`. Six frozen paths and full evidence are
in `/private/tmp/foks-auth-integration-{manifest.txt,sha256.json,results.json}`.
Focused7/full native146 (one intentional ignored), real TLS2, strict native/Wasm
lint/build, Node scalar5/page11/trace44 with58 domain cases all passed. The only
initial failure was test-only empty-array type inference, corrected after both
reviews with the failed log retained. The82,296-byte selected cumulative term
does not complete S3, total scalar formulas, or a production workspace bound.


R0 trusted restricted-profile descriptors committed `87c0f97e`, six source paths.
Focused9/full metadata155, strict native lint and two independent create-new
exports passed; all four artifacts matched exactly under SQLite3.53.2. Evidence:
`/private/tmp/foks-restricted-descriptors-qualification.json`. The original lint
failure and reviewed equivalent clamp correction remain recorded. This is a
native development generator only; sqlite_sequence/platform data and imported
snapshots are not admitted. R1 pure helper plan is the next source gate in
HOSTED_RESTRICTED_METADATA_PROFILE_PLAN.md; no R1 implementation authorized yet.

S3b resolver admission qualification completed on `87c0f97e` and committed as
`a777d0a9` with all five frozen source hashes unchanged.
Exact five reviewed paths: `/private/tmp/foks-resolver-admission-manifest.txt`.
Evidence `/private/tmp/foks-resolver-admission-results.json`: native162/2 ignored,
final focused15, real Rust+Go2, strict native/Wasm lint and default build PASS;
actual celld fresh personal suites33+31 PASS. Immutable default Wasm retains
the23 export name/kind set and64MiB declared maximum. Generic authority-load
serde ownership and broader workspace budgets remain excluded.

S3c private authority-state decoding admission qualification committed
`6e6b275d` with all four frozen source hashes unchanged. Four exact source hashes and full evidence:
`/private/tmp/foks-state-admission-results.json`. At W1 base345247ce, native179/2
ignored, focused5+requested-peak2 under two serde feature sets, strict native/Wasm,
real Rust+Go2 and actual celld133 across eight fresh suites all passed. No source
corrections or failed gates. Preserved serde plus preheld live envelope does not
claim cumulative allocator-traffic accounting or recoverable global OOM.

R1 pure restricted-profile storage bounds are source-reviewed and qualified on
`6e6b275d`, seven paths ready for root integration. The shared schema-contract API
uses opaque pinned IDs and borrowed/dimension inputs, checks each completed field
and complete table/index record, and preserves full generated R0 provenance.
R1a14 tests/lint/two identical source exports; R1b pure15/focused18/fullmetadata188
plus native/Wasm lint/check passed. Evidence is
`/private/tmp/foks-storage-bound-qualification.json`, including the reviewed
alias-count test-only correction and original failed log. Generated source remains
490,341 bytes SHA b117937fad909a736e6e7769ce7717bf573f49c208bccd545b36659a68b03539.
This implements reusable arithmetic, not R2 scanning, R3 readiness or R4 writes.

R1a+R1b committed together as `5a1f4bc6` after root independently verified all
seven final hashes, logs and generated artifact. Both qualification ledgers now
record that commit; original alias-test failure evidence remains preserved.
All Markdown remains unstaged. R2 scanner work is plan-only until two reviews.


### Deployed celld deadline diagnostic

`b837bd0d` commits three diagnostic files after direct and independent plan/source
reviews, including a separately reviewed empty-POST-stream correction. Five Node
tests and both fresh local celld deployments passed. Four fixed SQL workloads
each ran with 250 ms and 15 s operation settings, all returning exact aggregates,
committed markers and successful sync responses. The largest requests took
0.927 s and 0.931 s. Concurrent reads to the busy actor and a second actor both
waited about 0.91–0.92 s. This measures total request latency; SQL-only duration
and native overlap remain unmeasured. It demonstrates that the short actor setting
is not a total request deadline, not a failure of the 15 s recovery target.

The first deployment failed before SQL at an overly strict empty-body check; its
evidence is preserved. All disposable processes were reaped, including forced
termination where graceful exit exceeded the cleanup budget. Source hashes and
report bindings are in `/private/tmp/foks-deadline-probe-results.json`. No production
deadline, cross-host clock bound, host/AZ recovery or PUT termination is qualified.
Markdown remains unstaged.


Factory F0 committed `9285f2ca`: native-used outer KV authority parser extraction,
with original ownership and clock/error ordering preserved. Five frozen files;
focused3/fullRPC112, native oracle32 TCP/helper vectors+8 injected-clock assertions,
TeamKV1, pinnedGo3 and native/Wasm lint passed. Evidence:
/private/tmp/foks-outer-auth-results.json. No production route/factory switch.

W2 committed `0bf6eeb4`: one held page workspace/control reservation with reusable
local credits and sticky refusal, retaining original output ownership. Eleven
frozen files; native199 plus3 intentional ignores, focused11, 58 native/Wasm parity
cases, 44 trace tests and25 host tests plus native/Wasm lint/build passed. The
reviewed test-only correction preserves the exact Decode74→Roles75 sticky refusal
sequence; initial failed logs remain. Evidence:
/private/tmp/foks-page-allowance-qualification.json. Observed Wasm memory stays
52,559,872 bytes across large/small/large; no RSS/tenant-density proof is implied.
All Markdown remains unstaged.


Factory F1 committed `3d992330` after both source reviews and all gates: focused12,
listener4, transfer2, fullserver125 plus4 intentional ignores and strict lint.
The fixed five permit slots remove variable per-charge Vec bookkeeping, preserving
fallback, sticky failure, input/worker/output retention. Fixed Request layout grows
72→128 bytes on this target; complete Request/Arc/RSS admission remains separate.
The existing maintenance fixture startup failure was reproduced on baseline, then
corrected and separately committed `cae8a3f5` after both reviews: inject its test
trigger after verified startup and assert the exact cleanup error. Evidence is in
/private/tmp/foks-request-permit-results.json and
/private/tmp/foks-maintenance-fixture-results.json. Markdown remains unstaged.


### Current integration checkpoint (a11dc0ec)

The restricted hosted profile is the selected first milestone. R0/R1 storage
record descriptors and bounds are committed; R2 finalized-backup capture/scanning
is in validation, with a separately reviewed real backup-producer correction.
R3 semantic validation/fresh import and R4 typed-writer preservation remain required
before profile readiness. Storage fit alone cannot authorize the bounded authority
resolver to skip native historical-row conversion checks.

C4a committed a11dc0ec after both plan/source reviews and narrow correction reviews.
Final native qualification compares462 programs with trusted_schema ON/OFF plus19
behavior steps. Actual celld matches459 direct bytecode programs, qualifies3 mapped
nested transaction controls separately, and passes19 behavior steps. Python9 and
Node8 pass. Raw programs, compiler provenance (explicitly different compilers),
failed attempts and cleanup evidence remain in
/private/tmp/foks-sqlite-policy-results.json. This is pinned-corpus evidence, not a
connection capability or production admission.

F0 native outer auth parser, F1 fixed Request permits, W1/W2 page/output ownership
are committed. F2 original Request-to-owned-List bridge is source-reviewed and waits
for the serial Cargo gate. The closed connection/factory integration is next;
no hosted route is enabled by these prerequisites. Production durable ownership,
all supported typed routes, cross-host deadline qualification and successful15s
host/AZ recovery/capacity are still open. U/UC client receipt retirement and native
end-to-end deadlines are already committed. S3 remote reclamation stays report-only.
All Markdown remains unstaged; only validated implementation groups are committed.


Backup producer prerequisite committed7c0f1d47 after two reviews and focused6,
fullserver169/4ignored, external backup2 and strictlint passed. A real completed
backup had left32768 bytes of SHM. The producer now checkpoints/switches DELETE,
closes/syncs and verifies sidecars before manifest-last publication. The reviewed
macOS canonical-parent correction preserves leaf/hardlink identity checks.
Evidence: /private/tmp/foks-backup-finalization-qualification.json.

R2 committed55dec5e4 after two reviews and focused14, fullserver183/4ignored and
strictlint passed against the committed real producer. All99 tables/history,
complete table/index bounds, closed package/key companions, sequence highwater,
identity, cancellation and deadline paths are covered. Seven source hashes and
all initial failures remain in /private/tmp/foks-storage-scan-qualification.json.
The result is storage fit only. R3 semantic/fresh import and integrated F3/C4
serving ownership remain open; no import-ready or production claim follows.

F2 original Request bridge committed `b6ffacb9` after qualification on the exact
55dec5e4 union and root verification of13 sources/27 evidence hashes. Evidence is
`/private/tmp/foks-request-bridge-qualification.json`:13 reviewed source paths,
core200/server151 plus focused5/6, strict native and three Wasm feature builds,
Node25+44 with37 page/58 native-Wasm trace vectors. The one test-only entry-copy
expectation correction and sandbox-only OS permission failure remain recorded.
No actual-celld rerun or serving readiness follows from this accounting bridge.
Source reviews precede tests; one focused assertion omitted three existing
pre-query credential copies. The narrow test correction was directly and
independently reviewed; production accounting did not change. Old compiled Rust
artifacts were pruned to restore13GiB build headroom; source and test evidence
remain intact. All Markdown stays unstaged.


R3 committed `5d6d5fb7` after author/root/independent source reviews and validation.
The private fresh importer preserves exact source rows, rowids and sequence state,
restores only trusted triggers, validates personal metadata and existing keys, and
retains the database with its physical leases. Final default and celld-proof server
library suites each passed169 with4 existing intentional ignores; both strict
all-target lints and2 real backup tests passed. Focused R2/R3 coverage passed32 and
is included in the final library runs. The intentional-corruption fixture and
test-only import corrections were reviewed before reruns; original failures remain
in /private/tmp/foks-personal-import-qualification.json. Root verified13 source
hashes and20 evidence artifacts before commit. This is native import qualification,
not serving readiness, celld activation, recovery latency or capacity proof.
All Markdown remains unstaged. The reviewed F3 group now connects this owner to
guarded personal List execution and real TLS output.


### Guarded native List qualification committed (5967a5b2)

Committed `5967a5b2` after direct and independent reviews of the complete F3
source and every validation correction. The private route now consumes a genuine
validated personal import, authenticates the issued and current credential in
native order, executes Main and Fresh under one original deadline and shared
2,048-attempt SQL budget, and retains the original allocation/physical owner
through local socket write and flush or disposal. Ordinary listeners and public
FOKS wire encoding remain unchanged.

Final validation passed: core161; server228 in both default and celld-proof
builds, with4 existing opt-in S3/benchmark ignores each; external allocation5;
client/server integration12; explicit live Go v0.1.9 authority/compatibility1;
official RPC fixtures8; strict core/server lint and41-file format check. The live
Go flow does not invoke the private F3 List route. The pinned actual celld/Moto
SQL-policy run passed469 programs (466 direct SQL and3 mapped controls) plus19
behavior checks. Celld cleanup required SIGKILL and was reaped; that is not a
graceful runtime-termination proof.

The final49 source hashes,10 tested binary digests, review closure, initial
failures and final evidence are recorded in
`/private/tmp/foks-guarded-list-qualification.json`; commit binding is in
`/private/tmp/foks-guarded-list-commit.json`. The measured native PathSegment
layout correction tightened diagnostic allowance to27,494 bytes (27,539 with
ingress). The reviewed Corrupt-only page-retirement correction permits the native
retry status after dropping partial output, while infrastructure/allocation
failures and attempted page reuse stay fatal. A malformed-node fixture clears
its derived usage value while retaining actual database guards.

This qualifies the native personal List slice and pinned SQL compatibility. It
does not qualify production metadata mutation coverage, actual celld closed List
execution, distributed ownership/durable acknowledgment, process memory capacity
or successful15-second host/AZ recovery. Remote S3 reclamation stays report-only.
All Markdown remains unstaged. The separately reviewed A0 transport extraction
now precedes the actual celld runtime integration.


### Shared transport qualification completed

Commit `cc7777864c1eb1752e28425d9f6f5dcf2d15b352` completes celld runtime prerequisite A0:
the real server Request/Deadline/framing code now lives in a SQLite-free shared
crate. Both source reviews, isolated native/Wasm dependency checks, all three
transport profiles, both server suites, five allocation tests and strict lints
passed. See `HOSTED_CELLD_GUARDED_LIST_PLAN.md` for counts, retained evidence and
limitations. The reviewed A1 source/library build plan is next; production
metadata adapter selection and distributed fencing remain deferred. Markdown
was excluded from the commit.

### Pinned celld build and physical-lifetime qualification completed

Commit `8e31944080e03651caaa3512169fe468f8375342` completed A1's pinned native
celld/V8 library build and identity qualification. Commit
`dca38eecb08263a3594f70025df7a309ed3682a5` completed A2's default-off local
physical-lifetime fixture after both source reviews and direct/independent
result review. Both build modes and strict harness lint passed, along with 63
tooling tests and 28 actual-binary scenarios, including 32 cold cycles, four
saturation rounds, and five expected panic quarantines. The final evidence is
`/private/tmp/foks-celld-a2-qualification.json`; details and limits are in
[the lifecycle plan](HOSTED_CELLD_RUNTIME_LIFECYCLE_PLAN.md). Both commits exclude
Markdown.

Commit `fee8db31ae8df590e32bc02f8c8de815d9f7ecea` completed the
[ordinary-node baseline](HOSTED_CELLD_NODE_BASELINE_PLAN.md) after both source
reviews, 139 harness tests, exact-bin build/Clippy, and direct/independent review
of the actual 25-step local scenario. Canonical handoff, fresh-directory restore,
Preserve restart and SIGTERM handoff all passed with exact acknowledged markers.
Trigger-to-restored-response observations were 185.7 and 185.9 ms in this single
low-load Moto run. They are not abrupt-failure recovery bounds. Ordinary celld
does not expose complete internal transport/task-drain proof.

The [guarded canonical-node integration](HOSTED_CELLD_GUARDED_NODE_PLAN.md)
follows separately; its [foundation plan](HOSTED_CELLD_NODE_FOUNDATION_PLAN.md)
and staging amendment passed direct and independent review. Full hosted
metadata coverage, absolute SQL interruption, durable ownership/fencing, tenant
density, simultaneous recovery and the successful 15-second host/AZ target remain
open. Strongly consistent S3 remains the durable-storage assumption; remote
reclamation remains report-only. All Markdown stays unstaged.
