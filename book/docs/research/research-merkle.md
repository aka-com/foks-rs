# Research note: Transparency and verification in foks-rs

Scope: the Merkle tree (`crates/foks-merkle-store`), the client-side verifier (`crates/foks-verify`), the server's tree publication path (`crates/foks-server`, `crates/foks-server-db`), the client's persistence of verified heads (`crates/foks-client`, `crates/foks-client-db`), and the wire types in `crates/foks-proto`. All paths below are absolute under `/home/bnoland/projects/foks-rs/`. Nothing in the repository was modified.

The organising idea of this whole area is a single question: **how does a client that talks only to an untrusted server convince itself that the server is telling everyone the same story about who owns which keys?** FOKS answers it the way Keybase did (FOKS is by the same author) and the way the Certificate Transparency / CONIKS literature does: put every public claim into an append-only, hash-linked structure; have the server publish a signed digest of it ("the root") that changes every time anything changes; and make every claim the client consumes come with a short proof that it is inside the digest. Everything else in this note is machinery to make that idea sound, cheap, and offline-restorable.

The project's own overview of this design is in `docs/GUIDE.html`, sections 4 ("Secrecy vs Authority Chain") and 5 ("Compressed Patricia Merkle Trees & The Epoch DAG"). The GUIDE calls the server "an untrusted, Byzantine storage relay and sequencing engine" and summarises the authority chain as "an append-only signed link sequence, Merkle tree commitments, and client anti-rollback pins".

---

## 1. Typed hashing and canonical encoding (the substrate everything else stands on)

**Problem solved.** Every commitment in FOKS is a hash of some serialised object. If two implementations (Go and Rust) can serialise the same logical object differently, or if the same bytes could be interpreted as two different kinds of object, hashes and signatures stop being meaningful. FOKS needs "byte-for-byte identical encodings" and "no cross-type confusion".

**How it works.**
- Objects are encoded with "Snowpack", the project's canonical MessagePack profile (`crates/foks-snowpack`). Canonical means one and only one encoding per value; `validate_signable` rejects non-canonical encodings such as an `array16` header where a fixarray would have sufficed.
- Every hash is *domain-separated*: `prefixed_hash(type_id, bytes) = SHA-512/256(type_id as 8 big-endian bytes || bytes)`. See `crates/foks-crypto/src/lib.rs:708-713`, and the signable variant at `:719` which first validates canonicality. The type ids are 64-bit constants in `crates/foks-proto/src/lib.rs:16-39`, e.g. `MERKLE_NODE_TYPE_ID = 0xe941_750d_c5b9_6783`, `MERKLE_ROOT_TYPE_ID`, `MERKLE_BACK_POINTERS_TYPE_ID`, `MERKLE_TREE_RF_INPUT_TYPE_ID` (tree key derivation), `LINK_OUTER_TYPE_ID` (chain link hash), `HOSTCHAIN_LINK_OUTER_TYPE_ID`, `TREE_LOCATION_TYPE_ID`.
- Keyed commitments (used to hide usernames and device names while still committing to them) are `HMAC-SHA-512/256(key, type_id || bytes)`: `crates/foks-crypto/src/lib.rs:781-787`.

**Why.** The GUIDE ("Cryptographic Hashability") says it directly: because canonical Snowpack gives byte-identical encodings, "clients and servers can compute cryptographic hashes directly over serialized bytes", and domain separation "is enforced using 64-bit typed prefixes". A very concrete consequence of "bug-for-bug canonical" appears in section 4 below (the epoch 65,536 ceiling).

**References.** Standard practice; see the "domain separation" discussion in the NIST SP 800-185 (cSHAKE) rationale, or Bellare-Rogaway on hash-function domain separation [VERIFY exact citation]. MessagePack spec (msgpack.org) for the underlying wire format.

---

## 2. The Merkle tree: a compressed binary radix (Patricia) tree over 256-bit keys

**Problem solved.** The server needs an authenticated dictionary mapping 32-byte keys (derived from user ids, team ids, chain sequence numbers, usernames) to 32-byte values (hashes of chain links, or hashes of entity ids), such that (a) the whole dictionary is summarised by one 32-byte root, (b) any single lookup, present *or absent*, can be proven with O(depth) hashes, and (c) the construction is deterministic so that Go and Rust agree on the root for the same leaf set.

**Shape.** `crates/foks-merkle-store/src/node.rs:7-20` defines exactly two node kinds:

- `Leaf { key: [u8;32], value: [u8;32] }`
- `Interior { prefix_bit_start, prefix_bit_count, prefix: Vec<u8>, left: [u8;32], right: [u8;32] }`

This is a *binary radix tree with path compression* (a Patricia trie) over the 256 bits of the key, most-significant bit first (`bit_at` at `tree.rs:174`: `key[bit>>3] & (1 << (7 - (bit&7)))`). An interior node says: "all keys under me agree on bits `[start, start+count)` and those bits are `prefix`; the next bit, at position `start+count`, decides left (0) or right (1)". Its children start at bit `start+count+1`. The `prefix` bytes are *clamped*: bits outside the covered range are zeroed (`copy_and_clamp`, `tree.rs:178-192`), and `valid_prefix` (`node.rs:96-115`) rejects any node whose prefix is not in that canonical form. Because a leaf stores its whole key, a leaf can sit at any depth: the trie only splits where two keys actually diverge, so depth is about log2(n) for random keys rather than 256.

**Hashing.** A node's hash is `prefixed_hash_signable(MERKLE_NODE_TYPE_ID, encode(node))` (`node.rs:117`). The encoding wraps the node in an outer `[discriminant, variant(tag, payload)]` (leaf: discriminant 0, tag "1"; interior: discriminant 1, tag "0"), `node.rs:22-57`. The tree is *content-addressed*: the store is just `hash -> encoded bytes` (`store.rs`, `memory.rs`; on the server the SQLite table `merkle_nodes(node_hash PRIMARY KEY, exact_node)` in `crates/foks-server-db/src/schema/merkle.sql`). The empty tree's root is the all-zero hash `EMPTY_ROOT` (`lib.rs:27`).

