# FOKS v0.1.9 small-team server v1 plan

Status: proposed implementation plan

Baseline: the completed standalone personal server in this repository,
`github.com/foks-proj/go-foks` v0.1.9, and the existing public Rust
`foks-client` provisioning, recovery, team, PTK, and team-KV APIs

## 1. Goal

Extend the single-host SQLite server from a personal account and KV server into
a small-team server. The second slice adds:

- software-device provisioning, revocation, and PUK rotation;
- backup-key enrollment, account lookup, and software-device recovery;
- named and ad-hoc team creation;
- local named-team membership additions, role changes, and removals;
- PTK publication, delivery, history, and mandatory rotation after lost
  visibility;
- authenticated team-chain loading and short-lived team capabilities; and
- encrypted team KV using the existing KV wire protocol.

The server continues to use one process, one host/vhost, one SQLite database,
and one authoritative writer. It implements the v0.1.9 methods used by the
current Rust client rather than attempting to reproduce every upstream team,
federation, invite, or login service.

### V1 success scenario

From an empty isolated installation:

1. Create two software-device users and retain independent durable clients.
2. Provision a second device, authenticate it, revoke it, rotate a PUK prefix,
   and prove the revoked credential no longer works.
3. Enroll an owner backup key, reconstruct the UID through signed public
   lookup, authenticate the backup credential, and recover a replacement
   software device.
4. Create an ad-hoc team, load and verify its chain, unbox the visible PTKs,
   and round-trip team KV.
5. Reserve a team name and create a named team with the first user as owner.
6. Add the second local user, prove its role-limited PTK visibility and team KV
   access, then demote and remove it while rotating every PTK whose visibility
   it lost.
7. Prove stale view/admin tokens and removed-member credentials cannot load the
   team, retrieve removal material, or access team KV.
8. Restart clients and server, reauthenticate from durable state, and verify
   user chains, team chains, PTK histories, and both personal and team KV.
9. Back up and restore the installation, then repeat the reads with the
   original host pins and credentials.

This scenario must pass against the release-profile `foks-server` executable.
Focused in-process suites establish each invariant before it enters the
release scenario.

## 2. Scope and compatibility boundary

### Included

- Everything already supported by the personal-server slice.
- Software Ed25519 device provision and revocation.
- User PUK rotation through the v0.1.9 `revokeDevice` mutation shape.
- Backup-key enrollment, signed UID lookup, backup authentication, PUK
  recovery, and replacement software-device provision.
- Honest standalone host configuration with open local user/team viewership,
  as required by the current client's team flows.
- Single-owner ad-hoc team creation. Ad-hoc teams remain immutable after their
  founding link, matching the current verifier and client.
- Named-team reservation and creation.
- Local-user additions to named teams.
- Named-team member demotion, promotion within the actor's authority, removal,
  and exact PTK rotation/key-flood semantics implemented by `foks-client`.
- Full and incremental team-chain loading, including historical Merkle roots,
  names, disclosed roster state, PTK parcels, and PTK seed chains.
- Short-lived team-view tokens for team loading and team KV.
- Short-lived TeamAdmin bearer tokens for removal-key-box retrieval.
- Personal and team KV namespaces using the same root, directory, node,
  upload, cache, listing, and lock methods.
- Backup/restore, restart, maintenance, metrics, quotas, queue pressure, and
  release-binary testing for the expanded state.

The server accepts protocol objects produced by the client's Yubi-backed team
actor variants when the corresponding parent/subkey is already enrolled and
verifiable. It does not add physical YubiKey discovery, PC/SC, PIV, or hardware
enrollment to the server; those remain client concerns and fixture-only tests.

### Excluded

- Remote users, remote teams, federation, cross-host grants, remote view
  tokens, and team cycles.
- Team invites, inboxes, join requests, TeamMember and TeamGuest services,
  membership subchains, and remote removal delivery.
- Team certificates and team TLS identity.
- Passphrase login, KEX provisioning, SSO, bots, web administration, and
  interactive account recovery.
- Realtime notifications. Clients reconcile by loading signed chains and
  polling within bounded operations, as they do today.
- Multiple vhosts, multiple authoritative writers, replication, sharding, or
  automatic import/synchronization of an upstream PostgreSQL installation.
- Schema migrations for pre-release databases. The project has not shipped;
  incompatible physical-schema improvements replace the v1 schema directly.

Unsupported upstream methods remain explicitly registered where necessary and
return the stable v0.1.9 unsupported status. `HostConfig` must not imply that
federation, invitation, passphrase, or remote-team features exist.

TeamLoader and TeamAdmin do not add public-zone service slots in v0.1.9. They
are additional protocols served on the existing authenticated User endpoint;
the signed public zone remains structurally compatible with the personal
server.

### Mainline FOKS interoperability

Wire compatibility means an ordinary v0.1.9-compatible client can use the
listed local methods against this host. It does not mean that this server can
join another host's identity or team authority:

- host IDs, Merkle roots, users, names, teams, and credentials remain scoped to
  this installation;
- remote-member and remote-team arguments are rejected explicitly;
- team view and admin tokens are local, short-lived capabilities and are not
  portable to another FOKS host;
