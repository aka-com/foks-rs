# A2a historical reads and public Usage counters

Draft 2026-10-15; author reviewed, awaiting independent review. Plan only. Keep
this document unstaged. Parent: `HOSTED_METADATA_READ_BUNDLE_PLAN.md`. Two
separate implementation commits/migrations are proposed so an unresolved lookup
tie cannot block correcting Usage, and a counter bug cannot be hidden in an index
change. Schema numbers57/58 are provisional until the execution order is fixed.
No FOKS wire, crypto, historical verifier, quota policy or retention change.

## Evidence and compatibility contract

Native `server-db/src/kv.rs::kv_dirent_at_name` joins the exact historical
`kv_directories` version and filters `(uid,parent,name_mac,directory_version)`,
ordering only `e.version DESC LIMIT1`. The existing `kv_dirent_listing` index
orders `(uid,parent,name_mac,dirent_id,version)`; the measured retained-history
scan/sort is documented in the parent plan. It is not a current-head query.

Pinned Go is exactly `go-foks@v0.1.9` from the oracle go.mod, not the separately
cached newer pseudo-version. Its `server/kv-store/dirent.go:367` helper appends
`ORDER BY E.version DESC LIMIT1` to the same historical directory-version join.
`server/sql/foks_kv_store.sql:87` indexes host/party/directory/name/version and
specifies no dirent-ID tie. Therefore equal-version duplicate-name selection is
unspecified in Go SQL; source inspection cannot establish a universal Go-server
tie winner. A client wire-compatibility test is not proof of PostgreSQL tie order.
Preserve the supported current SQLite implementation's selection after explicit
qualification, retain Go's declared maximum-version semantics, and disclose this
existing ambiguity. Stop if the SQLite baseline does not select the proposed
ascending-ID tie consistently across the supported query plans/insertion orders.

Native `kv_usage` currently decodes every retained small-file or symlink box,
counts every such node and sums `ciphertext.len()`. It then counts every retained
upload, including incomplete/reclaiming ones, and sums each upload's logical
`chunk_count` and `chunk_bytes`. The latter already accounts for disaggregated
ciphertext length through schema50 triggers. These are five semantic counters;
namespace accounted bytes/objects/directories/dirents are different quantities.
A1's private `QuotaCounters` must remain distinctly named.

Go `server/kv-store/usage.go:15,277` stores the same five categories. Its small
write increments by encrypted `DataBox` length (`file.go:111`); large write by
chunk Data length, first chunk increments file count (`file.go:353`). This plan
preserves the existing Rust retention/deletion interpretation, rather than
silently copying a different Go GC policy. Exact retries cannot charge twice.
Metadata wrappers, object-store encoded length, filesystem/S3 physical bytes,
orphans and logical quota accounting are not public ciphertext Usage bytes.

## Group H: historical index and shared scalar lookup

Migration57 adds only
`kv_dirent_historical_name(uid,parent_id,name_mac,directory_version,version DESC,dirent_id ASC)`.
Do not drop the existing listing/reference indexes in this group: other query
plans and migration footprint need separate evidence. No UNIQUE constraint and
no historical row deletion. Pin the production native query to the new index
only after tie qualification, using explicit `ORDER BY e.version DESC,e.dirent_id ASC`.
Preserve all equality predicates, exact historical directory join and result
bytes. Publish a server-db SQL constant and assert literal equality against the
kernel catalog entry, as for NameConflict, until the final shared query layer.
Catalog IDs append; existing statement IDs never change.

First qualify the old query on genuine schema56 databases: swapped insertion
order, opposite IDs, equal/different versions, tombstone and live nodes with the
same name, multiple historical directory versions, deleted heads, large unrelated
history and SQLite ANALYZE plan changes. Compare complete chosen exact dirent and
historical directory bytes before/after; test a tie whose different followed node/directory keys
would change permission status (the parent key is necessarily identical for one
parent/directory-version equality tuple). The pinned Go live client must read the resulting
namespace without message/status changes. If a running upstream Go server is not
available, report source-derived max-version compatibility and the unavailable
tie-oracle gate explicitly; do not invent a deterministic upstream tie guarantee.