**Deterministic construction.** `prepare(reader, current_root, changes)` at `tree.rs:5-55`:
1. Walks the *entire* existing tree from `current_root` collecting every leaf into a `BTreeMap` (`collect_leaves`, `tree.rs:57-115`).
2. Applies the `Set`/`Remove` changes (duplicates in one batch are an error).
3. Rebuilds the whole tree from the sorted leaf list with `build` (`tree.rs:117-152`): find the longest common bit prefix of the group (`common_prefix`), split at the first differing bit (`partition_point`, valid because the entries are sorted and MSB-first bit order equals byte order), recurse.
4. Diffs the resulting node set against the store: nodes that already exist with the same bytes are skipped; nodes that exist with *different* bytes raise `ConflictingNode`; new nodes are returned in the `Commit`.

Because the output depends only on the leaf *set*, insertion order is irrelevant. `tests/properties.rs` (`insertion_order_does_not_change_the_root`) rotates and reverses 64 inserts and asserts one root. `tests/official_vectors.rs` rebuilds the tree from the Go fixture's leaves and asserts the root and *every* proof match Go v0.1.9 byte-for-byte.

**Persistence and structural sharing.** Because nodes are content-addressed and immutable, unchanged subtrees are reused across epochs for free (a persistent data structure in the functional-programming sense). Every historical root therefore remains fully queryable, which is what lets the server answer `merkleLookup` against an *older* root (`crates/foks-server/src/net/session.rs:1149-1176`, `select_merkle_lookup_root`). Schema triggers stop a root from referencing a missing node and stop deletion of any node a root references (`merkle.sql`, triggers `merkle_roots_require_node`, `merkle_nodes_restrict_delete`).

**Fail-closed rebuild.** `collect_leaves` is also an integrity checker: it detects cycles (`Cycle`), recomputes each node's hash and compares it to its address (`HashMismatch`), checks that each interior node's `prefix_bit_start` equals the expected depth, that every leaf under a child really carries that node's prefix, and that the left/right children's keys have the right branch bit. `tests/corruption.rs` swaps a node's children, corrupts bytes, and checks the tree refuses to be used. The comment in `crates/foks-merkle-store/README.md` is candid about the trade-off: "updates are O(n) ... an intentional v1 tradeoff", with node-store traits left in place for a future incremental implementation.

**Why this design.** The GUIDE says the tree lets "thousands of lightweight clients verify identity states, team memberships, and public keys without downloading entire transaction ledgers", and that "by collapsing non-branching paths into interior nodes via radix prefix compression, the tree remains extremely compact." Compatibility with go-foks forced the exact shape; determinism forced the rebuild-from-sorted-leaves approach.

**References.**
- Merkle, R. "A Digital Signature Based on a Conventional Encryption Function", CRYPTO '87 (hash trees).
- Morrison, D. R. "PATRICIA: Practical Algorithm To Retrieve Information Coded in Alphanumeric", JACM 1968 (radix/path-compressed tries).
- Ethereum's "Modified Merkle Patricia Trie" (Yellow Paper, Appendix D) as a widely-read comparison point; FOKS's variant is binary and stores a bit-range prefix rather than hex nibbles.
- Laurie & Kasper, "Revocation Transparency" (2012) and Dahlberg, Pulls, Peeters, "Efficient Sparse Merkle Trees" (NordSec 2016) for the sparse-tree family this belongs to [VERIFY year for Dahlberg].

---

## 3. Tree keys: how an entity's chain lands in the tree, and hidden tree locations

**Problem solved.** Which 32-byte key does link number `n` of user `U`'s chain live under? The mapping must be computable by the client from already-verified state (so it can ask for exactly the right proof and can recognise a fake), yet ideally *not* computable by an outsider who only knows `U`, so the tree cannot be used to enumerate or watch a user's activity.

**How it works.** `crates/foks-merkle-store/src/key.rs`:

- `chain_key(chain_type, entity, sequence, location)` (`key.rs:9-22`) = `H(MERKLE_TREE_RF_INPUT_TYPE_ID, [chain_type, entity_id_bytes, sequence, location or null])`.
  - Main user chain: `chain_type = 0`. Link 1 uses `location = null`. Link `n >= 2` uses the *tree location disclosed by link `n-1`*.
  - Every link carries `next_location_commitment = H(TREE_LOCATION_TYPE_ID, encode(Binary(next_tree_location)))` where `next_tree_location` is 32 random bytes chosen by the signer (`crates/foks-crypto/src/lib.rs:724-729`; random generation e.g. `crates/foks-client/src/account.rs:179`). The location itself is not in the signed link; it is disclosed alongside the *next* link in chain responses (`UserChain.locations`, `crates/foks-proto/src/identity/user.rs:11-21`).
  - The verifier checks the disclosed location against the commitment and derives the key: `crates/foks-verify/src/user.rs:704-716` (full load) and `:948-957` (incremental).
- `username_key(name, host, sequence)` (`key.rs:24-42`): the name is first hashed into a pseudo-entity (`0x09 || H(NAME_HASH_PREIMAGE_TYPE_ID, [name, host])`), then keyed as `[1, name_entity, sequence, null]`. The *value* under that key is `username_leaf(uid) = H(ENTITY_ID_MERKLE_VALUE_TYPE_ID, [1, variant("1", uid)])` (`key.rs:44-52`). `sequence` here is the "ownership sequence" of the name: sequence 1 is the first owner, 2 the next, and so on.
- The *value* under a chain key is the hash of the exact link bytes: `H(LINK_OUTER_TYPE_ID, exact_link)`.

**The hidden-chain trick (subchains).** Users and teams also have *generic subchains*: `CHAIN_TYPE_USER_SETTINGS = 2` and `CHAIN_TYPE_TEAM_MEMBERSHIP = 4` (`crates/foks-proto/src/lib.rs:75-76`). For these, even the *first* link has a location, and the location is derived from a secret seed: `subchain_tree_location(seed, chain_type) = HMAC(seed, CHAIN_LOCATION_DERIVATION_TYPE_ID, encode([chain_type, variant(None)]))` (`crates/foks-crypto/src/lib.rs:588-608`). The seed's commitment is placed in the entity's eldest link metadata as `ChangeMetadata::Eldest { subchain_location_commitment }` (`crates/foks-proto/src/identity/chain.rs:1404-1412`; encoded at `:270-296`), and the seed is disclosed to authorised readers in the `GenericChain` response (`location_seed`, `crates/foks-proto/src/identity/generic.rs:10-15`). The client re-derives and cross-checks: `crates/foks-client/src/team.rs:1700-1712`. The server does the same when it inserts settings links (`crates/foks-server/src/net/session.rs:284-310`).

