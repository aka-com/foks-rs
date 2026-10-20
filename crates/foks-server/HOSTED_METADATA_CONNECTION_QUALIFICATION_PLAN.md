# Connection configuration qualification slice

This is a prerequisite from the reviewed read-bundle plan. It qualifies local
celld 0.6.2 capabilities and records gaps; it creates no serving factory or
production admission token. All Markdown remains unstaged.

## Implementation

Extend the existing private schema-cookie probe with an explicitly selected
`--configuration` mode. Default cookie behavior stays unchanged. The controller
pins/copies the selected module and worker, checks private capability before cell
lookup, runs exactly once on a disposable cell, syncs after the probe, preserves
failure evidence and cleans up child processes. No public/user SQL input exists.
Add a fixed `configuration.mjs` exporting the same runProbe interface; deploy it
as probe.mjs with the existing unchanged worker. The controller pins the source
to deployed-file mapping and mode-specific success predicate, reusing process and
bucket logic. Default deployment retains the original probe.mjs.

The probe reads exact scalar boolean PRAGMA results (one row/one column, numeric
0 or 1) for foreign_keys, ignore_check_constraints and recursive_triggers. It
sets foreign_keys ON, ignore_check_constraints OFF and recursive_triggers ON
outside transactionSync and verifies all three inside it. Recursive ON is an
explicit qualification choice; it does not silently change the native server.
Record initial settings and final settings. Setting failure is a failed gate,
not an unsupported-capability success. Do not change trusted_schema: the pinned
platform disallows it, and this fixture cannot prove a native-equivalent policy.
Report that unresolved gate explicitly even if this selected probe passes.

Use fixed temporary fixture names in main to exercise FK, CHECK and recursive
trigger behavior. Seed parent/child/check/recursive tables and a bounded recursive
trigger inside a transaction; roll back every fixture with an owned sentinel.
Assert valid inserts succeed, missing-parent and failed CHECK inserts reject,
and an update recursively increments to the fixed terminal value. Failed inserts
must leave counts unchanged. The schema cookie and named-object inventory after
rollback must match before. Unexpected errors or rollback errors remain failures.

While retaining that same connection, attempt fixed CREATE TEMP TABLE/VIEW/TRIGGER,
CREATE TABLE temp.* and ATTACH ':memory:' AS fixture. Each operation
must be rejected, with main cookie unchanged and an unrelated SELECT succeeding
afterward. Denial alone does not prove provenance: the actual platform source
pin/authorizer audit accompanies the result. Each statement must be valid SQLite
syntax with existing trigger target as needed. Native Node SQLite tests demonstrate
TEMP/ATTACH are accepted without the authorizer and therefore make this gate fail;
a deny shim is only a test of wrapper assertions, not runtime security evidence.
Do not claim DROP TEMP or DETACH runtime coverage without a constructible TEMP
object or attached database; the separate source audit covers those branches. No file path,
network address or caller-selected identifier can reach ATTACH.

## Validation and review

Direct plan review: configuration is connection-local and cannot be inferred from
schema hashes. All setting changes occur on a disposable qualification connection;
restoring original settings is unnecessary and may hide enforcement failure.
Fixed rollback fixture data and post-denial SELECT distinguish functional SQL from
a universally failing fake. Actual-engine denials remain separate from the pure
wrapper tests. Full trusted-schema parity and readiness invalidation still require
the later sealed adapter.

Independent plan review before implementation. Author/root direct source review and
independent agent source review before any tests. Node tests cover denied/accepted
operations, malformed settings, enforcement failure, foreign transaction/rollback
failure and worker capability/sync/one-attempt behavior. Python tests cover selected
controller success predicates and bounded evidence. Then run both configuration
mode and the existing cookie mode on actual local celld/Moto, pinning source/binary
hashes. No timing, durability, host/AZ or tenant-density claim follows from this run.
Commit implementation files only after reviews/tests; retain failures in the ledger.

## First source reviews and observed failure

Root direct and quota independent source reviews passed on the four-file
implementation. All12 Node and3 Python tests passed. Actual celld failed with
initial settings [1,0,0]: CREATE TEMP TABLE was denied but CREATE TABLE temp.NAME
returned successfully. Evidence is preserved at
/private/tmp/foks-connection-config-celld-a/qualification.json; binary SHA256
92ededd2bcfebeb1e78cc2d97219f00214a79eb6606f0eef48208fca3f98c15a.
Do not weaken the success predicate or call this complete admission.

Diagnostic-only amendment: on an unexpectedly accepted fixed TEMP/ATTACH candidate,
record the case and current main schema cookie, plus fixed count queries for the
qualified fixture name in sqlite_schema and sqlite_temp_schema. Each diagnostic
query reports either one exact nonnegative decimal count or a fixed unavailable
marker; diagnostics cannot turn failure into success. All SQL remains hardcoded.
For the specific qualified-TEMP case, if the temp catalog confirms the object,
insert a fixed marker into it, create a main table of the same fixed name with a
different marker, and compare unqualified/qualified reads. This occurs solely in
the already-doomed rollback transaction and establishes whether actual shadowing
is possible. Record fixed string results only. Unexpected diagnostic errors are
reported as unavailable and original failure remains primary. Native tests assert
this path on unrestricted SQLite and rollback of main changes; no engine security
claim follows from the Node test. Root and independent narrow review precede rerun.

## Qualification outcome and commit

Committed implementation only as cb01959f after both source reviews and the narrow
diagnostic rereview. Final Node14/14 and Python3/3 tests pass; original actual-celld
schema-cookie mode still passes all11 phases. Configuration qualification FAILS.
The first attempt and diagnostic rerun both preserve failure status. In the rerun,
qualified TEMP creation yielded main_count=0/temp_count=1. After inserting distinct
fixed markers into TEMP and a same-named main table, explicit TEMP/main reads
returned their respective markers and the unqualified read returned temp-marker.
This is actual shadowing on the tested binary, not a source-only hypothetical.

