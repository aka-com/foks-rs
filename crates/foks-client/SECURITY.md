# FOKS client security model

This document describes the native FOKS v0.1.9 slice in this workspace. It is
an implementation guide for reviewers, not a claim that the client implements
the full FOKS product.

## Trust boundaries

All network responses and all hard-state SQLite rows are untrusted. A response becomes
usable only after `foks-snowpack` accepts its canonical representation,
`foks-proto` accepts its exact schema, and `foks-verify` returns a sealed
verified value. `foks-client-db` accepts those sealed values and independently
enforces monotonicity while committing the projection and its authenticating
pin in one transaction.

The separate soft database contains decrypted application projections. It is
replaceable and never supplies an identity, key, or transparency anchor, but
its local integrity is inside the operating-system trust boundary: a process
that can rewrite it can rewrite cached plaintext. `kvCacheCheck` establishes
server freshness, not protection against a local database writer. Deployments
that need that stronger local threat model must put the soft database on
authenticated encrypted storage or retain and re-open ciphertext instead.

The client trusts:

- the configured WebPKI roots for the initial probe TLS connection;
- the active delegated TLS CA set authenticated by the hostchain for every
  registration, user, Merkle, team, and KV service connection;
- the cryptographic implementations selected by `foks-crypto` and Rustls;
- the caller's encrypted key store to supply the right device seed; and
- the local process and operating system while plaintext secrets are in use.

SQLite is durable security state, but it is not the only rollback boundary.
Native standalone profiles publish the database's random identity, monotonic
hard-state revision, and per-revision random write token to macOS Keychain or
Linux Secret Service. Every checked profile operation holds both a
cross-process profile lock and a client-root-local lock keyed by database ID,
compares that external watermark before use, and republishes it after the
operation even when the operation reports an error. The native namespace also
claims each database ID for exactly one profile. A database behind the external
revision, with a different identity, with a conflicting profile claim, or with
a different token at the same revision is rejected. A database ahead of the
external checkpoint with a fresh token represents a crash after SQLite commit
and is repaired by advancing the external checkpoint.

The native credential namespace is also bound to the canonical client-state
root path. Copying a state root therefore cannot create a second set of
path-local lock files that shares the same master key and rollback watermark;
the copied root is rejected before either credential or checkpoint use. A
native state root is deliberately immovable. Export/import is not implemented.
The database-ID locks and claims deliberately coordinate only profiles below
that one client root; there is no machine-global or cross-root lock registry.

The explicit private-file credential backend provides neither an external
rollback checkpoint nor copied-database detection. Copying or restoring its
state directory can therefore copy credentials and hard state without an
independent watermark. It is suitable only when the surrounding filesystem or
operator supplies an equivalent protection boundary.

A missing external checkpoint is accepted only when no hard-state database or
SQLite sidecar exists, which is the initial enrollment case. Missing or
inconsistent state otherwise fails closed and reports the explicit
`profile reset-hard-state --confirm-delete` command. That command deliberately
removes both the native checkpoint and hard-state database; deleting only one
must never be used as recovery. Raw network/state operations are exposed only
through `CheckedProfileSession`, so standalone frontends cannot bypass the
lock/checkpoint sequence.

Arbitrary row modification is detected independently: host identity and
service projections are reproduced from the stored hostchain and signed public
zone; Merkle evidence is replayed recursively to its signed bootstrap; and
persisted user and team projections are reproduced from their authenticated
chain evidence. Selectively rewriting SQLite plus its monotonic metadata, or
compromising both the local database and the native credential service, is
outside this minimal whole-database-rollback threat model.

## Invariant ownership

| Invariant | Owning crate | Review entry point |
| --- | --- | --- |
| One canonical encoding, bounded depth, no trailing bytes | `foks-snowpack` | `decode`, `encode` |
| Exact v0.1.9 fields, tags, roles, metadata, and retained signed bytes | `foks-proto` | protocol decoders |
| Type-prefixed hashes, signatures, hybrid key derivation, PUK/PTK unboxing and seed chains | `foks-crypto` | `verify_typed`, `derive_device_public`, `open_puk_seed_chain` |
| Host-chain authority and service delegation | `foks-verify` | `verify_public_host` |
| Merkle signatures, host-chain binding, and skip-path continuity | `foks-verify` | `verify_merkle_advance` |
| User/team chain continuity, suffix proofs, roles, and transition authorization | `foks-verify` | `verify_user_chain`, `verify_user_chain_increment`, `verify_team_chain_increment` |
| Host identity, chain, Merkle, and user rollback/fork rejection | `foks-client-db` | `accept_verified_*` |
| Authenticated TLS roots, endpoint/vhost selection, transaction ordering, KV mutation preconditions, and cache revalidation | `foks-client` | `PinnedHost`, `authenticate_and_pin`, `KvWriteSession` |