The Go oracle test `tools/foks-v019-oracle/subchain_derivation_test.go` pins this derivation: seed `0x35 * 32`, chain type TeamMembership, expected location `90de9044fcee5b28...4096`; the Rust test at `crates/foks-crypto/src/lib.rs:4573` checks the same vector. The foks-crypto docstring calls it "the hidden first Merkle location for a user or team subchain".

**Why.** The GUIDE's fixture list mentions "hidden location commitments" among signup artefacts. The purpose is *unlinkability*: the tree key of link `n+1` depends on a random value revealed only in link `n`'s response, and a subchain's first key depends on a seed revealed only to parties entitled to read that subchain. A passive observer of the tree (or a party with `merkleLookup` access) cannot compute the keys of a chain it is not entitled to read. This is the same problem CONIKS solves with VRF-derived indices; FOKS uses commit-then-reveal and HMAC derivation instead [VERIFY the CONIKS analogy before publishing; it is the author's interpretation, not stated in the code].

**Invariant worth stating in the book.** "An honest server must publish link `seqno+1` under exactly `chain_key(type, entity, seqno+1, location_seqno)`; any other placement fails the increment verifier." This sentence is the docstring of `user_chain_next_link_key` at `crates/foks-verify/src/user.rs:1580-1595`, and it is what makes absence proofs meaningful (section 6).

---

## 4. Epochs, signed roots, and the skip-pointer "epoch DAG"

**Problem solved.** The tree changes on every mutation. A client that verified epoch 995 last week and now sees epoch 998 must confirm that 998 is a *descendant* of 995 (the server did not fork, rewind or replace history) without downloading three intermediate trees. This is the analogue of Certificate Transparency's "consistency proof", but structured very differently.

**The root object.** `MerkleRoot` (`crates/foks-proto/src/host.rs:276-330`): `{ epoch, time, back_pointers: [u8;32], root_node: [u8;32], hostchain: { seqno, hash }, extensions }`. Epoch 0 is rejected on decode. Root hash = `H(MERKLE_ROOT_TYPE_ID, exact_root_bytes)`. The root is served as a `SignedBlob { inner: exact_root, signature }` signed with `MERKLE_ROOT_BLOB_TYPE_ID` by the host's *delegated Merkle signer key*, which must be an active `ENTITY_HOST_MERKLE_SIGNER` key in the hostchain (section 5). Note the root commits to the hostchain tail; this binds "which host key set was current" to "which tree was current".

**Back-pointer sequence.** `back_pointer_sequence(epoch)` at `crates/foks-merkle-store/src/history.rs:13-31` (identical copy in `crates/foks-verify/src/merkle.rs:772-790`):

```
0,1 -> []      2 -> [1]      3 -> [2,1]      4 -> [3,2,1]
otherwise: d = 1; while epoch > d { push(epoch - d); if epoch & d != 0 { break }; d <<= 1 }
```

So epoch E points to E-1, E-2, E-4, E-8, ... stopping after the first pointer whose distance corresponds to a set bit of E. Examples from `tests/history.rs`: 8 -> [7,6,4]; 16 -> [15,14,12,8]; 998 -> [997,996]; 996 -> [995,994,992]. The number of pointers equals (trailing zeros of E) + 1. The root's `back_pointers` field is `H(MERKLE_BACK_POINTERS_TYPE_ID, [[epoch, hash], ...])` over that list (`history.rs:33-56`), or over `Null` when the list is empty. `tests/history.rs` (`official_back_pointer_lists_hash_to_the_published_roots`) checks the Rust hash against Go fixtures for epochs 996-998. The server stores the pointers relationally too (`merkle_back_pointers` table with `CHECK (target_epoch < root_epoch)` and a foreign key to `merkle_roots(epoch, root_hash)`).

This is a deterministic skip list laid over the epoch sequence: each root commits to a handful of ancestors at power-of-two distances, so any two epochs are connected by O(log N) hops.

**Choosing the hops: `collect_roots(start, end)`** (`history.rs:58-77`; verifier copy `merkle.rs:792-816`). Greedy descent: from `start`, look at its back pointers, jump to the smallest target that is still `>= end`, record every pointer target seen as a "sibling" whose hash will be needed, repeat until `start <= end`. Returns `(path, siblings)`. Fixture: `collect_roots(998, 995) = ([998, 996], [997, 996, 995, 994, 992])`. `merkle_history_requirements(latest, pinned)` (`merkle.rs:255-283`) turns this into a request: full roots for path epochs other than the endpoints (the client already holds the endpoints) and bare hashes for the sibling epochs. The server's `getHistoricalRoots` (`session.rs:977-1006`) answers with `HistoricalMerkleRoots { roots: Vec<MerkleRoot>, hashes: Vec<[u8;32]> }` (`host.rs:345-380`), capped at 64 epochs per list (`decode_epochs`, `session.rs:1326`).

**Verifying the advance: `verify_merkle_advance`** (`merkle.rs:309-497`):
1. Re-derive the pinned root's hash from its stored bytes and confirm it matches the pinned hash (a corrupted pin is a `MerkleFork`).
2. Decode the latest root; reject `latest.epoch < pinned.epoch` as `MerkleRollback`; require `latest.hostchain == trusted tail` (`MerkleHostchainMismatch`).
3. If epochs are equal, the hashes must be equal (`MerkleFork`) and nothing else changes.
4. Check the historical response has exactly the requested *shape* (counts and epochs match; `MerkleHistoryShape`), then build a table `epoch -> hash` from: the pinned root, the latest root, every supplied full root (hashed locally), every supplied bare hash, and every hash already in the pinned root's `authenticated_roots`. Any epoch supplied twice with different hashes is `MerkleBackPointer`/`MerkleFork`.
5. Walk `path` from latest downward. For each epoch on the path: recompute its pointer list from the table, hash it, and compare with that root's `back_pointers` field (this is the step that actually authenticates the bare hashes: they are only accepted because a signed-or-linked root commits to them). Also confirm that the *previous* path root's pointer list contained this epoch with this hash.
6. Finally require that the last pointer list contains `(pinned_epoch, pinned_hash)`.

