# foks-server

Standalone, single-host server implementing a deliberately bounded,
v0.1.9-compatible personal and small-team slice backed by a dedicated SQLite
database. It creates encrypted named host keys, constructs and verifies a fresh
genesis hostchain/public zone/Merkle root, and binds separate probe,
public-service, and authenticated mTLS listeners.

The implemented v1 slice supports username reservation, optional or required
signup invites (standard single-use and named multi-use), software-device
signup, device certificate issuance, current and historical Merkle roots,
authenticated user-chain and PUK reads, device provisioning/revocation,
backup-key enrollment and recovery, passphrase enrollment/change and public
challenge login, atomic PPE reboxing during owner-PUK rotation, YubiKey signup
and provisioning, delegated-subkey recovery, encrypted PIV management-key
storage, named and ad-hoc team creation, local team membership edits and
removals, PTK rotation/history, removal-key retrieval, scoped remote-team
membership with PTK view boxes, remote user/team view grants, and personal/team
KV. KV covers roots, directories, optimistic dirent writes,
small files, symlinks, chunked files, pagination, cache checks, and expiring
locks. Public-client tests exercise these paths without server test hooks.

Federation is limited to client-side Beacon discovery followed by independently
pinned remote hosts, expiring bearer grants for public user/team chains, and a
durable client-coordinated remote-team admission saga. There is no general
federated trust administration, remote-host push channel, direct remote-user
membership, remote authentication to local services, team nesting,
team-member/guest services, realtime service, or arbitrary cross-host
transaction.
Passphrase-only device provisioning/recovery also remains unsupported. Ad-hoc
teams are immutable after creation. This v0.1.9-compatible slice is not a
replacement for the full Go server. The executable contract is
[protocol-v1.toml](protocol-v1.toml), and tests require it to match the
registered route table exactly.

Remote-view grants are 30-day renewable leases. Reauthorization during the
last seven days preserves the exact bearer already sealed into a local team's
PTK box while extending its expiry; an expired but active grant is renewed the
same way. A grant under a retiring capability key is also rewrapped under the
active generation without changing that bearer. Revocation leaves a permanent
non-resurrection tombstone. Remote PTK/roster changes and deliberate grant
revocation require an explicit team-lifecycle operation rather than a
permissive fallback.

Invite policy and issuance are offline operator operations. Codes are checked
through the v0.1.9 `Reg.checkInviteCode` route and consumed in the same SQLite
transaction that publishes the identity, so a failed signup cannot burn a
code and concurrent redemption cannot exceed its use limit. SQLite stores a
domain-separated code hash, policy, expiry/use metadata, and the resulting UID
binding, never the plaintext code. Human-chosen multi-use codes remain
susceptible to offline guessing after database disclosure and should be
generated with adequate entropy. The operator CLI issues, lists, disables, and
changes policy with `foks-server invite`; stop the serving process first.

Passphrases use the v0.1.9 V1 Argon2id parameters and PPE wire format. The
server stores only the public verification key, salt, and encrypted SKMWK/PPE
boxes; the raw passphrase stays in the client. Signup can establish generation
1, an active ordinary device can set or change it, and `Reg.login` consumes a
host-bound, one-time challenge before returning encrypted PPE history. An
owner-PUK rotation must atomically append a reboxed PPE generation whenever a
passphrase exists. Generic user-settings links sent by Go clients are accepted
as interoperability inputs but are not projected because this slice does not
implement that separate chain. Public passphrase login does not by itself
provision a device or expose the upstream interactive recovery workflow.

YubiKey support matches the non-interactive v0.1.9 lifecycle slice: the server
validates P-256 parent signatures and PQ-slot hints, stores an encrypted
delegated Ed25519 mTLS subkey, burns one-time hardware recovery challenges, and
stores PUK-encrypted PIV management-key envelopes with monotonic generations.
The server never talks to PC/SC and never receives a PIN, PUK, management key,
or private hardware key. Client-side hardware support is macOS/Linux only.
Interactive cross-device KEX and rotation to another already-enrolled Yubi
recipient remain outside this slice.