- no realtime channel exists, so mainline clients that require notification
  delivery must fall back to explicit refresh;
- unsupported TeamAdmin, TeamLoader, TeamMember, and TeamGuest methods do not
  partially mutate local state; and
- checked-in v0.1.9 fixtures plus an opt-in Go client/server audit define the
  compatibility baseline and prevent Rust-only agreement from being mistaken
  for protocol compatibility.

## 3. Repository and dependency isolation

This remains a standalone FOKS subsystem. Its source belongs in:

```text
crates/foks-*
tools/foks-server/
tools/foks-v019-oracle/
crates/foks-snowpack/tests/fixtures/foks-v0.1.9/
```

with `Cargo.toml`, `Cargo.lock`, `rust-project.json`, `MODULE.bazel` and
`MODULE.bazel.lock` as shared build metadata.

No FOKS crate may directly or transitively depend on an AKA crate, be defined
outside `crates/foks-*` or `foks-tauri`, or declare a local path dependency
that resolves outside them. `foks-server-testkit` remains `publish = false`,
outside workspace `default-members`, and absent from every production
dependency graph.

No new production crate is expected. The current crate boundaries already
match the work:

```text
foks-snowpack      canonical values and hostile-input bounds
foks-proto         exact user/team/recovery/KV protocol objects
foks-crypto        typed signatures, HEPKs, boxes, commitments, and hashes
foks-verify        pure user/team transition and Merkle verification
foks-rpc           envelopes, route IDs, typed arguments/results, statuses
foks-merkle-store  immutable Merkle updates and proofs
foks-server-db     SQLite schema, snapshots, capabilities, atomic units of work
foks-server        TLS, principals, authorization, handlers, writer, operations
foks-server-testkit deterministic isolated server/client/team composition
foks-client-db     existing durable user/team acceptance and mutation journals
foks-client        existing public API used as the primary black-box driver
```

Prefer extending these crates over adding a `foks-team-server` crate or a
parallel KV implementation. If implementation reveals a reusable pure
transition primitive, it belongs in `foks-verify`; if it knows about SQLite,
listeners, or request scheduling, it does not.

### Build and foreign dependencies

The production build keeps the personal server's toolchain:

- stable Rust and Cargo for all FOKS crates and the server executable;
- a C compiler for `rusqlite`'s bundled SQLite build;
- the existing Cargo-resolved `rustls`/`rcgen`/`x509-parser` PKI stack, with no
  OpenSSL service dependency;
- no PostgreSQL, Redis, message broker, Go runtime, Node, or AKA library; and
- ordinary POSIX process/signal support for the current standalone binary and
  shell release scripts.

Go 1.25 is optional and confined to `tools/foks-v019-oracle`. Docker and
PostgreSQL are needed only for the explicitly opt-in live upstream integration
audit. They are not build, runtime, or ordinary CI dependencies of the Rust
server. Bazel may continue to consume the shared Cargo lockfile, but the
isolated development and acceptance gate uses explicit Cargo package selectors.

## 4. Architectural rules

### Exact signed bytes plus derived projections

The authoritative record is the exact canonical link, box, HEPK, root, and
response bytes. SQLite also stores current projections for authorization and
indexed reads: active credentials, current PUKs/PTKs, roster roles, removal
commitments, and chain heads.

Projection rows are written only as the output of a successful pure verifier.
Handlers must not independently interpret a few fields and then construct a
projection that the complete user or team chain would reject. Reopening a
database must be testable by replaying exact links and comparing the replayed
state to every materialized head.

### One identity/team commit path

User and team mutations share the same global Merkle authority and therefore
the same serialized writer. A mutation follows this shape:

```text
bounded canonical decode
load an immutable current snapshot
verify signatures, previous link, roles, boxes, and transition in pure code
build the new Merkle leaf, immutable nodes, signed root, and receipt
BEGIN IMMEDIATE
recheck chain head, Merkle head, reservation/token expiry, and operation ID
insert exact link and derived projections
insert Merkle nodes, leaf, root, backpointers, and replay receipt
publish chain and Merkle heads
COMMIT
```

The server returns only after the new signed root is durable. There is no
multi-stage Merkle pipeline, background signer, or eventually consistent team
index. KV changes remain outside the global identity Merkle tree.

All public mutating methods need a stable idempotency identity derived from
their immutable request bytes or protocol token. An exact retry returns the
stored result; conflicting bytes under the same identity fail closed. Client
operations whose wire result is void reconcile by loading the signed chain and
must never blindly repost an ambiguous edit.

### Principal and credential model

The current principal must grow from “Ed25519 SPKI implies a software device”
into a database-authenticated credential capability:

```text
AuthenticatedPrincipal
  host
  uid
  credential entity ID
  credential kind: software device | delegated subkey | backup key
  active role and current chain position
  certificate serial/expiry binding
```

TLS proves possession of the certificate SPKI. The database binds that SPKI
and certificate to an active credential and UID. Code must not guess an entity
type from a 32-byte SPKI when backup keys and delegated subkeys have different
protocol identities.

Route authorization remains narrower than successful mTLS:

- software devices may use ordinary user, team, and KV methods according to
  their verified role;