Evidence: /private/tmp/foks-connection-config-celld-b/qualification.json,
/private/tmp/foks-connection-config-cookie-a/qualification.json, and
/private/tmp/foks-connection-config-results.json. Frozen four-file manifest and
final hashes are /private/tmp/foks-connection-config-manifest.txt and
/private/tmp/foks-connection-config-sha256-b.json. The source audit's denial branches
did not predict this runtime behavior; do not certify a binary from that audit.
Initial actual settings were foreign_keys=1, ignore_check_constraints=0 and
recursive_triggers=0. The selected configuration and positive FK/CHECK/recursive
behavior passed before the name-resolution gate failed. TEMP VIEW/TRIGGER/ATTACH
were not reached in this run and therefore have no actual-runtime pass here.

Required next design: seal connection-local TEMP/attachment state with demonstrated
runtime controls, or separately review explicit main-qualified fixed catalogs and
all affected artifact hashes. Main qualification alone does not establish other
configuration/durability/ownership invariants. Neither remedy needs a public FOKS
wire or cryptographic change. Until qualified, the source-only celld authorizer
equivalence in earlier plans is superseded by this observed blocker.

## Candidate remedy for later review (not implemented)

Prefer an explicit connection-state invariant over assuming the platform denies
every SQL spelling. Before schema activation, inspect the actual owned connection's
TEMP catalog and reject any object. Qualification must establish the exact fixed
query is permitted and reads TEMP, not an accidentally resolved main object. A
bounded existence query is sufficient for rejection; it need not return names or
DDL. It should also detect TEMP triggers targeting main tables, which main-qualified
SELECT/UPDATE statements alone do not neutralize. Test both TEMP spellings and
TEMP triggers; a clean main cookie is deliberately insufficient.

The host must retain exclusive connection ownership from that check through the
activation transaction and every later guarded call. After readiness, the production
artifact/import allowlist permits only the fixed reviewed SQL catalog and no DDL,
ATTACH, TEMP or settings callbacks. Any migration/configuration transition destroys
readiness and requires reactivation. A connection loan that exposes arbitrary SQL
cannot keep its old proof. No mutation-capable arbitrary SQL callback or connection loan may occur during
the proven interval. Invalidate readiness before any such transition; if work has
already begun, roll back/restart under fresh admission. A final TEMP check cannot
retroactively certify earlier queries: a callback could create, use and remove a
TEMP object before that check. Publication checks supplement exclusive ownership
and never replace it.

Attachment state needs its own actual-engine gate. The current failing run never
reached ATTACH. A follow-up must individually qualify ATTACH denial and/or an exact
connection database inventory, rather than infer it from source. Foreign-key/CHECK
and recursive policy remain explicit on the actual connection. trusted_schema is
still unresolved; exact main DDL comparison does not automatically qualify every
connection setting. Expected DDL/catalog hashes must be updated only through the
reviewed source generator if a new inspection statement is added.

This remedy has not passed independent design review or runtime qualification.
Do not add a serving factory until those gates and the retained connection lifetime
are implemented. It preserves public protocol compatibility and does not require
choosing a production owner/bootstrap service.

Independent candidate-remedy review: quota PASS after strengthening callback
transitions to invalidate before mutation and require rollback/restart; a final
empty-TEMP observation cannot prove earlier name resolution. No runtime pass or
serving readiness follows from this plan review.

## C2: qualify a fixed TEMP inspection query and separate ATTACH denial

Implement a separate `--connection-state` controller mode, retaining the existing
configuration failure and cookie mode unchanged. The unchanged capability-gated,
one-attempt worker runs a fixed synchronous probe. No new SQL catalog statement,
production connection capability or activation factory is introduced in C2.

The proposed bounded query is `SELECT CASE WHEN EXISTS(SELECT 1 FROM
temp.sqlite_schema LIMIT 1) THEN '1' ELSE '0' END AS value`. Require exactly one
column/row with string0 or1. Check clean connection0; any initial TEMP object fails
the probe. Explicit temp qualification prevents accidental main resolution. Read
the existing main schema cookie before/after each rollback, but do not treat it as
TEMP evidence. The existence query returns constant-size data and stops at the
first object; it does not enumerate unbounded names or SQL text.

Test each fixed candidate in its own owned rollback transaction: CREATE TEMP TABLE,
CREATE TABLE temp.*, CREATE TEMP VIEW, and CREATE TEMP TRIGGER targeting a fixed
main table. A candidate may be denied or accepted. If accepted, the exact inspection
query MUST return1 while it exists; if denied, it must remain0 and an unrelated
SELECT must succeed. Explicit qualified TEMP creation is required to be accepted
and detected on the pinned binary, so a fake universally-denying backend cannot
claim positive inspection qualification. Each transaction creates the needed main
trigger target, and rolls back via an identity-checked sentinel. After rollback,
TEMP must be0, main cookie unchanged and fixed main target absent. Returning
normally, a foreign error or failed rollback fails qualification. The report keeps
each accepted-and-detected versus denied outcome distinct. A denied trigger is
not runtime evidence of detecting an existing trigger; unrestricted Node SQLite
supplies that positive test separately.

Attempt fixed `ATTACH ':memory:' AS foks_connection_state_attached` separately,
after TEMP cases, and require denial plus a working SELECT. Unexpected acceptance
fails immediately; do not reuse the connection or claim transaction rollback
detaches a database. The controller's disposable process is always terminated.
No filesystem/network address is accepted from input. This provides the missing
actual-engine ATTACH result without letting the prior TEMP failure skip it.

All success predicates explicitly retain production_admission=false and
trusted_schema='unqualified'. C2 establishes query/runtime prerequisites only:
exclusive ownership and before-loan invalidation remain future code requirements.
The controller must require exact mode, four ordered outcomes, qualified TEMP
positive observation, ATTACH denial, final empty TEMP and rollback restoration;
no earlier configuration failure is converted into a pass.

Direct plan review: isolate each candidate so an unexpected accepted operation
cannot contaminate subsequent evidence; inspect inside the transaction as well as
after rollback. Catch only the candidate execution for denial, never the assertion
or rollback error. Native tests must expose inspection against existing tables,
views and triggers, spoofed/denied inspection, ignored rollback and unrelated
failure. Controller tests reject missing/altered evidence. Independent plan and
two source reviews precede Node/Python tests and the pinned local celld/Moto run.
Commit implementation only, preserving Markdown unstaged and exact source hashes.

