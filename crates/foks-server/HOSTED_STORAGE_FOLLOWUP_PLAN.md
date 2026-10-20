# Storage identity, physical accounting and reclamation follow-up

Planning baseline: implementation `3c60f78e`, schema 50. This is a proposed
implementation sequence, not an implemented safety or availability guarantee.
Keep this document unstaged for review. No RPC, encrypted encoding, padding,
client cryptography or Go compatibility changes are required by this design.

## Scope and order

1. Bind each database to an explicit storage namespace; enforce the binding in
   serving, conversion and every backup path. This can ship independently.
2. Add a durable object ledger and conservative physical-capacity reservations.
   Continue retaining objects. Ship metrics and reconciliation before deletion.
3. Introduce non-reused object incarnations and coordinate reads, publication,
   backups and deletion. Exercise a collector in report-only mode.
4. Enable deletion first under exclusive single-host ownership. Enable hosted
   online collection only after the replication/fencing work protects this ledger
   and all object operations. Qualify each provider separately.

For each implementation component: direct self-review, independent subagent
review, fixes and re-review, then tests and commit. Review schema migrations and
state-machine failure cases before running tests. Documentation remains unstaged
unless the user later requests otherwise.

The current code has deliberately simpler guarantees: `ChunkStore` in
[src/chunks.rs](src/chunks.rs) supports only digest-addressed PUT/GET;
[src/services/kv.rs](src/services/kv.rs) performs a non-reserving preflight before
PUT and repeats admission at commit; upload maintenance deletes metadata without
objects; [chunk_storage.rs](src/chunk_storage.rs) hydrates a private backup.
A successful external PUT followed by rejected admission can therefore leave an
uncharged orphan. A client timeout does not establish whether PUT completed.

## 1. Namespace binding

### Durable representation and API

Allocate the next available schema version at implementation time. Add a singleton
`chunk_storage_binding` row with `format_version`, random 128-bit `namespace_id`,
installation HostID, and a canonical non-secret locator descriptor. The descriptor
contains backend kind and normalized bucket/endpoint/region/prefix, or the
canonical filesystem directory. Persist a binding generation for explicit moves.
Do not put credentials, credential-provider identity or secrets into the binding.
Normal credential rotation must not change storage identity. Treat endpoint and
path aliases as operator-reviewed configuration changes, not automatic matches.

Create a small immutable marker under a reserved non-chunk key in that namespace.
It contains format version, namespace ID and HostID. Use conditional creation and
exact verification on conflict; filesystem creation requires the existing durable
no-replace/fsync rules. A marker is protection against configuration mistakes, not
proof of exclusive ownership, authentication of an untrusted operator, or fencing.
Require a dedicated namespace/prefix per installation. Sharing a bucket is fine;
sharing a namespace between independently owned databases is not supported.

Separate control operations from content integrity: extend the backend with
bounded `read_namespace_marker` and `create_namespace_marker_if_absent`, and expose
an opened `BoundChunkStore` carrying the verified ID. Do not force the marker
through the canonical-chunk decoder or relax chunk validation. An identity failure
has a sanitized operator-visible diagnostic and prevents serving or mutation.

### Bootstrap, adoption and relocation

For a new or inline-only database, persist a pending binding under exclusive
ownership, create/verify its marker outside the writer, then activate the binding
in a second transaction. A crash resumes the same ID. Never write external rows
before activation. If an existing marker belongs to another binding, fail closed;
do not silently adopt or overwrite it. Pending binding state must not enable RPCs.

For an existing schema-50 database with external references, migration records
`unbound_legacy` and does not invent proof that the configured store is correct.
Provide an explicit offline adoption command: acquire exclusive ownership, verify
all distinct referenced objects (length, digest and canonical metadata), establish
or verify the marker, and activate the binding. Make the scan resumable with a
checkpoint tied to the unchanged database and proposed locator. Report bytes,
progress and missing/corrupt references. The user must supply the intended locator;
an empty store or partial successful scan cannot bind an existing external DB.
This one-time offline scan is outside the 15-second recovery budget. Before the
first adoption, preserve a consistent offline database backup using SQLite's
backup API (a plain copy of the main file may omit committed WAL). Pending state
pins the chosen locator even if verification fails. There is no automatic reset:
if the first locator was wrong, restore that pre-adoption copy only while offline
and before any binding activation or subsequent writes. Otherwise use a completed
self-contained backup restore. Do not transplant an old database over a live owner.

