# Metadata authority envelope and durable outcomes

Drafted against `2f5f904e` (M0) and `fbcf6640` (native deadline context),
2026-10-15. Author, storage-agent independent and root plan reviews passed. Keep this document
unstaged. No production backend is enabled by this phase.

M0 established actual native/Wasm PutDirents equivalence for initialized namespaces
and rollback after host failure, Wasm traps and deferred COMMIT failures. It did not
establish authorization, owner fencing, recoverable outcomes, SQL interruption or
complete protocol coverage. The current-name projection is a separate preparatory
change: the existing negative-name query costs 23/1,823/18,023 SQLite VM steps with
0/100/1,000 retained matching-name versions. Unrelated current heads did not increase
that probe's cost. Regenerate M0 vectors after that schema change before adoption.

## Identity and private access

Persist immutable installation anchors: random installation generation, tenant ID,
exact FOKS HostID and authority-cell ID. Separately persist a controller-versioned
activation record containing domain schema version/hash, kernel and statement-catalog
version/hash, active key generation/bundle digest, and chunk-storage namespace/placement
binding. Activation changes require an explicit CAS and invalidate older prepared
work; they cannot silently mutate an immutable installation anchor. These are separate
fields, not aliases inferred from a hostname. Existing FOKS user/team namespace is also mandatory on each scoped command.
Compare canonical byte encodings; integer fields use exact signed-i64-compatible
decimal strings across JS. A mismatched identity/version fails before domain SQL.

Use authenticated private transport between the native TCP/mTLS frontend and Worker.
The application envelope binds every field, command digest and deadline to a
tenant-scoped frontend credential. Constant bearer secrets are acceptable only in
the isolated local harness; production credentials require rotation, revocation and
deployment isolation. No public FOKS client supplies SQL, statement IDs, frontend
credentials, epochs or authority control messages.

Controller promotion and schema/key activation use a separate capability unavailable
to frontends. A frontend credential cannot initialize, promote, reset counters, import
backups or activate keys. Denied private calls return no domain data. Authenticated
frontend identity does not replace current FOKS principal/credential/team-role checks
inside the transaction kernel. The private fixture has a narrower explicit policy:
an enabled principal may observe its namespace aggregate usage and own outcomes;
its stored role governs PutDirents. It does not model real FOKS read-role policy.
Optional direct-DB normalization uses `normalize_at = checked(now_ms * 1000)` on
new registration, matching service microsecond creation times. Known exact retries
retain the original binding; omitting normalization preserves direct DB semantics.

## Owner epoch and revision

The private controller executes `Promote(expected_epoch, expected_owner, new_owner)`
as one SQL compare-and-swap, increments the epoch, closes the previous epoch to
execution, and persists the new owner. COMMIT plus `storage.sync()` must complete
before the controller acknowledges promotion or makes the replacement routable.
Lost promotion replies are resolved by reading the exact requested owner/epoch;
retry never increments again blindly. Epoch exhaustion fails closed.

Every domain transaction compares installation identity, active versions, native owner
epoch/owner and tenant revision in the same transaction as its queries/effects.
Celld's node ownership remains a separate fence supplied by the platform. Native
promotion does not modify or substitute for celld ownership. A paused old frontend
cannot register or commit after promotion. Previously committed outcomes remain
resolvable by the successor after current authorization checks.

The comparison revision is `domain_revision`: every domain mutation, authorization
change, promotion or version/key/storage activation increments it atomically. Intent
registration, outcome lookup and payload retirement do not change this revision; they
modify only the separate authority ledger. Registration binds the prepared bundle
revision without invalidating it itself. Execute compares that bound revision; if
other work advanced it, the attempt cannot reuse stale preparation. A later explicitly
prepared attempt needs a new binding; an unresolved executed attempt is resolved first.
The implementation must inventory every mutation affecting queried or authorized state
to enforce this rule; a missed revision increment is a correctness failure.