Cost acceptance uses the exact production SQL, positive/negative lookups at
1/100/10000 matching historical versions and unrelated names/dirent IDs/directory
versions. Require indexed seek with no temporary sort and VM work independent of
retained history (not a disk-latency claim). Measure insertion/update/delete VM
work and disk/index growth; single index maintenance remains logarithmic B-tree
work, not mathematically constant runtime. Old binaries reject the new schema.

Migration tests: real56 predecessor, canonical fresh/migrated schema equality,
rollback on injected index-build failure, preserved FK/projection/counter state,
backup/restore old56 and new57, and synthetic older fixtures explicitly removing
the new index before reconstruction. Activation schema/catalog digests change as
one reviewed release. Index construction is offline migration work, never hidden
inside a15-second warm request or unqualified cold activation.

## Group U: public semantic counters, typed derivation, corruption behavior

Next migration adds nullable `kv_nodes.ciphertext_bytes INTEGER CHECK(ciphertext_bytes>=0)`
and six nonnegative STRICT integer namespace fields: `usage_small_count`,
`usage_small_bytes`, `usage_large_count`, `usage_large_bytes`,
`usage_large_chunks`, and `usage_unvalidated_small_count`. The sixth is an internal
error sentinel, not a public field. NULL means the exact node has not been
successfully decoded; never treat its bytes as zero in a successful Usage result.

A nullable sentinel preserves existing failure boundaries: malformed legacy nodes
currently allow opening the DB but make Usage fail. Migration must not make them
silently successful, discard their ciphertext, or prevent every unrelated route
from starting merely because Usage could not decode them. Decode failures retain
NULL and increment the invalid count; allocation/storage errors abort migration.
Hosted activation still rejects malformed nodes during its invariant validation.

Derivation belongs to shared typed Rust code, not a caller-provided byte count or
JavaScript business logic. Add a pure helper in `foks-proto/src/kv.rs` that decodes the exact
`KvSmallFileBox` with existing size/allocation guards and returns checked u64
ciphertext length; SQL callers then perform checked i64 conversion. Both DB and
metadata core already depend on proto: add no server-db-to-metadata-core edge.
The helper has no RPC new-write cap or database quota policy. Native `put_kv_node` derives this from the exact bytes before
new insertion. A future hosted PutSmallFile/Symlink command calls the same helper;
A2 Usage does not by itself enable those writes. Preserve native RPC validation,
including the Go2064-byte ciphertext write cap and its error ordering. Existing
larger valid imported nodes may be up to the current64KiB DB/proto cap and remain
readable/accountable; do not apply the new-write cap during migration/backfill.

For robust invalidation without a SQLite Snowpack extension: insert exact node
with NULL derived length, then set the checked length in the same transaction via
an exact `(uid,node_id,exact_node)` predicate. A BEFORE UPDATE guard rejects any
changed exact bytes unless NEW.ciphertext_bytes IS NULL. Thus a typed replacement
first writes `(exact,NULL)`, then derives/stamps that exact row; it cannot silently
retain a stale length. Do not use nested AFTER triggers that race to invalidate
and account the same outer NEW value: their order is unspecified. Immutable
normal node writes are unchanged except for transactional derivation. A known
identical retry uses the existing row; if unvalidated, validate/stamp exact same
bytes before success. Invalid new raw DB input may be rejected by the typed API;
public RPC already decodes it. Raw direct counter edits remain outside the supported
database contract, as with existing quota counters.

Node INSERT/DELETE changes small_count and either bytes or invalid_count; UPDATE
of uid/ciphertext_bytes transfers the contribution from OLD to NEW. Exact-byte
change must pair with NULL as guarded above, and exact_node alone does not also
charge bytes. Keys/count columns use checked STRICT integer arithmetic. Use a
BEFORE INSERT key-collision guard rejecting every existing node/upload key, because
recursive_triggers=OFF can suppress REPLACE's implicit DELETE accounting. This
also rejects raw conflicting INSERT ... ON CONFLICT DO NOTHING/UPDATE: BEFORE
INSERT runs before the conflict action, so never claim otherwise. Preserve
supported typed API behavior through its existing transaction-local exact-byte
pre-read: exact retry returns/validates the existing row, mismatch returns its
existing conflict, and insertion follows proven absence. An intended supported
update must use explicit UPDATE under that same transaction; audit/convert any
UPSERT call site before installing the guard. Current production node/upload
initialization already follows that pre-read pattern. Existing randomized accounting
fixtures using raw kv_nodes UPSERT need explicit SELECT/UPDATE/INSERT branches,
plus tests that raw conflicting REPLACE/UPSERT fail without counter drift. Do not
rely on enabling recursive triggers globally. Namespace/head UPSERTs are unaffected.