C2 independent plan review: recovery PASS. Root implemented four files and
completed author/direct source review before tests, tightening scalar shape to
require an object containing exactly the fixed `value` key. Frozen manifest/hash:
`/private/tmp/foks-connection-state-{manifest.txt,sha256.json}`. Independent source
review and runtime validation are pending; no readiness claim.

C2 independent source review found an ATTACH false-pass boundary: exec may have
completed the effect before result iteration throws. An exception plus alive/TEMP/
main-cookie checks does not prove attachment absence. Amend the plan with fixed
`SELECT CASE WHEN EXISTS(SELECT 1 FROM pragma_database_list() WHERE name NOT IN
('main','temp')) THEN '1' ELSE '0' END AS value`; require exact0 initially and after
the candidate denial. The query returns one constant-size bit, not filenames or
an unbounded inventory. Unavailable/malformed inspection fails qualification.
Add a deterministic execute-ATTACH-then-throw-result regression. Storage independent
narrow plan PASS; source amendment requires both reviews before any tests.

### C2 result: attachment inspection is unavailable

Author/root and independent recovery/storage source reviews passed, including the
ATTACH completion-error correction. Node24 and Python4 tests passed. Actual celld
failed the initial checks; a diagnostic-only stage-label refinement passed root/
storage rereview, Node24 rerun and a second exact-binary celld run. Failure is now
localized to `initial-attachments`: the fixed pragma_database_list() query returns
`not authorized`. The exact TEMP existence query succeeded on the initially empty
connection. Positive TEMP detection and the ATTACH candidate were NOT reached;
do not claim runtime coverage for them. No pass predicate was weakened.

Evidence: `/private/tmp/foks-connection-state-celld-{a,b}/qualification.json`,
`/private/tmp/foks-connection-state-results.json`, frozen four-file hashes
`/private/tmp/foks-connection-state-sha256-c.json`. The same official celld binary
SHA92ededd2bcfebeb1e78cc2d97219f00214a79eb6606f0eef48208fca3f98c15a
was used. The controller always terminated its disposable processes.

This leaves connection activation blocked. A production remedy needs an allowed
read-only attachment inventory or a separately proved fresh-connection lifecycle
that seals all attachment/TEMP capability before any application work. An arbitrary
exception from ATTACH is insufficient proof of absence. The current fixture is a
repeatable failed capability gate, not a production adapter. trusted_schema,
exclusive connection ownership and invalidation before loans remain open.


## C3: qualify attachment presence through the allowed table-list interface

Proposed root-owned probe amendment, not a production activation design. Keep the
failed C2 artifact and its unavailable database-list inspection intact as evidence.
The next fixed query is `SELECT CASE WHEN EXISTS(SELECT 1 FROM
pragma_table_list WHERE schema NOT IN ('main','temp')) THEN '1' ELSE '0' END AS value`.
SQLite's table-list implementation enumerates every attached schema, including its
schema table. Verify this empirically on empty attached databases before relying on
it; an empty attachment must not disappear merely because it has no user tables.
The audited celld source allows table_list, unlike database_list. Runtime remains
a mandatory gate because this source has already differed from observed behavior.

Change only the fixed connection-state probe and focused tests. Preserve its exact
single-bit shape checks, initially empty TEMP requirement, per-candidate owned
rollback, qualified TEMP positive detection, ATTACH post-effect absence check and
all controller success predicates. Replace attachment inspection with this candidate
query explicitly; do not silently fall back if it is denied or malformed. No serving
catalog, schema/profile digest, ownership capability or production route changes.
Successful C3 establishes another inspection prerequisite, not connection readiness.

Native tests must independently attach an empty in-memory database and an empty
file-backed database in a temporary directory, assert detection before any table
creation, then after creation and after DETACH. Test multiple attachments with an
empty one remaining after detaching the nonempty one. Retain the ATTACH-effect then
cursor-exception negative test. Preexisting empty attachment must fail before any
candidate; TEMP-only state must not be mistaken for an attached database. Existing
spoofed/malformed/denied-query cases remain required. Independent direct PRAGMA
inventory assertions in tests distinguish the candidate's output from its oracle.
No filesystem path comes from an HTTP input; the actual worker still attempts only
the fixed in-memory attachment, and its disposable process always terminates.

This query has a bounded result, not a constant-time or bounded-engine-work proof.
SQLite table-list may initialize view/virtual-table metadata and enumerate schema
objects even under EXISTS. It is not yet authorized as an arbitrary-schema startup
admission operation. Future activation must account for that work and establish the
reviewed schema/lifecycle ordering, exact runtime controls, trusted_schema policy,
exclusive connection ownership and invalidation before arbitrary SQL loans.

Root plan self-review: no empty-attachment blind spot may be assumed away; no
exception constitutes absence; no post-operation check proves an earlier callback
was safe. Keep C2 failed evidence distinct from any C3 result. After independent
plan review, implement; then root and independent source reviews before Node/Python
and pinned celld/Moto validation. Record frozen sources and actual positive TEMP
outcomes if reached. Commit implementation only; keep Markdown unstaged.


C3 root and storage independent plan/source reviews PASS before validation. The
frozen two-file change passed19 Node tests and4 Python controller tests. Actual
pinned celld/Moto qualification PASSED: qualified TEMP table accepted and detected;
explicit TEMP table/view/trigger denied; fixed ATTACH denied and independently
checked absent afterward; final TEMP empty and owned rollbacks restored state.
This supersedes C2's missing inspection prerequisite, not its recorded failure.
The binary SHA is unchanged. Evidence: /private/tmp/foks-connection-table-list-
results.json and /private/tmp/foks-connection-table-list-celld-a/qualification.json.
No source correction was needed during tests. Connection lifecycle sealing,
trusted_schema policy and production activation remain separate requirements.


## C4 design proposal: seal one synchronous transaction before caching readiness

