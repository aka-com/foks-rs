# Research note: the FOKS server (storage, concurrency, federation, services)

Scope: `crates/foks-server`, `crates/foks-server-db`, `crates/foks-server-testkit`, `crates/foks-oidc`, `scripts/benchmarks`. All paths are under `/home/bnoland/projects/foks-rs/`. The Merkle tree itself is covered elsewhere; here it appears only where the server batches it into transactions.

A note on documents referenced by the README: `crates/foks-server/README.md` links to `KEY_ROTATION.md`, `OIDC.md` and `SCHEMA_MIGRATION_POLICY.md`, but those files do not exist in the tree (only `ADMINISTRATION.md`, `BENCHMARKS.md`, `PROTOCOL_SYNC_DESIGN.md` do). The README itself is the best prose statement of the design rationale and is quoted where useful.

---

## 0. The shape of the server in one paragraph

The Rust FOKS host is a single process that owns one SQLite file. Every authoritative mutation goes through **one writer thread** fed by a bounded channel; every network read goes through a small **pool of read-only SQLite connections** that see WAL snapshots. Three TLS listeners (probe, public services, authenticated mTLS) run on a small Tokio runtime and hand each request to a bounded pool of blocking handlers. A once-a-minute maintenance thread runs bounded cleanup through the same writer and asks SQLite for a *passive* WAL checkpoint. Backups use SQLite's online backup API into a hidden staging directory that is atomically renamed. Everything is bounded: connections, queue depth, request bytes in memory, per-IP token buckets, rows deleted per pass, chunk bytes reclaimed per pass, listeners per inbox. The README states the philosophy directly: "This intentionally favors a simple total order and predictable overload behavior over write parallelism" (`crates/foks-server/README.md`, "Architecture, jobs, and scheduling").

---

## 1. Storage engine: SQLite in WAL mode with a single-writer actor

### 1.1 Why SQLite, and why one writer

**Problem.** A FOKS host must publish a totally ordered log: user chains, team chains, name commitments and a global Merkle tree whose every new root cites the previous one. Concurrency across those structures is not free; the README says outright that "identity/name publication and the global Merkle log still require one leader or a consensus protocol." Given that, SQLite embedded in the process is the simplest durable store that gives real transactions and a single file to back up. The README frames the future explicitly: partitioning could start with "opaque chunks first and then independent KV namespaces", but the core log stays single-leader.

**Mechanism (the writer actor).** `crates/foks-server/src/writer.rs`:

- `Writer::start` (lines 131–184) spawns a dedicated OS thread that opens the one read-write `Database` and loops on `std::sync::mpsc::sync_channel(maximum_pending)` (line 144). The channel bound *is* the queue limit (default 64 pending writes in the executable, README).
- A job is a boxed `Task` trait object wrapping a `FnOnce(&mut Database) -> Result<T>` closure plus a one-shot response channel (`Call`, lines 16–53). The thread runs `run()` (lines 352–359): receive, execute, send the result back, until `Message::Shutdown`.
- Callers use `WriterHandle::call_observed` (lines 246–309). It takes a mutex on an `accepting` flag (so shutdown can close the door), `try_send`s the task (never blocks; a full queue returns `Error::WriterQueue`, which the RPC layer maps to FOKS status 1012 `RateLimited`, e.g. `services/federation.rs:511`), then blocks on `receiver.recv()` for the result. So write submission is non-blocking with respect to queue capacity, but the caller waits for completion. Queue-wait and execution time are measured per task (lines 40–45) into atomic counters exported as Prometheus summaries.
- **Authorization is re-checked inside the queue.** `for_authenticated_request` (lines 222–236) attaches the caller's uid/credential; the wrapped closure (lines 259–277) first calls `active_credential_owner` and `sso_require_access` *on the writer thread, at execution time*, so a device revoked or an SSO session expired while the job waited in the queue causes `AuthorizationChanged` instead of a stale write. This is a general FOKS pattern: validate on a reader for early rejection, then re-validate inside the serialized transaction.
- `call_with_current_time` (lines 311–324) samples the clock *after* the task starts, not at enqueue time. A test (`current_time_is_sampled_after_a_queued_task_starts`, lines 505–544) pins this so timestamps stay monotone with the total order.

**Process-level exclusion.** `DatabaseWriterGuard::acquire` (lines 379–405) opens `<db>.writer-lock` with `O_NOFOLLOW`, mode 0600, and takes an OS advisory lock via `File::try_lock`. Offline operator commands (invite issuance, key rotation, admin grants) use the same guard, so they and the running server are mutually exclusive (tests at lines 487–502). The README: "An operating-system lock on the database sidecar rejects a second writer process; active/active service instances are not supported."

**Path identity as a defense.** `crates/foks-server-db/src/connection.rs` `DatabasePathIdentity` (lines 22–102) records device+inode of the database leaf, refuses symlinks, non-regular files, and files with more than one hardlink (`validated_metadata`, lines 277–305), and `recheck()`s the identity *after* taking the writer lock and *again after* SQLite opens the file (`open_with_identity`, lines 140–157). The comment explains the race it closes: "so a replacement cannot become a newly accepted baseline between the lock-time check and SQLite open." Non-Unix platforms fail closed (lines 44–49). This is deliberately "path-local and does not use a global inode lock registry" (README).

### 1.2 How readers coexist: WAL snapshots and a bounded reader pool

- `configure()` (connection.rs lines 345–419) sets the writer connection's pragmas: `journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON`, `trusted_schema=OFF`, `temp_store=MEMORY`, `main.journal_size_limit` = `wal_reuse_limit_bytes` (default 16 MiB), and `max_page_count` derived from `maximum_database_bytes` (default 64 GiB) so the database file itself has a hard quota (lines 397–416). `busy_timeout` defaults to 5 s.
- Readers are opened `SQLITE_OPEN_READ_ONLY | NO_MUTEX | NOFOLLOW` (`ReadDatabase::open`, lines 214–225) and validate application id + schema version before use.
- `ReadSnapshot` (lines 104–118) wraps `unchecked_transaction()` on a read connection: a request-scoped, *pinned* view of one WAL state. The doc comment warns not to hold it across network I/O "because an open reader can delay WAL checkpoints"; the realtime handler comments "Return the entire lease before waiting for the writer" (`services/realtime.rs:201`).
- The pool is tiny and explicit: `crates/foks-server/src/read_pool.rs` keeps a `Vec` of idle connections behind a mutex, opens lazily up to `maximum_connections`, and returns `Error::ReaderPool` rather than waiting when exhausted (lines 47–75). A `ReadLease` returns the connection on drop (lines 88–97).

WAL gives readers-don't-block-the-writer and writer-doesn't-block-readers; the price is that a long-lived reader pins WAL frames. The whole checkpoint design in §3 exists to live with that trade.

### 1.3 Schema versioning, STRICT tables, and invariants written as constraints

`crates/foks-server-db/src/schema.rs`:

- The schema is one concatenated string of 21 `.sql` files (lines 8–30). `APPLICATION_ID = 0x464f4b53` ("FOKS") and `SCHEMA_VERSION = 48` are stored in SQLite's `application_id` and `user_version` pragmas.
- `initialize()` (lines 32–117): a fresh file (both pragmas zero) gets the full schema inside one `BEGIN IMMEDIATE` transaction with the version stamped last. A file at version 43–47 is upgraded **in one transaction**, and the version is *re-read under the write lock* before acting ("Recheck under the write lock: another opener may already have upgraded", line 46). Steps are conditional and idempotent (`pragma_table_info` probes decide whether a column rename is still needed, lines 68–98). Anything else must match exactly or the open fails (`validate`, lines 126–136). Readers only validate; only the writer upgrades — the README: schema 44 "automatically upgrades only FOKS schema 43 during writer database initialization, under the process lock and before reader pools open", and the writer test `schema_upgrade_waits_for_exclusive_writer_ownership` (writer.rs lines 439–476) proves an offline guard blocks the upgrade.
- The stated policy is conservative: roll back by restoring a backup with the old binary, "never by changing `user_version` or deleting WAL files."

