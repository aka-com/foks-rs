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

Status: protocol and durability design fixed; mutation command intentionally
not exposed until the generation ledger and atomic host-state transaction below
are implemented and compatibility-tested against upstream FOKS.

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
  phase(staged|published|activated|retired|complete),
  add_link_seqno, revoke_link_seqno, created_at, updated_at
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
4. Keep both host signers active for a configured observation interval and
   verify mainline clients have synchronized through the add-key root.
5. Append a separate `Revoke(old_host_entity)` link signed by the new active
   host key, then publish another bound Merkle root.
6. Mark the old generation revoked. Retain its encrypted file in backups for
   historical/audit verification; never use it for new signatures.

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

### Required gates before exposing the command

- Exact v0.1.9 hostchain add/revoke fixtures accepted by both Rust verification
  and the pinned Go oracle.
- Crash injection before staging, before/after the SQLite commit, and before the
  response; restart must resume one operation ID without duplicating a link.
- Existing pinned clients synchronize across add and revoke roots without
  repinning, including clients offline for the entire overlap interval.
- Backup/restore succeeds at every durable phase and rejects a generation file,
  ledger, hostchain, probe, or Merkle-root mismatch.
- Concurrent rotation requests serialize per host and purpose; attempts to
  retire the last active host, metadata, Merkle, or TLS key fail closed.
- Operator audit output contains operation/generation IDs and public entities,
  never root keys, KEKs, private keys, recovery material, or bearer tokens.

Until these gates pass, host-key files are immutable after bootstrap and an
operator needing a different public host identity must create a new
installation and explicitly repin clients.