Protocol IDs, method positions, status codes, and service numbers are extracted
from the checksum-pinned go-foks v0.1.9 module into
[`protocol/upstream-v0.1.9.json`](protocol/upstream-v0.1.9.json). The only
handwritten input is [`protocol/policy-v1.toml`](protocol/policy-v1.toml), which
owns listeners, authentication, support decisions, adapters, bounds, and test
coverage. Checked Rust constants, routes, and `protocol-v1.toml` are generated
from that merge. Each generated route also has a typed ID; family handlers and
KV dispatch match those IDs exhaustively instead of comparing method strings.
Normal builds need no Go toolchain or upstream source.

Key-at-rest and public hostchain rotation have different failure and
compatibility models. See [KEY_ROTATION.md](KEY_ROTATION.md) before creating a
durable installation. Operator-root rewrapping, the two-link public host-key
rotation, and local symmetric capability-key rotation are implemented; other
delegated-purpose key rotation remains design-only.

## Running

The probe certificate must be trusted by clients and cover the canonical
hostname. Service certificates are generated under the delegated CA committed
by the hostchain. Every state path and the 32-byte operator root-key file is
explicit; there is no home-directory or AKA path default.

For a new self-contained installation, initialize once and run the generated
versioned configuration:

```text
foks-server init \
  --directory /var/lib/foks \
  --canonical-name foks.example.test \
  --probe-address 0.0.0.0:4430 \
  --public-address 0.0.0.0:4431 \
  --authenticated-address 0.0.0.0:4432

foks-server config-check --config /var/lib/foks/server.toml
foks-server serve-config --config /var/lib/foks/server.toml
```

Initialization is non-overwriting and creates private data, key, and backup
directories, an operator root, and a self-signed Ed25519 probe certificate for
the canonical DNS name. Replace the generated probe certificate/key paths with
operator-managed material before exposing a public deployment. Use
`foks-server status` for an offline bootstrap/integrity report and
`client-bootstrap` to write a post-bootstrap client import bundle. Example
systemd units and deployment notes are in [`../../packaging`](../../packaging).

The lower-level `serve` command remains available when every artifact and
listener should be supplied individually:

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

The separate management listener defaults to `127.0.0.1:9090` and serves
`/healthz`, `/readyz`, and Prometheus text at `/metrics`. It is plaintext HTTP
and the server rejects every non-loopback bind address. A reverse proxy that
needs remote access must connect to this loopback listener and provide its own
authenticated transport. Readiness requires both completed startup and a
bounded round trip through the SQLite writer. Metrics cover end-to-end request
and actual handler duration (including work that outlives a client deadline),
writer queue wait and execution duration, database and WAL bytes, backup
duration, saturation, and backup outcomes. They use fixed, non-user-derived
names and never export paths, identities, request values, or key material.

Connection and request token buckets are enforced per source IP with bounded
tracking memory. CLI flags set their refill rates, bursts, and maximum tracked
IP count. Saturated connection admission is dropped; a framed request that
exceeds its bucket receives FOKS status 1012 and the connection closes.

Automatic backups are opt-in:

```text
  --automatic-backup-directory /srv/foks/backups \
  --automatic-backup-interval-seconds 86400 \
  --automatic-backup-retain 7
```

Each run uses SQLite online backup plus the encrypted KEK/purpose-key snapshot,
validates the database, publishes the manifest last, then atomically renames a
hidden staging directory. Only complete `backup-*` directories count toward
retention; unrelated paths are never removed. Operators should monitor backup
failure and last-success metrics and regularly restore a backup in isolation.

Public host-key rotation is an offline, two-command operation. Stop the server,
run `begin-host-key-rotation`, allow at least 24 hours for a separately pinned
client or canary to observe the printed add-link sequence, then acknowledge
that exact sequence when completing:

```text
foks-server begin-host-key-rotation --config /var/lib/foks/server.toml
foks-server complete-host-key-rotation \
  --config /var/lib/foks/server.toml \
  --operation-id <printed-operation-id> \
  --observed-addition-seqno <printed-add-seqno>
```

Both commands are idempotent. Completion fails before the printed observation
deadline or when the acknowledged sequence differs. The revoke transaction is
committed before the old encrypted private key is removed; a retry finishes
cleanup after a crash. Revoked public IDs and hostchain bytes remain in SQLite,
but revoked private keys are neither startup dependencies nor backup contents.
Rotation advances from the latest application Merkle head; client hard state
retains the independently authenticated evidence for older roots while using
the new signed probe root as its current anchor.

