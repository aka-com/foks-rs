# FOKS v0.1.9 personal server v1 plan

Status: proposed implementation plan

Baseline: `github.com/foks-proj/go-foks` v0.1.9 and the FOKS-compatible Rust
client in this repository

## 1. Goal

Build a single-host, single-vhost FOKS server backed by one SQLite database.
Version 1 supports software Ed25519 account creation, certificate-authenticated
user-chain access, PUK retrieval, and personal encrypted KV storage. It speaks
the exact v0.1.9 wire protocol needed by `foks-client`; it is not intended to be
a full replacement for every upstream service or login mode.

The server is designed for a personal or small-team installation. One process
owns one authoritative SQLite writer. Reads can run concurrently, and the
storage layer can be partitioned or replaced later without changing protocol,
verification, or service APIs.

### V1 success scenario

From an empty installation:

1. Start the server and publish a verifiable probe response.
2. Create a software-device account with `foks-client`.
3. Issue a client certificate whose public key is bound to the enrolled device.
4. Load and verify the user chain and initial PUK parcel.
5. Create the personal KV root and write/read a small encrypted file.
6. Write/read a chunked encrypted file and list its containing directory.
7. Restart both processes and repeat the reads from durable state.
8. Advance and verify Merkle history across multiple account creations.

This exact scenario is the release-level end-to-end test. Lower-level tests must
make failures attributable before this test is introduced.

## 2. Scope

### Included

- One host and one virtual host.
- Software Ed25519 devices only.
- Host bootstrap, hostchain, public zone, delegated TLS, and signed Merkle roots.
- Username reservation and software signup.
- Client-certificate issuance and mutual TLS authentication.
- Full and incremental user-chain loading.
- PUK parcel lookup for the authenticated role.
- Current and historical Merkle roots, hidden tree locations, and inclusion and
  absence proofs.
- Personal KV root, directories, listings, small nodes, chunked files, cache
  checks, optimistic preconditions, and expiring locks.
- Process restart, online backup, bounded maintenance, metrics, and explicit
  capacity limits.

### Excluded

- Passphrase login, interactive KEX, hardware devices, and SSO.
- Device provisioning, revocation, backup recovery, and PUK rotation. The data
  model must not prevent these from being added, but v1 exposes no methods for
  them.
- Named or ad-hoc teams, PTKs, invitations, and federation.
- Realtime notifications beyond advertising the service slot required by the
  v0.1.9 public zone.
- Payments, email, web administration, multi-region replication, and horizontal
  write scaling.
- Importing an upstream PostgreSQL database or maintaining pre-release schema
  migrations.

Unsupported RPC methods return the exact stable v0.1.9 status selected for
unsupported methods; they must never panic, close the connection ambiguously,
or silently accept a partial mutation.

### Repository isolation

V1 is implemented and tested as a standalone FOKS subsystem. It must not depend
on any AKA crate. FOKS source lives in:

```text
crates/foks-*
foks-tauri/                       # the foks-desktop-app shell package
tools/foks-v019-oracle/
tools/foks-server/
tools/foks-client/
packaging/
crates/foks-snowpack/tests/fixtures/foks-v0.1.9/server/
```

The root manifest and lock files (`Cargo.toml`, `Cargo.lock`, `rust-project.json`,
`MODULE.bazel`, `MODULE.bazel.lock`) are shared build metadata, not an
authorization to rewrite another product's crate.

The enforced rule is a property of the Cargo graph, not of a branch's changed
files: no `foks-*` production or test crate may directly or transitively depend
on an `aka-*` package, every `foks-*` package must be defined under
`crates/foks-*` or `foks-tauri`, and every local path dependency it declares
must resolve there too. `tools/foks-server/check.sh` enforces all three, under
the default feature set and under `--all-features`.

This is deliberately not a whole-branch changed-path allowlist. One workspace
builds both products, so a branch carrying FOKS work also carries AKA and
shared-UI (`ui/kit`) work; rejecting those changes said nothing about whether
FOKS still stands alone, and blocked the integrated branch from being tested at
all.

## 3. Architectural rules

### Dependency direction

```text
foks-snowpack   foks-proto   foks-crypto   foks-verify
       \            |             |             /
                    foks-rpc
                        |
              foks-merkle-store
                        |
                foks-server-db
                        |
                   foks-server
                        |
              foks-server-testkit
                        |
          foks-client end-to-end tests
```

The diagram shows conceptual layering, not that every crate depends on every
crate above it. In particular:

- `foks-proto` owns protocol values and canonical encodings, but no sockets or
  SQL.
- `foks-crypto` owns cryptographic construction and signing, but no persistence.
- `foks-verify` owns pure trust and transition checks shared by client and
  server.
- `foks-rpc` owns MessagePack envelopes, dispatch keys, and status mapping.
- `foks-merkle-store` owns deterministic FOKS tree algorithms behind storage
  traits. It has an in-memory implementation for tests and no network code.
