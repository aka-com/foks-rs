# Server, storage and operability

Area key `server`. 15 verified findings. Part of the [foks-rs review](../README.md).

## Current state

The server is carefully engineered at the request and SQL layers. It has typed generated routes, STRICT schemas with partial-unique-index state machines, a bounded writer actor with re-authorization inside the queue, bounded maintenance, PASSIVE checkpoints, loopback health/readiness/metrics, and a large process-level conformance suite. The main risks are at the scale and lifetime level, not in individual handlers:

- **Merkle cost and lifetime.** Every identity or team mutation mints one Merkle epoch, and the epoch count has a hard 65,535 ceiling. No per-principal budget protects it: one account posting user-settings generic links can exhaust it, because generic chains have no link cap. Each publication also rebuilds the whole Merkle tree on the single writer thread. A scratch benchmark of `foks_merkle_store::prepare`, using the in-memory node store, measured about 103 ms per mutation at 10k leaves, 655 ms at 50k, and 1.5 s at 100k.
- **Admission and fairness.** The server applies global caps but almost no per-principal or per-source sub-budgets:
  - an unauthenticated waitlist insert with no row limit;
  - no per-IP cap on concurrent connections, while KEX long polls can last an hour;
  - a single 32-slot realtime poll pool;
  - a 4,096-team global cap that one account can fill;
  - a FIFO writer queue with no deadlines.
- **Failure reporting.** About 300 call sites convert errors to `TransactionRetry` with no log or metric. Error-to-status mapping is duplicated across about 13 `map_write_error` functions. As a result, a full database is reported as retryable everywhere except KV, and team create/edit errors send internal error text (including SQLite messages) to clients.
- **Process liveness.** A transient `accept()` error or a writer-thread panic leaves the process running with no service, while `/healthz` still returns `ok`.
- **Operations.** `server.toml` cannot set any limit, and unknown keys are silently ignored. `/metrics` scans all KV tables on every scrape. Online backup does not hold a read snapshot across steps, so continuous writes can restart a large backup indefinitely. Operators have no capacity or abuse tooling (epoch headroom, cap usage, account suspension).
- **Structure.** `net/session.rs` mixes the transport loop with about 1,000 lines of signup, user-mutation and Merkle-query logic. The DB crate repeats repository forwarding methods across three connection types, and schema upgrades are a single ad-hoc block tested against fixtures built by reversing the current schema.

