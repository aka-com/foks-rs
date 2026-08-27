# FOKS client/server interoperability test plan

Status: proposed implementation plan

Scope: the public Rust `foks-client` talking to the standalone Rust
`foks-server`, with Go FOKS v0.1.9 fixtures used as the compatibility baseline

## 1. Goal

Prove that the client and server work as two independently restartable programs,
not merely that their internal functions agree. The primary test driver is the
public `foks-client` API. Raw RPC is reserved for protocol errors and recovery
conditions that the public client cannot express.

The release-level scenario is:

1. Start a server from empty temporary database and key directories.
2. Probe it, verify its host material, and persist the client's pin.
3. Create a software-device account and authenticate with its issued
   certificate.
4. Sync the initial user and KV state.
5. Create directories, small files, a symlink, a multi-chunk file, and a lock.
6. Shut down and reconstruct both client and server from their durable state.
7. Reauthenticate and read the same user, PUK, Merkle, and KV state.
8. Back up the server, restore it with the matching key material, and repeat the
   reads using the client's existing pins and device credential.

The scenario must pass through the installed server binary as well as the fast
in-process harness. The binary portion requires an operator-facing backup
command before backup/restore becomes part of that gate. A passing in-process
test alone is not a release gate.

## 2. Boundaries and invariants

This work remains isolated from AKA:

- Only `crates/foks-*`, `tools/foks-server`, FOKS fixtures, and unavoidable
  shared workspace metadata may change.
- No production or test FOKS crate may directly or transitively depend on an
  `aka-*` crate.
- `foks-server-testkit` remains `publish = false`, is omitted from workspace
  `default-members`, and is never a production dependency.
- Every test database, client store, key directory, backup, and log directory
  is beneath a test-owned temporary root.
- Every listener binds an ephemeral loopback address. Tests never consult a
  user home, an AKA data directory, or production FOKS environment variables.
- Tests are parallel-safe and do not depend on execution order, fixed ports,
  wall-clock sleeps, or state created by another test.

Client/server tests assert behavior through the public protocol. Direct
database reads are allowed only for testkit isolation, backup integrity, and
storage-corruption setup. They must not be used to make an otherwise failing
protocol assertion pass.

## 3. What exists now

`foks-server-testkit/tests/public_probe.rs` already demonstrates most of the
happy path against real loopback TLS:

- probe verification and durable pinning;
- username reservation and collision;
- current and historical Merkle reads;
- software account creation, certificate issuance, mTLS authentication, user
  chain verification, and PUK recovery;
- KV root creation, a small file, symlink, nested directory, multi-chunk file,
  sync, and lock contention;
- rejection of a certificate/device identity mismatch;
- creation of a server backup.

This is a useful smoke test, but it currently has important blind spots:

- It is one long test, so a failure does not identify the broken protocol
  boundary quickly.
- It starts the server in-process and does not exercise CLI parsing, process
  signals, exit status, or stdout/stderr diagnostics.
- It does not restart either side from durable state.
- It does not restore a backup while retaining the client's existing pins and
  credential.
- It does not exercise dropped responses, mid-upload disconnects, retries,
  expiry, quota boundaries, queue saturation, or concurrent writers.
- Positive public-client calls and raw TLS/RPC checks are mixed in the same
  scenario.
- One timing workaround sleeps after a TLS write failure; deterministic tests
  should instead poll bounded observable state or use explicit synchronization.

The existing test stays as a short release smoke test after its helpers and
domain assertions have moved into focused suites.

Two implementation details are expected to fail the first lifecycle tests and
should be treated as findings, not hidden by the harness:

- `start_standalone` currently reconstructs genesis bootstrap bytes from the
  supplied `now_microseconds` on every start, while SQLite idempotency compares
  the exact stored probe/genesis bytes. The testkit's fixed timestamp masks this
  issue; the real binary supplies the current wall clock and therefore needs a
  load-and-validate-existing-bootstrap path on restart.
- The binary currently waits for a signal after startup and has no
  operator-facing backup command. Process backup/restore tests require a real
  CLI operation or an equivalent stable operations interface.

## 4. Test layers

Use the narrowest layer capable of proving each property:

1. **Public-client contract tests** are primary. They use only exported
   `foks-client` operations and persisted client stores.
2. **Raw-RPC behavior tests** cover stable status codes, malformed messages,
   replay conflicts, and protocol operations not exposed by the public client.