**Invariants encoded in SQL rather than Rust.** All tables are `STRICT` (SQLite 3.37+ type enforcement). Beyond that, the schema encodes state machines with `CHECK` constraints and *partial unique indexes*:

- Exactly one active host key: `CREATE UNIQUE INDEX host_key_one_active ON host_key_generations(purpose) WHERE state = 2` (`schema/core.sql:28`). One in-progress rotation: `host_rotation_one_in_progress ... WHERE phase != 3` (line 51). Phase/column consistency: the three-way `CHECK` on `host_rotation_operations` (lines 43–48) says which columns must be NULL in each phase.
- One active capability key (`capability_key_one_active`, line 68).
- A name row is either a reservation (token+expiry, no uid) or an assignment (uid, no token): `names` CHECK (`schema/identity.sql:9–10`); one live name per user via `names_current_uid ... WHERE dead = 0` (line 24). A trigger auto-appends the initial `user_name_history` row (lines 33–36).
- Chain heads are foreign keys into `(uid, seqno, link_hash)` of the links table (lines 63–68), so a head can never point at a link that isn't stored.
- Merkle roots cannot reference missing nodes and referenced nodes cannot be deleted — enforced by triggers (`schema/merkle.sql:22–50`).
- SQLite has no "exactly one row" table type, so singleton tables use `singleton INTEGER PRIMARY KEY CHECK (singleton = 1)` (`host_metadata`, `merkle_root_heads`, `signup_policy`, `kv_upload_maintenance`). `sso_single_host_policy ON sso_policy((1))` is a unique index on a constant expression — a one-row table trick (`schema/sso.sql`).
- Federation grants: `state=1 ⇔ revoked_at IS NULL` (`schema/federation.sql:16`), and tables are `WITHOUT ROWID` because their composite primary key is the natural access path.
- Team membership identity uses `coalesce(scoped_host_id, X'')` in a unique index so NULL (local) and a remote host id are both single-valued keys (`schema/team.sql:76–78`).

Why this matters for the book: the server pushes as much correctness as possible into the database so that *any* code path — including a future one — cannot produce an inconsistent state. The Rust code then treats a constraint failure as a normal error rather than a corruption.

### 1.4 Transactions that batch many tables (one FOKS "mutation")

The canonical example is `Database::commit_user_mutation_with_failure` in `crates/foks-server-db/src/user_mutation.rs` (lines 105–400). One `BEGIN IMMEDIATE` transaction does, in order:

1. Idempotency lookup (`idempotency::lookup`) — if this exact request already committed, return the stored response (lines 114–121).
2. Compare-and-check the *current* chain head and Merkle root against what the client cited (`expected_sequence`, `expected_tail_hash`, `expected_root_epoch/hash`); mismatch is `StaleRoot` (lines 122–143). This is optimistic concurrency on the chain: the client cites the head it built on; the writer, being the only writer, checks it atomically.
3. Signer must still be an active device; no racing revoked credential (lines 144–156).
4. Optional username change: validate reservation, kill old name, assign new one, append history (lines 157–188).
5. Enforce device/credential capacity (line 190).
6. Append the chain link, move the head, record the next tree location (lines 192–221).
7. Project the change: devices, revocations, shared keys (PUKs), parcels, seed-chain boxes (lines 224–297) — note the `ON CONFLICT ... DO UPDATE ... WHERE exact_box = excluded.exact_box` idiom (lines 275–287) that makes re-sending an identical box a no-op but a *different* box for the same slot an error.
8. Passphrase reboxing during owner-PUK rotation (line 302), generic subchain link (lines 310–317).
9. Merkle nodes (insert-if-absent, conflict if the same hash decodes differently, lines 321–338), leaves, new root row, back pointers, and head move (lines 339–381).
10. Insert the idempotency receipt with the response bytes (lines 383–390), then `commit`.

`inject(failure, ...)` points (lines 222, 298, 318, 340, 382, 391) are test seams that let tests crash the transaction at each stage and assert nothing partial is visible. The team variant (`team.rs` lines 143–...) follows the same skeleton, including consuming a team-name reservation with a single conditional `UPDATE ... WHERE ... AND expires_at > now AND team_id IS NULL` whose affected-row count is the check (lines 205–224).

The RPC side (`crates/foks-server/src/net/session.rs` `commit_user_mutation_argument`, lines 189–330) shows the two-phase pattern: an early idempotency read on a pooled reader (lines 203–218) returns fast or detects a *conflicting* retry (`OperationConflict` → BadArguments), then the writer closure recomputes the Merkle commit from the authoritative root and applies it.

**Idempotency receipts** (`crates/foks-server-db/src/idempotency.rs`, `schema/receipts.sql`): key is the link hash (for chain mutations) or the self token (signup, session.rs:597), the request hash is stored to detect a different request reusing the key, the response blob is stored so a retry gets the *same* bytes, and receipts expire (24 h for mutations) via the maintenance sweep.

### 1.5 Signup invites consumed inside the publishing transaction

`crates/foks-server-db/src/invites.rs` `consume()` (lines 154–187) is a single `UPDATE ... SET use_count = use_count + 1 WHERE code_hash=? AND state=1 AND (expires_at IS NULL OR expires_at > ?) AND (max_uses IS NULL OR use_count < max_uses) RETURNING invite_id`. Because it runs inside the signup transaction (`identity.rs:107`), the README's guarantee holds: "a failed signup cannot burn a code and concurrent redemption cannot exceed its use limit." Only a domain-separated hash of the code is stored (`schema/invites.sql:11`); the `signup_invite_redemptions` table has `UNIQUE (uid)` so one account redeems at most one invite.

### 1.6 Quotas and capacity

- KV namespace accounting (`crates/foks-server-db/src/kv.rs` `ensure_kv_capacity`, lines ~1370–1402) is a *recomputed sum*: the stored byte size of every directory/root/node/dirent/upload/chunk row plus 64 bytes per lock, and a row count, compared against `maximum_kv_namespace_bytes` (16 GiB) and `maximum_kv_namespace_objects` (1,000,000). The README admits this scan is an expected saturation point. Tree-shape limits (directories, dirents) are checked separately (lines 1405–1428) and re-validated at open (`validate_kv_tree_capacity`) so a configuration tighter than the data fails closed.
- Global database size is enforced by SQLite's own `max_page_count` (connection.rs lines 407–416), which turns a full disk into `SQLITE_FULL` → `QuotaExceeded` rather than corruption.
- Every capacity number lives in `crates/foks-server-db/src/config.rs` (`Config`, lines 3–90): devices per user 16, backup keys 4, chain links 4096, teams 4096, members 64, role bands 16, boxes per mutation 1024, challenges 4096 global / 8 per entity, passphrase generations 4096, capability tokens 32 per pair / 16,384 global, federation permissions 256 per target / 16,384 global. `configure()` refuses any zero limit (lines 346–379) and refuses KV limits above the protocol constants (lines 380–388) — a config that the client could not synchronize is rejected at startup.
- Passphrase brute-force limiting is durable, not in-memory: `bad_passphrase_attempts` rows are inserted per failure, pruned by window, and counted (`passphrases.rs` lines 211–216, 518–541): five failures in ten minutes rate-limits that UID (config lines 71–72).
- KV locks are "expired by the next acquirer" (`acquire_kv_lock`, kv.rs lines 600–657): the row stores `created_at`; the acquirer supplies its own timeout and may take over only if `now >= created_at + timeout`. No background reaper is needed, matching go-foks (comment at `maintenance.rs:115–116`).