- `foks-server-db` owns every physical SQLite table and the authoritative unit
  of work. It adapts a SQLite transaction to the Merkle storage traits.
- `foks-server` owns TLS, authentication, request routing, authorization,
  scheduling, and composition. Handlers do not issue SQL directly.
- `foks-server-testkit` is dev-only composition code. Production crates must
  not depend on it.

### Atomicity boundaries

Identity mutations are serialized through the writer actor. The actor reads a
stable current head, builds and signs an immutable candidate, then applies it in
one short SQLite `BEGIN IMMEDIATE` transaction:

```text
decode and validate canonical request
verify signatures and construct a validated command
writer reads expected head and required immutable nodes
build and sign candidate root outside the SQLite transaction
BEGIN IMMEDIATE
recheck reservation, current root, expiry, and chain preconditions
insert exact signed link, projections, Merkle nodes, and request receipt
publish the signed root and request receipt
COMMIT
```

The mutation returns only after the signed root is durable. There is no v1
Merkle batcher, builder queue, or signer worker. This deliberately replaces the
multi-stage Go/PostgreSQL pipeline with one local commit path.

The transaction's expected-head check makes an externally modified database or
future competing writer reject the prepared candidate. The candidate can be
discarded safely because a signature is not authoritative until its root is
published in the committed head. This arrangement avoids holding a SQLite write
transaction across a platform-keystore or future remote-signer call.

KV mutations do **not** advance the global identity Merkle tree. They use their
own short SQLite transactions, path version vectors, namespace versions, and
request receipts. Chunk upload transactions are bounded per chunk; an upload
becomes visible only when its metadata is atomically finalized.

Crypto verification should run before the write transaction where possible,
but every state-dependent premise is checked again inside it. No network wait
or unbounded file operation occurs while the writer is held.

### Key boundary

The database contains public keys, certificates, signed objects, and opaque
encrypted user content. It does not contain plaintext host, Merkle, or CA
private keys. `foks-server` consumes a narrow `HostKeyProvider`/signer interface:

- deterministic in-memory keys in unit tests;
- a temporary protected test provider in integration tests;
- a standalone encrypted directory provider in production.

The production provider lives under `foks-server`, creates files atomically,
enforces owner-only permissions, wraps private material under an
operator-supplied root key, and records non-secret key-generation identifiers
in a backup manifest. It has no application-specific dependency.

Bootstrap creates or obtains named keys idempotently, then commits their public
identities and signed bootstrap objects to SQLite. Retrying bootstrap after a
crash must reuse the same named keys.

### SQLite boundary

Use a dedicated `foks-server.sqlite`; do not share it with another application
database. Open it with WAL, foreign keys, a busy timeout, defensive limits, and
one writer connection. `foks-server-db` exposes a synchronous, deterministic
API; the server runs writer calls on a dedicated blocking actor and uses
separate read-only connections for snapshots and proofs.

All identifiers, hashes, signed links, roots, proof nodes, HEPKs, boxes, KV
metadata, and encrypted chunks are stored as exact BLOBs. Integer microseconds
are used for time. Server limits are checked before conversions to SQLite's
signed integers.

## 4. Testing policy

Testing is part of each phase, not a final hardening pass.

1. **Exact fixture tests:** checked-in bytes generated or accepted by the Go
   v0.1.9 oracle. CI consumes fixtures and never needs Go or the network.
2. **Pure unit and property tests:** canonicalization, tree construction,
   authorization, state machines, overflow, and malformed input.
3. **Storage contract tests:** the same behavioral suite runs against the
   in-memory Merkle store and SQLite adapter where applicable.
4. **Transaction tests:** temporary databases, forced errors at commit stages,
   concurrent readers, retries, and restart/reopen.
5. **Transport tests:** real loopback TLS and exact RPC frames; no handler is
   tested only by calling it as a Rust function.
6. **Black-box tests:** the public `foks-client` API talks to a spawned server
   with no internal access.
7. **Differential tests:** an opt-in Go v0.1.9 harness decodes/verifies Rust
   responses and exercises generated clients for the supported methods.
8. **Abuse and capacity tests:** frame limits, reservation floods, stale writes,
   lock expiry, chunk quotas, database-full behavior, slow peers, and crash
   recovery.

Tests use injected clocks, entropy, signers, and failure points. Production code
must not branch on a global "test mode." Exact secrets are allowed only in test
fixtures and must be clearly non-production.

Every process-level integration test uses the testkit's sealed
`IsolatedTestServer` constructor. It owns temporary database and key directories
for the lifetime of the test and binds only ephemeral loopback sockets. Tests
must never derive paths from a user home directory, a production environment
variable, or an AKA data root. The testkit exposes no constructor that accepts
an arbitrary persistent root or non-loopback listen address.