Before accepting traffic, verify the active database binding, configured locator
and marker. Bounded remote verification consumes the activation budget; failure
leaves the tenant unavailable instead of accepting writes to a different store.
Repeat verification after backend replacement. Immutable bound configuration and
restricted operator permissions remain prerequisites during a running process;
a marker read at startup cannot stop an operator deleting or moving objects later.

Apply the check to runtime bootstrap, standalone and scheduled backups, offline
conversion and future repair/GC commands. Restore of a self-contained hydrated
backup explicitly clears active binding and source-ledger requirements in the
private restored database: all restored rows are inline. Preserve optional source
provenance separately. A restored installation may then bind a new namespace.
Do not silently clear binding on the original live database.

A later relocation command must hold exclusive ownership, copy and verify every
needed object, establish the new marker and atomically switch the binding. Keep
the source intact until backup/read obligations and rollback plans are resolved.
This initial component can simply reject relocation and document export/restore
as the supported move; never automatically rebind on config change.

### Acceptance

- Wrong bucket, prefix, endpoint, filesystem root or marker rejects startup before
  RPC mutation, and rejects conversion/backup; credential rotation still works.
- Concurrent or interrupted initialization converges on one durable binding;
  conflicting markers cannot be replaced, including after a lost create reply.
- Legacy adoption rejects one missing/corrupt object and resumes safely after
  interruption; inline-only upgrades require no full object scan.
- A completed hydrated backup restores with the original backend unavailable and
  can bind a fresh namespace. All backup entrypoints exercise this behavior.
- Go inline/external conformance and historical schema migration tests still pass.

## 2. Physical-capacity accounting without deletion

### Ledger and reservation transactions

Add `chunk_objects` keyed by a stable internal object ID, carrying namespace ID,
key-format version, actual object key, digest, encoded length, state, live reference
count and ownership epoch. Add `chunk_object_attempts` for PUT attempt IDs and
outcomes, and per-installation counters for committed object bytes, reserved bytes,
uncertain bytes and object count. Use checked integer arithmetic and indexed bounded
queries. Use foreign keys from external chunk rows to `chunk_objects`, unique
namespace/object-key constraints, fixed digest/ID length checks, nonnegative
length/count checks and indexes on `(state, object_id)` and attempt outcome/age.
Retain the existing inline/external representation CHECK while adding the object
ID; migration first backfills IDs and only then enforces the new foreign key.
Reference updates and counter updates must share the chunk mutation transaction,
including cascades. Migration refuses incompatible old writers through the schema
version; already running stale owners still require fencing.

Define capacity buckets as disjoint per-object states: reserved before outcome,
uncertain after ambiguity, and present after confirmed publication. The admission
sum counts each actual key once and retains any conservative allowance for legacy
unknown inventory; attempt counts independently bound remote work. Do not add
reserved and uncertain bytes twice for the same object, or release the only
reservation while another admitted attempt remains unresolved. These are
physical-capacity controls separate from existing logical KV
quota and `kv_usage`; do not change Go usage responses in this component.

An object state is `reserved`, `present`, `uncertain`, `deleting` or `retired`.
Presence does not imply a live DB reference: present objects with zero references
are reclamation candidates. Attempt state distinguishes admitted, known-success,
known-no-write and uncertain outcomes. Reference counts include incomplete uploads
and every retained historical reference reachable through existing chunk rows.
Any row deletion/cascade and inline/external conversion updates reference counts
transactionally. Verify aggregate counters against an offline recomputation.