Authenticated wire objects retain the bytes that were checked. Parsed fields
are indexes and projections; they are not re-encoded substitutes for signed or
Merkle-committed input. Top-level verified values have private constructors and
fields, so storage code cannot accidentally accept an application-assembled
lookalike.

## Transport lifetime and resource bounds

Pooled connections are partitioned by endpoint, HostID, authenticated TLS-root
fingerprint, a one-way private-key fingerprint, client certificate chain, and
whether virtual-host selection has completed. A private key is also checked
against its certificate on every checkout and is never copied into the pool
key. Connections survive only completely
decoded successful exchanges; transport, framing, sequence, and remote-status
errors drop the connection and are not automatically retried. This is required
because a failed write may have reached the server.

The configured timeout covers DNS completion, TCP attempts, TLS, request write,
and response read for one logical RPC. Standard synchronous DNS resolution
cannot be interrupted mid-call, but an overrun is rejected immediately after
resolution. Once a socket exists, cancellation and deadlines are checked at
most every 250 ms during stalled I/O. Frame limits are configurable from one
byte through 1 GiB; raising the default 16 MiB limit increases the maximum
single response allocation and should be done only for a protocol path that
requires it.

## Secret handling

Device and PUK seeds use `SecretSeed`, an opaque, non-`Clone`, redacted type
whose storage is zeroized on drop. Intermediate shared secrets and private-key
encodings are also held in zeroizing buffers where their dependency APIs allow
it. Encrypted Snowpack plaintexts constructed by `foks-crypto` are encoded
from borrowed fields directly into zeroizing output; seed-bearing plaintexts
are copied into zeroizing storage and redacted before generic schema decoding,
so an ordinary `Value::Binary` never owns the clear seed. SQLite stores only public
identifiers, HEPKs, signatures, ciphertext, and verified history. It never
stores a device seed or clear PUK seed.

Returned PUK/PTK seed sequences are intentionally live secret material. Their
caller must move them into its encrypted credential store or consume them
promptly; logging, serializing, or placing them in ordinary SQLite rows is
outside this crate's contract.

Crash-safe mutations require a caller-supplied `ProtectedMutationStore` backed
by an encrypted credential store, platform keychain, or equivalent durable
secret store. The client commits the exact request and required seeds there
before it creates the public SQLite journal row. That row contains only public
bindings, request and protected-material fingerprints, timestamps, attempt
count, and monotonic state. Protected bytes are fingerprint-checked before the
one allowed submission. Authenticated server reconciliation advances to the
nonterminal `RemoteVerified` state; protected bytes remain until the embedding
application durably commits its credential or projection and explicitly
advances to `Finalized`. A crash before the SQLite row can leave an unreachable
protected record; a crash during final deletion can leave a terminal record
plus orphaned protected material. Neither case permits network replay.

For local deployments, `EncryptedFileMutationStore` implements this contract
with a caller-supplied 256-bit master key, XChaCha20-Poly1305 records bound to
their logical key, synced temporary files, and atomic no-replace installation.
The standalone application obtains and retains the master key in macOS
Keychain or Linux Secret Service; another embedder must provide an equivalent
protected source. Losing or replacing that key makes pending mutations
unrecoverable.
Protected records are capped at 128 MiB to bound allocation when reading a
tampered file. On Unix, the adapter creates private files and refuses to follow
a symbolic link at a record path.

Only `Prepared` may transition to `Submitting`, and only once. `Submitting` and
`SubmissionUnknown` are deliberately indistinguishable for replay purposes:
after restart they may only reconcile against authenticated server state.
`RemoteVerified` is also returned by pending discovery so application-owned
commits can resume without reconstructing or replaying the network mutation.
Software signup, device provisioning and revocation, PUK rotation, and KV
namespace/root changes use this generic journal. `resume_software_account`
also proves that protected seeds derive the journaled UID/device and that the
normalized username and PUK match the authenticated accepted account.

