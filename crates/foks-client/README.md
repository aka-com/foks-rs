# foks-client

Native, non-interactive discovery and read/write access for FOKS v0.1.9. The
initial probe uses configured WebPKI roots; delegated services use the active
TLS CAs authenticated by the hostchain and select the pinned virtual host when
the upstream protocol requires it.

This crate is the protocol client library. The standalone application
composition, direct `foks-rs` executable, bounded local agent, and
toolkit-independent desktop boundary live in `foks-client-app`, `foks-cli`,
`foks-agent`, and `foks-desktop`. They use explicit state paths and do not
depend on AKA crates. Run their isolated gate with
`tools/foks-client/check.sh`.

Delegated RPC connections are pooled by endpoint, authenticated HostID/TLS
roots, client certificate identity, and virtual-host selection state. RPC
sequence numbers advance on reuse without rewriting signed protocol arguments;
any incomplete or invalid exchange evicts its connection and is never
automatically replayed. The idle pool defaults to two connections per key and
16 total and can be bounded or disabled with `set_connection_pool_limits`.
`set_timeout` is an overall connect/TLS/request deadline rather than a fresh
timeout for every socket read, and `CancellationToken` interrupts stalled I/O
at bounded polling intervals. Response allocation is controlled by
`set_maximum_frame_length`, capped at 1 GiB.

The client can obtain an enrolled device certificate without interaction,
derive its exact Ed25519 mTLS key, advance the signed Merkle pin through the
official historical-root protocol, replay device and PUK transitions, unbox
the enrolled device role's current and historical PUK generations, and commit
the public user projection atomically. It supports software and Yubi-backed
credentials, verified team chains and PTKs, and complete authenticated KV
projections in an isolated soft SQLite database. Persistent
path-version-vector checks avoid unchanged downloads and target stale
directories without discarding the rest of the verified cache. KV RPCs reuse
selected pooled mTLS connections, and large-file plaintext streams through
bounded SQLite chunks rather than a whole-file memory buffer. `KvWriteSession` adds
fresh-root initialization, streamed file and symlink puts, mkdir, unlink,
atomic move, and server lock operations. Namespace writes carry the persisted
version vector as an optimistic precondition and converge SQLite through the
same targeted incremental synchronization path after success. Merkle
advancement always starts from SQLite hard state, so an untrusted response can
never bless its own root.

User and team refreshes also start at the last authenticated chain sequence
and send the v0.1.9 current-name cursor. Only the returned suffix is replayed:
its first `previous` hash and hidden tree location must extend the stored tail,
every link and terminal absence must verify under an independently
authenticated Merkle root, and name proofs follow the upstream reassignment
rules. Exact response segments are retained so restart restoration replays the
same initial chain plus each authenticated suffix rather than trusting the
materialized SQLite projection. Consecutive no-link refresh evidence is
compacted to the latest root, so storage grows with chain changes rather than
with refresh frequency.

Signup, software-device provisioning and revocation, PUK rotation, and KV
namespace/root mutations use a generic hard-state write-ahead journal. Exact
retry material is committed first through the caller's `ProtectedMutationStore`
and is fingerprint-bound to the public SQLite row. `Prepared` is the only state
that may submit; once an operation becomes `Submitting`, recovery only compares
authenticated server state and does not replay an ambiguous request.
`EncryptedFileMutationStore` is the production local adapter: the standalone
application supplies its 32-byte master key from Keychain or Secret Service,
while the adapter atomically stores individually authenticated
XChaCha20-Poly1305 records in a private directory. The master key is never
written alongside those records.

`FoksScheduler` persists due times, bounded leases, failures, and retry state in
the public hard-state database for user refresh, federation capability refresh, and ambiguous-mutation
reconciliation. It deliberately owns neither an async runtime nor credentials:
the application timer invokes `run_due` on its blocking storage worker and
dispatches each public job identity to its protected account context. Jobs must
be idempotent because an expired lease is retried after a process crash.

That hard state detects inconsistent modification, rollback relative to the
database's retained pins, and same-sequence forks. The application layer also
binds its database ID, monotonic hard-state revision, and per-revision random
write token to a native Keychain or Secret Service checkpoint. A native
database ID is claimed by one profile and locked within its client root during
checked operations, rejecting whole-database rollback, substitution, and
equal-revision copies. The private-file backend does not provide this
copied-state or external rollback protection.
The protocol crate remains usable without that application policy, so embedders
must supply an equivalent external checkpoint if they open hard state directly.