Replace preflight with `reserve_external_chunk`, which checks current authority,
order, logical quota, physical capacity and pending-operation limits, then returns
a durable reservation token or a committed-replay result. Serialize/coalesce
same-chunk retries so they do not create unlimited attempts. Persist the token
before object I/O. `commit_external_chunk` checks the token/epoch and rechecks all
current admission conditions while atomically inserting the chunk reference and
settling the reservation. A durable object with rejected admission stays accounted
as an orphan; it must not disappear merely because the request failed.

Cap outstanding attempts, uncertain bytes and physical bytes per tenant and host.
Do not release an uncertain reservation on wall-clock expiry, caller cancellation,
a missing GET, or local task abort. A late remote PUT may still complete. Repeated
ambiguous attempts therefore eventually refuse further uploads instead of growing
unbounded unaccounted storage. Secret reads and administrative diagnosis must remain
available when bulk upload admission is exhausted.

Count object bytes once per actual key, independently of number of metadata
references. Track provider versions, multipart remnants and request charges as
separate operator concerns: the ledger cannot claim to equal a provider bill.
Initially disallow uncontrolled bucket versioning/lifecycle policies for this
accounting contract, or explicitly qualify and account for their retained data.
Inventory reports untracked objects but cannot declare them safely deletable.

### Legacy migration and reconciliation

Backfill referenced schema-50 digest keys into the ledger, verifying identical
lengths when multiple rows share a digest. This accounts for referenced objects;
it cannot discover old orphan keys. Require a bounded resumable inventory before
claiming complete capacity accounting. Report unknown historical inventory
separately and preserve it by default. Hold exclusive ownership during the initial
backfill/inventory, or later use a specified snapshot/cursor reconciliation scheme;
a concurrent listing alone is not a complete accounting proof.

Expose pending/uncertain/orphan bytes and counts, oldest attempt age, rejection
counts and inventory discrepancies. Supply report-only reconciliation. Metadata
reservation durability must share the eventual replicated commit boundary: if
failover loses the ledger while an old remote PUT succeeds, physical usage is again
unknown. Until that boundary exists, host-local accounting is not cluster-wide
fencing or a zero-loss claim.

### Acceptance

Fault injection covers each boundary: reservation commit, PUT start/success/lost
reply, authorization revocation, logical quota race, metadata commit/lost reply,
and restart. Exact replay preserves logical usage; conflicting replay preserves
existing errors. Repeated ambiguous PUTs reach a finite admission limit without
releasing uncertain charges. Maintenance and FK cascades maintain reference and
physical counters. Inventory distinguishes tracked, unknown and missing objects.
No provider deletion API is used in this component.

## 3. Coordinated reclamation

### Non-reused identities are required

Digest-only keys can be recreated by a delayed PUT after deletion, and a delayed
DELETE can destroy a newer publication at the same key. Before enabling deletion,
add an internal `object_id` reference to external chunk rows and use new keys such
as `objects/v2/<random-incarnation>/<digest>`. The digest still authenticates the
same canonical bytes and exact RPC retries remain unchanged. Keep legacy digest
keys readable. Never reuse an incarnation after entering `deleting`, even if the
provider says the object is absent. New publication of identical bytes uses a new
incarnation; committed replay resolves its existing object ID from the database.

This requires a backend API that accepts a validated internal key plus expected
digest/length rather than deriving every key solely from the digest. Add bounded
inventory and deletion APIs returning explicit confirmed/ambiguous outcomes.
Filesystem delete must reject symlinks/nonregular objects and fsync the directory.
Provider-specific conditional/version deletion may strengthen the implementation,
but never assume a common S3-compatible API offers it without qualification.

Legacy keys become delete candidates only after future publications have switched
to incarnations and no old-format writer can publish metadata. Retain their
permanent tombstones. Do not garbage-collect legacy keys while an old binary may
reuse them. Schema/downgrade refusal alone does not fence an already running owner.

### Pins, ownership and deletion state machine