If all of that holds, every hash in the table is now an *authenticated root*, and the result carries them as `AuthenticatedMerkleRoots` (a `BTreeMap<epoch, hash>`, `merkle.rs:154-186`) whose `merge` rejects any epoch proved to two different hashes (`MerkleFork`).

The signed wrapper `verify_signed_merkle_advance` (`merkle.rs:287-307`) first checks the Merkle-signer signature against the verified hostchain, then calls the above. The unsigned function is deliberately "not an authority" (docstring `merkle.rs:299-303`); sealed client state rejects a `SkipPath` whose `signed_root` is empty (`crates/foks-client/src/lib.rs:635-657`, test `unsigned_merkle_skip_evidence_cannot_become_durable_authority`).

**Authenticating roots cited by chain links.** Chain links cite the root the signer saw when signing (section 6). Those epochs are generally *older* than the client's latest root, so `authenticate_historical_roots_from_latest` (`merkle.rs:191-253`) runs the same skip-path verification in reverse: for each target epoch, it manufactures an "untrusted target" `VerifiedMerkleRoot` from the server-supplied full root and proves that the *trusted* latest root descends from it. The client batches these requests, at most 64 targets per call (`crates/foks-client/src/auth.rs:1180-1215`). The verify-crate test `user_selected_history_is_proved_from_latest_and_tampering_fails` (`crates/foks-verify/src/lib.rs:347-452`) flips one bit of one sibling hash and shows the whole authentication fails.

**The epoch 65,536 ceiling (a lovely boundary condition).** Epoch 65,536 = 2^16 is the first epoch whose pointer list has 16 entries. MessagePack encodes 16 elements as `array16`; go-foks v0.1.9's canonicality validator rejects any `array16` "where fixed array encoding is possible" (the Go oracle test `tools/foks-v019-oracle/merkle_boundary_test.go:11-25` demonstrates the Go failure). Rust matches bug-for-bug: `MAX_CANONICAL_MERKLE_EPOCH = 65_536` and `back_pointer_hash` returns `EpochLimitExceeded` when the pointer count is in `16..=31` (`history.rs:11`, `:33-40`; tests `maximum_canonical_epoch_is_rejected`). The README explains: "Because epoch 65,536 cannot be published, a compatible tree cannot advance to later epochs either ... Raising the ceiling requires a coordinated upstream canonicality/wire fix." The server surfaces it as `MERKLE_VERIFY_ERROR` (`crates/foks-server/src/error.rs:72`). For a book this is a perfect illustration of how "canonical encoding" and "hash commitment" interact: a serialisation quirk becomes a hard protocol limit because changing the bytes would change every hash.

**Why this design rather than CT-style consistency proofs.** CT's consistency proof works for a Merkle *log* (append-only list of leaves); FOKS's tree is a *dictionary* whose leaves are overwritten, so a log-style consistency proof is not available. The skip list over roots is the standard alternative (Keybase used the same construction). The GUIDE: "It proves monotonic continuation in O(log N) hashes without downloading intermediary Patricia trees. Result: Atomic, rollback-proof head acceptance verified in memory before hard-state commit."

**References.**
- RFC 6962 (Certificate Transparency), section 2.1.2, for the log-consistency-proof contrast.
- Pugh, W. "Skip Lists: A Probabilistic Alternative to Balanced Trees", CACM 1990; and Maniatis & Baker, "Secure History Preservation Through Timeline Entanglement" / "authenticated append-only skip lists" (USENIX Security 2002) for the authenticated-skip-list idea [VERIFY which Maniatis paper introduces AAOSL].
- Keybase, "Keybase's Merkle tree" and "Merkle root skip pointers" documentation [VERIFY exact URL; historically keybase.io/docs/server_security/merkle_root_in_sigchain and related pages].
- Haber & Stornetta, "How to Time-Stamp a Digital Document", 1991 (hash-linked epochs).

---

## 5. The hostchain: the server's own key history, and stacked signatures

**Problem solved.** Who signs the Merkle root, and how does the client know that key belongs to the host? A host may rotate keys, add TLS CAs, or revoke a compromised key, and clients must follow those changes without a central PKI.

**How it works.** `crates/foks-verify/src/host.rs`.
- The host has an *identity* equal to its genesis Ed25519 key (`host_id = ENTITY_HOST byte || key`). The hostchain is a list of `HostchainLink { inner, signatures }` (`crates/foks-proto/src/host.rs:48-99`); the inner change carries `chainer { seqno, previous, time }`, `host`, `signer`, and a list of `HostchainChangeItem::{Key, Revoke, TlsCa}`.
- `verify_hostchain_link` (`host.rs:348-429`) is a small state machine. Rules: seqno must be prior+1; genesis (seqno 1) has no `previous` and must be signed by the host key itself; later links must carry `previous == hash of prior link` (`H(HOSTCHAIN_LINK_OUTER_TYPE_ID, link)`), the same `host`, and a signer that is an active, unrevoked `ENTITY_HOST` key. Each link is signed by *every key it introduces* plus the signer (proof of possession), in that order; `SignatureCount` mismatch is fatal.
- **Stacked signatures.** Signature `i` is computed over `encode([inner, signatures[..i]])` (`signing_bytes`, `host.rs:55-74` in foks-proto): each later signer signs the earlier signatures too, so the stack is ordered and cannot be reshuffled. The same mechanism is used for user links (`crates/foks-proto/src/identity/chain.rs:948-972`) where the eldest link is signed by the PUK key, optionally a YubiKey subkey, and the device key.
- Delegated service keys are typed: `ENTITY_HOST_MERKLE_SIGNER`, `ENTITY_HOST_METADATA_SIGNER`, TLS CA. `verify_with_delegated_blob_key` (`host.rs:454-474`) accepts a blob if *any* currently active key of the right type verifies it. A subtle rule at `host.rs:174-182`: a revoked *hostchain-signing* key can never be re-added, but revoked delegated keys can be ("Go permits delegated metadata, Merkle, and TLS keys to be restored after revocation").
- `verify_public_host` (`host.rs:262-322`) is the bootstrap: verify the whole chain from genesis, verify the public zone (service endpoints) under the metadata key, verify the Merkle root under the Merkle key, and check `verify_merkle_binding`: the root's `hostchain.{seqno,hash}` must equal the chain tail (`host.rs:446-452`). The result is the first `VerifiedMerkleRoot`, with evidence `SignedBootstrap`.
- Host key rotation on the server (`crates/foks-server/src/host/rotation.rs:300-360`) appends a hostchain link *and* mints a new Merkle epoch with the *same* `root_node` but a new `hostchain` tail, then re-runs `verify_public_host` on its own output before publishing. The client records this as `MerkleRootEvidence::SignedRefresh` (`merkle.rs:22-26`): "Host-key rotation produces a new signed probe anchor without invalidating roots used by persisted user and team snapshots."