- backup credentials may load the user chain/PUK needed for recovery and may
  provision a replacement device, but cannot use personal/team KV or mutate
  teams merely because their certificate is valid;
- revoked credentials fail on the next request even if their certificate has
  not expired; and
- team capabilities remain additionally bound to their issuing host, team,
  member UID, role/generation, expiry, and intended capability class.

### Capabilities and secrets

Registration challenges, team-view tokens, and TeamAdmin tokens are random,
short-lived, bounded, and domain-separated. Store a keyed digest or ordinary
cryptographic hash of bearer tokens rather than the usable token where the
wire protocol permits it. Store challenge payloads only as long as needed to
verify an activation. Expired tokens are invalid in foreground checks even if
maintenance has not deleted them.

The server stores only opaque encrypted PUK/PTK parcels, seed-chain boxes, and
removal-key boxes. It never receives plaintext PUKs, PTKs, removal keys, user
file names, or file contents. Logs and metrics may contain route, outcome,
listener, coarse error class, and correlation IDs, but never tokens, exact
links, boxes, certificates, key identifiers that enable correlation, or
encrypted payload bytes.

### Generic KV namespace authorization

Do not copy the existing KV tables and handlers into a team-specific module.
Because no schema has shipped, replace the `uid`-only physical namespace with
an explicit party namespace:

```text
kv_namespaces
  namespace_id / party entity ID
  party_kind: user | named team | ad-hoc team
  host ID
  current root and accounting
```

Every KV table references the namespace ID. A request first resolves either:

- personal auth: active user credential and its current role; or
- team auth: active team-view token, current membership, and effective role.

The resulting sealed `KvAuthority` carries namespace, maximum readable role,
maximum writable role, and token expiry. KV handlers consume this capability
instead of parsing user/team auth repeatedly. Stored object key roles and
directory write roles are checked against the capability on every read and
write. PTK/PUK generations are not hard-coded to generation 1.

### Deliberate single-host tradeoffs

- All identity and team writes serialize behind one actor because they publish
  one global Merkle history. This favors simple crash consistency over write
  throughput.
- Team edits verify and commit synchronously. Large rosters create larger
  frames, more boxes, and longer writer occupancy, so v1 is explicitly bounded
  to small teams.
- Current authorization uses verified materialized projections for speed; exact
  chain replay and projection consistency tests defend that cache boundary.
- View/admin capabilities avoid replaying complete chains on every KV request,
  but introduce expiry, revocation, and cleanup state that must be checked in
  the foreground.
- Team KV shares the personal KV schema and writer. This keeps semantics
  uniform but couples team metadata throughput to other local writes.
- Local-only membership avoids federation, cycle detection across hosts, and
  remote revocation delivery. These features require new protocols and cannot
  be added by relaxing a validation check.

## 5. Components and physical files

The target structure is incremental; files are introduced in the phase that
first needs them. Existing files may stay consolidated when they remain small,
but SQL, validation, token state, and network dispatch must not collapse into
one handler module.

```text
crates/foks-rpc/src/
  lib.rs                          public client API and compatibility exports
  arguments/
    mod.rs
    user_mutation.rs              provision/revoke/rotation request decoding
    recovery.rs                   lookup challenge and signed lookup
    team_loader.rs                view challenge/activation/chain load
    team_admin.rs                 reservations, create/edit, admin tokens
  response.rs                     exact additional v0.1.9 status variants

crates/foks-proto/src/identity/
  mutation.rs                     existing user/team mutation values
  recovery.rs                     challenge and lookup result constructors
  team.rs                         team chains, views, parcels, edits

crates/foks-verify/src/
  user.rs                         user-chain replay and referenced-root epochs
  user_transition.rs              provision/revoke/rotation validation
  team.rs                         team replay, transition, PTK/roster invariants
  merkle.rs                       authenticated historical roots for team links
  server.rs                       narrow validated-command adapters if useful

crates/foks-server-db/src/
  schema/
    identity.sql                  expanded credentials and user transitions
    team.sql                      teams, names, chains, roster, PTKs, boxes
    capabilities.sql              challenges and hashed short-lived tokens
    kv.sql                        party-scoped namespace schema
  user_mutation.rs                atomic append/projection/Merkle commit
  recovery.rs                     lookup challenge lifecycle and credential lookup
  team.rs                         immutable snapshots and atomic team commit
  team_names.rs                   reservation and expiry semantics
  capabilities.rs                 issue/activate/resolve/revoke capabilities
  kv.rs                           generalized party-scoped storage API
  read.rs                         user/team/credential immutable snapshots
  config.rs                       team/token/mutation limits
  maintenance.rs                  bounded expiry and reclamation

crates/foks-server/src/
  auth/
    principal.rs                  database-bound credential capability
    team.rs                       team-view and TeamAdmin authorization
  identity/
    mutation.rs                   validated user transition commands
    recovery.rs                   backup enrollment/recovery constraints
    team_create.rs                named/ad-hoc founding validation
    team_edit.rs                  membership and PTK-rotation validation
  services/
    registration.rs               existing registration plus recovery lookup
    user.rs                       reads, host config, provision/revoke dispatch
    team_loader.rs                challenge, activation, and chain responses
    team_admin.rs                 reserve/create/edit/admin token methods
    kv.rs                         shared user/team KV dispatch
  kv/
    auth.rs                       sealed KvAuthority resolution
  rpc/routes.rs                   enforced expanded route table
  net/session.rs                  transport orchestration only
  maintenance.rs                 token/challenge cleanup registration

crates/foks-server-testkit/src/
  account.rs                      multiple deterministic users/devices/backups
  team.rs                         named/ad-hoc secrets and member fixtures
  client.rs                       reconstructed clients and protected stores
  environment.rs                 temp-only shared installation
  clock.rs                        deterministic expiry
  scheduling.rs                  writer/token race synchronization

crates/foks-server-testkit/tests/
  conformance/
    provisioning.rs
    recovery.rs
    team_loader.rs
    team_create.rs
    team_edit.rs
    team_kv.rs
  user_mutation_recovery.rs
  team_token_expiry.rs
  team_restart_recovery.rs
  team_concurrency.rs
  team_capacity.rs
  small_team_binary_release.rs

tools/foks-server/
  test-small-team.sh
  test-small-team-repeat.sh

tools/foks-v019-oracle/
  team_server_fixture.go          official handler/status fixture extensions
  team_server_fixture_test.go
  run-live-team-compat.sh         opt-in Go/Rust process audit
```

