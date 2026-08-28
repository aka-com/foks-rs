# FOKS key rotation architecture

This document fixes the rotation model before the first durable deployment.
It separates two operations with different trust and compatibility effects:

- **operator-root rotation** changes only local encryption-at-rest;
- **host-key rotation** changes public FOKS hostchain state and must be observed
  through Merkle roots and client host synchronization.

The two operations must never share a command or transaction state.

## Operator-root rotation

Status: implemented by `foks-server rotate-operator-root`.

The 32-byte operator root encrypts one randomly generated installation key
encryption key (KEK) in `key-encryption.key`. Every purpose key is encrypted by
the KEK and retains its independently random 16-byte generation ID. Rotation
therefore rewrites one small envelope rather than seven purpose-key files:

1. open and authenticate `key-encryption.key` with the old root;
2. write and `fsync` a private temporary envelope under the new root;
3. read back, authenticate, and compare the staged KEK;
4. atomically rename it over `key-encryption.key` and `fsync` the directory;
5. only then replace the externally managed root-key secret.

A crash before the rename leaves the old root valid. A crash after the rename
leaves the new root valid. Purpose key bytes, public keys, generation IDs,
database state, hostchain state, certificates, backups, and client pins do not
change. The old root must be retained until a backup made after rotation has
been restored and started in an isolated drill.

The command takes an exclusive process lock and fails while a server, backup,
or another rotation has the key directory open. This makes the required order
explicit: stop the server, rotate the envelope, atomically update the external
root secret, and then restart. The lock is advisory to FOKS processes; directory
ownership and operating-system permissions remain part of the security boundary.

Backups contain the encrypted KEK and encrypted purpose keys, never the
operator root. Restoring a pre-rotation backup needs the old root; restoring a
post-rotation backup needs the new root.

## Host-key rotation

Status: implemented for the public host signing key by
`begin-host-key-rotation` and `complete-host-key-rotation`. The commands are
offline and mutually exclusive with a running server or backup. Delegated-key
rotation remains design-only.

The immutable host ID remains the genesis `ENTITY_HOST`. Rotating signing
authority adds a new `ENTITY_HOST` key to the hostchain; it does not invent a
new host ID. Delegated metadata, Merkle, TLS-CA, client-CA, recovery, and
capability keys rotate independently by purpose.

### Persistent model

Add these records in the first shipped schema, before any migration promise:

```text
host_key_generations
  purpose, generation_id, encrypted_file_name, public_entity_id,
  state(staged|active|retiring|revoked), created_at, activated_hostchain_seqno

host_rotation_operations
  operation_id, purpose, old_generation_id, new_generation_id,
  phase(staged|published|complete), add_link_seqno, add_published_at,
  revoke_link_seqno, created_at, updated_at
```

The current generation is selected by the database ledger, not by a mutable
filesystem pointer. All generation files are immutable and authenticated by
purpose plus generation ID. Startup loads the database-selected generations,
verifies their public IDs against the complete stored hostchain, verifies the
current signed Merkle root commits to the hostchain tail, and fails closed on a
missing, duplicate, or extra-active generation.

The database transaction that advances a rotation also writes the exact
hostchain link, its hash, the newly signed Merkle root, the current-root head,
the updated probe blob, and the generation-ledger state. No network response is
sent until that transaction commits.

### Host signing-key sequence

Use two links so loss of the new key does not immediately destroy the only
online signer:

1. Stage and verify the new encrypted generation without changing current
   service state.
2. Append an add-key link containing `Key(new_host_entity)`. FOKS requires the
   link signatures in change order, so it is signed first by the new key and
   then by the current host signer.
3. Publish a Merkle root committing to the add-key hostchain tail and serve the
   full hostchain in probe and authenticated Merkle responses.
4. Keep both host signers active for at least 24 hours. Completion requires an
   explicit acknowledgement of the exact add-link sequence observed by a
   separately pinned client or canary.
5. Append a separate `Revoke(old_host_entity)` link signed by the new active
   host key, then publish another bound Merkle root.
6. Mark the old generation revoked in the same database transaction. After
   commit, remove and directory-sync its encrypted private-key file. A retry
   completes this idempotent cleanup after a crash. Public generation IDs,
   entities, and exact hostchain bytes remain in SQLite for historical audit;
   revoked private-key files are not required at startup or copied to backups.

The rotation root advances from the latest authoritative Merkle head, even
when user, team, or KV publications occurred after the previous probe root.
Clients persist the later signed probe as a signed refresh that retains and
revalidates the prior Merkle evidence needed by existing user/team snapshots.
They update the hostchain pin from probe before requesting a current root whose
hostchain tail contains the new add/revoke links.

Rollback before step 2 deletes only a staged generation. After step 2, rollback
is another hostchain operation, never database or file restoration. Once an
entity is revoked it is not reused.

### Delegated-key sequence

Metadata and Merkle signer rotations use the same add/observe/revoke shape with
their entity types. During overlap, new public-zone blobs and Merkle roots are
signed by the newly activated key, while verification accepts every active key
in the hostchain.

Delegated TLS rotation adds a `TlsCa` item carrying the new CA certificate,
publishes a server certificate under it, waits through at least the maximum
client cache/connection lifetime, and only then revokes the old TLS-CA entity.
The listener must present a chain clients can validate from an active
hostchain certificate throughout the overlap.

Client-CA rotation affects authenticated device certificates rather than the
public hostchain. The server trust store accepts old and new client CAs during
overlap; newly issued certificates use the new CA; retirement waits until every
active device certificate is replaced or explicitly revoked.

Recovery and capability keys rotate with versioned envelopes and dual-read,
single-write semantics. Existing recovery boxes or capability tokens remain
readable until their explicit expiry/reencryption completion gate.

### Enforced release gates

- `run-host-rotation-compat.sh` generates exact Rust add/revoke probes and has
  the checksum-pinned Go v0.1.9 implementation replay the chains, verify all
  signatures, verify the public zone and Merkle root, and check the committed
  hostchain tail.
- Restart tests cover staged state, committed add publication, and the crash
  window after the revoke commit but before private-key removal. Every retry
  returns the same operation ID and hostchain sequences without duplicate
  links. Opening the next exclusive rotation also removes private temporary
  files and generation-qualified files left by a crash before database staging.
- Client hard-state tests advance an existing pin through add and revoke. A
  client offline for the entire overlap advances directly from genesis to the
  complete chain without repinning. End-to-end coverage rotates after an
  account publication, reconstructs the client connection pool, advances the
  hostchain first, then proves Merkle and authenticated-user continuity.
- Backup/restore tests cover staged, published, and complete state. Startup
  rejects modified required secrets and mismatched generation ledger,
  hostchain table, probe, or Merkle-root bytes. Complete backups exclude the
  revoked private key.
- The key-directory lock serializes rotations with the server, backups, and
  other rotation processes. Database uniqueness constraints admit one active
  signer and one in-progress host rotation; the only revoke construction first
  activates and signs with the replacement.
- CLI audit output is intentionally limited to operation/generation IDs,
  public entities, link sequences, phase, and observation deadline. Secret-key
  debug output is redacted, and root keys, KEKs, recovery material, and bearer
  tokens are not reachable from the rotation state.
