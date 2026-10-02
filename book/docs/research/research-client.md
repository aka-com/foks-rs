# Research note: the FOKS client and local agent

Scope: `crates/foks-client`, `crates/foks-client-app`, `crates/foks-client-db`, `crates/foks-agent`, `crates/foks-agent-client`, `crates/foks-agent-proto`, `crates/foks-cli`, with excursions into `crates/foks-crypto` and `crates/foks-proto` where the KV and chat formats are defined. All paths below are relative to `/home/bnoland/projects/foks-rs`. Line numbers are as of this reading.

The client side of FOKS is organised as a stack of four crates plus a resident daemon:

- `foks-client-db` owns two SQLite databases and nothing else: a **hard-state** database (verified public bytes, monotonic pins, crash-recovery journals) and a **soft-state** database (a replaceable cache of decrypted KV projections, chat inbox state and routing hints). It sits *below* the protocol verifier and deliberately knows nothing about networking.
- `foks-client` owns the protocol workflows: authenticating a user, loading a team, synchronising and mutating the KV store, chat, passphrases, device provisioning and revocation, and the durable scheduler. It keeps no process-global state; every operation is given the paths and keys it needs.
- `foks-client-app` is "product-level orchestration": profiles, capability policy, credential serialisation, the encrypted vault, the OS-keyring-anchored rollback checkpoint, the file locks that serialise a profile across processes, and the "adapter" (a stable idempotent submission interface for external programs).
- `foks-agent` is the resident daemon. It owns a Unix socket, admission (which requests may run concurrently on which profile), read caches, background timers, chat long-polling, and retention. `foks-agent-proto` is the JSON-over-length-prefixed-frames IPC protocol; `foks-agent-client` is the small synchronous client the CLI and desktop use to talk to it. `foks-cli` is a thin command-line frontend.

The rest of this note is organised by technique rather than by crate.

---

## 1. Hard state versus soft state

### 1.1 The problem

A FOKS client accumulates two very different kinds of local data. Some of it is *evidence*: signed chain links, Merkle roots, the exact bytes a verifier accepted, and journals recording which mutations have been sent to a server. Losing or rolling back that data is a security event (a rolled-back Merkle head lets a server replay stale state). Other data is *derived*: decrypted file listings, chat inbox rows, discovered endpoints. Losing that data costs a re-download; it must never be trusted for anything the evidence does not independently justify.

FOKS makes this a **structural** distinction rather than a convention: two databases, two application IDs, two schemas, two sets of rules.

### 1.2 Hard state: SQLite as a monotonic, constraint-enforced ledger

`crates/foks-client-db/src/lib.rs:1-6` states the contract: "Durable SQLite hard state ... It preserves the exact signed bytes supplied by that verifier and enforces monotonic pins." The schema (`crates/foks-client-db/src/schema.rs`, `VERSION = 40`, application ID `0x464f4b53` = ASCII `FOKS`) has some notable properties:

- Every table is `STRICT` and most are `WITHOUT ROWID` with composite primary keys. Every column carries a `CHECK` on length or range (`length(host_id)=33`, `length(uid)=33`, `chain_seqno > 0`, ...). The database refuses malformed rows at the storage layer, so a bug in higher layers cannot persist an invalid pin (`schema.rs:125-330`).
- Invariants that span rows are expressed as **SQL triggers**. Examples: `kv_adapter_clock_irreversible` (`schema.rs:410-412`) aborts any update that would lower the adapter replay watermark; `kv_adapter_immutable_binding` (`schema.rs:455-469`) makes the identifying fields of a submission row immutable once written; `kv_adapter_capacity` (`schema.rs:445-453`) caps a user at 65,536 retained and 64 active submissions; `sso_commitment_immutable` (`schema.rs:73-75`) forbids changing a recorded SSO commitment. These triggers hold across processes and across crashes because they are part of the database, not the program.
- **Reserved positions as unique partial indexes.** A user-chain mutation reserves its expected chain seqno via `user_mutation_reserved_chain_position ON mutation_operations (host_id, scope_id, expected_version) WHERE operation_kind IN (2,3,4,9,10) AND state IN (1,2,3,4)` (`schema.rs:516-518`), and team mutations do the same via `team_mutation_reserved_position` (`schema.rs:377-382`). The comment explains why the `WHERE` clause excludes terminal states: "A definitively rejected or superseded operation remains available for audit without preventing a corrected mutation against the still-current head." This is the idempotency-key pattern applied at the level of chain positions: at most one live request per (host, chain, seqno).
- Rollback checks happen on every accept. `HardStateStore` refuses to store a chain, Merkle root or KV root older than the stored one, producing typed errors `ChainRollback`, `MerkleRollback`, `UserRollback`, `TeamRollback`, `KvRootRollback`, `KvDirectoryRollback` (`lib.rs:672-732`).
- The hard database is opened with `journal_mode=WAL`, `synchronous=FULL`, `secure_delete=true`, `trusted_schema=false`, `SQLITE_OPEN_NOFOLLOW` and a 0600 permission check (the soft database's `open` at `soft.rs:194-233` shows the same pattern). Secrets are *not* in this database; comments on every journal table say so: "Public crash-recovery journal ... Secret seeds, reservation tokens, and self tokens are deliberately excluded" (`schema.rs:328-342`, `356-359`, `382-384`, `536-539`, `578-580`).

### 1.3 A revision counter and a random write token

Every table in `REVISION_TABLES` (`schema.rs:4-33`) gets three triggers (`lib.rs:857-872`):

```
AFTER {insert|update|delete} ON {table}
BEGIN
  UPDATE hard_state_metadata
  SET hard_state_revision = hard_state_revision + 1,
      write_token = randomblob(16)
  WHERE singleton = 1;
END;
```

So `hard_state_metadata` (`schema.rs:118-123`) always holds `(database_id, hard_state_revision, write_token)`: a random 16-byte identity assigned at creation, a counter that increases on every write, and a fresh random token rerolled on every write. This small triple is used in two clever ways.

**(a) An exact cache key.** `crates/foks-client/src/host_cache.rs:1-30` explains that restoring a pinned host means re-verifying the hostchain, the Merkle anchor and every level of the skip-path nest, work that grows with the profile's age. The in-memory cache is keyed on `(database path spelling, discovery name, database_id, revision, write_token)`. Because every byte the restore reads lives in a revision-tracked table, "an unchanged triple means byte-identical inputs. The restore itself is a pure function of those bytes." The scheme "deliberately over-invalidates, because any hard-state write at all drops the entry, which is the safe direction."

**(b) An external rollback checkpoint.** `foks-client-app` copies the triple, together with the pinned host's chain tail and Merkle head, into a `RollbackCheckpoint` (`crates/foks-client-app/src/checkpoint.rs:2664-2685`) and stores it *outside* the state directory: in the macOS Keychain or the Linux Secret Service (`CredentialBackend::Native`, `checkpoint.rs:276-283`). On every checked session the on-disk database is compared with the keychain copy by `RollbackCheckpoint::reconciliation` (`checkpoint.rs:2729-2775`):

- profile or `database_id` changed → rollback;
- `hard_state_revision` went down → rollback;
- same revision but different `write_token` → rollback ("a write happened that we do not remember");
- higher revision but same `write_token` → rollback ("the token must have rerolled");
- host identity changed, or the stored hostchain no longer contains the checkpointed tail, or the Merkle epoch went down → rollback.

The purpose is to catch a restored-from-backup or copied state directory, which SQLite by itself cannot detect. The checkpoint is verified before and republished after every checked operation "even when the operation reports an error" (`checkpoint.rs:672-675`). The failure mode is explicit: the user is told to run `profile reset-hard-state ... --confirm-delete` (`crates/foks-client-app/src/lib.rs:163`). A separate "database claim" record in the keyring binds a `database_id` to one profile name so two profiles cannot share one database (`checkpoint.rs:1471-1490`).

### 1.4 Soft state: a replaceable projection with rollback anchors

The soft database (`crates/foks-client-db/src/soft_schema.rs`, application ID `FKVS`, version 6) holds `kv_parties` / `kv_directories` / `kv_entries` / `kv_large_files` / `kv_large_file_chunks` (the decrypted KV projection), `chat_inbox_state` / `chat_inbox_channels`, `known_stores`, and `federation_discovery_hints`. Its comments make the trust boundary explicit: "Beacon answers are routing hints, never trust anchors. Keeping them in the replaceable soft-state database makes that distinction structural" (`soft_schema.rs:143-145`). A schema mismatch does not migrate; it is an error naming "the safe recovery path" (delete and rebuild; `soft.rs:1709`, test `unsupported_soft_schema_names_the_safe_recovery_path` at `soft.rs:1941`). A comment at `soft.rs:1715-1717` says adding a column is fine "because soft state is replaceable."

Even though it is a cache, the soft store still enforces **monotonicity** so that a server cannot roll a client's *view* of the KV store backwards between two syncs. `project_tree_impl` (`soft.rs:972-1330`) refuses a root or directory with a lower version than stored (`KvRootRollback`, `KvDirectoryRollback`) and refuses different bytes at the same version (`KvProjectionConflict`). Two small tables exist purely for this: `kv_directory_history` and `kv_entry_history` keep "ciphertext-only rollback anchors [that] survive permission-driven cache pruning" (`soft_schema.rs:82-105`). If a user loses read permission to a subtree the plaintext is pruned but the version anchors remain, so a later regrant cannot present an older directory.

The plaintext-holding soft database must be private: `plaintext_soft_database_must_be_private` (`soft.rs:2850`) tests that a database with group/other bits set is refused (`InsecureSoftPermissions`).

**Invariant.** Hard state is authoritative and monotonic; soft state can be deleted at any time and re-derived from hard state plus the network; but soft state, while it exists, is also monotonic per party so the server cannot replay an old view.

---

## 2. Crash-safe mutations: a write-ahead intent log across two durability domains

### 2.1 The problem

A client that signs a chain link and sends it may crash before it learns the answer. If it regenerates the link with fresh randomness and resends, it can end up with two different links competing for one seqno, or two different PUK generations, or two KV dirents with the same name. FOKS treats "the response was lost" as a first-class state.

### 2.2 The mutation state machine

`mutation_operations` (`schema.rs:384-398`) is "a generic public write-ahead journal shared by all client mutations. Exact retry material is referenced by an opaque key and must live in a separate protected material store; it is never written to this hard-state database." The states (`crates/foks-client-db/src/lib.rs:337-344`) and allowed transitions (`lib.rs:359-374`):

```
Prepared ──► Submitting ──► SubmissionUnknown ──► RemoteVerified ──► Finalized
   │              │                 │
   └──► Rejected  └──► Rejected     └──► Rejected
                  └──► RemoteVerified
```

`advance_mutation` (`crates/foks-client-db/src/repositories/journals.rs:179-225`) checks `can_transition_to` inside a write transaction and clamps the timestamp monotonically "so a backward wall-clock step cannot block finalization."

### 2.3 The coordinator: protected request first, then SQLite, then erase

`MutationCoordinator` (`crates/foks-client/src/mutation.rs:63-70`) "coordinates the required ordering between two durability domains: protected request first, then SQLite WAL. Remote verification remains nonterminal until the application acknowledges its own durable commit; terminal SQLite state is committed before protected request is erased." The `ProtectedMutationStore` trait (`mutation.rs:25-36`) is deliberately minimal (`put_if_absent`, `get`, `remove`) and demands that implementations "durably commit `put_if_absent` before returning and must never silently replace different bytes at a key." The journal row carries `material_ref` (the opaque key) and `material_hash` (a domain-separated hash of the request), so on recovery the client can check that the bytes it reads back are the ones it journaled (`write.rs:750-757`).

The ordering guarantees: if the process dies after writing the protected request but before the journal row, an orphaned request is harmless ("an orphaned material record is safe, while a journal row with missing material is not recoverable", `journals.rs:6-9`). If it dies after the journal row but before the socket write, the row is `Prepared` and the exact bytes can be resent. If it dies after the socket write, the row is `Submitting`, which recovery treats as ambiguous and resolves only by *observation* (reloading the chain or namespace and checking whether the transition appears), never by resending.

`EncryptedFileMutationStore` (`crates/foks-client/src/protected_store.rs:21-30`) is the durable implementation: each record is independently authenticated with XChaCha20-Poly1305 under a master key derived from the vault key and installed with "an atomic no-replace operation after its contents have been synced."

### 2.4 The same pattern, per workflow

The KV namespace outbox (`crates/foks-client/src/kv/write.rs:670-720`, `726-783`, `943-1016`) is the clearest example. `mutate_namespace` prepares a `kvPut` request (precondition version vector plus the new dirents), journals it, moves to `Submitting`, then sends. On a stale-cache error it marks `Rejected`, refreshes the path and retries (up to `MAX_NAMESPACE_ATTEMPTS = 3`). On any other transport error it marks `SubmissionUnknown` and reconciles by re-reading the directory: `reconcile_namespace_mutation` refuses to finalise "until the authenticated projection carries every prepared dirent", comparing the observed dirent's `binding_payload` and `binding_mac` byte-for-byte with what was journaled ("Go assigns creation_time; every client-authenticated field must match", `write.rs:993-1003`). One subtle rule: an unacknowledged *unlink* cannot be proven by absence, because absence is also what you see if it was never sent: "absence cannot prove a specific unacknowledged unlink" (`write.rs:950-960`).

Team membership changes and PTK rotations use the same shape with a stronger identity: the operation ID is *derived* from the plan (`refresh_team_member_keys_operation_id`, `crates/foks-client/src/team/rotation/mod.rs:311-352`), so a caller that persisted its rotation seeds "can be resumed exactly" after a crash, and the three outcomes of a lost response are enumerated as a type: "our rotation committed, a conflicting transition took the seqno, or the position is not yet observable" (`rotation/mod.rs:156-158`, `reconcile.rs:402-405`). Recovery "never regenerates or reposts key material; it accepts only the exact authenticated transition bound to the journal" (`rotation/mod.rs:1408-1410`), and `replay_recorded_team_rekey` closes "the crash window after sequence reservation but before the first socket write without rebuilding boxes or signatures with different randomness" (`rotation/mod.rs:1677-1681`). Device provisioning and PUK rotation follow suit (`crates/foks-client/src/device.rs:1728-1730`, `1881-1884`, `2009-2013`).

### 2.5 Adapter submissions: idempotency keys for external callers

The "adapter" is the KV write interface offered to other programs through the agent. It adds a second layer of idempotency on top of the journal. A `SubmissionHandle` (`crates/foks-proto/src/submission.rs:4-13`) is "a canonical issuance time and 128 random bits", serialised as `v1-<16 hex seconds>-<32 hex random>` (52 characters, matching the `length(handle)=52` check in `schema.rs:416`). The `kv_adapter_submissions` ledger stores the handle's hash, an `input_hash` over the request, and the internal mutation ID, so a repeated submission with the same handle and same input returns the recorded outcome, while the same handle with a different input is a conflict. `prepare_data_write` is documented as "Idempotent preparation. Resolve names only for an unseen submission ID" (`crates/foks-client-app/src/kv/data_write.rs:58`).

The ledger also has an unusual **clock discipline** (`crates/foks-client-db/src/repositories/adapter/clock.rs`): a per-user row records a random `process_id`, a wall/monotonic anchor pair, a `validated_time_floor` and `reject_issued_before`. Submissions older than 24 hours are rejected; terminal rows are retained 30 days; and the trigger at `schema.rs:410-412` makes the rejection watermark irreversible so "a corrected clock behind the irreversible watermark cannot mint fresh identities until the clock itself has caught up" (`clock.rs:151-152`). This is a defence against a caller replaying an old handle after the local clock is set backwards.

A high-level adapter intent may spawn several low-level KV operations (create root, mkdir -p, then the final put). `mutation_children` (`schema.rs:488-507`) records the parent/child binding, and the `mutation_children_capacity` trigger checks that the child belongs to the same host and party and that the parent is `Submitting`. The `completion` flag marks which children constitute the final namespace attempt versus ancillary steps (`write.rs:38-76`).

**Why.** Every one of these journals is public (no secrets), so it can be inspected and reasoned about without unlocking anything; the secret request bytes are in one authenticated store keyed by opaque reference. The design intent is stated in the schema comments and in `MutationCoordinator`'s doc: exact replay, never regeneration.

---

## 3. The encrypted key-value store

### 3.1 Object model

The FOKS KV store (protocol v0.1.9, which the Rust client reimplements byte-compatibly) is a tree of the following server-visible objects (`crates/foks-proto/src/kv.rs`):

- **`KvRoot`** (`kv.rs:195-200`): `root` (16-byte directory ID), `version`, `key` (which PUK/PTK role and generation), `binding_mac`. The MAC binds `(party, key, root id, version)` under the party's KV MAC key (`crates/foks-crypto/src/lib.rs:164-181`), so the server cannot substitute a different root for a party.
- **`KvDirectory`** (`kv.rs:274-281`): `id`, `version`, `key`, `seed_ciphertext`, `write_role`, `status` (`Active` or `Encrypting`). Each directory has its own random 32-byte **directory seed**, sealed in a typed secretbox under the party's KV box key with the directory ID as nonce (`open_directory_seed`, `lib.rs:190-203`). A `KvDirectoryPair` (`kv.rs:307-311`) may carry both an `active` and an `encrypting` generation, which is how a directory is re-keyed after a PUK/PTK rotation without a stop-the-world rewrite: entries created under the old generation stay decryptable under the old seed while new ones use the new seed. The sync loop opens one seed per generation and indexes them by version (`sync.rs:920-935`).
- **`KvDirent`** (`kv.rs:339-351`): `parent`, `id` (random 16 bytes), `value` (a 17-byte `KvNodeId`: one type byte + 16 random bytes), `version`, `directory_version`, `write_role`, `name_mac`, `name_box`, `directory_status`, `binding_mac`, `creation_time`.
- **Nodes** (`kv.rs:659-665`): `SmallFile` (a `KvSmallFileBox`, ciphertext under the party key), `File` (large file: `KvLargeFileMetadata` with the per-file seed sealed inside), `Symlink` (a small-file box whose plaintext is tagged as a symlink target), `Directory`.

### 3.2 The key hierarchy

`derive_kv_keys` (`crates/foks-crypto/src/lib.rs:136-161`) takes one exact PUK or PTK seed and derives: `app_key = derive_key(seed, 5)`; `kv_app = HMAC(app_key, APP_KEY_DERIVATION_TYPE_ID || encode([0, variant "0"]))`; then `mac = HMAC(kv_app, KV_KEY_DERIVATION_TYPE_ID || encode([1, None]))` and `box_key = ... encode([2, None])`. Every HMAC is `typed_hmac` (`lib.rs:562-570`): HMAC-SHA512/256 over `type_id.to_be_bytes() || object`, so every MAC in the system is domain-separated by a 64-bit type ID. `derive_seed_kv_keys` (`lib.rs:546-560`) applies the same two-way split to a directory seed or a file seed.

So there are three key levels:

1. Party level (PUK for personal stores, PTK for team stores; role and generation named in every object) → root binding MAC, directory seeds, small-file boxes, large-file metadata.
2. Directory level (per-directory seed) → dirent name MAC, dirent name box, dirent binding MAC.
3. File level (per-large-file seed) → chunk encryption.

Because the party key is generation-specific, key rotation only requires re-sealing directory seeds and file seeds (which is what the `Encrypting` status supports), not re-encrypting content.

### 3.3 Hiding names while keeping them searchable

The central trick of the KV store is how a name is stored. `seal_kv_dirent_name` (`crates/foks-crypto/src/lib.rs:408-433`):

```
payload  = encode([parent_dir_id, directory_version, name])
name_mac = HMAC(dir_mac_key, KV_DIRENT_NAME_PAYLOAD_TYPE_ID || payload)   // deterministic
name_box = secretbox(dir_box_key, nonce=random16, payload)              // randomised
```

The `name_mac` is a **deterministic keyed hash** of the name, bound to its parent directory and directory version. Anyone holding the directory seed can compute it for a candidate name; the server cannot, and because the key is per-directory the same file name in two directories yields unrelated MACs, so the server cannot even tell that two directories contain a file with the same name. The `name_box` is a randomised encryption of the same payload so that a *listing* can recover the plaintext name. On open (`open_kv_dirent_name`, `lib.rs:482-508`) the client decrypts the box, re-encodes canonically, verifies the MAC over the canonical bytes, verifies the dirent's `binding_mac`, and checks that the `parent` and `directory_version` inside the box match the dirent's own fields, defeating a server that moves a dirent between directories.

The `binding_mac` (`bind_kv_dirent`, `lib.rs:435-441`; payload at `kv.rs:414-425`) covers `(parent, id, value, version, directory_version, write_role, name_mac, name_box)`, so every client-authored field of a dirent is authenticated, and a server that changes the node a name points to, or the version, is detected.

`name_mac` doubles as the **pagination cursor**: a directory listing is fetched in pages of at most 100 entries (`MAXIMUM_KV_LIST_PAGE_ENTRIES`, `kv.rs:16`), and the next page is requested with `KvListCursor::Mac(last_mac)` (`sync.rs:1252-1254`). Ordering by MAC is stable and opaque to the server; the client re-sorts by plaintext name after decrypting (`sync.rs:1259-1263`).

This is the same idea as deterministic (convergent) encryption of file names in encrypted filesystems, restricted to a keyed MAC per directory so that the determinism does not leak across directories or across parties.

### 3.4 Path lookup without revealing structure

The server never receives a path. The client's `sync_kv_with_fetch_mode` (`crates/foks-client/src/kv/sync.rs:827-1358`) drives one of two traversal plans (`KvTraversalPlan`, `sync.rs:1598-1606`): `Complete` (BFS over every reachable directory, with a `visited` set and a 4,096-directory cap) or `Path(components)` (root, then one directory per component). In both, the client:

1. fetches the root (`KvRequest::Root`) and verifies `binding_mac` under the key generation the root names (`sync.rs:885-890`);
2. fetches each directory by ID, checks the response ID matches and the status is sane, and opens the seed for each generation (`sync.rs:905-935`);
3. lists pages, decrypts each name, and validates it (UTF-8, no NUL, no `/`, not `.`/`..`, no duplicates; `sync.rs:1003-1016`);
4. for a path walk, finds the child whose plaintext name equals the next component and enqueues its directory ID only if the node type is `Directory` (`sync.rs:1266-1281`); a missing or non-directory component "leaves the walk exactly as short as a complete traversal would leave this path."

What the server sees is therefore a sequence of directory-ID fetches and listings. It learns the shape of the *access pattern* (which directory IDs were fetched in what order, how many entries each has) but not names, not node types beyond the type byte in a node ID, and not which entry the client was looking for on a page. (Access-pattern leakage of this kind is the standard residual leakage of this design; the code does not attempt ORAM-style hiding.)

Path grammar lives in `foks-client-app` (`crates/foks-client-app/src/kv.rs:1441-1490`): absolute, at most 4,096 bytes, components percent-decoded into raw bytes, each 1–255 bytes and not `.`/`..`. Component bytes are what gets MACed, so a name can be any UTF-8 without `/` or NUL. Symlinks are stored as encrypted targets (validated by `validate_kv_symlink` at `crates/foks-client/src/kv/support.rs:261-270`: 1–4,096 bytes, UTF-8, no NUL) and are *returned* as a `symlink_target` rather than auto-followed by the core read path (`kv.rs:1371-1400`); only the adapter's whole-tree resolver follows them (`kv.rs:1595-1598`).

### 3.5 Optimistic concurrency with version vectors

Every mutation is a `kvPut` carrying a `KvPathVersionVector` (`kv.rs:99-107`): the root version, and for each cited directory its version and the `(dirent id, version)` of every dirent listed in it (`kv_version_vector`, `support.rs:189-211`). The server compares the vector against its heads and answers `KV_STALE_CACHE_ERROR` "containing only objects whose current versions are newer than the submitted values" (`kv.rs:99-101`). The doc on `kv_path_version_vector` (`sync.rs:1361-1370`) explains a subtlety: "a dirent version restarts at 1 after an unlink and a re-create on a fresh dirent identifier, so the same pair can name different nodes at different times. The vector names the dirent identifiers themselves, and the server reports a cited dirent whose head has moved — which an unlink does, because the tombstone is a write to that dirent — as stale."

Two more design points:

- **A path walk cites only what it used.** `KvPathScope` (`crates/foks-client/src/kv/mod.rs:79-100`) records the directories a resolved path traversed; a later mutation in that directory cites exactly those, "so a peer's change to any of them is what the server's precondition check rejects." An uncited directory is simply not asserted (`sync.rs:1299-1310`). This keeps write cost proportional to path depth, not store size.
- **Closing the read race.** After a complete traversal the client sends `KvRequest::CacheCheck(final_versions)` (`sync.rs:1311`); if anything changed during the traversal the server answers stale and the whole sync is retried, up to three times (`sync.rs:1344-1357`). The comment at `sync.rs:869-873` notes why a complete projection cannot rely on the vector alone: "a successful cache check proves that cited entries are unchanged, but it cannot prove that a peer did not add an uncited entry."
- **Remembered paths.** `read_user_kv_chunk_if_current` (`sync.rs:112-160`) lets a caller who resolved a path earlier keep reading chunks by submitting the remembered vector "as its own request"; `Ok(None)` means "the caller's resolved path is no longer evidence about this node — a peer may have unlinked the dirent and created another one at the same name — so the caller must walk the path again." The agent's KV path cache (`crates/foks-agent/src/read_cache.rs:45-53`) is exactly such a remembered vector: "A hit is not an answer. It is a candidate node the caller still has to put to the server as a version vector" (`read_cache.rs:214-217`).

Dirent-level preconditions are also available to the application: `KvWriteOptions.expected_version` (`kv/mod.rs:42-47`) makes a put fail if the existing dirent's version differs, and `overwrite=false` refuses to replace an existing live entry; only file→file/symlink overwrites are allowed, never directory replacement (`write.rs:1112-1153`).

### 3.6 Small files, large files and chunking

`put_file_with_size` (`write.rs:256-394`) reads the first upload chunk and decides: if the whole file fit in that read and is at most `SMALL_FILE_BYTES = 2048 - 8 = 2040` bytes (`write.rs:36`), it becomes a `SmallFile` sealed in one box under the party key and uploaded with `PutSmall`. Otherwise it becomes a large `File`:

- a random per-file seed is generated and sealed inside `KvLargeFileMetadata.key_seed` together with the file's object ID and metadata version (`seal_file_seed`, `lib.rs:376-404`; opened by `open_file_seed`, `lib.rs:224-252`, which checks both);
- upload chunks are at most `MAX_KV_UPLOAD_CHUNK = 4 MiB` of plaintext (`kv/mod.rs:22-23`); the file is capped at `MAX_KV_FILE_BYTES = 1 GiB`;
- each chunk is `seal_kv_chunk` (`lib.rs:443-480`): the plaintext is Snowpack-encoded, zero-padded to a bucketed length (`kv_chunk_padded_length`), and encrypted with XSalsa20-Poly1305 under the file seed with a **derived nonce**: the first 24 bytes of `hash(KV_CHUNK_NONCE_PAYLOAD_TYPE_ID, encode([file_object_id, offset, final_chunk]))`. Because the nonce is a function of (file, offset, is-final), a server that returns chunk N in answer to a request for chunk M produces a decryption failure; `open_kv_chunk` additionally checks `chunk.offset == requested_offset` (`lib.rs:514-518`) and the sync loop rejects an empty non-final chunk;
- the first chunk travels with `UploadInit` (metadata plus chunk 0) and the rest with `UploadChunk`; the final chunk carries `KvUploadFinal { size: total encrypted size }`. Only after the upload completes is the dirent linked (`link_uploaded_node`), so a crashed upload leaves an unreferenced node but never a dangling name.

A Rust-only extension: the plaintext length is sealed into `custom_metadata` (`LARGE_FILE_SIZE_PAYLOAD_TYPE_ID`, `lib.rs:125-127`), which "servers treat ... as opaque". Legacy/Go files lack it; the metadata sync then falls back to a locally measured size stored in `kv_large_file_measurements`, keyed on the immutable node ID (`soft_schema.rs:125-136`, `sync.rs:1160-1185`), and a mismatch between the two is an integrity error.

On download the sync loop streams chunks straight into the soft store (`begin_large_file` / `append_large_file` / `finish_large_file`, `soft.rs:802-903`) so that "Large files stream directly into SQLite and therefore do not consume this in-memory plaintext budget" (`sync.rs:897-899`; the inline budget is 512 MiB, `sync.rs:844`). Orphaned staging rows from a crash are reclaimed by `reclaim_orphaned_large_files`, which refuses to delete a stage still owned by a live sync (`soft.rs:905-919`, test at `soft.rs:2779`).

### 3.7 Two sync modes and cached ciphertext

`KvSyncMode::Metadata` builds a catalog (names, types, sizes, roles) without downloading content; `Content` downloads everything. In metadata mode the loop asks `kvList` for `load_small_files: true` so that small-file boxes arrive in the listing's "extended side table" and their key/role header can be authenticated without an extra RPC per file (`sync.rs:948-957`). It also reuses *ciphertext* already in the soft store for file and symlink nodes via `cached_node_bytes` (`sync.rs:856-866`, `soft.rs:1368`), with the rule that "Cached bytes follow the same decoding and authentication path as fetched bytes"; the test `a_tampered_cached_node_still_fails_authentication` (`sync.rs:2024`) flips a byte of cached ciphertext and checks the traversal still fails. Only an explicit permission denial marks an entry `readable = false` while retaining its authenticated dirent; every other failure fails closed (`sync.rs:1414-1421`, `soft.rs:78-80`).

### 3.8 Roots, mkdir -p, and namespaces

A party's KV namespace is created lazily by `initialize_root` (`write.rs:198-224`): a first directory with a random seed and a root binding MAC, put with the "upstream two-step protocol" in which the directory may become unreferenced if `kvPutRoot` loses a race. Reads never create a namespace: a missing root (server status 8016) is an empty store *only if* the client has never seen a root for that party (`sync.rs:876-886`), otherwise "disappearance of a known root" is an error.

Personal and team stores are the same code with a different `KvParty` (`kv.rs:72-75`: party ID plus host ID) and different auth: `KvAuth::User` versus `KvAuth::Team(view_token)`, a short-lived bearer obtained by the team loader (section 5). The application layer names stores as `KvStoreRef::Account` or `KvStoreRef::Team` and keeps a `known_stores` table in soft state (`soft_schema.rs:4-24`).

`resolve_write_parent` (`crates/foks-client-app/src/kv.rs:1530-1545`) implements `mkdir -p` inside one write session: missing parents are created "inheriting the item's read and write roles to ensure consistent access control across the directory hierarchy", and each mkdir is a journaled namespace mutation bound to the adapter parent.

---

## 4. Chat: ordering, delivery and inbox synchronisation

### 4.1 Message ordering as a per-channel hash chain

A chat message (`RtMessage`) has a server-assigned `sequence`, a client-chosen random `metadata.id`, and `metadata.previous_sequence` / `previous_id` naming the message the sender saw as the head. The client turns every verified message into a `ChatAnchor { sequence, id, digest }` where the digest is a domain-separated hash of the whole encoded message (`crates/foks-client/src/realtime/history.rs:27-33`) and stores it in `chat_anchors` in **hard** state, with a primary key on `(scope, sequence)` and a `UNIQUE` on `(scope, message_id)` (`schema.rs:106-116`).

`verify_page` (`history.rs:161-282`) is the ordering check. For every page it:

- rejects duplicate IDs or sequences within the page and pages out of order (`read_recent` requires strictly descending sequences, `history.rs:57-66`; `range` checks bounds and monotonicity, `history.rs:144-158`);
- decrypts each message under the channel's read role and key generation (`open_message`, `history.rs:283-321`), rejecting a message whose box names a different role than the channel metadata, or whose `previous_sequence >= sequence`, or whose predecessor fields are half-set;
- for each text message whose predecessor is neither in the page nor in the anchors table, requests the predecessor by sequence (`GetThread` with `sequences`, bounded by `PREDECESSORS = 32`) and checks that the `previous_id` claimed matches the predecessor's actual ID ("predecessor ID mismatch", "predecessor contradiction");
- calls `chat_accept_page(scope, observed, anchors)` (`crates/foks-client-db/src/repositories/chat.rs:402-450`), which in one transaction checks every observed `(sequence, id, digest)` against what is already stored under both keys and raises `ChatConflict` if the server has changed the mapping ("message mapping or envelope changed"), then inserts new anchors and prunes the oldest beyond `RETAINED_ANCHORS = 10,000`.

Predecessors that could not be fetched within the byte budget are reported as `missing_predecessors`, and the doc on that field is emphatic: "Missing predecessors are incomplete evidence, never a successful check" (`history.rs:24-25`, `247-249`). Unsupported message kinds still contribute to `observed` (so the server cannot hide a swap behind a kind the client does not render) but only authenticated text messages become anchors (`chat.rs:402-403`).

### 4.2 Incremental history with a cursor

`read_after(channel, after, limit)` (`history.rs:69-97`) reads the channel metadata to learn the head, computes `start = max(after+1, head-limit+1)`, fetches that range, and returns a `gap` flag when `head < after` (sequence rollback), when more than `limit` rows were skipped, or when the range came back short: "A reset or omitted row between metadata and history needs a reload too." Windows are capped at `HISTORY_ROWS = 1000` and `HISTORY_BYTES = 8 MiB` (`crates/foks-client-db/src/chat_limits.rs`).

### 4.3 The account inbox: cursor, head, degraded

The inbox is an account-scoped list of channels ordered by an `inbox_version`. Soft state keeps `chat_inbox_state (cursor, head, degraded)` per `(host, uid, app)` and one row per channel with its `inbox_version`, `read_through`, and a `pending_read` staged before confirmation (`soft_schema.rs:27-50`). `drain_inbox_scope` (`crates/foks-client/src/realtime/inbox.rs:493-569`):

1. asks the server for the inbox version (`remote_head`); if it is lower than the stored head or cursor, the whole inbox is reset (server rollback is not an error here because the inbox is soft state, but it is never silently merged);
2. requests changed threads `since: cursor` in pages (`INBOX_PAGES = 64`), applying each page with `apply_chat_inbox_page`, which advances the cursor;
3. treats an empty delta with a version above the cursor as a **degraded** observation (the server claims changes it will not enumerate) after one retry with a larger page; a degraded scope is never reported as drained ("the rows behind that version have never been read", `inbox.rs:739-741`).

The scope is host+user+app with no team in it, so one drain applies every team's changes. The agent exploits this with an `INBOX_DRAIN_TTL = 2 s` gate (`crates/foks-agent/src/chat.rs:327-418`): "Five teams of one account synchronizing in one cycle: the first runs the version query and the delta page, and the four behind it are told the account is already current. Two round trips for the cycle rather than two per team." Only a complete, nondegraded drain sets the gate.

### 4.4 Previews, key rotation and notification

Inbox previews (the decrypted last message of each channel) are cached in the agent process under a `ChatPreviewKey` that "names the reader, the channel and the exact message the preview renders, together with the role and key generation the reader currently holds for that channel, so a preview is never served to another reader, for another message, or across a key rotation" (`inbox.rs:37-51`; `crates/foks-agent/src/chat.rs:244-261`). Snippets are truncated on the way in so that the cache is bounded by entries × snippet bytes. A channel whose read role the reader no longer holds a current key for "is never cached, so a reader that has lost the role cannot be served one" (`inbox.rs:224-227`). Real-time delivery is by long poll: the agent runs cancellable per-account polls (`crates/foks-agent/src/chat_poll.rs`, `CHAT_POLL_TIMEOUT = 60 s`) that hold no profile admission while waiting on the network (`chat_poll.rs:244`).

### 4.5 Sending: prepare, attempt once, reconcile by scanning forward

A send is split into *prepare* (encrypt and journal; `crates/foks-client/src/realtime/operations/prepare.rs`) and *attempt* (`attempt.rs:6-53`), so "the external checkpoint is published before network delivery" (`crates/foks-client-app/src/chat.rs:1-2`). The journal row (`chat_operations`, `schema.rs:77-95`) has states Prepared/Delivering/Confirmed/Rejected/Cancelled with CHECKs tying `receipt` to Confirmed and `rejection_code` to Rejected. The attempt "never automatically resubmits an uncertain operation" and re-validates before delivery that the pending message is still encrypted under the *current* key generation ("pending message key is stale; preserve operation for review", `attempt.rs:55-69`), so a message prepared before a rotation is not sent under a retired key.

If the response is lost, `reconcile_operation` (`recovery.rs:5-90`) scans forward from the operation's `scan_cursor` in windows of `RECOVERY_WINDOW = 100` sequences, verifying each page exactly like history, and confirms the send if it finds a message with the same client-chosen `metadata.id` **and** identical metadata, wrapper and sender; a message with that ID but different content is an integrity error. The cursor is persisted after each window so a restart resumes where it left off. Idempotency for callers is provided by `ChatSubmission { id, input_mac }` (`crates/foks-client-db/src/repositories/chat.rs:72-100`): a keyed MAC of the plaintext under the vault master key, so "durable metadata does not reveal guessable text" (`crates/foks-client-app/src/chat.rs:28-34`); the same ID with a different MAC is a conflict.

Channel names are normalised with Go's simple lowercase mapping and validated against Go's `unicode.IsPrint/IsPunct/IsSpace` tables, generated into `name_ranges.rs` (`prepare.rs:8-31`), an example of byte-compatibility with the reference implementation being treated as a correctness requirement. A limit file shared with the desktop (`chat-limits.json`) is asserted at parity by the agent (`chat_limits.rs:17-18`).

---

## 5. Team loading and the membership graph

### 5.1 Loading a team

`load_and_pin_team` (`crates/foks-client/src/team.rs:2219-2277`) tries the user's PUKs newest-first (preferring the one the last pinned roster named), and for each calls `load_and_pin_team_for_actor_with_view_token` (`team.rs:3325-3539`), which:

1. enters the per-host **pinning span** (`crates/foks-client/src/pinning.rs`), a process-local reentrant mutex per hard-state database that keeps two operations from interleaving their "advance the Merkle head, then accept a chain under it" steps, which would otherwise read as a rollback;
2. advances the Merkle root, then loads the team chain **incrementally** from `prior.chain_seqno + 1` when a pinned snapshot exists, falling back to a full load if the delta reply omits the HEPK of the parcel sender (`team.rs:3338-3418`);
3. verifies the chain (`verify_team_chain` / `verify_team_chain_increment`) against Merkle roots authenticated for the returned links;
4. finds the requesting party in the verified roster and checks that the roster's `verify_key` and `generation` match the PUK being used (`team.rs:3420-3440`), so a stale key cannot open the team;
5. expects exactly one PTK parcel per role the member may see (`role <= member.role`) and no more (`team.rs:3443-3452`);
6. opens each parcel with the PUK, checks the sender's HEPK against the roster fingerprint, and then opens the **seed chain**: a parcel for generation G must yield every generation ≤ G of that role, and each opened seed must derive to a public key present in the verified history (`team.rs:3487-3528`); a chain missing a generation or containing an unauthenticated key is rejected;
7. atomically pins the verified team snapshot in hard state (`accept_verified_team`, which runs the rollback checks).

The view token obtained by the challenge (a bearer valid for roughly six hours server-side) is cached in the agent for five hours, keyed on the acting party and a digest of the credential's mTLS material so it can never be replayed over another credential (`crates/foks-client-app/src/auth_cache.rs:71-77`; `read_cache.rs:40-43`).

### 5.2 Checking a roster cheaply: batched absence proofs

The periodic team-refresh pass must decide whether any member's PUK generation has advanced beyond what the roster froze. `crates/foks-client/src/change_marker.rs:1-24` explains why the team chain's own seqno is useless here ("a member's key rotation advances that member's *user* chain and leaves the team chain where it is") and introduces a batched marker: for each pinned member the two Merkle keys whose absence prove the user chain has not grown (no link one past the tail, no further claim on the username sequence). One batched Merkle lookup of up to 256 tails per request replaces a chain load per member; anything short of a proof is treated as "advanced" and loaded, so "a server can therefore at worst cost the caller the work it would have done anyway" (`change_marker.rs:62-68`).

### 5.3 Membership changes force PTK rotation

Removing or demoting a member is modelled as "membership change with mandatory PTK rotation" (`crates/foks-client/src/team/rotation/mod.rs:1`). The rule for which roles rotate is a pure function, `required_rotation_roles` (`crates/foks-client/src/team/rotation/support.rs:240-258`): every current PTK role `r` with `r <= old_role` rotates if the member is being removed (`destination == NONE`), or if its PUK generation is being advanced (its old PUK may be compromised), or if `r > destination_role` (it is losing access to that role). Update (2026-10-01): promotion now uses a distinct distribution schedule in this edit path. It distributes newly visible existing PTKs, creates a generation-one destination PTK when missing, and refuses stale/changed source keys until a separate refresh (`team/rotation/support.rs`, `team/rotation/mod.rs`). The older rejection described in the original research is superseded.

New PTK seeds are boxed only to the *remaining* recipients, and the recipients must be **witnessed**: a `VerifiedTeamRecipient` (`rotation/types.rs:98-103`) is "a team recipient whose complete direct roster was independently checked against current authenticated user/team projections. Recursively requiring this witness for child teams prevents a parent team member-key refresh from boxing new PTKs to a team key that is still exposed through a stale descendant PUK/PTK." Removal also requires the member's **removal key**: a secret retained at admission whose commitment is in the roster (`team_members.removal_key_commitment`, `schema.rs:284`) and checked before the edit (`rotation/mod.rs:386, 441`).

### 5.4 Sweeping the membership DAG

Teams can be members of teams, so a rotation in a child must propagate to parents that box keys to the child's PTK. `refresh_authenticated_team_graph` (`crates/foks-client-app/src/runtime.rs:557-620`) "sweeps the authenticated local membership DAG child-before-parent. Any mutation advances the Merkle tree, so the whole graph is rediscovered before another PTK is emitted; this prevents mixing recipient projections authenticated at different heads." Up to 256 rediscovery rounds are allowed. Teams whose pending rotation belongs to a device this process cannot unlock (a YubiKey-held rotation, say) are placed in a *blocked* set that is "expanded upward through `(child, parent)` membership edges" (`runtime.rs:433-436`), because "the parent would be rekeyed against a roster that is about to change again."

### 5.5 Device revocation

`revoke_user_credential_with_software_device` (`crates/foks-client/src/device.rs:569-591`) "revokes a different enrolled user credential ... and rotates exactly every PUK role the target could read." The rotation seeds must be caller-durable before the call; the request is journaled and reserves the user-chain position via the unique index in section 1.2; and if the owner PUK rotates, the passphrase-encrypted parcel (PPE) must be re-boxed in the same transition ("the PPE annex a rotation that touches the owner PUK must carry", `device.rs:1493-1505`). Reconciliation after a crash can be performed by *any* owner device (it observes the chain), but only the original signer may resend (`device.rs:2009-2013`). A rotated user PUK then makes that user's roster entry stale in every team, which is what the team-refresh sweep detects and repairs.

---

## 6. Local agent, IPC and secret handling

### 6.1 Securing the Unix socket

The agent's startup (`crates/foks-agent/src/main.rs:230-330`, `6067-6255`) is a small textbook of local-IPC hardening:

- The socket's parent directory must be a real directory (checked with `symlink_metadata`, which does not follow links) whose canonical path lies inside the explicit state root (`main.rs:244-254`).
- A **lock file** `.foks-rs.lock` is opened with `O_NOFOLLOW`, mode 0600, and rejected unless it is a regular file owned by the current effective UID with no group/other bits; then `flock(LOCK_EX|LOCK_NB)` is taken, and `EWOULDBLOCK` is reported as "another agent owns this state directory" (`AgentLock::acquire`, `main.rs:6103-6138`).
- A leftover socket path is removed only if it is a socket, owned by this UID, mode exactly 0600, *and* a `connect()` gets `ECONNREFUSED` (a live agent answers, and is not stale). The inode is re-checked after the probe so that a path swapped during inspection is not deleted (`remove_stale_agent_socket`, `main.rs:6159-6201`).
- The socket is bound at a **staging path** (`<socket>.bind`), chmod'ed to 0600 while nobody can find it, and then `rename`d into place. The comment notes this "works across all UNIX platforms including macOS/Darwin where hard linking domain sockets returns EPERM" (`bind_private_agent_socket`, `main.rs:6067-6084`). This closes the window between `bind` (which creates the socket world-connectable under the umask) and `chmod`.
- Every accepted connection is checked with `peer_cred()` (SO_PEERCRED / LOCAL_PEERCRED) and rejected unless the peer UID equals the agent's effective UID (`main.rs:314-328`).
- A `SocketGuard` remembers the bound socket's `(dev, inode)` and, on exit, removes the path only if it still names that inode: "An exiting agent must never remove its successor's socket" (`main.rs:6205-6248`). An `ownership` timer every 5 s re-validates that the lock file, socket inode and state lease are still this process's; if displaced (e.g. a desktop takeover), the agent exits (`main.rs:296-308`).

The client side mirrors the checks before connecting: the path must be a socket, mode with no group/other bits, owned by the same UID (`crates/foks-agent-client/src/lib.rs:318-333`), else `Error::UnsafeSocket`. The bot-token file helpers (`secret_file.rs`) apply the same rules (`O_NOFOLLOW|O_CLOEXEC`, 0600, refuse symlinks, bounded length, `create_new` so an export cannot clobber).

### 6.2 The framing protocol

`foks-agent-proto` (`crates/foks-agent-proto/src/frame.rs`) is a 4-byte big-endian length prefix followed by JSON, capped at `MAXIMUM_MESSAGE_BYTES = 1 MiB`. Every message carries a `version` and a correlation `id`; `request_id` recovers the ID even from an unsupported frame so that an error reply can still be correlated (`frame.rs:66-72`). Binary payloads are base64, and the crate does the arithmetic once: `MAXIMUM_KV_PAYLOAD_BYTES = 700 KiB` because "716,800 bytes encode to 955,738, which fits the 1,015,808-byte budget that remains once `FRAME_ENVELOPE_RESERVE_BYTES` [32 KiB] is held back" (`frame.rs:19-32`). Every KV bound in the workspace is defined in terms of this one constant "so the four cannot drift apart." Large uploads stream as a `PutKvStream` header followed by `KvUploadFrame` chunks on the same connection (`foks-agent-client/src/lib.rs:188-260`).

Operations are classified as reads or mutations by `Operation::is_mutation` (`crates/foks-agent-proto/src/message.rs:1225-1264`), an explicit allow-list of read operations so a new operation defaults to "mutation". The client wraps any transport error on a mutation as `Error::Ambiguous` (`lib.rs:169-184`), because a request that reached the agent may have been executed; the caller must then query status by submission ID rather than retry blindly. `Debug` for `Operation` redacts secret-bearing variants (`message.rs:1267-1290`).

### 6.3 Admission, caches and session lifetimes

The agent uses a set of semaphores (`main.rs:257-265`): a per-process cap on active connections, `MAXIMUM_CONCURRENT_READS = 4` blocking workers, one recovery slot, one local-maintenance slot, `MAXIMUM_CONCURRENT_CHAT_POLLS = 32`. Above that sits **profile admission** (`crates/foks-agent/src/profile_work.rs`): a FIFO of scopes where reads may share a profile, mutations hold it exclusively, and registry changes reserve the whole state root; "Disjoint profiles may pass a blocked waiter; no partial multi-profile hold" (`profile_work.rs:371-372`). Cross-process serialisation is by OS file locks on the profile directory and on the hard database (`crates/foks-client-app/src/runtime.rs:60-62`, `166-168`), with the client's process-local pinning span handling readers within one process.

Read caches (`crates/foks-agent/src/read_cache.rs`) hold authenticated users (30 s, 64 entries), team view tokens (5 h, 64 entries), resolved KV paths (30 s, 32 entries) and base transports (16). They are attached only to operations on the read allow-list; "Operations that may mutate state clear all entries before and after execution, including on failure" (`read_cache.rs:9-11`, `main.rs:3304-3321`). Because the server re-validates the device credential and all response data is verified locally, a stale entry can only cause a *failed* read, which `dispatch_result` retries once with fresh authentication (`read_cache.rs:13-18`). Every session's work is timed by phase (queue, session open, prepare, network wait, rescope) and reported so that a slow request can be attributed (`chat_poll.rs:188-194`, `timers.rs`).

### 6.4 The durable scheduler

`crates/foks-client/src/scheduler.rs` implements leased jobs in the `scheduled_jobs` table (`schema.rs:580-595`): `claim_due_scheduled_jobs(now, claimed_until, limit)` takes a lease, the handler runs, then `complete_scheduled_job` or `fail_scheduled_job` is called with the lease so a stale worker cannot overwrite a newer claim. Failures back off exponentially (`base * 2^failures`, capped, jittered; `scheduler.rs:186-215`) with the cap for security-relevant refreshes (user and team refresh) never exceeding the job's own interval. Handlers "must be idempotent: a crash after their side effect but before lease completion can cause the job to run again" (`scheduler.rs:81-87`). A thread-scoped restriction (`crates/foks-client-db/src/repositories/jobs.rs:18-45`) lets a worker holding only one profile's admission defer jobs that would reach other profiles, leaving them "exactly as [they were] found: still due, still unleased." The agent's loop treats the poll interval as an upper bound and wakes earlier when a pass reports an earlier due time (`main.rs:274-282`).

### 6.5 Credentials, the vault and the OS keyring

`foks-keystore` (`crates/foks-keystore/src/lib.rs:26-31`) stores small secrets "outside the application state directory. On macOS this uses generic-password records in the login Keychain. On Linux it uses the freedesktop Secret Service default collection with an encrypted D-Bus session. The namespace is an opaque, random state-root ID." The records are: a 32-byte vault master key (`MASTER_KEY_RECORD`), a state-root binding (`STATE_ROOT_RECORD`, so a state directory copied elsewhere does not match), the rollback checkpoint of section 1.3, database claims, and import-readiness markers. Everything else (account vault, protected mutation store, pending chat intents) is encrypted on disk under keys derived from the master key with distinct type IDs (`crates/foks-client-app/src/lib.rs:63-64`). A `PrivateFile` backend exists for development and headless use and is explicitly documented as having "no external rollback boundary" (`checkpoint.rs:280-282`).

Because a keychain read is "a native credential round trip (a Keychain query, or on Linux a fresh D-Bus session)", the session code caches the manifest read for the life of a checked session on the thread that opened it (`checkpoint.rs:194-215`), and the agent counts these reads in tests to keep the cost visible (`main.rs:5829-5850`).

Server-side passphrase state is a PPE parcel with a generation; the client classifies a lost enrollment/rotation race by re-reading the parcel rather than by parsing the server's generic error (`crates/foks-client/src/passphrase.rs:129-138`). The hard-state table `user_local_security` (`schema.rs:212-228`) records a local attestation that an account was created without a passphrase, so "an unattended owner rotation [can] distinguish a genuinely passphrase-free Rust account from a legacy Go account whose server-only PPE has no UserSettings link."

---

## 7. Invariants and clever tricks, collected

- **Two databases, two trust levels.** Hard state is public, monotonic, and constraint-checked by SQLite triggers; soft state is private plaintext, replaceable, but still monotonic per party via ciphertext-only rollback anchors that survive pruning.
- **A revision counter plus a random write token** turns SQLite into something whose *freshness* can be checked from outside: an exact cache key in-process and a rollback detector in the OS keyring.
- **Protected request first, journal second, erase last.** Every mutation's exact bytes are durable before any socket write; ambiguous outcomes are resolved by observing the chain, never by regenerating randomness.
- **Chain positions as idempotency keys** (unique partial indexes on live states) and **submission handles with keyed input commitments** for external callers.
- **Deterministic per-directory name MACs** make names searchable and paginable by the server without revealing them; a randomised box beside the MAC makes listings decryptable; the parent/version bound inside both stops relocation.
- **Nonces derived from (file, offset, final)** for chunks make chunk substitution a decryption failure rather than a check the client might forget.
- **Version vectors cite dirent IDs, not just versions**, so unlink-and-recreate cannot be confused with "unchanged"; path-scoped writes cite only what they walked; a closing cache check catches concurrent adds.
- **A per-channel hash chain with a bounded anchor ledger** lets the client detect a server rewriting history without storing whole messages; "missing predecessor" is reported as incomplete evidence rather than success.
- **Account-level inbox cursor with degraded flag and a 2-second drain gate** across teams.
- **Prepare/attempt-once/scan-forward** for chat sends; re-validation of key generation before delivery.
- **Team loader requires exactly the visible PTK parcels and a complete seed chain**, cross-checked against the verified key history; view tokens are cached under a credential digest.
- **Batched Merkle absence proofs** as a change marker for roster members.
- **Rotation roles as a pure function**; recipients must be recursively witnessed; DAG sweep child-before-parent with blocked sets expanded upward.
- **Socket bind-then-rename, lockfile with O_NOFOLLOW + flock, inode-pinned guard, peer UID check, and 5-second ownership re-validation.**
- **One payload constant** derived from the base64 expansion so four independent bounds cannot drift.

---

## 8. Suggested external references

- Keybase KBFS design and "Keybase filesystem" whitepaper: per-directory keys, encrypted names, block-level encryption. [VERIFY exact document title/URL]
- CryptFS / eCryptfs / gocryptfs design notes on filename encryption (deterministic filename encryption with per-directory IVs; gocryptfs "diriv"). gocryptfs's `diriv` per-directory IV is the closest analogue to FOKS's per-directory seed. [VERIFY]
- Bellare, Boldyreva, O'Neill, "Deterministic and Efficiently Searchable Encryption" (CRYPTO 2007) for the security model of deterministic/searchable encryption of names.
- Douceur et al., "Reclaiming Space from Duplicate Files in a Serverless Distributed File System" (2002) for convergent encryption, as background for why FOKS's name MACs are *keyed* per directory rather than convergent.
- SQLite documentation: `STRICT` tables, `WITHOUT ROWID`, partial indexes, triggers, WAL mode and `synchronous=FULL`, `secure_delete`, `SQLITE_OPEN_NOFOLLOW`, `PRAGMA application_id`/`user_version`.
- Mohan et al., "ARIES: A Transaction Recovery Method" (1992) as the canonical write-ahead-logging reference; Pat Helland, "Idempotence Is Not a Medical Condition" (ACM Queue, 2012) for idempotency keys and "the response was lost" reasoning; Stripe's idempotency-key API docs as a practical example. [VERIFY Helland citation details]
- Sagas: Garcia-Molina and Salem, "Sagas" (SIGMOD 1987), relevant to `federation_saga_operations` and the multi-step adapter intents.
- Optimistic concurrency control: Kung and Robinson, "On Optimistic Methods for Concurrency Control" (1981); HTTP `ETag`/`If-Match` as the everyday form of the KV version-vector precondition.
- Hash chains / tamper-evident logs: Crosby and Wallach, "Efficient Data Structures for Tamper-Evident Logging" (USENIX Security 2009); Certificate Transparency (RFC 6962) for Merkle-anchored history, which the team loader and change marker rely on.
- Unix domain socket security: `unix(7)` and `socket(7)` man pages (SO_PEERCRED), `open(2)` `O_NOFOLLOW`, `flock(2)`; the classic "TOCTOU" literature (Bishop and Dilger, "Checking for Race Conditions in File Accesses", 1996) for why the agent re-checks inodes after probing. macOS `LOCAL_PEERCRED` / `getpeereid(3)`. [VERIFY that tokio's `peer_cred` maps to `LOCAL_PEERCRED` on macOS]
- freedesktop Secret Service API specification; Apple Keychain Services documentation, for the native credential backends.
- NaCl/libsodium `secretbox` (XSalsa20-Poly1305) and XChaCha20-Poly1305 (draft-irtf-cfrg-xchacha) for the AEAD primitives; HMAC-SHA-512/256 (FIPS 180-4, RFC 2104) for the typed MACs.
- Leases: Gray and Cheriton, "Leases: An Efficient Fault-Tolerant Mechanism for Distributed File Cache Consistency" (SOSP 1989), for the scheduler's job leases and the server's view-token lifetime; exponential backoff with jitter (AWS Architecture Blog, "Exponential Backoff and Jitter", 2015).
- Long polling: RFC 6202 ("Known Issues and Best Practices for the Use of Long Polling and Streaming in Bidirectional HTTP") as background for the chat poll model.