3. **Process tests** spawn the actual `foks-server` binary and prove startup,
   shutdown, restart, backup/restore, and diagnostics.
4. **Server/storage tests** remain responsible for transaction internals,
   SQLite corruption classification, and exhaustive fault points. An
   integration test should not duplicate a lower-level invariant unless it
   proves a client-visible outcome.
5. **v0.1.9 compatibility tests** consume checked-in Go-generated frames and
   objects. An optional live Go harness is an audit tool, not a normal CI
   dependency.

Every route marked supported in `protocol-v1.toml` must have at least one
client/server success test or a documented raw-RPC test when no public client
operation exists. Every documented status for that route must have a negative
test at the nearest practical layer. The route-coverage test compares this
registration to the actual Rust route table so the inventory cannot drift.

## 5. Harness structure

Refactor `foks-server-testkit` before adding a large matrix of tests:

```text
crates/foks-server-testkit/
  src/
    lib.rs                       # narrow public testkit exports
    environment.rs               # temporary root and all owned paths
    server.rs                    # in-process lifecycle and restart
    process.rs                   # real binary lifecycle and captured logs
    client.rs                    # durable public-client construction
    account.rs                   # account fixture and credential helpers
    pki.rs                       # test roots and certificate helpers
    clock.rs                     # deterministic clock for in-process tests
    faults.rs                    # sealed synchronization/failure controls
    assertions.rs                # protocol-level reusable assertions
  tests/
    isolation.rs
    listener_boundaries.rs
    conformance.rs               # route coverage plus focused nested tests
    conformance/
      registry.rs
      probe_and_pin.rs
      signup_and_user.rs
      authorization.rs
      kv_small.rs
      kv_large.rs
    kv_locks.rs
    restart_recovery.rs
    backup_restore.rs
    concurrency.rs
    capacity.rs
    binary_release.rs
```

The central fixture becomes an owned environment rather than a single running
server:

```text
TestEnvironment
  temp root
    server/database/foks-server.sqlite
    server/keys/
    server/backups/
    server/logs/
    clients/<client-id>/hard.sqlite
    clients/<client-id>/soft.sqlite
    clients/<client-id>/protected/
  probe, public, authenticated loopback endpoints
  probe roots and retained delegated trust material
  deterministic fixture identity/seed namespace
```

It provides two lifecycle implementations with the same externally observable
interface:

- `InProcessServer`: fast, deterministic, able to use an injected clock and
  sealed fault synchronization.
- `BinaryServer`: spawns an explicit binary path supplied by the release-test
  script, captures bounded logs, and supports graceful termination and forced
  termination for crash tests. The test does not recursively invoke Cargo or
  guess a shared target-directory layout.

The testkit chooses paths and loopback sockets. It does not expose constructors
that accept an arbitrary persistent root or non-loopback address. A test may
restart a server using the same owned paths, but may not detach those paths from
the environment lifetime.

`TestClient` reconstructs a new `FoksClient` around the same hard, soft, and
protected stores. The test must not keep an authenticated session alive across
a client restart and call that persistence.

Fault controls are Rust dependencies injected at composition boundaries, not
wire-accessible admin endpoints or global production "test mode" branches.
Actual-binary tests do not depend on those controls. They use operating-system
process termination only at externally meaningful boundaries.

## 6. Phased implementation

Each phase lands independently. The phase gate is its focused test suite plus
`tools/foks-server/check.sh`. After implementation, perform two explicit
self-reviews: first for code quality and test determinism, then for security,
isolation, and protocol compatibility.

### Phase 0: freeze the matrix and split the smoke test

Changes:

- Add an executable client/server scenario registry keyed by protocol and
  method. Each registry entry contains callable scenario functions plus
  metadata identifying public-client or raw-RPC coverage; it is not a TOML list
  of unverifiable test names.
- Add a route-coverage test within `conformance.rs` to compare that registry
  with both `protocol-v1.toml` and the registered route table.
- Split the current monolithic test into probe, identity, KV, authorization,
  and backup suites without expanding behavior.
- Reduce `public_probe.rs` to the concise release success scenario.
- Remove direct raw TLS probing from public-client success tests.

Likely files:

```text
crates/foks-server-testkit/tests/conformance.rs
crates/foks-server-testkit/tests/conformance/
  registry.rs
  probe_and_pin.rs
  signup_and_user.rs
  authorization.rs
  kv_small.rs
  kv_large.rs
crates/foks-server-testkit/tests/public_probe.rs
```

Tests and exit criteria:

- Every supported route has a named success test.
- Every unsupported advertised route has an exact-status test.
- Removing a registered scenario function or changing its signature fails to
  compile; route coverage fails if its metadata no longer covers the protocol
  matrix.
- The split suites preserve every assertion in the current smoke test.
- Each suite can run alone and in parallel with all other suites.

### Phase 1: lifecycle and durable-client harness

Changes:

- Introduce `TestEnvironment`, `InProcessServer`, `TestClient`, and account
  fixture helpers.
- Allow graceful shutdown and restart on the same test-owned database, key
  directories, and initially selected loopback ports. The signed service
  endpoints remain stable across an installation restart.
- Reconstruct the client from the same three durable stores.
- Replace sleeps with barriers, injected time, or bounded polling of a public
  readiness condition.
- Change existing-installation startup to load and validate the stored
  bootstrap identity instead of reconstructing genesis with a new wall-clock
  timestamp. Canonical name, configured endpoints, key generations, and stored
  signed material must still agree.
- Update the isolation gate so every integration test obtains server and client
  paths through the sealed environment, without requiring brittle source-text
  checks for one exact constructor spelling.

Tests and exit criteria:

- Empty start, graceful shutdown, and same-state restart all succeed.
- Restart with a different wall-clock value succeeds without changing any
  signed genesis byte or Merkle epoch.
- Restart preserves host ID, hostchain, delegated trust, Merkle head, user
  chain, PUK, and KV objects.
- Client reconstruction preserves host acceptance, user acceptance, and the
  ability to authenticate; it does not silently reprobe into a new trust root.
- Starting the installation on different ports is rejected rather than
  silently rewriting its signed service identity. Each test installation gets
  unique ephemeral ports initially and retains them for its restarts.
- Isolation tests prove every path is below the temporary root and every bound
  address is loopback.
- The isolation gate distinguishes pure matrix tests, which cannot open files
  or sockets, from stateful scenarios, whose callable signature requires a
  sealed temporary environment. It rejects direct database construction,
  arbitrary path constructors, and direct listener binding in stateful test
  modules.

### Phase 2: probe, trust, registration, and identity conformance

Changes:

- Expand focused public-client tests for first pin, unchanged pin, and valid
  Merkle advancement.
- Add raw-RPC negative cases for malformed names, reservation conflicts,
  reservation expiry, replay conflicts, and unsupported methods.
- Add account helpers that derive unique deterministic credentials from a test
  seed while never logging private seed bytes.

Tests and exit criteria:

- First probe inserts and a repeated identical probe is unchanged.
- A changed or incorrectly signed hostchain/public zone is rejected without
  modifying the durable pin.
- TLS hostname, bootstrap root, delegated root, and listener-role mismatches
  fail closed.
- Two accounts advance Merkle history and historical reads remain verifiable.
- Username reservation collision and expiry return their documented statuses.
- Replaying the exact signup request returns the committed result or a client
  reconciliation result; reusing its idempotency identity with changed bytes is
  rejected.
- Certificate issuance proves possession of the enrolled Ed25519 key.
- Full and unchanged user-chain loads, PUK recovery, and reconstructed-client
  authentication all work.
- A different user, device key, UID, certificate, or certificate issuer cannot
  be substituted in any valid combination.

### Phase 3: KV metadata and small-object conformance

Changes:

- Add reusable assertions over the client's decrypted projection rather than
  database rows.
- Add table-driven cases for root creation, directory creation, small files,
  symlinks, listings, and cache/version preconditions.
- Add raw-RPC cases only where the public client lacks an operation needed to
  induce a documented error.

Tests and exit criteria:

- Initial KV root creation is idempotent and scoped to its authenticated user.
- Empty, one-byte, and maximum-small-object payloads round trip exactly.
- Binary names/content at permitted boundaries round trip; invalid names and
  lengths return stable errors.
- Nested directories, symlinks, and listing pagination retain canonical order.
- Duplicate create, stale expected version, invalid parent, and cross-user
  object IDs fail without a partial mutation.
- A failed write leaves the prior client projection and subsequent sync
  coherent.
- Cache checks distinguish unchanged and stale namespaces without exposing
  another user's versions.