The normal gate for every phase is `tools/foks-server/check.sh`. It runs only
explicit FOKS package selectors for formatting, clippy, unit tests, and
integration tests. It must not invoke a bare Cargo command, `--workspace`,
`just test`, or `bazel test //...`. Its final test set is equivalent to:

```sh
cargo test \
  -p foks-snowpack \
  -p foks-proto \
  -p foks-crypto \
  -p foks-verify \
  -p foks-rpc \
  -p foks-merkle-store \
  -p foks-server-db \
  -p foks-server \
  -p foks-server-testkit \
  -p foks-client
```

Before all crates exist, the script selects only the FOKS packages introduced
through the current phase. It also inspects Cargo metadata and fails if any
selected package's transitive graph contains an `aka-*` package, if a production
crate depends on `foks-server-testkit`, or if implementation changes escape the
repository-isolation allowlist. The Go oracle is opt-in and runs only when adding
or auditing fixtures; ordinary CI consumes its checked-in output without Go or
network access.

Cargo manifests and `Cargo.lock` remain the source for Bazel's crate universe;
`MODULE.bazel` changes only if a new dependency needs an annotation. A
repository-wide build may run independently in the repository's normal CI, but
it is not a development or acceptance gate for this server. Regenerate
`rust-project.json` after adding crates.

## 5. Implementation phases

Each phase should land as a reviewable vertical or foundational increment. Do
not begin a phase while the preceding phase's tests are flaky or depend on test
ordering.

When a crate is introduced, add it to the root workspace members and workspace
dependencies in `Cargo.toml`, then update `Cargo.lock` and regenerate
`rust-project.json`. Leave the root `default-members` list unchanged; all new
server crates are exercised only through explicit package selectors. New
third-party dependencies are declared once under workspace dependencies and
consumed with `workspace = true`. The repository's Bazel crate universe derives
from the Cargo manifests and lockfile, so the FOKS crates do not gain
hand-written `BUILD.bazel` files; edit `MODULE.bazel` only when a dependency
requires a feature or build annotation. Phase 0 introduces the minimal
`foks-server` scaffold for the X.509 feasibility gate; phase 2 introduces
`foks-merkle-store`; phase 3 introduces `foks-server-db`; phase 4 completes
`foks-server` and introduces the dev-only `foks-server-testkit`.

`foks-server-testkit` must declare `publish = false` and must never be added to
workspace `default-members`. Production manifests may not depend on it.

### Phase 0: freeze the supported protocol contract

Produce an executable method matrix and response fixtures before server
behavior exists. Record, for each method, its listener, authentication rule,
request type, result type, status mapping, size limit, and whether it is v1 or
explicitly unsupported.

Files introduced or changed:

```text
crates/foks-server/
  Cargo.toml                      # minimal scaffold; not a default member
  PLAN.md                         # this document
  protocol-v1.toml                # machine-readable supported-method matrix
  src/
    lib.rs                        # empty public shell until phase 1
  tests/
    protocol_schema.rs            # matrix schema and unique-key validation
    x509_existing_key.rs          # certificate for externally held Ed25519 key

tools/foks-v019-oracle/
  server_fixture.go               # response/proof fixture construction
  server_rpc_fixture_test.go      # Go decodes checked Rust-facing frames
  README.md

tools/foks-server/
  check.sh                        # isolated format/lint/test/boundary gate

crates/foks-snowpack/tests/fixtures/foks-v0.1.9/server/
  manifest.json
  rpc/                            # call and success/error response frames
  merkle/                         # small deterministic tree transcripts
  pki/                            # public cert fixtures only
```

Tests and exit criteria:

- The manifest hashes every fixture and identifies the upstream v0.1.9 source.
- Go-generated clients decode every planned success response and selected
  errors.
- The method matrix covers Probe, Reg, User, MerkleQuery, KVStore, and the
  advertised-but-unsupported Realtime slot.
- A schema test parses `protocol-v1.toml`, rejects duplicate protocol/method
  keys, and requires listener, authentication, result, status, and limit fields
  for every entry.
- The selected pure-Rust X.509 library issues a CA-signed certificate containing
  an existing Ed25519 device public key without receiving that device's private
  key. The test parses and verifies the certificate, asserts exact SPKI bytes,
  and completes an mTLS proof-of-possession handshake using the separately held
  private key.
- The X.509 library remains a dev-dependency until its feasibility test passes;
  phase 4 promotes it to a production dependency.
- No server listener, database, or production request handler is added in this
  phase.

### Phase 1: server-side RPC and protocol construction

Add strict request decoding, bounded dispatch, and response/error encoding to
the existing protocol crates. Keep the existing client encoders stable; avoid a
large cosmetic rewrite of `foks-rpc/src/lib.rs` while adding the server path.

Files introduced or changed:

```text
crates/foks-rpc/src/
  lib.rs                          # public exports; existing client calls remain
  envelope.rs                     # shared bounded frame/envelope parser
  server.rs                       # DecodedCall and request dispatcher keys
  response.rs                     # success and v0.1.9 status encoders
  arguments/
    mod.rs
    probe.rs
    registration.rs
    user.rs
    merkle.rs
    kv.rs
crates/foks-rpc/tests/
  server_requests.rs
  server_responses.rs
  server_rejections.rs

crates/foks-proto/src/
  host.rs                         # server-safe constructors/encoders
  identity/merkle.rs              # root/history/proof construction APIs
  identity/user.rs                # response construction APIs
  kv.rs                           # KV result construction APIs

crates/foks-crypto/src/
  server.rs                       # typed host/Merkle signing helpers

crates/foks-verify/src/
  server.rs                       # pure signup/authorization entry points

crates/foks-server/src/
  rpc/
    mod.rs
    routes.rs                     # declarative registered route metadata
crates/foks-server/tests/
  protocol_matrix.rs              # TOML and Rust registry are exactly equal
```

Rules:

- Preserve exact canonical argument bytes when a signature or hash covers them.
- Reject duplicate map keys, wrong wrapper versions, trailing values, excessive
  depth, noncanonical Snowpack, unknown variants, and oversized frames before
  dispatch.
- Use typed method arguments after decoding; handlers do not index raw arrays.
- Status responses are typed and centrally mapped. Handlers cannot invent
  numeric status codes.
- `protocol-v1.toml` and the registered Rust route table must match in both
  directions, including unsupported routes, listener class, authentication,
  request/result type, status mapping, and size limit. The comparison test fails
  if either side adds, removes, or changes an entry without updating the other.

Tests and exit criteria:

- Every v1 request fixture decodes to the expected typed value and re-encodes to
  the official byte sequence where the protocol requires canonical bytes.
- Every planned response is accepted by the Go oracle fixture tests.
- The enforced protocol-matrix comparison passes; the TOML file cannot become
  stale documentation or silently advertise an unregistered handler.
- Truncation and mutation tests fail closed without panics or large allocation.
- Property tests cover arbitrary frame boundaries and request sequences.
- The isolated package test includes `foks-rpc`, `foks-proto`, `foks-crypto`,
  `foks-verify`, and `foks-server` and passes.

### Phase 2: deterministic Merkle engine

Implement the v0.1.9 authenticated tree independently of SQLite. This is the
highest-risk compatibility component and should be proven before server or DB
code depends on it.

Files introduced:

```text
crates/foks-merkle-store/
  Cargo.toml
  README.md
  src/
    lib.rs
    error.rs
    hash.rs                       # v0.1.9 node/leaf/domain hashes
    key.rs                        # tree keys and hidden locations
    node.rs                       # canonical persistent node representation
    store.rs                      # NodeReader/NodeWriter traits
    memory.rs                     # deterministic test implementation
    tree.rs                       # insert/replace and root calculation
    proof.rs                      # presence and absence proof generation
    history.rs                    # epochs and skip/backpointer requirements
    commit.rs                     # immutable nodes and root produced by update
  tests/
    official_vectors.rs
    properties.rs
    history.rs
    corruption.rs
```

The engine accepts an existing root and a set of validated leaf changes and
returns an immutable commit description: newly addressed nodes, the next tree
root, and the proof/history material required to publish it. It does not sign
roots and does not know about users, SQL, TLS, or RPC.

Tests and exit criteria:

- Empty, singleton, branching, prefix-collision, replacement, presence, and
  absence cases match Go-generated v0.1.9 vectors.
- Rust-generated proofs verify in `foks-verify` and in the Go oracle.
- Insertion order does not change the canonical root for the same leaf set.
- Reopening from only persisted nodes produces identical proofs.
- Missing, swapped, malformed, and hash-mismatched nodes fail closed.
- Backpointer sequences and historical response construction match v0.1.9 for
  dense early epochs, powers of two, and large epochs.
- `cargo test -p foks-merkle-store` passes under Miri-compatible safe Rust where
  practical; the crate forbids unsafe code.

### Phase 3: authoritative SQLite store

Introduce the complete v1 physical schema and synchronous storage API. The
schema is modularized by domain but initialized as one v1 schema. Because the
product has not shipped, incompatible schema changes during development replace
v1 rather than adding migration compatibility.

Files introduced:

```text
crates/foks-server-db/
  Cargo.toml
  README.md
  src/
    lib.rs
    error.rs
    config.rs                      # limits, pragmas, open modes
    connection.rs                  # writer/read connection setup
    schema.rs                      # application ID/version and initializer
    transaction.rs                 # unit-of-work and failure boundaries
    clock.rs                       # injectable time interface
    host.rs                        # public host state and bootstrap records
    names.rs                       # reservations and expiry semantics
    identity.rs                    # users, devices, chains, HEPKs, boxes
    merkle.rs                      # SQLite adapter for Merkle traits
    certificates.rs               # issuance records, not CA private keys
    receipts.rs                    # idempotency and lost-response replay
    read.rs                        # immutable read snapshots
    schema/
      core.sql
      identity.sql
      merkle.sql
      certificates.sql
      receipts.sql
  tests/
    common/mod.rs
    schema.rs
    reservations.rs
    identity_transactions.rs
    merkle_transactions.rs
    receipts.rs
    concurrency.rs
    restart.rs
```

Core tables:

- host metadata, public hostchain links, service endpoints, and key references;
- names and expiring reservation tokens;
- users, active devices, exact chain links, chain heads, tree locations, HEPKs,
  shared keys, parcels, and seed-chain boxes;
- content-addressed Merkle nodes, leaves, roots, signed root blobs, root heads,
  and historical backpointers;
- issued certificate serials and device bindings;
- request hashes, committed response blobs, creation time, and expiry.

Tests and exit criteria:

- Schema constraints reject malformed lengths, negative values, dangling
  references, duplicate chain positions, and conflicting current heads.
- A forced error after each mutation stage leaves no partial user, name, node,
  root, certificate, or receipt visible.
- Repeating a request hash returns the stored response; reusing an idempotency
  identity with different bytes is rejected.
- Expired reservations are unusable even before cleanup runs.
- Readers observe the old or new committed root, never a mixed state.
- Database reopen and SQLite online backup preserve all signed bytes exactly.
- Foreign keys and expected pragmas are asserted on every connection.
- `cargo test -p foks-server-db` passes with temporary on-disk databases, not
  only `:memory:` databases.

### Phase 4: host bootstrap, TLS, and server shell

Build a process that can start, bootstrap one host, advertise all required
service slots, and answer probe and Merkle reads. Use three logical listeners in
one process so authentication policy is visible at the socket boundary:

- probe listener: publicly trusted server certificate and `Probe` only;
- public service listener: host-delegated TLS, no client certificate, `Reg` and
  `MerkleQuery`;
- authenticated listener: host-delegated TLS plus required client certificate,
  `User` and `KVStore`.

Multiple service types may advertise the same public or authenticated endpoint.
Realtime is advertised because the current public-zone verifier expects it, but
its RPC methods return the documented unsupported status.

Files introduced:

```text
crates/foks-server/
  Cargo.toml
  README.md
  src/
    lib.rs
    error.rs
    config.rs
    clock.rs
    entropy.rs
    keys/
      mod.rs                       # HostKeyProvider and signer handles
      memory.rs                    # tests only or feature-gated
      directory.rs                 # standalone wrapped-key persistence
      manifest.rs                  # public generation and backup binding
    pki/
      mod.rs
      server_cert.rs
      client_ca.rs
      peer.rs                      # cert public-key extraction/binding
    host/
      mod.rs
      bootstrap.rs
      hostchain.rs
      public_zone.rs
    net/
      mod.rs
      listener.rs
      probe.rs
      public_services.rs
      authenticated.rs
      session.rs
    rpc/
      mod.rs
      router.rs
      context.rs
      limits.rs
    services/
      mod.rs
      probe.rs
      merkle.rs
      unsupported.rs
    writer.rs                      # dedicated blocking SQLite actor
    maintenance.rs
  src/bin/foks-server.rs           # standalone development/operations binary
  tests/
    bootstrap.rs
    probe_tls.rs
    delegated_tls.rs
    rpc_limits.rs
    restart.rs
```

Introduce a dev-only composition crate once more than one integration suite
needs a real process:

```text
crates/foks-server-testkit/
  Cargo.toml                       # publish = false; never a default member
  src/
    lib.rs
    keys.rs                        # deterministic test key provider
    certs.rs                       # test probe CA and client trust setup
    process.rs                     # sealed IsolatedTestServer constructor
    config.rs                      # owned TempDirs and loopback port zero only
  tests/
    public_probe.rs                # uses foks-client as a dev-dependency
    isolation.rs                   # proves path and socket confinement
```

`foks-server-testkit` has a normal dependency on `foks-server` and a
dev-dependency on `foks-client`; production server crates never depend on the
testkit. This avoids a Cargo dependency cycle while keeping cross-crate tests at
a real process boundary. Tests inside `foks-server` use local `tests/common`
helpers for lower-level server composition.

Promote the pure-Rust X.509 library proven by phase 0 from a dev-dependency to a
production dependency and move the proven construction behind `pki`. Phase 4
must not substitute a different certificate path without repeating the phase 0
SPKI and proof-of-possession tests.

Tests and exit criteria:

- First bootstrap and idempotent restart produce byte-identical host identity
  and monotonic roots.