---

## 2. Concurrency and admission control on the network side

**Listeners** (`crates/foks-server/src/net/listener.rs`): each listener has an accept task and a session loop. Accepts pass a per-IP token bucket (`allow_connection`, line 282) and then `try_send` into a bounded `mpsc` admission queue (lines 286–291): "A full admission queue deliberately rejects new work. This keeps memory bounded and lets TCP/TLS clients retry with backoff." Sessions acquire an `active` semaphore permit (default 256 per listener) before being spawned (lines 298–333).

**Token buckets with bounded memory** (`crates/foks-server/src/rate_limit.rs`): classic continuous-refill token bucket per IP — `tokens = min(burst, tokens + elapsed_seconds * rate)`, take one if ≥ 1 (lines 109–119) — but the per-IP map is an `lru::LruCache` capped at `maximum_tracked_ips` (4096), so an attacker cycling source addresses cannot grow memory; the oldest bucket is simply evicted (test at lines 132–153). Separate buckets exist for connections (512 burst / 256 s⁻¹) and framed requests (2000 burst / 1000 s⁻¹). A request that exceeds its bucket gets status 1012 and the connection closes (session.rs 1397–1403).

**Per-request bounds** (`crates/foks-server/src/net/session.rs`):
- Frame length is parsed from msgpack-style length markers with canonical-encoding checks (`read_frame_length_async`, lines 1783–1826) and capped at 16 MiB.
- Request memory is a **semaphore of bytes**: `read_call_body_async` (lines 1828–1856) acquires `2 × length` permits from `request_memory` before allocating, so total decoded request bytes in flight are bounded process-wide.
- Handlers run in `spawn_blocking` under an `execution` semaphore (default 64) with `try_acquire_owned` — saturation is an immediate `RateLimited`, not a wait (`execute_bounded`, lines 1753–1781). A 30-second timeout closes the connection but, as the README stresses, "without claiming that a durable mutation was cancelled"; the permit stays with the still-running closure.
- Long-polls (realtime and KEX) take a separate, smaller `realtime_polling` semaphore (min(execution, 32), lines 162–164) so they cannot starve ordinary handlers, and a `select!` on socket readability drops an abandoned poll immediately (lines 1519–1533): "Snowpack is request/response serial. Readability while this response is pending is therefore either a client cancellation/control frame or socket shutdown."

**KEX relay** (`crates/foks-server/src/net/session/kex.rs`): the pairing relay for new devices is an in-memory `VecDeque` bounded by 4096 messages / 32 MiB / 2-hour lifetime. Sends are idempotent on `(session_id, sequence, sender)` (lines 42–53, with the comment that replayed packets have different randomized box bytes so the first is retained), full capacity rejects rather than evicts live packets (lines 55–65), and receivers long-poll using `Notify` registered *before* inspecting the queue (lines 83–86) — the standard lost-wakeup avoidance — in five-second slices to mirror Go.

**Readiness** (`crates/foks-server/src/operations/management.rs` `readiness`, lines 223–250): `/readyz` requires startup complete, service live, and one bounded round trip through the writer within 1 s, with a compare-and-swap `in_flight` flag so probes cannot pile up.

---

## 3. Maintenance: bounded cleanup and the WAL checkpoint policy

`crates/foks-server/src/maintenance.rs`: a thread wakes every 60 s *after the previous pass ends* (`recv_timeout(INTERVAL)`, lines 29–43) and runs `run_with_checkpoint` (lines 88–133) as **one writer task**: cleanup transaction, optional web-admin cleanup, then a PASSIVE checkpoint, with metrics recorded in that order so "Committed cleanup remains counted if a later checkpoint fails" (test at lines 221–288).

**Bounded expiry** (`crates/foks-server-db/src/maintenance.rs` `run_maintenance`, lines 93–153; `maintenance/expiry.rs`): each family is `DELETE FROM t WHERE rowid IN (SELECT rowid ... LIMIT 128)` — a fixed page per pass. The `recovery_challenges` selector merges "expired and unconsumed" with "consumed" via `UNION ALL ... ORDER BY expires_at, rowid LIMIT 128` so one limit covers both branches (lines 41–55). Partial indexes in `schema/expiry_indexes.sql` are shaped for these exact predicates; the comment on `team_names_reservation_expiry` explains a planner trick: including the constant-NULL column first "gives the planner equality plus an expiry range ... even without statistics." Federation permissions are *never* swept (they are renewable or tombstones), and locks are not swept (next-acquirer expiry).

**Upload reclamation with a durable cursor** (`maintenance/uploads.rs`): the problem is deleting abandoned encrypted file uploads (older than 24 h) without a long transaction. The algorithm:
1. Read the cursor `(updated_at, uid, file_id)` from the singleton `kv_upload_maintenance` row (lines 27–31).
2. Select up to 128 candidates *after* the cursor in `(updated_at, uid, file_id)` order (keyset pagination, lines 32–56).
3. For each, `UPDATE ... SET reclaiming = 1 WHERE ... NOT EXISTS (dirent referencing this file in any retained version)` — liveness and marking are one statement on the single writer, so no dirent can be published in between (lines 63–74).
4. Advance the cursor; wrap to zero only after exhausting the window (lines 78–93).
5. Separately, for uploads already marked `reclaiming`, delete chunks one at a time, fetching *lengths not payloads* (lines 111–121), stopping at 16 chunks or 32 MiB or a 50 ms work budget (`WORK_BUDGET`, checked between statements); delete the upload row only when no chunks remain (lines 122–131). Everything is restartable because state is in the table, not in memory.

**Checkpoint policy.** `Database::checkpoint_passive` runs `PRAGMA main.wal_checkpoint(PASSIVE)`; `checkpoint()` runs `TRUNCATE` and is reserved for explicit callers (db maintenance.rs lines 155–172). `CheckpointReport::outcome` (lines 59–71) interprets SQLite's `(busy, log, checkpointed)` triple: complete only if counts are known and equal and busy is 0; `(-1,-1)` is "Unavailable". The README's Prometheus section documents why: PASSIVE never invokes the busy handler, so a pinned reader makes the pass *deferred* rather than stalling the writer for the 5-second busy timeout. `journal_size_limit` = 16 MiB bounds space retained on *natural* WAL reuse, not the live WAL. `BENCHMARKS.md` reports the measured effect: with a reader held, TRUNCATE checkpoints cost 286–288 ms and p99 queue wait ~290 ms, versus ~0.2 ms / ~1–6 ms with PASSIVE. The test `maintenance_and_following_write_finish_with_reader_still_held` (server maintenance.rs lines 147–218) pins the non-stall property.

Metrics semantics are unusually careful: frame gauges are "observations, not additive counts" (db maintenance.rs lines 40–50); `foks_checkpoint_frame_counts_available` goes to 0 during an attempt (`metrics/checkpoint.rs` lines 43–50) so a scrape cannot read a half-updated pair.

---

## 4. Backups, restore and retention

`crates/foks-server/src/operations/backup.rs` and `standalone.rs`:

- **Online backup**: `online_backup()` (connection.rs lines 307–343) uses SQLite's backup API in 128-page steps, sleeping 1 ms on `More/Busy/Locked`, with a cancellation callback; it creates the destination with `create_new` + mode 0600, `fsync`s it after close. A *read-only* source connection is used (backup.rs line 111), so backups never enter the writer queue.
- **Staging + rename**: each run writes into `.backup-<now>-<seq>.tmp`, and only on success renames to `backup-<now>-<seq>` and fsyncs the directory (lines 107–131). The manifest is written last inside `create_backup` ("its presence is the durable completeness marker", standalone.rs lines 461–462). Leftover staging directories are removed at startup (`cleanup_staging`).
- **Retention never touches foreign paths**: `is_completed_backup_name` (lines 198–207) requires exactly `backup-` + 20 digits + `-` + 20 digits; anything else (including `backup-user-data`) is ignored, and an *incomplete* directory with a valid name makes retention fail rather than delete (test lines 220–249). Every retained backup is re-validated before counting toward the retain limit (lines 179–186).
- **Key snapshot**: encrypted purpose keys for the current generation ledger plus a `key-manifest.txt`; the operator root key is never copied (README "Backup, restore, and integrity"). `restore_backup` (standalone.rs lines 97–150) requires empty destinations and publishes the database *first* so a failed key copy leaves a state that fails closed at startup rather than bootstrapping a new host over it (comment lines 117–119).

---

## 5. Keys at rest and the host's cryptographic identity

### 5.1 Encrypted key files and generation IDs

`crates/foks-server/src/keys/directory.rs`: every purpose key (`Host`, `Metadata`, `Merkle`, `ClientCa`, `DelegatedTls`, `Recovery`, `Capability`; `keys/mod.rs` lines 22–30) is a 32-byte secret stored as `FOKSK01\0 || generation(16) || nonce(24) || XChaCha20-Poly1305(secret, aad = purpose || generation)` (`load`, lines 132–174; fixed length check at line 138–143). The wrapping key itself is stored under the operator root key in `key-encryption.key` (`FOKSW01\0`). **Operator-root rotation** rewraps only that one file (temp file → verify by decrypting → rename → fsync dir, lines 77–112), so purpose keys and their generation IDs never change and "the database and clients do not observe operator-root rotation." A shared/exclusive advisory lock (`.key-encryption.lock`) separates a running server (shared) from offline rotation (exclusive) (lines 52–61).

`KeyGenerationManifest` (`keys/manifest.rs`) is a text file of `purpose hex-generation` lines persisted in `host_metadata.key_manifest_blob`; at startup `validate_existing` decrypts every key and compares generation IDs (lines 37–65) so a substituted key file fails closed.

### 5.2 Bootstrapping a host: the genesis hostchain

`crates/foks-server/src/host/bootstrap.rs` `bootstrap()` (lines 38–205) builds, in memory, the whole public identity of a new host:
1. Derive entity IDs (type byte + Ed25519 public key, `entity()` lines 346–351) for host, metadata signer, Merkle signer and delegated TLS CA.
2. Hostchain link 1: a `HostchainChange` naming the metadata key, Merkle key and TLS CA certificate, signed by all four keys in sequence over `signing_bytes(i)` (lines 63–95) — each signature covers the previous ones.
3. A signed `PublicZone` mapping service types to endpoints, signed by the metadata key (lines 100–120).
4. Merkle root epoch 1 containing a single zero leaf ("go-foks seeds the tree with a zero-key/zero-value leaf ... this makes absence proofs available at epoch 1", lines 123–129) and a `HostchainTail` pointing at link 1, signed by the Merkle key.
5. Package as a `ProbeResponse` and **verify it with the same client-side verifier** (`foks_verify::verify_public_host`, line 161) before storing — the server refuses to publish anything a client would reject.

On restart `load_or_bootstrap` (lines 210–344) does not rebuild; it validates that stored probe bytes, hostchain rows, Merkle rows and the key ledger all agree (`validate_generation_ledger_against_probe`, lines 353–423, cross-checks every `Key`/`Revoke` item in the hostchain against `host_key_generations` states). "Restart validation deliberately does not reconstruct genesis bytes from the current wall clock."

### 5.3 Two-link host key rotation with a mandatory observation window

`crates/foks-server/src/host/rotation.rs`. Rotating the *host* signing key is dangerous: clients pin the host by key. The design:
- `stage` creates a new immutable key generation and a durable `host_rotation_operations` row (phase 1) — repeated calls return the same operation, giving crash recovery a stable ID (lines 39–80). If a same-host contender wins the race, the unpublished key file is removed (lines 64–72).
- `begin` publishes hostchain link *N+1* adding the new key, signed by **both** new and old keys (`construct_publication` with signers `[new, old]`, lines 104–113), and a new Merkle root that cites it (lines 323–358). Phase 2.
- `complete` refuses unless (a) the operator passes the exact add-link seqno they observed from an independently syncing client or canary (`HostKeyRotationObservation`, lines 16–21, checked at 140–144) and (b) at least 24 h have elapsed (`MINIMUM_HOST_KEY_OBSERVATION_MICROS`, line 13, checked 145–155). It then publishes link *N+2* revoking the old key, signed only by the new key, commits, and only afterwards deletes the old private file (`retire_revoked_secret`, lines 409–425) — so a crash between commit and delete is finished by re-running the command.
- `validate_host_key_generations` (lines 179–224) is a small state-machine checker: exactly one active key, and the staged/retiring sets must match the operation phase.

### 5.4 Capability key rotation

Symmetric "capability" keys MAC challenges and encrypt federation bearer tokens. `keys/rotation.rs`: `rotate_capability_key` creates a new generation file first, then selects it in one SQL transaction, and removes the file if SQL fails (lines 16–33). `retire_capability_keys` marks revoked in SQL before deleting files (lines 38–56), and `cleanup_untracked_generations` (lines 61–77) erases files that SQL never referenced — "A crash after publishing an immutable file but before selecting it in SQLite leaves no durable reference. The ledger is authoritative." Retiring generations stay readable until dependents expire (core.sql lines 54–56).

---

## 6. Federation: how hosts talk about each other's users and teams

The README is candid: "Federation is limited to Beacon discovery followed by independently pinned remote hosts, expiring bearer grants for public user/team chains, and a durable client-coordinated remote-team admission workflow." There are no host-to-host push channels; **the client is the courier**. Everything below is a server-side primitive that a client stitches together.

### 6.1 Discovery: Beacon and probe

- `Beacon.beaconLookup` (`net/session.rs` lines 1113–1117) answers only for its own host ID with its advertised probe endpoint; any other host ID gets a typed not-found. The client treats the hint as untrusted "until a direct probe authenticates the requested HostID" (README).
- `Probe.probe` (`validate_probe`, lines 1119–1146) checks the canonical hostname and optional host ID and returns the stored, self-verified `ProbeResponse` (hostchain + signed public zone + signed Merkle root). The probe listener uses an operator-managed TLS certificate for the DNS name; service listeners use certificates under the *delegated CA committed in the hostchain* (bootstrap.rs lines 59–61, 78–82), so once a client has verified the hostchain it can authenticate the service endpoints without any external PKI.

### 6.2 Proving identity across hosts

Local users authenticate on the mTLS listener with device certificates issued by the host's client CA; `Principal::authenticate` (`auth/principal.rs` lines 23–63) binds the certificate to an active credential row and *re-checks that the certificate's Ed25519 key equals the credential's key*, classifying the credential kind (software device, delegated subkey, backup key, bot token). Remote users never get a certificate on a foreign host. Instead they present **bearer tokens** on the public listener, and the host checks them against rows it issued.

