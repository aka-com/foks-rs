# foks-client

Native, non-interactive discovery and read/write access for FOKS v0.1.9. The
initial probe uses configured WebPKI roots; delegated services use the active
TLS CAs authenticated by the hostchain and select the pinned virtual host when
the upstream protocol requires it.

The client can obtain an enrolled device certificate without interaction,
derive its exact Ed25519 mTLS key, advance the signed Merkle pin through the
official historical-root protocol, replay device and PUK transitions, unbox
the enrolled device role's current and historical PUK generations, and commit
the public user projection atomically. It supports software and Yubi-backed
credentials, verified team chains and PTKs, and complete authenticated KV
projections in an isolated soft SQLite database. Persistent
path-version-vector checks avoid unchanged downloads and target stale
directories without discarding the rest of the verified cache. KV RPCs reuse
one selected mTLS connection, and large-file plaintext streams through bounded
SQLite chunks rather than a whole-file memory buffer. `KvWriteSession` adds
fresh-root initialization, streamed file and symlink puts, mkdir, unlink,
atomic move, and server lock operations. Namespace writes carry the persisted
version vector as an optimistic precondition and converge SQLite through the
same targeted incremental synchronization path after success. Merkle
advancement always starts from SQLite hard state, so an untrusted response can
never bless its own root.

That hard state detects inconsistent modification, rollback relative to the
database's retained pins, and same-sequence forks. It cannot detect replacement
of the entire database with an older, completely valid copy. Deployments that
need that guarantee must retain a newer checkpoint outside SQLite—for example
in a platform keychain, an external transparency pin, or an independently
protected backup—and compare it when opening the database.

`create_software_account` implements the narrow non-interactive registration
slice: one software owner device, its generation-1 owner PUK, verified
post-signup user loading, public mutation journaling, and initial personal KV
root creation. The caller must durably store the supplied seeds and self token
in an encrypted credential store before calling it; those secrets are never
written to the hard-state database.

`provision_software_device` and `revoke_software_device` implement the
non-interactive software-credential mutations. Provisioning constructs the
new-device countersignature and delivers the accessible PUK to the new
credential; introducing a new role also distributes its generation-1 PUK to
all eligible existing software devices. Revocation rotates every PUK visible
to the removed credential, preserves the encrypted historical seed chain, and
waits for the exact user-chain transition before reporting success. Callers
must durably retain every supplied seed before submission. Physically
separated countersigning/KEX, passphrase annex updates, Yubi/P-256 mutation
recipients, and self-revocation are not yet exposed by these convenience APIs.
Any operation that reaches the owner PUK requires an explicit
`NoPassphraseConfigured` assertion, so omission of the passphrase annex cannot
be accidental.

`rotate_software_puks` performs the corresponding membership-preserving
operation. Its input must be a complete ordered role prefix through the
highest PUK being rotated; it rejects skipped roles, stale previous seeds,
unchanged or duplicated replacements, and non-owner signers before posting.
Like revocation, it distributes the new generations, retains encrypted history,
and returns only after the exact public-key transition is authenticated.

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
byte-identical signed input. Named teams, additional founding members, later
membership edits, and PTK rotation remain outside this slice.

See [SECURITY.md](SECURITY.md) for the trust boundaries, invariant ownership,
secret lifecycle, review order, and explicitly unimplemented surfaces.