### Phase 4: large files, uploads, and locks

Changes:

- Introduce a deterministic byte-stream generator and digest assertion so
  boundary tests do not retain several full copies of large payloads.
- Expose server limits through sealed test composition so maximum-boundary tests
  can use small configured values rather than allocating production maxima.
- Add synchronization around upload stages for disconnect and restart tests.

Tests and exit criteria:

- Payloads immediately below, at, and above the small/large cutoff take the
  correct path and round trip exactly.
- Large-file bytes are read back through the public soft-state streaming API
  (`SoftStateStore::write_large_file`) and compared by length and digest; a
  projection's `large_file_size` alone is not a round-trip assertion.
- Empty, one-chunk, exact-chunk, chunk-plus-one, and multi-chunk files succeed.
- An unfinished upload is never visible in directory sync.
- Repeated identical chunks/finalization are idempotent; conflicting repeats
  are rejected.
- Disconnect or server termination before finalization leaves no visible file;
  retry after restart either resumes safely or starts cleanly according to the
  documented client contract.
- Finalization followed by a lost response cannot produce two visible files or
  leak upload quota.
- Per-file, per-chunk, and namespace quota boundaries return typed errors before
  excessive allocation.
- Lock contention, release, wrong token, wrong user, expiry, and restart
  semantics match the documented contract.

### Phase 5: retry, lost-response, restart, and backup recovery

Changes:

- Add sealed in-process fault synchronization at these externally relevant
  points: before durable mutation, after durable commit but before response,
  during response write, and between large-file chunks.
- Add real process restart tests without injected faults.
- Add restore helpers that copy only declared backup artifacts into a fresh
  test-owned server directory.
- Add or stabilize an operator-facing backup entry point shared by the running
  server library and the server CLI. It must not require a testkit dependency or
  direct SQLite copying by the test.

Tests and exit criteria:

- A failure before commit is retryable and leaves no mutation or receipt.
- A dropped response after commit is reconciled by a repeated request without a
  duplicate identity, Merkle epoch, KV object, chunk, or quota charge.
- Client and server may each restart independently at every completed step in
  the release scenario.
- Restoring the database with the matching key manifest and wrapped keys
  preserves host ID and works with the original client's existing pin and
  device credential.
- Missing, swapped, modified, or wrong-root-key key material fails startup with
  a bounded non-secret diagnostic.
- A truncated/corrupt backup or a database/key generation mismatch fails
  closed; the server does not bootstrap a replacement identity over it.
- Backup made during concurrent reads/writes is transactionally coherent.

### Phase 6: concurrency, scheduling, and capacity

Changes:

- Add barrier-start helpers for multiple independent clients.
- Add a small-limit server profile for deterministic queue, frame, namespace,
  object, upload, and storage-capacity testing.
- Record non-secret request and queue metrics in test diagnostics.

Tests and exit criteria:

- Concurrent independent reads proceed while writes are serialized.
- Independent-user writes both complete without leaking identity or KV state.
- Conflicting writes to the same directory produce one valid winner and a
  typed stale/conflict result, not a SQLite `BUSY` leak.
- The configured pending-write queue limit rejects excess work promptly with
  the documented rate-limit status and recovers after the queue drains.
- Slow or oversized frames do not monopolize an unbounded worker, memory, or
  database transaction.
- Reservation, object-count, namespace-byte, file-byte, chunk-count, and total
  storage limits are tested at limit minus one, limit, and limit plus one.
- Capacity rejection does not partially commit data and releases reservations,
  uploads, or locks according to policy.
- A second server process cannot open the same installation as an authoritative
  writer.
- A bounded soak test performs mixed account reads, KV reads/writes, uploads,
  and restarts while checking progress, memory bounds, error classes, and final
  state. It is nightly/manual unless its runtime becomes suitable for CI.

The purpose is to verify architectural promises, not publish a misleading
single throughput number. Release notes should report measurements with the
hardware, server limits, client count, object mix, payload sizes, durability
settings, and percentile latency. Correctness and bounded rejection remain CI
gates; performance regression thresholds are established only after repeated
measurements are stable.

### Phase 7: actual-binary release gate and v0.1.9 audit

Changes:

- Add `BinaryServer` and a script that builds once, passes the canonical binary
  path explicitly, and runs the release scenario against that executable.