`QueryBundle` reads targeted rows, negative predicates and current principal authority
in one transaction and returns its revision. Additional query bundles require that
same revision. Final `ProveObservation` verifies the owner/revision/version tuple and
awaits sync before a native frontend publishes authority-derived data or errors.
Any revision change requires bounded restart within the original deadline. Use the
existing per-tenant scheduling/admission ownership rather than an unlimited retry
queue. Local transport rejection such as overload cannot return domain state and
does not count as successful recovery.

This linearizes an observation, not the eventual TCP packet: a frontend paused after
a successful proof can release an overlapping old observation after promotion. Such
an acknowledged mutation must already be in authoritative history. Tests distinguish
this case from pauses before commit/proof, which must fence.

## Registration, execution and recoverable outcomes

Use `(installation generation, owner epoch, sequence)` as the private intent identity.
The caller knows the proposed sequence before registration: read the epoch's durable
high-water, propose `high_water+1`, then atomically compare/increment and insert the
full live intent. The per-tenant scheduler serializes allocation; a competing exact
registration is reconciled, and a different-input collision is a definite conflict.
A missing identity at or below high-water is retired/unknown, never a fresh request.
No range allocation that leaves executable holes is introduced in this phase.

The registration binds canonical command bytes/hash, command/output format version,
principal credential, host/user/team namespace, prepared domain revision, policy and
preconditions, key/storage
generation and a fixed authoritative execution-time input where encoding requires it.
Before source consumption or remote object PUT, reserve the intent and all applicable
admission/debt, COMMIT and sync. The frontend retains the complete private intent
identity before sending its first registration request; registration retries use
identical bytes. A promise to resume after frontend process loss requires that caller
context to be durable and recoverable by the successor before first send. Otherwise
classify that public attempt as unknown, even if the authority can retain its outcome.
If the process loses its caller context, public clients without an
existing stable operation identity cannot be promised cross-reconnect exactly-once
execution. This limitation is explicit; do not synthesize a new private intent for
an unresolved prior public attempt.

`ExecuteIntent` performs in one short synchronous transaction:

1. Check installation/version/owner/revision, exact live registration and deadline.
2. Recheck current credential, team token expiry, membership/role and key generations.
3. Run the shared kernel's domain validation and mutation, preserving idempotence and
   protocol error precedence. No asynchronous I/O occurs inside the transaction.
4. Record a versioned recoverable typed outcome and advance revision atomically.

Outcomes contain the actual response data or immutable references needed to recreate
it, including exact signed material where applicable. An output hash alone fails the
gate. Bound and charge outcome rows/bytes before execution; late quota exhaustion
must not strand a committed operation without its result. Commit failure rolls back
domain changes, revision and outcome. Sync failure yields an unknown outcome, not
cancellation. Exact retry resolves the recorded outcome rather than executing again.

`ResolveOutcome` runs under the successor's current owner authority, matches the
complete original intent binding, and reauthorizes any sensitive returned material.
Closed-epoch pending intents cannot begin execution. Their classification is recorded
as definite unexecuted only after the domain transaction boundary proves no commit;
external publication ambiguity remains live and charged. A new owner may reconcile
that work but cannot relabel a potentially executed intent as fresh. The first
authority fixture supports SQL-only PutDirents: no external side effect can sit between
registration and its atomic execution. Before adding object/identity routes, specify
each continuation state and whether successor action only settles prior work or
requires a separately authorized new domain mutation; generic replay is insufficient.

Authority-derived command errors need recoverable error outcomes too. Implement the
domain work inside a savepoint under the owner/intent transaction: a definite domain
rejection rolls back its domain effects, then records its exact error outcome. Storage
faults, traps and commit ambiguity abort the entire transaction; they are not durable
domain rejections. Native and celld savepoint semantics receive explicit tests.

Namespace creation is part of the new atomic domain transaction. M0 showed that the
legacy native API creates a namespace before its mutation transaction, so failed
first writes may leave an empty namespace there. Preserve that difference as an
explicit differential fixture; production adoption needs a reviewed bootstrap contract
and compatibility tests, rather than silently claiming whole-method equivalence.