Upload INSERT/DELETE/UPDATE OF uid,chunk_count,chunk_bytes transfers its complete
large contribution. Existing chunk triggers remain the sole source of each
upload's logical byte/chunk totals, including inline/external conversion. Thus
chunk insertion/deletion updates the upload row and its namespace contribution
in one transaction. Parent upload deletion subtracts the old total once; cascading
chunk deletion must not subtract twice when that parent row is gone. Namespace
cascades may find no destination row and are harmless. Explicitly test both cascade
orders/deferrable identity relocation rather than infer trigger execution order.
Updating complete/reclaiming/time alone does not change Usage. No physical-object
publication, uncertainty, GC or replica work changes these semantic counters.

Bootstrap/backfill runs under the existing exclusive schema transaction. Stream
nodes in bounded keyset pages and decode one exact payload at a time, checking
length before allocation; preserve malformed bytes with NULL as above. Initialize
namespace aggregates with checked sums, using existing upload counters. Check
those upload counters against logical chunk rows during this already-offline
migration (including external ciphertext length), so migration cannot bless a
stale source aggregate. Compare every new aggregate with an independent scan;
abort on arithmetic overflow or inconsistency. All DDL/data/user_version changes
roll back together. No mutation or remote I/O occurs between pages outside that
transaction. Cost is proportional to retained data and must be measured/offline.

Production native `kv_usage` becomes one UID-primary-key read. Absent namespace
returns zero as before. `usage_unvalidated_small_count>0` returns the same existing
invalid-stored-small-node failure mapped to TransactionRetry; counters must never
produce a partial success. Checked integer conversion preserves errors. Encode
via existing `foks_proto::KvUsage`, preserving field order and exact response
bytes. Add a separately named kernel public Usage scalar returning these five
fields, with the sixth checked internally. It is distinct from QuotaCounters and
runs under the real current personal authority/fixed-revision wrapper.

## Group U validation and ownership

Freeze a test-only copy of the old scanning Usage algorithm as the differential
oracle. Compare exact encoded KvUsage and error statuses after each operation:
small and symlink insertion; duplicate and conflicting retry; unlinked nodes;
partial/final/reclaiming uploads; repeated chunks; inline/external conversion;
retained overwritten versions; cancelled upload cleanup; chunk/upload/namespace
cascades; exact-node mutation invalidation and explicit revalidation; malformed
and oversized legacy payload; identical ciphertext lengths with different exact
bytes; namespace transfer; rollback/fault injection and arithmetic boundaries.
Use real encrypted client fixtures plus direct malformed-row fixtures, not only
hand-authored counters. Empty namespaces and unknown UID retain zero semantics.

Migration: genuine57 predecessor, older chained upgrades, fresh canonical schema,
invalid-node sentinel preservation, injected failure during backfill and before
user_version publication, latest backups and hydrated restore with external
chunks, migration hooks in read-only/startup/trusted opener, and old-binary refusal.
Update every synthetic downgrade fixture to remove only the newly added schema.
Legacy native-backed celld proof must reconstruct derived counters from reviewed
base mutations/triggers; derived rows/fields cannot become a direct write escape.

Cost tests execute actual read/trigger SQL at1/100/10000 small nodes, uploads and
retained chunks, verify fixed public Usage VM shape and bounded per-row trigger
work, and inspect production EXPLAIN plans. Time migration separately and measure
retained-data disk growth. Verify native, Wasm and actual celld return identical
Usage bytes/status, including a pinned Go client's existing usage/read/write flow.
No claim about host/AZ durability follows from these counter tests.