This is a proposed next integration design, not implementation authorization or a
passed gate. It depends on restricted-profile import/readiness admission and the
reviewed owned List workspace. It avoids requiring a connection identity API that
celld does not currently expose: the first qualified route re-establishes its
connection invariants within each synchronous transaction and never carries an
activation capability across an event, await, rollback, or transaction boundary.
Only deployment-owned code may hold the raw storage reference. A private adapter
must expose fixed typed operations, with no arbitrary SQL, callback, migration,
configuration, or connection-loan method during the admitted interval. JS private
fields alone cannot revoke aliases: the worker composition and every reference to
storage must be audited as part of the proof. A test host with externally retained
aliases is an adversarial fixture, not a production ownership guarantee.

The entry sequence is: reject unsupported routes and bounded structural inputs;
enter the one owned transaction; establish the selected FK/CHECK/recursive policy;
inspect empty TEMP and attachment absence; compare the bounded exact schema and
artifact/profile identity; prove tenant/incarnation/ownership/revision and current
credential; run the fixed typed domain operation; verify the same transaction's
final coordinates and deadline; produce an owned pending result. Setting changes
that SQLite requires outside a transaction occur at construction, then must be
read and checked inside every transaction. A mismatch causes refusal, never a
mid-operation attempt to repair settings. Exact admission query order and charging
must be frozen with the adapter plan; these checks count toward the unchanged
2,048 SQL-call bound and end-to-end deadline. A later caching optimization needs a
separate lifecycle proof; an unchanged main cookie is insufficient.

The imported database must already have passed the immutable-copy R2/R3 inventory.
For the first route, prefer logical import into fresh trusted DDL rather than opening
an arbitrary raw SQLite image in the serving process. Even a constant-result
pragma_table_list query can initialize schema metadata. Do not use that query as
the first safety check on an unrestricted attacker-controlled schema. Unknown
views, virtual tables, statistics policy, SQLite-owned data and platform state
must have explicit admission treatment before opening the serving interval.

The unreadable trusted_schema setting is still a design gap, but does not by
itself require a celld fork. A candidate policy is to demonstrate that the exact
trusted DDL and fixed catalogs have equivalent permitted behavior with that setting
ON and OFF: inventory every reachable schema function, collation and virtual
module, allow only reviewed builtin behavior, and audit the pinned platform's
registered extension set for overrides. Native differential tests must exercise
constraints and triggers under both settings using the pinned engine version,
compile features, and registered function/collation/module set and flags, or
explicitly reviewed differences. A generic Node or system SQLite comparison alone
is insufficient. Actual celld must exercise the same behavioral fixtures without
pretending to have read its setting. No regex-only
function inventory, generic extension allowance, or assumption that all SQLite
builtins are harmless constitutes this proof. If the inventory finds a meaningful
setting dependency, retain refusal and design a platform control explicitly.

A schema/configuration change, migration, raw-storage escape, reentrant domain
entry, foreign callback, exception or Wasm trap invalidates the whole interval.
Do not recover by checking that TEMP is empty afterward: a callback may create,
use and remove a shadow table before the check. No asynchronous callback is allowed
inside the transaction. Expected domain rejection has its separately reviewed
identity-bound savepoint behavior; infrastructure failure cannot become a durable
domain rejection. Publication remains gated on outer commit, S3 durability sync,
remaining deadline, and ownership validation. Pending output binds the completed
transaction and instance generation, retaining its memory owner; the transaction
admission capability is unusable after transaction exit. Any post-sync ownership
check is a new sealed check, never revival of the old readiness. Invalidation
during sync discards pending publication. An overlapping reply already proved
under its committed operation is not a new mutation or a perpetual freshness
promise. This proposal creates no public network operation or zero-loss
replication claim.

Plan self-review: testing a wrapper with a fake exclusive connection proves only
wrapper behavior. Required integration evidence includes real worker reference
ownership, real celld transaction affinity, preexisting TEMP and empty attachment
refusal, setting mismatch, attempted nested/loaned access, fixed schema changes,
trap/rollback disposal, exact native response/error parity and SQL counts including
all entry/exit checks. R0 descriptors alone do not establish any of these. Freeze
concrete files and the complete call graph, obtain independent plan review, then
both source reviews before running tests or claiming C4 completion.


C4 independent design review (quota) accepted the direction with two refinements:
pinned-engine and registration equivalence for trusted_schema, and explicit pending
output generation/expired admission ownership across sync. Root reviewed and
incorporated both. This remains a design proposal pending concrete implementation
paths/call graph and plan review; it does not authorize a new serving factory yet.


## C4a proposed pinned SQLite policy qualification

Plan gate only; no implementation or tests yet. This independently useful factory
prerequisite qualifies the exact trusted schema and SQL catalogs under the
otherwise unreadable celld trusted_schema policy. It does not create readiness
or permit arbitrary SQL in the production adapter. Six proposed diagnostic
paths under tools/foks-sqlite-policy are sqlite_api.py, qualify.py,
test_qualify.py, worker.mjs, worker.test.mjs and run-local.py. No production
dependency, public wire field or platform patch is introduced.

Source reconnaissance: celld0.6.2 locks rusqlite0.31/libsqlite3-sys0.28.0, whose
bundled SQLite is3.45.0. This differs from the repository native3.53.2 and must
not be substituted. Downloaded locked crate checksum matches
0c10584274047cb335c23d3e61bcef8e323adae7c5c8c760540f73610177fc3f; source only has
been inspected. finish_open installs no extension unless the explicit sqlite_vec
compatibility flag is enabled; the first profile excludes that flag. Its source
only permits the reviewed builtin function/collation/module registration set,
and denies application load_extension. Freeze source hashes and exact bundle
compile defines from build.rs; compare runtime compile-option inventory to the
actual pinned celld binary before accepting equivalence. A source release tag
alone is not evidence of the binary's build flags.