## Retirement and payload recovery

Keep bounded live/terminal row and byte budgets, with terminal capacity reserved at
registration. Retire only definite terminal outcomes whose sensitive payload and
owned resources have been durably cleaned. Maintain the epoch allocation high-water
after deletion; lookup precedes expiry so known old live work stays recoverable.
Explicitly handle out-of-order completion and low live sequences. The monotonic current epoch itself rejects execution in every closed epoch. Keep
only the current epoch allocation high-water: old per-epoch summaries can be retired
when no live/resource obligation needs them. Missing closed-epoch outcome lookup
returns retired/unknown and never executes. Epoch values never repeat; rollback
protection remains mandatory. This avoids an unbounded row per past promotion.

The first fixture uses controller-configurable bounds capped at 256 retained intent
rows and 1 MiB of reserved logical payload bytes, with 4,096 bytes reserved for each
worst-case recoverable response before any domain mutation. Canonical binding metadata
is charged in addition to that reserve. SQL-only PutDirents responses and fixed domain
errors must fit the reserve; an oversized result rolls back domain work. Pruning
retires at most 64 terminal rows per pass. Tests use smaller limits for deterministic
exhaustion and many promotion/retirement cycles. These are logical budgets, not a
claim about SQLite page/WAL size. No live or unresolved intent is evicted for space.
A controller settlement pass is also capped at 64 rows. For this SQL-only fixture,
a pending row with closed epoch, expired immutable deadline or invalidated domain
revision proves no domain-plus-outcome transaction committed; settlement records
`not_executed` without domain execution. This enables cleanup after principal
revocation. The rule must not be reused for an intent with possible external effects.

Do not import authority epochs, sequence high-water, live intent ownership or key
activation state from an older domain backup. The supported restore API imports
domain payload under a fenced maintenance job while preserving the authority ledger
and externally pinned installation/epoch floor. Raw full-cell rollback, restoring
that external floor too, and cloning live authority are unsupported and must not be
qualified as safe recovery. Normal celld owner loss restores current bucket-durable
authority history; it is a different operation from deliberate older-payload import.
The exact external floor/control store and its restart reconciliation are a release
gate before enabling restore, not a best-effort configuration assumption.

Key material is immutable by generation and stored encrypted under a separately
controlled wrapping key. Register its digest/reference and verify retrievability
before activation. Atomically activate the new generation with metadata capable of
using it; do not acknowledge a signed/identity mutation whose successor cannot obtain
the signing/decryption material. Old generations remain while live intents/outcomes
refer to them. Key staging, activation and retirement are controlled jobs, not ordinary
frontend commands. Local fixtures must prove this with replacement frontend processes.

## Deadline and resource contract

Carry the existing native absolute monotonic request budget into the first private
envelope. Across processes use authenticated UTC expiry with a fixed, qualified
clock-error bound, cap the received remaining budget, and anchor it once to local
monotonic time. Reject untrusted time or expired work; retries and successor lookup
cannot renew the original execution deadline. A later authenticated `ResolveOutcome`
is a new observation request with its own bounded deadline: it may return an already
committed outcome after execution expiry, but it cannot execute or reopen pending work.
This distinction permits recovery without extending the original mutation lifetime.
The exact deployment clock-error bound
must be decided and measured before the distributed deadline gate passes. The
current fixture persists only a committed-observation clock floor. An expired
Execute rolls back its attempted floor update; if trusted host time later rolls
back while revision remains unchanged, that pending intent can become eligible
again. Fixed expiry bytes do not prove irreversible expiry or untrusted-clock
safety. The persistent service must choose terminal expired-intent settlement or
an independently durable monotonic floor, and test rejected-expiry → clock rollback
→ retry alongside its per-request monotonic budget.