Suggested split: storage owner server-db migration/typed derivation/triggers/read
constant/tests; metadata owner shared helper/catalog/Wasm query and proof-derived
state compatibility; root native service/Go comparison and backup integration.
Freeze interfaces/overlap before code. Each group gets author review, independent
review, then focused tests/lint and its own implementation commit. Markdown stays
unstaged. No implementation authorized by this draft.

## Private page boundary inherited by both groups

Pages have separate row and byte limits: a valid64KiB imported node can reduce a
page below64 rows. Return explicit exhausted/continuation state; private row count
below64 never implies public final_page. Charge the next candidate's transient
payload before copying, including a row that does not fit the remaining page and
must be fetched on the next page. Measure actual SQLite/celld cursor buffering,
not only the final binary buffer. Until qualified, activation may explicitly
reject out-of-profile legacy nodes; the native path must not acquire a new silent
unsupported response. Public final_page remains based on the total assembled
requested page and the established native rule.

## Group H reviewed interface and source preparation

Root and independent recovery-agent plan reviews passed2026-10-15. Root approved
the shared-selection CTE refinement: the identical pinned key-selection literal
returns only(dirent_id,version), retaining the exact historical directory-existence
join and tie predicate. Native wraps it in a single statement fetching unchanged
exact payloads; private catalog wraps it with length-first CASE bounds. This avoids
imposing A1's4096-byte profile on native payloads. Test literal equality on the
selection and VM/EXPLAIN on both complete production statements.

Source prepared in the isolated historical-name-validation worktree atf30c70c8
plus frozen A1 core context, leaving main/A1 untouched. Group H's11-path manifest
is `/private/tmp/foks-historical-name-manifest.txt`. Author review completed; no
build/test has run. New gates include the old schema56 query as an independent tie
oracle (both insertion orders, live/tombstone ties, ANALYZE, unrelated history),
canonical rollback/reopen and data preservation, positive/negative complete-query
costs at1/100/10000 rows, old/new insert/update/delete VM work and added index pages,
and native8192-byte directory retention versus private pre-copy NULL projection.
All observed tie differences remain a stop condition before commit.

Group H direct root and independent recovery-agent code reviews passed before
execution. Frozen hashes: `/private/tmp/foks-historical-name-source-sha256.json`
(all11 final blobs plus their A1 predecessor hashes). H-only integration delta:
`/private/tmp/foks-historical-name-after-a1.patch`, generated against the frozen
A1 tree rather than the older f30 baseline. After A1 commits, create a fresh
validation worktree at that commit under QMUX_WORKTREE_ROOT; verify overlapping
base hashes, apply the H-only delta, and verify all11 final hashes. Any changed
base requires inspection/review, not a forced patch. Keep the original source
snapshot immutable. Native DB/metadata, backup/restore and pinned Go gates precede
commit. Catalog41 has no caller yet; real celld qualification is mandatory before
A2 enables it, and this group does not invent another private probe command.

## Group U proposed interface freeze (2026-10-15)

This section freezes proposed ownership and interfaces for review; it does not
authorize implementation before root and independent approval. Build its isolated
source tree from the reviewed H snapshot or, preferably, the committed H release;
never overwrite frozen A1/H bytes in their validation trees. Final validation
must use the combined committed predecessor and exact reviewed U delta.

- Protocol helper: `KvSmallFileBox::ciphertext_length(exact: &[u8]) ->
  foks_proto::Result<u64>`, using the existing decoder and checked length
  conversion. It has no new-write ciphertext cap and does not silently impose a
  smaller legacy decode profile. Both typed native admission and future hosted
  admission use the same exact-byte derivation. Existing decode errors retain
  their categories; migration distinguishes Snowpack AllocationLimit from invalid
  stored data and aborts on resource exhaustion.
- DB migration: private `schema/kv_usage.rs::migrate(&Transaction) -> Result<()>`
  and `schema/kv_usage.sql`, schema58 after57. Add the nullable node derivation
  and six namespace fields before the trigger/backfill phase; install the same
  final SQL through fresh creation and upgrades. Keep the whole migration inside
  the existing exclusive schema transaction. Scan one borrowed exact payload at a
  time with length inspection before allocation; do not hydrate all node records
  or all namespace payloads into a Rust collection. Upload-counter validation is
  a separate independent logical-chunk scan, including external ciphertext sizes.