- `foks-client` verifies the real loopback probe, delegated TLS chain, service
  map, and initial signed Merkle root. Merkle reads remain public because account
  preparation needs the current root before a device certificate exists.
- Public listeners reject private methods; the authenticated listener rejects
  absent, foreign-CA, expired, wrong-host, and unbound client certificates.
- Oversized frames, idle connections, and excessive requests are bounded.
- `isolation.rs` asserts that every process-level test server's database, key
  directory, backup directory, and logs are descendants of test-owned temporary
  directories, that no path resolves beneath a user home or known AKA data
  root, and that every listener address is loopback with an ephemeral port.
- Process-level integration tests can obtain a server only through
  `IsolatedTestServer`; the isolated check rejects direct persistent-root or
  non-loopback construction in the testkit integration-test tree.
- Graceful shutdown stops accepts, drains bounded in-flight work, checkpoints
  if configured, and does not abandon a published mutation.
- `cargo test -p foks-server -p foks-server-testkit` passes.

### Phase 5: registration, certificate issuance, and user reads

Implement the first complete identity mutation. The server validates the exact
eldest link and supplied commitments/boxes, derives the authoritative UID,
consumes the reservation, stores the user state, advances the Merkle tree, and
returns only after the signed root is published.

Files introduced or expanded:

```text
crates/foks-server/src/
  auth/
    mod.rs
    principal.rs                  # authenticated host/user/device identity
    certificate.rs                # mTLS certificate to active device
    authorization.rs              # method-level policy
  services/
    registration.rs               # reserve, signup, certificate chain
    user.rs                       # load chain, get PUK for role
    merkle.rs                     # current and historical responses
  identity/
    mod.rs
    signup.rs                     # validation -> ValidatedSignup command
    chain.rs                      # full/incremental response assembly
    parcels.rs                    # exact PUK parcel selection

crates/foks-server-db/src/
  signup.rs                       # atomic signup unit of work

crates/foks-server/tests/
  signup.rs
  signup_rejections.rs
  client_cert.rs
  user_chain.rs
  merkle_history.rs
  lost_response.rs
```

Security rules:

- The username reservation token is random, single-use, expiring, and bound to
  the normalized name and sequence.
- V1 uses an explicit local open-signup policy: it accepts the protocol's empty
  invite variant and no SSO assertion, and rejects unsupported invite or SSO
  variants. Email is not verified or used as an authorization identity.
- The eldest link must cite the currently published tree root.
- Commitments, device identity, PUK identity, HEPKs, boxes, hidden next
  locations, and claimed UID are cross-checked rather than trusted separately.
- A certificate is authorized by both its CA chain and its public-key binding to
  an active device in the requested host. A valid CA signature alone is not an
  account credential.
- Certificate lookup is intentionally public, matching the client and v0.1.9
  flow. The server returns a persisted, bounded-lifetime certificate for an
  enrolled public key and rate-limits lookup; only possession of the matching
  private key makes it useful in the subsequent mTLS handshake.
- Incremental chain loading returns the exact link/location shape expected by
  v0.1.9, including `links + 1` hidden locations.

Tests and exit criteria:

- The integration harness submits the exact client-generated signup frame, then
  uses public `foks-client` certificate-fetch and authentication APIs against a
  freshly spawned server. The higher-level `create_software_account` completion
  gate belongs to phase 6 because that API also creates the personal KV root.
- The returned chain and PUK verify/unbox through the existing client verifier,
  without a test bypass.
- Duplicate names, expired or reused reservations, stale roots, bad signatures,
  altered boxes, wrong HEPKs, inconsistent UIDs, and conflicting retries are
  rejected atomically.
- A simulated dropped response followed by the identical request returns the
  committed result without creating another root.
- Multiple signups create valid historical proofs and backpointer transcripts.
- Full and no-op incremental chain loads both pass official-shape fixtures.

### Phase 6: personal KV metadata and small nodes

Add an authenticated user namespace, root creation, directories, list/get,
small files or symlinks, cache checks, and optimistic write preconditions. The
server treats file contents and names as opaque encrypted protocol objects; it
validates structure, authorization, sizes, and version transitions, not
plaintext semantics.

Files introduced:

```text
crates/foks-server-db/src/
  kv/
    mod.rs
    root.rs
    directory.rs
    node.rs
    versions.rs
    quota.rs
    receipts.rs
  schema/
    kv.sql
crates/foks-server-db/tests/
  kv_root.rs
  kv_directory.rs
  kv_preconditions.rs
  kv_quota.rs

crates/foks-server/src/
  services/kv.rs
  kv/
    mod.rs
    auth.rs
    read.rs
    write.rs
    limits.rs
crates/foks-server/tests/
  kv_small.rs
  kv_stale_cache.rs
  kv_authorization.rs
```