**References.** Keybase "sigchain" docs for the general pattern; for TLS-CA-in-chain, compare DANE (RFC 6698) as an alternative approach [VERIFY].

---

## 6. Signature chains and the user-chain verifier

**Problem solved.** A user's set of devices and shared keys evolves (provision, revoke, rotate). The client needs to reconstruct the *current* key state from history, in a way that the server cannot edit, reorder, truncate, or fork.

**What a link contains.** `UserLink` (`crates/foks-proto/src/identity/chain.rs:13-17`) is stored as *exact bytes* "because both stacked signatures and the Merkle leaf commit to these bytes". Decoded (`UserEldest`, `chain.rs:1455-1470`; `DecodedGenericLink`, `chain.rs:74-84`), every link carries:
- `seqno` and `previous: Option<[u8;32]>` (hash of the prior link; `None` only for seqno 1);
- `root: TreeRoot { epoch, hash }`: the Merkle root the signer observed when signing (a *cross-link* from chain to tree);
- `time`;
- `next_location_commitment` (section 3);
- the fully-qualified entity `[uid, host]` and `signer`;
- the payload: member changes, shared-key (PUK/PTK) generations, and `metadata` commitments (username, device name, eldest subchain seed, team name, index range, load floor).

**How integrity is checked.** `verify_user_chain_at_root` (`crates/foks-verify/src/user.rs:629-827`):
1. Response root gating: the response's root epoch must be `>= latest.epoch`, byte-identical if equal, and its hash must be in the authenticated set (`UntrustedUserRoot`).
2. Disclosures: for every `Username`/`DeviceName` commitment in the links, the response must supply the opening (name, sequence, 16-byte commitment key) and `HMAC(key, NAME_COMMITMENT_TYPE_ID, [name, sequence])` must equal the commitment (`verify_user_disclosures`, `user.rs:1324-1416`). Names are Unicode-normalised (`normalize_username`).
3. The username's own little chain in the tree: for ownership sequences 1..k-1 the name key must be *present* (any leaf), sequence k must be a leaf whose value is `username_leaf(uid)`, and sequence k+1 must be *absent* (`user.rs:1386-1414`). This proves that "this name currently belongs to this uid and nobody has taken it since".
4. Per link `n`: `seqno == n`, `previous == hash(link n-1)`, uid/host match, and `authenticated_roots[link.root.epoch] == link.root.hash` (the cited root must be an authenticated ancestor of latest, `UserChainContinuity`). Location commitment checked; tree key derived; `verify_merkle_path(path, key, Some(link_hash), root_node)` must succeed.
5. Eldest link: exactly two or three stacked signatures (PUK key, optional YubiKey subkey, device key), the uid must equal the PUK verify key re-typed as a user id (`persistent_user_id`, `user.rs:1530`), and metadata order must be `[Username, DeviceName, Eldest]`.
6. Subsequent links are fed to a replay state machine (`UserReplayState`, `crates/foks-verify/src/user_transition.rs`) that enforces policy: one member change per link, signer must be an active device, shared-key generations increase, revocation rotates keys, an owner device must remain, etc. (see the `UserTransitionRule` list in `crates/foks-verify/src/error.rs:96-115`).
7. **Terminal absence proof**: after the last link, the key for link `n+1` (using the last disclosed location) must be proven *absent* under the response root. Without this, a server could silently truncate a chain.

Incremental loads (`verify_user_chain_increment_at_root`, `user.rs:869-1019`) replay only the suffix, but still require the first disclosed location to equal the pinned `next_tree_location`, `previous` to equal the pinned tail hash, and the same terminal absence proof.

**The team chain** (`crates/foks-verify/src/team.rs`) follows the same skeleton with team-specific policy (roster, shared PTK schedule, rational index ranges, member load floors), name-tree proofs for team names (`verify_team_disclosures`, `team.rs:1386-1453`), and a quirk for ad-hoc teams: the server returns two absence proofs for a host-wide `-` name row that no team owns, and the Rust verifier reproduces Go's counting (`verify_adhoc_team_name_paths`, `team.rs:1588-1613`).

**Generic subchains** (settings, team membership) are verified in the client crate (`crates/foks-client/src/team.rs:1690-1800`): one signature per link, `previous` continuity, keys derived with `chain_key(chain_type, entity, seq, location)` starting at the seed-derived hidden location, and each link's cited root recorded for later authentication.

**The bidirectional binding.** Links cite roots (chain -> tree) and roots include link hashes as leaves (tree -> chain). The server enforces the forward direction at submission time: `require_cited_root` (`session.rs:1264-1277`) demands the cited `(epoch, hash)` exists and is not newer than the current head; `mutation_cites_superseded_user_head` (`session.rs:1279-1305`) detects a link built against a stale chain head. This is Keybase's "merkle root in sigchain" idea: a link that cites a root proves the signer had seen at least that much history, which limits how far back a server could rewind without the signer noticing.

**Why exact bytes and evidence retention.** The client stores the exact server responses as `evidence_bytes` (concatenated as a variant-"1" list of segments, `user.rs:1125-1160`) and on restart re-verifies them (`restore_verified_user`, `user.rs:1021`) to reproduce the projected state "byte-for-byte" (`PersistedUserEvidence` otherwise). `user_evidence_root_epochs` extracts every root epoch the evidence depends on so the local Merkle garbage collector keeps them (section 7). Tests: `offline_user_and_team_restore_with_only_referenced_roots` (`lib.rs:771-838`).

**References.**
- Keybase, "Keybase sigchain" and "Teams sigchain" docs [VERIFY URLs].
- Li, Krohn, Mazières, Shasha, "Secure Untrusted Data Repository (SUNDR)", OSDI 2004: the fork-consistency model that describes exactly what one client can and cannot detect about an equivocating server (see section 9 for how this applies).