- Typed writer: existing `Database::put_kv_node` signature stays unchanged. Its
  transaction-local pre-read distinguishes identical retry, conflict and absent
  insertion. Identical unvalidated records can be derived/stamped only with an
  exact-byte predicate, and this mutated identical-retry transaction must explicitly
  commit (the existing read-only retry returns without commit). Namespace creation/error ordering remains consistent with
  the existing API; public RPC decode/2064-byte validation stays in the service.
  The internal stamp helper is not a public caller-provided byte-count API.
- Public read: export `KV_PUBLIC_USAGE_SQL` returning the five semantic columns
  plus the internal invalid count in this order: small_count, small_bytes,
  large_count, large_bytes, large_chunks, unvalidated_small_count. Use canonical
  CAST(... AS TEXT) columns for the shared catalog ABI; native and hosted adapters
  both validate signed i64 decimal then checked unsigned conversion. This keeps
  one byte-identical production SQL constant without allowing JavaScript Number
  rounding or relying on rusqlite coercion. Keep all existing
  Database/ReadDatabase/ReadSnapshot `kv_usage` signatures and KvUsage encoding.
  No row yields the existing zero result; nonzero invalid count yields the exact
  existing `Invalid("stored KV small node")` failure. QuotaCounters stays separate.
- Metadata integration: append `Statement::PublicUsage` after all committed
  catalog IDs, with an exact-SQL equality test against the exported DB constant.
  `PersonalPoint::PublicUsage` decodes signed SQL counters with checked conversion,
  applies the same corruption rule and emits the existing exact encoded KvUsage.
  Real personal authentication/fixed revision remains mandatory. Its error mapping
  must be compared to native service status, not guessed from the private
  QuotaCounters variant. No new public RPC or write command is introduced.

Storage agent owns the proto helper and server-db migration/triggers/read/writer,
new independent scan oracle, genuine57 predecessor/removal helper and cost tests.
Recovery agent owns metadata catalog/point/Wasm/actual-celld vectors and old local
proof derived-field handling after coordinating the frozen A1/H context. Root owns
backup compatibility fixture integration and native service/pinned Go comparison
wiring. Existing server-db tests that insert malformed pseudo-boxes through the
typed API must use real encoded boxes, or deliberately insert raw legacy rows if
corruption is the scenario; do not weaken production derivation to retain invalid
fixtures. Raw conflicting INSERT OR REPLACE/UPSERT tests assert rejection and no
counter drift, while supported typed exact retries still succeed.

Two code reviews precede all tests. Gates include the old scanning oracle's exact
bytes/status, independent SQL trigger invariants and overflow rollback, genuine57
and older upgrade chains, backups/hydrated restore, constant-query VM work,
native/Wasm and actual-celld comparison, and pinned Go usage/read/write behavior.
No schema58 commit can precede the separate H57 gate.

Group U independent plan review identified one derived-state replay boundary: the
old local celld proof cannot reconstruct node ciphertext length from SQL alone.
Root approved extending that qualification bundle with an immutable pure Wasm
helper that calls the same Rust exact-box derivation. Before inserting/updating a
node, replay derives and validates the supplied count or performs NULL then the
validated stamp; forged lengths never become trusted inputs. Namespace counters
remain exclusively trigger-derived and full after-state hashes remain mandatory.
No JavaScript Snowpack decoder is introduced. Recovery owns the exact ABI/artifact
contract and its independent review before implementation. Tests must use the
actual helper artifact for valid, malformed, forged-count and ordinary node replay
cases; this additional dependency is explicit in the legacy proof bundle.

## Group U exact derivation artifact and legacy replay contract

Root approved this design direction after the independent review found that SQL
triggers cannot reconstruct decoded ciphertext length from an opaque Snowpack
box. The following concrete ABI and replay contract require root and storage
review before implementation. The old native-backed proof keeps its existing
small-file write scope; it does not gain production authority or HA guarantees.

