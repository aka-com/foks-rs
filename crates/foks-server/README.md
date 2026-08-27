# foks-server

Standalone, single-host FOKS v0.1.9 personal and small-team server backed by a
dedicated SQLite database. It creates encrypted named host keys, constructs and
verifies a fresh genesis hostchain/public zone/Merkle root, and binds separate
probe, public-service, and authenticated mTLS listeners.

The implemented v1 slice supports username reservation, software-device
signup, device certificate issuance, current and historical Merkle roots,
authenticated user-chain and PUK reads, device provisioning/revocation,
backup-key enrollment and recovery, named and ad-hoc team creation, local team
membership edits and removals, PTK rotation/history, removal-key retrieval,
and personal/team KV. KV covers roots, directories, optimistic dirent writes,
small files, symlinks, chunked files, pagination, cache checks, and expiring
locks. Public-client tests exercise these paths without server test hooks.

Federation, remote users/teams, invites, passphrase login, YubiKey enrollment,
team nesting, team-member/guest services, realtime services, and cross-host
operation are intentionally unsupported. Ad-hoc teams are immutable after
creation. This is not a drop-in replacement for the full Go server. The
executable contract is [protocol-v1.toml](protocol-v1.toml), and tests require
it to match the registered route table exactly.

## Running

The probe certificate must be trusted by clients and cover the canonical
hostname. Service certificates are generated under the delegated CA committed
by the hostchain. Every state path and the 32-byte operator root-key file is
explicit; there is no home-directory or AKA path default.

```text
foks-server serve \
  --canonical-name foks.example.test \
  --database /srv/foks/foks-server.sqlite \
  --key-directory /srv/foks/keys \
  --root-key-file /run/secrets/foks-root-key \
  --probe-certificate-der /run/secrets/probe-leaf.der \
  --probe-certificate-der /run/secrets/probe-ca.der \
  --probe-private-key-der /run/secrets/probe-key.pk8 \
  --probe-address 0.0.0.0:4430 \
  --public-address 0.0.0.0:4431 \
  --authenticated-address 0.0.0.0:4432
```

Root-key and private-key files must be regular, non-symlink files with no group
or other permissions. `SIGINT` and `SIGTERM` stop accepts, cancel idle
sessions, drain accepted writer work, stop maintenance, and join all threads.

## Architecture, jobs, and scheduling

One bounded writer actor owns every authoritative SQLite mutation. Network
reads use read-only connections on fixed worker pools; the default limits are
four workers, 32 pending connections, 4,096 requests per connection, 64 pending
writes in the executable, a 15-second I/O timeout, and a 16 MiB frame ceiling.
This intentionally favors a simple total order and predictable overload
behavior over write parallelism. A future partition can move opaque chunks
first and then independent KV namespaces; identity/name publication and the
global Merkle log still require one leader or a consensus protocol.

The only background job is a bounded in-process maintenance loop. Once per
minute it submits one ordinary writer task that removes expired user/team
reservations, receipts, recovery challenges, team-view and TeamAdmin
capabilities, and locks; reclaims uploads not referenced by a current
directory entry after 24 idle hours; and requests a truncating WAL checkpoint.
Foreground expiry, current-device, current-roster, role, and key-generation
checks enforce correctness even if this job never runs. There is no durable
general-purpose job queue, retry farm, cron dependency, or multi-process lease system.
An operating-system lock on the database sidecar rejects a second writer
process; active/active service instances are not supported.

## Capacity limits

Defaults are enforced before authoritative KV writes:

- 64 KiB per encoded small node;
- at most 64 dirents per mutation batch and 1,000 entries per list page;
- 9 MiB per stored encrypted chunk and 512 chunks per upload;
- the client-compatible 1 GiB cleartext file ceiling, allowing up to just over
  2 GiB of v0.1.9 padded ciphertext;
- 16 GiB and 1,000,000 stored objects per user or team KV namespace; and
- 64 GiB for the SQLite main database through `max_page_count`.