---

## 7. Compressed Merkle proofs: presence, exact value, and absence

**Problem solved.** Proofs must be small, must be checkable with only the query key and an authenticated root, and must not be redirectable (a proof about key A must not be accepted as a proof about key B).

**Wire shape.** `MerklePathCompressed { edges: Vec<[u8;33]>, terminal }` (`crates/foks-proto/src/identity/merkle.rs:9-27`). Each edge is one interior node on the path, compressed to *33 bytes*: byte 0 is the node's `prefix_bit_count`, bytes 1..33 are the *sibling* child's hash. The terminal is either `Leaf { leaf: value, found_key: Option<key> }` (`found_key` is `Some` only when the leaf reached has a *different* key than the query) or `PrefixMiss { prefix_bit_start, prefix_bit_count, prefix, left, right }` (a full interior node whose prefix the query does not match).

**Generation** (`crates/foks-merkle-store/src/proof.rs:9-87`): descend from the root; at each interior node, if the query does not carry the node's prefix bits, stop with `PrefixMiss`; otherwise pick the child by the branch bit, push `(count, sibling)`, continue. At a leaf, return its value and, if the key differs, the found key. Structural checks along the way fail closed (`MissingNode`, `HashMismatch`, `Cycle`, `InvalidNode`). An empty tree cannot be proven against (`EmptyTree`).

**Verification** (`crates/foks-verify/src/proof.rs:33-144`), the clever part:
- The verifier never trusts the proof to tell it *which* prefix bits an interior node covered. For each edge it takes `count` from the edge, then *recomputes* the prefix from the **query key's own bits** (`copy_and_clamp(query_key, cursor, count)`) and the branch direction from the query key's next bit. It then rebuilds the interior node encoding and hashes it. Consequently a path that hashes up to the authenticated root at all is necessarily a statement about *this* key: as the `classify_user_chain_tail` docstring puts it (`user.rs:1617-1628`), "Which query a path was minted for does not matter... a path that reconstructs the root at all is proof about `key` and nothing else."
- Three expectations are supported (`ExpectedLeaf`, `proof.rs:26-31`): `Exact(value)` (leaf must be at the query key with this value), `Present` (leaf at the query key, any value; used for older name-ownership sequences), and `Absent`.
- **Absence** is accepted in two forms: (a) `Leaf` with `found_key != query` where the found key agrees with the query on every bit consumed so far (`bits_equal`, `proof.rs:67-71`), meaning the trie's path for the query terminates at somebody else's leaf; (b) `PrefixMiss` where `prefix_bit_start == cursor` and the query does **not** match the node's prefix (`proof.rs:92-99`). In both cases the terminal node is hashed exactly as the tree would hash it, so the absence claim is bound to the root.
- Finally the reconstructed hash must equal `expected_root_node` (the `root_node` field of an authenticated `MerkleRoot`).

**Why 33-byte edges.** The prefix bits are redundant given the query key, and the child's own hash is what the verifier is computing; only `count` and the sibling are information the verifier lacks. Go v0.1.9 fixed this format and the Rust fixtures check every official proof byte-for-byte.

**References.** Merkle audit paths, RFC 6962 section 2.1.1; CONIKS (Melara, Blankstein, Bonneau, Felten, Freedman, USENIX Security 2015) for absence proofs in prefix trees.

---

## 8. Server side: minting an epoch per mutation, atomically

**Problem solved.** Every accepted mutation must appear in a signed root, and nobody should ever observe a root that does not correspond to stored, hash-consistent nodes.

**How it works.** Each mutating request (signup, user link, team link, generic link, host rotation) is one SQLite transaction executed by a single writer actor (GUIDE: "mutating RPC endpoints ... send transactional closures across a bounded Tokio MPSC channel to the writer actor. Mutations execute sequentially and commit atomically"). The pattern, e.g. `crates/foks-server/src/net/session.rs:660-760` (user mutation) and `:280-360` (settings link), also `crates/foks-server/src/services/generic.rs:201-228`, `team_admin.rs:454-478`:
1. Read the current head (`current_root`), and check the link's cited root (`require_cited_root`).
2. Compute leaf changes (`chain_key(...)` -> link hash; for signup also `username_key` -> `username_leaf(uid)`, `crates/foks-server/src/identity/signup.rs:188-225`).
3. `foks_merkle_store::prepare(node_reader, head.root_node, changes)` (full rebuild; O(n) per mutation as documented).
4. `root_epoch = head.epoch + 1`; load the roots at `back_pointer_sequence(root_epoch)` from `merkle_roots`; `back_pointer_hash`.
5. Build `MerkleRoot { epoch, time: now/1000, back_pointers, root_node, hostchain: current tail }`, hash it, sign it with the `KeyPurpose::Merkle` key as a `SignedBlob`.
6. Hand everything to the DB layer, which re-validates (`crates/foks-server-db/src/identity.rs:365-395`, `team.rs:826-856`): node count and back-pointer count under configured maxima, every node re-decodes and re-hashes to its address, the back-pointer epoch list equals `back_pointer_sequence(root_epoch)`, `root != EMPTY`.
7. `publish_merkle` (`crates/foks-server-db/src/generic.rs:355-411`): insert-if-absent for content-addressed nodes (conflicting bytes are an error), upsert `merkle_leaves(leaf_key, leaf_value, epoch)`, insert `merkle_roots`, insert `merkle_back_pointers`, update the singleton `merkle_root_heads`.

So there is *no cross-request batching* in foks-rs: one mutation = one epoch, and "Root publication and signing are one atomic standalone commit" (comment at `session.rs:1079`). The `merkle_leaves.epoch` column records the epoch at which a key was last written, which is what `merkleCheckKeyExists` returns (`session.rs:1068-1084`).

**Read path.** Reads use a separate WAL read pool (`read_database`, `session.rs:900-910`). `merkleLookup`/`merkleMultiLookup` (`session.rs:1008-1049`) select a root (current or a requested historical epoch, optionally requiring the signed form), validate the stored root's binding (`decode_stored_root`, `session.rs:1253-1262`), and run `proof` per key. Proof-generation errors other than storage errors are reported as `MERKLE_VERIFY_ERROR` rather than retries ("Merkle integrity failures are not laundered as retries", test at `session.rs:1962`).

