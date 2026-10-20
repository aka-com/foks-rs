# Current-head projection: schema56 preparatory phase

Plan only, 2026-10-15. Server predecessor schema55. No implementation or tests before plan review. Markdown remains unstaged. This phase changes local SQL access paths, not FOKS wire types, ciphertext, signed data, key semantics, historical retention, or logical quota charging.

## Problem and bounded objective

M0 measured the exact NameConflict query at 23 VM steps with no retained matching-name history and 18,023 with 1,000 retained versions; adding unrelated current heads did not increase that particular measured plan. The existing `kv_dirent_listing` index is over historical rows, so a negative current-name lookup can inspect obsolete candidates. Do not generalize the measurement into a claim that every head lookup scales with unrelated heads.

Make NameConflict and current live directory listing use a fixed-width current projection. Negative lookup must be independent of unrelated heads and retained history; positive lookup inspects at most the matching current candidates needed to exclude its own dirent. Listing cost may grow with its requested output, not discarded historical versions or tombstones. Preserve historical lookup behavior and all old file-version references.

## Storage and exact invariants

Add `kv_dirent_current`, a STRICT, WITHOUT ROWID derived table with:

* `uid` (33 bytes), `parent_id` and `dirent_id` (16 bytes each), primary key `(uid,parent_id,dirent_id)`.
* `version >= 1`, `name_mac` (32 bytes), `creation_time >= 0`, and `node_id` (17 bytes, first byte not zero).
* A foreign key `(uid,parent_id,dirent_id,version)` to the exact historical `kv_dirents` row, `ON DELETE CASCADE`.

There is exactly one projection row for each head whose referenced historical node has a nonzero first byte, and zero projection rows for tombstones. Every copied field equals the historical row selected by that head; no exact ciphertext/dirent blob is duplicated. Keep the original head schema/primary key unchanged, avoiding compatibility churn for existing positional head inserts. No UNIQUE name invariant is added: historical databases may contain duplicate live names, and conflict detection must retain its existing behavior.

Add nonunique indexes `(uid,parent_id,name_mac,dirent_id)` and `(uid,parent_id,creation_time,dirent_id)`. These cover seek and ordering; exact bytes are fetched from the historical primary key only for selected current rows. Projection/index storage is derived metadata, not a new charged user object. Existing namespace byte/object and directory/dirent counters continue counting the existing base tables, unchanged.

The projection is not an independently writable application API. All normal mutations, maintenance cascades and recovery paths derive it through triggers. It does not authorize dropping a historical file reference or deleting an object.

## Trigger contract

Use small keyed triggers, never a namespace/directory scan or all-history aggregate:

1. Head INSERT: select the one historical row for NEW's full key and insert the live projection if non-tombstone.
2. Head UPDATE: delete OLD's projection by exact primary key; insert/update NEW's projection from its exact historical key. Handle identity as well as version changes, including transitions to/from tombstones and explicit rollback of a head in maintenance/tests.
3. Head DELETE: delete OLD's projection by exact primary key.
4. Historical INSERT: if a head already references this exact version, populate its live projection. This is needed for a legal transaction using deferred foreign keys that inserts the head before its historical row.
5. Historical UPDATE of key or projected fields: remove the old matching projection and rebuild only affected OLD/NEW current-head keys from their referenced historical rows. Changes to exact encoded bytes alone need no projection work. A historical non-head update must not remove a newer current projection.
6. Historical DELETE: existing foreign-key cascades remove a referencing head and the projection; head DELETE trigger remains idempotent. Test both explicit deletes and namespace/directory cascades.

Historical INSERT/UPDATE must only affect a projection when the head's full version identity matches, not merely the same dirent ID. This is crucial for old-version edits. Deferred valid sequences must end with a complete correct projection regardless of head/history insert order; invalid foreign-key sequences still fail and roll back. Trigger ordering must not depend on SQLite's unspecified ordering among separate AFTER triggers. Projection triggers neither change existing quota triggers nor recursively write base heads/history.

## Native and kernel queries

* NameConflict: query only `kv_dirent_current` by uid/parent/name MAC, exclude the supplied dirent ID, `LIMIT 1`, return `SELECT '1'`. Native code tests presence so the scalar type change is internal; the Wasm catalog already requires text-encoded integer results. Existing tombstone shortcut and replay/quota/error ordering remain unchanged.
* Initial and MAC-cursor List: seek current-name index, preserve `ORDER BY name_mac,dirent_id`, cursor `name_mac > supplied`, and the existing limit. Fetch exact bytes by the selected projection's historical key. Existing duplicate-MAC boundary behavior is preserved.
* Time-cursor List: current-time index, preserve `creation_time >= supplied` and `ORDER BY creation_time,dirent_id`. No new public cursor or timestamp normalization.
* Point current-dirent/head reads already use the exact head key; leave them unchanged. Full version-vector traversal remains inherently proportional to visited live entries and is not claimed constant-time.
* Historical `kv_dirent_at_name` remains unchanged, including its `directory_version` restriction and `ORDER BY version DESC LIMIT 1`. Keep the original `kv_dirent_listing` index. A proposed historical composite index is deferred: changing the access path may select a different tied historical row, so that requires its own baseline/tie review. This phase must not conflate historical lookup with current-name lookup.