The physical model includes namespace roots, immutable directory versions,
directory entries, small encrypted nodes, node references, usage counters, and
path version vectors. Foreign keys and transactional reference updates prevent
visible dangling entries. Namespace use is charged from stored encoded bytes,
not caller-provided plaintext sizes.

Tests and exit criteria:

- The public `foks-client::create_software_account` flow completes end to end,
  including its automatic personal KV-root creation. The client then creates
  nested directories, writes and reads a small file, lists entries, and observes
  cache hits/misses.
- Lost-response retries are idempotent; stale preconditions produce the exact
  retry/cache status expected by the client.
- Cross-user reads and writes fail even with guessed node IDs.
- Concurrent writes to one directory produce one winner and an explicit stale
  loser; writes to independent directories remain correct.
- Reference counts and usage counters equal a full recomputation after every
  property-generated operation sequence.
- Quota, encoded-size, count, and integer-boundary checks occur before writes.

### Phase 7: chunked files and locks

Complete the KV surface used by the client with upload initialization, bounded
chunk writes, final metadata visibility, encrypted chunk reads, and expiring
directory locks.

Files introduced:

```text
crates/foks-server-db/src/kv/
  upload.rs
  chunk.rs
  lock.rs
  gc.rs
crates/foks-server-db/tests/
  kv_upload.rs
  kv_locks.rs
  kv_gc.rs

crates/foks-server/src/kv/
  upload.rs
  lock.rs
crates/foks-server/tests/
  kv_large.rs
  kv_upload_recovery.rs
  kv_lock_expiry.rs
```

An upload session records expected metadata and quota reservation. Each chunk is
idempotent by upload ID and index. Directory publication occurs only after all
expected chunks and keys are durable. Abandoned uploads are never visible and
can be reclaimed after expiry. Correctness cannot depend on the cleanup job
running promptly.

Tests and exit criteria:

- The client crosses the small-file threshold, uploads multiple chunks, reads
  them in order, and verifies the resulting encrypted object.
- Duplicate chunks with identical bytes succeed idempotently; conflicting
  bytes, indexes, ownership, sizes, or completed sessions fail.
- Kill/restart at every upload stage yields either a resumable hidden upload or
  a complete visible file, never a partial visible file.
- Locks enforce owner/token/expiry semantics using an injected clock. Expired
  locks cease blocking immediately even before deletion.
- Boundary tests cover the client's 1 GiB per-file maximum without allocating a
  1 GiB test buffer; a smaller configurable test limit exercises the same path.

### Phase 8: operations, capacity, and release hardening

Make the service operable without adding distributed architecture. Maintenance
is a bounded in-process scheduler, not a durable general-purpose job system.
Correctness is enforced in foreground queries; jobs only reclaim or optimize.

Files introduced or expanded:

```text
crates/foks-server/src/
  operations/
    mod.rs
    health.rs
    metrics.rs
    backup.rs
    checkpoint.rs
    maintenance.rs
    shutdown.rs
  limits.rs
  config.rs
crates/foks-server/tests/
  backup_restore.rs
  disk_full.rs
  graceful_shutdown.rs
  maintenance.rs
  load.rs                         # ignored/manual benchmark gate

crates/foks-server-db/src/
  backup.rs
  integrity.rs
  maintenance.rs
crates/foks-server-db/tests/
  backup.rs
  integrity.rs
```

Scheduled work is limited to reservation/receipt/lock expiry cleanup, abandoned
upload reclamation, WAL checkpointing, integrity sampling, and backup triggers.
Tasks use leases only if multiple server processes are later allowed to open the
same database; v1 rejects a second writer process instead.

Operational requirements:

- Explicit maximum frame, request, reservation, user, directory-entry, chunk,
  upload, file, namespace, and total database sizes.
- Default per-file compatibility ceiling of 1 GiB, with a lower configurable
  deployment quota if desired.
- SQLite online backup paired with a key-provider snapshot/version manifest.
- Restore refuses an absent or mismatched signing-key generation.
- WAL size and checkpoint duration metrics; bounded busy and shutdown timeouts.
- Structured audit records for account creation, certificate issuance, rejected
  authorization, quota events, backup, and recovery. Never log signed secret
  material, tokens, encrypted chunk bytes, or private keys.
- A documented procedure for `quick_check`, full integrity verification,
  backup, restore, key rotation preparation, and database growth.

Release tests and exit criteria:

- The v1 success scenario passes repeatedly under a real process boundary.
- Backup to a second directory, restore with the matching protected keys, and
  reconnect from an existing client all succeed without repinning.
- Restore with old/mismatched keys, modified nodes, a truncated database, or a
  missing chunk fails explicitly.
- Load tests publish observed hardware, PRAGMAs, durability mode, dataset, p50,
  p95, p99, writer saturation, reader latency, WAL growth, and checkpoint cost.
  No throughput number becomes a product claim without this record.