The native diagnostic compiles a private library from the checksum-pinned
SQLite source into a new disposable directory, using fixed reviewed definitions
and a scrubbed compiler environment. A small ctypes wrapper specifies every
argument/result type, holds authorizer callbacks through connection close,
finalizes every statement on exceptions, and never enables extension loading.
All input DDL/catalog bytes must match compiled-in SHA256 pins from the existing
trusted profile exporter before parsing/execution. No observed database supplies
expected definitions. The tool may read the fixed exporter artifact directory,
but cannot accept an arbitrary existing SQLite database.

Create two fresh in-memory databases from identical trusted FOKS+authority+
platform DDL, with trusted_schema ON and OFF. Enumerate typed table metadata and
prepare EXPLAIN INSERT/UPDATE/DELETE covering every writable column/table plus
every fixed scalar/page catalog statement with bounded neutral bindings. This
compiles constraints, defaults, index expressions, partial predicates and trigger
programs without executing arbitrary data writes. Collect SQLite authorizer
function dependencies and complete EXPLAIN rows with bounded row/text totals;
compare ON/OFF programs after ignoring only demonstrably nonsemantic formatting
(if any; default exact equality). Reject unknown virtual tables, collations,
functions or preparation failures rather than waiving their dependency. Inspect
function_list flags for each reachable name/arity and record exact builtin and
innocuous/deterministic properties; not every builtin is assumed innocuous.
Positive and negative synthetic fixtures prove that a non-innocuous schema
function is detected and that triggers, expression indexes, CHECK/defaults and
partial-index dependencies are actually visited. These fixtures are diagnostic
only and never enter the trusted serving profile.

Preparation equivalence is necessary, not a blanket claim of full behavioral
equivalence. Add fixed real DDL behavioral cases for FK/CHECK/recursive behavior
and dependency-revision triggers, using isolated rollback transactions; compare
ON/OFF row outcomes, exact rejection categories and unchanged state after failure.
Existing full native/Go conformance remains separate. Record every excluded
statement and fail completeness if the trusted inventory is not covered.

The actual celld worker gets only the same frozen trusted artifacts and a private
capability before routing. One disposable actor installs trusted FOKS/authority
objects atop independently checked pinned platform objects; it does not recreate
or modify platform-owned objects. Fixed operations collect bounded compile-option
inventory and EXPLAIN programs for application DDL/catalogs and run the same
behavior fixtures. No arbitrary SQL/path/body input or production data is accepted.
Compare permitted application program/dependency behavior to the native engine
fixture; platform-internal statements are source/inventory-qualified separately,
not bypassed through a relaxed authorizer. Refused diagnostic introspection is
recorded as a missing proof, not silently treated as equality.

Controller follows the reviewed disposable celld/Moto recipe: fresh root, dummy
credentials, loopback listeners, pinned binary and artifact hashes, bounded total
clients, explicit process termination/reaping, no real AWS. Unit tests verify
artifact refusal, bounded native wrapper/resource cleanup and fixed worker
authorization-before-effects. Source reviews precede all compilation/tests.
A failed real engine comparison remains a failed qualification; it cannot widen
production admission or change the SQLite limits.

Root plan self-review: EXPLAIN alone is insufficient for runtime semantics;
behavioral fixtures and exact source/registration/build inventory are retained.
Authorizer callbacks are preparation-time observations, so every table mutation
shape and catalog must be compiled; regex function searches are not evidence.
A private rebuilt library is not automatically the deployed engine, hence the
actual compile-option/program comparison and binary source provenance gate.
Only after independent plan review should exact files/artifact pins be authored.

C4a plan clarification before independent approval: ON/OFF databases are built in
identical order and require exact EXPLAIN equality. Cross-engine/celld comparison
may differ in physical root-page numbers because platform objects preexist. Any
normalization must be narrowly limited to documented root-page operands of
OpenRead/OpenWrite/ReopenIdx, mapped through the independently enumerated exact
object identity; temporary/zero-root or unmapped operands cannot be guessed.
Keep original programs in evidence. All other opcode/operand differences fail
comparison until separately reviewed. Do not silently erase planner differences.


C4a implementation source review (before any compilation/testing): root author
review completed for six frozen diagnostic files. Independent storage review found
four proof gaps, all corrected and returned for final source review: explicitly
allow the trusted SchemaObjects query's NOCASE builtin (without widening BINARY
index policy); verify capability inside the actor as well as before lookup; compile
a private copy of the already checksum-verified SQLite source instead of reopening
the caller's path; and verify all native sqlite3_limit readbacks. Tests include
unknown collation rejection and direct actor authorization refusal. Full original
EXPLAIN/P4 programs remain in evidence and only proven object root operands are
normalized. Frozen manifest and hashes are /private/tmp/foks-sqlite-policy-manifest.txt
and /private/tmp/foks-sqlite-policy-sha256.json. No production admission or new
serving factory is introduced.


C4a first qualification failures retained (not a policy pass): Node4 and Python
structural3 passed. Pinned native3.45 compiled all462 trusted statements with
trusted_schema OFF, then the behavioral fixture used an obsolete positional
three-column namespace INSERT. Both reviewers approved naming its exact three
columns and relying on ten pinned DEFAULT0 counters. Fresh native-b then compiled
both462-program sets with matching behavioral outcomes; the sole raw-program
difference is scalar44 VOpen's printed runtime pointer to builtin
pragma_schema_version. Original raw programs remain retained.

The engine negative tests additionally found authorizer-only dependency collection
is incomplete for stored CHECK/default/index expressions: eight subcases did not
report policy_test during DML preparation. This is a diagnostic proof gap, not a
waived test. Proposed correction augments authorizer data with closed, typed
function-opcode P4 parsing and retains schema-install authorizer observations.
A narrowly pinned scalar44 builtin virtual-table pointer normalization requires
review; all other opcode differences remain failures. Native-a/native-b and
python-b logs under /private/tmp/foks-sqlite-policy-* preserve these findings.


C4a reviewed refinements now pass pinned-native qualification: all462 ON/OFF
programs and19 behavior steps agree; Python7/Node5 passed. Function dependency
collection combines schema-install authorizer callbacks, per-query callbacks and
closed function-op P4 parsing. Only the exact scalar44 pragma_schema_version
program may normalize one VOpen runtime pointer; every other operand remains.
The original negative-test failures remain in evidence.