Named-team mutations use a separate public journal with an explicit
`Prepared → Submitting → SubmissionUnknown/Submitted → Verified` lifecycle.
Every edit commits `Submitting` before transport. Recovery first compares the
authenticated team transition at the reserved chain position; a matching
transition verifies the journal and a different transition supersedes it.
Only federation additions retain exact protected request bytes, so only those
can retry after an ambiguous send, and an RPC rejection from that retry is not
treated as proof that the original submission failed.

Federation local preparation has one SQLite owner: the saga's `LocalPrepared`
checkpoint and the corresponding prepared team-mutation row are committed in
the same immediate transaction. The general saga transition API cannot enter
`LocalPrepared`. An exact retry verifies both immutable bindings; a chain-slot
or operation-ID conflict rolls back without advancing the saga.

The scheduler database contains only public host/scope identifiers, timing,
leases, counters, and the handler's last error string. Handlers must not put
credentials, tokens, seeds, or sensitive server responses into that error.
Leases prevent concurrent claims but intentionally expire after crashes, so
scheduled refresh and reconciliation handlers must tolerate repeat execution.

## Commit ordering

The safe authenticated-user flow is:

1. Load a `PinnedHost` from the selected hard-state database.
2. Fetch and verify a Merkle extension from that database's current anchor.
3. Commit the extended Merkle history.
4. Fetch and replay the user chain against those authenticated roots.
5. Verify the enrolled device and unbox its role's current PUK plus the
   authenticated historical seed chain.
6. Commit the public user projection and its Merkle reference atomically.

An interrupted run after step 3 is safe: the cumulative authenticated root
history is durable, and a retry can verify the same user response without
trusting it to supply its own root.

When a user or team projection already exists, step 4 requests only links
after the stored tail and name slots after the stored name sequence. The
verifier binds the suffix's first link to the prior tail hash and prior hidden
tree location, verifies all returned proofs and the next-link absence under
the new root, and replays transitions from the sealed prior state. Hard state
stores the exact initial response and later response segments; reopening
repeats that sequence of full and incremental verification before exposing a
capability.

## Implemented scope and exclusions

Implemented here: non-interactive single-owner software account creation;
non-interactive software-device provisioning and revocation with required PUK
rotation and historical seed-chain preservation; standalone software PUK
rotation over a complete role prefix; single-owner ad-hoc and named-team
creation with four generation-1 PTKs and authenticated creator membership;
same-host named-team user additions under authenticated open-viewership policy;
same-host named-team user removal plus generalized removal, demotion, and
member-key-generation changes for local, nested-team, and federated members,
with TeamAdmin removal-key retrieval, mandatory PTK rotation, and historical
seed-chain preservation;
public probe; hostchain-delegated TLS CAs and virtual-host
selection; host, Merkle, user, and team-chain verification; certificate
retrieval for already enrolled software and Yubi subkeys; device mTLS;
role-correct PUK and PTK unboxing including historical seed chains; hard-state
persistence; and authenticated read/write personal/team KV traversal with an
isolated monotonic soft SQLite projection. KV synchronization persists exact
path-version vectors, submits `kvCacheCheck` before using cached rows, replaces
only stale directories when the root is unchanged, prunes newly unreachable
subtrees atomically, and retries bounded cache races. One selected mTLS
connection carries the complete synchronization exchange. Large-file
plaintext is committed as bounded SQLite chunks and is exposed through a
streaming writer rather than a whole-file allocation.

The account path reserves and normalizes the username, binds a fresh device
and generation-1 owner PUK to the current verified Merkle root, constructs the
official stacked PUK/device eldest signatures and initial hybrid PUK box,
submits `Reg.signup`, obtains the new mTLS certificate, waits for and verifies
the Merkle/user-chain projection, seals public hard state, and creates the
initial personal KV root. The software path supports an Ed25519 eldest and an
optional PPE passphrase established atomically at signup. The parallel Yubi
path builds the v0.1.9 P-256 eldest and encrypted delegated Ed25519 mTLS
subkey, binds both retired-key slots and their public keys to application
durable state, and supports the same optional signup passphrase. SSO and
passphrase-only device recovery setup remain outside this slice.