Use named production SQL constants for all redesigned queries so VM-step tests execute the actual strings. Expose the NameConflict constant for the M0 differential test. Keep the typed catalog's literal byte-identical and assert equality with that constant: this avoids a new dependency cycle or cross-crate `include_str!` packaging dependency in this preparatory phase. M1 can move the statement to its eventual common catalog when the domain kernel becomes the production implementation. Recovery agent owns the catalog/test edit, coordinated with the DB change.

Prefer ordinary indexed SQL first. Inspect EXPLAIN and measure actual VM work; if SQLite chooses a historical join as the outer loop, use a specifically reviewed query shape or named `INDEXED BY` for the current projection. Do not assert scalability from index existence alone. Initial listing must not materialize/sort every head before LIMIT.

## Migration, failure and backup compatibility

Fresh initialization and schema43–55 migration apply the identical schema56 projection SQL. Extend the supported migration upper bound to55; keep publication-policy migration conditional on predecessor <=54 (currently it is unconditional inside the old <=54 branch). Run projection backfill after prior migrations, in the existing IMMEDIATE upgrade transaction; update user_version only after complete validation. No partial projection is visible to serving readers.

Backfill by joining heads to their exact referenced historical primary keys and selecting live rows, not scanning/reducing every historical version. Validate missing, extra and mismatched projection rows against the authoritative join using fixed-width values, plus foreign-key checks. Missing/corrupt head targets, conflicting schema objects or failed index construction must abort the entire transaction, preserve predecessor schema/version/data and leave no new triggers/indexes. Existing counters, chunk incarnation/authority/publication provenance and SSO state must be unchanged.

The migration is a bulk upgrade job requiring appropriate writer/installation exclusion and disk headroom; it is not included in the ordinary 15-second request budget. Upgrade before traffic routing. Do not use an existing open old binary as a migration strategy; old binaries reject schema56 upon opening. Normal upgraded startup must not rebuild or rescan the projection on each activation. One fixed-width row and two indexes per live head add metadata; retained versions add no projection entries.

Add `tests/fixtures/remove_kv_current.sql` to reconstruct genuine predecessor schemas by removing projection triggers/indexes/table. Update every predecessor fixture that starts from a fresh current database, including schema54's publication test and older reconciliation/accounting/schema fixtures. Preserve original historical tables/indexes exactly. A genuine55 populated fixture must include live heads, tombstones, overwritten names, duplicate historical MACs, multiple directory versions, historical file references, external chunk metadata, and nonempty unrelated state. Compare complete canonical schema with fresh56 after migration and prove exact historical/current results and counters survive.

Root owns the small server backup compatibility fixture update; provide the exact removal helper invocation for all predecessors <=55. Run existing offline/online backup and restore gates: schema55 backup is accepted via private upgrade, schema56 backup restores projection correctly, backup hydration preserves the projection and original live/historical bytes, read-only opens do not silently migrate, unknown/newer schema remains refused. No new backup format or object-store semantics.

## Acceptance matrix and review gates

Before tests, author review plus root and independent recovery-agent review of production SQL, triggers, migration and fixtures. Then reserve Cargo slot and run:

* Genuine55→56 and43–54 regression migrations, empty/populated canonical parity, version refusal, failed migration rollback and reopen; verify deferred head-first/history-first insert order.
* Head creation/update/delete, rename, tombstone/resurrection, non-head historical edits while a newer version stays current, deferred primary-key/head relocation, direct projected-field edits, current-history deletion and namespace cascades; compare projection to the authoritative join after each committed operation and after rollback. Existing quota/accounting and file-retention tests must stay unchanged in outcome.
* Current-name duplicate conflict, own-ID exclusion, exact replay before stale-precondition/quota checks, permissions, late-batch rollback; first/MAC/time listing order and limits, duplicate MAC/time boundaries, and historical lookup exact-byte parity against a genuine55 baseline.
* Production SQL VM/EXPLAIN tests across retained matching-name versions 0/100/1,000/10,000, unrelated live heads 0/100/1,000/10,000, and tombstones separately and combined. Keep the sought current result/output size fixed. Negative and positive conflict work and first/page listing work must stay bounded as unrelated dimensions grow; record observed steps and chosen indexes. Measure head update trigger work across the same dimensions. Use deterministic VM steps, not CI wall-clock thresholds.
* M0 actual-schema native differential and celld/Wasm vectors with the updated catalog, plus current-history cost probe. Use the exact shared NameConflict SQL comparison to prevent future drift.
* Server KV/authenticated transfer/pinned Go flows and backup/restore compatibility; targeted clippy and format checks. No broad protocol test rewrite or wire golden changes are expected.