- A sustained reader load plus serialized signups/KV writes has no unbounded
  queue, starvation, or `SQLITE_BUSY` leakage to clients.
- `tools/foks-server/check.sh` passes from a clean checkout without building or
  testing an AKA crate, desktop target, UI, or npm package.

## 6. V1 RPC surface

The exact positions remain defined in `foks-rpc`; this table expresses product
support and authentication.

| Protocol | Method | Listener | Auth | V1 |
| --- | --- | --- | --- | --- |
| Probe | probe | probe | public | yes |
| Reg | selectVHost | public services | delegated TLS | yes |
| Reg | reserveUsername | public services | delegated TLS | yes |
| Reg | signup | public services | reservation + signatures | yes |
| Reg | getClientCertChain | public services | public; possession proven by later mTLS | yes |
| MerkleQuery | selectVHost | public services | delegated TLS | yes |
| MerkleQuery | getCurrentRoot | public services | delegated TLS | yes |
| MerkleQuery | getHistoricalRoots | public services | delegated TLS | yes |
| User | loadUserChain | authenticated | active device mTLS | yes |
| User | getPukForRole | authenticated | active device mTLS | yes |
| User | getHostConfig | authenticated | active device mTLS | explicit unsupported |
| KVStore | selectVHost | authenticated | active device mTLS | yes |
| KVStore | putRoot/getRoot | authenticated | active device mTLS | yes |
| KVStore | mkdir/put/putSmall | authenticated | active device mTLS | yes |
| KVStore | uploadInit/uploadChunk | authenticated | active device mTLS | yes |
| KVStore | getNode/getDir/list/getChunk | authenticated | active device mTLS | yes |
| KVStore | cacheCheck | authenticated | active device mTLS | yes |
| KVStore | lockAcquire/lockRelease | authenticated | active device mTLS | yes |
| Realtime | all | authenticated | active device mTLS | explicit unsupported |

If `getHostConfig` is added after v1, it must return an honest capability set
and must not claim team, federation, invite, or login modes that are absent.

## 7. Capacity model and future partitioning

V1 has one global writer because both authoritative SQLite state and global
identity Merkle publication need a total order. Reads, proof generation, and
chunk reads use snapshots and may be concurrent. KV writes do not touch the
global Merkle tree, but still pass through the same writer actor in v1 to keep
operational behavior predictable.

Measure separately:

- signup/root-signing mutations per second;
- small KV metadata mutations per second;
- chunk ingest and read bytes per second;
- proof-generation latency by tree size;
- database/WAL growth per account, link, small node, and chunk;
- checkpoint and backup time by database size.

The first partitioning boundary is KV chunk data: move content-addressed opaque
chunks to a local blob store while retaining metadata and publication in
SQLite. The second is independent KV namespaces. Identity/name mutations and
the global Merkle log remain on a single leader unless a consensus protocol is
introduced. Public Rust traits and commands should preserve these boundaries,
but v1 must not add distributed abstractions with no second implementation.

## 8. Review gates

Every phase is reviewed against five questions:

1. Does untrusted input become typed and verified before it reaches authority?
2. Can a crash expose a state that no signed root, chain head, or KV version
   describes?
3. Can the behavior be tested without a live upstream server or production
   key store?
4. Does the new public API preserve a future storage boundary without exposing
   SQLite details unnecessarily?
5. Do the Cargo graph, package locations, test roots, and listen addresses
   remain inside the standalone FOKS isolation boundary?

Phases should also receive an additional intermediate review for recommended
changes, fixes, refactors to improve quality to be idiomatic Rust, to avoid
unnecessary indirection, macros, or other code smells that wouldn't be
expected in single-host server system code without complex concurrency.

Security-sensitive phases 2 through 7 receive a final review focused only on
canonical encoding, authorization, transaction boundaries, replay, expiry,
integer limits, and secret handling. Fixture parity is evidence of protocol
compatibility, not a substitute for adversarial tests.

## 9. Definition of v1 done

V1 is done when:

- the supported method matrix is accurate and enforced;
- the Rust client completes the release scenario with no special test hooks;
- Rust-generated roots, history, proofs, RPC results, and certificates are
  accepted by independent v0.1.9-compatible checks;
- signup and KV writes are idempotent across lost responses and restarts;
- backups restore with existing client trust state;
- quotas and resource bounds fail explicitly before database corruption or
  process exhaustion;
- unsupported upstream functionality is documented and returns stable errors;
- all private signing material is behind the standalone production key-provider
  boundary;
- no FOKS dependency graph contains an `aka-*` package and no implementation
  change touches an AKA crate;
- every process-level integration test proves temporary-path and loopback-socket
  confinement;
- the isolated FOKS package, lint, formatting, route-matrix, and boundary gates
  pass; and
- measured capacity and recovery procedures are recorded in the server README.