### 6.3 Remote view grants (bearer tokens sealed at rest)

`crates/foks-server/src/services/federation.rs`:
- A local user (or team admin) asks their own host to grant a specific remote party `(party_id, host_id)` the right to read their chain: `grant_remote_user_view` (lines 25–91) / `grant_remote_team_view` (lines 94–210). For teams the request carries a signature by the team's *current* admin/owner PTK (verified at lines 153–160) and a fresh timestamp.
- The token is 17 bytes: tag byte 54 + 16 random bytes (lines 309–311). The database stores only `token_hash` (a domain-separated hash), plus the token **encrypted under the active capability key** with XChaCha20-Poly1305 and an AAD binding of `domain || target || viewer || viewer_host || token_hash || key_generation` (`TokenBinding::aad`, lines 282–293; `seal_permission_grant`, lines 369–398). Decrypting checks the tag byte and the hash (lines 457–485). So a leaked database cannot be used to mint tokens without the capability key, and a token cannot be re-bound to another target.
- **Renewal without changing the bearer**: `prepare_permission_grant` (lines 296–366) decrypts the existing token with its old key generation and re-seals under the active generation (`migrate`) or with extended expiry (`extend`); the plaintext bearer already handed to the remote party stays valid. The README describes grants as 30-day renewable leases whose reauthorization "preserves the exact bearer already sealed into a local team's PTK box." Revocation is a permanent tombstone (state 0, `revoked_at` set) — `renewable_*` queries never return revoked rows (federation.rs db, lines 480–496).
- Verification on the *target* host: `load_remote_user_chain` (lines 212–241) hashes the presented token and asks `remote_user_view_token_is_current(hash, uid, now)`. No decryption is needed to *check* a token; only to *reissue* it.

The conformance test `federation_lifecycle` (`crates/foks-server-testkit/tests/conformance/federation.rs` lines 23–87) walks this end to end and even opens the SQLite file to assert the ciphertext does not contain the plaintext token.

### 6.4 Remote team membership

How a team on host A includes a team (or user) from host B (`remote_team_membership_and_ptk_tokens`, federation.rs test lines 89–330):
1. A's team admin grants B's team a remote view permission on A (so B can read A's team chain).
2. The client coordinates **team index range allocation** on both hosts (`allocate_federated_team_index_ranges`), each recorded as `ChangeMetadata::TeamIndexRange` in the respective chains — this is the "general team nesting" boundary: the server checks that persisted index ranges are consistent (`foks_verify::persisted_team_index_range`, used in `team_invitations.rs:80`).
3. A's admin posts a membership link adding B's team with `scoped_host_id = B` (`team_members.scoped_host_id`, team.sql:66) and a **PTK view box** for it in `team_remote_member_view_tokens` (federation.sql lines 44–58), which is keyed to a specific PTK generation by foreign key.
4. B's members later fetch the box with `load_remote_view_tokens` (team_loader.rs lines 534–601) and can decrypt A's PTK through their own team's seed chain, even after A rotates PTKs for unrelated reasons (the test's comment: "The federation bearer box stays bound to its authenticated historical PTK generation and must remain recoverable through the PTK seed chain").
5. On A's public listener a foreign member is authenticated "by its scoped roster key" rather than mTLS (`team_loader.rs` comment at lines 743–744): the team-view challenge in §6.5 is signed with the member's source key.

Remote *user* membership and nested teams in general are explicitly out of scope; the README enumerates the missing pieces.

### 6.5 Stateless challenges and capability tokens

Two related mechanisms let a party prove authority once and then use a short bearer for a session:

- **Team-view challenge** (`services/team_loader.rs`): `issue_challenge` (lines 12–77) builds a `TeamViewChallenge` = request + time + 16-byte token + capability key id + **MAC** over the payload under the capability key. The server stores nothing at issue time — the MAC makes the challenge self-authenticating (stateless). `activate` (lines 79–199) verifies the MAC, checks the 6-hour lifetime, re-derives the member's *current* verify key from the roster (`team_view_authority`), verifies the member's signature over the exact challenge, then inserts `team_view_tokens` keyed by `hash(token)` in the writer with an `activation_hash` of the whole argument so an exact retry is idempotent while a different activation of the same challenge is rejected as superseded (lines 161–178; schema `team_view_challenges.activation_hash`). Subsequent chain loads present the bearer, which is resolved by hash (`resolve_team_view_token`).
- **Team admin tokens** (`services/team_admin.rs`): `make_inert_token` (lines 70–128) mints a random token bound to (team, holder, PTK role, generation) in an *inert* state; `activate_token` (lines 130–208) requires a signature by the team's admin/owner PTK over a challenge naming that token, and a fresh timestamp, before the token can be used for invitation-inbox operations. Both token families expire and are swept.

### 6.6 Invitations