Check before admission, queue dispatch, SQL calls, external I/O, commit, sync and
publication. Retain work permits and object debt until actual execution terminates,
even when the caller's response times out. S3 provenance remains report-only; a
successful retry/GET does not terminate an earlier physical PUT. The pinned celld
production SQL path lacks a SQLite interruption hook; indexed bounded statement work
and a qualified cancellation/worker-isolation seam are required for the remaining
hard deadline claim. Pinned celld also freezes `Date.now()` and `performance.now()`
at the last I/O timestamp while synchronous JS/SQL runs (`harness.js:10748`), so
checking either before COMMIT cannot establish elapsed synchronous-work bounds.
The fixture checks before admission/commit and after awaited sync, but the held-sync
test proves only the output gate. Never present these checks as a hard SQL guarantee.

## Implementation ownership and gates

Storage agent owns the current-head projection and SQL cost regression first.
Recovery agent owns the shared authority kernel, bounded typed ABI, Worker transaction
wrapper and native/Wasm differential/fault fixtures. Root owns the native frontend
backend interface, existing deadline-context integration, controller capability/routing
integration and final commits. Coordinate schema ownership explicitly; the domain
schema migration and authority schema/version activation must not race.

Start with a private, disabled-by-default authority fixture exercising the real kernel.
Its controller-supplied identity and activation labels prove exact tuple comparison;
they do not prove correspondence to imported host_metadata, artifact digests or
retrievable key material. Those bootstrap/publication checks remain production gates.
Per-vector whole snapshots are setup/assertions only, not a persistent HTTP/native
adapter. Durable sync may hang; post-sync deadline checks withhold late publication
but do not themselves bound response completion or terminate storage work.
After the envelope/outcome gate passes, integrate complete authenticated KV plus its
identity/bootstrap dependencies. Track all remaining routes and side effects; no local
database authority fallback and no production-mode flag while coverage is incomplete.
Review each component directly and independently before its tests and commit.

Required gates include: exact native/Wasm outcomes and trigger results; same-transaction
rejections/savepoint rollback; changed identity/version/auth between prepare and commit;
old frontend paused before registration/commit/proof; lost register/commit/promotion
replies and process loss; successor outcome resolution with revocation; retirement and
restart with high-water preserved; outcome/intent capacity exhaustion; key activation
loss; fixed deadline across all retries; and duplicate/malformed envelopes. Restore
tests must independently attempt authority rollback. Combined frontend plus celld
owner/volume failure through one stable frontdoor is required before claiming zero-loss
successful recovery within 15 seconds. Local S3 emulator runs cannot certify AZ/KMS or
production network behavior.

Author review: made sequence identity known before registration to handle lost replies;
separated closed-epoch execution from successor resolution; required actual durable
outcome payloads and capacity reservation; included definite-error savepoint behavior;
kept namespace initialization's new atomicity explicit; rejected epoch/floor rollback
and clarified external-floor release gate; separated cooperative deadlines from SQL
interruption. Independent review clarified that registration does not advance domain
revision, all authorization/activation changes do, and later outcome observation has
its own bounded budget without reopening expired execution. Storage-agent independent
rereview passed. Root approved implementation after requiring bounded epoch retention
and explicit configurable row/byte/reservation/prune limits, now recorded above.

## Qualification correction: supported nested domain transaction

The first actual celld authority run retained27/28 passing vectors; real execution
failed at the raw SAVEPOINT statement. Pinned celld0.6.2 denies application SQL
transaction/savepoint control (`storage.rs` authorizer), while its supported nested
`storage.transactionSync(callback)` uses privileged internal savepoints. Native-only
success did not qualify this boundary. Preserve the failed evidence and the exact
error-outcome contract; do not call celld private transaction internals.

Proposed correction, requiring independent review before code:

- Add a narrow `Sql::domain(input)` host operation for only the shared domain kernel.
  Its default native implementation begins the existing named savepoint, invokes
  `wire::run` on the same Sql instance, and releases on success. Conflict/Permission/
  Quota roll back the nested scope and release before returning that exact rejection.
  All infrastructure/host/limit errors escape and abort the outer transaction.
  Authority execution calls this operation after all current binding checks, then
  records its outcome and revision in the still-open outer transaction.