Actual celld-a stopped at compile-option inventory: all50 noncompiler options
match, but private Apple clang21 and deployed clang15 identities differ. Both
reviewers approved separating only that exact compiler pair, recording
compiler_identity=false and raw inventories. No reproducible-build identity is
claimed; all program/behavior checks remain mandatory. Node6 passed afterward.

Actual celld-b then matched436 direct programs, stopping at scalar29 SAVEPOINT
because the platform deliberately refuses raw transaction SQL. The existing
pinned host maps scalar29/30/31 through nested transactionSync; it does not send
those SQL strings. Both plan reviews approved an explicit diagnostic distinction:
all462 native ON/OFF comparisons remain, while deployed coverage requires459
direct bytecode comparisons plus exactly3 separately reported mapped controls.
Mapped records require authorization-specific raw EXPLAIN denial and fixed nested
commit/rollback/outer-survival/outer-rollback outcomes with unchanged schema cookie;
they never carry a fabricated normalized bytecode hash. The current host.mjs SHA
is da8d96348f693bbcedfc0107b19335e6d7278e846fbbb6cef16c97657f5cd085,
checked before/after. This tests runtime control semantics, not a new invocation
of the Wasm domain ABI or durability proof. The mapped-control source correction
is frozen for independent review before rerunning. All prior failures and child
cleanup reports are retained; no production admission follows.


C4a final source gates and qualification: independent review caught that a runtime
could swallow rollback exceptions or discard callback return values while matching
row outcomes. Required identity sentinels and return markers now reject both,
with negative tests. Celld-c matched436 direct programs and3 mapped controls, then
exposed Transaction P4 generation drift after the diagnostic rollback DDL. Root
and independent quota plan/source reviews approved scheduling all459 direct
programs before the3 mapped controls, retaining original indices, exact set
validation, and every bytecode operand. No additional normalization was added.

Final frozen source passed Python9, Node8, native462 trusted_schema ON/OFF program
comparisons and19 behavioral steps, then actual pinned celld459 direct program
comparisons,3 mapped-control fixtures, and19 behavioral steps. Evidence is
/private/tmp/foks-sqlite-policy-results.json, with raw programs, all earlier failed
attempts, source hashes and child cleanup reports retained. Celld needed forced
termination after the harness graceful-cleanup allowance; this is cleanup, not a
failover measurement. Compiler identity remains explicitly false. This diagnostic
qualifies only the pinned corpus and runtime control semantics: it does not mint
production admission, establish durability, or prove15-second recovery. Six tool
implementation files are committed separately; this Markdown remains unstaged.

## C4 concrete integration proposal: private connection checks inside F3

Plan only, authored after C4a commit `a11dc0ec`; no serving implementation or
constructor is authorized by this section. C4a passed native 462 ON/OFF programs
and 19 behaviors, deployed 459 direct programs plus three explicitly mapped
transaction controls and 19 behaviors, Python nine and Node eight tests. That is
fixed-corpus policy evidence, not an exclusive-connection capability. In
particular celld still does not expose readable `trusted_schema` or a stable
application-visible connection identity. The failed a/b/c diagnostic runs remain
part of the evidence. C4 must be implemented inside the **one F3 owner**; it must
not introduce a second factory, public connection wrapper, transferable hash
receipt, or generic dispatcher.

### Ownership and exact implementation boundary

Recovery owns F3 orchestration and public/private entry selection. C4 supplies
private fixed checks and transaction-local fields on that owner. Proposed native
paths are `crates/foks-server/src/hosted_personal/factory.rs` (F3 owner) and its
private `connection.rs` and `connection/tests.rs` children. If the existing
`hosted_personal.rs` remains a file, its child module path is explicit; converting
it to a directory is unnecessary. Storage owns the R3 importer and its
`NativePersonalListImport`; C4 must consume that import through a server-private
seam, not reopen the input image or accept an arbitrary path/Connection. No
server-db-to-core production dependency is needed. The connection, writer lock,
canonical path identity, applicable StorageLease, imported artifact identity and
semantic witness live together in this server owner. `DatabaseWriterGuard` in
`writer.rs` is a cooperative process mutation lock; it does not prove other raw
SQLite handles disappeared. R3 creates a private destination, never hands out an
alias, and closes its import handle before transferring the sole serving handle.
No normal Writer, migration, maintenance, archive or operator handle shares that
serving destination. Lease/path rechecks cannot substitute for this ownership.

C4's conceptual signature is a private `enter_connection_checks(&mut self,
&Request) -> Result<ConnectionInterval<'_>, FactoryError>` reached only after F3
starts its transaction. The result borrows the same connection/owner, is neither
Copy nor Clone nor serializable, and has no connection getter, SQL method or
callback runner. Its fields are target, owner generation, transaction sequence,
schema cookie and compiled profile identity. The only consumer is the lexical
closed List implementation in F3. Prefer a lexical private local over a type if
that eliminates an otherwise unnecessary abstraction. Commit consumes/ends this
interval; it cannot be stored in pending output. F3 stores only observations and
its existing pending-output ownership. There is no `verified: bool` argument.

The celld counterpart is a private module/closure in the F3 deployment worker,
with one captured root storage reference and immutable compiled artifacts. It
must be a new serving integration, not an exported subclass of the mutable
qualification `tools/foks-metadata-spike/host.mjs` Kernel. That fixture publicly
exposes storage/catalog/instance, accepts fault controls and is not an ownership
boundary. No new generic JS factory API is proposed here. Freeze the final
serving module paths with F3 before source work; this C4 section does not turn the
diagnostic tools into production modules. Internal Wasm import closures may use
only the fixed scalar/page enums and the three already reviewed control mappings.
They never return storage, a cursor, arbitrary SQL execution or a host callback.

### Why a synchronous celld interval is the qualification unit

