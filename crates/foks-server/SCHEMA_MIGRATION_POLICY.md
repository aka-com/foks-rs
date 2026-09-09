# FOKS server schema migration policy

Status: design only. The product has not shipped, so the current
`SCHEMA_VERSION = 16` is a development counter, not a compatibility promise.
This policy must be implemented and its baseline frozen before accepting the
first durable deployment.

## First shipped schema

Immediately before the first release candidate:

1. incorporate every required durable structure, including the host-key
   generation/rotation ledger in `KEY_ROTATION.md`;
2. squash the development SQL into one reviewed baseline;
3. set the first public schema version to `1` and record a canonical schema
   fingerprint;
4. generate a golden empty database and a representative populated database;
5. declare that exact version/fingerprint the oldest supported durable format.

Before that freeze, schema changes update the baseline directly and no migration
is written. There is no compatibility guarantee for databases produced by
pre-release commits, consistent with the workspace's current policy.

After the freeze, a committed schema is immutable. Every durable change gets a
new monotonically increasing integer version and a forward migration. Version
numbers are never reused, reordered, or renumbered on a release branch.

## Operating policy

- One server process and one migrator may own a database. The existing writer
  sidecar lock must be acquired **before any writable database open,
  initialization, bootstrap, or migration**, not after bootstrap.
- `serve` never performs an implicit migration by default. It accepts exactly
  the current schema, reports older/newer versions clearly, and leaves bytes
  unchanged.
- `foks-server migrate --plan` is read-only. `migrate --apply` requires an
  explicit destination for a verified pre-migration backup. An opt-in
  `--migrate-on-start` may be added for supervised single-node deployments but
  uses the identical backup and migration engine.
- Migrations are forward-only. Downgrade means restoring the pre-migration
  backup with the older binary. There are no down-migration scripts.
- A newer database is always refused. An older version outside the documented
  support window is refused with the required intermediate binary/version.
- Initial support should be all released versions because the history is small.
  A bounded window can be proposed later only with an LTS/export policy.
- Automatic destructive migrations are prohibited. Column/table removal,
  reinterpretation, or cryptographic format changes use expand/backfill/verify/
  contract across releases so the previous release remains recoverable during
  the rollback window.
- Migration code never contacts the network, rotates keys, changes hostchain
  identity, or rewrites signed canonical protocol bytes unless the migration's
  security review explicitly proves the byte-preservation requirement.

## Proposed files and components

```text
crates/foks-server-db/src/schema/
  baseline.sql                    # first shipped schema only
  fingerprint.rs                 # canonical sqlite_master hashing
  migrations.rs                  # ordered registry and graph validation
  migrations/
    v0001_to_v0002.sql
    v0002_to_v0003.rs            # bounded Rust transform when SQL is insufficient

crates/foks-server-db/src/
  migration.rs                   # plan/apply API, journal, invariant hooks
  process_lock.rs                # lock acquired before all writable opens

crates/foks-server-db/tests/
  migration_registry.rs
  migration_golden.rs
  migration_failures.rs
  fixtures/schema-v0001-empty.sqlite.zst
  fixtures/schema-v0001-populated.sqlite.zst

crates/foks-server/src/operations/
  migration.rs                   # key-aware preflight and backup orchestration

crates/foks-server/src/bin/foks-server.rs
  # migrate --plan/--apply command wiring

tools/foks-server/
  make-schema-fixture.sh          # deterministic reviewed fixture generation
  test-migrations.sh

crates/foks-server/
  MIGRATION_RUNBOOK.md            # operator steps, space, rollback, verification
```

SQL migrations are embedded at compile time. Rust migrations are allowed only
for bounded transforms that cannot be expressed safely in SQLite SQL; they
must page by primary key, be restart-safe within their version step, and avoid
holding unbounded rows/blobs in memory.

## Durable metadata

The baseline adds a strict migration ledger in addition to SQLite's
`application_id` and `user_version`:

```text
schema_migrations
  version INTEGER PRIMARY KEY
  migration_id TEXT UNIQUE NOT NULL
  source_sha256 BLOB(32) NOT NULL
  applied_at INTEGER NOT NULL
  binary_version TEXT NOT NULL

migration_journal
  singleton = 1
  target_version
  phase
  last_primary_key             # only for explicitly chunked transforms
  updated_at
```

`application_id = 0x464f4b53` continues to reject foreign databases.
`user_version` is the fast current version. The ordered ledger authenticates
which migration source produced it. Startup recomputes a canonical fingerprint
of tables, indexes, triggers, foreign keys, strictness, and relevant check
constraints and compares it with the compiled fingerprint for that version.
Unknown tables are permitted only through an explicitly documented extension
namespace; silent schema drift fails closed.

