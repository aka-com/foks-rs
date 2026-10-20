# Real FOKS / celld integration proof

Status: bounded opt-in implementation and planned local qualification complete,
2026-10-15. See HOSTED_CELLD_PROOF_RESULTS.md for measured passes and failures.
Keep unstaged. The first gate is local qualification, with no AWS environment or production deployment. User
approval to implement the overall work does not bypass the requested plan,
direct review, independent review, test and component-commit sequence.

## Decision and scope

Use the installed, unmodified celld v0.6.2 binary for a bounded proof with one
real FOKS installation. Retain native FOKS TCP, delegated TLS, client-certificate
authentication, codecs and cryptography. Make a cell's actual SQL tables the
authority for the supported KV operations. Do not store a whole mutable SQLite
image beside a replicated generation counter and describe that as a cell SQL
adapter. Do not rebuild celld/V8 or attempt the whole server's Wasm port first.

The proposed bridge is deliberately experimental: execute an allowed request
speculatively with existing native FOKS logic against a fresh, disposable
projection of one cell snapshot, then apply its bounded base-table delta in a
cell SQL transaction under a revision and adapter-owner check. Keep constraints
and accounting triggers active in the authoritative database. Only release the
native response after the cell's output gate proves the resulting transaction.
The cell contains real encrypted FOKS nodes, dirents, heads and quota state;
the native projection is never a recovery source or independent authority.

Limit the first proof to a pre-provisioned personal account, its existing root
directory, encrypted small-file insertion/overwrite, listing/get and usage.
Include the lock operations actually exercised by the pinned client, if any,
as explicit reviewed additions. Account creation happens during provisioning,
before activation. Identity, membership, key rotation, federation, external
chunks, maintenance, background jobs and administrative mutations are disabled
in proof mode. Their integration remains future work, not silently local state.
Unknown routes fail closed. This is not a deployable replacement backend.

## Why this approach

`foks-server-db::Database` embeds a concrete `rusqlite::Connection` in
[connection.rs](../foks-server-db/src/connection.rs). Existing writer jobs are
Rust closures, not serializable commands. Cell APIs expose synchronous SQL and
`transactionSync`, but the released celld binary has no demonstrated API that
turns this native connection into an actor-owned one. A socket proxy cannot
hold a cell transaction across arbitrary native round trips without introducing
new lifetime, interleaving and failure semantics.

The narrow native seam is [KV dispatch](src/net/session/handlers/kv.rs) and
[KV services](src/services/kv.rs). The latter already accepts an explicit reader,
writer, clock and object backend. An opt-in proof adapter can invoke this logic
with a temporary database and writer, without reimplementing FOKS cryptography
or placing arbitrary SQL from a network client into the cell.

A native SQLite session changeset is not directly consumable by celld's public
SQL API. The repository pins rusqlite 0.40.2 with `backup`, `bundled` and `uuid`,
but not `session`; enabling session brings another SQLite build feature and
still requires a reviewed changeset translator. Changesets also include trigger
effects: blindly applying these while accounting triggers run double-charges
quota. For this small proof, prefer typed before/after row comparison under
strict table/column allowlists. A production incremental backend is a separate
decision after measuring this proof.

## Component A: authoritative schema and deterministic projection

1. Export the initialized fixture schema and typed rows from a consistent native
   snapshot. Pin schema version and a schema fingerprint in the proof manifest.
   Install the actual application tables, indexes and triggers through cell SQL;
   keep celld's reserved `_cf_` objects untouched. Do not replay migration scripts
   against celld's internal schema or attempt its disallowed `user_version` pragma.
   Store the FOKS schema version in a separate application-owned metadata table.
2. Import the fixture before serving: create tables/indexes, load data, create
   triggers after loading already-accounted rows, run foreign-key and invariant
   checks, then mark the installation initialized atomically. Missing/partial
   initialization cannot serve. Initialization must be idempotent only for the
   same immutable manifest and HostID; never overwrite a nonempty installation.
3. Serialize values with explicit SQLite types. Encode signed 64-bit integers as
   canonical decimal strings and convert using exact SQL casts where needed;
   never round them through JavaScript Number. Preserve blobs, null and text
   distinctly. Reject unsupported values, invalid identifiers and schema drift.
   Cell snapshots select typed values without lossy integer conversion.