ISSUES.md stages 1, 3, 5 and 6 already cover chat, invitation and fanout retention and caps. The findings below add the shared mechanisms those stages need: a usage ledger, a storage reserve, per-principal budgets, and status-mapping and observability plumbing.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [A transient accept() error or a writer panic leaves a live process with no service, and /healthz still returns ok](#server-fatal-thread-exit) | correctness-risk | high | M |
| [One authenticated account can permanently exhaust the host's 65,535 Merkle epochs (and the 4,096-team cap)](#server-merkle-epoch-budget) | security | high | M |
| [No per-source concurrency caps: one IP can hold every public connection slot, and one account can take every realtime poll slot](#server-source-admission-fairness) | security | high | M |
| [Replace per-write KV namespace scans with a transactional usage ledger and add a storage reserve for identity writes](#server-storage-usage-ledger) | performance | high | L |
| [Unauthenticated joinWaitList inserts unbounded, permanent, unreadable PII rows](#server-waitlist-unbounded) | security | high | S |
| [Online backup steps without a held read snapshot, so continuous writes can restart a large backup indefinitely](#server-backup-snapshot-restart) | correctness-risk | medium | S |
| [server.toml cannot set any limit, ignores unknown keys, and gives misleading validation errors](#server-config-surface) | maintainability | medium | S |
| [About 13 divergent map_write_error functions: a full database is reported as retryable, and internal error text leaks to clients](#server-error-status-mapping) | correctness-risk | medium | M |
| [Full Merkle rebuild on the single writer thread grows to seconds per identity/team mutation](#server-merkle-incremental-prepare) | performance | medium | L |
| [Every /metrics scrape and every new pooled reader full-scans the KV tables](#server-metrics-scrape-cost) | performance | medium | S |
| [Operators have no capacity, headroom or abuse controls (usage report, account suspension, waitlist)](#server-operator-capacity-console) | missing-feature | medium | M |
| [Request failures are invisible: no per-route/status metrics, no internal-error reasons, no latency histograms](#server-request-failure-observability) | missing-feature | medium | M |
| [net/session.rs mixes transport and domain logic, and request authorization relies on cloning ServerData by convention](#server-session-decomposition) | maintainability | medium | M |
| [Writer queue runs abandoned jobs and has no priority lanes or per-principal fairness](#server-writer-deadlines) | performance | medium | M |
| [Schema upgrades are one ad-hoc branch block tested only against fixtures built by reversing the current schema](#server-migration-framework) | testing | low | M |

### server-fatal-thread-exit

**A transient accept() error or a writer panic leaves a live process with no service, and /healthz still returns ok**

- Type: correctness-risk
- Priority: high
- Effort: M
- Layers: server, tooling, docs
- Verification: adjusted

The acceptor propagates any accept() error with `?`. EMFILE, ENFILE, ECONNABORTED and ENOBUFS can occur under load. The serve-config defaults allow about 3 x (256 active + 32 pending) sockets plus 32 SQLite reader connections, each with database, WAL and shm descriptors. That is close to the 1,024 soft RLIMIT_NOFILE systemd applies when a unit sets no LimitNOFILE.

One such error stops that listener's acceptor immediately. The listener then waits for its existing sessions to finish (KEX receives can last up to an hour) before returning the error. At that point tokio::try_join! drops the other two listeners and the runtime thread exits. /readyz turns 503 only after the thread exits, when LivenessGuard clears service_live, so during the drain one listener is dead while readiness still reports ready. After the drain the main thread still blocks only on SIGINT/SIGTERM, so the process stays alive, Restart=on-failure never fires, and /healthz keeps returning 200.

A panic inside any writer closure kills the writer thread, which has no catch_unwind or monitor. Later writes fail with Error::WriterQueue, which most handlers map to the retryable RateLimited status. /readyz detects this case through its writer round trip, but nothing exits or restarts the process.

**Evidence**

- [`crates/foks-server/src/net/listener.rs:281`](../../../crates/foks-server/src/net/listener.rs#L281): `let (stream, peer) = accepted?;` ends the acceptor task on the first accept error
- [`crates/foks-server/src/net/listener.rs:351`](../../../crates/foks-server/src/net/listener.rs#L351): after the admission channel closes, run_listener awaits every in-flight session (join_next) before returning accept_result, so the failure reaches try_join! only after the listener's long polls drain
- [`crates/foks-server/src/net/listener.rs:240`](../../../crates/foks-server/src/net/listener.rs#L240): tokio::try_join! over the three run_listener futures, so one failure drops all listeners and ends the runtime thread
- [`crates/foks-server/src/net/listener.rs:83`](../../../crates/foks-server/src/net/listener.rs#L83): LivenessGuard clears the listener liveness flag when the runtime thread exits; this is the service_live input to readiness
- [`crates/foks-server/src/operations/management.rs:229`](../../../crates/foks-server/src/operations/management.rs#L229): readiness() already returns false when service_live is false or the writer round trip fails; /healthz (line 186) is a static 200
- [`crates/foks-server/src/bin/foks-server.rs:680`](../../../crates/foks-server/src/bin/foks-server.rs#L680): serve(), also used by serve_config (line 520) from the systemd unit, blocks on signals.forever().next() only, with no watch on listener, writer, maintenance or backup thread death
- [`crates/foks-server/src/writer.rs:351`](../../../crates/foks-server/src/writer.rs#L351): run() calls task.run(database) with no catch_unwind; a panic ends the thread, and later try_send/recv fail with Error::WriterQueue, mapped to RpcStatus::RateLimited (e.g. net/session.rs:488, services/generic.rs:430)
- [`packaging/foks-server/foks-server.service:11`](../../../packaging/foks-server/foks-server.service#L11): Restart=on-failure with no LimitNOFILE

**Recommendation**

1. **Retry transient accept errors.** Handle EMFILE, ENFILE, ENOBUFS, ENOMEM and ECONNABORTED by incrementing foks_accept_errors_total{kind}, sleeping 50-100 ms and continuing. Treat only listener closure as fatal.
2. **Exit on fatal thread loss.** Add a fatal-exit channel. The listener thread signals it on unexpected exit, without waiting for its sessions to drain. The writer thread signals it through a drop guard or catch_unwind around task.run, and the maintenance and backup threads signal it the same way. serve() selects on signals or fatal and exits non-zero on fatal, so Restart=on-failure applies. Do not continue writing after a writer panic.
3. **Liveness probe.** /readyz already reflects listener exit and writer failure. Make /healthz return 503 when any core thread has exited, or document /readyz as the probe operators must use. Clear service_live as soon as an acceptor fails, not after the drain.
4. **File descriptors.** At startup, compute the descriptor budget from SessionLimits and the reader count, then raise the soft RLIMIT_NOFILE to the hard limit, or warn or fail when it is insufficient. Add LimitNOFILE=65536 to the unit.
5. **Tests.** Cover the accept-retry path with an injectable accept wrapper. Cover writer-panic-to-process-exit in testkit.

<details><summary>Verifier note</summary>

The core holds. listener.rs:281 propagates any accept error with `?`. try_join! at lines 239-243 tears down the other listeners. serve() (bin/foks-server.rs:680, also reached from serve_config at line 520, which the systemd unit runs) blocks only on signals. writer.rs:351-357 has no catch_unwind or monitor, so after a panic try_send and recv fail with Error::WriterQueue, which most handlers map to RateLimited (session.rs:488, generic.rs:430, kv.rs:1225 and others). /healthz is a static 200 (management.rs:186). The unit has Restart=on-failure and no LimitNOFILE. There is no panic=abort release profile. The finding omits two existing behaviours that change the observability claim:
- /readyz already returns 503 once the listener thread exits, because LivenessGuard (listener.rs:83, 149-153) clears the service_live flag that readiness() checks (management.rs:229). It also returns 503 when the writer round trip fails.
- After an accept error, the failing listener does not return immediately. It drops its admission channel and then awaits all of its in-flight sessions (listener.rs:351-362) before returning the error. KEX receives can last an hour, so one listener can be dead for that long while readiness still reports ready. Only then does try_join! stop the other listeners.

The file-descriptor arithmetic is plausible. serve_config hardcodes 256 active and 32 pending per listener across 3 listeners, plus 32 SQLite readers that each hold database, WAL and shm descriptors, which approaches the systemd default soft limit of 1,024. Priority stays high because a transient EMFILE can become a permanent unrestarted outage.

</details>

### server-merkle-epoch-budget

**One authenticated account can permanently exhaust the host's 65,535 Merkle epochs (and the 4,096-team cap)**

- Type: security
- Priority: high
- Effort: M
- Layers: server, docs
- Verification: confirmed

Every user, team and generic-chain mutation mints exactly one Merkle epoch, and the v0.1.9 skip-pointer encoding makes epoch 65,536 unpublishable, so a host can accept only 65,535 such mutations in its lifetime. Nothing budgets that resource per principal. Generic user-settings links have no link-count cap, and team creation counts against a 4,096-team global cap with no per-creator limit and no team deletion. One device can therefore loop UserPostGenericLink, or create and edit teams, until signups, device revocations and team edits fail for everyone with MERKLE_VERIFY_ERROR. Operators cannot see the headroom: no metric, status output or admin page reports the current epoch.

**Evidence**

- [`crates/foks-merkle-store/src/history.rs:11`](../../../crates/foks-merkle-store/src/history.rs#L11): MAX_CANONICAL_MERKLE_EPOCH = 65_536; back_pointer_hash rejects 16..=31 pointers (lines 33-40)
- [`crates/foks-server/src/merkle.rs:38`](../../../crates/foks-server/src/merkle.rs#L38): prepare_publication always mints authoritative.epoch + 1, so one epoch per mutation with no cross-request batching
- [`crates/foks-server/src/services/generic.rs:20`](../../../crates/foks-server/src/services/generic.rs#L20): post() commits a generic link (and a Merkle epoch) for any ordinary device; foks-server-db/src/generic.rs has no per-chain link-count limit (only blob, node and back-pointer bounds at lines 292-348)
- [`crates/foks-server-db/src/team.rs:197`](../../../crates/foks-server-db/src/team.rs#L197): team creation checks SELECT count(*) FROM teams against the global maximum_teams (4,096) only; there is no per-creator cap and no team deletion path
- [`crates/foks-server-db/src/config.rs:59`](../../../crates/foks-server-db/src/config.rs#L59): maximum_user_chain_links and maximum_team_chain_links are 4,096 each, so per-chain caps alone do not keep total epochs under 65,535
- [`crates/foks-server/src/operations/management.rs:275`](../../../crates/foks-server/src/operations/management.rs#L275): the Prometheus export has no Merkle epoch gauge; `foks-server status` (bin/foks-server.rs:589) prints only bytes and integrity

**Recommendation**

Sequence this in three steps.

1. **Visibility (S).** Export `foks_merkle_epoch` and `foks_merkle_epochs_remaining`, read from merkle_root_heads in the existing metrics read. Add `epoch=` and `epochs_remaining=` to `foks-server status`. Add a `foks_merkle_epoch_warning` gauge at 80% and 95%.
2. **Budgets (M).** Classify epoch-consuming routes from the generated RouteId table. Signup, provision, revoke, username change, generic link, team create/edit/removal and host rotation all consume epochs.
   - Add a per-UID token bucket for epoch-consuming writes, checked inside the writer closure so it is authoritative. An in-memory LRU keyed by UID is enough to start. Make the default generous, for example 60 per hour with a burst of 20.
   - Add a per-creator cap on teams. Add a per-chain link cap for generic chains that mirrors maximum_user_chain_links.
   - Reserve the last N epochs (for example 2,048) for security-critical operations only: device revocation, team member removal, recovery and host-key rotation. Ordinary mutations get QuotaExceeded once headroom drops below the reserve.
   - None of this changes the wire, so Go interop is unaffected.
3. **Amortization (L, optional).** Group-commit concurrent queued Merkle mutations into one epoch. See server-merkle-incremental-prepare and server-writer-deadlines for the writer mechanics.

Add a testkit case that posts generic links until the per-UID budget refuses them, and checks that a revocation still succeeds inside the reserve.

**Already tracked:** The ceiling itself is documented as a protocol limit in book/12-epochs.qmd ('The epoch 65,536 ceiling') and in crates/foks-merkle-store/README.md. Neither ISSUES.md nor the book covers per-principal exhaustion, reserved headroom or an epoch metric.

<details><summary>Verifier note</summary>

All cited code matches. history.rs:11 sets MAX_CANONICAL_MERKLE_EPOCH = 65_536, and back_pointer_hash rejects pointer counts 16..=31 at lines 33-40. merkle.rs:38 always mints authoritative.epoch + 1. Every publisher calls prepare_publication with 1-3 leaves: signup and user mutations in net/session.rs:319 and :651, generic links in services/generic.rs:209, and team create and edit in team_admin.rs:455 and :763. EpochLimitExceeded maps to MERKLE_VERIFY_ERROR (server/src/error.rs:62). foks-server-db/src/generic.rs has no sequence or link-count check against maximum_user_chain_links; only user_mutation.rs:408 and team.rs:818 apply chain caps. The user's own TEAM_MEMBERSHIP generic chain has no payload precondition, and USER_SETTINGS links only require an existing passphrase generation, so one active device can loop User.postGenericLink (policy-v1.toml:692, active_device_mtls). team.rs:197-201 checks only the global count(*) against maximum_teams (4,096). No Rust code deletes from teams; the kv.sql delete trigger is never exercised. The only throttle is the per-IP 1,000 req/s bucket (rate_limit.rs). No per-UID budget exists: grep found only web_admin capacity helpers. The Prometheus export (management.rs:275 onward) has no epoch gauge, and `status` (bin/foks-server.rs:589) prints bytes and integrity only. book/12-epochs.qmd and book/23-compatibility.qmd document the ceiling but not per-principal exhaustion, reserve headroom or metrics, and ISSUES.md does not mention it. The recommendation is server-local admission and metrics with no wire change, so it is feasible under the Go v0.1.9 constraint.

</details>

### server-source-admission-fairness

**No per-source concurrency caps: one IP can hold every public connection slot, and one account can take every realtime poll slot**

- Type: security
- Priority: high
- Effort: M
- Layers: server, docs
- Verification: confirmed

Admission is limited by per-IP token buckets on new connections and frames, plus global semaphores. Nothing bounds how many connections or long polls one source holds at once, which leaves these exposures:
- One IP inside its 512-connection burst can open all 256 public-listener slots and keep them with KexReceive polls of up to one hour, or by sending one small frame every 14 s (15 s I/O timeout, 4,096 requests per connection).
- Buckets are keyed by the full IpAddr, so an IPv6 /64 gets 2^64 independent buckets, and LRU eviction at 4,096 entries hands evicted attackers a full bucket.
- Realtime long polls share one global 32-permit semaphore with no per-UID limit, so a single account can make every other member's chat poll fail with RateLimited.
- The anonymous KEX relay keeps 4,096 messages or 32 MiB for 2 hours and refuses new sends when full, so one source can block device pairing for everyone.

**Evidence**

- [`crates/foks-server/src/net/listener.rs:282`](../../../crates/foks-server/src/net/listener.rs#L282): only allow_connection(peer.ip()) is checked; the active-connection semaphore (line 297) is global per listener with no per-IP count
- [`crates/foks-server/src/rate_limit.rs:78`](../../../crates/foks-server/src/rate_limit.rs#L78): buckets are keyed by the full IpAddr in an LruCache; a new or evicted address starts with a full burst
- [`crates/foks-server/src/net/session/kex.rs:13`](../../../crates/foks-server/src/net/session/kex.rs#L13): MAX_BLOCKING_WAIT = 1 hour for unauthenticated receives; send() refuses at MAX_MESSAGES/MAX_RELAY_BYTES globally (lines 55-62)
- [`crates/foks-server/src/net/session.rs:160`](../../../crates/foks-server/src/net/session.rs#L160): realtime_polling = min(maximum_in_flight_requests, 32), one global pool; the poll path (line 1478) uses try_acquire_owned with no per-principal accounting
- [`crates/foks-server/src/net/session.rs:1413`](../../../crates/foks-server/src/net/session.rs#L1413): the KEX receive branch holds the connection until the relay answers, the client becomes readable, or stop

**Recommendation**

1. **Per-source connection cap.** Add a `SourceKey` that maps IPv4 to /32 and IPv6 to /64 (prefix configurable). Use it for the token buckets and for a new per-source concurrent-connection counter, with a default of 32 per listener, checked in the acceptor before try_send and released by a guard in the session task.
2. **Bound long-lived connections.** Add a per-connection lifetime, for example 2 h, and a per-source cap on concurrent long polls (KexReceive, SSO poll, RtPollInbox), for example 4.
3. **Per-UID realtime polls.** Replace the single 32-permit pool with a per-UID cap (for example 4 polls per account) plus a global cap derived from the authenticated listener's maximum_active_connections. Polls are async and hold no blocking thread, so 32 is lower than it needs to be.
4. **KEX relay fairness.** Track bytes and messages per SourceKey in the relay, with a cap of about 1/16 of global capacity, so one source cannot fill it.
5. **Observability and tests.** Export rejections as `foks_admission_rejected_total{reason}`. Add testkit/listener_boundaries.rs cases for one IP holding N > cap connections, and for IPv6 rotation within a /64.

All of this is server-local admission, so wire compatibility is unaffected.

**Already tracked:** ISSUES.md Stage 5 notes that per-IP limiting does not give per-certificate accounting for invitations. Concurrent-connection, IPv6 prefix, KEX relay and realtime-poll fairness are not tracked.

<details><summary>Verifier note</summary>

All cited mechanics hold.
- **Connections:** listener.rs:282 checks only allow_connection(peer.ip()). The active-connection semaphore (around line 298) is per listener and has no per-source accounting.
- **Rate limiter:** rate_limit.rs keys an LruCache on the full IpAddr, with 4,096 entries, burst 512 and 256/s. A new or evicted address gets a full bucket via get_or_insert_mut. book/18-server-storage.qmd discloses the eviction leniency as a deliberate trade-off, but the /64 aggregation gap is not discussed.
- **KEX receive:** Kex.receive is public, unauthenticated and on public_services (policy-v1.toml:186-196). MAX_BLOCKING_WAIT is 1 h (kex.rs:13), and session.rs:1413 onward holds the connection until the relay answers, the socket becomes readable, or the server stops.
- **KEX relay:** send refuses globally at 4,096 messages or 32 MiB with a 2 h lifetime and no per-source share. Senders only need a self-generated device key with a valid signature.
- **Idle connections:** io_timeout is 15 s and maximum_requests is 4,096 (config.rs).
- **Realtime polls:** realtime_polling = min(maximum_in_flight_requests, 32) is one global semaphore in ServerData (session.rs:162). The poll path at around line 1478 uses try_acquire_owned and returns RateLimited on failure, with no per-UID accounting. Polls last up to 55 s (handlers/realtime.rs).

ISSUES.md Stage 5 covers only invitation per-certificate accounting. The recommendations are server-local admission controls with no wire impact, so they are feasible.

</details>

### server-storage-usage-ledger

**Replace per-write KV namespace scans with a transactional usage ledger and add a storage reserve for identity writes**

- Type: performance
- Priority: high
- Effort: L
- Layers: server, docs
- Verification: adjusted

ensure_kv_capacity runs `sum(length(...))` and `count(*)` over seven KV tables for the namespace on every KV mutation, including every chunk upload (up to 512 per file), inside the writer. The cost is O(namespace objects), up to 1,000,000, and the README names these scans as an expected first saturation point. Separately, every retention problem in ISSUES Stages 1, 3 and 5 needs per-scope accounting (account, team, channel, certificate) that does not exist yet. All data classes share one max_page_count ceiling. Chat, KV, waitlist or log uploads can therefore fill the file and block device revocation and team removal, which are the writes that matter most during an incident.

**Evidence**

- [`crates/foks-server-db/src/kv.rs:1364`](../../../crates/foks-server-db/src/kv.rs#L1364): ensure_kv_capacity sums lengths and counts across kv_directories, roots, nodes, dirents, uploads, chunks and locks per call
- [`crates/foks-server-db/src/kv.rs:560`](../../../crates/foks-server-db/src/kv.rs#L560): called on every put_kv_file_chunk; also at lines 209, 267, 353, 646 and 735
- [`crates/foks-server-db/src/connection.rs:399`](../../../crates/foks-server-db/src/connection.rs#L399): the only storage admission is the page_count/max_page_count ceiling, with no reserve tier
- [`crates/foks-server-db/src/realtime/limits.rs:3`](../../../crates/foks-server-db/src/realtime/limits.rs#L3): realtime limits bound requests but define no retained-byte budget (ISSUES Stage 1)
- [`crates/foks-server-db/src/team_invitations.rs:134`](../../../crates/foks-server-db/src/team_invitations.rs#L134): invitation admission counts historical rows for global caps (ISSUES Stage 3)

**Recommendation**

1. **Ledger.** Add a STRICT `storage_usage(scope_kind, scope_id, bytes, rows)` table in schema 49 and backfill it in the migration. Apply deltas in the same writer transaction. Where ON DELETE CASCADE removes child rows (kv_file_uploads → kv_file_chunks, kv_namespaces → all KV tables), either compute the cascaded bytes before the parent delete or use AFTER DELETE triggers on the child tables, and test both reclamation paths. Maintenance recomputes one sampled scope per pass and exports `foks_storage_ledger_drift_total`.
2. **Fast quota check.** Point ensure_kv_capacity at the ledger, an O(1) primary-key read. Express chat (Stage 1) and invitation (Stages 3/5) budgets as ledger checks in the same transaction.
3. **Reserve tiers aligned with ISSUES Stage 1.** Unbounded-growth bulk classes (chat, waitlist, log upload) are refused with QuotaExceeded once used bytes exceed `maximum_database_bytes - reserve`. KV stays bounded by its per-namespace quota plus an aggregate KV budget below the reserve line. Identity, team, revocation and invitation-decision writes may consume the reserve.
4. **Metrics.** Export `foks_storage_bytes{class}` and `foks_storage_reserve_bytes`.
5. **Test.** Fill the database with chat and KV, then prove revocation, team removal and an in-quota KV write still commit.

**Already tracked:** ISSUES.md Stages 1, 3 and 5 list the required per-scope limits and the database reserve as requirements without a mechanism. crates/foks-server/README.md ('Capacity limits') names namespace quota scans as a saturation point. This finding adds the shared ledger, the reserve-tier design and the sequencing.

<details><summary>Verifier note</summary>

Most claims are confirmed. ensure_kv_capacity (kv.rs:1364) runs seven sum(length)/count(*) subqueries per call, at call sites 209, 267, 353, 560, 646 and 735. Line 560 runs on every chunk put, and MAXIMUM_CHUNKS is 512 (kv.rs:462). The default maximum_kv_namespace_objects is 1,000,000 (config.rs:86), and the README (line 379) names namespace quota scans as a saturation point. The only database-wide admission is the page_count/max_page_count ceiling in connection.rs:399-415. No reserve tier or usage ledger exists anywhere in foks-server or foks-server-db. Schema is at version 48, so 49 is next. ISSUES Stages 1, 3 and 5 state the requirements without a mechanism, as the finding says. Two recommendation corrections are needed. (1) ISSUES Stage 1 says the reserve must protect 'account, team, invitation and KV writes' from chat. The proposed tiering puts KV in the bulk class with chat, which contradicts the tracked requirement; KV should keep its own per-namespace budget and not compete with chat for the same headroom. (2) kv.sql uses ON DELETE CASCADE throughout: kv_file_chunks cascades from kv_file_uploads, and every KV table cascades from kv_namespaces. Maintenance upload reclamation therefore deletes chunk bytes implicitly. Accounting applied only through explicit repository deltas, with triggers ruled out, will drift unless each deleting path pre-computes cascaded bytes. The design must name this.

</details>

### server-waitlist-unbounded

**Unauthenticated joinWaitList inserts unbounded, permanent, unreadable PII rows**

- Type: security
- Priority: high
- Effort: S
- Layers: server, protocol, cli, docs
- Verification: confirmed

Reg.joinWaitList is served on the public listener without client authentication. Each call inserts a fresh waitlist row with a server-generated ID and the submitted email address. The table has no row cap, no uniqueness on email, no retention or expiry family, and nothing in the server or CLI reads it. An unauthenticated client can therefore turn the 1,000 requests/second per-IP budget into permanent database growth. Rotating IPv6 source addresses defeats even that budget. Growth continues until the single max_page_count ceiling stops every write on the host. The data collected is personal information that the operator cannot list, export or purge without opening SQLite directly.

**Evidence**

- [`crates/foks-server/protocol/policy-v1.toml:425`](../../../crates/foks-server/protocol/policy-v1.toml#L425): joinWaitList: listeners = ["public_services"], authentication = "delegated_tls", supported = true
- [`crates/foks-server/src/services/waitlist.rs:11`](../../../crates/foks-server/src/services/waitlist.rs#L11): join_waitlist generates a random ID and enqueues an insert for every call, with no admission check
- [`crates/foks-server-db/src/waitlist.rs:6`](../../../crates/foks-server-db/src/waitlist.rs#L6): join_waitlist performs a plain INSERT with no count limit or deduplication
- [`crates/foks-server-db/src/schema/waitlist.sql:1`](../../../crates/foks-server-db/src/schema/waitlist.sql#L1): waitlist_entries has a non-unique (email, created_at) index and no expiry column used by maintenance
- [`crates/foks-server-db/src/maintenance/expiry.rs:25`](../../../crates/foks-server-db/src/maintenance/expiry.rs#L25): the expiry families (sso_sessions, names, team_names, request_receipts, ...) do not include waitlist_entries

**Recommendation**

1. **Gate the route.** Add `[waitlist] enabled = false` to InstallationConfig. When it is disabled, answer Unsupported, which is the status already used for supported=false routes, so the Go client is unaffected.
2. **Bound and deduplicate.** Add UNIQUE(email) in schema 49 and return the existing waitlist_id for a repeated email, which keeps the call idempotent and Go-compatible. Add a global row cap, for example 10,000 (configurable), and return RateLimited (an already-listed status) when it is reached.
3. **Retention.** Add a `created_at`-based expiry family, for example 180 days, to maintenance/expiry.rs with the standard LIMIT 128 page.
4. **Operator access.** Add `foks-server waitlist list|export --json|purge [--before]` as offline commands under DatabaseWriterGuard, and show the row count on the web-admin overview.
5. **Test.** Add a testkit case that issues N+1 joins and asserts the cap is enforced and the table size is bounded.

<details><summary>Verifier note</summary>

policy-v1.toml:423-434 declares Reg.joinWaitList on public_services with authentication = "delegated_tls" (no client certificate) and supported = true. services/waitlist.rs generates a random 13-byte ID and enqueues an insert on every call with no admission check. foks-server-db/src/waitlist.rs:6-18 performs a plain INSERT after email-shape validation. schema/waitlist.sql has only a non-unique (email, created_at) index. maintenance/expiry.rs has no waitlist family. A repo-wide grep finds no reader of waitlist_entries outside one DB unit test: no CLI, web-admin, export or purge path. The only bounds are the per-IP request bucket (1,000/s; rate_limit.rs keys on the full IpAddr) and the global max_page_count (connection.rs:414, 64 GiB default), so the claimed growth-until-host-write-failure path holds. ISSUES.md and the book do not track it. The recommendation is feasible and wire-compatible: schema 49 is next after SCHEMA_VERSION 48, Unsupported is already returned for disabled routes, and RateLimited is in the route's declared statuses. Two details need care. Returning the existing ID for a repeated email gives anyone who knows an address its stored ID. Adding UNIQUE(email) needs a dedupe step in the migration.

</details>

### server-backup-snapshot-restart

**Online backup steps without a held read snapshot, so continuous writes can restart a large backup indefinitely**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: server
- Verification: adjusted

online_backup copies 128 pages per sqlite3_backup_step, sleeping 1 ms between steps, from a read-only connection that holds no transaction between steps. Each step therefore opens a new read transaction. When a step's read transaction sees that the WAL changed because the writer committed, SQLite's pager_reset restarts the backup from page 0. A multi-GB database needs thousands of steps, and the server commits continually: chat sends, realtime reconciliation, the 60 s maintenance pass. Under steady traffic a scheduled backup can loop without finishing. It records neither success nor failure until it completes, and its only exit is server shutdown. The existing concurrency test uses a small database that completes in a few steps.

**Evidence**

- [`crates/foks-server-db/src/connection.rs:321`](../../../crates/foks-server-db/src/connection.rs#L321): Backup::new(source, &mut target), then a loop of backup.step(128) with sleep(1 ms); the source holds no outer read transaction
- [`crates/foks-server/src/operations/backup.rs:111`](../../../crates/foks-server/src/operations/backup.rs#L111): the scheduler opens a fresh ReadDatabase and calls online_backup_until with shutdown as the only cancellation
- [`crates/foks-server-testkit/tests/backup_restore.rs:300`](../../../crates/foks-server-testkit/tests/backup_restore.rs#L300): online_backup_is_coherent_while_kv_writes_continue writes 12 small files to a near-empty database

**Recommendation**

1. **Hold one snapshot.** Before the step loop, open a read transaction on the source with ReadDatabase::snapshot() or unchecked_transaction, run a trivial SELECT to pin it, and hold it until StepResult::Done. The backup then copies one snapshot without restarts. WAL frames stay pinned for the duration, which PASSIVE maintenance checkpoints tolerate. Alternatively, run `VACUUM INTO` on the read connection outside any explicit transaction, since SQLite rejects VACUUM inside one. It reads a single snapshot and compacts. Pre-create the target as an empty 0600 file and benchmark both approaches.
2. **Visibility.** Add `foks_backup_steps_total` and a configurable wall-clock budget (default 1 h). Exceeding it fails the run with phase=backup_create, reason=busy, so last-success alerting fires with a reason.
3. **Test.** Seed at least 64 MiB, commit every 10 ms during the backup, and assert completion within a bound and a valid restore.

<details><summary>Verifier note</summary>

Core claim holds. online_backup (connection.rs:307-341) calls backup.step(128) with a 1 ms sleep, on a ReadDatabase connection that holds no transaction between steps. sqlite3_backup_step opens and closes its own read transaction when none is open. In WAL mode, pagerBeginReadTransaction calls pager_reset when the WAL changed, and pager_reset calls sqlite3BackupRestart, so each writer commit between steps restarts the copy. The scheduler (operations/backup.rs:108-125) opens a fresh ReadDatabase, and its only cancellation is the Drop-set flag. Backup metrics record success or failure only on completion. The testkit test writes 12 small files. The checkpoint bench backs up 32 MiB alongside a finite write workload, which also does not test steady-state commits. Maintenance uses checkpoint_passive (maintenance.rs:85), so pinning a snapshot is compatible. One correction: the 'VACUUM INTO on a ReadSnapshot' alternative is infeasible as written, because SQLite refuses VACUUM, including VACUUM INTO, inside an open transaction ('cannot VACUUM from within a transaction'). VACUUM INTO has to run on the read connection outside any transaction; it reads a single snapshot itself. ReadDatabase::snapshot() already exists, but it uses unchecked_transaction (BEGIN DEFERRED), so a SELECT is needed to pin the snapshot, as the recommendation says.

</details>

### server-config-surface

**server.toml cannot set any limit, ignores unknown keys, and gives misleading validation errors**

- Type: maintainability
- Priority: medium
- Effort: S
- Layers: server, cli, docs
- Verification: adjusted

`serve-config` is the documented production path and the one packaging/foks-server/foks-server.service runs. It hard-codes every SessionLimits, writer-queue and rate-limit value (worker_threads 4, 32 read connections, 30 s request timeout, 64 pending writes, the rate-limit buckets). `serve` exposes those values as CLI flags, but both paths pass `foks_server_db::Config::default()`. No binary can therefore set maximum_database_bytes, maximum_teams, maximum_team_members or KV namespace quotas without code changes, and the TOML path cannot tune any runtime limit. InstallationConfig lacks `#[serde(deny_unknown_fields)]`, although its nested oidc and web_admin tables have it. A misspelled key or an attempted `[limits]` table is silently ignored, and `config-check` reports the file valid. `validate()` returns 'installation paths must be absolute' for a wrong version, a zero backup interval or retention, or a non-loopback management address. ISSUES Stage 6 (realtime fanout vs. team size) needs a configuration home where that relationship can be validated, and none exists.

**Evidence**

- [`crates/foks-server/src/installation.rs:15`](../../../crates/foks-server/src/installation.rs#L15): InstallationConfig derives Deserialize without deny_unknown_fields and has no limits, rate-limit or storage sections
- [`crates/foks-server/src/installation.rs:89`](../../../crates/foks-server/src/installation.rs#L89): one 'installation paths must be absolute' error covers version, backup interval/retention, management loopback and path checks
- [`crates/foks-server/src/bin/foks-server.rs:538`](../../../crates/foks-server/src/bin/foks-server.rs#L538): serve_config hard-codes worker_threads 4, read connections, rate limits, timeouts and the writer queue
- [`crates/foks-server/src/bin/foks-server.rs:638`](../../../crates/foks-server/src/bin/foks-server.rs#L638): `database: foks_server_db::Config::default()` on every serve path
- [`crates/foks-server/src/web_admin/config.rs:5`](../../../crates/foks-server/src/web_admin/config.rs#L5): the nested config already uses #[serde(deny_unknown_fields)]
- [`packaging/foks-server/foks-server.service:10`](../../../packaging/foks-server/foks-server.service#L10): the systemd unit runs serve-config

**Recommendation**

1. **Strict parsing.** Add `#[serde(deny_unknown_fields)]` to InstallationConfig, keeping INSTALLATION_VERSION = 1, which is acceptable pre-v1.
2. **Optional tables.** Add `[limits]`, `[rate_limits]` and `[storage]` tables whose serde defaults equal today's constants, and map them onto SessionLimits, RateLimitConfig and foks_server_db::Config.
3. **Relationship checks.** Validate relationships centrally in one `EffectiveConfig::validate`:
   - maximum_team_members ≤ RealtimeLimits::FANOUT_MEMBERS (ISSUES Stage 6);
   - the file-descriptor budget vs RLIMIT_NOFILE;
   - maximum_pending_writes vs maximum_in_flight_requests;
   - storage reserve < maximum_database_bytes.
4. **Distinct errors.** Split validate() into one error per condition.
5. **Show effective values.** Add `config-check --print-effective` (TOML or JSON) so operators can confirm what will run.
6. **Test.** Add a TOML round-trip test that every field of every table is reachable.

**Already tracked:** ISSUES.md Stage 6 asks for configuration-relationship validation between team size and realtime fanout. This finding supplies the configuration surface needed to implement it.

<details><summary>Verifier note</summary>

The core claims hold:
- InstallationConfig (installation.rs:15) has no deny_unknown_fields, while WebAdminConfig, OidcOperatorConfig and the SSO payload do.
- validate() returns one 'installation paths must be absolute' error for a wrong version, a zero backup interval or retention, a non-loopback management address and relative paths (installation.rs:74-90).
- serve_config (foks-server.rs:520-551) hard-codes ttl, worker threads, read connections, request timeout, pending writes and rate limits.
- serve passes foks_server_db::Config::default() (line 638).
- packaging/foks-server/foks-server.service runs serve-config.
- ISSUES Stage 6 asks for the team-size vs. fanout relationship check, and RealtimeLimits::FANOUT_MEMBERS = 1024 exists.
One imprecision: the `serve` subcommand already exposes worker_threads, request_timeout_seconds, read and connection limits and rate limits as CLI flags. The gap for those values is serve-config only; database Config limits cannot be set from either binary. The recommendation is feasible: it is a server-local configuration surface with no wire impact.

</details>

### server-error-status-mapping

**About 13 divergent map_write_error functions: a full database is reported as retryable, and internal error text leaks to clients**

- Type: correctness-risk
- Priority: medium
- Effort: M
- Layers: server, docs
- Verification: adjusted

Status mapping is hand-written in about 20 places and diverges. Only kv.rs (3 sites) and log_upload.rs (1 site) call foks_server_db::Error::is_quota(), so SQLITE_FULL from the max_page_count ceiling becomes TransactionRetry in registration, team loader, federation, generic, invitations, user, waitlist and realtime. On the same failure, identity proof writes return PermissionDenied, SSO binding returns a generic OAuth2 failure, and team create/edit return TeamError with the formatted crate::Error, which carries SQLite text. The agent and desktop already present 1060 as a non-retryable 'capacity limit' error, so users on a full host get a misleading generic failure instead. The book promises a clean quota error. Realtime's write_error also maps the writer-level AuthorizationChanged/Sso guards and WriterQueue to TransactionRetry, unlike the KV and team-admin mappers.

**Evidence**

- [`crates/foks-server-db/src/error.rs:78`](../../../crates/foks-server-db/src/error.rs#L78): is_quota() treats QuotaExceeded and SQLITE_FULL as quota errors
- [`crates/foks-server/src/services/kv.rs:1213`](../../../crates/foks-server/src/services/kv.rs#L1213): KV mappers use is_quota() at 1213, 1231 and 1246; log_upload.rs:101 is the fourth and last call site
- [`crates/foks-server/src/services/registration.rs:559`](../../../crates/foks-server/src/services/registration.rs#L559): map_write_error matches only QuotaExceeded; SQLITE_FULL falls through to TransactionRetry (same pattern in team_loader.rs:694, federation.rs:505, generic.rs:425, team_invitations.rs:27, waitlist.rs:33)
- [`crates/foks-server/src/services/team_admin.rs:1281`](../../../crates/foks-server/src/services/team_admin.rs#L1281): `other => RpcStatus::TeamError(format!("team edit validation failed: {other}"))`; also line 1314 for create, so Busy and SQLITE_FULL become a non-retryable TeamError that carries SQLite text
- [`crates/foks-server/src/net/session/handlers/identity.rs:58`](../../../crates/foks-server/src/net/session/handlers/identity.rs#L58): every error except Capacity and WriterQueue, including disk-full, maps to PermissionDenied("identity proof could not be verified")
- [`crates/foks-server/src/sso/binding.rs:271`](../../../crates/foks-server/src/sso/binding.rs#L271): unmatched storage errors map to OAuth2("authentication could not be verified")
- [`crates/foks-server/src/services/realtime.rs:266`](../../../crates/foks-server/src/services/realtime.rs#L266): write_error forwards only Database variants; crate::Error::AuthorizationChanged, Sso and WriterQueue become TransactionRetry, and db_error has no DiskFull case (line 291)
- [`crates/foks-server/src/services/log_upload.rs:88`](../../../crates/foks-server/src/services/log_upload.rs#L88): some Capacity values are transient concurrency caps that are intentionally RateLimited, so Capacity cannot be mapped to QuotaExceeded wholesale
- [`crates/foks-agent/src/main.rs:2285`](../../../crates/foks-agent/src/main.rs#L2285): the agent maps 1060 to non-retryable QuotaExceeded with an operator-capacity message and keeps the server reason text
- [`book/18-server-storage.qmd:53`](../../../book/18-server-storage.qmd#L53): the book says a full disk produces a clean quota error

**Recommendation**

1. **One classifier.** Add `crate::status::classify(&Error) -> Class` and reuse the SQLite-code logic already in diagnostics/operational.rs::reason(). Map the shared classes first:
   - is_quota → QuotaExceeded;
   - WriterQueue or ReaderPool → RateLimited;
   - writer-level AuthorizationChanged → PermissionDenied;
   - Sso → OAuth2Auth (via mutation_failure_status);
   - EpochLimitExceeded → MerkleVerify;
   - DatabaseCorrupt or NotADatabase → a fixed non-retryable status;
   - Busy or Io → TransactionRetry.
   Let each family map only its domain variants.
2. **Split Capacity.** Do not fold Capacity into QuotaExceeded. Either split it into `Capacity` (retained storage → QuotaExceeded) and `Concurrency` (active sessions, admin writer, per-request realtime limits → RateLimited), or keep the existing per-family handling.
3. **No error text on the wire.** Replace `format!("{other}")` in team_admin with fixed strings. Add a check in tools/foks-server/check.sh against formatting crate::Error into RpcStatus payloads. mutation_failure_status's MerkleVerify detail is a deliberate, tested exception.
4. **Contract test.** Inject SQLITE_FULL, WriterQueue, writer AuthorizationChanged and Sso into every family's mapper and assert the status.
5. **End-to-end test.** In a testkit case with a tiny maximum_database_bytes, assert that signup, team edit, chat send, identity proof and KV write all return QuotaExceeded.

**Already tracked:** book/18-server-storage.qmd says a full disk produces 'a clean quota error'. The README lists disk-full fault injection as release-hardening work but does not note the inconsistent mapping.

<details><summary>Verifier note</summary>

Core claim holds: SQLITE_FULL from max_page_count is recognized only where is_quota() is called, so full-database writes in registration, team_loader, federation, generic, team_invitations, user, waitlist and realtime return TransactionRetry, and team create/edit format the full error into TeamError (team_admin.rs:1281, 1314). Several details are wrong. (1) is_quota() has 4 call sites, 3 in kv.rs (1213, 1231, 1246) and 1 in log_upload.rs:101, so KV is not the only user. (2) The Capacity claim is false. Capacity is produced only by log upload, realtime, SSO, web admin and identity paths, and each maps it to RateLimited, Realtime or BadArguments, never TransactionRetry. Some Capacity values are transient concurrency caps that are deliberately RateLimited: 'active log-send sessions' and 'log-send bytes' (log_upload.rs:88) and 'admin writer' (web_admin/service.rs:99). The recommended blanket 'Capacity → QuotaExceeded' would therefore turn transient refusals into permanent ones. (3) There is more divergence than the finding reports. identity.rs:58 maps every non-capacity error, disk-full included, to PermissionDenied('identity proof could not be verified'). sso/binding.rs:271 maps them to OAuth2('authentication could not be verified'). realtime.rs:266 write_error handles only Database variants, so the writer-level AuthorizationChanged and Sso guards, and WriterQueue, all become TransactionRetry. In total there are about 20 hand-written mappers, not 13. (4) 'Clients retry indefinitely' is overstated. The agent maps 1060 to a non-retryable quota-exceeded error with operator guidance (foks-agent main.rs:2285; desktop transport.rs:115), so the real harm is that users see a generic or retryable failure instead of the quota message. (5) diagnostics.rs:7 governs operator diagnostics sinks, not RPC statuses. The wire leak is still real: the agent preserves the server's reason text, so SQLite text can reach clients. The book's 'clean quota error' claim (18-server-storage.qmd:53) is confirmed. Priority is lowered to medium: this is mislabeled status on an operational edge case, and the authorization races are narrow because authorize_principal already checks before the writer runs.

</details>

### server-merkle-incremental-prepare

**Full Merkle rebuild on the single writer thread grows to seconds per identity/team mutation**

- Type: performance
- Priority: medium
- Effort: L
- Layers: server, tooling
- Verification: adjusted

For every Merkle-publishing mutation, foks_merkle_store::prepare runs inside the single writer closure. It works in three passes:
- collect_leaves walks, decodes and re-hashes every node of the current tree.
- build() reconstructs the whole tree.
- get_node is called for every rebuilt node.

A scratch release benchmark against the in-memory MemoryStore (no SQLite I/O) measured a single-leaf prepare at about 9 ms for 1k leaves, 103 ms for 10k, 655 ms for 50k and 1.37 s for 100k. Each mutation adds one to three leaves, so the 65,535-epoch ceiling bounds the tree at very roughly 65k-200k leaves. On a mature host, the worst case is therefore a 1-3 s writer stall per Merkle mutation plus SQLite node reads. The writer is FIFO, so every chat send, KV write and revocation queued behind such a mutation waits for it. A burst of Merkle mutations on a large tree can exceed the 30 s request timeout and fill the 64-deep queue, which produces timeouts and ambiguous outcomes. Team edits also load and decode every persisted team link (up to 4,096) inside the writer.

**Evidence**

- [`crates/foks-merkle-store/src/tree.rs:12`](../../../crates/foks-merkle-store/src/tree.rs#L12): prepare() calls collect_leaves over the entire current tree, then build() over all entries, then reader.get_node for every rebuilt node (lines 39-49)
- [`crates/foks-server/src/merkle.rs:37`](../../../crates/foks-server/src/merkle.rs#L37): prepare_publication calls foks_merkle_store::prepare with the writer's node_reader, inside WriterHandle::call
- [`crates/foks-server/src/services/team_admin.rs:683`](../../../crates/foks-server/src/services/team_admin.rs#L683): team edit runs database.team() (all links and members), identity::team_edit::validate (decodes every persisted link, verify_team_transition) and prepare_publication inside writer.call
- [`crates/foks-server-db/src/read.rs:706`](../../../crates/foks-server-db/src/read.rs#L706): team_snapshot_inner loads every team_chain_links row, including exact_link blobs, for each edit
- [`crates/foks-merkle-store/README.md:4`](../../../crates/foks-merkle-store/README.md#L4): documents 'updates are O(n)' as an intentional v1 trade-off

**Recommendation**

1. **Incremental prepare.** Add prepare_incremental(reader, root, changes) to foks-merkle-store. It descends from the root along each changed key, splits or replaces only the affected compressed edges, and rehashes the touched path. Keep prepare as the oracle. Add a differential property test requiring byte-identical roots and node sets for random leaf sets and change batches. Keep the Go fixture replays.
2. **Move full-tree validation off the write path.** Run the cycle, hash and prefix checks done by collect_leaves as a startup, maintenance or backup verify_tree job. The write path then verifies only the nodes it reads.
3. **Shrink the team-edit critical section.** Run identity::team_edit::validate on a read snapshot before enqueueing.
   - Inside the writer, re-check expected_sequence and expected_tail_hash (commit_team_mutation already takes both), the cited root, member key currency, and the bearer and local-member-key checks that depend on non-chain state.
   - Store persisted_team_index_range as a projection so the writer does not decode every link.
4. **Benchmarks.** Add 10k, 50k and 100k-leaf prepare cases to BENCHMARKS.md with a regression gate.

**Already tracked:** crates/foks-merkle-store/README.md and book/11-merkle-tree.qmd ('Insertion and rebuilding') state O(n) updates as an intentional v1 trade-off. Not tracked: measured latency, the fact that it runs on the server's only writer thread, and the incremental design with a differential-test oracle.

<details><summary>Verifier note</summary>

The core claim holds. tree.rs:5-55 calls collect_leaves over the whole tree. That walk decodes and re-hashes every node and merges per-subtree BTreeMaps upward. build() then rebuilds every entry and calls get_node for each rebuilt node. merkle.rs:37 runs this inside the writer closure. team_admin.rs:690-770 loads database.team(), and identity/team_edit.rs:91-96 decodes every persisted link for persisted_team_index_range inside writer.call. read.rs:707 selects all team_chain_links with exact_link. I reproduced the reviewer's benchmark in a scratch crate (release build, MemoryStore, 4 vCPU): 9.2 ms at 1k leaves, 103 ms at 10k, 655 ms at 50k and 1,370 ms at 100k. The figures match. The priority and the throughput framing are overstated. Each mutation adds 1-3 leaves, so the 65,535-epoch ceiling bounds the tree at roughly 65k-200k leaves. The worst case is therefore a bounded writer stall of about 1-3 s per Merkle mutation, plus SQLite node reads, not unbounded growth. A compatible host can perform at most 65,535 Merkle mutations in its lifetime, so sustained Merkle write rates are necessarily low. The practical harm is latency spikes for chat and KV writes queued FIFO behind each mutation, not a host-wide collapse to a few writes per second. README.md and book/11 already state O(n) as an intentional trade-off. The measured latency and the single-writer placement are new information. The incremental design is feasible because the compressed-trie shape depends only on the leaf set, so the existing prepare can act as a differential oracle.

</details>

### server-metrics-scrape-cost

**Every /metrics scrape and every new pooled reader full-scans the KV tables**

- Type: performance
- Priority: medium
- Effort: S
- Layers: server
- Verification: adjusted

ReadDatabase::open runs validate_kv_tree_capacity. When the data is valid, this visits every row of kv_directory_heads and kv_dirent_heads (GROUP BY uid HAVING count(*)) and every row of kv_nodes and kv_dirents (an unindexed length() predicate). The management listener's prometheus() opens a fresh ReadDatabase on every /metrics scrape to sample SSO gauges. It does so even when OIDC is not configured, because the open comes before the policy lookup, and when a policy exists it then runs a cohort count. prometheus() is a synchronous call inside a task on the management thread's current-thread runtime, so concurrent /healthz and /readyz requests are not polled until the KV-table scan finishes. Sample errors are silently discarded (`if let Ok(Some(s))`) with no failure counter. The read pool opens connections lazily on the request path, so each additional pooled connection, at most 31 (29 with web admin), pays the scan once when it is first opened; connections are retained afterwards. Passing Default::default() instead of the running Config is inconsistent but does not cause false failures, because Default's KV limits equal the maximum the writer accepts.

**Evidence**

- [`crates/foks-server-db/src/connection.rs:223`](../../../crates/foks-server-db/src/connection.rs#L223): ReadDatabase::open calls crate::kv::validate_kv_tree_capacity after the schema check
- [`crates/foks-server-db/src/kv.rs:1432`](../../../crates/foks-server-db/src/kv.rs#L1432): validate_kv_tree_capacity: EXISTS(GROUP BY uid HAVING count(*) > ?) over kv_directory_heads and kv_dirent_heads, plus unindexed length() scans of kv_nodes.exact_node and kv_dirents.exact_dirent
- [`crates/foks-server/src/operations/management.rs:437`](../../../crates/foks-server/src/operations/management.rs#L437): prometheus() opens ReadDatabase::open(database_path, Default::default()) on every scrape, before checking for an SSO policy; errors are dropped by `if let Ok(Some(s))`
- [`crates/foks-server/src/operations/management.rs:49`](../../../crates/foks-server/src/operations/management.rs#L49): the management server uses new_current_thread; handle() calls prometheus() inline (around line 205), blocking the only runtime thread
- [`crates/foks-server/src/read_pool.rs:61`](../../../crates/foks-server/src/read_pool.rs#L61): checkout() opens a new validating ReadDatabase when no idle connection exists; returned connections are retained in `idle`, so this happens once per connection
- [`crates/foks-server-db/src/config.rs:81`](../../../crates/foks-server-db/src/config.rs#L81): Default KV limits equal the foks_proto maxima, and configure() rejects anything larger, so a Default-limit check cannot fail where the writer's check passed

**Recommendation**

1. **Split the reader constructor.** Add a ReadDatabase constructor that performs only the identity and schema-version checks, such as `open_pooled`. Use it for the ReadPool, the web-admin readers and the metrics sample. Keep the validating ReadDatabase::open for offline tools and for the backup and restore verification paths. The writer's Database::open stays the startup authority for 'configuration tighter than data fails closed'.
2. **Cheap metrics.** Hold one long-lived read connection in ManagementState, opened at start. Run the SSO sample in spawn_blocking with a 1 s timeout, and cache the gauges for about 30 s. Skip the sample entirely when no OIDC configuration is present. Count sample failures, for example in foks_oidc_sample_failures_total, instead of dropping them.
3. **Consistency.** Pass the running database Config rather than Default. This is a consistency fix, not a correctness fix.
4. **No pool warming.** Once pooled opens no longer scan, pre-opening connections is unnecessary.

<details><summary>Verifier note</summary>

Most of the finding holds. connection.rs:223 calls validate_kv_tree_capacity from ReadDatabase::open. When nothing is violated, the query in kv.rs:1432 has to visit every row of kv_directory_heads and kv_dirent_heads, and every row of kv_nodes and kv_dirents, because the length() predicate has no index. prometheus() opens a new ReadDatabase on each scrape (management.rs:437). It does this before looking up the policy, so it runs even when OIDC is not configured. prometheus() is called synchronously inside handle(), on a new_current_thread runtime (management.rs:49), so /healthz and /readyz cannot make progress until the scan finishes. Three statements are wrong. (1) The scan covers the four KV-tree tables, not the whole database. (2) Using Default::default() does not make the gauges disappear. configure() rejects KV limits above the foks_proto maxima, and Config::default() sets the KV limits to exactly those maxima (config.rs:81-84). A check against Default limits therefore cannot fail where the writer's check passed. The gauges disappear only on other errors, which `if let Ok(Some(s))` discards without counting. (3) The pool keeps every connection it opens and never closes one. The scan is a one-time cost for each of at most 31 extra connections (29 with web admin, standalone.rs:884), not a cost after each burst, so pool warming is unnecessary once pooled opens skip the scan. The 'validate only in the writer' recommendation also needs care: the backup and restore paths (standalone.rs:204, 428, 542) rely on the validating ReadDatabase::open.

</details>

### server-operator-capacity-console

**Operators have no capacity, headroom or abuse controls (usage report, account suspension, waitlist)**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: server, cli, docs
- Verification: adjusted

The operator surface covers bootstrap, keys, invites, OIDC rollout, browser sessions and audit. The web-admin overview shows a hard-coded 'Service ready' line plus four counts: accounts, operators, invites and sessions. /metrics exposes database and WAL bytes, the last backup success time and a maintenance-warning gauge. No surface reports used vs. limit for the permanent caps: Merkle epochs (back-pointer overflow at epoch 65,536), teams (4,096), invitation certificates and remote requests (65,536), local requests (1,000,000), and maximum_database_bytes. Nothing reports per-team KV or chat consumption, and backup state is not shown to operators in the CLI or web admin. Operators cannot suspend an abusive account (for example a chat flooder) except through SSO enforcement, and cannot read, export or purge the waitlist, which has only a join path. ISSUES Stage 1 ('usage metrics, capacity diagnostics') and Stage 3 ('operator observability') require diagnostics but name no surface for them.

**Evidence**

- [`crates/foks-server/src/web_admin/render.rs:48`](../../../crates/foks-server/src/web_admin/render.rs#L48): the overview hard-codes 'Service ready · Software ... · Database schema ...'; line 51 adds only account, operator, invite and session counts
- [`crates/foks-server/src/web_admin/http/routes.rs:106`](../../../crates/foks-server/src/web_admin/http/routes.rs#L106): the admin GET routes are /admin, /admin/sessions, /admin/invites and /admin/audit; POST adds signup-policy, invite and session actions only
- [`crates/foks-server/src/bin/foks-server.rs:21`](../../../crates/foks-server/src/bin/foks-server.rs#L21): the subcommands cover init, serve, status, backup/restore, key rotation, invite, oidc and admin grants; there are no usage, account or waitlist commands
- [`crates/foks-server/src/installation.rs:217`](../../../crates/foks-server/src/installation.rs#L217): status() reports only initialized, host ID, bytes and integrity
- [`crates/foks-server-db/src/waitlist.rs:6`](../../../crates/foks-server-db/src/waitlist.rs#L6): join_waitlist is the only waitlist API; no list, export or purge exists
- [`crates/foks-server/src/operations/management.rs:311`](../../../crates/foks-server/src/operations/management.rs#L311): /metrics already exports database/WAL bytes, last backup success and the maintenance warning, but no limits or headroom

**Recommendation**

1. **Read-only usage report.** Add `ReadSnapshot::capacity_report()`, computed from one snapshot. It returns:
   - the Merkle epoch vs. the back-pointer overflow at 65,536;
   - teams and team-name reservations vs. maximum_teams and maximum_team_name_reservations;
   - invitation certificates, local and remote request rows vs. their hard caps;
   - the top 10 KV namespaces by bytes (IDs only);
   - rt_messages bytes and rows per team (top 10);
   - waitlist rows;
   - database and WAL bytes vs. maximum_database_bytes;
   - the last backup and maintenance results, taken from ServerMetrics.
   Expose it as `foks-server usage [--json]`, which is safe online because it is read-only, and as a web-admin /admin/capacity page. Also export the limits as Prometheus gauges so alerting can compute headroom.
2. **Account suspension.** Add `foks-server account suspend|resume` and a web-admin action. Both write a `suspended_users(uid, reason_code, created_at)` row and an audit event. Enforce suspension in ServerData::authorize_principal and in the writer's in-queue authorization re-check, next to the existing sso_require_access check, returning PermissionDenied. This is a server-local decision with no wire change, and queued writes are refused at their next writer job.
3. **Waitlist page.** Add /admin/waitlist with a count, a paginated list, CSV export and purge-before-date.
4. **Mockup scope.** Label any revocation epoch reserve, storage reserve or per-account epoch attribution as a proposal that depends on other findings, not as current behaviour.

**Already tracked:** Each of ISSUES.md Stages 1, 3 and 5 lists 'operator observability' or 'usage metrics, capacity diagnostics' as a requirement but specifies no surface. The crates/foks-server README lists audit export as release-hardening work.

<details><summary>Verifier note</summary>

The core holds:
- The web-admin routes are only /admin, /admin/sessions, /admin/invites, /admin/audit and /admin/signup-policy (routes.rs:106-253).
- The CLI has no usage, account or waitlist commands.
- No suspension mechanism exists anywhere; 'suspend' appears only in SuspendClock.
- The waitlist has a write path only (join_waitlist).
- InstallationStatus holds only initialized, host ID, bytes and integrity.
- The cited caps exist: back-pointer arrays overflow at epoch 65,536; maximum_teams is 4096; invitation globals are 65,536 / 1,000,000.

Several statements are overstated:
- The overview is not purely static. It also renders WebOverview counts: accounts, operators, invites and live sessions (render.rs:51-53).
- /metrics already exposes foks_last_backup_success_unixtime, database and WAL bytes, and a maintenance-warning gauge. Backup freshness is therefore observable, though not in the operator UI or CLI, and no limits are reported.
- 'Each ISSUES stage ends with operator observability' is false, and so is already_tracked's 'Stages 1, 3 and 5'. Only Stage 1 ('usage metrics, capacity diagnostics') and Stage 3 ('operator observability') say this; Stage 5 does not.

The mockup brief presents mechanisms that do not exist as current behaviour: a 2,048-epoch revocation reserve, 'bulk writes stop at 62.7 GiB', 'only revocation/removal/recovery proceed', and per-account Merkle-epoch attribution. The mockup should label these as proposed, depending on other findings, or drop them. For suspension, the natural hook is ServerData::authorize_principal (session.rs:1858) together with the writer's in-queue re-check (writer.rs:258-275), which already re-checks credentials and SSO access. The recommendation is feasible: it is server-local, has no wire change, and the counts are metadata only.

</details>

### server-request-failure-observability

**Request failures are invisible: no per-route/status metrics, no internal-error reasons, no latency histograms**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: server, docs
- Verification: adjusted

There are 300 `.map_err(|_| RpcStatus::TransactionRetry)` sites, plus the `internal(_)` helpers in four services, and they all discard the underlying error. Nothing logs or counts request-level outcomes by status; diagnostics are only connection-level session failures and backup/maintenance phases. Request, handler, writer queue-wait and writer execution durations are exported only as count, sum and a since-start max gauge, so tail latency is unavailable. Only realtime reconciliation (Poll/Delta) attributes writer time and records histograms, through WriterHandle::call_observed. Merkle publication, KV writes, chunk uploads and chat sends are unattributed. Reader-pool exhaustion is counted as rate_limited_requests.

**Evidence**

- [`crates/foks-server/src/services/team_loader.rs:694`](../../../crates/foks-server/src/services/team_loader.rs#L694): map_write_error falls back to TransactionRetry. team_loader.rs has 61 of the 300 `map_err(|_| RpcStatus::TransactionRetry)` sites (user.rs 58, kv.rs 48, net/session.rs 36)
- [`crates/foks-server/src/services/team_admin.rs:1334`](../../../crates/foks-server/src/services/team_admin.rs#L1334): `fn internal(_: impl Display) -> RpcStatus { TransactionRetry }`; the same helper is in generic.rs:455, federation.rs:525 and team_invitations.rs:21
- [`crates/foks-server/src/metrics.rs:16`](../../../crates/foks-server/src/metrics.rs#L16): DurationMetric keeps observations, a total and fetch_max, with no buckets
- [`crates/foks-server/src/writer.rs:40`](../../../crates/foks-server/src/writer.rs#L40): Task::run observes queue wait and execution globally; only the optional `reconciliation` field adds per-source attribution
- [`crates/foks-server/src/metrics/realtime.rs:157`](../../../crates/foks-server/src/metrics/realtime.rs#L157): existing per-source writer queue/execution histograms on BUCKET_MICROS for realtime reconciliation, which can serve as the template
- [`crates/foks-server/src/net/session.rs:838`](../../../crates/foks-server/src/net/session.rs#L838): ReaderPool exhaustion increments request_rate_limited
- [`crates/foks-server/src/diagnostics/operational.rs:69`](../../../crates/foks-server/src/diagnostics/operational.rs#L69): reason() classifier (disk_full, corrupt_storage, busy, ...) is used only for backup and maintenance phases

**Recommendation**

1. **Outcome counter.** Thread the RpcStatus wire code out of handlers::response into RequestOutcome and export `foks_requests_total{family,status}`, with family taken from RouteSpec.
2. **Internal-error funnel.** Add `fn internal(e: impl Into<crate::Error>, site: &'static str) -> RpcStatus`. It applies operational::reason(), increments `foks_request_internal_errors_total{reason}`, and writes a rate-limited stderr line with no error text. Replace the `|_|` closures and the four `internal(_)` helpers with it.
3. **Histograms.** Replace DurationMetric with BUCKET_MICROS histograms for request, handler, writer_queue_wait and writer_execution.
4. **Writer attribution.** Generalize WriterHandle::call_observed: replace the realtime-only ReconcileSource with a `WriterOp` enum (merkle_publish, kv_write, kv_chunk, rt_send, rt_reconcile_poll, rt_reconcile_delta, maintenance, readiness, ...). Label the writer histograms by it so the existing realtime reconcile metrics become one case of the general mechanism.
5. **Pool exhaustion.** Add `foks_reader_pool_exhausted_total` and a `foks_reader_pool_open` gauge, separate from token-bucket rejections.

**Already tracked:** The README lists 'structured audit export' as release-hardening work. Per-route status metrics and internal-error reasons are not tracked.

<details><summary>Verifier note</summary>

Core claim holds. There are exactly 300 `map_err(|_| RpcStatus::TransactionRetry)` sites: 61 in team_loader.rs, 58 in user.rs, 48 in kv.rs and 36 in net/session.rs. The `internal(_)` helpers in team_admin.rs:1334, generic.rs:455, federation.rs:525 and team_invitations.rs:21 also discard errors. No per-route or per-status counter exists: RequestOutcome (session.rs:1664) holds only response bytes and the route, and management.rs exports only started and completed totals. Request, handler, writer queue-wait and writer execution durations are exported as summary count/sum plus a fetch_max gauge (management.rs:301-326). ReaderPool exhaustion calls request_rate_limited (session.rs:840). operational::reason() is used only by backup and maintenance phases. One correction: 'writer time cannot be attributed to operations' is partly wrong. Realtime reconciliation writer jobs already go through WriterHandle::call_observed with a ReconcileSource. They record queue-wait and execution histograms on BUCKET_MICROS (metrics/realtime.rs:157) and are exported (management.rs:773, 835). The recommendation should generalize this mechanism rather than add a parallel one. Also, the cited team_loader.rs:694 is a map_write_error definition, not a `|_|` site.

</details>

### server-session-decomposition

**net/session.rs mixes transport and domain logic, and request authorization relies on cloning ServerData by convention**

- Type: maintainability
- Priority: medium
- Effort: M
- Layers: server
- Verification: adjusted

net/session.rs is 2,127 lines, of which about 245 are tests. Lines 106 to about 1083 are `impl ServerData` domain logic, followed by about 180 lines of free Merkle and validation helpers. This logic covers user-mutation commit, username reservation, signup, client certificate issuance and the Merkle query handlers. services/registration.rs and services/user.rs already exist for this kind of logic. The transport loop has three hand-written long-poll branches (SSO poll, KexReceive, RtPollInbox), each repeating the readable/stop select and response encoding. They have already diverged: the SSO branch has no fault-injection check and no handler timer, while the KEX and realtime branches have both. Per-request state is created by cloning the whole ServerData and then overwriting `peer_ip`, and `writer` with an authorized writer. Re-authorization of writes in the queue therefore depends on each path remembering this convention, and the realtime poll path builds its own authorized writer separately. Both paths currently do so; nothing in the types enforces it.

**Evidence**

- [`crates/foks-server/src/net/session.rs:549`](../../../crates/foks-server/src/net/session.rs#L549): signup() (about 190 lines) lives in the transport module
- [`crates/foks-server/src/net/session.rs:190`](../../../crates/foks-server/src/net/session.rs#L190): commit_user_mutation_argument (about 300 lines), the full user-mutation writer closure, lives in the transport module
- [`crates/foks-server/src/net/session.rs:1565`](../../../crates/foks-server/src/net/session.rs#L1565): `let mut request_data = data.as_ref().clone();`, then peer_ip and writer are overwritten per request
- [`crates/foks-server/src/net/session.rs:1388`](../../../crates/foks-server/src/net/session.rs#L1388): the SSO poll branch has no should_disconnect or handler_timer; the KEX (1413) and realtime (1463) branches each repeat the fault check, select and encode logic
- [`crates/foks-server/src/net/session/handlers/realtime.rs:110`](../../../crates/foks-server/src/net/session/handlers/realtime.rs#L110): the realtime poll separately calls for_authenticated_request on data.writer after cloning ServerData

**Recommendation**

1. **Move domain logic.** Move signup, reserve_username and client_certificate_chain into services/signup.rs, the user-mutation commit into services/user_mutation.rs, and the Merkle queries into services/merkle_query.rs, as free functions over narrow ports, matching the handlers/* family pattern.
2. **Typed request context.** Introduce `RequestContext<'a> { server: &'a ServerData, peer_ip, principal: Option<Principal> }`. Add an `AuthorizedWriter` newtype that can only be built from `&Principal` via `WriterHandle::for_authenticated_request`. Mutation services that act for a user take `&AuthorizedWriter`, so a missing re-check fails to compile. Keep the unauthenticated `WriterHandle` for public routes and maintenance only.
3. **Long-poll helper.** Add `async fn run_long_poll<F>(stream, stop, fault_point, fut) -> LongPollOutcome` and use it for all three branches.
4. **Further split.** Split serve() into read_frame, admit, dispatch and write_response, so that session.rs is under about 600 lines of transport. Make team_admin.rs's 1,591 lines a later step, separating create, edit and the bearer-token helpers.

<details><summary>Verifier note</summary>

Confirmed:
- net/session.rs is 2,127 lines, of which about 245 are tests (1881 onwards).
- impl ServerData spans lines 106 to about 1083, with free Merkle and validation helpers to 1265: user_mutation at 172, commit_user_mutation_argument at 190 (about 300 lines), reserve_username at 494, signup at 549 (about 190 lines), client_certificate_chain at 741, and the Merkle query handlers at 912-1023.
- services/registration.rs (576 lines) and services/user.rs (712 lines) already hold free-function domain logic.
- Line 1565 clones all of ServerData (Strings, Vecs and about 15 Arcs) per request and overwrites peer_ip and writer.
- handlers/realtime.rs:110-114 builds its own authorized writer after another data.clone().
- services/team_admin.rs is 1,591 lines.

One detail is wrong: the three long-poll branches are not identical duplicates. The SSO poll branch (1388) has no should_disconnect fault check and no handler_timer, while the KEX (1413) and realtime (1463) branches have both. This divergence supports the helper recommendation. Today both authorizing paths do wrap the writer, so the authorization concern is a maintainability risk rather than a current bypass. The recommendation is feasible and internal only.

</details>

### server-writer-deadlines

**Writer queue runs abandoned jobs and has no priority lanes or per-principal fairness**

- Type: performance
- Priority: medium
- Effort: M
- Layers: server, docs
- Verification: confirmed

A request that times out after 30 s closes its connection, but its queued writer job still runs later. Under overload the single writer spends its time on work no client is waiting for, while new requests time out behind it, a congestion-collapse pattern made worse by the O(n) Merkle rebuild. The queue is a strict FIFO of 64 jobs. One principal can fill it, and the per-IP default of 1,000 requests/second is far above single-writer throughput. Security-critical writes (device revocation, team removal) and maintenance wait behind bulk KV chunk uploads and chat sends.

**Evidence**

- [`crates/foks-server/src/writer.rs:291`](../../../crates/foks-server/src/writer.rs#L291): try_send into a single sync_channel FIFO; Call carries queued_at but no deadline
- [`crates/foks-server/src/writer.rs:306`](../../../crates/foks-server/src/writer.rs#L306): the caller blocks on receiver.recv() with no timeout; the job runs whether or not the caller is still connected
- [`crates/foks-server/src/net/session.rs:1614`](../../../crates/foks-server/src/net/session.rs#L1614): on BoundedExecution::TimedOut the connection is closed, while the closure and its queued writer job continue
- [`crates/foks-server/src/rate_limit.rs:18`](../../../crates/foks-server/src/rate_limit.rs#L18): request defaults of a 2,000 burst at 1,000/s per IP, with no cost weighting for writer-bound routes

**Recommendation**

1. **Deadlines.** Add `deadline: Instant` (queued_at plus the remaining request budget) to Call. In Task::run, return `Error::DeadlineExceeded` without opening a transaction when the deadline has passed. This is safe because the job never started, and the client has already lost the response and treats the outcome as ambiguous. Idempotency keys make the client's retry execute normally. Count skipped jobs in `foks_writer_expired_total`.
2. **Two lanes.** Use a control lane (revocation, team removal, recovery, maintenance, readiness, SSO enforcement) and a bulk lane (KV, chat, waitlist, log upload). Use strict priority with a starvation guard, for example take a bulk job at least every 4 control jobs. Two sync_channels drained with a fixed preference are enough.
3. **Per-principal cap.** Cap pending writer jobs per authenticated UID (for example 4) in the WriterHandle produced by `for_authenticated_request`; over the cap returns RateLimited.
4. **Cost-weighted tokens.** Charge writer-bound routes more tokens than reads, using a cost column in policy-v1.toml.
5. **Test.** Extend testkit/concurrency.rs with a slow-writer stub showing that expired jobs are skipped and that a revocation overtakes 60 queued KV chunks.

<details><summary>Verifier note</summary>

All claims check out. Call (writer.rs:16) carries queued_at but no deadline. call_observed uses try_send into a single sync_channel of maximum_pending_writes, which defaults to 64 (bin/foks-server.rs:544), then blocks on receiver.recv() with no timeout (writer.rs:306). On BoundedExecution::TimedOut (session.rs:1614) the connection closes, but the spawn_blocking closure keeps running. It also keeps its execution-semaphore permit, which execute_bounded moves into the closure (session.rs:1688), so abandoned requests consume request concurrency as well as writer time. The README (line 152) acknowledges 'work that outlives a client deadline' only as a metrics note. request_timeout is 30 s (config.rs:67), and the rate-limit defaults are a 2,000 burst at 1,000/s per IP (rate_limit.rs:18-22) with no cost weighting. policy-v1.toml exists, and the Merkle store README documents O(n) updates. Neither ISSUES.md nor the README tracks lanes or deadlines. The recommendation is feasible. Skipping a job that never started is safe whether or not the route has an idempotency key, because the client has already lost the response and must treat the outcome as ambiguous. Lane reordering is benign because queued writes re-check active_credential_owner inside the writer, so a revocation that overtakes them makes them fail with AuthorizationChanged.

</details>

### server-migration-framework

**Schema upgrades are one ad-hoc branch block tested only against fixtures built by reversing the current schema**

- Type: testing
- Priority: low
- Effort: M
- Layers: server, docs, tooling
- Verification: confirmed

schema::initialize handles 43..=47 in one transaction with version-conditional and probe-conditional statements, plus unconditional v48 SSO deletes. The next migration (schema 49 is needed by several findings here) means editing that block again. The upgrade test does not use frozen historical schemas. It builds each predecessor by applying inverse DDL to the current schema, so any schema change that predecessor() does not invert passes unnoticed. It compares only the expiry index SQL afterwards, not full schema equivalence with a fresh install. The README links SCHEMA_MIGRATION_POLICY.md, KEY_ROTATION.md and OIDC.md, none of which exist; book/docs/research/research-server.md notes this, but the links remain.

**Evidence**

- [`crates/foks-server-db/src/schema.rs:44`](../../../crates/foks-server-db/src/schema.rs#L44): a single `matches!(version, 43..=47)` block with if-version and pragma_table_info probes
- [`crates/foks-server-db/tests/schema.rs:132`](../../../crates/foks-server-db/tests/schema.rs#L132): predecessor() synthesizes older schemas by dropping and renaming objects in the current schema
- [`crates/foks-server-db/tests/schema.rs:175`](../../../crates/foks-server-db/tests/schema.rs#L175): the upgrade test asserts only EXPIRY_INDEXES SQL and one data row after the upgrade
- [`crates/foks-server/README.md:333`](../../../crates/foks-server/README.md#L333): links SCHEMA_MIGRATION_POLICY.md (also KEY_ROTATION.md at 94 and OIDC.md at 504), all absent from the tree

**Recommendation**

1. **Ordered steps.** Restructure the upgrade as `const MIGRATIONS: &[Migration { from: i64, apply: fn(&Transaction) -> Result<()> }]`, applied in order inside the existing single IMMEDIATE transaction, keeping the re-check under the lock.
2. **Frozen snapshots.** At each version bump, commit a frozen `schema/history/v{N}.sql` dump, generated by a test that is gated on an environment variable.
3. **Equivalence test.** Build each supported predecessor from its frozen file, migrate it, and compare a normalized description (pragma table_xinfo, index_xinfo, foreign_key_list, and the trigger SQL with whitespace normalized) against a fresh install.
4. **Drop old versions.** Pre-v1, retire versions below a stated floor rather than accumulating branches.
5. **Fix the links.** Restore the three missing documents or remove the links. KEY_ROTATION content may already be covered by README sections.

**Already tracked:** book/docs/research/research-server.md (line 5) notes the missing README-linked documents. The migration structure and test-fixture weakness are not tracked.

<details><summary>Verifier note</summary>

All cited facts check out:
- schema.rs:44 is one `matches!(version, 43..=47)` block with version-conditional DDL (43, <=44), pragma_table_info probes (fence, admission_hash) and unconditional v48 SSO DELETEs. SCHEMA_VERSION is 48.
- tests/schema.rs:132 predecessor() builds versions 43-47 by dropping and renaming objects in the current schema.
- The upgrade test (line 175) asserts only the EXPIRY_INDEXES SQL, user_version, one recovery_challenges row and foreign_key_check. It never compares the full schema_objects of the upgraded database with a fresh install.
- No frozen historical schema files exist anywhere in the tree.
- foks-server's backup compatibility_tests.rs:59 copies the same reconstruction approach, which widens the blind spot.
- crates/foks-server/README.md links KEY_ROTATION.md (94), SCHEMA_MIGRATION_POLICY.md (333) and OIDC.md (504). None exists; only ADMINISTRATION.md, BENCHMARKS.md and PROTOCOL_SYNC_DESIGN.md are present.
- book/docs/research/research-server.md line 5 notes the missing documents; the migration-structure weakness is untracked.
The recommendation is feasible and server-internal. Versions 43-47 were never captured, so they would need a one-time frozen dump produced from the current predecessor() reconstruction. Low priority is appropriate.

</details>