Identity/team defaults also bound each user to 16 active devices and four
backup credentials; each team to 64 members, 16 PTK role/visibility bands,
4,096 links, and 1,024 boxes of each class per mutation; the installation to
4,096 teams and pending team-name reservations; and team view/admin
capabilities to 32 per member/team pair and 16,384 per class globally.

Namespace accounting charges encoded bytes actually stored, including upload
envelopes and ciphertext duplication. Tombstone/history versions remain
charged until a future authenticated compactor can prove they are reclaimable.
SQLite, filesystem, WAL, backup, and key-directory overhead means operators
must reserve more disk than the configured database limit.

No throughput number is a product claim yet. The single writer, `FULL`
synchronous WAL commits, repeated reachable-tree version-vector construction,
and namespace quota scans are expected to be the first saturation points.
Benchmark reports must state hardware, SQLite settings, dataset shape, p50/p95/
p99 latency, writer saturation, reader latency, WAL growth, and checkpoint
cost before quoting capacity.

## Backup, restore, and integrity

`RunningStandaloneServer::backup` creates a new backup directory containing an
online SQLite snapshot, the five immutable encrypted key files, and a canonical
key-generation manifest. It never copies the operator root key. Protect that
root key separately; neither the encrypted key snapshot nor the database is
recoverable without it.

The executable uses the same validated library entry points:

```text
foks-server backup \
  --database /srv/foks/foks-server.sqlite \
  --key-directory /srv/foks/keys \
  --root-key-file /run/secrets/foks-root-key \
  --destination /srv/backups/foks-2026-08-27

foks-server restore \
  --backup-directory /srv/backups/foks-2026-08-27 \
  --database /srv/foks-restored/foks-server.sqlite \
  --key-directory /srv/foks-restored/keys
```

Backup authenticates every encrypted key generation with the root key and
uses a read-only source connection. Restore accepts only a complete manifest,
integrity-checked database, and exact declared key set, and requires empty
destination paths.

The SQLite snapshot includes names, recovery credentials, user/team chains,
current projections, encrypted PUK/PTK histories, capability policy state, and
all user/team KV namespaces. Usable bearer tokens are never stored; only their
hashes are present and restored tokens still undergo foreground expiry and
current-authority checks.

For restore, stop the process, retain the damaged directory, place the backed-up
database and key directory at new explicit paths, supply the matching operator
root key, and start the server. Startup revalidates the database application and
schema IDs, decrypts the keys, verifies key generations against persisted host
state, and reconstructs the signed bootstrap. A missing, modified, or
mismatched key fails startup rather than rotating identity. Reconnect an
already-pinned client before switching traffic.

Operational checks should run against a copy or during a maintenance window:

1. call SQLite `quick_check` for routine sampling and `integrity_check` for a
   full scan;
2. verify the returned result is exactly `ok`;
3. create and validate a fresh online backup and key manifest;
4. record main-database and `-wal` sizes plus checkpoint duration; and
5. periodically perform a restore rehearsal with an existing pinned client.

The Rust API exposes full integrity checking, online backup, storage-size
reporting, bounded maintenance, and WAL checkpoint results. HTTP health and
metrics endpoints, automated backup triggers, structured audit export, disk-
full fault injection, and published load results remain release-hardening work.

## Isolated development gate

`foks-server-testkit` is publish-disabled and excluded from default workspace
members. It starts only through owned temporary database/key directories and
ephemeral loopback sockets. Run the complete standalone boundary, dependency,
format, lint, protocol-matrix, unit, and process suite with:

```text
tools/foks-server/check.sh
tools/foks-server/test-client-server.sh
tools/foks-server/test-small-team.sh
tools/foks-server/test-small-team-repeat.sh 5
```

The gate rejects any AKA dependency or changed path outside the standalone FOKS
boundary. The optional official-Go frame audit is
`tools/foks-v019-oracle/run-live-team-compat.sh`; it needs a Go 1.19-compatible
toolchain and may populate Go compiler/module caches, but does not use an AKA
crate or user data path.