Under the authoritative writer, select an object with zero references, no active
publication reservation and no read/backup pin; atomically transition it to
`deleting`. No later operation may acquire a reference or pin on that incarnation.
Perform DELETE outside the writer, then settle its result under the same epoch.
Retain a tombstone and retry ambiguous deletion only against that non-reused key.
A late PUT can resurrect retired bytes but cannot create an authorized live
reference; keep them charged/uncertain and resweep until publication completion
is settled. Never treat DELETE success or a missing GET as proof that an earlier
in-flight PUT can no longer commit. Conservatively retain capacity reservations
where the provider cannot establish a terminal outcome. Define a provider-specific
drain/reconciliation procedure before claiming eventual physical-space recovery.

Downloads acquire a pin atomically with authorized reference lookup and release it
after object bytes are captured. Avoid adding a SQLite write to every GET if a
process-wide shared/exclusive coordination guard can provide the same guarantee
under single-host ownership; choose and benchmark this implementation explicitly.
Cross-host pins must be durable or covered by storage-enforced ownership. Expiring
a pin by wall clock alone is unsafe for a paused operation. Reclaim crashed-owner
pins only after its continued relevant I/O/publication has been fenced or otherwise
safely drained; an ambiguous remote request is a separate outstanding obligation.

Every backup acquires a durable collection barrier before opening its SQLite
snapshot and holds it through final hydration or verified abandonment. A coarse
per-installation barrier is the first implementation: pause deletion, not normal
writes, for the backup duration. It avoids an unbounded per-object pin insertion
inside the writer. Standalone CLI backups participate too. Conversion, restore and
repair must join this coordinator. Self-contained completed backups need no
ongoing source-object pin; external-reference backup exports would need a new
retention contract and are outside this plan.

Initially enable collection only with exclusive ownership that excludes serving,
conversion and standalone backups. Stopping one server process is insufficient if
remote requests remain ambiguous. Hosted online collection additionally requires
replicated ledger/pins, epoch-fenced metadata commits and protection against stale
collector operations. Non-reused keys prevent old DELETEs from harming new
incarnations, but do not replace that ownership design. Until these prerequisites
are demonstrated, keep collector report-only and objects retained.

Keep current historical file-version retention in this component. Optional history
pruning is separate: define retained-version policy, remove references atomically,
and test cached Go references and user-visible missing-version behavior. Changing
retention does not require changing the wire format, but does change availability
of previously retained versions.

### Fault and compatibility acceptance

- Pause PUT past its timeout, mark/delete its unreferenced incarnation, then let
  PUT complete. No current metadata points at it, no capacity is silently freed,
  and reconciliation reports/reclaims the resurrected orphan safely.
- Pause DELETE, publish identical content into a new incarnation, then complete
  old DELETE. New content survives and reads correctly.
- Race upload publication, metadata cleanup, GET, backup snapshot/hydration and
  collection; no acknowledged live reference or accepted pin loses its object.
- Kill at every ledger transition; retry and restart preserve counters and
  non-reuse. Pause an old owner across promotion, including queued callbacks and
  standalone backup/collector processes; stale work cannot publish authoritative
  references or delete a successor's objects.
- Run local FS and mock-provider tests, then explicitly authorized live-provider
  tests for lost replies, conditional conflicts, throttling and delete ambiguity.
  Mock success does not certify provider behavior or host/AZ recovery.
- Actual pinned Go clients exercise upload/download, exact/conflicting replay,
  logical usage, mixed legacy/new keys and restored self-contained backups.

## Relationship to the 15-second recovery target

Namespace validation, ledger ownership and required key availability are activation
prerequisites; inventory, adoption, compaction, full backup hydration and sweeping
must stay off the recovery critical path. A successor starts only after metadata
visibility and ownership are safe, using already durable chunk objects. Test an
active file upload and reader during real host/AZ loss as part of the recovery
harness, not only secret writes. Record end-to-end successful reads and a new
durable write within 15 seconds under the stated quorum/placement assumptions.
Neither this plan nor conditional object writes alone establish that result.

## Direct plan review

