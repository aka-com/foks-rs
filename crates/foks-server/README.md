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
the bounded public KEX relay used by interactive software-device pairing,
backup-key enrollment and recovery, passphrase enrollment/change and public
challenge login, atomic PPE reboxing during owner-PUK rotation, YubiKey signup
and provisioning, delegated-subkey recovery, encrypted PIV management-key
storage, named and ad-hoc team creation, local team membership edits and
removals, PTK rotation/history, removal-key retrieval, scoped remote-team
membership with PTK view boxes, remote user/team view grants, and personal/team
KV. KV covers roots, directories, optimistic dirent writes,
small files, symlinks, chunked files, pagination, cache checks, and expiring
locks. It also supports durable local and remote invitation requests and
decisions, invitation certificates and scoped view grants, committed removal
delivery, atomic username changes, OIDC signup, existing-account migration, and reauthentication with ongoing
access enforcement, and Basic encrypted realtime chat for named teams.
Public-client tests exercise these paths without server test hooks.

Federation is limited to Beacon discovery followed by independently pinned
remote hosts, expiring bearer grants for public user/team chains, and a durable
client-coordinated remote-team admission workflow. This server answers the public
`Beacon.beaconLookup` for its own HostID with its advertised probe endpoint and
returns a typed not-found for any other host; the client still treats the hint
as untrusted until a direct probe authenticates the requested HostID. There is no general
federated trust administration, remote-host push channels, direct remote-user
membership, general team nesting, extended or ad-hoc-team chat, or arbitrary
cross-host transactions. The invitation implementation provides bounded
team-member and guest workflows; it is not a general federated workflow engine.
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

`Reg.joinWaitList` is unauthenticated and stores the submitted email address,
so it is disabled by default and answers `Unsupported`, the status Go clients
already receive from routes a host does not serve. Enable it with
`enabled = true` in the `[waitlist]` table of `server.toml` (or
`serve --enable-waitlist`). The table holds one entry per address, compared
under ASCII case folding; a repeated join returns the stored identifier. The
identifier grants nothing, and no route accepts it. Once `maximum_entries`
(default 10,000, at most 1,000,000) is reached, every join answers
`RateLimited`, including joins for listed addresses, so a full table does not
reveal which addresses it holds. The cap is global: one client can fill it,
after which the route refuses all joins until entries expire or the operator
purges them. Maintenance deletes entries more than `retention_days`
(default 180) after their first join; a repeated join does not extend
retention. The operator reads entries with `foks-server waitlist list`,
writes them to a new owner-only JSON file with `waitlist export --output`,
and deletes them with `waitlist purge --before-unix-seconds`, `--email` or
`--all`. Listing and export read a snapshot; stop the serving process before
a purge.

Passphrases use the v0.1.9 V1 Argon2id parameters and PPE wire format. The
server stores only the public verification key, salt, and encrypted SKMWK/PPE
boxes; the raw passphrase stays in the client. Signup can establish generation
1, an active ordinary device can set or change it, and `Reg.login` consumes a
host-bound, one-time challenge before returning encrypted PPE history. An
owner-PUK rotation must atomically append a reboxed PPE generation whenever a
passphrase exists. Generic user-settings links are verified and persisted with
their chain history; accompanying PPE updates commit in the same transaction.
Public passphrase login does not by itself provision a device or expose the
upstream interactive recovery workflow.

