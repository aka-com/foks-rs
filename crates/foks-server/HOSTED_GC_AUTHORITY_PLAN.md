# Fail-closed filesystem collection authority

Plan for review, 2026-10-15. Keep unstaged. No protocol, cryptographic, RPC, client,
or Go compatibility changes. This is local storage qualification, not an HA fence.

## Finding and selected scope

The existing placement marker identifies namespace and HostID, not which SQLite
file owns references. A copied database passes that check. Database-path locks
also differ between copies. A collector using copy A can delete an object
referenced by copy B. Namespace locking alone prevents simultaneous activity but
does not protect a stopped divergent copy that starts later.

Select a bounded filesystem-only qualification: a new immutable v2 namespace
marker pins one canonical database path and Unix device/inode, and all supported
access paths enforce it. Add object-namespace shared/exclusive maintenance leases.
Existing/v1/unproven namespaces and remote backends remain report-only. No automatic
qualification upgrade or raw-DB handoff command. This closes supported-path clone
and live-access races without putting full database hashing on hosted activation.

The contract explicitly excludes direct SQL mutation, in-place copying over a
qualified SQLite file, raw database/WAL rollback and rollback of the object store
itself. Path/inode identity does NOT detect an in-place overwrite preserving the
inode. Do not describe this mechanism as rollback detection, replication fencing,
HA failover or support for raw restored copies. Official self-contained restore
refuses an existing destination, hydrates payloads, clears the binding, and can
establish a new namespace; that remains the supported restoration path.

## Implementation and ordering

1. Add an exact versioned internal marker encoder/validator. Preserve exact v1 compatibility for
   reading/writing objects but never qualify v1 collection. For a demonstrably
   fresh, empty filesystem namespace, create v2 with namespace ID, HostID,
   and a domain-separated SHA-256 authority digest over bounded raw canonical DB
   path bytes plus device/inode. The explicit marker version identifies this format.
   No secret material. Use bounded encoding, no lossy path conversion, immutable
   create-if-absent with fsync, and exact retry comparison. Existing old binaries
   must reject the v2 marker rather than silently treat it as v1. Marker identity
   and filesystem identity are independent checks, not interchangeable.
2. Persist enough pending-binding format intent to make interrupted creation
   deterministic. A pending v1 adoption or namespace with existing objects cannot
   become v2 by retry or inventory. No active v1 requalification. A fresh v2 retry
   using a different path/inode fails closed rather than adopting the winner.
   If the simplest safe encoding needs a DB schema increment, do that explicitly;
   do not guess format from marker absence after a crash.
3. Acquire a filesystem namespace maintenance lease from a fixed reserved lock
   file below the canonical object root. Serving, object conversion and backups
   hold it shared through their last object access; collection holds it exclusive.
   Retain it in the same ownership chain as existing storage leases, including
   abandoned blocking filesystem work and independent backup lifetime. Distinct
   backend instances must coordinate. Verify regular file/root identity and reject
   symlink substitution; canonical path aliases resolve to the same root/lock.
   Keep DB writer locks and current DB-path storage leases as additional guards.
4. Validate the v2 pinned DB identity before external reference usage, publication,
   backup hydration, conversion, inventory/adoption and collection. Different-path
   copies and inode replacement fail even when namespace/HostID/locator match.
   Preserve existing DatabasePathIdentity rules rejecting hardlinked DBs and
   leaf symlinks; canonical parent-path aliases must not create another lock
   authority. Audit low-level ServerData entry
   and standalone/manual/scheduled backups, not just CLI startup. Temporary backup
   database views carry the original source identity and its still-held namespace
   lease; never compare the private snapshot inode to the source pin or silently
   skip validation because hydration uses a temporary path.
5. Separate backend DELETE mechanism from collection authority. Collector obtains
   qualification only after binding validation under namespace exclusion and DB
   writer ownership. Unknown/custom backends default unqualified. All otherwise-idle v1/S3
   namespaces return explicit report-only and retain physical charges/deletion
   intents. An active local reader may first return namespace-busy while the
   collector attempts exclusive exclusion; it never deletes in either outcome. A dry-run reports eligibility without implying qualification. No
   operator boolean or successful inventory can assert authority.
6. Namespace lock and marker are known bounded control files excluded from chunk
   inventory accounting. Do not exclude arbitrary unknown files. Marker creation
   must establish freshness before creating control files that otherwise make an
   empty store appear nonempty, or count only the precisely known control entries.
   Namespace identity pin is immutable; deleting/replacing it is not a supported
   recovery procedure. Backup private output does not inherit the v2 pin.

Backends expose a namespace-lease abstraction plus marker qualification operation
receiving a verified DatabasePathIdentity. Return an owned lease/token, not a bool
that can be detached from the lock lifetime. S3 has no deletion qualification and
needs no misleading local namespace lock. Public custom stores remain usable for
ordinary object tests but are unqualified for collection unless their explicit
fixture wrapper forwards the real filesystem lease and validation capability.

Concrete initial interface: schema 54 adds a nullable bounded
`authority_identity BLOB` to `chunk_storage_binding`; NULL means v1. A new
`prepare_chunk_storage_binding_with_authority` variant persists the supplied
identity only with a genuinely new PendingNew insertion. Existing prepare calls
remain an unqualified wrapper, and existing v1/pending-adoption rows never gain an
identity. The marker encoder emits v2 iff the persisted identity exists. Unix
DatabasePathIdentity supplies canonical raw path bytes/device/inode using its
existing validated identity; unsupported platforms cannot qualify. ChunkStore's
namespace lease method defaults to unqualified; filesystem returns an owned
file-lock guard. Acquire the namespace guard before marker validation and retain
it with the source identity through temporary backup hydration.