Reviewed against current chunk backend, upload preflight/commit, schema-50 external
references, upload cleanup, startup check and hydration paths. The design preserves
wire bytes and logical charging, makes existing-orphan accounting limitations
explicit, separates marker identity from fencing, protects all backup entrypoints,
and does not equate timeout/404/deletion with terminal remote PUT cancellation.
The deliberate unresolved provider capability is terminal-operation reconciliation;
without it, uncertainty consumes capacity and deletion cannot promise reclaimed
space. Independent review must validate that constraint and the ownership/pin
state machine before implementation.

## Schema-52 implementation slice (reviewed before coding)

Keep deletion disabled. Use a 32-byte internal object ID (the existing digest for
version-1 chunk keys), a distinct key string and namespace binding, and a nullable
digest for unrelated historical inventory entries. Add an external-object foreign
key to chunk rows, backfill referenced keys, and enforce representation consistency
and reference counters with triggers. Object states reserved, uncertain and present
are disjoint capacity buckets; all remain charged even at zero references. Attempt
IDs and outcomes independently cap admitted or ambiguous remote publications.

`reserve_external_chunk` performs current logical preflight then commits a physical
reservation before PUT. It returns a new publication token, an existing-present
object token, or an unresolved-key token eligible only for GET verification. An
unresolved key cannot spawn another PUT. A matching bounded digest/length GET can
mark its object present and permit a current authorized metadata commit; the old
ambiguous attempt stays recorded and counted because object presence cannot prove
its remote PUT is no longer in flight. Missing GET never releases a charge.
Successful PUT settles presence before the final metadata/auth/quota check, leaving
a charged orphan on a rejected final commit. Tokens bind object ID and namespace;
local epoch 1 is deliberately not a distributed ownership fence.

Inventory must complete before new physical reservations. A new binding can prove
an empty namespace with one bounded initial page; any existing entries require
explicit offline enumeration, not a full activation-time walk. Existing schema-51
bindings also require inventory. Enumeration runs under the installation writer
lock and checkpoints backend opaque tokens; it never interprets a generic listing's
last key as an ordered provider cursor. Count all regular object entries except
the namespace marker, including unknown keys and temporary remnants. Preserve
conservative charges for missing tracked objects; list absence does not prove a
late PUT cannot arrive. Unknown legacy publishers must have been drained externally
before the operator declares the initial scan complete. A stopped local process
and an HTTP timeout are not proof of a remote drain. Subsequent known attempts are
already charged, so their late arrival cannot escape the capacity ledger.

Limits are per installation: total physical bytes, object count, outstanding attempt
count and uncertain bytes. Lowering a limit blocks growth without deleting data.
No host aggregate claim is made without a supervisor distributing those limits.
Report-only CLI inventory exposes incomplete inventory, reserved/present/uncertain
bytes, orphan reference counts, missing tracked keys and unknown keys. Private
hydrated backups clear both binding and ledger only after every external reference
is inline. No provider DELETE or wall-clock charge expiry is added.

## Schema-53 offline collection implementation slice (plan for review)

This slice follows the reviewed schema-52 physical ledger. It changes internal
placement only; Go and Rust clients continue sending exactly the same encrypted
chunks and RPC messages. The collector remains offline, with no claim of safe
multi-host failover or online distributed GC. New object incarnations avoid
permanently rejecting a digest after collection: a client replay may publish the
same ciphertext under a fresh physical key without fresh encryption.

### Representation and migration

Keep every schema-52 object ID, digest key, reference and counter intact. Rebuild
the ledger tables transactionally where SQLite CHECK constraints require it;
validate foreign keys and exact totals before committing version 53. Add key
format 2 with a random 256-bit object ID and flat key `v2-<64 lowercase hex>`.
Flat names preserve the filesystem backend's bounded directory enumeration and
avoid unreviewed directory recursion. IDs are generated before admission, but
uniqueness is enforced by SQLite; collision fails admission without object I/O.
Existing format-1 keys remain the lowercase digest, and continue to read normally.