Capability keys protect team-view challenges and recoverable federation bearer
tokens at rest. Rotate them offline, then periodically retire generations whose
last dependent capability has expired:

```text
foks-server rotate-capability-key --config /var/lib/foks/server.toml
foks-server retire-capability-keys --config /var/lib/foks/server.toml
```

Rotation selects the new generation in one SQLite transaction. Challenge keys
are retained through expiry; federation-envelope keys are retained while any
active grant references them, including an expired grant awaiting renewal.
Retirement marks the generation revoked before deleting its key file, so
repeating the command completes cleanup after a crash. Startup and backup
validate the full generation ledger and fail closed on missing, substituted,
or untracked key files. A retiring generation and its encrypted key are
included in backups; revoked generations are retained as non-secret audit
metadata only.

Root-key and private-key files must be regular, non-symlink files with no group
or other permissions. `SIGINT` and `SIGTERM` stop accepts, cancel idle
sessions, drain accepted writer work, stop maintenance, and join all threads.

## Architecture, jobs, and scheduling

One bounded writer actor owns every authoritative SQLite mutation. Network
reads use read-only connections from bounded asynchronous sessions; the default
limits are four async runtime workers, 256 active and 32 pending connections per
listener, 64 request handlers shared across listeners, 4,096 requests per
connection, 64 pending writes in the executable, a 15-second I/O timeout, a
30-second request-execution timeout, and a 16 MiB frame ceiling. A timed-out
handler closes its connection without claiming that a durable mutation was
cancelled; any surviving synchronous work retains its execution permit until
it exits, keeping resource use bounded.
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
Passphrase history is limited to 4,096 generations. Login state is limited to
4,096 live challenges globally and eight per UID; five bad proofs within the
default ten-minute window temporarily rate-limit that UID.

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
online SQLite snapshot, the encrypted keys required by the current durable
generation ledger, and a canonical bootstrap-generation manifest. It excludes
revoked host private keys and never copies the operator root key. Protect that
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

The SQLite snapshot includes names, invite policy/hash/redemption metadata,
passphrase salts and encrypted PPE history, recovery credentials, user/team
chains, current projections, encrypted PUK/PTK histories, capability policy
state, encrypted federation bearer envelopes plus their hashes, and all
user/team KV namespaces. Plaintext bearer tokens are never stored. Restored
envelopes remain bound to their exact target/viewer/key generation and tokens
still undergo foreground expiry and current-authority checks.

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
reporting, bounded maintenance, and WAL checkpoint results. The standalone
server also provides loopback health, readiness, and Prometheus metrics
endpoints, plus scheduled online backups with bounded retention. Structured
audit export, disk-full fault injection, and published load results remain
release-hardening work.

## Isolated development gate

`foks-server-testkit` is publish-disabled and excluded from default workspace
members. It starts only through owned temporary database/key directories and
ephemeral loopback sockets. Run the complete standalone boundary, dependency,
format, lint, protocol-matrix, unit, and process suite with:

```text
tools/foks-server/check.sh
tools/foks-server/generate-protocol.sh --check --offline
tools/foks-server/test-client-server.sh
tools/foks-server/test-small-team.sh
tools/foks-server/test-small-team-repeat.sh 5
```

The gate rejects any AKA dependency or changed path outside the standalone FOKS
boundary. The optional official-Go frame audit is
`tools/foks-v019-oracle/run-live-team-compat.sh`; it needs a Go 1.25-compatible
toolchain and may populate Go compiler/module caches, but does not use an AKA
crate or user data path.

Run `tools/foks-server/generate-protocol.sh --write` only for a reviewed
baseline or policy update. The scheduled
`tools/foks-server/diff-upstream-protocol.sh` audit compares the pinned artifact
with upstream's immutable current commit, publishes semantic JSON/Markdown
drift reports, and never edits the baseline or a lockfile.

The standalone direct client, local agent, and desktop backend have a separate
AKA-dependency and test gate:

```text
tools/foks-client/check.sh
```