Separate implementation commit after all gates; Markdown stays unstaged. Remaining whole-tree traversal, atomic namespace bootstrap contract, ownership/intent envelope, real SQL interruption and full hosted route coverage remain M1/M2 work, not implied by this optimization.

## Implementation review record

Root and recovery-agent independent plan reviews passed. The first implementation
uses schema56, `kv_dirent_current`, five keyed triggers, the two planned current
indexes, and `KV_NAME_CONFLICT_SQL` exported for catalog equality. Fresh creation
applies this after the prior schema migrations, keeping the raw historical-schema
fixtures genuine. All fresh-to-predecessor reconstruction paths explicitly remove
the projection. Root supplied the backup compatibility reconstruction hook.

Author review completed before testing: checked deferred head/history insertion and
primary-key relocation in both orders, old-version edits behind a newer head,
projection deletion/resurrection/cascades, migration rollback, all-base-table data
preservation, historical tied-name bytes, and exact production-query VM assertions.
The source manifest is `/private/tmp/foks-current-head-manifest.txt`; the kernel
catalog/test overlay is maintained separately from unfinished authority-envelope
work. Independent implementation review and test results are pending.

Independent implementation review found an `INSERT OR REPLACE` edge with
`recursive_triggers=OFF`; the head INSERT trigger now clears its keyed projection
before deriving the new row. Its regression replaces a live head with a tombstone.
The legacy local celld proof also now treats the table as derived: base writes alone
reconstruct it, and the full snapshot hash still rejects mismatched or directly
supplied projection state. Its native vector uses the real PutDirents DB method;
new Node assertions cover direct writes, forged after-state and derived-only changes.
Recovery-agent independent review passed both corrections. The complete source
manifest has 20 paths, with the two metadata-core paths sourced from a frozen
projection-only overlay rather than unfinished authority code. No test results
are claimed yet.


## Validation record (2026-10-15)

Validated the reviewed 20-file snapshot in
`/Users/raymond/Code/foks-rs/.claude/worktrees/current-head-validation`, based on
`fbcf6640`. The metadata catalog and differential test came from the separate
projection-only overlay, excluding unfinished authority-envelope work. Source
mapping and SHA-256 records are `/private/tmp/foks-current-head-sources.tsv` and
`/private/tmp/foks-current-head-source-hashes.sha256`. Markdown remains unstaged.

Both direct and independent implementation reviews passed before testing. Test
runs exposed only historical-fixture maintenance: the schema54 test now expects
`SCHEMA_VERSION`, and legacy external-adoption/schema43 writer fixtures remove
the projection before reconstructing their old schemas. Each correction received
author and independent review before rerunning. An unused legacy test import was
removed and an existing export list formatted; no production behavior changed in
those corrections. The first local-network run was blocked by sandbox listener
permissions; the authorized loopback rerun passed.

Passed gates:

- Full server-db suite: 181 tests passed, one existing ignored test.
- Backup unit/compatibility: 7; live backup/restore: 5.
- External encrypted storage/adoption/GC: 12; transfer-reader/memory: 2.
- Pinned official Go client: 3 (inline, external files, OIDC).
- Small-file KV conformance: 6, including pagination beyond the v0.19 page boundary.
- Exclusive writer/schema upgrade: 1; legacy celld proof Rust: 5.
- Native metadata differential/cost tests: 3; Node metadata host: 3;
  Node replay authority: 7, including derived-only tampering.
- Wasm build with a declared 64 MiB maximum; actual local celld/Moto replay:
  all 17 vectors matched exact native replies/outcomes and full final state on
  schema56. Evidence: `/private/tmp/foks-current-head-celld/qualification.json`.
- All-target Clippy with warnings denied for server-db, server, testkit (celld-proof)
  and metadata-core (native); targeted rustfmt and `git diff --check`.

The exact production negative NameConflict query used 14 SQLite VM steps throughout
all measured dimensions, including 10,000 same-name retained versions combined
with 10,000 unrelated live heads. Positive lookup used 19; initial/MAC/time limited
listing used 30/33/34; the keyed head-update statement used 354. The tests also cover
10,000 tombstones and obsolete-history updates behind a current head. VM opcode
counts do not measure B-tree depth, disk I/O or wall-clock tail latency. This proves
the tested query/trigger shape and compatibility, not hosted HA, AZ recovery or
hard execution deadlines.

Command transcripts are `/private/tmp/foks-current-head-*.log`. The failed initial
fixture/sandbox transcripts are retained separately from passing reruns. No tests
or protocol validators were weakened to obtain these results.