No new third-party runtime dependency is expected. Prefer the existing
`rusqlite`, `rustls`, `rcgen`, `ed25519-dalek`, `zeroize`, `tempfile`, `serde`,
and `toml` stack. Any new dependency requires an explicit reason, license and
feature review, an offline lockfile update, and confirmation that it does not
pull an AKA crate or native database/runtime toolchain into FOKS.

## 6. Testing policy

Testing remains phase-local and uses the narrowest layer that proves the
property:

1. Go-generated exact fixtures for route IDs, arguments, results, statuses,
   signed links, PTK parcels, removal boxes, and team KV frames.
2. Pure verifier tests for every valid and invalid user/team transition.
3. SQLite transaction tests with failure injection at every authoritative
   publication boundary.
4. Handler tests for typed status mapping and listener/authentication policy.
5. Public `foks-client` tests over real loopback TLS.
6. Release-binary lifecycle tests with no injected production fault endpoint.
7. Opt-in live Go audits; ordinary CI needs neither Go, Docker, nor network.

Every new supported route is added to `protocol-v1.toml`, the Rust route table,
and the executable conformance registry in the same phase. Tests compare all
three. Every documented status has a negative test at the nearest practical
layer.

All stateful tests use `TestEnvironment`; every database, key directory,
backup, log, client store, and protected secret lies below its temporary root.
All listeners are ephemeral loopback sockets. The isolation scanner continues
to reject user/AKA paths, direct server database opens, and direct listener
binding in integration tests.

Each phase gate is its focused tests plus:

```sh
tools/foks-server/check.sh
```

After implementation, each phase receives two explicit reviews:

1. code quality, modularity, deterministic scheduling, bounded waits, and
   useful error classification; then
2. canonical encoding, signatures, authorization, replay, expiry, atomicity,
   secret handling, v0.1.9 compatibility, and AKA/test isolation.

## 7. Implementation phases

### Phase 0: freeze the second-slice protocol and fixtures

Add the exact method/status matrix before enabling behavior.

Files:

```text
crates/foks-server/protocol-v1.toml
crates/foks-server/src/rpc/routes.rs
crates/foks-server/tests/protocol_matrix.rs
crates/foks-server-testkit/tests/conformance/registry.rs
crates/foks-rpc/src/response.rs
crates/foks-rpc/tests/server_responses.rs
tools/foks-v019-oracle/team_server_fixture.go
tools/foks-v019-oracle/team_server_fixture_test.go
```

Add exact status codes used by the slice, including the v0.1.9 team family
(`7001` through the specifically exercised team errors), ad-hoc errors
(`7101`-`7104`), expired (`1062`), device-already-provisioned (`1072`), and KV
permission (`8011`). Do not map distinct upstream meanings to a generic 1013
or 1014 merely because the current client could retry both.

Exit criteria:

- Every added method has exact protocol ID, position, listener, auth class,
  request/result type, size limit, and status list.
- TOML, Rust routes, and callable test registry match in both directions.
- Official Go code decodes every checked success and structured error frame.
- All still-excluded methods are explicit and cannot reach a fallback mutation.

### Phase 1: typed decoding and pure transition readiness

Add server-side typed decoders without changing existing client encodings.
Factor pure validation around the complete existing user/team verifiers.

Files:

```text
crates/foks-rpc/src/arguments/{user_mutation,recovery,team_loader,team_admin}.rs
crates/foks-rpc/tests/server_requests.rs
crates/foks-proto/src/identity/{mutation,recovery,team}.rs
crates/foks-verify/src/{user,user_transition,team,merkle}.rs
crates/foks-verify/tests/server_transitions.rs
```

The team client must gain the same historical-Merkle-root recovery already
used for fresh user clients: extract referenced epochs from an untrusted team
chain, fetch the bounded required roots, authenticate every skip path back to
the exact trusted latest root, then verify the chain. The extracted epoch list
grants no trust and is capped before any request allocation.