Replace the unique digest index with a partial unique index on digest for active
states reserved, uncertain and present. Add deleting and deleted states. Deleting
bytes and objects remain physically charged; a successful finalization moves them
out of physical capacity but retains the immutable tombstone and object key.
Expose deleting bytes/count and tombstone count separately. Tombstones are not
physical payloads; their SQLite footprint remains a separate density/retention
limit, and this first slice does not prune them. Never change an object's key or
reuse a tombstoned object ID. A new reservation for a digest with only deleting or
deleted incarnations allocates a new random ID/key. Concurrent retries of one
active incarnation return the same object token; unresolved retries remain
GET-only, following schema 52. Historical retained chunk references still count
and prevent collection even when no current directory head points to the file.

Every internal external-chunk reference must carry namespace ID, object ID, key
format, exact key, digest and encoded length. SQL reads join the existing
external_object_id foreign key to the ledger rather than reconstructing a key
from the digest. Existing RPC responses expose only ciphertext as before. Cover
normal reads, exact metadata replay, conversion, adoption, all backup hydration
paths, restored databases, and test stores. A source backup must protect its
objects until hydration finishes; the final self-contained backup clears binding
and ledger only after every reference becomes inline.

### Backend API and publication proof

Introduce an internal `ChunkObject` value containing the immutable fields above.
Add `put_object(&ChunkObject, bytes)`, `get_object(&ChunkObject)`, and
`delete_object(&ChunkObject)` operations. Validate the key against the ID/key
format before any filesystem or provider call; forbid separators, traversal and
control-marker names. Digest validation is independent of the physical key.
Namespace marker and inventory operations retain their existing separate APIs.
Migration/adoption may use an explicit legacy-object constructor; no normal caller
may silently fall back to a digest-derived key for a format-2 reference.

Publication outcomes need stronger semantics than Result success for collection.
The current S3 client retries requests: a later successful retry can coexist with
an earlier timed-out request still running at the provider. Presence is sufficient
for reads, but cannot establish that a future DELETE will not be followed by a
late PUT. Before enabling remote GC, disable automatic retries for mutation
requests or durably represent every possibly outstanding subattempt. Prefer one
HTTP PUT per admitted token. A verified successful response to that sole request,
or a completed rejection followed by exact existing-object verification, can
settle its attempt only when there was no earlier ambiguous subattempt. Transport
errors and timeout cancellation remain uncertain regardless of later GET/list
presence. Any legacy successful S3 attempts from a retrying implementation are
GC-ineligible until an external drain establishes their termination; migration
must preserve this conservatively, not invent proof. Filesystem operations that
return successfully after fsync have completed locally. If the implementation
cannot provide these distinctions, ship filesystem GC only and keep S3 report-only.

### Offline exclusion and state machine

The command takes the installation writer lock for its full lifetime, verifies
active binding and marker, and opens the ledger. Refuse to operate if the server,
conversion or another operator task owns the installation. Audit every backup
entry point: the same exclusion must cover snapshot creation through final object
hydration. Add the missing shared ownership mechanism if any backup can outlive
the existing guard. The lock cannot fence another host, direct database users,
external bucket writers or old remote operations; these remain explicit deployment
prerequisites. No online pin scheme is introduced in this slice.

Select a bounded batch (default at most 128 objects and a configurable byte cap)
with known digest/key format, active namespace, state present, references_count=0,
and no admitted, uncertain or legacy-unproven publication attempt. Unknown
inventory entries are report-only. Select and transition to deleting in one short
transaction, with a persisted random deletion token and generation. This durable
intent is the tombstone: no later publication may reuse its physical key. Release
the transaction before calling the backend, while keeping installation ownership.

DELETE is idempotent and always targets the exact persisted key. On durable
success, finalize deleting -> deleted with token/namespace/ID/key CAS in a second
transaction. An ambiguous DELETE leaves deleting and its full charge intact; a
restart resumes that same token/key. A qualified strongly consistent absent-object
check can finish the operation only after there are no possible PUT publishers
for that incarnation. Late DELETEs then affect only the retired key. Filesystem
absence requires synchronizing the directory before releasing charge; successful
unlink followed by failed fsync is unresolved until that durability barrier
succeeds. Remote provider semantics require separate qualification; a mock proves
the application state machine, not provider durability.