The storage owner adds the pure proto helper specified above. The recovery owner
adds two qualification exports to the existing metadata-core Wasm artifact:
`small_box_input_alloc(u32) -> u32` and `small_box_ciphertext_length() -> u32`.
The first rejects zero or more than65536 exact input bytes before allocation and
otherwise uses the existing owned INPUT buffer. A rejected allocation clears any
previous input/output so a later call cannot reuse an earlier box/result. The
second consumes that buffer,
checks the same bound again, calls only the shared proto helper and performs a
checked i64 conversion. It never invokes SQL, domain, authority or caller imports.
Status0 publishes canonical nonnegative ASCII decimal through the existing
`output_pointer`/`output_length` accessors (at most19 bytes, no JSON Number).
Nonzero status publishes a bounded fixed diagnostic code; every such outcome is
an infrastructure/replay failure, never a durable domain rejection or zero count.
Malformed, allocation-limit, overflow and out-of-profile inputs remain failures.
A synchronous host wrapper rejects recursion, validates exported memory ranges,
copies output before another Wasm allocation, checks canonical decimal/i64 range,
and has no method exposing the other kernel exports to replay callers. Imported
SQL/domain functions throw unconditionally in this dedicated instance.

The legacy proof deployment requires an explicit `--node-decoder-wasm` artifact
path. Its controller validates the module's declared memory maximum (64MiB),
records SHA256 with schema/controller/Worker source hashes, and copies the exact
artifact into the immutable deployment bundle. `worker.mjs` imports the module
and creates one private decoder per cell; an immutable decoder function is passed
to Authority at construction. Requests cannot supply a module, decoder, code,
length override or alternative schema. Source and Wasm must be rebuilt together
for the reviewed release; no downloaded/fallback decoder and no JavaScript
Snowpack implementation are allowed. The existing overall8MiB proof JSON/snapshot
budget remains distinct from the exact-box65536-byte qualification limit.

Replay retains complete typed old/new rows and full-state hashes. Before any
base-row mutation, validate every changed kv_nodes row's next exact payload and
derived field. Check the exact payload's base64 text length against the65536-byte
ceiling before decoding/copying it, enforce canonical base64 and decoded length,
then invoke the pure Wasm helper. The supplied next ciphertext_bytes must be a
canonical nonnegative integer equal to the derived value; a NULL next value,
wrong count, corrupt box, oversized box or decoder failure rejects the whole
transaction. An old NULL value is permitted only as existing snapshot state; a
valid identical-node revalidation may replace it with the checked derived value.
No supplied count is trusted merely because it matches the requested after hash.

For a valid insert/update, SQL first inserts or updates the exact bytes with
ciphertext_bytes=NULL, then stamps the verified length under the exact
(uid,node_id,exact_node) predicate in the same transaction. Preserve the complete
old-row compare-and-update predicate and require the expected changed-row count
for both phases. This satisfies the new exact-byte invalidation trigger and
charges namespace counters solely through triggers. Namespace semantic counters
are excluded from directly editable identity fields; they are never replayed as
base writes. Final full-state equality includes all node derivations and all six
new namespace counters, so omitted or forged derived effects still fail before
receipt/revision publication. Existing current-head derived-table protections
remain unchanged. Any late failure rolls back all base rows, stamps, counters,
receipts and revision changes.

Tests use actual compiled Wasm, not only a mocked decoder: valid small/symlink
boxes (including legacy payloads above the new-write cap), exact retries,
NULL-to-valid revalidation, supplied-NULL and forged-count rejection, corrupt and
oversized input, memory/output bounds, decoder import/trap failures, a batch whose later invalid
node prevents every mutation, and an injected later SQL failure that rolls back
an earlier valid change. Node SQL tests retain
full-state/receipt/revision checks. Run the real native proof and pinned Go flow
with the decoder artifact, plus personal PublicUsage native/Wasm/actual-celld
exact-byte and error-status vectors. These are functional compatibility gates;
repeat neither the earlier tenant-capacity matrix nor its timing claims.