Exit criteria:

- Every official provision, revoke, PUK rotation, backup, team creation, team
  edit, view, PTK, and removal fixture decodes exactly.
- Full-chain replay and one-link incremental replay produce identical states.
- Mutation tests reject wrong previous links, roots, hosts, signers, roles,
  generations, boxes, removals, and PTK schedules before persistence.
- Team historical-root tampering and future/duplicate epoch requests fail.

### Phase 2: atomic user provisioning, revocation, and PUK rotation

Generalize the signup-only identity commit into an append-only user mutation
unit of work. Keep signup behavior and receipts unchanged.

Files:

```text
crates/foks-server-db/src/schema/identity.sql
crates/foks-server-db/src/user_mutation.rs
crates/foks-server-db/src/read.rs
crates/foks-server-db/tests/user_mutations.rs
crates/foks-server/src/identity/mutation.rs
crates/foks-server/src/services/user.rs
crates/foks-server/tests/user_mutations.rs
crates/foks-server-testkit/tests/conformance/provisioning.rs
```

The transaction appends the exact link, next tree location, HEPKs, PUK
parcels/seed chains, device activation/revocation, current projections, Merkle
leaf/root, and receipt atomically. It rechecks signer activity, expected user
head, expected Merkle head, role authority, and capacity inside the write.

Exit criteria:

- A new software device authenticates only after the provision link is durable.
- Revocation immediately invalidates existing mTLS use despite certificate
  lifetime.
- Provisioning a new role stores generation-1 PUK material; provisioning an
  existing role cannot replace its PUK.
- Standalone PUK rotation accepts only a complete role prefix and preserves
  exact historical seed chains.
- Dropped responses reconcile through the signed user chain without duplicate
  links, roots, devices, parcels, or quota charges.
- Failure injection at every stage exposes either the old state or complete new
  state, never a mixed credential/chain/Merkle projection.

### Phase 3: backup recovery, credential principals, and host policy

Implement public signed lookup and make authentication credential-aware.

Files:

```text
crates/foks-server-db/src/schema/capabilities.sql
crates/foks-server-db/src/{recovery,capabilities}.rs
crates/foks-server-db/tests/recovery.rs
crates/foks-server/src/auth/principal.rs
crates/foks-server/src/identity/recovery.rs
crates/foks-server/src/services/registration.rs
crates/foks-server/src/services/user.rs
crates/foks-server/tests/{recovery,credential_authorization}.rs
crates/foks-server-testkit/tests/conformance/recovery.rs
```

Return a fixed standalone `HostConfig` whose exact bytes match a v0.1.9
fixture: no metering claims, `Open` local user/team viewership, standalone host
type, and the actual empty-invite signup regime. Configuration that would make
this response dishonest is rejected at startup.

Exit criteria:

- Backup enrollment is an ordinary verified user-chain transition.
- UID lookup reveals account material only after a fresh, unexpired challenge
  is signed by the exact enrolled backup key.
- Challenge replay, substitution, expiry, wrong host, revoked key, and
  ambiguous key bindings fail without information leakage.
- Public certificate issuance binds the requested entity to the stored UID and
  SPKI; subsequent mTLS resolves the persisted credential kind correctly.
- Owner backup recovery provisions a durable software device but cannot access
  KV or team administration directly.
- Restart and matching-key backup/restore preserve recovery; mismatched
  database/key generations fail closed.

### Phase 4: team schema and atomic team authority

Introduce the complete local team schema before exposing creation routes.

Files:

```text
crates/foks-server-db/src/schema/team.sql
crates/foks-server-db/src/{team,team_names}.rs
crates/foks-server-db/src/read.rs
crates/foks-server-db/src/config.rs
crates/foks-server-db/tests/{team_schema,team_transactions,team_replay}.rs
crates/foks-server/src/identity/{team_create,team_edit}.rs
```

Core tables include:

- team identities and kind (named/ad-hoc), local host, creation time;
- named-team reservations and normalized name ownership;
- exact chain links, heads, names, hidden locations, and root epochs;
- materialized local roster entries with source/destination role, generation,
  verifier, HEPK fingerprint, scoped host, and removal commitment;
- exact PTK public material by role/generation;
- PTK parcels and seed-chain boxes addressed to members; and
- opaque removal-key boxes with authenticated member/host/source-role metadata.

Exit criteria:

- Constraints reject cross-host parties, invalid entity kinds, duplicate
  seqnos, dangling heads, nonmonotonic generations, duplicate current roles,
  and boxes without matching keys/members.
- Replaying exact stored chains reproduces every roster/PTK projection.
- Forced errors around link, projection, Merkle, and receipt publication roll
  back the entire team mutation.
- Backup, integrity check, and restore cover every new table automatically.

### Phase 5: team view capabilities and chain/PTK loading

Implement TeamLoader challenge, activation, and chain loading before creation
depends on client reconciliation.

Files:

```text
crates/foks-server-db/src/capabilities.rs
crates/foks-server-db/tests/team_capabilities.rs
crates/foks-server/src/auth/team.rs
crates/foks-server/src/services/team_loader.rs
crates/foks-server/tests/{team_view,team_chain}.rs
crates/foks-server-testkit/tests/conformance/team_loader.rs
```

The view challenge echoes no server-selected authority that the client will
sign blindly. Activation verifies the exact user PUK role/generation from the
current user chain, current team membership, signature, host/team/member
binding, time window, and one-time challenge. The activated token carries only
the member's effective current authority.

Exit criteria:

- Full and incremental named/ad-hoc chain responses verify in `foks-client`.
- Responses contain exactly one parcel per currently visible PTK role and the
  necessary historical seed chain—never keys above the member role.
- Removed/demoted users, stale PUK generations, replayed challenges, expired
  tokens, wrong-host/team/member requests, and altered signatures fail.
- Expiry is enforced without cleanup; cleanup merely reclaims rows.
- Exact activation replay returns the same committed capability/result after a
  lost response; conflicting replay is rejected. Concurrent activation cannot
  create two meanings for one token.

### Phase 6: ad-hoc and named-team creation

Expose the two founding mutations using the common team authority.

Files:

```text
crates/foks-server/src/services/team_admin.rs
crates/foks-server/src/identity/team_create.rs
crates/foks-server-db/src/{team,team_names}.rs
crates/foks-server/tests/{team_create,team_name_reservations}.rs
crates/foks-server-testkit/src/team.rs
crates/foks-server-testkit/tests/conformance/team_create.rs
```

Exit criteria:

- Named reservation is random, expiring, name-bound, and consumed in the same
  transaction that publishes the founding chain and Merkle root.
- The deterministic TeamID, founder identity, owner PUK binding, PTK roles,
  parcels, hidden locations, commitments, and HEPKs are cross-checked.
- Ad-hoc creation requires open user viewership, has no name reservation, and
  rejects subsequent edits.
- Same-request retries reconcile without duplicate names, teams, links, roots,
  or receipts; conflicting team/name reuse returns exact team statuses.
- Both public client create/resume paths finish by loading and pinning the
  ordinary verified team chain.

### Phase 7: named-team edits, removal material, and PTK rotation

Implement local additions and the complete current client removal/demotion
surface.

Files:

```text
crates/foks-server/src/identity/team_edit.rs
crates/foks-server/src/services/team_admin.rs
crates/foks-server/src/auth/team.rs
crates/foks-server-db/src/{team,capabilities}.rs
crates/foks-server-db/tests/{team_edits,team_removal_boxes}.rs
crates/foks-server/tests/{team_edits,team_admin_tokens}.rs
crates/foks-server-testkit/tests/conformance/team_edit.rs
```

Pure additions reuse existing PTKs and deliver only roles visible to the new
member. Any transition that removes visibility must rotate the exact affected
PTK prefix, increment each generation once, preserve the required historical
seed chain, and box new keys to every and only remaining eligible member.

TeamAdmin removal-key retrieval uses a separate inert-token/activation flow
signed by a current admin/owner PTK. The token is scoped to the team, actor,
role, generation, and expiry; it does not grant arbitrary team edits or KV.
Exact activation replay is idempotent so a lost void response does not strand
the client between inert and active states.

Exit criteria:

- Owner-authorized local addition, demotion, promotion, and removal match
  official transition fixtures and the current client's reconciliation logic.
- The server rejects actor escalation, wrong source role, stale seqno, stale
  PTK, omitted/excess rotation, duplicate recipients, mismatched removal MAC,
  wrong removal commitment, and modifications to ad-hoc teams.
- Removed members cannot activate new tokens; all already-issued view/admin
  tokens become invalid through current-roster/generation checks.
- Lost responses reconcile by chain seqno and exact link hash without reposting
  ambiguous signed edits.

### Phase 8: party-scoped KV and team KV

Generalize the existing storage and handlers, preserving personal-KV behavior
while enabling team bearer auth.

Files:

```text
crates/foks-server-db/src/schema/kv.sql
crates/foks-server-db/src/kv.rs
crates/foks-server-db/tests/{kv_namespace,team_kv}.rs
crates/foks-server/src/kv/auth.rs
crates/foks-server/src/services/kv.rs
crates/foks-server/tests/team_kv_authorization.rs
crates/foks-server-testkit/tests/conformance/team_kv.rs
```

Exit criteria:

- Every existing personal-KV test remains green against the generalized
  namespace model.
- Named and ad-hoc teams create/read roots and round-trip directories, small
  files, symlinks, chunks, cache vectors, listings, and locks through the
  public team client APIs.
- Read and write roles are enforced from current membership and token scope;
  generation greater than one works after PTK rotation.
- Namespace IDs cannot cross user/team or team/team boundaries even with
  guessed object IDs or a valid token for another team.
- Removal/demotion invalidates access immediately; ciphertext already cached by
  a former member is outside server revocation guarantees, while all future
  writes use rotated PTKs.
- Team KV quota, uploads, locks, and lost-response receipts are charged to the
  team namespace and never to the acting user.

### Phase 9: recovery, concurrency, capacity, and release gate

Expand operational tests and maintenance without adding a general job system.

Files:

```text
crates/foks-server-db/src/{config,maintenance}.rs
crates/foks-server/src/{maintenance,metrics,standalone}.rs
crates/foks-server-testkit/tests/
  team_restart_recovery.rs
  team_concurrency.rs
  team_capacity.rs
  small_team_binary_release.rs
tools/foks-server/test-small-team.sh
tools/foks-server/test-small-team-repeat.sh
tools/foks-v019-oracle/run-live-team-compat.sh
crates/foks-server/README.md
```

Exit criteria:

- Client/server restart and forced termination at each completed mutation leave
  a replayable user/team chain and matching Merkle head.
- Matching-key backup/restore preserves names, recovery credentials, teams,
  PTK histories, capabilities policy, and team KV under existing client pins.
- Concurrent independent-team reads progress during serialized writes; two
  edits at one head produce one winner and an exact typed stale/team-race loser.
- Queue saturation returns bounded rate limiting, not SQLite `BUSY` or an
  unbounded wait.
- Member, role, parcel, link, token, team-name, team-count, namespace, upload,
  and total-database limits have minus-one/exact/plus-one tests with no partial
  charge or mutation.
- The release binary passes the complete success scenario repeatedly with
  varied deterministic seeds and secret-leak log scanning.
- Checked fixtures and the opt-in Go v0.1.9 audit accept supported Rust frames
  and reject documented unsupported operations cleanly.

## 8. RPC surface delta

Existing personal-server methods remain supported. The small-team slice adds
or changes these routes:

| Protocol | Pos | Method | Listener | Authentication | V1 |
| --- | ---: | --- | --- | --- | --- |
| Reg | 6 | getUIDLookupChallege | public services | delegated TLS; well-formed entity requested | yes |
| Reg | 7 | lookupUIDByDevice | public services | fresh challenge + entity signature | yes |
| User | 6 | provisionDevice | authenticated | authorized active device or owner backup credential | yes |
| User | 7 | revokeDevice | authenticated | authorized active device; also carries PUK rotation | yes |
| User | 24 | getHostConfig | authenticated | active enrolled credential | yes |
| TeamLoader | 0 | getTeamVOBearerTokenChallenge | authenticated | active device + current local user | yes |
| TeamLoader | 1 | activateTeamVOBearerToken | authenticated | current PUK signature over exact challenge | yes |
| TeamLoader | 3 | loadTeamChain | authenticated | active unexpired team-view token | yes |
| TeamAdmin | 0 | reserveTeamname | authenticated | active owner-capable local user | yes |
| TeamAdmin | 1 | createTeam | authenticated | reservation + signed named-team founding material | yes |
| TeamAdmin | 2 | editTeam | authenticated | current named-team admin/owner authority | yes |
| TeamAdmin | 3 | makeInertTeamBearerToken | authenticated | current named-team admin/owner member | yes |
| TeamAdmin | 4 | activateTeamBearerToken | authenticated | current PTK signature over inert token payload | yes |
| TeamAdmin | 10 | loadRemovalKeyBoxForTeamAdmin | authenticated | active scoped TeamAdmin token | yes |
| TeamAdmin | 15 | createTeamAdHoc | authenticated | signed ad-hoc founding material; open viewership | yes |
| KVStore | all existing | existing KV methods | authenticated | user auth or active team-view token | expanded |

The misspelling `getUIDLookupChallege` is the upstream v0.1.9 method name and
must not be “corrected” on the wire. TeamLoader positions 2 and 4-7, TeamAdmin
positions 5-9 and 11-14, TeamMember, TeamGuest, remote/federated forms, and
Realtime remain unsupported unless a later plan expands the scope.

## 9. Jobs, scheduling, and capacity

### Jobs and scheduling

No mutation depends on a background job. User links, team links, projections,
Merkle roots, and receipts commit synchronously through the bounded writer
queue. Client polling observes already-authoritative state; it is not a server
job queue.

The maintenance loop performs only bounded, correctness-independent work:

- delete expired registration challenges;
- delete expired/consumed view and TeamAdmin tokens;
- reclaim expired name reservations and request receipts;
- reclaim abandoned uploads and expired locks;
- checkpoint WAL and report integrity/storage metrics; and
- run operator-triggered online backups through the existing operations path.

Correctness checks include time and current chain/roster state in foreground
queries, so delayed maintenance cannot resurrect a token or block reuse beyond
documented reservation policy. A second writer process remains rejected; no
leases or distributed scheduler are added.

### Capacity model

Every potentially multiplicative input has an explicit configurable bound:

- total users, credentials, recovery keys, teams, and reserved names;
- team members and distinct active role/visibility bands;
- user/team links and maximum links returned per response;
- HEPKs, PTK/PUK boxes, seed-chain boxes, removal boxes, and recipients per
  mutation;
- active challenges/tokens per user and team plus global totals;
- encoded link, request, response, and receipt bytes;
- Merkle nodes/backpointers per commit;
- KV objects, bytes, uploads, chunks, locks, and total database pages per
  namespace and installation; and
- writer queue depth, worker count, requests per connection, frame size, and
  operation deadline.

Phase 0 should freeze these proposed deployment defaults after checking exact
fixture sizes and the largest client-generated mutation in the test matrix:

| Limit | Proposed default |
| --- | ---: |
| active credentials per user | 16 |
| active backup credentials per user | 4 |
| active members per named team | 64 |
| active PTK role/visibility bands per team | 16 |
| HEPKs/parcels/seed or removal boxes in one mutation | 1,024 each |
| signed links per user or team | 4,096 |
| live view/admin capabilities per user/team pair | 32 per class |
| view/admin capability lifetime | 6 hours (v0.1.9 default) |
| teams per installation | 4,096 |
| KV bytes per user or team namespace | existing 16 GiB default |
| total SQLite database bytes | existing 64 GiB default |

These are server policy limits, not claims about the upstream protocol's
maximum. A regular client that exceeds one receives the exact applicable
quota/too-big/team status before mutation; the server does not truncate a
roster, parcel set, or signed chain silently. Chain responses also retain the
existing frame/response-byte bounds. If v0.1.9 does not provide a continuation
shape sufficient to load a chain near the link ceiling, Phase 0 must lower the
link ceiling or add a proven client loop before enabling edits that would make
the team unloadable.

All limits remain configurable downward for deployments and through sealed
test profiles. Boundary tests use reduced values to exercise the same code
without large allocations. CI proves bounded acceptance/rejection, not a
universal throughput number.

Measure and report separately:

- user mutation and signed-root commits per second;
- team creation/edit latency by member, role, and box count;
- token activation and team-chain load latency;
- team KV metadata operations and chunk bytes per second;
- reader latency during identity/team writer saturation;
- WAL/database growth per team, link, member, PTK generation, and KV workload;
  and
- checkpoint, backup, restore, and replay-verification time by database size.

The expected v1 throughput ceiling is one serialized authoritative user/team
mutation at a time. Reads remain concurrent. Team KV writes are also serialized
by the current writer actor even though they do not advance the global Merkle
tree. No fixed throughput or total-storage claim is made until measurements
include hardware, PRAGMAs, durability mode, worker/queue limits, roster shape,
payload mix, and percentile latency.

## 10. Future partitioning

Preserve these seams without implementing distribution prematurely:

1. Move opaque chunk bodies to a content-addressed local blob store while
   retaining metadata and publication in SQLite.
2. Partition independent KV namespaces behind the existing DB API and
   `KvAuthority` capability.
3. Separate token/challenge storage from long-lived chain state if activation
   volume requires it.
4. Partition team-chain reads and projections only after replay consistency is
   independently checkable.
5. Keep the global identity/team Merkle log on one leader unless a consensus
   protocol is introduced.
6. Treat federation as a new authenticated replication and revocation design,
   not as another `party_kind` accepted by local handlers.

The present architecture deliberately accepts a single-writer bottleneck,
SQLite database-size ceiling, synchronous large-team edit cost, polling instead
of realtime, and one-host trust domain in exchange for compact deployment,
simple backup/restore, and auditable atomicity.

## 11. Review gates

Every phase answers:

1. Does untrusted data become typed and fully verified before authority?
2. Is every authorization decision based on current verified user/team state,
   not merely a valid certificate or bearer token?
3. Can a crash expose a chain head, projection, Merkle root, key generation, or
   KV namespace that disagrees with the others?
4. Are retries and ambiguous responses reconciled without duplicate signed
   mutations?
5. Are challenge/token expiry and revocation effective before maintenance?
6. Are secret material and opaque encrypted payloads absent from logs/errors?
7. Does the result match v0.1.9 exact bytes and statuses where promised?
8. Are paths, sockets, dependencies, and tests still isolated from AKA and user
   data?

Security-sensitive phases receive explicit adversarial review for role
ordering, visibility, PUK/PTK generations, recipient sets, removal proofs,
cross-host substitution, stale capabilities, integer overflow, and
quadratic/unbounded inputs.

## 12. Definition of done

The small-team server slice is complete when:

- the expanded route matrix is exact and enforced by executable scenarios;
- the public Rust client completes provisioning, revocation, PUK rotation,
  backup enrollment/lookup/recovery, named/ad-hoc creation, PTK loading, local
  membership edits, PTK rotation, and team KV without test-only APIs;
- fresh and durable clients authenticate historical user and team roots back to
  an existing pin;
- removed/demoted members and revoked credentials lose future access
  immediately, while remaining members can decrypt the required PTK history;
- user/team mutations, Merkle publication, receipts, and projections are atomic
  across faults, lost responses, and restarts;
- personal KV remains compatible and team KV enforces party/role/generation
  boundaries throughout the full existing KV surface;
- matching-key backup/restore works with existing pins and credentials and
  corruption/mismatched keys fail closed;
- the release-profile binary passes repeated deterministic end-to-end runs;
- v0.1.9 fixtures and the optional independent Go audit accept the supported
  wire behavior;
- unsupported federation/invite/realtime methods fail explicitly;
- capacity and queue saturation reject work predictably without partial state,
  unbounded allocation, or SQLite errors leaking to clients;
- no implementation or test touches an AKA crate or user data path; and
- `tools/foks-server/check.sh` passes from a clean checkout using only explicit
  FOKS package selectors.