Do not roll back a deleting intent to present after a remote timeout. A newly
admitted same-digest object uses another incarnation, so a late old DELETE cannot
remove it. No age threshold, missing listing, session disconnect or host death
constitutes publication termination. Report skipped references, unknown objects,
uncertain publishers, incomplete inventory and unqualified remote proof separately.
No object I/O occurs on the writer or while a read snapshot is held.

### Ordered work and acceptance

1. Implement schema-53 migration and object-reference routing, then review before
   tests. Exercise real schema-52 mixed inline/legacy-external data, active and
   uncertain attempts, unknown inventory, rollback on inconsistency, and foreign
   key/counter equality. Migration cannot release charges or clear uncertainty.
2. Implement new incarnation publication and reference-aware filesystem/mock-S3
   methods. Verify same ciphertext across different keys decrypts identically,
   concurrent retries share one active key, exact metadata replay remains a no-op,
   and existing Go uploads/downloads work across mixed representations.
3. Implement report-only eligibility and installation/backup exclusion; review
   actual lock lifetimes. Tests must hold each serving/backup/conversion operation
   while collection attempts to acquire ownership and verify rejection or waiting
   under the documented bound, without deleting any protected data.
4. Enable filesystem collection, then separately qualify remote mutation outcomes.
   Fault tests cover crash after intent/before DELETE, DELETE before lost response,
   crash before finalization, failed directory sync, missing objects, restart,
   malformed/stale tokens, corruption and provider refusal. Counter reconciliation
   must equal a complete physical inventory, with unresolved entries still charged.
5. Gate the old DELETE, admit the same digest under a new incarnation after the
   collector relinquishes local ownership, then deliver the delayed old DELETE;
   the new upload must remain readable. Simulate late PUT after an ambiguous
   publication and prove the old incarnation is never eligible or uncharged.
   Verify historical refs and backup hydration prevent deletion, and that repeated
   completed collection is idempotent. Do not claim a 15-second recovery guarantee
   from this offline maintenance test; activation must not scan all tombstones.

Direct self-review identified the retry/subattempt termination issue above and
made remote deletion contingent on an explicit solution. Independent plan review
must approve that proof boundary and the backup exclusion audit before production
changes. Implementation then receives direct and independent review before tests;
all documentation remains unstaged.

The direct backup audit found a concrete exclusion gap to close in step 3:
`backup_standalone_installation_with_chunk_store` opens a read-only source and can
outlive the server's writer lock. The scheduler is joined before writer shutdown,
and `RunningStandaloneServer::backup(&self)` retains the server owner, but those
facts do not protect the standalone backup API or a read-only low-level server.
Introduce a separate per-installation storage-maintenance file lock, using the
existing database path identity and no-symlink rules. Serving (including low-level
read-only serving) and all backup entry points hold it shared through their last
object read; the offline collector holds it exclusive alongside the writer lock.
Acquire the backup lease before opening its source snapshot. The collector uses
nonblocking acquisition and returns a clear retryable operator error if either
lock is busy; it does not hold a reader snapshot while waiting. Preserve writer
lock semantics for database mutations. Test source backup surviving server shutdown,
read-only serving, ordinary scheduled/manual backup, and cancellation/drop paths.

Schema-53 implementation choices after direct review: new 256-bit IDs are derived
from the namespace and the random 128-bit admission token with a domain-separated
hash; the IDs are physical names, not a change to client cryptography. The retained
`present_bytes` counter includes deleting objects until durable deletion; the
report exposes deleting bytes as a subset. S3 collection is always report-only,
including calls without `--dry-run`, because existing retrying requests lack the
necessary subattempt termination proof. No absence heuristic resolves a PUT.
The migration copies the constrained chunk/ledger tables transactionally, validates
foreign keys plus logical-reference and physical counters, and needs temporary
SQLite capacity proportional to the migrated data; complete it before activation
under the 15-second recovery budget. Filesystem inventory uses bounded RAM but
rescans the directory per page (O(N²/page size) work); replace it with a scalable
provider-specific cursor/index before large object-count deployments. Neither
migration nor offline inventory/collection has a 15-second completion guarantee.