`crates/foks-server/src/services/team_invitations.rs` and `foks-server-db/src/team_invitations.rs`, schema `team_invitations.sql`:
- A **team certificate** (signed by the team's admin PTK) is uploaded and stored by hash, scoped to a PTK generation; lookups return it with the team's index range (`certificate_lookup`, lines 57–87; `certificate_put` verifies the signature before writing, lines 88–125).
- Join requests are durable rows: local (`team_local_join_requests`, with a partial unique index guaranteeing one *pending* request per joiner/role) and remote (`team_remote_join_requests`, holding the encrypted request bytes and the certificate hash). Decisions record the chain link that implemented them (`decision_sequence`, `decision_link_hash`).
- The DB layer's comments explain the authority split: "Called before replacing the roster, so metadata-only edits cannot approve an existing member's self-invite. The chain remains acceptance authority" (db lines 451–452), and removal proofs are posted after the fact with "server authority is the team-scoped commitment" (lines 638–639).

---

## 7. Realtime chat: inbox versions, durable reconciliation, and long-poll wakeups

The realtime subsystem stores only ciphertext (`schema/realtime.sql` header comment) and is where the server's notification design lives.

### 7.1 Data model: a per-user inbox version is the cursor

- `rt_channels` are immutable configuration plus an append-only `last_sequence`/`last_message` (a trigger makes `format` immutable, realtime.sql lines 23–26).
- `rt_messages` are keyed by client-chosen `message_id` with `UNIQUE (channel_id, sequence)`.
- `rt_user_inboxes(uid, app_id, version, reconcile_*)` and `rt_user_channels(uid, channel_id, inbox_version, read_through, accessible)` with `UNIQUE (uid, app_id, inbox_version)`: **every change visible to a user bumps that user's inbox version, and each channel row records the version at which it last changed**. A client that knows version *v* asks `GetChangedThreads(since = v)` and gets exactly the channel rows with `inbox_version > v` (inbox.rs `rt_changed_threads`, lines 441–544). That is a per-user monotonic cursor: cheap to compare, trivially resumable, no per-message fan-out storage.

### 7.2 Send: idempotent append with ordering preconditions

`crates/foks-server-db/src/realtime/messages.rs` `rt_send` (lines 5–130), all inside one `BEGIN IMMEDIATE`:
- Authorization and policy (`authorize`, `require_write`), encryption role must match the channel's read role.
- **Idempotency**: if `message_id` exists, it must be the same channel, sender and envelope (with the volatile precondition zeroed, lines 47–50) — then return the *original* receipt with no wake targets; otherwise `OperationConflict` (lines 51–63).
- **Ordering**: `expected_previous_sequence` is a compare-and-set on the channel's `last_sequence` (`RtMessageOrder` on mismatch, lines 65–74), and `previous_id/previous_sequence` inside the encrypted metadata must name the actual predecessor (lines 75–86). Sequence numbers are allocated by the server as `last + 1`.
- Then `stamp()` (inbox.rs lines 38–72) fans out: for each eligible recipient (roster query bounded by `FANOUT_MEMBERS = 1024`, lines 4–37), create the inbox if missing, increment its version, upsert the channel row with `read_through = MAX(read_through, excluded.read_through)` (the sender's own read pointer advances to the new message), and collect a `RealtimeWakeTarget`.
- Wake targets are returned *with* the commit result; the service layer calls the notifier only `after_commit` (`services/realtime.rs` lines 51–60), and a test asserts retries and failures never notify (lines 307–338). Hints are explicitly best-effort: "Inbox versions remain the source of truth: missed/coalesced hints must never affect send confirmations or recovery" (lines 7–9).

### 7.3 Membership changes: triggers dirty the inbox; readers reconcile in pages

Problem: when a user joins or leaves a team, which channels they can see changes, but recomputing every inbox on every roster edit inside the roster transaction would be unbounded work on the single writer.

Solution (schema 44): `schema/realtime_invalidation.sql` installs triggers on `team_members` insert/update/delete and on `teams` host/kind changes that set `reconcile_dirty=1` and clear the cursor on *existing* inboxes only — O(members) trivial updates, in the same transaction as the membership change, so there is never a window where an inbox is clean but wrong. Reconciliation is then **pulled** by the affected user:
- `rt_inbox_state` (inbox.rs lines 92–127) classifies an inbox as Missing / Dirty / Incomplete (clean flag but a saved cursor) / Clean.
- `rt_reconcile_inbox_page` (lines 212–330) runs in the writer: it walks the union of "channels of teams I'm a member of" and "channels I currently have rows for", ordered by `channel_id`, **one page (4096) past the saved cursor** (`(?3 IS NULL OR channel.channel_id > ?3)`), decides readability per channel using a per-team role cache, inserts or hides rows (`insert_user_channel`/`remove_user_channel` bump the inbox version only when something actually changes), and stores the next cursor or NULL. If the dirty flag was set, it restarts from the beginning (`restarted`). A wake is emitted only if accessibility changed.
- Read paths (`GetChangedThreads`, `PollInbox`) check state on a *reader snapshot* first; only a dirty/incomplete inbox enters the writer queue (`services/realtime.rs` lines 179–219; handler `poll_inner` lines 149–163). The README summarizes: "Clean PollInbox and GetChangedThreads requests read authenticated durable inbox state without entering the writer queue." Metrics distinguish `clean_skips` from `submissions` so operators can see idle reconciliation is zero.

### 7.4 Long-poll delivery: `Notify` hub with weak registrations plus a 1-second fallback

`crates/foks-server/src/net/session/handlers/realtime.rs` `poll_inner` (lines 64–198):
1. Authenticate the certificate and authorize on a pooled reader (`blocking`), create the `RealtimeActor`.
2. Get (or create) the user's `Arc<Notify>` from the `InboxHub` (`listener`), and *enable* a `notified_owned()` future **before** reading state (line 119–120) — again the lost-wakeup pattern.
3. Read inbox state; if `version > since` return `bumped` immediately; if dirty, reconcile and re-read.
4. Otherwise wait on the notify future with `timeout(min(remaining, 1 s))` (line 187). The 1-second cap is the "authorization and missed notification check" the README mentions: every second the loop re-reads state on a fresh snapshot, so a revoked device or a coalesced hint costs at most a second. Default wait 25 s, max 55 s (lines 84–87), 32 concurrent polls (`realtime_polling` semaphore).
5. Outcomes (Bumped / TimedOut / Failed / Cancelled) are recorded by a `Drop` guard so cancelled futures are counted (lines 32–56).

`services/realtime/inbox_hub.rs`: the hub maps `WakeKey{host, uid, app}` → `Weak<Notify>` plus a generation number. Weak references mean a poller that disconnects leaves no strong reference, and `notify()` (lines 132–160) prunes dead entries opportunistically; `listener()` amortizes cleanup with a `CLEANUP_BUDGET = 8` FIFO scan per registration (lines 22–41) so no single call does unbounded work. `notify_all()` on membership change wakes every live listener (lines 98–120). `notify_waiters()` (not `notify_one`) is used so a hint reaches every concurrent poll for that user.

### 7.5 Bounds

`crates/foks-server-db/src/realtime/limits.rs` fixes 256 channels per team, 1024 fan-out members, 16 KiB channel metadata, 256 B activity summary, 1000 inbox rows / 4096 scan rows per response, 8 MiB history bytes — and a `const` block plus a test asserts these fit both the protocol's wire container limits and the SQL `CHECK(length(...) <= N)` bounds in the schema (lines 21–75). History reads use a `Budget` that counts rows and bytes and fails with `Capacity` rather than truncating silently (messages.rs lines 216–230).

---

## 8. OIDC / SSO: binding an identity-provider login to a FOKS account

### 8.1 The `foks-oidc` crate: a deliberately narrow OpenID Connect client

`crates/foks-oidc/src/lib.rs`: the module doc says it all — "A validated provider token is not a FOKS device credential." The crate validates ID tokens and manages the code flow; it never creates FOKS keys.

- **Token validation** (`TokenValidator::validate_mode`, lines 83–159) wraps the `openidconnect` crate's verifier with the issuer, client id and a JWKS fetched from configured discovery; allowed algorithm is pinned to RS256; issue time must not be in the future; nonce is mandatory (optional only on refresh); `azp` must equal the client id when present and is required for multi-audience tokens; subject must be non-empty, ≤255 ASCII. A fixture corpus of bad tokens (lines 216–242) enumerates the rejected cases.
- **Session state machine** (`SessionState::permits`, lines 162–189): Created → AwaitingBrowser → Exchanging → Ready → Binding → Completed, with terminal Expired/Cancelled/Denied/Rejected/ExchangeUnknown; a test asserts terminal states cannot restart and Binding cannot be cancelled (lines 252–279). `ExchangeUnknown` exists because a code exchange that times out may or may not have consumed the one-time code — the design refuses to guess.
- **Network policy / SSRF defence** (`provider.rs`): `NetworkPolicy::check_url` (lines 24–50) requires https (http only for loopback in tests), no userinfo/fragment, and a public IP; `permits_ip` and `public_ipv4` (lines 52–82) exclude RFC 1918, link-local, CGNAT, documentation, multicast ranges and the IPv6 non-global blocks. Critically, DNS is also policed: `GuardedResolver` (lines 84–101) resolves names *and rejects any resolved address that is not public*, so a provider hostname pointing at 169.254.169.254 fails. The HTTP client has redirects disabled, no proxy, 10-second deadlines, and every response is read with a hard 1 MiB cap (`bounded_response`, lines 135–151); the `SyncHttpClient` impl refuses redirects for token requests specifically so "Never forward a code, verifier or client secret to a redirect target" (lines 273–276).
- **Discovery** (`Provider::discover`, lines 159–190) validates every advertised endpoint URL with the same policy and requires RS256 in `id_token_signing_alg_values_supported`.
- **Authorization code + PKCE** (`exchange.rs` `authorization_url`, lines 57–101): state = the FOKS session id, nonce from the client, S256 challenge from the client-held verifier. There is a documented compatibility branch for the pinned Go client's 27-character verifier (below the RFC 7636 minimum of 43), computed by hand because the SDK would panic (lines 76–88). Scopes are requested only if the provider advertises them.
- **Key rotation tolerance**: if ID-token validation fails after a code exchange, the JWKS is refetched *once* and validation retried — "never replay the consumed code" (`checked_tokens`, lines 193–215; same in `refresh`).

### 8.2 The server-side coordinator (`crates/foks-server/src/sso/`)

- **Durable sessions, encrypted payload** (`mod.rs` `init`, lines 185–255): the client's `OAuth2SessionId` is hashed for the row key; the verifier, nonce and later the tokens live only in `ciphertext`, sealed by `envelope.rs` with XChaCha20-Poly1305 under a key derived from the host's `Recovery` purpose key, with AAD over `host || session_hash || config_hash || source_hash || uid || state || revision || authorization_epoch || expires_at` (lines 14–29). Because state and revision are in the AAD, a row cannot be rolled back to an earlier state by copying old ciphertext.
- **Optimistic concurrency on sessions**: `sso_transition` (`foks-server-db/src/sso.rs` lines 153–176) is `UPDATE ... WHERE state=? AND revision=? AND expires_at_ms>? AND interrupted=0` and requires the transition be permitted by the state graph; one changed row or `AuthorizationChanged`. "CAS losers reload the already-persisted result/reservation instead of making a second one" (`binding.rs` `poll_once`, lines 37–111, loop of 4).
- **Crash safety**: at startup `sso_abandon_exchanges` flags any session that was mid-exchange as `interrupted` ("No authorization code replay", sso.rs lines 177–184); such sessions fail closed.
- **Browser adapter** (`http.rs`): a hyper server on a loopback port with 32 connection slots, keep-alive off, 16 headers, 16 KiB buffers, 45-second connection timeout; `/oauth2/start?state=` redirects to the provider, the callback accepts only a whitelisted set of query keys with no duplicates, requires exactly one of `code`/`error`, and enforces RFC 9207 `iss` if present (lines 114–171). Start and callback use separate semaphores "so new sessions/start requests cannot starve callback completion" (mod.rs lines 35–41).
- **Binding to a FOKS identity** (`binding.rs` `prepare_binding`, lines 136–269): the client submits a **signed binding** by its device key over `{uid, host, Merkle root, ...}`; the server verifies the signature, checks `oauth2_binding_nonce(binding) == session.nonce` — i.e. the OIDC nonce *was derived from the binding*, so the ID token the provider signed is cryptographically tied to this device and host — and checks the submitted `id_token` equals the one stored in the session (lines 153–159). The cited Merkle root must exist on this host (lines 178–186). The result is an `SsoAccess` row `(host, uid, issuer, subject)` with `UNIQUE(host, issuer, subject)` — one FOKS account per provider subject. Purposes are Signup / LinkExisting / Reauthenticate, with revision and `authorization_generation` counters.
- **Ongoing enforcement**: every authenticated write (`writer.rs` lines 267–274) and many reads call `sso_require_access`, which consults `sso_policy::decision` (NoPolicy / MigrationEligible / LinkedActive / LinkOnly / Denied). Access expiry triggers a refresh (`access.rs` `ensure_access`, lines 29–150): the refresh token is decrypted, the state is CAS'd to Refreshing so only one refresh runs, and provider outages map to `ProviderUnavailable` rather than revocation. "Network I/O never owns a writer transaction" (line 28).
- **Policy hardening**: the `sso_policy` singleton records issuer, rollout id and a config fingerprint; a mismatch at startup *blocks* the policy rather than silently changing providers (`mod.rs` lines 58–76, `KeyUnavailable` if the Recovery key is missing); enforced rollout requires an empty host, and the migration cohort table is immutable by trigger (`schema/sso.sql`).

---

## 9. Web administration: tickets, cookies, CSRF and a suspend-aware clock

`crates/foks-server/ADMINISTRATION.md` plus `web_admin/`:
- Authority comes only from explicit offline grants (`host_admin_grants`, checked "against the singleton host_metadata in every transaction", `schema/web_admin.sql:1`).
- **Handoff**: the native app gets a 20-byte ticket (≤60 s) via RPC; `GET /?session=` only *stages* a pending cookie and redirects away from the secret URL; the confirmation `POST` atomically consumes the ticket and installs a 32-byte session cookie (`sessions.rs` `web_issue_ticket`/`web_stage`/`web_redeem`, lines 4–150; the redeem does `UPDATE web_login_tickets SET state=1 WHERE ... AND state=0` and requires one changed row). Cookies are `__Host-`, Secure, HttpOnly, SameSite=Strict (`http.rs` lines 20–21, 200).
- **CSRF**: tokens are hashes of the cookie under domain-separated constants (`PENDING_CSRF_DOMAIN`, `SESSION_CSRF_DOMAIN`, `VERIFY_CSRF_DOMAIN`, `service.rs` lines 13–16); the server stores `hash(VERIFY, hash(SESSION, cookie))` and compares with constant-time equality (`sessions.rs:112`); every POST also requires exactly one `Origin` header equal to the configured origin (`http.rs` lines 235–237). Form nonces (`web_admin_nonces`) make invite creation retry-safe: a retry "returns the committed invite ID without regenerating or storing its plaintext code."
- **Time**: `web_admin/clock.rs` combines UTC with a suspend-inclusive monotonic clock (Linux `CLOCK_BOOTTIME`, macOS `CLOCK_MONOTONIC_RAW`) and a random per-process epoch; a backwards UTC step *fences* the epoch (sample errors until UTC catches up) and then rotates it, invalidating all deadlines issued under the old epoch (lines 67–86, test 110–123). Deadlines are stored as elapsed microseconds tagged with the epoch, so copying a cookie or restoring a backup cannot extend a session.
- Everything is capacity-capped per account and globally (3/6/5 tickets/confirmations/sessions per account; 1024/2048/4096 globally) and audited atomically with the mutation.

---

## 10. The test kit: ephemeral, isolated servers

`crates/foks-server-testkit` (excluded from default workspace members, publish-disabled):
- `IsolatedPaths::create` (`config.rs` lines 11–28) makes a `tempfile::TempDir` with `database/`, `keys/`, `backup/`, `logs/` subdirectories; the environment owns a fixed test root key (`[0x51; 32]`), a `TestClock` (an `AtomicU64` of microseconds the test can `set`/`advance`, `clock.rs`), rcgen-generated Ed25519 CA + `localhost` leaf certificates for the probe listener (`certs.rs`), and a `SessionFaults` handle for fault injection.
- `InProcessServer::start_with_services` (`process.rs` lines 30–109) calls the real `foks_server::start_standalone` with all listeners bound to `127.0.0.1:0` (ephemeral ports), the injected clock and entropy, and a `TestProfile` (`environment.rs` lines 543–556) that shrinks capacities (`SmallCapacity`, `SmallTeamCapacity`, `SmallFederationCapacity`), the writer queue (`QueuePressure`), I/O timeouts (`TightIo`), reader pool size (`RealtimeSingleReader`), rate limits (`RateLimited`, e.g. 1 request/s burst 2) or enables 25 ms automatic backups (`BackupAutomation`). A `running` mutex prevents two servers on one environment, and the environment retains the addresses so a restart must reuse them.
- `TestEnvironment::mutate_database` (lines 258–281) routes test-side seeding through the live `WriterHandle` when the server runs, or through the same `DatabaseWriterGuard` when stopped — the test kit obeys the single-writer rule too.
- `WriterQueuePressure` (`scheduling.rs`) deliberately blocks the writer with one job and queues a second, then waits until `pending == 2`, so tests can observe rejection and drain behaviour deterministically.
- `BinaryServer` (`binary.rs`) spawns the real `foks-server` executable with generated files, captures stdout/stderr up to 1 MiB, and waits for readiness — used by release/binary tests and backup/restore rehearsals.
- Offline operations (host key rotation, capability rotation) are exercised by stopping the server and using `DirectoryKeyProvider::open_for_rotation` with the advanced clock to satisfy the 24-hour window (`environment.rs` lines 150–214).

---

## 11. Benchmarks (`crates/foks-server/BENCHMARKS.md`, `scripts/benchmarks/`)

- `tools/foks-server/bench.sh` runs three loopback TLS workloads: 128 simultaneous connections, 48 slow-loris clients withholding frames alongside 32 healthy ones on one runtime worker, and 10,000 submissions against a full writer queue followed by drain-and-recovery. Assertions are correctness properties (bounded rejection, healthy progress, recovery), not latency thresholds.
- The WAL checkpoint comparison (TRUNCATE vs PASSIVE vs PASSIVE+16 MiB) is described in §3, with the explicit caveat that it "does not prove that 16 MiB is optimal."
- `scripts/benchmarks/README.md` describes the desktop notification acceptance harness: a Python driver runs a TypeScript worker against `TestEnvironment::ProductionBenchmark`, alternating arm order, checkpointing trials, hashing binaries, and measuring foreground-history p95 and ≥99 % candidate discovery. The "inbox CPU benchmark" and the server expiry fixtures (`expiry_scale_and_write_cost`, `concurrent_expiry_and_foreground_writes`) round it out. The recurring stance: measurements are observations for a machine, never product capacity claims.

---

## 12. Invariants and clever tricks worth calling out in the book

1. **One writer, many snapshot readers** — total order for free; WAL for reader concurrency; every read that must be consistent across tables pins a `ReadSnapshot`.
2. **Authorize twice**: reject early on a reader, re-check *inside the serialized write* (writer.rs 259–277; team/user mutation `active_signer` checks; `check_actor` in realtime). Time is sampled inside the job.
3. **Never block on capacity; reject**: `try_send`, `try_acquire_owned`, admission `try_send`, reader pool `Err(ReaderPool)`. Overload becomes a typed status the client can back off from.
4. **Bounded work per pass with durable cursors** (upload reclamation, expiry `LIMIT 128`, reconciliation pages of 4096, hub cleanup budget 8).
5. **Idempotency by content hash + stored response** (request receipts; realtime send by message id; activation hashes on token activation; rotation operations returning the same operation id).
6. **Conditional UPDATE with affected-row check** as the atomic primitive (invite consumption, reservation consumption, ticket redemption, SSO CAS, KV lock takeover).
7. **AEAD with rich AAD as a binding mechanism**: bearer tokens bound to target/viewer/host/key generation; SSO ciphertext bound to state/revision/epoch; key files bound to purpose/generation.
8. **Stateless MAC'd challenges** so issuing a challenge costs no storage, and only activation writes.
9. **Ledger-before-file for secrets**: SQL rows are authoritative; files that SQL never referenced are garbage; delete files only after the revoking commit.
10. **Self-verification at bootstrap**: the server runs the client verifier over its own probe bytes before storing them.
11. **Two-link key rotation with a human-observed sequence number and a 24-hour clock** — a social/temporal safety interlock encoded in code.
12. **Triggers as invalidation, readers as reconcilers**: O(1) dirtying inside the roster transaction, paged repair later, never a stale-but-clean inbox.
13. **Lost-wakeup avoidance**: register `Notify` before checking state (KEX relay, inbox poll, SSO poll), plus a 1 s re-check as belt and braces.
14. **Bytes as semaphore permits** for request memory.
15. **LRU-bounded token buckets** for per-IP limits.
16. **Partial unique indexes as state machines**, `CHECK` clauses tying nullable columns to state, `STRICT` tables, singleton tables via `CHECK(singleton = 1)` or a unique index on a constant.
17. **Metrics that cannot lie**: frame gauges marked unavailable mid-attempt; committed cleanup counted even if the checkpoint fails; rolled-back cleanup never counted.
18. **Epoch-tagged, suspend-aware deadlines** that survive clock steps and restores.

---

## 13. Suggested external references

- SQLite: "Write-Ahead Logging" (https://www.sqlite.org/wal.html), including checkpoint modes PASSIVE/FULL/RESTART/TRUNCATE and `wal_checkpoint` return values; "PRAGMA journal_size_limit", "PRAGMA max_page_count", "PRAGMA synchronous", "STRICT Tables" (https://www.sqlite.org/stricttables.html), "Partial Indexes" (https://www.sqlite.org/partialindex.html), "Online Backup API" (https://www.sqlite.org/backup.html), "Isolation in SQLite" (https://www.sqlite.org/isolation.html), `application_id`/`user_version` pragmas.
- Actor model / single-writer designs: Hewitt's actor model; the "single writer principle" as popularised by Martin Thompson (Mechanical Sympathy blog) [VERIFY exact citation].
- Token bucket: RFC 2697 / RFC 2698 (single/two-rate three-colour markers) as the formal description [VERIFY relevance], or Tanenbaum's networking text for the classic algorithm.
- Keyset (cursor) pagination vs OFFSET: Markus Winand, "Use The Index, Luke" — "Paging Through Results" [VERIFY].
- Optimistic concurrency control: Kung & Robinson, "On Optimistic Methods for Concurrency Control" (ACM TODS 1981).
- Idempotency keys: Stripe's engineering write-up on idempotent requests [VERIFY]; also the "exactly-once" discussion in Kleppmann, *Designing Data-Intensive Applications*, ch. 11.
- AEAD and associated data: RFC 5116 (AEAD interface); XChaCha20-Poly1305 draft `draft-irtf-cfrg-xchacha` [VERIFY status]; RFC 8439 (ChaCha20-Poly1305).
- OpenID Connect Core 1.0 (nonce, `azp`, ID token validation §3.1.3.7); OpenID Connect Discovery 1.0; RFC 6749 (OAuth 2.0); RFC 7636 (PKCE, verifier length 43–128); RFC 7519 (JWT); RFC 7517 (JWK); RFC 7515 (JWS); RFC 9207 (`iss` in authorization responses); RFC 6819 / OAuth 2.0 Security Best Current Practice (draft-ietf-oauth-security-topics) [VERIFY RFC number if published].
- SSRF and DNS rebinding defences: OWASP SSRF Prevention Cheat Sheet [VERIFY URL].
- Cookie prefixes and SameSite: RFC 6265bis (`__Host-` prefix, SameSite) [VERIFY draft status]; OWASP CSRF Prevention Cheat Sheet (double-submit / synchroniser token).
- mTLS and X.509: RFC 5280; Ed25519 in X.509: RFC 8410.
- Long polling: RFC 6202 ("Known Issues and Best Practices for the Use of Long Polling and Streaming in Bidirectional HTTP").
- Lost-wakeup / condition-variable discipline: Tokio `Notify` documentation (`enable()` on `Notified`) [VERIFY], and the classic Mesa-semantics discussion (Lampson & Redell, "Experience with Processes and Monitors in Mesa", 1980).
- Linux `CLOCK_BOOTTIME` and macOS `CLOCK_MONOTONIC_RAW`: `clock_gettime(2)` man pages.
- Prometheus exposition format 0.0.4 (text format documentation).
- Argon2id: RFC 9106 (for the passphrase parameters the README mentions).