4. A cell snapshot transaction exports one revision's rows plus fingerprint,
   HostID and key-bundle identity. Reconstruct a fresh native projection with
   native SQLite constraints; create accounting triggers after initial row load.
   Validate the complete projection against the schema and FOKS invariants.
   Do not replace an open database beneath existing read pools or change their
   captured inode identities. Each proof request owns its temporary database.
5. Initially cap snapshot/delta bytes, rows, table count and SQL statements with
   explicit configuration. Start with small fixtures (for example 8 MiB total
   snapshot and 10,000 rows), then set exact tested defaults before implementation
   is accepted. Decline larger histories before executing the request. No hidden
   fallback to unreplicated native SQLite is permitted.

## Component B: request execution and authoritative publication

One serial adapter owner serves the tenant initially. This is not a claim that
the native process has acquired celld's own ownership epoch. The application
maintains a separate adapter epoch and owner token in cell SQL, checked in every
snapshot, publication and response-proof operation. Promoting a native frontend
increments this epoch inside the cell; stale frontends cannot publish afterward.
Cell ownership/replication still belongs to celld underneath this application
check. The proof controller alone can request promotion on its private interface.

For each supported RPC:

1. Acquire the per-tenant adapter operation guard. Check route, frame and aggregate
   limits before materializing the snapshot. Fetch a cell snapshot for the current
   adapter owner. Do not expose data before the gated cell response returns.
2. Revalidate the ordinary device/party against that projection, not merely a
   principal cached by the native TLS session. Invoke the existing KV service
   against its temporary reader/writer with no external object backend. Drain
   and join that writer before comparing state; an outer timeout must not permit
   detached work to reuse or publish through a released guard.
3. Hold the encoded protocol result, including errors, as candidate output.
   Compare every application table before/after. Reject unexpected changed tables
   rather than omitting them. The initial writable base allowlist includes only
   the concrete tables required by small nodes/dirents/heads and any approved
   locks. Derived accounting columns/tables are observations, not replay inputs.
4. Send a canonical delta with expected revision, adapter epoch, unique attempt
   ID and digest of the candidate output. In `transactionSync`, require the
   current owner/revision, verify old row values, apply parameterized base-table
   changes in an explicitly reviewed dependency order, and let normal triggers
   produce accounting effects exactly once. Do not use `REPLACE` or disable
   constraints. Check affected-row counts, FK validity and all predicted derived
   values. Check the projected after-state digest, then insert the immutable
   receipt and advance the revision in the same transaction. Any mismatch rolls
   back the complete transaction.
5. The cell must fully consume write cursors before returning. Explicit
   `storage.sync()` outside the SQL transaction may make the proof boundary easier
   to inspect, but does not replace celld's ownership/output gate. The native
   frontend releases its candidate RPC response only after receiving a matching
   durable receipt. A read-only or definite application-error result still needs
   an owner/revision validation/proof before publication; a conflict discards the
   candidate rather than returning data from an obsolete projection.
6. Lost publication replies query/retry the same attempt ID, epoch and digest.
   An existing matching receipt resolves ambiguity; a mismatching payload fails.
   Never rerun a native mutation merely because an HTTP request timed out. A
   higher adapter epoch rejects all stale attempts; the replacement reconciles
   committed state using current authority and existing FOKS client semantics.
   An unavailable cell returns only an existing retryable FOKS error or closes
   the connection, never a speculative success or data-bearing failure.

Receipt storage is bounded. The first proof retains its finite declared attempt
set for the complete run and refuses additional attempts at its cap. Do not
introduce unsafe timed receipt deletion or claim indefinite idempotency.

The important atomic example is FOKS `put_kv_dirents`: insertion of exact dirent
rows, advancement of head rows and trigger-maintained namespace accounting must
commit together in cell SQL. A new encrypted node and its later directory
publication remain separate RPCs, exactly as in the current protocol. Preserve
that behavior instead of claiming one atomic operation across both.

## Component C: wire boundary and key custody

Use the existing three native listeners and Go-compatible authentication. Add
only a feature-gated proof hook; ordinary production startup cannot accidentally
select it. Keep native framing, TLS/mTLS and public protocol statuses identical.
All accepted responses must pass the proof gate, including read-only identity
responses that use the immutable provisioning snapshot. Install a checked route
allowlist before dispatch so disabled state-changing methods cannot reach the
ordinary local writer. Disable maintenance and every secondary HTTP/admin path.