`create_software_account` implements the narrow non-interactive registration
slice: one software owner device, its generation-1 owner PUK, verified
post-signup user loading, public mutation journaling, and initial personal KV
root creation. It can also establish a v0.1.9 PPE passphrase in the same signup
transaction and verify it through the public challenge protocol. It preflights
optional standard or multi-use invites through
`Reg.checkInviteCode`; the server still performs authoritative atomic
redemption during signup. The supplied `ProtectedMutationStore` must be a durable encrypted
credential store in production; the client records the exact signup request,
seeds, and self token there before journaling or submission. Those secrets are
never written to the hard-state database.

`provision_software_device` and
`revoke_user_credential_with_software_device` implement the non-interactive
software-signer mutations. Provisioning constructs the new-device
countersignature and delivers the accessible PUK to the new credential;
introducing a new role also distributes its generation-1 PUK to all eligible
existing software devices. Revocation rotates every PUK visible to the removed
credential, preserves the encrypted historical seed chain, and waits for the
exact user-chain transition before reporting success. Callers must durably
retain every supplied seed before submission. Physically separated software
countersigning is available through `publish_kex_provision_offer`,
`finish_kex_provisioning`, and `accept_kex_provisioning`; these use the v0.1.9
13-token HESP phrase and public headerless KEX relay while keeping the final
mutation in the durable WAL. Yubi/P-256 mutation recipients and self-revocation
are not yet exposed by these convenience APIs. When an owner PUK rotates, the client
queries passphrase state and atomically appends the required PPE annex; an
explicit `NoPassphraseConfigured` token is accepted only after the server
confirms that no passphrase exists. Rust-to-Go standalone set/change requests
use the upstream wire methods but do not publish the Go client's separate
generic user-settings links; PPE annexes on user mutations therefore remain a
known cross-server compatibility boundary until that chain family is
implemented.

`create_yubi_account` and `provision_yubi_device` build the corresponding
P-256 parent plus delegated Ed25519 mTLS credential. A fresh, host-bound,
single-use parent signature recovers the server-held encrypted subkey, while
PUK-encrypted management-key envelopes let an alternate software owner restore
PIV administration after local loss or PIN lockout. The application crate
adds exact-card locators, crash-safe management-key replacement, PIN/PUK/retry
administration, scheduled envelope refresh, and the supported revocation
sequence. Rotating a PUK to another already-enrolled Yubi recipient remains
unsupported by the software-signer convenience API; revoke the old YubiKey
before provisioning its replacement.

`set_passphrase`, `change_passphrase`, and `verify_passphrase` implement the
authenticated PPE lifecycle. V1 Argon2id runs in zeroizing client memory; the
server receives only public verification material and encrypted boxes. Change
uses the current owner-PUK backup box, matching v0.1.9, so it does not ask for
the old passphrase. Verification signs a one-time public registration challenge
and decrypts the returned SKMWK history locally. This verifies PPE state; it
does not lock the local credential vault or implement passphrase-only
new-device recovery. Set/change reconcile a lost or partial post-commit RPC
response in-process by reading the stored parcel back and requiring an exact
match. They do not yet journal the intended PPE update in protected durable
client state: if the client process dies after the server commit, the user must
inspect or verify the active passphrase before deciding whether to rotate it
again rather than repeating a change unverified.

`rotate_software_puks` performs the corresponding membership-preserving
operation. Its input must be a complete ordered role prefix through the
highest PUK being rotated; it rejects skipped roles, stale previous seeds,
unchanged or duplicated replacements, and non-owner signers before posting.
Like revocation, it distributes the new generations, retains encrypted history,
and returns only after the exact public-key transition is authenticated.

`enroll_backup_key`, `load_backup_key`, and `recover_software_device` implement
FOKS v0.1.9's account-recovery path. A backup key is the official 17-token
HESP encoding of a 203-bit seed and is enrolled as its own Curve25519/ML-KEM
user credential. The caller must record the phrase before enrollment; neither
SQLite nor this client retains it. Loading signs the host-bound registration
challenge, locates the owning user, verifies the pinned Merkle/user chain, and
unboxes the backup role's PUK. The loaded value contains only derived
zeroizing key material, is intentionally non-serializable, and is consumed
when it provisions a permanent software device. Recovery is idempotent for a
caller-retained device seed and role, so a retry can reconcile a committed
provision whose response or certificate fetch was lost. Backup enrollment is
likewise idempotent for the same key and role. The complete workflow is ordinary
probe/registration/user RPC and requires no browser or interactive KEX.
Passphrase-based device recovery is a separate upstream protocol and is not
part of this slice.

The first word/number pair becomes the public backup-device name, leaving 179
bits secret after enrollment. Use `BackupKey::generate`; `from_seed` exists for
import and deterministic fixtures, not as an entropy source. An enrolled
backup key can be removed with
`revoke_user_credential_with_software_device`.