Migration source hashes change only by adding a superseding migration. Editing
an already released migration is a build/review failure, even if the resulting
SQL appears equivalent.

## Apply protocol

1. Acquire the exclusive process lock before opening SQLite read-write.
2. Reject symlink/non-regular database, key, backup, or lock paths using the
   same path policy as normal server operation.
3. Read `application_id`, `user_version`, ledger, schema fingerprint, SQLite
   version, page size, journal mode, integrity status, and current storage/WAL
   sizes without changing them.
4. Resolve exactly one contiguous path from current to target. Print the plan,
   estimated temporary space, migration IDs, and whether a restart boundary is
   required. Refuse gaps, branches, duplicate versions, or unknown history.
5. Require free space of at least the database + WAL + estimated transform
   scratch + backup safety margin; values and override policy belong in the
   runbook.
6. Create a coherent database plus encrypted-key backup in a new destination,
   validate it, and `fsync` its completion marker. The operator root remains
   external. Refuse to overwrite a prior backup.
7. Run `PRAGMA quick_check`, foreign-key checks, and application invariants.
8. Apply each ordinary migration in `BEGIN IMMEDIATE`; update its ledger row,
   fingerprint, and `user_version` in the same transaction. A crash yields
   either the old or new step, never a partially declared version.
9. For an approved chunked transform, first land an expand-only schema step;
   resume idempotent batches using `migration_journal`; verify counts/hashes;
   then use a final short transaction to switch authoritative reads and version.
10. After every step run foreign-key and step-specific invariants. At target,
    run `quick_check`, representative semantic reads, host bootstrap/key-manifest
    verification, Merkle-root/hostchain binding verification, and a WAL
    checkpoint.
11. Publish a non-secret report containing versions, migration IDs/hashes,
    durations, row counts, database/WAL sizes, backup path, and checks—not user
    IDs, names, protocol blobs, encrypted payloads, or keys.

If any step fails, stop. Do not attempt an automated reverse migration or custom rollback script. Transactional
steps roll back; chunked steps resume from their authenticated journal with the
same binary. If post-migration verification fails, quarantine the result and
restore the pre-migration backup before running the older binary.

## Migration author rules

- New constraints are validated against all existing rows before becoming
  authoritative.
- New non-null columns use a safe default or an explicit bounded backfill.
- Table rebuilds reproduce every index, trigger, foreign key, strict-table
  property, check constraint, and row count; tests compare canonical schema.
- Large encrypted blobs are copied byte-for-byte and hash-compared. They are
  not decrypted merely to move schema.
- Signed hostchain, Merkle, user/team chain, certificate, receipt, and request
  bytes remain exact unless a separately reviewed protocol operation replaces
  them. A database migration cannot manufacture a new signed history.
- Clock/time unit, signed/unsigned integer, uniqueness, collation, and NULL
  changes require explicit boundary fixtures.
- Migrations are deterministic and idempotent at their documented resume phase.
- Business behavior supports both expand-phase representations until the
  contract release; do not rely on a multi-process mixed-version deployment.
- Backup retention never deletes the mandatory pre-migration backup. Normal
  automatic retention applies only after an operator marks the migration
  verified and the rollback window expires.

## Test matrix and release gate

For every released source version:

- migrate empty and representative populated golden databases to current;
- verify exact final schema fingerprint and semantic snapshots;
- start the current server and run the full public client/server conformance,
  team, recovery, KV, backup/restore, and isolation suites;
- restore the pre-migration backup with the old root/binary and prove rollback;
- attempt current→current (no-op), future→current, foreign `application_id`,
  missing/edited ledger entry, schema drift, corrupt/truncated DB, missing key,
  wrong root, and insufficient disk;
- inject process termination before backup completion, before/after every
  transactional commit, during every chunk batch, before fingerprint/version
  publication, and during checkpoint; retry must converge without duplicate or
  missing rows;
- inject SQLITE_BUSY, I/O error, disk full, and read-only filesystem failures;
- compare row counts, foreign-key checks, important uniqueness/capacity
  constraints, exact signed/blob hashes, current Merkle root, hostchain tail,
  key generations, and backup restorability;
- run migration tests repeatedly under the standalone boundary check and with
  no AKA dependency/path.

A schema-changing pull request is incomplete until its prior-version fixture,
migration, invariant checks, failure injection, capacity estimate, operator
runbook update, and rollback drill all pass. The first shipped baseline cannot
be declared until the same gates pass for version 1 creation and restore.