## Failure invariants

- Every DELETE occurs under both database writer ownership and the physical
  namespace's exclusive lease, after exact immutable v2 authority validation.
- A clone can never qualify itself using an existing v2 marker, including after
  the original server stops. Marker v2 cannot be automatically replaced or reset.
- Existing namespaces may contain unknown former copies; they stay report-only.
- Failed/malformed/missing qualification never deletes or releases physical charge.
  Persisted deleting intents from v1 remain reportable and charged.
- Lock ownership survives cancellation and server shutdown until actual filesystem
  operations terminate. No claim about remote PUT termination is introduced.
- Supported fresh restores use independent objects and fresh bindings. Raw HA
  copy/handoff needs future distributed ownership plus authoritative revision proof.

## Acceptance before testing and committing

Direct review then independent review must inspect marker evolution, all access
entry points and lease ownership, including migration/crash cases, before tests.

- Fresh v2 namespace retains normal clean offline collection and new-incarnation
  retry behavior. Exact pending-creation retry works; changed DB identity fails.
- Same-namespace database copies A/B are rejected on B startup, backup and operator
  access even with identical HostID and SQL binding. If a fixture constructs the
  former vulnerable history, collection via unqualified legacy A is report-only
  and B's exact object bytes survive, both while B serves and after B stops.
- Inode replacement at the pinned path fails before object publication/collection;
  hardlink aliases cannot bypass path pinning; symlink behavior matches the
  canonical identity policy. In-place overwrite is explicitly excluded rather
  than included in a fake passing rollback test.
- Independent backend instances share exclusion; original database reader,
  standalone backup, scheduled backup, read-only server and abandoned filesystem
  work prevent collection until the last lease drops.
- Existing v1 and missing/unproven namespaces never silently become v2; inventory,
  restart, migration, pending adoption and marker creation errors preserve that.
- Old marker validator rejects v2; existing v1 object reads and Go wire flows
  remain compatible. Official hydrated restore can bind a fresh namespace.
- Unknown/custom and S3 stores are report-only; persisted deleting objects keep
  charge unless a currently qualified v2 local collector settles them.

## Future stronger rollback protection

If arbitrary same-inode DB restoration must become supported, this pin is
insufficient. A persistent external revision/checkpoint protocol tied to every
writer lifecycle, or a fenced authoritative replication layer, is required.
A conservative clean-session journal could disable GC permanently after crashes,
but adds lifecycle complexity and full-state validation cost. Do not implement
that protocol as incidental scope here, and do not imply this narrower pin has
its guarantees. Likewise filesystem/device identity is host-local; moving the
installation or replacing its volume requires a separately reviewed migration.

## Implemented and validated

Schema54 implements the nullable32-byte authority digest and preserves every
existing binding as NULL/unqualified. Exact marker encoding changes the existing
marker's version byte to 2 and appends the digest; v1 validation therefore rejects
v2 rather than ignoring a new optional file. Existing non-Unix writer opening is
already unsupported; the qualification capability itself also defaults to None
there. A writer handle's captured DB identity must equal the source capability,
so mismatched low-level reader/writer configurations cannot bind another file.

The source capability owns both DB-path and physical namespace leases. Startup
hands ownership to ServerData without a gap; standalone, manual and scheduled
backups retain original source identity through private snapshot hydration.
Pending initialization validates the pinned identity before creating a marker.
Inventory and conversion retain namespace exclusion. An active legacy reader can
cause collection to return busy before its report-only eligibility calculation;
neither outcome deletes objects or frees their charge.

Author direct review and root/independent-agent review passed before execution,
including narrow follow-up reviews for namespace-lock file validation, mismatched
writer identity and final fresh-client plaintext verification. Validation used
an isolated `57790f38` baseline plus the 20-file authority snapshot; no proof overlays
or production infrastructure were involved. All 122 tests passed, with 2 intentional
skips (manual performance/live-S3 gates):

- Server database library 62, schema 13, external database 3.
- Server chunk/binding/backend 19, storage leases 2, backup compatibility 6.
- Real-client external storage 12 and transfer-reader/memory 2.
- Prepared pinned Go oracle 3 (inline, external chunks and OIDC flows).

The clone regression first verifies live namespace exclusion, then stops the
original copy and verifies legacy report-only behavior. A newly created client
with an independent soft-state database authenticates and reads all 8,192 plaintext
bytes afterward, so uploader cache cannot mask a missing backend object. Its first
test draft incorrectly inspected the inline-content field of a large-file entry;
the double-reviewed correction uses the existing full-sync/large-file assertion
with that fresh client. No production correction was needed after testing.

Commands used `CARGO_INCREMENTAL=0`, the shared root Cargo target and `-j2`:
`cargo test --locked -p foks-server-db --lib --test schema --test kv_external`;
server library filters `chunk`, `storage_lease`, `compatibility_tests`;
`cargo test --locked -p foks-server-testkit --test external_chunks --test transfer_leases`;
and `--test go_client_live official_go -- --test-threads=1` with the prepared local
`FOKS_GO_ORACLE_DIR`. Logs are `/private/tmp/foks-authority-{db,backend,leases,backups,integration-rerun,go}.log`.
All manifest Rust files passed rustfmt check and the diff passed whitespace checks.
Combined final default/proof clippy gates are owned by the parent task, including
its independently reviewed inherited lint cleanups; no suppressions were added.

Committed implementation and tests as `c6040630`; this plan remains unstaged.