---

## 9. Client side: monotonic heads, evidence, restoration, and equivocation detection

**Problem solved.** A client must (a) never accept an older or conflicting root once it has accepted a newer one, (b) survive restarts without re-downloading and re-proving history, and (c) do this without ever trusting its own SQLite file blindly, since local storage is untrusted hard state that is re-verified on load.

**The pin.** `crates/foks-client-db/src/schema.rs:148-164` defines `merkle_roots(host_id, epoch, root_hash, root_bytes)` and the singleton-per-host `merkle_heads(host_id, epoch, root_hash, evidence_kind, anchor_epoch, evidence_bytes)`. Every verified user/team/generic-chain row carries `merkle_epoch, merkle_root_hash` with a foreign key into `merkle_roots`.

`accept_merkle_root` (`crates/foks-client-db/src/lib.rs:1652-1718`) is the monotonic gate:
- `root.epoch < stored.epoch` -> `MerkleRollback { stored, received }` and the whole transaction (including any hostchain advance in the same call) rolls back (`repositories/host.rs:8-9`).
- equal epoch -> hash *and* root bytes must be identical, else `MerkleFork { epoch }`.
- greater -> accept, merge authenticated roots, and store the head with evidence flattened to `LocalCheckpoint { signed_root }`. The comment: "Acceptance receives a verifier-issued capability. Retain its signed head, not the proof history used to obtain that capability."

The same discipline applies to host chains (`ChainRollback`, `ChainFork`, `GenesisChanged`, `HostIdentityChanged`, `repositories/host.rs:70-110`), with a neat prefix test: a longer chain is accepted only if the stored chain bytes are an encoded-array prefix of the new bytes.

**Evidence kinds.** `MerkleRootEvidence` (`crates/foks-verify/src/merkle.rs:10-35`): `SignedBootstrap` (from a probe), `SignedRefresh` (host key rotation), `SkipPath` (an advance, carrying the exact historical response), `LocalCheckpoint` (flattened durable form). Older databases stored the *recursive* evidence spine; schema v39 migrates them to flat checkpoints with an iterative, allocation-free parser written specifically to avoid recursive decoding of a 6000-deep spine (`crates/foks-client-db/src/merkle_checkpoint.rs:17-81`, test `legacy_spine_is_iterative_and_strict_beyond_old_depth_limits`). The recursive restorer in the verifier also caps depth at 4096 (`merkle.rs:584-586`).

**Restoration.** On open, `restore_local_merkle_checkpoint` (`merkle.rs:501-547`) re-verifies the signed head against the *stored* hostchain and checks that each stored authenticated root's bytes hash to its stored hash, but "deliberately does not re-prove the historical roots" (the docstring says: "Never use it for network-supplied roots"). The trust boundary is explicit: what is proved is that the database is *self-consistent* and its head is *signed by the host*; the rest is trusted local state. `restore_merkle_anchor`/`restore_merkle_evidence` (`merkle.rs:549-770`) can fully re-prove a recursive spine when present.

**Serialising verify-and-accept.** `crates/foks-client/src/pinning.rs` provides a per-database, thread-reentrant span so that "fetch head, verify, accept, then accept the chain verified under it" cannot interleave with another operation on the same host database; otherwise a slower operation accepting an older head would read as a rollback of the newer one (module docstring, lines 1-14).

**The advance driver.** `advance_merkle_root` (`crates/foks-client/src/auth.rs:697-747`): restore the pinned host and anchor from the local DB, fetch `getCurrentRootSigned`, compute `merkle_history_requirements(latest, anchor)`, fetch `getHistoricalRoots` if needed, `verify_signed_merkle_advance`, then `accept_verified_merkle_root`. Chain-cited roots are then authenticated in batches (`auth.rs:880-905`, `:1180+`).

**Garbage collection.** Because chain evidence pins roots, `compact_merkle_roots` (`crates/foks-client-db/src/merkle_gc.rs:28-197`) computes the live set from heads, users, teams, generic chains, every epoch named in stored evidence (`user_evidence_root_epochs`, `team_evidence_root_epochs`), import markers, and caller-supplied pins, refuses to run while any workflow is non-terminal, and deletes only unreferenced roots; it starts automatically above 128 roots.

**Cheap "has anything changed?" probes.** `crates/foks-client/src/change_marker.rs` (module docstring, lines 1-15) is a nice application of absence proofs: to learn whether *any* of a team's members rotated keys, the client asks a single `merkleMultiLookup` for two keys per member, derived from pinned state alone: `user_chain_next_link_key` (link `seqno+1` at the committed location) and `user_chain_next_username_key` (name ownership `sequence+1`). `classify_user_chain_tail` (`user.rs:1630-1639`) returns `Unchanged` only if a valid absence proof under an authenticated root comes back; anything else, including a malformed response, degrades to "reload the chain", so the server "can at worst force the caller to do the chain load it would have done anyway".

**What equivocation detection actually covers (important for honest exposition).** Within one client's view, the design guarantees: no rollback (epoch monotonic), no fork at an epoch it has seen (hash equality at equal epoch; `AuthenticatedMerkleRoots::merge`), no unlinked history (every newer root must skip-path back to the pinned one; every older root cited by a link must skip-path forward to the latest), no truncated chains (terminal absence proofs), and no substituted keys (leaf value = exact link hash, and links are signed by keys authorised by the replayed chain). What it does *not* do, as far as the code shows, is compare roots *between* clients or with third-party auditors; a server that maintains two permanently separate histories for two clients is detectable only if those clients exchange roots out of band. That is precisely the *fork consistency* guarantee of SUNDR, and the GUIDE's phrase "cannot rewind history undetected" should be read in that light. The cross-link of roots into signed chain links (section 6) narrows the server's room: any link a client submits publishes, in the tree, the root that client saw, so other clients that read that chain learn about that root and must be able to skip-path to it from their own view. [VERIFY: I did not find a gossip or auditor mechanism; state this as "not present in foks-rs as inspected" rather than as a design limitation of FOKS in general.]

---

## 10. Invariants and tricks worth calling out in the book