Recovery owns metadata-core catalog/PublicUsage scalar and tests, pure Wasm
exports, the proof decoder wrapper, legacy authority/Worker/controller changes,
proof-derived-field guards and actual-celld fixtures. Storage owns the proto
helper and all production DB derivation/migration/triggers/read code. Root owns
native service error/status comparison and backup/Go integration. Coordinate
catalog IDs against committed H57; never modify the frozen A1/H snapshots.
The identical-unvalidated native retry must explicitly commit after its exact
stamp: the old already-validated retry path's implicit transaction rollback
cannot persist newly derived metadata. Independent review of the storage API
and this complete artifact/replay contract precedes code; two implementation
reviews precede every build/test gate.

## Group H final validation record (2026-10-15)

Final isolated tree: `historical-name-final-validation`, committed A1 predecessor
cbaf3ef3. All11 frozen predecessor hashes matched, the H-only delta applied cleanly,
and all11 reviewed final hashes remained unchanged through validation. No source
correction was required. Root, recovery-agent and quota-agent code reviews passed
before tests. Source manifest/hash files remain the ones recorded above.

Passed: full server-db suite184 tests with1 preexisting ignored test; metadata-core
native suite25 tests; backup unit compatibility7; backup/restore integration5;
external storage integration12; pinned Go v0.1.9 live flows3; focused raw historical
Get/Usage compatibility1. Native all-target Clippy for server-db/server/metadata-core
and Wasm-lib Clippy passed with warnings denied; Wasm build, changed-file rustfmt
and diff whitespace checks passed. The first localhost integration attempt was
denied by sandbox listener permissions before test behavior; the authorized rerun
passed without source changes.

Historical query VM work stayed [69,69,26,39] across matching-history dimensions
1/100/10000 (positive parent versions, absent version and absent name); predecessor
positive query work grew49/2128/210028. Insert/update/delete VM counts were
[154,121,135] before and [164,135,144] after the index. Added pages were0/10/828 in
the fixture with three retained-history dimensions. These are bounded SQL-work
measurements, not request-latency or recovery guarantees. Genuine56 tie selection,
canonical upgrade/rollback, exact native8192-byte historical directory payload and
private pre-copy bounds all passed. The unused private catalog41 has no command
caller; actual-celld execution qualification remains required before A2 enables it.

Logs: `/private/tmp/foks-historical-db-focused.log`, `foks-historical-db-all.log`,
`foks-historical-metadata-all.log`, `foks-historical-backup-unit.log`,
`foks-historical-backup-external-authorized.log`, `foks-historical-go-live.log`,
`foks-historical-kv-conformance.log`, `foks-historical-clippy-native.log`,
`foks-historical-clippy-wasm.log`, `foks-historical-wasm-build.log` (all under
`/private/tmp`). Markdown remains unstaged; root integrates and commits the exact
reviewed11-path implementation separately from Group U.

Group U narrow author-audit refinement: raw upload parent-key relocation is not a
typed supported operation. Existing chunk total triggers cannot safely infer a
deferred uid/file_id relocation in arbitrary parent/child order. Root and recovery
independently approved rejecting changed upload keys with a BEFORE UPDATE guard;
unchanged-key assignments and normal counters/state/time updates remain allowed.
Tests attempt parent-first and child-first relocation inside transactions and
assert complete rollback without counter drift. Native APIs and legacy proof do
not relocate upload keys (the proof excludes upload rows entirely). This replaces
the earlier suggestion of supporting arbitrary deferred upload relocation; normal
small-node namespace transfers and deletion cascades are still exercised.

## Group U metadata/replay author review

After H committed as 9e8632e9, recovery authored and reviewed the 16 paths in
`/private/tmp/foks-usage-metadata-manifest.txt`; no build/test has run. Catalog42
adds PublicUsage with exact SQL equality coverage. Scalar vectors compare native
public-route bytes, integer precision above2^53, malformed-node TransactionRetry
versus the private Corrupt category, and exact revalidation. Direct aggregate
editing in the precision fixture tests the integer ABI only.