The device-mutation convenience path requires the owner device and new
software-device seeds in one process so both exact signatures can be built.
It does not implement FOKS's interactive device-to-device KEX. Revocation
rejects self-revocation and can distribute rotated PUKs only to Curve25519
software recipients; accounts whose remaining eligible credentials include
Yubi/P-256 keys therefore fail closed. Owner-PUK rotations query server PPE
state and include the next encrypted passphrase annex in the same mutation. A
lost provision response can be reconciled with the caller-retained seed through
the public certificate-fetch and authentication methods; mutation secrets are
never placed in SQLite. If a caller supplies `NoPassphraseConfigured`, the client
still confirms absence through `User.getPpeParcel`; a stale assertion fails
closed rather than orphaning PPE history. Generic user-settings links are not
part of the verified projection, so Go-server interoperability for PPE annexes
on user mutations remains narrower than the standalone Rust path.

Passphrase enrollment/change uses the upstream V1 Argon2id and PPE box formats.
The raw input and stretched credential are redacted and zeroized locally, and
public login binds a one-time challenge to the expected UID and HostID before
the encrypted SKMWK history is accepted. This is not a local-vault lock: the
active device is still needed to fetch PPE metadata, and no passphrase-only
device provisioning or recovery ceremony is exposed. The client reconciles an
ambiguous set/change response by exact parcel readback while the process is
alive. It does not durably journal the intended PPE generation and boxes, so a
process death in the post-commit response window requires verifying the desired
passphrase before another change; blindly retrying a change can create another
PPE generation.

Ad-hoc team creation requires an enrolled software or Yubi owner device and
the current owner PUK. A software device signs the membership link directly;
a Yubi parent signs the exact SHA-512/256 FOKS digest with P-256 and its
software subkey remains mTLS-only. The returned DER signature is verified
against the authenticated parent EntityID before use. The caller must persist
all four PTK seeds before posting; the stable TeamID is derived from the admin
PTK and is available before the network call for reconciliation. A public-only
hard-state journal binds an operation ID to the host, user, parent device,
TeamID, and exact request hash before submission; resume never replays the
mutation. The response is not trusted as success: the client repeatedly loads
the predicted team, verifies its Merkle proof and chain, opens exactly the
expected PTKs, and seals the public team projection.
The authenticated `User.getHostConfig` response must advertise open user
viewership before a request is journaled. Structured server rejection is
definitive and is not followed by polling; transport ambiguity is reconciled,
and only the explicit transaction-retry status is reposted once with identical
bytes. The host remains authoritative for its open-viewership policy. v0.1.9
ad-hoc membership is fixed at founding, so this API exposes no misleading edit
path.

Named-team creation additionally reserves the normalized name, commits the
name and one caller-retained removal key in the eldest link, boxes that removal
key independently to the admin PTK and creator PUK, and emits an `Approved`
membership link carrying the same commitment. Official Go v0.1.9 builders and
RPC encoding provide differential fixtures for the eldest link, membership
link, reservation, removal boxes, and complete request. The public journal
excludes the reservation token, name-commitment key, removal key, PTKs, and
hidden tree locations. This slice supports one local owner only.

Named-team addition accepts only a sealed, verified same-host user state and
the target's current owner PUK. The actor must be exactly one unscoped admin or
owner in the verified team roster, cannot grant above its own role, and can
only grant a role with an existing PTK. The signed link contains exactly one
addition and no PTK rotation. The caller-retained removal key is committed in
that link, dual-boxed to the current admin PTK and target PUK, and excluded
from SQLite. Reconciliation replays the authenticated team chain and checks
the exact expected link rather than trusting the latest roster projection.
FOKS's closed-viewership three-way invitation flow is intentionally absent.

Named-team removal and downgrade accept exactly one member already present in
the authenticated roster. A caller-retained removal key may be used by the
legacy local-user API; the general path instead activates a short-lived
TeamAdmin bearer token with the current admin/owner PTK and opens the returned
historical admin box. In either case, the key must reproduce the member's
signed commitment and its encrypted metadata must bind team, member, host
scope, and source role. The actor must be an unscoped admin or owner with
authority over the target, and sealed verified user/team states must exactly
cover the post-transition roster. The rotation role set is derived from the
pre-transition PTKs, old destination role, new destination role, and source
credential generation; skipped, duplicated,
reordered, surplus, unchanged, or key-reusing replacements fail before
journaling. New PTKs are distributed only to
remaining members whose roles can read them, while each old generation is
secret-boxed under its replacement for authorized history. The link's stacked
signatures, off-chain box set, seed chain, and removal MAC have byte-exact Go
v0.1.9 fixtures. Reconciliation binds the target, scoped host, source and
destination roles, replacement PUK/PTK generation and verify key, introduced
PTK verify keys and generations, and expected chain position. A unique public
journal lookup allows reconciliation after a TeamAdmin-loaded removal key has
already left memory.

