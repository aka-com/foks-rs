# foks-server-db

The authoritative SQLite storage layer for the standalone FOKS server. One
writer owns a dedicated database opened in WAL mode; snapshots use separately
configured read connections. All signed and encrypted protocol objects are
stored as exact blobs. This crate contains no network or desktop runtime
dependencies.
Signup invites and passphrase salts, encrypted PPE generations, one-time login
challenges, and bounded failed-proof accounting use the same single-writer
transaction boundary as identity state.

That writer also owns username mutations, invitation certificates and pending
requests, scoped team view grants and removals, realtime channels/messages/inbox
versions, and encrypted OIDC sessions and access state. Authorization and
capacity are rechecked in the committing transaction; read snapshots never
publish a partially committed mutation or inbox wakeup.
`ReadSnapshot::capacity_report` counts, in one snapshot, the stored rows and
bytes that each permanent or storage limit checks, using the same limits and
the same materialized KV namespace counters as the writer; it scans every
realtime message row.

Database opens reject a symlink at the database leaf. On Unix, the main
database must be a regular file with exactly one hardlink and a stable nonzero
device/inode identity; unsupported filesystem identity semantics fail closed.
The server writer retains its adjacent path lock and rechecks the captured
identity and link count under that lock before opening SQLite.

The schema is pre-release v1, currently SQLite schema version 60. Fresh creation
and writer upgrades use the same index definitions.

`Database::open` (including the server writer's identity-checked open) upgrades
versions 43 through 59 in one immediate transaction. Version 43 receives the
inbox reconciliation column and triggers; versions 43 and 44 receive the seven
expiry indexes. Older SSO schemas rename `fence` to `blocked_reason` and
`admission_hash` to `source_hash` when those columns are present. Version 48 also
changes the SSO provider fingerprint and authenticated-encryption domains, so
upgrades from version 47 or earlier remove existing browser sessions, access
rows, identity challenges, and authorization-binding records. Users must relink
under the new provider identity. Versions 49 through 59 add KV and upload
accounting counters, external chunk references, the chunk storage binding and
physical object ledger, chunk incarnations, authority and publication
provenance, the derived current-entry and historical-name indexes, public
ciphertext usage counters, and the metadata read-dependency coordinate.
Version 60 makes waitlist addresses unique under ASCII case folding: the
upgrade keeps the earliest entry for each address, deletes later duplicates,
replaces the `(email, created_at)` index with a unique `email COLLATE NOCASE`
index, adds a `created_at` index for retention, and creates the one-row
`waitlist_count` table. The upgrade initialises that row from the deduplicated
entries, insert and delete triggers keep it current, and the join cap check
reads it instead of counting the table. These changes and the version stamp
commit together; any error leaves the previous schema and data intact.
A version-49 store written before the celld port (where 49 was the waitlist
step: `waitlist_count` and the unique email index, without the KV accounting
columns) is refused with a schema-version error and must be recreated; the
upgrade recognises it by the missing `kv_namespaces.accounted_bytes` column. Index
creation can take time and disk/WAL space proportional to the existing tables,
so allow for that work during a controlled writer restart and retain a backup.
No upgrade runs inside recurring maintenance.

`Database::open_existing` and `ReadDatabase::open` validate only: they refuse 43
through 59 until the writer has upgraded them. All validating paths reject
foreign, future and unsupported older versions. Upgraded files and backups
cannot be opened by older binaries; there is no downgrade path. Other
incompatible pre-release changes may still require replacement unless an
explicit migration is provided.

Schema 49 backfills namespace byte/object and synchronization-head counters,
and upload chunk byte/count counters, in the schema migration transaction.
SQLite triggers maintain these counters across writes, deletions, cascades,
and rollback; quota charging is unchanged. Runtime quota checks use indexed
counter lookups instead of rescanning retained history. Upload ordering uses
an indexed last-offset lookup. Migration still scans existing contents once
and needs to complete before the server can accept traffic.

`ReadDatabase::open` continues to validate stored synchronization limits.
`ReadDatabaseOpener` validates once and opens additional readers with the same
configuration and checked file identity, avoiding those scans when the server
read pool grows. `Database::read_opener` returns an opener without any scan
when the writer's own open covers it: the same file, and reader KV limits at
least the writer's. Only the repository writer may modify a database while an
opener is live; out-of-band SQL or a configuration change requires a fresh
validating opener. `ReadDatabase::open_without_capacity_check` skips the check
for offline reports that read the KV rows themselves.

Schema 50 adds optional immutable external chunk references. The schema stores
SHA-256 of the exact encoded upload chunk and its logical ciphertext/encoding
lengths, with empty inline payloads. Mixed inline and external rows are supported.
The database API requires successful durable object publication before an external
commit; it does not perform object-store I/O. Preflight checks are advisory and
reserve no capacity, so commit independently rechecks ordering, retries and quota.

Placement does not change quota charges or wire usage: namespace quotas still
charge ciphertext plus exact encoding, and usage counts logical ciphertext bytes.
Conversion and backup hydration preserve those counters through transactional
triggers. Hydration verifies digest, lengths and decoded chunk fields; conversion
uses a compare-and-swap against the original inline bytes. Both operate one chunk
at a time through an indexed cursor, and should run under exclusive offline
ownership or on a private backup. Changing placement does not reclaim SQLite
freelist pages automatically.

External references require their configured object store after restart. Namespace
and abandoned-upload cleanup delete metadata references only; they do not delete
external objects. Orphans and removed objects therefore keep consuming physical
space until the server's offline `collect-chunk-store` command deletes them,
which it does only for a filesystem store whose binding carries an authority
identity, and only for objects the ledger records with a digest, no references
and no unresolved publication attempt. S3 objects are never deleted. Do not apply
bucket lifecycle deletion: a referenced object or an in-progress backup could
still require it.