`create_single_owner_adhoc_team` and its Yubi variant create the narrow
non-interactive team slice: one local owner, generation-1
member-min/member/admin/owner PTKs, the stacked team eldest signatures, PTK
delivery to the owner's current PUK, and the creator's immutable
`ApprovedAdHoc` membership link. Software devices sign that link with their
Ed25519 device key. Yubi credentials sign it with the P-256 parent while the
delegated software subkey remains mTLS-only. The caller must store all four PTK
seeds before submission. `AdHocTeamSecrets::team_id`
derives the stable TeamID locally, so a lost response can be reconciled through
the matching software or Yubi resume method; a public-only monotonic journal
records the prepared/submitted/verified transition without retaining seeds or
hidden tree locations. Its operation ID is deterministically recoverable from
the retained admin PTK, so an ambiguous error cannot lose the resume handle.
PTK inputs are role-keyed rather than positional. FOKS v0.1.9 requires open
user viewership for ad-hoc creation; the client now checks the authenticated
host configuration before preparing or journaling a request. Explicit server
rejection returns immediately, while transport ambiguity enters verified
reconciliation. The official transaction-retry status is retried once with
byte-identical signed input.

`create_single_owner_named_team` and its Yubi variant add exact normalized-name
reservation and commitment, the same four generation-1 PTKs, an owner removal
key committed in the eldest link and independently boxed to the admin PTK and
creator PUK, and the creator's `Approved` membership link. The caller must
durably retain the PTKs, removal key, and name-commitment key before the call.
The public journal stores only the actor, team, expected sequence, and exact
request fingerprint.

`add_local_user_to_named_team` and its Yubi variant implement the exact local
open-viewership addition path. The target must be a verified same-host user;
the signed team link binds its current owner PUK, destination role, and a
caller-retained removal-key commitment. Existing PTKs visible to that role are
boxed to the target PUK without rotating them, and the removal key is boxed
independently to the team admin PTK and target PUK. The caller must persist the
removal key before submission. The public mutation journal supports
reconciliation without replay, and reconciliation authenticates the exact
expected chain position even if later team links have already landed.

`remove_local_user_and_rotate_ptks` and its Yubi variant cover the matching
local-user removal path. The caller supplies the member's retained removal key,
every newly generated PTK required by the authenticated FOKS gameplan, and
verified public user states for the remaining roster. The client rotates every
PTK visible to the removed role, boxes each new generation only to eligible
remaining members, chains the old generations under the new keys, constructs
the committed removal MAC, and reconciles the exact signed transition.

`remove_team_member_and_rotate_ptks` retrieves that same committed key through
an exact, short-lived TeamAdmin bearer token, so callers do not need to retain
it. `change_team_member_and_rotate_ptks` generalizes the authenticated rotation
path to role demotions, PUK/PTK generation advances, nested-team members, and
federated users or teams. Callers supply sealed verified user/team states for
the complete post-transition recipient roster; remote host scope and source
role are part of every lookup, encrypted PTK parcel, removal MAC, operation ID,
and reconciliation check. The resume API locates the unique public journal row
by team and expected sequence, so an interrupted server-loaded removal does not
need the removal key or a returned secret-derived operation ID.

Federation support is an intentionally client-coordinated slice. A Beacon is
an untrusted HostID-to-address hint; `discover_and_pin` accepts it only after a
direct probe authenticates the requested HostID. Remote user/team grants use
the exact v0.1.9 bearer wire, remote public chains are independently verified
and pinned, and `admit_remote_team_to_named_team` journals a secret-free
cross-host saga before constructing the local membership edit. The application
stores the removal key and stable membership binding in its encrypted vault.
This standalone workflow is deliberately operator-mediated: one operator must
hold credentials for both profiles, the remote team's source role is pinned to
`ADMIN`, and the client constructs the join RSVP needed by the server-side
tuple. It is **not** Go v0.1.9's three-party invite/RSVP/inbox consent flow and
must not be presented as a wire-compatible implementation of that flow.

The hash domain `0x45cf32f37d38a811` is used only for one-way local
storage/journal identity of the federation permission bearer. It is a
Rust-standalone implementation detail, never a v0.1.9 Snowpack type ID or wire
surface. This slice does not implement federated trust policy, push
propagation, remote-user membership, join inboxes, or cross-host PTK rotation
after a remote roster/key change.

See [SECURITY.md](SECURITY.md) for the trust boundaries, invariant ownership,
secret lifecycle, review order, and explicitly unimplemented surfaces.