- Add an opt-in compatibility audit against the Go v0.1.9 oracle or server when
  available. Normal CI continues to use checked-in fixtures and needs no Go
  toolchain or network.
- Add a repeat runner that reports the first failing seed and preserves only
  explicitly requested, non-secret diagnostics.

Likely files:

```text
crates/foks-server-testkit/src/process.rs
crates/foks-server-testkit/tests/binary_release.rs
tools/foks-server/test-client-server.sh
tools/foks-server/test-client-server-repeat.sh
tools/foks-v019-oracle/            # opt-in audit extensions only
```

Tests and exit criteria:

- The release scenario passes against a release-profile server binary.
- The supplied executable path is canonicalized and verified as a regular
  executable before launch; all data arguments still come exclusively from
  `TestEnvironment`.
- SIGTERM/graceful shutdown exits successfully; forced termination followed by
  restart preserves only committed operations and passes integrity checks.
- Startup readiness is observed through a bounded public probe, not log text or
  a fixed sleep.
- Logs contain request correlation and actionable error classes but no private
  seeds, root keys, decrypted content, bearer tokens, or full certificates.
- Repeated runs with varied deterministic seeds have no flakes. CI begins with
  a modest repeat count; nightly testing raises it.
- Supported Rust responses remain accepted by the v0.1.9 fixture/oracle tests.

## 7. CI and command structure

Keep the existing isolated gate as the required superset:

```sh
tools/foks-server/check.sh
```

Add focused commands for development and failure reproduction:

```sh
cargo test -p foks-server-testkit --test conformance probe_and_pin
cargo test -p foks-server-testkit --test conformance signup_and_user
cargo test -p foks-server-testkit --test conformance kv_small
cargo test -p foks-server-testkit --test conformance kv_large
cargo test -p foks-server-testkit --test restart_recovery
cargo test -p foks-server-testkit --test backup_restore
cargo test -p foks-server-testkit --test concurrency
cargo test -p foks-server-testkit --test capacity
tools/foks-server/test-client-server.sh
```

Suggested tiers:

- Pull request: route coverage, public-client conformance, negative protocol
  cases, one restart/restore path, and debug-binary smoke.
- Main branch: all isolated FOKS tests plus repeated process lifecycle tests.
- Nightly/manual: release binary, compatibility oracle, forced-termination
  matrix, capacity boundaries at larger profiles, and mixed-workload soak.

All commands print a deterministic reproduction seed on failure. Test timeouts
are per operation and per scenario, and timed-out subprocesses are terminated
and reaped. Logs are bounded so a failing peer cannot exhaust CI storage.

## 8. Review checklist for every phase

First self-review — code quality and determinism:

- Does the test fail at one identifiable boundary?
- Is setup expressed through reusable public testkit types rather than copied
  TLS, path, or account code?
- Are waits synchronized or bounded, with no fixed timing assumptions?
- Can the test run alone, concurrently, repeatedly, and with a printed seed?
- Is the assertion about public behavior rather than an implementation detail?
- Are errors precise enough to distinguish transport, verification, RPC,
  authorization, conflict, capacity, and storage failures?

Second self-review — security, isolation, and compatibility:

- Are all paths temporary, test-owned, and outside user/AKA data roots?
- Are all listeners loopback and ephemeral?
- Can any secret, decrypted content, certificate credential, or root key reach
  logs or panic formatting?
- Does a failure leave pins, identity state, Merkle state, KV state, and quota
  accounting unchanged or durably reconciled?
- Is the expected result consistent with `protocol-v1.toml` and v0.1.9 fixture
  behavior?
- Did the phase introduce a test-only production branch, remotely accessible
  fault hook, AKA dependency, or default-workspace coupling?

## 9. Completion definition

Client/server interoperability is complete for v1 when:

- every supported route has enforced success and failure coverage;
- public-client scenarios cover trust, signup, authentication, user/PUK sync,
  small and large KV, locks, and historical Merkle reads;
- client restart, server restart, lost responses, and matching-key backup
  restore preserve verified behavior;
- concurrency and configured capacity boundaries reject work predictably
  without leaking SQLite internals or partial state;
- the same core scenario passes in-process and against the real server binary;
- checked-in v0.1.9 compatibility fixtures remain green;
- all tests are isolated from AKA and user data; and
- `tools/foks-server/check.sh` passes without requiring Go, the network, or any
  non-FOKS workspace test.