- The Wasm host overrides that method with one bounded imported domain callback.
  It passes owned serialized canonical domain bytes, never arbitrary SQL, authority
  operations, pointers to caller-owned mutable SQL state, or a closure supplied by a
  request. JS opens supported nested transactionSync and re-enters the same module's
  existing domain-only run export. Only the Rust domain kernel executes inside it.
  The nested callback cannot register/promote or recurse into authority execution.
- Reentrancy contract: outer input has already been moved out of thread-local INPUT;
  no RefCell borrow of INPUT/RESPONSE/OUTPUT is held across any host import. The
  outer Rust Host instance is not exposed to reentrant code; the domain export uses
  its own stack Host. Copy the bounded command to JS-owned bytes before any Wasm
  allocation/growth; obtain fresh views after each allocation. Nested OUTPUT may be
  overwritten, then read and copied before constructing the domain response in
  RESPONSE. The outer import consumes that response before issuing another SQL call.
  A one-level host guard rejects nested domain callbacks. Native and Wasm execute
  the same `wire::run` logic, with platform-specific transaction ownership only.
- Preserve the original outer host deadline and aggregate SQL-call counter across
  reentry; do not call the top-level reset path. The existing 2MiB command and64KiB
  typed-result caps apply before allocation/copy. Domain success or exact allowlisted
  rejection is serialized into a closed typed response. Traps, failed rollback,
  malformed results, Bounds/Expired/IntegerRange/Invalid/Storage and arbitrary host
  exceptions escape the outer transaction; none become a durable rejection receipt.
- A domain rejection must throw through nested transactionSync before JS converts
  it into a typed rejection result. No output is published from the nested callback.
  Outer outcome mutation, COMMIT, durable sync and unchanged output-deadline gate
  remain mandatory. If outer COMMIT/sync fails, successful domain work is not reported
  as a proved success.
- Native/Wasm SQL control statement counts legitimately differ. Change authority
  fault vectors to target the exact real statement ID after its effect, rather than
  a native call ordinal that included raw savepoints. Keep existing M0 ordinal faults
  unchanged. Test after real InsertDirent and real AuthorityOutcome, plus Wasm trap
  and deferred outer COMMIT failure; final full-state equivalence remains mandatory.

Add deterministic host tests for nested rollback before rejection conversion,
non-allowlisted exception propagation, no deadline/counter reset, recursive callback
refusal, and shared-buffer reentry lifecycle. Rerun native10 plus differential3,
Node, native/Wasm Clippy, and all six actual celld suites. A missing supported nested
transaction or any failed exact-state comparison remains a qualification failure.
No production route or deadline claim is added. Author review: the outer authority
transaction must own every binding/counter/outcome change; the nested host operation
only isolates domain effects, and its callback must not mutate the outer Rust Host.

Budget clarification before implementation: retain native's existing logical count
for SAVEPOINT, rollback and release. The JS domain callback charges the equivalent
one admission before nested transaction entry, one on successful release, or two
on the rejected rollback/release path. These are budget charges only; JS executes
no raw transaction-control SQL. Both hosts therefore reject the same boundary-sized
command; rollback/release budget exhaustion aborts the outer transaction. Reentrant
Wasm stack-local counters are secondary guards; the unchanged JS aggregate counter
is authoritative across both entries. A separate one-level domain guard and active
outer-entry guard prohibit recursive domain work or a top-level reset. Native and
host tests cover exact2048 success and one-over-budget failure on both success and
rejection paths. Statement-ID fault injection remains tied to actual SQL effects.


Correction review and qualification completed: root and storage independent reviews
passed the callback design and implementation, including a follow-up error-origin
factory that distinguishes decoded domain status from arbitrary imported exceptions.
Native11 plus existing differential3, Node13, both Clippy targets and actual celld69
vectors passed against schema56. The first failed celld run and its unsupported
SAVEPOINT error remain retained. See `tools/foks-metadata-spike/README.md` for exact
artifacts and limitations. This completes the private semantic fixture gate; the
persistent authenticated vertical slice and combined failover gates remain open.
