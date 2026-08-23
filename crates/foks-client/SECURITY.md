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

SQLite is durable security state, but it is not a source of protocol truth.
An attacker who can roll back both the database and every external backup can
present an old, internally consistent view. Detecting that requires a newer
pin retained outside the rolled-back state.

Hard-state durability is therefore conditional on retaining the database (or
one of its backups) across runs. Arbitrary row modification is detected: host
identity and service projections are reproduced from the stored hostchain and
signed public zone; Merkle evidence is replayed recursively to its signed
bootstrap; and persisted user and team projections are reproduced from their
authenticated chain evidence. The accepted projection and its authenticating
pin are committed in one SQLite transaction. There is currently no second pin
in a platform keychain, transparency witness, or remote backup, so replacement
of the complete database with an older valid copy remains indistinguishable
from restoring that copy intentionally.

## Invariant ownership

| Invariant | Owning crate | Review entry point |
| --- | --- | --- |
| One canonical encoding, bounded depth, no trailing bytes | `foks-snowpack` | `decode`, `encode` |
| Exact v0.1.9 fields, tags, roles, metadata, and retained signed bytes | `foks-proto` | protocol decoders |
| Type-prefixed hashes, signatures, hybrid key derivation, PUK/PTK unboxing and seed chains | `foks-crypto` | `verify_typed`, `derive_device_public`, `open_puk_seed_chain` |
| Host-chain authority and service delegation | `foks-verify` | `verify_public_host` |
| Merkle signatures, host-chain binding, and skip-path continuity | `foks-verify` | `verify_merkle_advance` |
| User-chain continuity, proofs, roles, and transition authorization | `foks-verify` | `verify_user_chain` |
| Host identity, chain, Merkle, and user rollback/fork rejection | `foks-client-db` | `accept_verified_*` |
| Authenticated TLS roots, endpoint/vhost selection, transaction ordering, KV mutation preconditions, and cache revalidation | `foks-client` | `PinnedHost`, `authenticate_and_pin`, `KvWriteSession` |

Authenticated wire objects retain the bytes that were checked. Parsed fields
are indexes and projections; they are not re-encoded substitutes for signed or
Merkle-committed input. Top-level verified values have private constructors and
fields, so storage code cannot accidentally accept an application-assembled
lookalike.

## Secret handling

Device and PUK seeds use `SecretSeed`, an opaque, non-`Clone`, redacted type
whose storage is zeroized on drop. Intermediate shared secrets and private-key
encodings are also held in zeroizing buffers where their dependency APIs allow
it. SQLite stores only public identifiers, HEPKs, signatures, ciphertext, and
verified history. It never stores a device seed or clear PUK seed.

Returned PUK/PTK seed sequences are intentionally live secret material. Their
caller must move them into the encrypted AKA key store or consume them
promptly; logging, serializing, or placing them in ordinary SQLite rows is
outside this crate's contract.

Software account creation requires the caller to persist its device seed, PUK
seed, and self token in that encrypted store before submission. The hard-state
signup journal contains only public identifiers, normalized username, an
exact-request hash, timestamps, and monotonic prepared/submitted/verified
state. It is therefore useful for reconciliation but cannot recover a lost
credential or retry a request without the caller's protected material.
`resume_software_account` never replays signup: it proves that the supplied
seeds derive the journaled UID/device, reloads the accepted account, and
idempotently loads or creates the initial KV root. This covers a lost signup
response as well as interruption between account and KV creation.

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

## Implemented scope and exclusions

Implemented here: non-interactive single-owner software account creation;
non-interactive software-device provisioning and revocation with required PUK
rotation and historical seed-chain preservation; standalone software PUK
rotation over a complete role prefix; single-owner ad-hoc team creation with
four generation-1 PTKs and an immutable creator membership link;
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
initial personal KV root. It supports only a software eldest credential with
no passphrase, Yubi parent, SSO, or recovery setup.

The device-mutation convenience path requires the owner device and new
software-device seeds in one process so both exact signatures can be built.
It does not implement FOKS's interactive device-to-device KEX. Revocation
rejects self-revocation, omits the passphrase annex, and can distribute rotated
PUKs only to Curve25519 software recipients; accounts whose remaining eligible
credentials include Yubi/P-256 keys therefore fail closed. A lost provision
response can be reconciled with the caller-retained seed through the public
certificate-fetch and authentication methods; mutation secrets are never
placed in SQLite.
Because passphrase configuration is not part of the verified user projection,
an owner-PUK mutation additionally requires the caller to supply the explicit
`NoPassphraseConfigured` capability. The API cannot construct or post an owner
rotation when that assertion is omitted.

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

KV namespace writes use the complete accepted path-version vector as the
server precondition. Explicit stale-cache responses trigger at most three
targeted refresh-and-retry attempts; transport failures are never replayed.
Content objects are uploaded before their authenticated
dirent is linked, as required by v0.1.9; an interrupted or raced operation can
therefore leave an unreachable encrypted object for server-side collection,
but cannot expose a partially linked plaintext name. After a successful
dirent/root mutation the client immediately runs cache-check synchronization.
If that final synchronization fails, the remote mutation may already be
committed and callers must treat the result as indeterminate and resynchronize
before retrying. Large uploads are bounded to FOKS's 1 GiB limit and retain at
most one 4 MiB cleartext chunk plus one-byte lookahead in memory.

Not yet implemented: named-team creation; additional ad-hoc founding members;
team membership or credential management; PTK rotation; beacon resolution;
federation; CLKR; Git; chat/realtime; account recovery; SSO; a production
encrypted key-store integration; or a FOKS server. These omissions should fail
by absence, not by permissive fallbacks.

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