Store the encrypted host-key files and their manifest with the initialized cell
installation. The wrapping/root secret stays with a separate surviving fixture
secret provider; it is never logged or stored unencrypted in cell rows. Recovery
fetches the bundle through the gated cell API and verifies HostID/generations
before starting TLS. The first proof freezes key generations and explicitly does
not qualify key rotation or a production secret manager. Native disposable files
and keys can be deleted and reconstructed; pinned client stores must survive.

The private bridge requires an authenticated, installation-scoped controller/
adapter capability; it must not expose arbitrary schema loading, SQL, promotion
or key-bundle retrieval on the public FOKS endpoints. Local experiments bind the
bridge to loopback and use synthetic credentials. Deployment would additionally
need encrypted private transport and real credential lifecycle; that is outside
this local proof.

## Components D/E: compatibility, recovery and bounded capacity

Retain the existing lab at `/private/tmp/foks-celld-lab`, pinned celld v0.6.2 and
source commit `90b43017241f81189453d326d05948f388b34652`. Keep new reproducible
controller/Worker/native adapter sources in this repository; generated state and
tools stay temporary. Moto remains an explicitly nonpersistent local emulator;
the experiment does not requalify managed S3's promised durability/consistency.

Required tests after both reviews of each component:

- Differential schema/typed snapshot tests, exact integer/blob round trips,
  malformed input bounds, FK failures and trigger-accounting equivalence. Inject
  failure between dirent insertion, head advancement and receipt insertion: no
  partial authoritative state is visible. Stale revisions/owners must fail.
- Actual Rust and pinned Go v0.1.9 clients authenticate via native mTLS, put/get/
  overwrite small encrypted content and compare usage. Assert actual cell SQL
  contains those FOKS rows. Delete every disposable native database and recover
  successfully from the cell plus surviving root secret and existing client pins.
- Lose the cell publication reply after commit; retry/reconcile the same attempt
  and verify one row transition/charge. Crash the adapter before/after publication
  and before network response. A failed cell proof must release no candidate
  response, including a read or error containing observed state.
- Pause an old native frontend, promote a replacement adapter epoch, then resume
  its queued candidate. Verify no authoritative state change or success output.
  This tests the application adapter fence; separately pause/resume or partition
  a celld owner to test the platform's own fence. Do not conflate the two.
- Under one external clock, kill the celld owner during active native FOKS writes.
  Keep every acknowledged observation in the external ledger and recover through
  successful authenticated read plus new write/read within the selected 15-second
  budget. Report failures, retries, exact revision/epoch and complete history audit.
  Retain the controller, surviving replicas and emulator. This is local failure
  qualification, not AZ certification or proof after emulator loss.
- Measure adapter bytes/RPC, snapshot reconstruction time, allocation peaks,
  SQL statement count, candidate conflicts and cell proof time. Repeat at several
  bounded metadata-history sizes. Whole-state export and temporary native writer
  creation are expected to limit throughput; do not extrapolate to 10,000 tenants.
  Density baseline for ordinary native FOKS remains a separate experiment.

## Stop/go criteria and implementation slices

Review in stages: (A) typed schema/projection plus JS authority;
(B) publication/idempotency/fencing engine; (C) native route/output/key hook;
(D) real Go/Rust compatibility and fault harness; (E) measured local run and
capacity report. Tests follow direct and independent review for every slice.

The implemented proof is one opt-in integration commit after all qualification
gates. Its server/testkit Cargo features, exact generated schema, native bridge,
fixture lifecycle, controller markers and Go compatibility test form a shared
build and validation boundary. Splitting it now would require partial manifests
and temporary harness variants, creating incomplete states that need additional
boundary tests. This changes the originally proposed per-stage commit layout,
not the required staged direct/independent reviews or test gates. The primary
hosted-hardening items 1–5 retain their separate commits. Plans/results stay
unstaged as requested.

Stop the proof and report the specific blocker if celld SQL cannot preserve the
required schema semantics, a request has uncontained side effects, or a fully
gated native seam cannot be made small and auditable. Do not weaken constraints,
copy trigger effects twice, silently omit changed tables, or broaden the native
response gate to an unreviewed production subsystem to force a passing demo.

Success demonstrates a real but intentionally expensive native-to-cell authority
path with unchanged client wire behavior. It does not establish production
backend coverage, key-rotation recovery, external object publication, deployment
scheduling, multi-tenant isolation, active-AZ failover or the final throughput/
latency target. Those require subsequent design decisions informed by this proof.