Pinned celld source `crates/celld/storage.rs::Cells` stores OpenCell connections
on the isolate; a turn holds its isolate lock. `finish_open`, `close` and
`open_at_epoch` can replace this mapping and clear cursors/caches. Internal
`storage_identity` is Rust-private and cannot be inferred from the JS storage
object's identity or a main schema cookie. In `js/harness.js::transactionSync`
the callback finishes synchronously before another event starts; nested root
calls use the active synchronous view. The callback result is returned unchanged
and a throw rolls back. The API itself does **not** reject a returned Promise.
Therefore the serving callback is a closed, non-async internal function. It
stores an internally constructed pending result in an owner-private slot and
returns only a module-private constant identity marker. An outer closed callback
checks `result === successMarker` before allowing COMMIT; it never reads `then`,
walks a prototype or reflects over an unknown result. A Promise, foreign object,
Proxy or object with a throwing/counting `then` getter fails identity without
executing that getter or any trap. No result object from an external callback is
accepted; there is no public callback entry. The pending slot itself is filled
only by the fixed internal constructor, with fixed data fields and no accessors.
No user function, logging hook, thenable getter,
foreign serializer, await, timer, network access, root-storage escape or arbitrary
Wasm export can execute inside this interval. Data crossing it is already typed,
bounded and free of user-defined accessors. Imports are nonreentrant except the
reviewed allocator and mapped control transitions. Busy/generation fields belong
to F3, not an additional C4 state machine.

This is a source/composition requirement, not merely a private-field convention:
review constructor parameters, all closures and handlers, alarms, admin routes,
prototype mutation, worker helpers and any retained `ctx` references. The first
worker has only the closed List entry and trusted construction path; unsupported
routes cannot reach storage. Platform-owned SQL is covered by the pinned platform
contract, not forbidden by claiming application exclusivity over celld internals.
Reactivation or replacement requires reconstruction from the persistent R3
binding and fresh checks; no prior JS receipt survives it.

### Fixed setup and per-transaction sequence

Construction occurs on the fresh trusted R3 destination, not arbitrary imported
DDL. Set `foreign_keys=ON`, `ignore_check_constraints=OFF` and
`recursive_triggers=ON` outside any transaction, and read back their typed integer
values. Native additionally sets `main.journal_mode=WAL`,
`main.synchronous=FULL` and `trusted_schema=OFF` outside the transaction, then
requires exact readbacks `wal`, integer 2 and integer 0 on its own pinned target.
A returned journal mode other than WAL is failure, not successful fallback. This
matches the existing native writer policy in server-db `connection.rs` and does
not inherit arbitrary imported-connection durability settings. Retain the
validated database-size/max-page-count, WAL reuse/journal-size and temp-store
limits; construction must not bypass Config validation. Keep configured busy
waits bounded and constrained by original remaining time at operation admission;
never add an unbounded lock wait or a fresh request timeout. Synchronous SQLite
commit/fsync retains physical ownership if it exceeds a cooperative deadline.
Celld uses the separately pinned C4a policy qualification; it must
never report that setting as observed. Disable extension loading/registration in
native construction, and exclude celld sqlite_vec compatibility. Constructors
accept no runtime function, collation, module, authorizer or SQL registration.
Remaining SQLite settings are enumerated in the separately frozen target policy;
this is not permission to use arbitrary connection defaults. If F3 needs a
changed setting, extend its exact policy and rerun qualification first.

Inside each synchronous transaction, before any authority/domain SQL:

1. Check the original Request deadline/cancellation and F3 busy generation;
   acquire no new Request and refresh no budget. Confirm the connection is in
   the transaction on native; celld reaches this code only in its owned callback.
2. Execute the fixed readbacks `PRAGMA foreign_keys`,
   `PRAGMA ignore_check_constraints`, `PRAGMA recursive_triggers`; require exactly
   one integer 1, 0, 1 respectively and EOF. Native also reads its
   `trusted_schema` integer 0, `main.journal_mode` text `wal` and
   `main.synchronous` integer 2. A mismatch is refusal, not in-transaction repair.
3. Require one canonical zero from
   `SELECT CASE WHEN EXISTS(SELECT 1 FROM temp.sqlite_schema) THEN '1' ELSE '0' END`.
   No name/type filtering: TEMP triggers, autoindexes and views also forbid entry.
4. Require one canonical zero from the C3 qualified inventory query
   `SELECT CASE WHEN EXISTS(SELECT 1 FROM pragma_table_list WHERE schema NOT IN
   ('main','temp')) THEN '1' ELSE '0' END AS value`.
   Empty attached databases are covered by their schema row. The query is safe
   only after trusted R3 schema construction: constant output is not a proof of
   bounded engine metadata work on a hostile raw SQLite file.
5. Read the typed main schema cookie; stream ordered main schema objects through
   the existing pure exact-union verifier and the target's compiled artifact.
   Native F3 uses an exact serving union, not ordinary Database's extension-
   permitting NativeRequired policy. Celld includes its ten pinned platform
   objects. Include malformed non-TEXT names/SQL and reject them rather than
   hiding them with LIKE or NULL filters. Keep the existing descriptor limits
   (1,024 objects, 2 MiB aggregate, 65,536-byte SQL, 256-byte identifiers and page
   bounds); consume each row before advancing, with no full actual-schema Vec.
   Re-read cookie; mismatch invalidates. First implementation verifies every
   transaction, with no cookie-only cache or activation proof reused across turns.
6. Verify the fixed R3 artifact/target binding and F3's tenant, incarnation,
   dependency revision and external ownership prerequisites using their typed
   seams. C4 does not mint those receipts. Only then may F3 perform its unchanged
   authority-shape/clock/classification ordering, proof and List operation.

All these statements, including schema pages, final checks and any post-sync
seal, count against the same original 2,048-call policy and deadline; no hidden
second allowance. NativeSql currently starts its own counter at zero, so F3 must
freeze a minimal shared/request-held call-counter seam or a checked remaining-
budget constructor before implementation. Do not subtract only on successful
return: attempted preparations/refusals also count. Host Rust/JS row copies and
schema verifier storage need pre-admission through the existing Request/host
budget; SQLite/V8 internals remain separately bounded/qualified rather than
claimed covered by Rust payload accounting. Exact schema-enumeration SQL,
readback transports, scalar types and count accounting must be frozen with the
F3 host source, not supplied by requests or mutable catalog objects.