- Bit-level path compression with *clamped* prefixes gives one canonical encoding per interior node; the verifier rejects non-clamped prefixes (`node.rs:96-115`).
- MSB-first bit order means sorting keys as byte strings is the same as trie order, so `partition_point` on a sorted slice finds the split for free (`tree.rs:140`).
- Content addressing makes the store idempotent and makes historical roots free to keep; a node written twice with different bytes is treated as corruption, never overwritten (`memory.rs:31-39`, `generic.rs:355-370`).
- Proof edges omit the prefix bits; the verifier reconstructs them from the query key so proofs cannot be redirected (`proof.rs:41-58`).
- Absence proofs are first-class and are used for three distinct jobs: chain termination, username-ownership termination, and cheap change detection.
- Every link commits to the *next* link's tree location; locations are random, disclosed lazily, and for subchains derived from a secret seed, so tree keys are unlinkable to outsiders (`key.rs`, `crypto/lib.rs:588-608`).
- Roots commit to the hostchain tail; the hostchain authorises the key that signs the root; the root's leaves commit to chain links; chain links commit to roots. Every arrow in that cycle is verified.
- Back-pointer lists are hashed *into* the root, so a bare hash of an older epoch becomes trustworthy the moment a trusted root's pointer hash is reproduced from it (`merkle.rs:421-446`).
- The 65,536-epoch ceiling: a canonical-encoding rule turns into a hard protocol constant (`history.rs:11-40`).
- Persisted evidence is exact wire bytes that are replayed on restore; the verifier, not the database, is the source of truth (`user.rs:1021`, `team.rs:1615`).
- Rollback checks are done in SQLite transactions so a failed Merkle check also rolls back the hostchain update that accompanied it (`repositories/host.rs:8-9`).
- Legacy recursive evidence is parsed iteratively to avoid deep-recursion hazards on adversarial or merely old data (`merkle_checkpoint.rs:17-81`).

---

## 11. Suggested reading list for this chapter

Primary (confident):
- Merkle, R. C. "A Digital Signature Based on a Conventional Encryption Function." CRYPTO 1987.
- Laurie, Langley, Kasper. RFC 6962, "Certificate Transparency" (2013); RFC 9162 for the updated version.
- Melara, Blankstein, Bonneau, Felten, Freedman. "CONIKS: Bringing Key Transparency to End Users." USENIX Security 2015.
- Li, Krohn, Mazières, Shasha. "Secure Untrusted Data Repository (SUNDR)." OSDI 2004 (fork consistency).
- Morrison, D. R. "PATRICIA." JACM 15(4), 1968.
- Pugh, W. "Skip Lists." CACM 33(6), 1990.
- Haber & Stornetta. "How to Time-Stamp a Digital Document." Journal of Cryptology, 1991.
- Keybase documentation on the Merkle tree and sigchains, and the go-foks repository `github.com/foks-proj/go-foks` (the v0.1.9 oracle these crates are tested against; `tools/foks-v019-oracle/README.md`).

Secondary / marked for verification:
- Maniatis & Baker, "Secure History Preservation Through Timeline Entanglement", USENIX Security 2002 (authenticated append-only skip lists) [VERIFY].
- Dahlberg, Pulls, Peeters, "Efficient Sparse Merkle Trees", NordSec 2016 [VERIFY].
- Eijdenberg, Laurie, Cutter, "Verifiable Data Structures" (Google, 2015) and the Trillian project [VERIFY exact title/venue].
- Chase, Deshpande, Ghosh, Malvai, "SEEMless: Secure End-to-End Encrypted Messaging with less Trust", CCS 2019; and "Parakeet" (NDSS 2023) for modern key-transparency history trees [VERIFY].
- Ethereum Yellow Paper, Appendix D (Modified Merkle Patricia Trie) as a comparison [VERIFY appendix letter].
- FOKS project documentation site (foks.pub) [VERIFY URL].

---

## 12. File index (absolute paths)

- Tree construction and proofs: `/home/bnoland/projects/foks-rs/crates/foks-merkle-store/src/{lib.rs, node.rs, tree.rs, proof.rs, key.rs, history.rs, commit.rs, store.rs, memory.rs, error.rs}`; README at `crates/foks-merkle-store/README.md`; tests in `crates/foks-merkle-store/tests/{official_vectors.rs, history.rs, corruption.rs, properties.rs}`.
- Verifier: `/home/bnoland/projects/foks-rs/crates/foks-verify/src/{merkle.rs, proof.rs, host.rs, user.rs, team.rs, user_transition.rs, error.rs, lib.rs}` (tests in `lib.rs:164-1520`).
- Wire types: `/home/bnoland/projects/foks-rs/crates/foks-proto/src/host.rs` (HostchainLink, MerkleRoot, HistoricalMerkleRoots), `crates/foks-proto/src/identity/{merkle.rs, chain.rs, user.rs, generic.rs, team.rs}`, constants in `crates/foks-proto/src/lib.rs`.
- Server: `/home/bnoland/projects/foks-rs/crates/foks-server/src/net/session.rs` (commit minting ~280-360 and ~660-760; RPC handlers 919-1090; helpers 1149-1340), `crates/foks-server/src/services/{user.rs, generic.rs, team_admin.rs}`, `crates/foks-server/src/host/rotation.rs`, `crates/foks-server/src/identity/signup.rs`.
- Server DB: `/home/bnoland/projects/foks-rs/crates/foks-server-db/src/{merkle.rs, generic.rs, identity.rs, team.rs, host_rotation.rs}` and `schema/merkle.sql`.
- Client: `/home/bnoland/projects/foks-rs/crates/foks-client/src/{auth.rs, pinning.rs, change_marker.rs, team.rs, host.rs}`.
- Client DB: `/home/bnoland/projects/foks-rs/crates/foks-client-db/src/{lib.rs, schema.rs, merkle_checkpoint.rs, merkle_gc.rs, repositories/host.rs}`.
- Crypto primitives: `/home/bnoland/projects/foks-rs/crates/foks-crypto/src/lib.rs` (prefixed_hash 708, prefixed_hash_signable 719, tree_location_commitment 724, commitment 781, subchain_tree_location 588).
- Project docs: `/home/bnoland/projects/foks-rs/docs/GUIDE.html` (sections 4 and 5), `docs/INTRO.html`.
- Go oracle boundary tests: `/home/bnoland/projects/foks-rs/tools/foks-v019-oracle/{merkle_boundary_test.go, subchain_derivation_test.go}`.