The replay implementation validates every next node before any mutation, checks
both base64 encoded and padding-derived decoded size before allocating, then
performs NULL→exact-predicate stamping inside the existing transaction. The new
pure decoder tests use actual compiled Rust Wasm; negative artifact tests redirect
an export to the existing Rust trap or rejected allocator to verify trap/output
handling without a JavaScript business decoder. Other tests cover supplied NULL,
forged counts, corrupt/oversized nodes, no-effect batch rejection, late SQL
rollback, legacy NULL revalidation and all six derived namespace fields. The
legacy import unit deliberately preserves malformed existing snapshot data; that
is distinct from accepting a newly replayed malformed node, which fails closed.
Root and storage implementation reviews passed before validation. The final two
fixture corrections replace obsolete placeholder small-node bytes with valid
encoded boxes; the replay vector includes both file and symlink node types.

Two additional narrow refinements passed root and independent recovery review:
node uid/node_id UPDATE rejects a changed key if the destination already exists,
closing UPDATE OR REPLACE implicit deletion with recursive triggers disabled while
preserving unchanged-key/noncolliding moves; migration inspects borrowed exact
length and refuses >64KiB with QuotaExceeded before decoding, matching the existing
maximum supported opener policy. Malformed in-bound rows retain NULL. Both have
rollback/no-drift regressions; the pure proto helper retains its policy-free API.

### Group U native validation record (2026-10-15)

The complete reviewed 40-file union was frozen at `9e8632e9` in
`.claude/worktrees/usage-counters-validation`. The source manifest and SHA-256
record are `/private/tmp/foks-usage-validation-manifest.txt` and
`/private/tmp/foks-usage-validation-sha256.json`. The source and isolated checkout
match exactly; Markdown remains outside the implementation manifest.

Author review and root/recovery independent reviews passed before testing. Tests
found two fixture-only omissions: the typed retry fixture needed a real claimed
user, and an existing node read assertion needed to expect the new valid test box
encoding. Both corrections received author and independent review before reruns;
production code did not change during these gates.

Native gates passed: shared proto derivation 1; full server-db suite 192 plus one
existing ignored benchmark; backup unit compatibility 7; live backup/restore 5;
external storage 12; explicitly enabled pinned Go live flows 3; authenticated raw
KV Get/Usage compatibility 1. Default all-target Clippy for proto/server-db/server
passes with warnings denied. All 31 Rust paths in the union pass rustfmt, and the
isolated diff passes whitespace checks.

The actual public query consumed 23 VM steps for a present namespace and 9 for an
absent namespace at 1, 100 and 10,000 small-node/upload/chunk rows per kind. The
measured node invalidation and chunk-update statements used 132 and 440 steps at
each size. EXPLAIN selected the namespace primary-key lookup. Separate observed
migration times were 14.8, 16.1 and 182.4 milliseconds; these local measurements
are not hosted latency guarantees. Page growth was zero in this fixture, which
already contained free pages after the synthetic downgrade, not evidence of zero
storage overhead.

Logs are `/private/tmp/foks-usage-{proto-focused,db-all-c,backup-unit,backup-external,go-live,kv-conformance,cost,clippy-native,rustfmt}.log`.
Native/Wasm metadata, exact decoder, and actual-celld replay qualification remain
the recovery agent's next gates on this same snapshot; this record does not claim
those gates have passed.


Group U metadata/replay qualification completed on the exact40-file isolated
schema58 source union based on9e8632e9. Shared metadata26 native tests,8 proof/
ledger unit tests, actual Rust and explicitly enabled pinned Go personal fixtures,
29 Node/actual-Wasm tests, and150 actual celld vectors passed. The latter comprise
33 Rust personal,31 Go personal,17 mutation differential and69 prior authority
vectors. Native all-target and Wasm lint, bounded Wasm build and relevant Rust
format checks passed. Full immutable source/artifact/catalog audit is recorded in
`/private/tmp/foks-usage-final-qualification.json` and
`/private/tmp/foks-usage-validation-sha256.json`. The initial test-only API compile
failure and sandbox loopback failure are preserved; corrected/re-authorized runs
passed without production changes. Storage's separate schema/native/live/Go gates
are recorded by its owner. These seeded-cell fixtures establish semantic parity,
not a persistent service, synchronous SQL interruption, throughput or HA results.