### Exit, sync and invalidation

Before committing, F3 checks the dependency/fence observations and original
clock/deadline; C4 reads the same cookie and settings/TEMP/attachment checks again
as defense in depth. This final check does not repair an earlier storage escape:
any attempted escape/reentry/configuration/DDL already invalidates the operation.
No callback creates and removes TEMP unnoticed under the source ownership rule.
Cursors and borrowed views are dropped before COMMIT; failure rolls back and
latches F3 unusable where connection state is uncertain. An expected domain
rejection remains its typed path; policy/schema failures cannot be converted to
a durable domain rejection or native NotFound.

Transaction exit ends C4's interval whether success or failure. F3 pending output
retains original Request, generation and transaction observations, never a C4
capability or raw connection reference. On celld call root `storage.sync()` only
after the outer callback returns/commits; never start it from inside the active
transaction. `storage.rs::sync_sample` samples connection epoch+position and
`js/storage_ops.rs::op_storage_sync` requires the running event's same cell and
output-gate channel. That platform proof is necessary but not the application's
ownership/fence proof. Await retains F3 busy and memory ownership, while later
queued entries cannot mutate this owner. After sync, a fresh synchronous sealed
check validates the pending generation/current fence and unchanged dependency;
no data writes or floor advancement are allowed in that publication check. If a
required check would write, it belongs before the first sync or requires another
sync, never implicit publication. Before issuing the native local commit receipt,
require the WAL/FULL policy to have been verified on the actual connection
immediately before COMMIT and to remain unchanged under the closed owner; only a
successful COMMIT on that connection can produce the receipt. Setting mismatch,
commit error or an uncertain outcome produces no successful durability receipt.
Native local durability assumes the platform SQLite/VFS/filesystem contract and
is distinct from celld S3 durability; do not label it S3 durable. Physical work and retained output stay owned
after caller cancellation until completion/disposal; no hard filesystem deadline
claim. Sync failure, replacement, expiry or changed ownership discards pending.

### Prerequisites and narrow qualification sequence

R2 StorageScanComplete is insufficient. The first F3 constructor needs R3's
whole finalized personal-only package, fresh trusted destination, exact artifact
binding and native conversion/authority semantic witness including unrelated
rows. Team, SSO and federation packages are rejected; org64 is later expansion.
R4 is required before any externally initiated domain write, import mutation or
maintenance is exposed. The first List-only owner admits only the reviewed
private authority clock-floor write; prove that write preserves the R3 witness,
restricted physical bounds and dependency semantics. Full-history maintenance,
restore, credentials/key rotation and app writes cannot sneak through the List
connection. Replacement/import destroys the old owner and constructs a new one.
No network switch occurs on C4a evidence or a test-only R3 witness.

Implement C4 with F3 rather than as a standalone unguarded helper commit. Before
source edits, freeze F3's exact host module paths, original-call-counter seam and
R3 receipt signature; review the combined entry/exit call graph. Then qualify:

- Native real exclusive imported destination: wrong settings, TEMP table/view/
  trigger, empty ATTACH, extra/corrupt schema, target mismatch and stale R3 binding
  all refuse before authority/List. Attempted concurrent owner/open, reentry,
  loan and panic cannot publish; retained work keeps leases until physical exit.
- Actual celld same fixed worker/module: positive closed List, seeded qualified
  TEMP shadow, attachment capability/absence, wrong settings, nested controls,
  arbitrary import/export misuse and Promise-return injection. Require exact
  refusal origins, original call/deadline bounds, rollback and no response.
  Run target-specific native parity and immutable Wasm/catalog/source hashes.
- Pause at COMMIT/sync/fresh-check/publication boundaries; inject failed sync,
  changed generation/fence/dependency, expiry and replacement. Prove the old
  interval cannot be reused and no frame escapes, including slow consumers.
- Requalify C4a exact corpus when reachable SQL/schema/functions/engine/platform
  artifacts change; its compiler exception remains only the reviewed clang21 to
  clang15 pair with all other options exact. New C4 probe SQL must be included in
  the pinned diagnostic corpus before claiming policy closure. Do not generalize
  the one VOpen-pointer normalization or mapped-control handling.

Author self-review: this plan keeps one exclusive F3 owner, distinguishes actual
connection lifetime from JS object/cookie identity, and does not bless the current
qualification Kernel as a serving boundary. Full per-transaction schema checking
is intentionally conservative and may exhaust the existing deadline/call budget;
measure it before any caching proposal. The celld proof still depends on exact
platform source/binary and closed worker composition. R3/F3 source signatures,
shared SQL accounting and serving artifact generation are explicit pre-code
gates; distributed ownership, R4 mutation coverage and deployment activation
remain independent requirements. Requested root/independent plan review next.


Root-review corrections and author rereview: native construction/receipt now
explicitly requires actual WAL/FULL policy with bounded Config/lock behavior;
closed callback completion uses only private identity comparison, with a getter/
Promise/Proxy rejection test that proves no accessor/trap executes. Add native
DELETE/MEMORY journal and synchronous OFF/NORMAL mismatch tests, constructor
readback failure, and commit-failure/no-receipt checks to the qualification matrix.
No mode repair is allowed after a transaction has begun.

Production profile identifiers and immutable artifacts remain a separate concrete
**pre-code** freeze gate with F3. Current `read_schema::QualificationProfileId`
values `0x80000001/0x80000002` and generated qualification descriptors must never
be accepted as serving profiles. The reviewed production target IDs, exact schema/
catalog/ABI/policy/artifact hashes and target-specific union must be generated and
reviewed before constructing a serving owner; caller-provided hashes or changing
a qualification enum name cannot mint this authority. C4a qualification does not
authorize choosing these production values implicitly.

C4 corrected concrete plan: root independent review **PASS** after explicit native
WAL/FULL durability policy, accessor-free private completion marker validation and
production-profile/artifact pre-code gates. Author rereview complete. This approves
the design for integration into recovery's F3 combined plan, not a detached C4
implementation or serving activation. F3/R3 combined review and source gates remain;
no implementation, build or test was performed for this plan amendment.