KV namespace writes use the complete accepted path-version vector as the
server precondition. The canonical vector and exact authenticated dirent bytes
are protected outbox material; the public journal binds their fingerprint,
party, parent, and root version. Explicit stale-cache responses terminally
reject that attempt and trigger at most three freshly prepared targeted
refresh-and-retry attempts; transport failures are never blindly replayed.
Content objects are uploaded before their authenticated
dirent is linked, as required by v0.1.9; an interrupted or raced operation can
therefore leave an unreachable encrypted object for server-side collection,
but cannot expose a partially linked plaintext name. After a successful
dirent/root mutation the client immediately runs cache-check synchronization.
If that synchronization or the response fails, the operation remains pending;
its public resume method performs targeted authenticated synchronization and
accepts only the exact root or dirent transition. Large uploads are bounded to
FOKS's 1 GiB limit and retain at most one 4 MiB cleartext chunk plus one-byte
lookahead in memory. Content-object creation and upload chunks are not replayed
after process loss; callers restart the stream, possibly leaving unreachable
encrypted objects. Only the subsequent namespace link is authoritative and
durably recoverable.

Backup-key account recovery is implemented through the exact v0.1.9 HESP,
registration lookup, PUK unboxing, and loopback provision semantics. The raw
26-byte phrase seed is caller-owned and must be recorded before enrollment.
Its first word/number pair is disclosed as the device name, leaving 179 secret
bits from the 203-bit encoding after enrollment. New keys should be made with
`BackupKey::generate` rather than caller-assembled random bytes.
Loading consumes it and retains only a derived zeroizing credential in a
non-serializable value; successful recovery returns a normal caller-durable
software credential. Backup keys are not written to SQLite. Losing both every
permanent device and every enrolled backup phrase remains unrecoverable.

The bounded federation slice treats Beacon answers as untrusted routing,
authenticates and pins the requested remote HostID directly, issues exact
remote-view permissions, verifies remote public user/team chains, and can add
a scoped remote team through a durable client-side saga. Public scheduler rows
are wake-up identities only: an encrypted membership record must reproduce the
job ID before aliases, roles, or credentials are used. Bearers and removal keys
never enter SQLite. Recurring federation jobs renew and verify the already
admitted bearer only; they do not re-enter the one-time admission saga or edit
the local team chain.

Not yet implemented: additional founding members; promotion/addition through
closed-viewership invitation and remote-join protocols; federated trust
administration or push propagation; direct remote-user membership; automatic
cross-team PTK rotation after remote roster/key changes; CLKR; Git;
chat/realtime; passphrase-based device recovery; SSO; or a full federated FOKS
server. These omissions should fail by absence, not by permissive fallbacks.

## Testing strategy

The checked-in fixtures are generated by the official Go v0.1.9 code and are
used as a differential oracle. Snowpack tests cover every MessagePack marker,
wire boundaries, nesting limits, arbitrary inputs, canonical uniqueness, and
recursively shrinkable generated values. Verifier tests mutate signatures,
roots, proofs, metadata, roles, and history. SQLite tests exercise rollback,
fork, projection, atomicity, and reopen behavior. The local Rustls integration
test covers host-selected registration followed by authenticated user, Merkle,
and PUK RPCs without a browser or interactive input. A separate official Go
oracle creates the full software-signup link, box set, hidden values, and
byte-exact registration RPC frames entirely from the command line.
The command-line live compatibility harness starts the official v0.1.9 Go
server with PostgreSQL, then drives native Rust probe, registration, user/PUK
authentication, root creation, KV write, and incremental SQLite projection.
Deterministic mutation tests place a lost-response proxy after authoritative
acceptance and restart at every durability boundary, including protected-only,
Prepared, Submitting, SubmissionUnknown, RemoteVerified, and
Finalized-before-cleanup states.
