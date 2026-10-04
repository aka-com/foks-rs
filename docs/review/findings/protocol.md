# Protocol, crypto, RPC and verification

Area key `protocol`. 13 verified findings. Part of the [foks-rs review](../README.md).

## Current state

The protocol, crypto, RPC and verification crates are in good shape. Every crate sets `#![forbid(unsafe_code)]`. The Snowpack decoder enforces canonical form, a depth limit and a global value-count limit. Schema helpers (`array(value, n)`) check exact field counts before indexing. Merkle edges are fixed `[u8; 33]` arrays. Verification uses checked arithmetic and Ed25519 `verify_strict`. Behaviour is pinned to 351 Go v0.1.9 fixtures. The generated protocol IDs, status codes and routes have both an offline drift test (crates/foks-protocol-metadata/tests/pinned.rs) and a CI check. The few `expect`/`unreachable!` calls outside tests guard real invariants; I found no panic that untrusted input can reach.

The gaps are elsewhere:
- **Fuzzing** covers only the generic codec, the RPC envelope, server call framing, agent frames and the state archive. The ~114 foks-proto schema decoders, ~50 server-side argument decoders and all of foks-verify are not fuzzed, and the corpora ignore the 351 fixtures.
- **Secret hygiene** has two gaps that contradict invariants the book states: seed-chain plaintext goes through the non-zeroizing decoder, and the caller-supplied randomness structs (which hold ephemeral secrets) are plain Copy arrays.
- **hybrid.rs** repeats the hybrid KDF and typed-nonce secretbox code nine times, with inconsistent input validation.
- **Type IDs:** about 100 hand-written 64-bit domain-separation IDs are not checked against upstream or for collisions.
- **Status codes** appear as magic numbers outside foks-rpc, and server-supplied status text reaches error messages with no size or character limit.
- **Verification duplication:** user and team verification each have separate full and incremental paths with duplicated logic.
- **Chat:** two protocol-level issues block moving it past MVP. The fixed-shape capability response cannot be extended without breaking older clients, and the pinned Go Pegged message bodies (Reply/Edit/Reaction) cannot be opened, which also blocks sending after one arrives.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [Box randomness structs expose secret material as plain Copy arrays and leave key/nonce uniqueness to the caller](#protocol-box-randomness-secret-api) | security | medium | M |
| [Open the pinned Go Pegged bodies (Reply first) and stop blocking sends after an unsupported message](#protocol-chat-pegged-reply-bodies) | feature-refinement | medium | M |
| [Collapse duplicate crypto crate versions and add RustSec advisory checks to CI](#protocol-crypto-dependency-hygiene) | security | medium | S |
| [Fuzz the schema decoders, server argument decoders and verifiers, and seed the corpora with the Go fixtures](#protocol-fuzz-schema-and-verification-decoders) | testing | medium | M |
| [hybrid.rs repeats the hybrid KDF and typed-nonce secretbox nine times, with inconsistent input validation](#protocol-hybrid-seal-duplication) | code-quality | medium | M |
| [Seed-chain and KV plaintexts go through the non-zeroizing decode_prefix, contrary to the book's zeroization rule](#protocol-seed-chain-plaintext-nonzeroizing-decode) | security | medium | S |
| [Replace magic status-code literals with typed statuses, and limit server-supplied status text](#protocol-typed-status-codes-and-detail) | maintainability | medium | S |
| [Share one per-link step between the full and incremental user/team verifiers, and test that they agree](#protocol-verify-shared-chain-step) | maintainability | medium | L |
| [Make the chat capability response extensible before any extended-chat feature is advertised](#protocol-chat-capabilities-extensible-shape) | feature-refinement | low | S |
| [Document the foks-crypto root API, type its raw-seed signers, and use constant-time equality on secret types](#protocol-crypto-public-api-docs-and-ct-eq) | docs | low | S |
| [Split the 2853-line foks-crypto tests.rs and consolidate the Yubi test doubles and fixture loaders](#protocol-crypto-test-organization) | testing | low | M |
| [Replace the six-function parcel-opening ladder with a named-field expectation struct](#protocol-parcel-open-expectation-struct) | code-quality | low | M |
| [Hand-written 64-bit type IDs have no upstream drift check and no collision registry](#protocol-type-id-registry-and-drift) | tooling | low | M |

### protocol-box-randomness-secret-api

**Box randomness structs expose secret material as plain Copy arrays and leave key/nonce uniqueness to the caller**

- Type: security
- Priority: medium
- Effort: M
- Layers: protocol, client-lib
- Verification: confirmed

Each sealing function takes caller-built `PukBoxRandomness` / `YubiPukBoxRandomness` / `*SetRandomness` structs with public `[u8; 32]` fields, so deterministic Go fixtures can be produced. `kem_message` is the ML-KEM encapsulation seed: with the public encapsulation key it yields the KEM shared secret. `ephemeral_secret` is a DH private key. Neither is zeroized, and both are copied freely (`StaticSecret::from(set_randomness.ephemeral_secret)`). This contradicts the book's statement that every secret type is held in zeroizing storage. The API also requires 'one independent randomness pair per box' but never checks it. The same device can legitimately receive several generations in one call, so a caller that reuses one entry produces the same hybrid key and nonce for two different seeds. About 20 production call sites in foks-client build these structs by hand.

**Evidence**

- [`crates/foks-crypto/src/hybrid.rs:107`](../../../crates/foks-crypto/src/hybrid.rs#L107): InitialPukBoxRandomness/PukBoxRandomness/YubiPukBoxRandomness/YubiPukBoxSetRandomness/SoftwarePukBoxSetRandomness: public plain-array secret fields, no Drop/Zeroize.
- [`crates/foks-crypto/src/hybrid.rs:166`](../../../crates/foks-crypto/src/hybrid.rs#L166): Doc comment: 'callers must supply one independent randomness pair per box'; no distinctness check follows.
- [`crates/foks-crypto/src/hybrid.rs:437`](../../../crates/foks-crypto/src/hybrid.rs#L437): `StaticSecret::from(set_randomness.ephemeral_secret)` copies the secret; the original stays in the caller's struct.
- [`crates/foks-client/src/device.rs:253`](../../../crates/foks-client/src/device.rs#L253): A production caller builds a Vec<PukBoxRandomness> from random_bytes(); the same pattern recurs in team/named.rs:487, team.rs:2720, recovery.rs:147, bot_token.rs:258 and others.
- [`book/08-local-keystore.qmd:173`](../../../book/08-local-keystore.qmd#L173): Claims every secret type is wrapped in Zeroizing.
- [`crates/foks-crypto/Cargo.toml:19`](../../../crates/foks-crypto/Cargo.toml#L19): foks-crypto already depends on getrandom, so it can generate this randomness itself.

**Recommendation**

Make the fields private and implement `Drop` with `zeroize()`, matching the existing `PassphraseKeyring`, or wrap them in `Zeroizing<[u8; 32]>`. Add `PukBoxRandomness::generate() -> Result<Self>` (and the same for each struct), backed by `getrandom::fill`, as the production constructor. Gate `from_fixture(kem_message, nonce)` behind `#[cfg(any(test, feature = "fixtures"))]`, and enable that feature only from dev-dependencies and the oracle tests. In each seal function, reject an input set in which two entries share a `kem_message` or a (receiver, nonce) pair. Then replace the ~20 foks-client construction sites with `generate()`. None of this changes any wire bytes.

<details><summary>Verifier note</summary>

Verified at hybrid.rs:107-136. InitialPukBoxRandomness, PukBoxRandomness, YubiPukBoxRandomness, YubiPukBoxSetRandomness and SoftwarePukBoxSetRandomness expose public `[u8; N]` fields and have no Drop or Zeroize. kem_message is passed to `encapsulate_deterministic`, so together with the public encapsulation key it determines the KEM shared secret. ephemeral_secret is a DH private key, copied at line 439 by `StaticSecret::from(set_randomness.ephemeral_secret)` and at line 218 via P256SecretKey::from_slice. The 'one independent randomness pair per box' requirement (doc comment at line 168) is never checked. Production construction sites in foks-client number about 22: device.rs (255, 429, 777, 792, 1060, 1072, 1396, 1408), team.rs:2720, team/named.rs (446, 488, 492), recovery.rs (147, 445), bot_token.rs (258, 271, 292), kex.rs:342, account.rs:213, yubi_account.rs:147 and team/membership.rs:2145. passphrase.rs:363 in foks-crypto is another. A partial helper already exists: `random_box_randomness()` at team/membership.rs:2144. It could become the proposed `generate()`. The book's 'Every secret type' list is at 08-local-keystore.qmd:176 (cited as 173). The foks-crypto dependency on getrandom (Cargo.toml:19) is confirmed. Reusing a kem_message for the same receiver would reproduce the hybrid key, and with a reused nonce that means key/nonce reuse, so the distinctness check is sound. The recommendation does not change wire bytes. A cargo `fixtures` feature is needed, rather than cfg(test), because foks-server tests and foks-go-interop build fixtures across crates. The recommendation already provides for this. Not tracked in ISSUES.md.

</details>

### protocol-chat-pegged-reply-bodies

**Open the pinned Go Pegged bodies (Reply first) and stop blocking sends after an unsupported message**

- Type: feature-refinement
- Priority: medium
- Effort: M
- Layers: protocol, client-lib, server, desktop-ui
- Verification: adjusted

The pinned v0.1.9 schema defines Pegged bodies `{kind: Edit|Reaction|Reply, body, reply_to}`, and foks-proto encodes and decodes them. foks-crypto opens only Basic bodies, so the Rust client shows every pegged message as Unsupported without decrypting it, and its metadata stays unauthenticated. prepare_send then refuses to send while the channel's latest message is Unsupported. No conforming client produces such a message: the Go v0.1.9 client sends only Basic and fails history loads on anything else, and the Rust server rejects non-Basic sends. A nonconforming member client posting to a Go server can still block Rust users from sending in that channel. Decrypting pegged kinds on read, and choosing an authenticated predecessor, fixes that without a wire change. Sending Replies is not interop-free: it needs a server change and breaks Go v0.1.9 readers, so it belongs with the deferred extended-chat negotiation.

**Evidence**

- [`crates/foks-proto/src/realtime.rs:205`](../../../crates/foks-proto/src/realtime.rs#L205): RtMessageType includes Edit=2, Reaction=4, Reply=6 (Go: Reactji).
- [`crates/foks-proto/src/realtime.rs:341`](../../../crates/foks-proto/src/realtime.rs#L341): from_value decodes Edit/Reaction/Reply as Pegged { kind, body, reply_to }.
- [`crates/foks-crypto/src/realtime.rs:120`](../../../crates/foks-crypto/src/realtime.rs#L120): open_basic_message rejects non-Basic bodies; check_basic (line 141) requires metadata.kind == Basic. The nonce (line 56-65) hashes the full noncer.
- [`crates/foks-client/src/realtime/history.rs:298`](../../../crates/foks-client/src/realtime/history.rs#L298): Non-Basic kinds return ChatContent::Unsupported without decryption.
- [`crates/foks-client/src/realtime/operations/prepare.rs:221`](../../../crates/foks-client/src/realtime/operations/prepare.rs#L221): Send fails with ChatUnsupported('latest message has unsupported content').
- [`crates/foks-server-db/src/realtime/messages.rs:16`](../../../crates/foks-server-db/src/realtime/messages.rs#L16): Rust server rejects any send whose metadata.kind != Basic, so Phase 2 needs a server change. Lines 74-85 allow previous_sequence to lag the channel tail.
- [`tools/foks-v019-oracle/go.mod:6`](../../../tools/foks-v019-oracle/go.mod#L6): Pinned go-foks v0.1.9. librt/minder.go:966 sends only Basic, and minder.go:1138 rejects non-Basic bodies, which fails the whole history page at 1881-1884.

**Recommendation**

Phase 1, read-side only with no wire or server change: add `RealtimeKeys::open_message(noncer, ciphertext)` that uses the existing noncer-derived nonce and requires `noncer.metadata.kind` to match the decoded body kind. In history.rs, decrypt Edit/Reaction/Reply so their metadata is authenticated, render Reply with its parent reference, and show Edit/Reaction as authenticated but not-yet-rendered. Separately, remove the send block in prepare.rs. When the latest message cannot be authenticated, take the predecessor from the latest authenticated message; the server permits a lagging previous_sequence (messages.rs:74-85). This keeps the channel writable for every kind, including Attachment/System/Join/Leave. Add Go-oracle fixtures for a Reply body. Phase 2, sending Reply, is not interop-only. It requires lifting the Rust server's Basic-only check and makes the channel unreadable for Go v0.1.9 clients, whose history load fails on any non-Basic message. Treat it as part of the deferred extended-chat capability and namespace work, enabled only when negotiation guarantees every reader supports it. Mock the composer as capability-gated, not as default behaviour.

**Already tracked:** ISSUES.md 'Existing disclosed limitations' (edits, reactions, threads remain disabled) and 'Deferred extended-chat prerequisite'. This finding adds a smaller, interop-only first step using the already-pinned Pegged wire form, plus the send-blocking defect, which is not recorded.

**Mockup:** [Extended chat preview](../mockups/chat-capability-gated-extensions.html)

<details><summary>Verifier note</summary>

Several code facts hold. The pinned Go schema defines `case Reply, Reactji, Edit @2 : RTMsgPlaintextPegged` (go-foks v0.1.9 proto-src/lib/realtime.snowp:108). foks-proto decodes Pegged (realtime.rs:341). open_basic_message and check_basic (foks-crypto realtime.rs:120/141) accept only Basic. history.rs:298 returns Unsupported without decrypting. prepare.rs:221-228 blocks sends when the latest message is Unsupported, and ISSUES.md does not record that. The nonce is a hash of the full noncer (realtime.rs:56-65), so read-side decryption of pegged kinds would authenticate their metadata; Phase 1 is sound and needs no wire change. Corrections: (1) The pinned Go v0.1.9 client never sends a pegged body. It always uses RTMsgType_Basic (librt/minder.go:966) and rejects non-Basic on read (minder.go:1138), which fails the whole history page (minder.go:1881-1884). The Rust server rejects non-Basic sends (foks-server-db realtime/messages.rs:16). So 'a Reply from a peer using the pinned wire' cannot come from any conforming client. The send block is reachable only when a nonconforming member client posts to a Go v0.1.9 server, which stores any type. That is a real availability gap but not high priority. (2) Phase 2 is not free of protocol changes. Sending Reply requires lifting the Rust server's Basic-only rule, and any pegged message makes the channel unreadable for Go v0.1.9 clients, which Rust-server interop tests cover (go_client_rust_live_test.go). It therefore needs negotiation that excludes Go v0.1.9 readers, under the extended-chat capability and namespace work ISSUES.md defers.

</details>

### protocol-crypto-dependency-hygiene

**Collapse duplicate crypto crate versions and add RustSec advisory checks to CI**

- Type: security
- Priority: medium
- Effort: S
- Layers: protocol, ci, tooling
- Verification: adjusted

The lockfile carries two curve25519-dalek versions, two each of digest, sha2, sha3 and rand_core, and three of getrandom. Several crates pin pre-digest-0.11 crypto crates locally outside `[workspace.dependencies]`, where sha2 is 0.11: foks-crypto pins hmac 0.12, p256 0.13 and sha2 0.10, and foks-oidc, foks-snowpack, foks-yubi and foks-server-testkit pin sha2 0.10. The digest-0.10 requirement is a Rust API choice, not a wire constraint. Third-party dependents (openidconnect =4.0.1, argon2 0.5, ml-kem 0.3.2) also keep the older generations in the graph. No workflow runs cargo-audit or cargo-deny, so advisories against these crypto crates would go unnoticed.

**Evidence**

- [`Cargo.lock:1098`](../../../Cargo.lock#L1098): curve25519-dalek 4.1.3 and 5.0.0 (line 1114).
- [`Cargo.lock:1309`](../../../Cargo.lock#L1309): digest 0.10.7/0.11.3; sha2 0.10.9/0.11.0 at 5578/5589; sha3 0.11/0.12 at 5600/5610; rand_core 0.6.4/0.10.1 at 4778/4787; getrandom 0.2/0.3/0.4 at 2522/2535/2547.
- [`crates/foks-crypto/Cargo.toml:17`](../../../crates/foks-crypto/Cargo.toml#L17): Comment 'FOKS v0.1.9 uses the digest-0.10 RustCrypto API' with local `hmac = "0.12"`, `p256 = "0.13"`, `sha2 = "0.10"` (lines 18-22).
- [`Cargo.toml:99`](../../../Cargo.toml#L99): Workspace pins sha2 = "0.11" and sha3 = "0.12" (line 100); argon2 = "0.5" at line 76 pulls blake2 on digest 0.10.
- [`crates/foks-oidc/Cargo.toml:17`](../../../crates/foks-oidc/Cargo.toml#L17): openidconnect = "=4.0.1" pulls ed25519-dalek 2, curve25519-dalek 4, p256/p384 0.13, hmac 0.12, rsa 0.9 and sha2 0.10; foks-oidc also pins sha2 0.10 at line 13.
- [`crates/foks-yubi/Cargo.toml:19`](../../../crates/foks-yubi/Cargo.toml#L19): Local p256 0.13 and sha2 0.10 (line 21); foks-snowpack/Cargo.toml:16 and foks-server-testkit/Cargo.toml:16 also pin sha2 0.10.
- `github/workflows/foks-standalone.yml:1`: No cargo-audit or cargo-deny step in any of the five workflows; there is no deny.toml.

**Recommendation**

Add `deny.toml` with `[advisories]` denying vulnerabilities and unmaintained crates, and `[bans] multiple-versions = "warn"`. Give the duplicates an explicit `skip` list naming the dependent that forces each one: openidconnect 4.0.1 for ed25519-dalek 2 / curve25519-dalek 4 / p256 0.13 / rsa 0.9; argon2 0.5 for blake2 on digest 0.10; ml-kem 0.3.2 for sha3 0.11. Run `cargo deny check advisories bans licenses` in foks-standalone.yml on push and nightly. Move the scattered local pins into `[workspace.dependencies]` (e.g. a renamed `sha2-digest010`) in foks-crypto, foks-oidc, foks-snowpack, foks-yubi and foks-server-testkit, each with a comment naming the crate that forces it. Plan the digest-0.11 migration across openidconnect, argon2, ml-kem, ed25519-dalek, hmac and p256 together; foks-crypto alone will not drop curve25519-dalek 4 or digest 0.10. The Go fixture tests (signatures, HMAC commitments, P-256 ECDH) are the regression gate.

<details><summary>Verifier note</summary>

Core holds. Cargo.lock carries curve25519-dalek 4.1.3/5.0.0, digest 0.10.7/0.11.3, sha2 0.10.9/0.11.0, sha3 0.11/0.12, rand_core 0.6.4/0.10.1 and getrandom 0.2/0.3/0.4. foks-crypto pins hmac 0.12, p256 0.13 and sha2 0.10 locally under a 'digest-0.10 API' comment, while the workspace pins sha2 0.11 and sha3 0.12. There is no deny.toml, and none of the five workflows runs cargo-audit or cargo-deny. Corrections: (1) The workspace pins are at Cargo.toml:99-100, not 96. (2) foks-crypto is not the only local pin: sha2 0.10 is also pinned in foks-oidc, foks-snowpack, foks-yubi (which also pins p256 0.13) and foks-server-testkit. (3) The duplicates are not removable through foks-crypto alone. `cargo tree -i` shows openidconnect =4.0.1 pulls ed25519-dalek 2, curve25519-dalek 4, p256/p384 0.13, hmac 0.12, rsa 0.9 and sha2 0.10. argon2 0.5 pulls blake2 on digest 0.10, and ml-kem 0.3.2 pulls sha3 0.11. Upgrading ed25519-dalek, hmac and p256 alone would not drop curve25519-dalek 4 or digest 0.10.

</details>

### protocol-fuzz-schema-and-verification-decoders

**Fuzz the schema decoders, server argument decoders and verifiers, and seed the corpora with the Go fixtures**

- Type: testing
- Priority: medium
- Effort: M
- Layers: protocol, ci, tooling
- Verification: adjusted

The five fuzz targets cover only the generic layers: the Snowpack codec, the probe-response envelope, server call framing, agent frames and the state archive. Three groups of decoders that parse network input have no coverage-guided target: the 114 typed foks-proto `decode` functions, the 47 server-side `foks_rpc::arguments::decode_*` functions (14 modules), and the foks-verify state machines. foks-verify does have a deterministic byte-flip sweep over the official user chain, and foks-rpc has quickcheck envelope properties, but no fuzz target reaches these code paths. The snowpack and rpc corpora contain one synthetic seed each, while 137 canonical Go .snowp fixtures (among 351 fixture files) are checked in. When verify and crypto targets are added, the workflow path filter must also be extended to foks-verify and foks-crypto.

**Evidence**

- [`crates/foks-rpc/src/arguments/mod.rs:3`](../../../crates/foks-rpc/src/arguments/mod.rs#L3): 14 submodules re-exported; 47 pub decode_* functions in total, none fuzzed.
- [`crates/foks-verify/src/lib.rs:1097`](../../../crates/foks-verify/src/lib.rs#L1097): Existing deterministic single-byte-flip sweep over the official user chain. This is not coverage-guided fuzzing, and the team chain, host probe and Merkle inputs are not swept the same way.
- [`crates/foks-snowpack/tests/fixtures/foks-v0.1.9`](../../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9): 351 fixture files, 137 of them .snowp; the corpora for snowpack and rpc each hold one synthetic seed.
- `github/workflows/foks-fuzz.yml:4`: The path filter omits foks-verify and foks-crypto. No current target depends on them (foks-rpc has foks-crypto only as a dev-dependency), so they must be added together with the new targets.

**Recommendation**

(1) Add a `proto_schema` target. Take the first input byte as a selector over a table of `fn(&[u8])` closures that call every exact-retaining foks-proto decoder (UserLink, UserChain, TeamChain, SharedKeyBoxSet, PukParcel, HybridBox, MerkleRoot, MerkleLookupResponse, KvNode, KvRoot, KvListResponse, KvPathVersionVector, the Rt* types through RealtimeWire::decode, TeamRsvp, and so on). Where a type retains its exact bytes, assert `decode(x).encoded() == x`. (2) Add a `server_arguments` target that drives every `foks_rpc::arguments::decode_*` and `RealtimeRequest` decoder the same way. (3) Add a `verify` target. Call `verify_public_host("foks.app", input)`, then call `verify_user_chain` and `verify_team_chain` with the fixture probe's authenticated roots and the input as the chain bytes. (4) Extend fuzz/examples/seed_corpus.rs to copy crates/foks-snowpack/tests/fixtures/foks-v0.1.9/**/*.snowp into the snowpack, proto_schema and verify corpora (with the selector byte prepended), and to wrap them in `encode_success_response_at` for the rpc corpus. (5) Factor the response prelude into `fn response_result_slot(content, expected_sequence) -> Result<&[u8]>`, keep the three public functions as thin wrappers, and fuzz all of them. (6) Add crates/foks-verify/** and crates/foks-crypto/** to the workflow path filter and the new targets to its matrix.

<details><summary>Verifier note</summary>

Core claim holds. The five targets (snowpack, rpc, server_rpc, agent_frames, state_archive) cover only the generic codec, framing, agent frames and the archive. fuzz/Cargo.toml does not depend on foks-verify or foks-crypto, and none of the 114 `pub fn decode(` functions in foks-proto or the 47 `decode_*` functions in foks-rpc/src/arguments has a fuzz target. decode_void_response (line 106) and decode_bare_response (line 169) do repeat the decode_response prelude, and rpc.rs exercises only decode_probe_response, read_frame and read_probe_response. The corpora hold one seed each for snowpack ('array') and rpc ('empty-frame'). Neither ISSUES.md nor the book tracks this. Corrections: (a) arguments/mod.rs declares 14 submodules, not 15, and 47 decode_* functions. (b) Of the 351 fixture files, 137 are .snowp. The rest are .frame/.bin/.json and similar. (c) foks-verify is not entirely unexercised by mutation. lib.rs:1097 `user_response_mutations_cannot_change_verified_state` flips every byte of the official user chain and asserts that verified state cannot change. Other tests in lib.rs tamper with specific fields, and foks-rpc/tests/properties.rs has quickcheck 'never panic' properties for envelopes. What is missing is coverage-guided fuzzing. (d) The workflow path filter is consistent with today's targets: foks-rpc depends on foks-crypto only as a dev-dependency, and nothing in the fuzz workspace depends on foks-verify. The filter change only matters once the new targets exist. (e) Priority should be medium, not high. Unwinding panics are the default (no panic=abort profile), and the positional decoders check arity before indexing, so the realistic failure is a rejected or dropped connection or a logic mismatch, not memory unsafety.

</details>

### protocol-hybrid-seal-duplication

**hybrid.rs repeats the hybrid KDF and typed-nonce secretbox nine times, with inconsistent input validation**

- Type: code-quality
- Priority: medium
- Effort: M
- Layers: protocol
- Verification: adjusted

Nine production functions in hybrid.rs repeat, verbatim, the sequence: SHA3-256 over the type ID and derivation payload, build a 24-byte nonce from an 8-byte type prefix plus a 16-byte nonce, then XSalsa20-Poly1305 seal or open. A tenth copy exists as a test helper, while `primitives::seal_typed_secretbox`/`open_typed_secretbox` already implement the nonce construction. Every copy maps an encryption failure to `Error::Decryption`. Input validation differs between paths: the mixed and Yubi sealers reject `generation == 0`, `Role::NONE` and non-host `host` IDs, but `seal_shared_key_boxes_with_sender`, which backs the software, backup and shared-key sealers, checks neither. `commitment` duplicates `typed_hmac`.

**Evidence**

- [`crates/foks-crypto/src/hybrid.rs:273`](../../../crates/foks-crypto/src/hybrid.rs#L273): KDF and secretbox copy #1 (seal_software_puk_boxes_mixed); further copies at ~366, ~494, ~670, ~737, ~790, ~840, ~982 and ~1476 (open_hybrid_box).
- [`crates/foks-crypto/src/hybrid.rs:1589`](../../../crates/foks-crypto/src/hybrid.rs#L1589): A #[cfg(test)] derive_hybrid_key re-implements the KDF a tenth time.
- [`crates/foks-crypto/src/primitives.rs:75`](../../../crates/foks-crypto/src/primitives.rs#L75): open_typed_secretbox / seal_typed_secretbox (line 91) already build the typed nonce; the hybrid code does not use them.
- [`crates/foks-crypto/src/hybrid.rs:619`](../../../crates/foks-crypto/src/hybrid.rs#L619): seal_shared_key_boxes_with_sender does not check generation/role or require_type(ENTITY_HOST), unlike lines 213 and 430-455.
- [`crates/foks-crypto/src/hybrid.rs:284`](../../../crates/foks-crypto/src/hybrid.rs#L284): `.encrypt(...).map_err(|_| Error::Decryption)`: an encryption failure is reported as a decryption failure.
- [`crates/foks-crypto/src/primitives.rs:152`](../../../crates/foks-crypto/src/primitives.rs#L152): commitment() duplicates typed_hmac() (line 14).

**Recommendation**

Introduce three private helpers. (1) `fn hybrid_symmetric_key(kem_shared, dh_shared, receiver_hepk, sender_dh) -> Result<Zeroizing<[u8;32]>>`. (2) `fn seal_hybrid(receiver_hepk, agreement: DhAgreement { sender_dh, dh_shared, dh_type }, randomness, payload_type_id, cleartext) -> Result<HybridBox>`, built on `seal_typed_secretbox(.., padded = false)`. (3) `fn validate_box_input(host, generation, role, receiver)`. Have each public sealer choose only the DH agreement (software X25519, Yubi P-256 or temporary key) and the target, and call validate_box_input from seal_shared_key_boxes_with_sender as well. Use `open_typed_secretbox` in `open_hybrid_box`. Rename the existing `Error::KvEncryption` to a general `Error::Encryption`, or add a hybrid-specific mapping, so that seal failures are no longer reported as `Decryption` and hybrid failures are not reported as KV failures. Have the test helper call `hybrid_symmetric_key`. Make `commitment` delegate to `typed_hmac`. The existing official-fixture seal/open tests in tests.rs are the regression gate, so no new vectors are needed.

<details><summary>Verifier note</summary>

Confirmed. The SHA3-256(HYBRID_SECRET_KEY_SHA3_PAYLOAD_TYPE_ID || derivation) → typed 24-byte nonce → XSalsa20-Poly1305 sequence appears verbatim nine times in production code: lines 274, 367, 495, 671, 738, 791, 841, 983 and 1477. A tenth copy is the #[cfg(test)] derive_hybrid_key at 1589/1622. Each production seal maps an encrypt failure to Error::Decryption (e.g. line 290). primitives.rs already has open_typed_secretbox (75) and seal_typed_secretbox (91). commitment() (primitives.rs:152) is byte-for-byte typed_hmac() (line 14). seal_shared_key_boxes_with_sender does validate receiver entity type, receiver role/generation consistency and the optional receiver_host type. It does not check input.generation == 0, input.role == Role::NONE, or that the `host` argument is ENTITY_HOST, whereas seal_software_puk_boxes_mixed (212, 229) and seal_yubi_puk_boxes do. One recommendation detail needs correcting: an encryption error variant already exists. seal_typed_secretbox maps failures to `Error::KvEncryption`, so building seal_hybrid on it unchanged would report hybrid-box failures as KV errors. Generalize the existing variant rather than adding a parallel `Error::Encryption`. seal_typed_secretbox also takes a `padded` flag; hybrid boxes must pass false. The Go v0.1.9 fixture tests in tests.rs, e.g. official_go_mixed_curve_box_sets_open_with_the_matching_sender_path and backup_enrollment_and_recovery_links_match_go_v019, are adequate regression gates.

</details>

### protocol-seed-chain-plaintext-nonzeroizing-decode

**Seed-chain and KV plaintexts go through the non-zeroizing decode_prefix, contrary to the book's zeroization rule**

- Type: security
- Priority: medium
- Effort: S
- Layers: protocol
- Verification: adjusted

The book says seed plaintexts never pass through the generic decoder: the trailing 32 seed bytes are copied out and zeroed first. `open_shared_key_seed_chain` and `open_team_shared_key_seed_chain` (hybrid.rs:1221, :1263) break this. They call `foks_snowpack::decode_prefix(&plaintext)` on the decrypted older-generation PUK/PTK plaintext only to learn how many bytes the canonical value consumes. That builds a `Value::Binary` copy of the older seed in an ordinary heap Vec, which is dropped without zeroization. The redacting `SharedKeySeed::decode` runs only afterwards. `decode_sensitive` cannot be used because it rejects trailing padding. The KV openers' use of decode_prefix on user content is a separate matter: that data is returned to callers in ordinary storage anyway, so it does not break the rule for secrets.

**Evidence**

- [`crates/foks-crypto/src/hybrid.rs:1221`](../../../crates/foks-crypto/src/hybrid.rs#L1221): `let (_, consumed) = decode_prefix(&plaintext)?;` builds a non-zeroized Value::Binary copy of the older PUK/PTK seed before the redacting SharedKeySeed::decode at line 1223.
- [`crates/foks-crypto/src/hybrid.rs:1263`](../../../crates/foks-crypto/src/hybrid.rs#L1263): Same pattern in open_team_shared_key_seed_chain.
- [`crates/foks-snowpack/src/decode.rs:21`](../../../crates/foks-snowpack/src/decode.rs#L21): decode_prefix builds an ordinary Value tree; decode_sensitive (line 30) zeroizes but requires full consumption.
- [`crates/foks-snowpack/src/lib.rs:68`](../../../crates/foks-snowpack/src/lib.rs#L68): `impl zeroize::Zeroize for Value` is opt-in; dropping a Value does not erase it.
- [`book/05-generations.qmd:120`](../../../book/05-generations.qmd#L120): States that foks-crypto copies the seed out and zeroes it before running the decoder.
- [`crates/foks-crypto/src/kv.rs:119`](../../../crates/foks-crypto/src/kv.rs#L119): KV openers (also 173, 386, 434) use decode_prefix on user content or a size field, not key material, and return that data in ordinary storage. This is not a violation of the documented rule.

**Recommendation**

Add `foks_snowpack::canonical_prefix_len(input) -> Result<usize, Error>`, which runs the Decoder in a count-only mode that allocates no Values. Alternatively, add `decode_prefix_sensitive(input) -> Result<(Zeroizing<Value>, usize)>` that reuses the existing `sensitive` flag. Use it at hybrid.rs:1221 and :1263. Merge the two near-identical seed-chain loops into one helper that takes an `Option<&EntityId>` receiver check. Add a regression test asserting that the seed-chain path never calls decode_prefix on unredacted plaintext, for example with a `#[cfg(test)]` counter or by routing through the new function only. Leave the KV openers unchanged unless the KV plaintext return types are made zeroizing too. If a lint is wanted, scope it with `#[deny(clippy::disallowed_methods)]` on the seed-handling module rather than the whole crate, because kv.rs uses decode_prefix legitimately on non-secret data.

<details><summary>Verifier note</summary>

The seed-chain claim is confirmed. hybrid.rs:1221 and :1263 call `decode_prefix(&plaintext)` on the decrypted SharedKeySeed plaintext. decode_prefix (decode.rs:21) builds an ordinary Value tree, so the older seed is copied into a Value::Binary Vec. That Vec is dropped without erasure, because `impl Zeroize for Value` (lib.rs:68) is opt-in. Only after this does the redacting SharedKeySeed::decode (key_material.rs:599, via decode_with_redacted_trailing_seed) run. This contradicts book/05-generations.qmd ('only then runs the decoder on the redacted bytes') and the rule in 08-local-keystore.qmd. decode_sensitive cannot be used because it rejects trailing padding. Neither ISSUES.md nor the book records this. The KV part is overstated. Those openers decrypt user file content and directory names, not key material, and they hand that content back in ordinary storage: open_kv_chunk moves the Binary Vec out and returns it, KvSmallFilePlaintext::decode_value consumes the Value, and open_kv_dirent_name re-encodes the value with encode(). A zeroizing prefix decoder there would protect nothing unless the KV return types also became zeroizing. kv.rs:173 decodes the Rust large-file size extension, which is not secret. The book's rule concerns secrets, so the KV sites do not violate it. A crate-wide disallowed-methods lint would also flag those legitimate non-secret uses.

</details>

### protocol-typed-status-codes-and-detail

**Replace magic status-code literals with typed statuses, and limit server-supplied status text**

- Type: maintainability
- Priority: medium
- Effort: S
- Layers: protocol, client-lib, agent
- Verification: adjusted

foks-rpc generates `STATUS_*` constants from the pinned upstream source, but status.rs itself (8012, twice) and about 28 non-test client, app and agent sites match numeric literals. The literal sites use `RemoteStatus { code: N }` patterns, `match code { .. }` arms and `matches!(*code, ..)` guards. foks-rpc/response.rs maps server errors to about 35 more literals. The generated-constant drift check covers none of these, and at least one used code (1058) has no constant. check_status turns an arbitrary server payload into StatusDetail with no character filtering. The agent truncates IPC error text to 4096/1024 bytes but passes control and bidi characters through. Two crates repeat the same OAuth2 free-text matching to drive SSO state.

**Evidence**

- [`crates/foks-rpc/src/status.rs:79`](../../../crates/foks-rpc/src/status.rs#L79): `if code == 8012` (and again at line 173 in the positional path) instead of STATUS_KV_STALE_CACHE_ERROR.
- [`crates/foks-rpc/src/status.rs:96`](../../../crates/foks-rpc/src/status.rs#L96): Server payload becomes StatusDetail via String::from_utf8 or `format!("{other:?}")` with no length or character limit; same at lines 190-191.
- [`crates/foks-rpc/src/lib.rs:138`](../../../crates/foks-rpc/src/lib.rs#L138): describe_status interpolates the server detail into the Display text.
- [`crates/foks-rpc/src/response.rs:71`](../../../crates/foks-rpc/src/response.rs#L71): Server-side error-to-status mapping uses ~35 numeric literals (1067, 1069, 1009, 12001..12006, 8012, ...) inside foks-rpc itself.
- [`crates/foks-client/src/sso.rs:314`](../../../crates/foks-client/src/sso.rs#L314): Literal 1009/1067 and `d == "authorization denied" || d.contains("access_denied")`.
- [`crates/foks-client-app/src/sso.rs:279`](../../../crates/foks-client-app/src/sso.rs#L279): Duplicated 1067/1069 literals and text matching, plus `detail == Some("provider unavailable")`; also line 71.
- [`crates/foks-client/src/web_admin.rs:110`](../../../crates/foks-client/src/web_admin.rs#L110): `match code { 1020, 1018, 1058 | 1062, 1069 }`; 1058 has no generated constant.
- [`crates/foks-client/src/realtime/operations/attempt.rs:133`](../../../crates/foks-client/src/realtime/operations/attempt.rs#L133): `matches!(*code, 1013 | 1030 | 12002 | 12003 | 12005 | 12006)`, which a `RemoteStatus { code: <digit>` grep would miss.
- [`crates/foks-client/src/team/invitations.rs:282`](../../../crates/foks-client/src/team/invitations.rs#L282): `code: 1013 | 7004 | 7006 | 7010`; others at account_conveniences.rs:194, kv/sync.rs:1389/1419, foks-agent/src/main.rs:2016, foks-agent/src/chat.rs:859.
- [`crates/foks-agent-proto/src/message.rs:2451`](../../../crates/foks-agent-proto/src/message.rs#L2451): bounded_field already truncates IPC error messages to 4096 bytes. It does not filter control or bidi characters.
- [`crates/foks-client-app/src/registry.rs:1742`](../../../crates/foks-client-app/src/registry.rs#L1742): sanitize_server_message already bounds and de-controls untrusted server notice text and can be reused.

**Recommendation**

Generate a `#[non_exhaustive] pub enum Status` (with `Other(u64)`) from the same pinned source as status_codes.rs. Extend the generator for every code used today, including 1058. Add `foks_rpc::Error::status(&self) -> Option<Status>`. Migrate all literal sites, including status.rs:79/173, response.rs, `match code {..}` blocks and `matches!(*code, ..)` guards. Do not rely on a `RemoteStatus { code: <digit>` grep. Make the repository test reject any 3-5 digit integer literal compared or matched against a status code outside generated/ and tests, or route every comparison through `Status` so literals cannot type-check. In check_status, reuse the registry.rs sanitize_server_message approach: lossy UTF-8, replace control and Unicode bidi/format characters, bound to 256 bytes. Add `StatusDetail::oauth2_outcome()` so both SSO crates share one classification of the 'authorization denied', 'access_denied' and 'provider unavailable' strings. Render the detail as quoted server-provided text.

<details><summary>Verifier note</summary>

Core holds. status.rs:79 and 173 use `code == 8012` though STATUS_KV_STALE_CACHE_ERROR exists. About 28 non-test client, app and agent sites match numeric literals. foks-rpc/src/response.rs:71-105 maps server errors to about 35 more literals. foks-client/src/sso.rs:314-322 and foks-client-app/src/sso.rs:279-292 duplicate the 1067 'authorization denied'/'access_denied' text match, and the app copy also matches 'provider unavailable'. The detail decode is at status.rs:96-97 and 190-191, not line 104, and it has no bound or character filter at that layer. Corrections: (1) The proposed guard test only catches `RemoteStatus { code: <digit>`. It would miss `match code { 1020 => .. 1058 | 1062 => .. }` (web_admin.rs:110-114), `matches!(*code, 1013 | 1030 | 12002 ..)` (attempt.rs:133), `code == 8016` (foks-client lib.rs:712,740) and status.rs's own `code == 8012`. (2) Code 1058, used at web_admin.rs:113, has no generated constant (status_codes.rs is a 58-line curated subset), so the generator must be extended. (3) The agent already bounds IPC error text: Response::error truncates the message to 4096 bytes (foks-agent-proto message.rs:2451) and remote_status_response truncates the reason to 1024 (main.rs:6145). Only control/bidi filtering and non-agent paths remain open. A precedent sanitizer exists at foks-client-app/src/registry.rs:1742. ISSUES.md does not track any of this.

</details>

### protocol-verify-shared-chain-step

**Share one per-link step between the full and incremental user/team verifiers, and test that they agree**

- Type: maintainability
- Priority: medium
- Effort: L
- Layers: protocol
- Verification: adjusted

foks-verify has four parallel chain replays (user full and incremental, team full and incremental) and four parallel disclosure verifiers. Each replay repeats the same per-link checks: seqno and previous-hash continuity, uid/team and host binding, authenticated root epoch, the next-location commitment, Merkle key derivation and inclusion proof, and the final absence proof. The location-commitment check alone appears six times. The copies have already diverged in form; for example, the Merkle path slice bound is enforced differently in the full and incremental user paths. For security-critical replay, a rule added to one path and missed in its twin is the main risk. Tests cover no-op and overlap increments, but do not check that every way of splitting a chain gives the same result.

**Evidence**

- [`crates/foks-verify/src/user.rs:629`](../../../crates/foks-verify/src/user.rs#L629): verify_user_chain_at_root (~200 lines).
- [`crates/foks-verify/src/user.rs:869`](../../../crates/foks-verify/src/user.rs#L869): verify_user_chain_increment_at_root repeats the loop body at 922-960.
- [`crates/foks-verify/src/user.rs:663`](../../../crates/foks-verify/src/user.rs#L663): The full path filters `end <= paths.len()`. The incremental path (904-912) uses only get(). The behaviour is the same; the code has diverged in form.
- [`crates/foks-verify/src/team.rs:489`](../../../crates/foks-verify/src/team.rs#L489): Team full/increment (659) repeat the step; location commitment at team.rs 190/259/558/745, user.rs 697/945, user_transition.rs:43.
- [`crates/foks-verify/src/user.rs:776`](../../../crates/foks-verify/src/user.rs#L776): Full verification requires an absence proof for seqno n+1 under the response root, so a prefix of a single-root fixture cannot be verified on its own.
- [`crates/foks-verify/src/lib.rs:454`](../../../crates/foks-verify/src/lib.rs#L454): Increment tests cover only no-op and overlap (also line 607), built by rewriting a single-root fixture (incremental_noop_response at line 96).

**Recommendation**

Extract a shared per-link step, `verify_link_inclusion(link_change, step, authenticated_roots) -> Result<[u8;32]>`, parameterised by a small ChainKind trait for the user vs team Merkle key and change decoding. Express full verification as an increment from an empty genesis state, so the eldest-link and first-team-link handling are the only differences. Do the same for the disclosure verifiers. Before refactoring, add the differential test. Use foks-server-testkit, or a Go-oracle fixture generator, to build user and team chains one link at a time and capture a full load response at each epoch. Then assert for every k that `verify_*_increment(verify_*(resp_k), suffix_n)` equals `verify_*(resp_n)` field by field. Slicing the existing single-root fixtures cannot do this, because a prefix has no valid absence proof under the later root.

<details><summary>Verifier note</summary>

Core holds. user.rs:629 (full) and 869 (increment), and team.rs:489 and 659, repeat the same per-link checks: seqno and previous continuity, uid/team and host binding, authenticated root epoch, location commitment, Merkle key and inclusion, and the final absence proof. The location-commitment check appears at user.rs:697/945 and team.rs:190/259/558/745, plus user_transition.rs:43. Disclosure verifiers are duplicated at user.rs:1163/1324 and team.rs:1386/1493. The `.filter(|end| *end <= paths.len())` difference is form-only, because `.get()` already bounds it, as the summary says. lib.rs has only the no-op and overlap increment tests (454, 607). ISSUES.md does not track this. Correction: the proposed differential test is not feasible as written. The existing Go v0.1.9 fixtures are single snapshots at one Merkle root, and full verification of chain[..k] needs an absence proof for seqno k+1 under the response root (user.rs:776-786). At that root, link k+1 is present, so a prefix cannot be verified by slicing one fixture. The test needs responses captured at successive roots.

</details>

### protocol-chat-capabilities-extensible-shape

**Make the chat capability response extensible before any extended-chat feature is advertised**

- Type: feature-refinement
- Priority: low
- Effort: S
- Layers: protocol, server, client-lib
- Verification: adjusted

`RtChatCapabilities` is a fixed version-1 array of version, host and six booleans, and an unknown shape or version is rejected. Any later chat feature would need a new response version, which a strict older client would treat as an error instead of degrading to basic chat. Discovery is not yet called by the agent, client-app or desktop, and the server always answers basic_only. The shape can therefore be made extensible now at no compatibility cost, before any client depends on it. ISSUES.md tracks only the method namespace. The deferred chat_v2 crypto and the `RtChatContext` types are also exported as public API with no caller outside their modules.

**Evidence**

- [`crates/foks-proto/src/realtime_extension.rs:23`](../../../crates/foks-proto/src/realtime_extension.rs#L23): Version, host and six fixed booleans; the doc says 'Unknown shapes/versions fail closed'.
- [`crates/foks-proto/src/realtime_extension.rs:79`](../../../crates/foks-proto/src/realtime_extension.rs#L79): from_value requires exactly 8 fields and version 1.
- [`crates/foks-client/src/realtime/capabilities.rs:17`](../../../crates/foks-client/src/realtime/capabilities.rs#L17): basic_only only for MethodNotFound/NotImplemented. ChatSession::capabilities has no caller in foks-agent, foks-client-app or the desktop.
- [`crates/foks-server/src/services/realtime.rs:233`](../../../crates/foks-server/src/services/realtime.rs#L233): The server always answers basic_only.
- [`crates/foks-crypto/src/lib.rs:24`](../../../crates/foks-crypto/src/lib.rs#L24): `pub use chat_v2::*` re-exports extended-chat crypto with no caller outside chat_v2.rs; RtChatContext likewise (foks-proto/src/lib.rs:103).

**Recommendation**

Change the response to `[1, host, features: [u64...]]`. Assign feature IDs in the local registry proposed in protocol-type-id-registry-and-drift. Clients ignore IDs they do not recognise and check dependencies (e.g. threads requires content_actions) only among recognised IDs. A response whose version is higher than the client knows should map to `basic_only` with a logged diagnostic, not an error. Keep `RtChatCapabilities` as the decoded view, with `has(Feature)`. Do this together with the method-namespace assignment ISSUES.md already requires, and add a test that a response with an unknown feature ID decodes to the known subset. Put chat_v2 and RtChatContext behind an `extended-chat` cargo feature until a capability actually enables them.

**Already tracked:** ISSUES.md 'Deferred extended-chat prerequisite' (method-position namespace only). This finding adds the response-shape extensibility and feature-gating of the unused public chat_v2 API.

**Mockup:** [Extended chat preview](../mockups/chat-capability-gated-extensions.html)

<details><summary>Verifier note</summary>

The code facts hold. RtChatCapabilities is an 8-field array: version, host and six booleans (realtime_extension.rs:23-35). from_value requires exactly 8 fields and version 1 (line 79-80). client capabilities.rs maps only MethodNotFound/NotImplemented to basic_only, so a decode error propagates. The server always returns basic_only (services/realtime.rs:233). chat_v2 and RtChatContext have no callers outside their modules. ISSUES.md covers only the method namespace. Correction: ChatSession::capabilities has no production caller. Only its own unit tests and the server-testkit conformance test call it; the agent, client-app and desktop never query capabilities. No shipped client would turn a newer response into an error today, and the shape can change at no compatibility cost. This is preparatory design work, so the priority should be low.

</details>

### protocol-crypto-public-api-docs-and-ct-eq

**Document the foks-crypto root API, type its raw-seed signers, and use constant-time equality on secret types**

- Type: docs
- Priority: low
- Effort: S
- Layers: protocol, docs
- Verification: adjusted

foks-crypto re-exports every module at the crate root with globs; lib.rs:81 says this is deliberate. About 85 of its roughly 186 top-level public items have no doc comment. Examples are seal_puk_seed_chain_box, derive_shared_public, hepk_fingerprint, sign_yubi_typed and seal_backup_puk_boxes_from_credential. From these, a caller cannot tell which bindings the function enforces and which the caller must enforce. foks-verify has docs on about 46% of its top-level public items. SecretSeed implements PartialEq as a plain array comparison, and PermissionToken (a bearer token) derives PartialEq. Client code compares seeds with == in many places (e.g. foks-client passphrase.rs:1595 and team/rotation). These comparisons happen locally in the agent, and the server looks up permission tokens by hash, so constant-time equality is defense in depth, not a fix for a known timing leak.

**Evidence**

- [`crates/foks-crypto/src/lib.rs:81`](../../../crates/foks-crypto/src/lib.rs#L81): Comment 'Keep the public API at the crate root; implementation domains stay private.' and glob re-exports; no missing_docs lint. Root has about 186 top-level public items, about 85 undocumented.
- [`crates/foks-crypto/src/hybrid.rs:77`](../../../crates/foks-crypto/src/hybrid.rs#L77): seal_puk_seed_chain_box has no doc; likewise seal_backup_puk_boxes_from_credential (592), derive_shared_public (757), hepk_fingerprint (765).
- [`crates/foks-crypto/src/signatures.rs:93`](../../../crates/foks-crypto/src/signatures.rs#L93): sign_yubi_typed has no doc.
- [`crates/foks-server/src/keys/mod.rs:61`](../../../crates/foks-server/src/keys/mod.rs#L61): Host signing seeds are already held in SecretKey { bytes: Zeroizing<[u8; 32]> } and passed to sign_ed25519_typed/blob by reference, so the raw-seed signer concern is unfounded.
- [`crates/foks-proto/src/key_material.rs:549`](../../../crates/foks-proto/src/key_material.rs#L549): impl PartialEq for SecretSeed compares as_bytes() with ==.
- [`crates/foks-proto/src/federation.rs:18`](../../../crates/foks-proto/src/federation.rs#L18): PermissionToken derives PartialEq; the server looks it up by hash (foks-crypto/src/primitives.rs:147).
- [`crates/foks-client/src/passphrase.rs:1595`](../../../crates/foks-client/src/passphrase.rs#L1595): owner.seed != *previous_seed: a local, non-constant-time seed comparison inside the agent.

**Recommendation**

First, document the sealing and opening functions in hybrid.rs, backup.rs and team.rs. Each should list the bindings it enforces and the ones the caller must enforce; open_team_shared_key_seed_chain's doc ('Callers must bind each recovered seed...') shows the pattern. Then enable `#![warn(missing_docs)]` on foks-crypto and foks-verify in stages, per module or after a documentation pass. Budget M effort, because the lint also requires docs on public fields and variants. Implement PartialEq for SecretSeed and PermissionToken in foks-proto with `subtle::ConstantTimeEq`, which adds `subtle` as a foks-proto dependency (foks-server-db already uses it directly). Do not add a HostSigningSeed newtype: the server already holds host seeds in `keys::SecretKey { bytes: Zeroizing<[u8;32]> }` and passes borrows. Leave the crate-root re-export policy as it is unless the docs pass shows a concrete need for grouped modules.

<details><summary>Verifier note</summary>

Several parts of the core hold. The root glob re-exports exist, as do the undocumented functions it names: hybrid.rs 77, 592, 757, 765 and signatures.rs:93. SecretSeed's PartialEq is a plain array comparison (key_material.rs:549), PermissionToken derives PartialEq (federation.rs:18), and passphrase.rs:1595 compares seeds with `!=`. Several figures and two recommendations are wrong. (1) foks-crypto has about 186 top-level public items, not 136, and about 85 (46%) have no doc. The 136 figure matches a count that skips all of hybrid.rs. (2) foks-verify is not about 85% documented. 33 of its 72 top-level public items have docs (about 46%); functions alone reach about 78%. `missing_docs` also fires on public fields and variants, so adding it is M effort, not S. (3) The raw-seed signer concern does not hold. The only production callers are in foks-server (host/bootstrap.rs, host/rotation.rs, merkle.rs, standalone.rs). They pass `SecretKey::expose()` borrows of a `Zeroizing<[u8;32]>` (foks-server/src/keys/mod.rs:61). ed25519-dalek 2's SigningKey zeroizes on drop. A HostSigningSeed newtype would duplicate the server's existing type. (4) lib.rs:81 states that the root re-exports are a deliberate design choice, so the grouped-module restructuring is optional at most. (5) SecretSeed appears in the server only inside #[cfg(test)] modules. The server stores and looks up permission tokens by hash (foks-crypto/src/primitives.rs:147, federation_permission_token_hash), so constant-time equality is defense in depth. Both types live in foks-proto, which does not depend on `subtle` yet; foks-server-db already depends on it directly. ISSUES.md does not track this.

</details>

### protocol-crypto-test-organization

**Split the 2853-line foks-crypto tests.rs and consolidate the Yubi test doubles and fixture loaders**

- Type: testing
- Priority: low
- Effort: M
- Layers: protocol, tooling
- Verification: adjusted

foks-crypto/src/tests.rs is one 2853-line module holding 40 tests across signing, user mutations, PUK boxes, teams, KV, Yubi and backup, with no section markers. foks-verify keeps about 1520 lines of tests inline in lib.rs (52-1573). About 61 source files in proto, crypto, verify, rpc, client, client-app, server and testkit hardcode relative paths into crates/foks-snowpack/tests/fixtures, each with its own constants or loader. Finding the test for a given construction is slow, and the fixture paths are copied in each crate.

**Evidence**

- [`crates/foks-crypto/src/tests.rs:59`](../../../crates/foks-crypto/src/tests.rs#L59): PROBE include_bytes! and USER_DIR/SIGNUP_DIR/MUTATION_DIR path constants with their own loaders; similar hardcoded fixture paths appear in about 61 files across the workspace.
- [`crates/foks-crypto/src/tests.rs:77`](../../../crates/foks-crypto/src/tests.rs#L77): MockYubi: the only full software P-256 Yubi double. FixtureYubi (1646) is signing-only with a software HEPK; FixtureHardware (2647) replays recorded DH/KEM secrets and does not implement YubiDevice.
- [`crates/foks-client/src/auth.rs:1293`](../../../crates/foks-client/src/auth.rs#L1293): StubYubiParent: an identity-only stub whose operations call unreachable!(); it is not a P-256 double.
- [`crates/foks-yubi/src/mock.rs:417`](../../../crates/foks-yubi/src/mock.rs#L417): MockYubiDevice is non-test library code behind the public MockYubiProvider, already shared by foks-client, foks-client-app and foks-server-testkit tests.
- [`crates/foks-verify/src/lib.rs:52`](../../../crates/foks-verify/src/lib.rs#L52): Lines 52-1573 are an inline #[cfg(test)] module with include_bytes! fixture constants.

**Recommendation**

Split foks-crypto/src/tests.rs into src/tests/{mod.rs, fixtures.rs, signing.rs, user_mutations.rs, puk_boxes.rs, team.rs, kv.rs, yubi.rs, backup.rs}, with MockYubi moving into the yubi or fixtures module. Add one shared fixture loader, for example a `test-fixtures` feature on foks-snowpack that resolves names against its CARGO_MANIFEST_DIR, and use it from dev-dependencies in place of the per-crate path constants. Move foks-verify's inline tests into a src/tests module file, or into tests/ where they use only the public API. Do not merge the Yubi doubles behind a foks-crypto test-support feature: foks-yubi's MockYubiProvider is already the shared card-level mock, it is compiled into a library that foks-agent links, and the in-crate doubles (FixtureYubi, FixtureHardware, StubYubiParent) model different failure and replay behaviour on purpose.

<details><summary>Verifier note</summary>

Some claims hold. tests.rs has 2853 lines and 40 #[test] functions, with no section markers. foks-verify/src/lib.rs lines 52-1573 are an inline #[cfg(test)] module. 61 source files hardcode relative paths into foks-snowpack/tests/fixtures. The claim that the Yubi doubles duplicate each other does not hold. MockYubi (tests.rs:77) is the only full software P-256 Yubi. FixtureYubi (1646) only signs: DH and KEM return errors and its HEPK comes from a software seed. FixtureHardware (2647) replays recorded DH/KEM fixture secrets and implements only HybridSecretDecapsulator, not YubiDevice. foks-client's StubYubiParent (auth.rs:1293) is an identity-only stub whose operations call unreachable!(). foks-yubi's MockYubiDevice (mock.rs:417) is a card-level mock with slots, PIN authorization and failpoints. It is compiled into the non-test foks-yubi library and exposed as MockYubiProvider, which tests in foks-client, foks-client-app and foks-server-testkit already share. Making it depend on a foks-crypto `test-support` feature would turn that feature on in production builds, because foks-agent links foks-yubi. The doubles also model different behaviour on purpose, so one SoftwareP256Yubi cannot replace them. The section line numbers are approximate (the team tests start at 958, not 963), and the tests are only loosely grouped. Splitting the file and sharing the fixture loaders are still sound low-priority changes.

</details>

### protocol-parcel-open-expectation-struct

**Replace the six-function parcel-opening ladder with a named-field expectation struct**

- Type: code-quality
- Priority: low
- Effort: M
- Layers: protocol, client-lib
- Verification: confirmed

Opening a PUK/PTK parcel goes through six public functions (`open_puk_parcel`, `_for_role`, `_with`, `_with_for_role`, `open_shared_key_parcel_with`, `open_scoped_shared_key_parcel_with`). They take 7 to 12 positional parameters, several of the same type: two or three `&EntityId` (verify key, host, receiver host), two `Role` (receiver role, parcel role) and two `u64` generations. Swapping `expected_role` and `expected_receiver_role`, or the two generations, compiles. The authenticated-binding checks are correct, but whether a caller gets them right depends on argument order, which `#[allow(clippy::too_many_arguments)]` explicitly accepts.

**Evidence**

- [`crates/foks-crypto/src/hybrid.rs:1086`](../../../crates/foks-crypto/src/hybrid.rs#L1086): open_puk_parcel → open_puk_parcel_for_role (1112) → open_puk_parcel_with (1136) → open_puk_parcel_with_for_role (1161).
- [`crates/foks-crypto/src/hybrid.rs:1281`](../../../crates/foks-crypto/src/hybrid.rs#L1281): open_shared_key_parcel_with: 11 positional parameters, including Role, u64, Role, u8 in sequence.
- [`crates/foks-crypto/src/hybrid.rs:1314`](../../../crates/foks-crypto/src/hybrid.rs#L1314): open_scoped_shared_key_parcel_with: 12 positional parameters.

**Recommendation**

Define `pub struct ParcelExpectation<'a> { verify_key: &'a EntityId, verify_key_type: u8, shared_hepk: &'a Hepk, generation: u64, role: Role, host: &'a EntityId, receiver: ReceiverScope<'a> }`, where `enum ReceiverScope { Device, Party { role, generation, host: Option<&EntityId> } }`. Provide constructors `ParcelExpectation::owner_puk(..)` and `::ptk(..)`. Expose a single `open_parcel(parcel, receiver: &dyn HybridSecretDecapsulator, sender_hepk, &ParcelExpectation)`. Keep the old functions as `#[deprecated]` wrappers for one release, then remove them. Optionally add `HostId`/`VerifyKeyId` newtypes over EntityId (RtHostId in foks-proto realtime already uses this pattern) so host and verify-key arguments cannot be swapped.

<details><summary>Verifier note</summary>

The six public openers are at hybrid.rs 1087, 1112, 1136, 1161, 1281 and 1314. Line 1086 in the evidence is the last line of the doc comment; the fn starts at 1087. They take 7, 8, 7, 8, 11 and 12 parameters. open_shared_key_parcel_with ends in `expected_receiver_role: Role, expected_receiver_generation: u64, expected_role: Role, expected_verify_key_type: u8`, and open_scoped_shared_key_parcel_with adds `receiver_host: Option<&EntityId>`. Swapping two values of the same type compiles. The checks at 1328-1356 make most swaps fail closed with an error rather than bypass a binding, so low priority is correct. One detail strengthens the recommendation. Only open_puk_parcel_with_for_role and open_scoped_shared_key_parcel_with have callers outside foks-crypto: foks-client auth.rs (4), recovery.rs (2) and team.rs:3473, plus backup.rs:278 inside foks-crypto. open_puk_parcel, open_puk_parcel_for_role, open_puk_parcel_with and open_shared_key_parcel_with are reached only from foks-crypto's tests and its internal call chain, so collapsing them changes little outside the crate. RtHostId exists (foks-proto/src/realtime.rs:224, rt_entity! macro). A named-field expectation struct does not change the wire format and stays within the project constraints. ISSUES.md does not track this.

</details>

### protocol-type-id-registry-and-drift

**Hand-written 64-bit type IDs have no upstream drift check and no collision registry**

- Type: tooling
- Priority: low
- Effort: M
- Layers: protocol, tooling, ci
- Verification: adjusted

There are 103 `*_TYPE_ID: u64` domain-separation constants across 34 files in 7 crates. Upstream changes to the Snowpack sources are already flagged by the source-hash drift check. However, the hand-transcribed upstream values (42 in foks-proto/src/lib.rs plus others) have never been compared mechanically with the upstream `@0x` declarations, and proto-src/lcl is not hashed at all. A spot check against go-foks v0.1.9 finds no current mismatch. The Rust-local domain separators have no registry: SIGNUP_REQUEST_HASH_TYPE_ID is defined in three files across two crates, the server uses an informal 'FOKS'-suffix scheme, and nothing tests that local IDs are distinct from each other and from upstream IDs. The fix is preventive hardening, not a correction of an existing error.

**Evidence**

- [`tools/foks-protocol-sync/internal/drift/drift.go:302`](../../../tools/foks-protocol-sync/internal/drift/drift.go#L302): compareSources already reports changed Snowpack source hashes (lib and rem), so upstream changes to type IDs surface as source drift. Transcription accuracy is not checked, and proto-src/lcl is not hashed.
- [`tools/foks-protocol-sync/internal/extract/extract.go:488`](../../../tools/foks-protocol-sync/internal/extract/extract.go#L488): snowpackSourceFiles covers only proto-src/lib and proto-src/rem; lcl-derived IDs used by foks-go-interop (lib.rs:21, hard_state.rs:15) are outside the artifact.
- [`crates/foks-proto/src/lib.rs:15`](../../../crates/foks-proto/src/lib.rs#L15): Lines 15-58: 42 hand-typed upstream IDs. Every value matches an @0x declaration in go-foks v0.1.9, so there is no current error.
- [`crates/foks-client/src/account.rs:21`](../../../crates/foks-client/src/account.rs#L21): SIGNUP_REQUEST_HASH_TYPE_ID duplicated at yubi_account.rs:26 and foks-server/src/net/session.rs:551; 0x6d10_7e4a_464f_4b53 is also repeated in foks-server-testkit/tests/team_capacity.rs:51.
- [`crates/foks-crypto/src/chat_v2.rs:11`](../../../crates/foks-crypto/src/chat_v2.rs#L11): The only documented local derivation rule (first eight bytes of SHA-256 of an ASCII label); other local IDs follow no stated scheme.

**Recommendation**

(1) Extend extract.go to record each `struct X @0x…` type unique ID from the Snowpack sources it already reads, adding proto-src/lcl, in a schema v3 artifact, then regenerate upstream-v0.1.9.json. (2) Add a pinned.rs test that maps each upstream-derived Rust constant to its upstream struct name, through a small table in the policy file, and asserts equal values. Generating the constants is optional. (3) Create a single `local_type_ids` registry module, in foks-proto or a small shared crate, for Rust-local domain separators, recording each value's label or derivation. Add a unit test asserting that all local and upstream IDs are pairwise distinct. (4) Replace the three SIGNUP_REQUEST_HASH_TYPE_ID copies, and the testkit's copy of the team-view token ID, with imports.

**Already tracked:** ISSUES.md 'Deferred extended-chat prerequisite' covers only the method-position namespace for the capability method; type-ID drift and collisions are not tracked.

<details><summary>Verifier note</summary>

The core holds in part. foks-protocol-sync's Artifact (model.go:48) has no type-ID field, and pinned.rs has no test comparing the Rust type-ID constants against upstream. Nothing registers the Rust-local domain separators or checks that they are pairwise distinct. SIGNUP_REQUEST_HASH_TYPE_ID is defined three times (account.rs:21, yubi_account.rs:26, session.rs:551). The team-view token ID 0x6d10_7e4a_464f_4b53 is repeated in foks-server-testkit/tests/team_capacity.rs:51. Several framings are wrong. (1) Upstream drift is already detected. extract.go hashes every proto-src/lib and proto-src/rem .snowp file, and drift.go:302 compareSources reports changed SHA-256 or semantic hashes, so an upstream type-ID change would surface as source drift. The real gap is that the original transcription was never checked mechanically, and that proto-src/lcl, which go-interop's SECRET_KEY_BUNDLE and LOCAL_USER_INDEX IDs come from, is not hashed. (2) I compared all 104 unique `*_TYPE_ID: u64` values in the workspace against the 221 `@0x` declarations in the go-foks v0.1.9 proto-src copy in the session scratchpad. Every upstream-derived value appears upstream. The 39 that do not are all Rust-local (client, client-app, server, the KV large-file-size extension, IDENTITY_PROOF), and none collides with an upstream ID or with another local ID apart from the intended duplicates. No transcription error or collision exists today, and the live Go-client tests (foks-server-testkit/tests/go_client_live.rs) plus the official fixtures exercise most wire IDs. The check is preventive, so the priority should be low. (3) Counts: 103 constants in 34 files across 7 crates, not 8. foks-proto/src/lib.rs:15-58 holds 42 upstream IDs. chat_v2.rs is in foks-crypto, not foks-proto. pinned.rs also checks protocol-v1.toml and the digest manifest, not only the three generated .rs files. ISSUES.md tracks only the method-position namespace.

</details>