YubiKey support matches the non-interactive v0.1.9 lifecycle slice: the server
validates P-256 parent signatures and PQ-slot hints, stores an encrypted
delegated Ed25519 mTLS subkey, burns one-time hardware recovery challenges, and
stores PUK-encrypted PIV management-key envelopes with monotonic generations.
The server never talks to PC/SC and never receives a PIN, PUK, management key,
or private hardware key. Client-side hardware support is macOS/Linux only.
Interactive software-device KEX is supported, for Rust clients and for Go
v0.1.9 clients, which multiplex KEX calls on one connection (see the session
limits below). Rotation to another
already-enrolled Yubi recipient remains outside this slice.

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
explicit; the server does not infer state paths from the user's home directory.

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
`foks-server status` for an offline bootstrap/integrity report,
`foks-server usage` for stored usage against the capacity limits (see
[Capacity limits](#capacity-limits)) and `client-bootstrap` to write a
post-bootstrap client import bundle. Example
systemd units and deployment notes are in [`../../packaging`](../../packaging).

`server.toml` is parsed strictly: an unknown key or table is an error, so
`config-check` does not accept a misspelled setting. Besides the paths,
listeners, backup schedule and `[waitlist]` table, it holds four limit
tables. `init` writes them with their defaults; an omitted key or table takes
the same default.

- `[limits]` sets the RPC session limits and the writer queue:
  `worker_threads` (4), `maximum_read_connections` (32),
  `maximum_active_connections` (256), `maximum_pending_connections` (32),
  `maximum_kex_receive_polls` (32), `maximum_in_flight_requests` (64, at
  least 2), `maximum_realtime_polls_per_account` (8),
  `request_timeout_seconds` (30) and `maximum_pending_writes` (64). The
  `serve` flags of the same names set the same values.
- `[rate_limits]` sets the per-source admission limits: the token buckets
  `connection_burst` (512), `connections_per_second` (256), `request_burst`
  (2,000) and `requests_per_second` (1,000), `maximum_tracked_ips` (4,096),
  `ipv6_source_prefix` (64, 1 to 128), `maximum_connections_per_source`
  (32; 0 removes the limit) and `trusted_sources` (empty; a list of
  addresses or prefixes such as `"10.0.0.0/8"`, at most 256). The `serve`
  flags set the same values; the token bucket and tracking flags are named
  `--connection-rate-burst`, `--request-rate-burst` and
  `--maximum-rate-limit-ips`, and `--trusted-source` is repeated once per
  entry.
- `[storage]` sets database capacity: `maximum_database_bytes` (64 GiB),
  `maximum_teams` (4,096), `maximum_team_members` (64),
  `maximum_kv_namespace_bytes` (16 GiB) and `maximum_kv_namespace_objects`
  (1,000,000). Other database limits keep their compiled defaults. Every
  command that reads `server.toml` opens the database with these limits;
  `serve` has no flags for them and uses the defaults. `backup` and `restore`
  take their paths from flags and use the defaults, including the 64 GiB
  database ceiling, unless `--config` names a `server.toml`; pass it when the
  table raises `maximum_database_bytes`.
- `[chunk_capacity]` sets the physical capacity of an external chunk store:
  `maximum_bytes` (64 GiB), `maximum_objects` (1,000,000),
  `maximum_unresolved_attempts` (128) and `maximum_uncertain_bytes`
  (256 MiB). The `serve` flags `--maximum-chunk-bytes`,
  `--maximum-chunk-objects`, `--maximum-chunk-unresolved-attempts` and
  `--maximum-chunk-uncertain-bytes` set the same values. Each value must be
  at most 2^63 - 1.

Each value must be non-zero, except `maximum_kex_receive_polls`, where zero
refuses every KEX receive. `maximum_team_members` bounds the rows of one
team roster and must not exceed 1,024, the number of roster rows a realtime
mutation scans when it records message recipients; a larger roster could not
create channels or send messages. A configured `[web_admin]` listener requires
at least 3 read connections and 2 writer slots. `config-check` reports the
first condition that fails. `config-check --print-effective` writes the
configuration that `serve-config` would run, defaults included, as TOML to
standard output and the check report to standard error, so the output can be
redirected to a loadable `server.toml`. The zone TTL is not a setting: the
first start signs 60 seconds into the stored probe response, and later starts
refuse a different value.

A write that would grow the database past `maximum_database_bytes` fails in
SQLite with `SQLITE_FULL`. Every RPC route reports it as FOKS status 1060
(over quota), the same status as a write refused by a configured storage
quota, and the status detail does not include SQLite error text. On an SSO
host this includes the access-token refresh that runs before an authenticated
request: a full database during the refresh is reported as 1060, not wrapped
in an OAuth2 authentication error that clients treat as a reauthentication
requirement.

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
and actual handler duration (including work that outlives a client deadline,
and the time an OIDC completion, KEX receive or realtime inbox long poll
waits), writer queue wait and execution duration, database and WAL bytes, backup
duration, saturation, backup outcomes, transient `accept()` failures, and the
configured capacity limits (see [Capacity limits](#capacity-limits)). They
use fixed, non-user-derived names and never export paths, identities, request
values, or key material.

Each request has one deadline, `request_timeout_seconds` after the first byte
of its frame arrives. Reading the rest of the frame, waiting for a handler
permit, reader checkout, the handler, its queued writes and external chunk
transfers all consume it; idle time between requests does not. Once the
handler has returned, the response write gets a fresh `io_timeout` allowance,
so a response whose work used most of the deadline is still delivered to a
slow reader instead of being discarded. A request that reaches its deadline before the handler returns is
closed without a response. If its write is still in the writer queue at that
point, the writer does not run it: it opens no transaction for the job and
counts it in `foks_writer_expired_total`. A write that the handler submits
after that point is refused without entering the queue and is counted the same
way. A write that has already started runs to completion, and so do the writes
that settle an already published chunk and the write that records the outcome
of a committed SSO access-refresh claim. A rising count means that handlers,
including their wait in the writer queue, take longer than the request timeout
before their writes start. Long polls (OIDC completion, KEX receive and
realtime inbox) wait on their own timeouts instead, and their response write
gets the same fresh allowance.

A long poll does not block its connection. Go clients multiplex calls on one
connection by sequence number, and go-foks device pairing issues further KEX
calls while an hour-long KEX receive is pending. While long polls are
pending, the session keeps reading frames: a call is served (ordinary
requests one at a time, as on an idle connection), and each response is
written when it is ready, so responses can arrive in a different order than
their calls. A CANCEL control message for a pending poll's sequence number
ends that poll without a response and releases its permits; end of stream or
a read error closes the connection and drops its pending polls. One
connection holds at most four pending long polls; a fifth is answered with
status 1012 and the connection closes, counted as
`foks_admission_rejected_total{reason="connection_long_polls"}`. A connection
with a pending long poll is not idle, so `io_timeout` does not close it.

`foks_requests_total{family,status}` counts written responses by route family
(the protocol name, such as `KvStore` or `Reg`) and wire status code (`0` for
a success). Responses written before a route is resolved have
`family="unrouted"`: per-address rate-limit refusals, authentication
failures on the authenticated listener, unknown methods, methods on the wrong
listener and oversized arguments. A request whose client certificate is not
bound to an active credential, such as one from a revoked device whose
certificate has not expired, receives status 1006 (`AUTH_ERROR`, as on a Go
host) and the connection stays open; a storage fault during that lookup is
reported with site `authenticate`. A request that ends without a
response, for example on a handler timeout, appears only in
`foks_requests_started_total`. `foks_request_internal_errors_total{site,reason}`
counts server faults that a handler reported as status 1014 (transaction
retry). `site` names the handler's error mapper, such as `kv_write` or
`team_admin`, and `reason` is a fixed class such as `busy`, `corrupt_storage`,
`disk_full`, `io` or `internal`. Each site and reason also writes at most one
stderr line per minute, `FOKS request failed: site=..., reason=...,
count_since_last_line=...`, without error text. This counter is process-wide.
Handler paths that still map an error to status 1014 inline, without a mapper,
are not counted. `foks_reader_pool_exhausted_total` counts requests refused
with status 1012 because every pooled read connection was in use. The
`foks_reader_pool_open` gauge is the number of connections that pool holds
open; it never decreases, because the pool keeps every connection it opens.
`foks_rate_limited_requests_total` counts per-source token-bucket,
handler-permit, request-memory and KEX receive refusals.
`foks_admission_rejected_total{reason}` counts every admission refusal by its
cause: `connection_rate` and `request_rate` (the per-source token buckets),
`source_connections` (the per-source connection limit), `admission_queue` (a
full queue of accepted connections), `request_handlers` (every handler permit
in use), `account_realtime_polls` (the per-account realtime inbox poll limit),
`kex_receive_polls` (the global KEX receive limit and each source's share
of it), `connection_long_polls` (more than four long polls pending on one
connection) and `request_memory` (the shared request input, working and
response memory budgets). A request whose working or response memory runs
out is answered with status 1012 when the shortfall comes while its argument
is decoded, before any handler runs, or when its route only reads (KV reads
and listings, chain and Merkle loads, realtime reads). On any other route the
handler may already have committed a write, so the connection closes without
a response and the client keeps the outcome as unknown. `foks_kex_active_polls`,
`foks_kex_admitted_total`, `foks_kex_rejected_total` and
`foks_kex_completed_total{reason}` (`delivered`, `timeout`, `disconnected`,
`shutdown`, `failed`) describe KEX receives.

When OIDC is configured and the host has a rollout policy, `/metrics` also
reports the `foks_oidc_*` rollout gauges: mode, provider block, authorization
epoch and cohort counts. The sample runs on a blocking worker over one retained
read connection and answers scrapes for 30 seconds. A scrape that receives no
sample within 1 second, or whose sample fails, omits these gauges and
increments `foks_oidc_sample_failures_total`. Without OIDC configuration the
gauges are absent and `/metrics` does not read the database for them.

`/healthz` returns 200 while every core thread runs: the three RPC listeners,
the SQLite writer, the maintenance loop, the automatic backup scheduler, the
management listener, and the administration and OIDC listeners when they are
configured. None of them restarts in place. When one exits without a requested
shutdown, for example because a writer operation panicked or an accept loop
lost its listening socket, `/healthz` and `/readyz` return 503 and
`foks-server` shuts down the remaining threads and exits with status 1, so a
service manager that restarts on failure starts a new process. The writer runs
no further operations after a panic; queued and later writes fail, and most
handlers report that as FOKS status 1012. Readiness turns 503 as soon as an
RPC accept loop ends, without waiting for that listener's open sessions to
finish.

Every listener retries transient `accept()` failures (EMFILE, ENFILE, ENOBUFS,
ENOMEM, ECONNABORTED, the network errors that Linux reports for a pending
connection, and other errors that leave the listening socket usable). It
retries at once for ECONNABORTED and the pending network errors, which concern
only the connection that `accept()` dequeued, and after 50 ms otherwise. Only
errors that invalidate the listening socket end the accept loop. Retries are
counted in `foks_accept_errors_total{kind}`. With every connection limit
reached, the `serve-config` defaults can use up to an estimated 1,202 file
descriptors, more than systemd's default soft `RLIMIT_NOFILE` of 1,024.
`serve` and `serve-config` therefore raise the soft limit to the hard limit at
startup and print a warning when it stays below the estimate; the example unit
sets `LimitNOFILE=65536`.

Connection and request token buckets are enforced per source with bounded
tracking memory. A source is one IPv4 address, or one IPv6 prefix of
`ipv6_source_prefix` bits (a /64 by default), so that a host cannot obtain a
new bucket by choosing another address within its prefix. An IPv4-mapped IPv6
peer is its IPv4 source. The `[rate_limits]` table or the `serve` flags set
the refill rates, bursts, prefix length and maximum tracked source count. A
connection whose bucket is empty is closed without a response. A framed
request that exceeds its bucket is still read, within the frame and request
memory limits, and receives FOKS status 1012 at the request's own sequence
number; the connection then closes. A control frame (NOTIFY or CANCEL) over
the bucket has no sequence number and closes the connection unanswered.

Each listener also limits the connections one source holds at once, queued
or active, to `maximum_connections_per_source`, so that one source cannot
occupy all `maximum_active_connections` slots with long polls or slow
requests. A connection over the limit is closed without a response and
counted in `foks_admission_rejected_total{reason="source_connections"}`; at
most once a minute per listener, stderr gets a line `FOKS connection refused:
reason=source_connections, source=...` with the number of refusals since the
previous line. Loopback peers are exempt: they are local processes or a local
TCP proxy, and behind such a proxy every client would share one source.

The same holds for any forwarder or network address translation between the
clients and the server that rewrites the client's source address: a load
balancer or TCP proxy on another host that uses source NAT (for example a
Kubernetes Service with `externalTrafficPolicy: Cluster`, or Docker's
userland proxy), an office NAT, carrier-grade NAT, or an IPv6 site whose
clients share one /64. Every client behind it is one source. A Go or Rust
client holds a connection open for each realtime inbox poll, KEX receive or
OIDC completion poll, so a small number of devices behind one such address can
reach the default limit of 32, and a refused client sees only a reset
connection. Go v0.1.9 hosts have no such limit. For a forwarder the operator
controls, list its address or prefix in `trusted_sources`: those peers are
exempt from the per-source connection limit and the KEX relay's per-source
share, as loopback peers are. For clients behind NAT that the operator does
not control, raise `maximum_connections_per_source` or set it to 0 to remove
the limit. The token buckets still count a trusted source or a NAT address
as one source, so `connection_burst`, `request_burst` and their rates must
cover the aggregate of the clients behind it. Passthrough load balancers that
preserve the client address need neither setting. The server does not read
the PROXY protocol.

A realtime inbox poll (`RtPollInbox`) holds its connection for up to 55
seconds. One account, across all of its devices, can hold at most
`maximum_realtime_polls_per_account` polls; a poll over that limit receives
status 1012 (rate limited). The route is served only on the authenticated
listener, so that listener's `maximum_active_connections` bounds the total.
Each poll re-reads its inbox state from the reader pool at least once per
second and after every wake hint, and one hint can wake many polls at once.
Poll reads therefore wait for one of `maximum_read_connections / 2` permits
(at least one) before leasing a reader, so that polls never hold more than
half of the pool and ordinary requests are not refused with status 1012
because polls hold every reader.

A KEX receive can stay open for up to an hour. At most
`maximum_kex_receive_polls` receives, capped at half of
`maximum_active_connections`, are outstanding at once across all
connections; a receive over that limit is refused at once with status 1012
and is not queued, so receives cannot hold every connection of the public
listener. One source can hold at most an eighth of that budget (4 of the
default 32, and at least one), so that a single address cannot take every
pairing slot for an hour; a receive over its source's share is refused the
same way. Both refusals are counted in `foks_kex_rejected_total` and
`foks_admission_rejected_total{reason="kex_receive_polls"}`. Loopback peers
and `trusted_sources` are exempt from the per-source share.

The public KEX relay holds at most 4,096 pairing packets or 32 MiB of them,
each for up to two hours, and refuses a send at either bound with status 1012.
One source can hold at most a sixteenth of each (256 packets or 2 MiB), so
that it cannot fill the relay and block device pairing for other users.
Loopback peers and `trusted_sources` are exempt, as from the per-source
connection limit.

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
or other permissions. The SQLite database leaf must also be a regular,
non-symlink file. On Unix it must have exactly one hardlink and a reliable
device/inode identity. Startup captures that identity, takes the adjacent
writer lock, and rechecks the identity and link count before SQLite opens the
database; platforms or filesystems that cannot supply those semantics fail
closed. This is deliberately path-local and does not use a global inode lock
registry. `SIGINT` and `SIGTERM` stop accepts, cancel idle
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

The in-process maintenance loop runs once per minute through the ordinary writer.
It removes expired user/team reservations, idempotency records, recovery challenges,
team-view and TeamAdmin capabilities, log uploads older than a day, and waitlist entries
older than the configured retention, then requests a PASSIVE WAL checkpoint.
Manual online maintenance uses the same path. PASSIVE does not invoke SQLite's
busy handler to wait for readers or writers, but its I/O still runs on the sole
writer. The 60-second interval starts after the preceding pass ends. Explicit
`Database::checkpoint()` callers still request TRUNCATE and can wait for readers.
Lock expiry is enforced by the next acquirer rather than background deletion.
Uploads become eligible for reclamation after 24 idle hours only if no retained
directory-entry version references them. Overwritten, unlinked historical files
and linked incomplete uploads remain protected. Reclamation marks an upload
unavailable to reads, chunk writes and publication, removes bounded chunk batches,
and deletes the empty upload record last. It resumes after restart.

Each upload pass examines at most 128 age candidates and visits at most 128
reclaiming objects, removes at most 16 chunks and 32 MiB of stored chunk payload,
and checks a 50 ms work budget between SQL operations. Individual SQL operations,
commit/fsync and the other maintenance domains can take longer. The durable age
cursor advances even through entirely live history. Eligibility is not a promise
of deletion exactly at 24 hours; backlog, contention or failures can delay it.

Management metrics expose maintenance attempts, successes, failures, last-success
time, reclaimed upload/chunk/byte totals, and consecutive deferred passes without
object identifiers. `foks_maintenance_warning` becomes 1 after three consecutive
failures or 60 consecutive deferred upload passes. Alert on that gauge and on
unexpected growth in `foks_reclaimed_uploads_total`; metrics delivery never gates
cleanup. Counters reset on server restart. Pinned Go hosts retain their own upload
lifecycle policy; these guarantees apply to the Rust host.
Maintenance success means cleanup and the checkpoint call returned without an
error; it does not mean all WAL frames were backfilled or the file shrank.
Checkpoint metrics cover these explicit online passes only, excluding SQLite's
automatic checkpoints and direct database API calls:

- `foks_checkpoint_attempts_total` starts after cleanup succeeds. Each completed
  attempt increments one of `complete_total`, `deferred_total`,
  `unavailable_total`, or `errors_total` under the `foks_checkpoint_` prefix.
  `busy_total` is a subset of deferred outcomes; PASSIVE can be deferred with
  `busy=0` when a reader pins newer frames.
- `foks_checkpoint_duration_seconds` is a histogram with bounds 1, 5, 10, 50,
  100, 500 ms, 1 s, 5 s, and +Inf. It includes failed checkpoint calls.
- `foks_checkpoint_wal_frames`, `foks_checkpoint_backfilled_frames`, and
  `foks_checkpoint_remaining_frames` describe the last known WAL positions,
  including earlier backfill. They are gauges, not additive work counters.
- `foks_checkpoint_frame_counts_available` is 0 before a first observation,
  during an attempt, or after unavailable/error results. Prior frame gauges
  remain stale in those cases. Use `foks_last_checkpoint_observation_unixtime`
  and `foks_last_checkpoint_complete_unixtime` to assess freshness and the last
  confirmed completion. Timestamps use the maintenance pass's sampled clock.

Scraping these observations does not run a checkpoint or queue writer work.
Watch sustained remaining frames together with rising `foks_wal_bytes` and
checkpoint freshness. Physical WAL allocation alone can be reusable space, even
when all frames have been backfilled. Committed cleanup remains counted if a
later checkpoint fails.

Writable connections retain FULL synchronous mode and SQLite's normal automatic
checkpointing. `foks_server_db::Config::wal_reuse_limit_bytes` defaults to 16 MiB
and sets `main.journal_size_limit`; zero is allowed. This bounds allocation
retained on **natural WAL reuse**, not the active WAL. A complete PASSIVE
checkpoint can leave a large idle file until a later write resets it or
connections close. Long-lived readers can still prevent reuse and grow the WAL;
investigate them rather than forcing them out or deleting WAL files. This
setting is not a database-size quota and has no new CLI/TOML switch.

No schema migration is needed for this policy. Rolling back restores routine
TRUNCATE and its reader-wait risk; do not remove the WAL as a rollback procedure.
See [the checkpoint workload comparison](BENCHMARKS.md#wal-checkpoint-comparison)
for measurement commands and limits of the default reuse policy.

Foreground expiry, current-device, current-roster, role, and key-generation
checks enforce correctness even if this job never runs. There is no durable
general-purpose job queue, retry farm, cron dependency, or multi-process lease system.
An operating-system lock on the database sidecar rejects a second writer
process; active/active service instances are not supported.

## Realtime inbox reconciliation and schema 44

Clean `PollInbox` and `GetChangedThreads` requests read authenticated durable
inbox state without entering the writer queue. Membership changes invalidate
existing inboxes transactionally. Dirty or incomplete inboxes advance one bounded
page per request/iteration. Polls retain the one-second authorization and missed
notification check and the 25-second default/55-second maximum. The
per-account poll limit and the poll reads' share of the reader pool are
described under [Running](#running).

Schema 44 automatically upgrades only FOKS schema 43 during writer database
initialization, under the process lock and before reader pools open. Stop the
old server and make a consistent pre-upgrade backup first, including the WAL
through the normal backup mechanism; a live copy of the main file alone is not
a consistent backup. Do not mix old and new servers. Roll back by restoring that
backup with the old binary, never by changing `user_version` or deleting WAL
files. Existing messages, inbox versions, accessibility and read pointers survive
the upgrade. See [the migration policy exception](SCHEMA_MIGRATION_POLICY.md#implemented-development-exception-43-to-44).

Prometheus exposes `foks_realtime_reconcile_*_total` with fixed `source="poll"`
and `source="delta"` labels, plus queue-wait/execution histograms in seconds.
State reads count successful authenticated state checks; clean skips count
requests/iterations that bypass reconciliation after finding no pending work.
Submissions count attempts, with submission and execution failures separate.
Candidates include the lookahead row, not all SQLite rows scanned. Complete,
incomplete, already-clean and restarted page counts describe committed results.
Worker timings include execution-time credential checks. Poll wake and outcome
counters include fallback, hint, bump, timeout, failure and cancellation.

Idle reconciliation submissions should remain zero after initialization. Dirty
page latency still depends on membership/channel size; the existing 4096-channel
page cap bounds returned work, not every SQLite operation. Future channel policy
edits or deletion must add transactional invalidation or equivalent fanout.

## Capacity limits

Defaults are enforced before authoritative KV writes:

- 64 KiB per encoded small node at the database layer; public small-file RPCs
  enforce the Go-compatible 2 KiB plaintext plus authenticator boundary;
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
capabilities to 32 per member/team pair and 16,384 per class globally. The
waitlist holds at most 10,000 entries by default.
Passphrase history is limited to 4,096 generations. Login state is limited to
4,096 live challenges globally and eight per UID; five bad proofs within the
default ten-minute window temporarily rate-limit that UID.

`foks-server usage --config server.toml` reports stored usage against the
permanent and storage limits, one `resource=... used=... limit=... remaining=...`
line per limit, or one JSON object with `--json`. The limits are the last
Merkle epoch whose back-pointer array has a canonical encoding (65,535; see the
`foks-merkle-store` README), teams, unexpired unclaimed team-name
reservations, team invitation certificates (65,536), local and remote join
requests (1,000,000 and 65,536, counting accepted and rejected requests),
local view permissions (1,000,000), waitlist entries, main-database bytes
including free pages, and, for the largest KV namespace by each measure, its
bytes, objects, directories (4,096) and directory entries (100,000) against
the per-namespace limits. Caps on short-lived rows that expire (active team
capability tokens, remote view permissions, and passphrase and recovery
challenges) are not reported. The report also lists free database bytes, WAL
bytes, the ten largest KV namespaces by stored bytes and the ten teams with
the most stored realtime message bytes, by identifier only. The command reads
the configuration and the database but not the key, TLS or OIDC files. It
reads one snapshot over a read-only connection, so it runs while the server
serves, but it scans every realtime message row. `/metrics` therefore
exports only the limits, as `foks_capacity_limit{resource}` with the same
resource names; alerting can compare
`foks_capacity_limit{resource="database_bytes"}` with `foks_database_bytes`.

Namespace accounting charges the inline representation, including upload
envelopes and ciphertext duplication, even when chunks are stored externally.
Transactional counters avoid rescanning namespace history for each mutation.
Tombstone/history versions remain
charged until a future authenticated compactor can prove they are reclaimable.
SQLite, filesystem, WAL, backup, and key-directory overhead means operators
must reserve more disk than the configured database limit.

No throughput number is a product claim yet. The single writer, `FULL`
synchronous WAL commits, repeated reachable-tree version-vector construction,
and whole-database validation on cold startup remain saturation candidates.
Benchmark reports must state hardware, SQLite settings, dataset shape, p50/p95/
p99 latency, writer saturation, reader latency, WAL growth, and checkpoint
cost before quoting capacity.

## External encrypted chunks

Large encrypted file chunks can live outside SQLite without changing go-foks
RPCs, encryption, padding, exact upload retries, or usage replies. Small-file payloads (the Go-compatible 2 KiB plaintext boundary), directory
entries, file metadata and authoritative identity state remain in SQLite. Larger
text notes also use the chunk path. Mixed inline/external rows are supported; enabling a store affects
new uploads and does not copy existing files during startup.

Add one store table to the installation TOML used by `serve-config`:

```toml
[chunk_store]
kind = "s3"
bucket = "foks-chunks"
region = "auto" # Use the actual region for AWS S3.
endpoint = "https://ACCOUNT_ID.r2.cloudflarestorage.com"
prefix = "tenant-unique-namespace"
timeout_seconds = 10
```

Use a distinct, stable prefix per installation. Static credentials use `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and
optional `AWS_SESSION_TOKEN`; do not put secrets in the TOML. The backend also
supports its object-store library's environment-configured web identity,
container credentials and instance-metadata fallback; it does not load general
AWS shared profiles. Grant object GET and conditional PUT in the chosen prefix. Objects
must remain immutable and readable. Endpoint HTTPS is mandatory except an
explicit `allow_http_loopback = true` for literal loopback HTTP test endpoints.
Conditional creation must be supported; there is no overwrite fallback.

For a local filesystem instead:

```toml
[chunk_store]
kind = "filesystem"
directory = "/srv/foks/chunks"
```

The filesystem implementation fsyncs objects and directories, rejects symlink
leaves, and verifies existing objects. It cannot bound a hung filesystem call.
S3 uses one deadline across each operation, retries and verification reads. That
deadline alone does not bound the complete RPC: queuing, SQLite and client
retries also count toward the 15-second hosted target.

Object publication precedes the SQLite commit; authorization, ordering and quota
are checked again before commit. An interrupted or rejected commit can leave an
unreferenced object. **There is no object garbage collector in this release.**
Objects remain after upload cleanup or removal of their SQLite references. Do
not configure bucket lifecycle deletion. Monitor physical object growth; current
quota charging does not include orphan objects. History pruning and deletion
need coordinated writers, reads and backups before they are safe to enable.

To convert existing inline chunks, stop the server and run:

```sh
foks-server externalize-chunks --config /srv/foks/server.toml
```

The command takes the installation writer lock and commits each reference only
after object publication. It is resumable after interruption. It leaves freed
SQLite pages available for reuse; it does not vacuum or shrink the database.
Keep the selected store and prefix stable. Startup fails if external references
exist with no configured store; a wrong/missing object fails on access.

Library and scheduled backups fetch external chunks into the private SQLite
snapshot before publishing its completeness manifest. Completed backups remain
self-contained and restore without the original store. Missing/corrupt objects
fail backup creation. Hydration requires temporary disk and database capacity
for the original duplicated inline representation and can be slow; it is not a
15-second failover path. Scheduled shutdown checks cancellation between chunks.

For the standalone `serve` and `backup` commands, `--chunk-store-config PATH`
accepts a separate TOML file containing the same settings **without** the
`[chunk_store]` table header. Supply it to `backup` for an external source;
omitting it fails rather than publishing a backup that depends on the bucket.
Restore does not need this flag.

Separating chunk storage does not replicate or fence SQLite. Qualify the actual
object provider and metadata replication/failover deployment together. Local
mock tests establish API behavior, not S3/R2 durability across an AZ failure.

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
destination paths. Both commands open the database with the compiled limits
unless `--config /var/lib/foks/server.toml` supplies the `[storage]`,
`[waitlist]` and `[chunk_capacity]` tables; the paths still come from the
flags.

Restore and retention validation also accept the same prior schemas as server
startup (43 through 59, upgraded to 60). They copy the database and any WAL into
a private temporary directory, apply the normal transactional migrations,
checkpoint, and validate that copy. Restore installs the migrated database;
validation never opens or changes the original snapshot in SQLite. Retention
can still remove validated snapshots according to its configured count.
Unsupported schemas, failed migrations, incomplete key sets, and symlinked
artifacts fail validation before a restore destination is created or retention
deletes any snapshots. Restoring a schema older than 48 invalidates its old SSO
browser challenges, sessions, access state, and binding receipts, as normal
startup migration does.

Provide temporary disk space for the database, optional WAL, and migration
work. Each input file is limited to `maximum_database_bytes`; migration also
uses the configured database limit. Temporary copies are removed on success
and on returned errors. Backup directories must remain immutable while being
validated or restored; copying a concurrently changing live database is not a
supported backup procedure.

The SQLite snapshot includes names, invite policy/hash/redemption metadata,
passphrase salts and encrypted PPE history, recovery credentials, user/team
chains, current projections, encrypted PUK/PTK histories, capability policy
state, encrypted federation bearer envelopes plus their hashes, waitlist email
addresses, and all user/team KV namespaces. Plaintext bearer tokens are never stored. Restored
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

The gate first runs `scripts/check-crate-layers.py` (Python 3.11 or newer),
which reads the declared dependencies from `cargo metadata` (optional and
target-specific ones included) and checks them against
`tools/crate-layers.toml`. It rejects a workspace package defined outside
`crates/foks-*` or `apps/desktop/src-tauri`, a path dependency that is not a
workspace package, a server crate that depends on a client, device, agent or
front-end crate, any other edge the layer rules do not allow and no listed
exception covers, and a production dependency, direct or transitive, on
`foks-server-testkit`.
The optional official-Go frame audit is
`tools/foks-v019-oracle/run-live-team-compat.sh`; it needs a Go 1.25-compatible
toolchain and may populate Go compiler/module caches, but does not use a state
path outside its test-owned temporary root.

Run `tools/foks-server/generate-protocol.sh --write` only for a reviewed
baseline or policy update. The scheduled
`tools/foks-server/diff-upstream-protocol.sh` audit compares the pinned artifact
with upstream's immutable current commit, publishes semantic JSON/Markdown
drift reports, and never edits the baseline or a lockfile.

The standalone direct client, local agent, and desktop backend have a separate
dependency-boundary and test gate:

```text
tools/foks-client/check.sh
```

Organization authentication setup, operational policy and acceptance guidance are
in [OIDC.md](OIDC.md).

The optional Rust-hosted administration application supports explicit host operator
grants, signup invites/policy and browser-session management. It preserves the
pinned native handoff RPCs and exchanges tickets once for short-lived browser
sessions. See [ADMINISTRATION.md](ADMINISTRATION.md) for HTTPS deployment, bootstrap,
authority and backup/restore rules. It is disabled by default.

## Operational failure diagnostics

Scheduled backups and manual/periodic maintenance emit a bounded stderr record:
`FOKS operation failed: phase=backup_retention, reason=schema_mismatch`.
Backup phases are `backup_read`, `backup_create`, `backup_publish`, and
`backup_retention`. Maintenance phases are `maintenance_admission` (including
clock sampling before dispatch), `maintenance_expiry`, `maintenance_admin`, and
`maintenance_checkpoint`.

Reasons are `disk_full`, `permission_denied`, `io`, `schema_mismatch`,
`corrupt_storage`, `busy`, `capacity`, `invalid_state`, `storage`, or `internal`.
These records contain no paths, SQL, identities, request content, or underlying
error text. Existing failure counters and committed-reclamation accounting retain
their meanings; a retention failure can occur after a new snapshot is published.
