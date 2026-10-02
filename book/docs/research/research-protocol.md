# Research note: wire protocol, encoding, RPC, and compatibility testing in foks-rs

Scope: `crates/foks-snowpack`, `crates/foks-proto`, `crates/foks-rpc`, `crates/foks-protocol-metadata`, `crates/foks-go-interop`, `crates/foks-compat-artifact`, `crates/foks-agent-proto`, `tools/foks-protocol-sync`, `tools/foks-v019-oracle`, `tools/foks-server`. Where the transport lives one crate over (`crates/foks-client/src/transport.rs`, `crates/foks-server/src/pki`, `crates/foks-server/src/net/session.rs`) I followed it, because the crates in scope are meaningless without the TLS and connection story around them.

All paths are under `/home/bnoland/projects/foks-rs/`. Line numbers were read from the working tree on 2026-09-25.

The big picture first. FOKS's Rust implementation must produce and consume bytes that are indistinguishable from the official Go implementation (go-foks v0.1.9), because signatures, hashes, and Merkle leaves all commit to those exact bytes. The project solves this with a layered design:

1. A deliberately tiny, canonical subset of MessagePack ("Snowpack") for every authenticated object, where each abstract value has exactly one byte encoding.
2. A separate, more permissive MessagePack envelope for RPC framing (borrowed from Keybase's framed-msgpack-rpc lineage via go-snowpack-rpc) that carries canonical payloads opaquely.
3. Protocol and method identifiers extracted mechanically from the Go source by parsing its AST, pinned by checksum, and rendered into generated Rust constants.
4. Differential testing against the Go implementation at three distances: checked-in byte fixtures produced by the Go code, a Go "oracle" that verifies Rust output with Go's own validators, and live mixed-implementation runs (Rust client against Go server, Go client against Rust server).
5. A signed "compatibility artifact" that turns the outcome of hosted canary runs into a short-lived, revocable capability lease.

Each section below covers one technique: the problem, the mechanism, why, where, and references.

---

## 1. Snowpack: a canonical MessagePack subset

### Problem

FOKS signs things: chain links, Merkle roots, public zones, KV directory entries. A signature is over bytes, so if Rust and Go could encode the same logical struct two different ways, a link built by one would fail verification by the other, and worse, the Merkle tree (whose leaves are hashes of encoded links) would diverge. MessagePack as specified has many ways to encode the same value (an integer 5 can be `0x05`, `0xcc 0x05`, `0xcd 0x00 0x05`, ...; maps have arbitrary key order). Snowpack is go-foks's answer: pick one encoding per value and reject the rest.

### How it works

The data model is in `crates/foks-snowpack/src/lib.rs:21-38`. There are only eight kinds of value:

```
Null, Bool, Unsigned(u64), Negative(i64), Binary(Vec<u8>), Text(Vec<u8>),
Array(Vec<Value>), Variant(Option<(Vec<u8>, Box<Value>)>)
```

Notice what is absent: floats, extension types, and general maps. The doc comment (`lib.rs:1-6`) states the two structural conventions:

- **Structs are positional arrays** (Go's `toarray` codec option). A struct with fields `{a, b, c}` is encoded as a 3-element array. Field names never appear on the wire, so there is no key ordering problem and no ambiguity about which fields are present.
- **Variants (tagged unions) are fixed maps with at most one entry.** An unset variant is the empty map `0x80`. A set variant is a one-entry map `0x81` whose key is a short string tag (at most 31 bytes, `fixstr`) and whose value is the payload. This is what go-codec emits for a Go union struct with one active arm, and Snowpack freezes it.

The encoder (`crates/foks-snowpack/src/encode.rs`) enforces minimality:

- Unsigned integers use the shortest marker: `0..=0x7f` as a positive fixint, then `uint8/16/32/64` (`encode.rs:212-229`).
- Negative integers likewise: `-32..=-1` as a negative fixint, then `int8/16/32/64` (`encode.rs:231-252`). Crucially, `Value::Negative` refuses zero or positive values (`encode.rs:232-234`), so `-0` and `+5` can't sneak into the signed branch. This gives one canonical form for every integer.
- Binary uses `bin8/16/32` with the smallest header (`encode.rs:254-258`, `352-375`).
- Text uses `fixstr` (up to 31 bytes), then `str8/16/32` (`encode.rs:260-277`).
- Arrays are **never empty**. `array_header` returns `ErrorKind::EmptyArray` for length 0 (`encode.rs:301-304`); an empty list is represented as `Null` at the schema level (see `foks-proto/src/codec.rs:34-40`, where `list()` maps `Null` to an empty `Vec`). The error message says it plainly: "empty arrays must be null" (`error.rs:571`).
- Variant tags longer than 31 bytes are rejected (`encode.rs:324-326`).

The decoder (`crates/foks-snowpack/src/decode.rs`) is the mirror image and is where the "reject everything else" policy lives. The marker dispatch at `decode.rs:146-281` is worth reading in full; the highlights:

- `0xc1` (reserved) is an error; `0xc7..=0xc9`, `0xd4..=0xd8` (extensions) are `UnsupportedExtension`; `0xca`, `0xcb` (float32/64) are `UnsupportedFloat`; `0xde`, `0xdf` (map16/32) are `ArbitraryMap`; `0x82..=0x8f` (fixmaps with 2-15 entries) are `InvalidVariantSize`.
- Each multi-byte integer or length form checks that the value could not have fit a shorter form. Example: `0xcc` followed by a byte `<= 0x7f` is `NonMinimal("unsigned integer")` (`decode.rs:180-186`). The same check applies to `str8` with length `<= 31`, `bin16` with length `<= 255`, `array32` with length `<= 65535`, and so on.
- `decode()` requires the whole input to be consumed (`decode.rs:10-16`, `TrailingBytes`), so a canonical object can't have junk appended without failing.

The invariant this buys is stated and tested as a QuickCheck property in `crates/foks-snowpack/tests/properties.rs:131-141`: for any canonical `Value`, `decode(encode(v)) == v`, and `encode(decode(bytes)) == bytes` for any accepted `bytes`. The checked-in Go fixtures are round-tripped byte-exactly in `tests/official_fixtures.rs:134-173`. Canonical uniqueness is the property that lets the Rust and Go implementations sign the same bytes without ever exchanging a schema at runtime.

### The `array16` wrinkle (a real cross-implementation subtlety)

There is one place where "minimal" and "what Go's canonical checker accepts" disagree, and the code handles it with two validation modes. MessagePack's `fixarray` only covers lengths 0-15, so lengths 16-31 have no shorter form than `array16`. But go-foks's canonical validator for *signed* objects rejects any `array16` with length `<= 0x1f` regardless (`decode.rs:261-267`). Meanwhile, real RPC arguments use it: the 16-field signup argument is an `array16(16)`. So:

- `validate()` / `decode()` accept `array16` of 16..=31 (general RPC codec).
- `validate_signable()` (`decode.rs:47-54`) sets `signable = true` and rejects them with `NonCanonicalSignable("array16")`, recursively (`decode.rs:265-267`).

The README (`crates/foks-snowpack/README.md`) spells this out. Every signing and hashing primitive in `foks-crypto` calls `validate_signable` first (`crates/foks-crypto/src/lib.rs:719-722`, `746-759`), so a Rust-produced object that Go would refuse to verify can't be signed in the first place. This is a good example of matching the *reference implementation's* behavior rather than the spec's.

### Defensive decoding

Snowpack frames are network-facing, so the decoder has three resource bounds:

- `MAX_DEPTH = 64` nesting (`lib.rs:19`, checked at `decode.rs:143-145` via the path stack).
- `MAXIMUM_DECODED_VALUES = 1_000_000` total nodes (`decode.rs:5-8`), because a few bytes of nested `[[[[...]]]]` expand into many heap `Value` nodes. The comment explains the amplification concern.
- Array length pre-checks before allocation (`decode.rs:307-313`): a header claiming N elements is rejected if fewer than N bytes remain, so `[0xdd, 0xff, 0xff, 0xff, 0xff]` fails with `UnexpectedEof` rather than trying to allocate 4 billion slots. Tested at `decode.rs:451-466`.

### Error paths and sensitive decoding

Errors carry a structural path (`error.rs:500-505`: `PathSegment::Index(n)` and `PathSegment::Variant(tag)`), rendered as `Snowpack error at byte 7[3].tag: ...`. This makes fixture mismatches diagnosable. There's a `decode_sensitive` variant (`decode.rs:30-38`) that returns a `Zeroizing<Value>` and redacts variant tags from error paths so secrets don't leak into logs, and `encode_ref` (`encode.rs:99-109`) pre-computes the output capacity so that a `Vec` growing mid-write never leaves an unerased plaintext copy in a freed allocation. `decode_prefix` (`decode.rs:21-25`) decodes one value and reports bytes consumed, used for plaintexts that are zero-padded after the canonical value.

### Why

The lib.rs and README are explicit: this is "not a general-purpose MessagePack codec"; it exists so that "signed, verified, and protocol-hashed objects" have exactly one representation. The design was inherited from go-foks; foks-rs's contribution is proving the subset with property tests and fixtures.

### References

- MessagePack specification: https://github.com/msgpack/msgpack/blob/master/spec.md (marker table, fixint/fixstr/fixarray ranges).
- go-foks Snowpack: https://github.com/foks-proj/go-foks (the `proto-src/*.snowp` schema files and `lib/core` canonical checker). [VERIFY exact paths]
- go-snowpack-rpc: https://github.com/foks-proj/go-snowpack-rpc (referenced by name in the oracle imports, `tools/foks-v019-oracle/main.go:21`).
- Keybase's `go-framed-msgpack-rpc`, the ancestor of go-snowpack-rpc: https://github.com/keybase/go-framed-msgpack-rpc [VERIFY lineage claim; the Rust code references only go-snowpack-rpc by name].
- go-codec's `toarray` and union-as-single-key-map conventions: https://github.com/ugorji/go [VERIFY].

---

## 2. Schema layer: exact positional types with fail-closed decoding

### Problem

Snowpack proves that bytes are canonical; it doesn't say what they mean. `foks-proto` applies the v0.1.9 schemas (host chain, Merkle root, user/team chain links, KV objects, etc.) on top of a decoded `Value`.

### How it works

`crates/foks-proto/src/codec.rs` provides small typed accessors: `array(value, expected_len)` fails with `FieldCount` on the wrong arity (`codec.rs:8-19`), `variant(value, "1")` requires an exact tag (`codec.rs:53-64`), `fixed_blob::<32>` requires an exact byte width (`codec.rs:103-110`), `expect_unsigned` requires a known enum discriminant (`codec.rs:88-94`). The README says "Unknown versions, union tags, entity types, field counts, and fixed-size blobs fail closed."

Versioned objects follow a fixed pattern that shows up everywhere: `[version_number, {"version_number": payload}]`. E.g. a `LinkOuter` is `[1, {"1": [inner_bytes, [signatures]]}]` (`crates/foks-proto/src/identity/chain.rs:674-696`), a Merkle root is `[1, {"1": ...}]`, a hostchain link the same (`crates/foks-proto/src/host.rs:89-102`). The integer and the variant tag both carry the version, redundantly, which is how go-foks's `Future(T)`-style versioned unions serialize.

**Forward compatibility by retaining extension fields.** Where go-foks may append fields, the Rust decoders accept `array_at_least` (`codec.rs:21-32`) and keep the tail. `foks-proto/src/lib.rs:379-440` tests that a probe response, a Merkle root, and a hostchain change with future appended fields still decode and, importantly, that the Merkle root *re-encodes to the same evolved bytes* (`lib.rs:411`), because those bytes are what the hash and signature cover. The `PublicZone` service list (`lib.rs:344-376`) accepts five or more service endpoints with the sixth (`realtime`) defaulting to empty.

**Exact inner bytes are retained, never re-derived.** `UserLink` keeps `inner: Vec<u8>` verbatim (`chain.rs:11-17`), with the comment "because both stacked signatures and the Merkle leaf commit to these bytes." `SignedBlob` (`entity.rs:135-152`) is `[inner_bytes_as_binary, signature]`. The pattern is: a signed object is a Snowpack *binary* containing another Snowpack encoding, so the outer layer never has to re-serialize the inner one. This is the load-bearing trick for cross-implementation signature stability: verification only requires agreeing on the *outer* layout and hashing/signing the *inner bytes as delivered*.

**Entity IDs** (`entity.rs:9-33`) are a one-byte type prefix followed by a key: 33 bytes for Ed25519 entities (types 1-7, 9-21) and 34 for the P-256 YubiKey type 8. The type byte doubles as a namespace: `to_rolling_entity_id` (`entity.rs:48-57`) rewrites a user ID (type 1) to a PUK-verify ID (type 14) by changing only the prefix, so the same public key bytes name both the persistent identity and its current signing key.

### Where

- `crates/foks-proto/src/codec.rs` (accessors), `src/lib.rs:181-238` (type IDs and entity type constants), `src/entity.rs`, `src/host.rs`, `src/identity/chain.rs`.

---

## 3. Domain-separated hashing and typed signatures

### Problem

A signature over raw bytes is only as safe as the guarantee that those bytes couldn't be interpreted as something else. If a Merkle root and a public zone could produce the same encoding, a signature on one would be valid for the other.

### How it works

Every object type in FOKS has a 64-bit random "type ID" (the constants at `crates/foks-proto/src/lib.rs:181-222`, e.g. `MERKLE_ROOT_TYPE_ID = 0xa88f_c49b_6df3_a111`). Hashing and signing always prepend the 8-byte big-endian type ID:

- `prefixed_hash(type_id, object)` = SHA-512/256( be64(type_id) || object ) (`crates/foks-crypto/src/lib.rs:708-713`).
- `prefixed_hash_signable` first runs `validate_signable` (`lib.rs:719-722`).
- `sign_ed25519_typed(seed, type_id, object)` signs `be64(type_id) || object` (`lib.rs:746-759`).
- `sign_ed25519_blob(seed, blob_type_id, inner)` validates `inner`, wraps it as a Snowpack binary, and signs that (`lib.rs:761-767`), mirroring go-foks's `Future(T)` signed-blob shape.
- HMAC commitments and capability MACs use the same prefix idea with HMAC-SHA-512/256 (`lib.rs:781-789`, `560-586`).
- Even symmetric nonces are domain-separated: the KEX and secret-box nonces put the type ID in the first 8 bytes of the 24-byte XSalsa20 nonce (`crates/foks-crypto/src/kex.rs:177`, `crates/foks-go-interop/src/lib.rs:179-181`).

**Stacked signatures.** A chain link may be signed by more than one key (e.g. the device key and the PUK). The signing bytes for signature k are `[inner, [sig_0 .. sig_{k-1}]]`, i.e. each signature covers the inner bytes *plus all previous signatures*. `UnsignedUserLink::signing_bytes(&signatures)` (`chain.rs:662-672`) and `HostchainLink::signing_bytes(signature_count)` (`host.rs:58-80`) both build exactly this prefix, with an empty prefix encoded as `Null` (again: no empty arrays). This is why the order of signatures is part of the protocol and why the oracle README notes the "pinned Go client's reversed verifier order for rotated certificates."

### Why

The `foks-crypto` doc comments say it: "Enforces canonical signable Snowpack rules before hashing to ensure cross-implementation digest consistency." The type prefix is classic domain separation; the "inner bytes as binary" wrapper is what makes the outer signature independent of re-serialization.

### References

- SHA-512/256: FIPS 180-4, section 5.3.6.
- Ed25519: RFC 8032.
- Domain separation as a design principle: e.g. the "Cryptographic Doom Principle" and NaCl/libsodium docs on unique nonces. [VERIFY choice of citation]

---

## 4. The RPC envelope: framed MessagePack with a canonical core

### Problem

Snowpack is too strict for an RPC layer that carries headers, status codes, and log tags; those are ordinary Go structs encoded with named fields. foks-rs therefore keeps two codecs and is careful about the boundary.

### Framing

Each message is prefixed by its length, itself encoded as a minimal MessagePack unsigned integer (`crates/foks-rpc/src/lib.rs:2624-2635`, `frame()`). The reader (`read_frame_length`, `lib.rs:2637-2668`) accepts only `fixint`, `uint8`, `uint16`, `uint32` length markers, rejects non-minimal ones with `FrameLengthMarker`, and enforces a maximum (`DEFAULT_MAX_FRAME_LENGTH = 16 MiB`, `lib.rs:50`) before allocating (`read_frame`, `lib.rs:1954-1965`). This is exactly the framed-msgpack-rpc framing: "the length is a msgpack int, then a msgpack array."

### Call and response shapes

A call is a 5-element array (`lib.rs:333-358`, `encode_call_with_validated_argument`):

```
[ METHOD_CALL_V2 (=5), sequence, protocol_id, method_position, payload ]
```

The server also accepts a 6-element form with a trailing log-tags element, because go-foks tags every call including the first probe (`crates/foks-rpc/src/server.rs:101-124`).

A response is a 4-element array (`lib.rs:1973-2003`, `decode_response`):

```
[ METHOD_RESPONSE (=1), sequence, error_or_nil, result ]
```

Method type constants come from go-snowpack-rpc's `protocol.go`: 0 = CALL (v1), 1 = RESPONSE, 2 = NOTIFY, 3 = CANCEL, 4 = CALL_COMPRESSED, 5 = CALL_V2, 6 = NOTIFY_V2, 7 = CANCEL_V2 (`server.rs:38-45` and `tests/control_messages.rs:89-99`). The Rust server answers only CALL_V2; NOTIFY and CANCEL (both versions) are parsed for well-formedness and silently discarded (`server.rs:56-98`, `InboundMessage::Control`), because go-snowpack-rpc never expects a reply and treating them as fatal would drop otherwise-healthy connections. CALL_COMPRESSED is deliberately *fatal* ("expects a reply and must not be dropped", `tests/control_messages.rs:89-91`). There is no compression anywhere in the Rust wire path; `grep -i compress` finds only Merkle "compressed path" types.

### The DataWrap envelope and the compatibility header

Most protocols wrap the argument and the result in a two-key named map `{"Data": <canonical snowpack>, "Header": <compat header>}` (`lib.rs:350-354`). The header is the fixed byte string at `lib.rs:54-56`:

```
0x82 "V" 1 "f1" 0x81 "Vers" 1
```

i.e. `{V: 1, f1: {Vers: 1}}`, a versioned union in go-codec's *named* encoding (contrast the positional Snowpack encoding of the same shape). `check_compatibility_header` (`lib.rs:2599-2622`) accepts any nonzero `Vers`, matching go-foks, which only treats generation 0 as too old. `CURRENT_COMPATIBILITY_VERSION = 1` (`lib.rs:49`).

The map keys `Data` then `Header` are written in that order because go-codec sorts struct fields canonically (`lib.rs:350` comment). The decoder requires the exact order (`decode_data_wrap`, `lib.rs:2168-2203`).

**Headerless protocols.** The team protocols (TeamLoader, TeamAdmin, TeamMember, TeamGuest) and Kex put the bare argument/result in the payload slot with no DataWrap at all. This is a quirk of how those Go files were generated, and it is captured as two generated predicates, `is_headerless_argument_protocol` / `is_headerless_result_protocol` (`crates/foks-rpc/src/generated/protocol_ids.rs:146-160`), keyed by protocol ID. The client picks the matching response decoder by reading the protocol ID out of the request it is about to send (`call_protocol_id`, `lib.rs:307-331`; used in `crates/foks-client/src/transport.rs:293-296`). `tests/data_wrap.rs:29-62` locks the split. Void results also differ: a wrapped void is `{"Header": ...}` alone (`response.rs:155-166`), a bare void is a literal nil (`lib.rs:2101-2112`), and the exact bytes `[0x05, 0x94, 0x01, 0x05, 0xc0, 0xc0]` are pinned in a test (`tests/data_wrap.rs:117-123`).

### Keeping the layers separate

`encode_call` validates the argument with `foks_snowpack::validate` (general mode) before wrapping it (`lib.rs:246-258`). `decode_response` returns the *exact result bytes* out of the DataWrap without re-encoding (`lib.rs:2168-2203`, comment at 2198-2201: "RPC results are generated MessagePack structs, not authenticated Snowpack values as a whole... Type-specific decoders and verifiers still canonicalize every signed or MACed inner object"). One sanctioned exception: Go encodes a zero-field struct argument as an empty array `0x90`, which canonical Snowpack forbids; the server accepts exactly that one byte string (`server.rs:141-158`) and the client emits it for `getHostConfig` (`lib.rs:1183-1192`).

The envelope parser is a separate skipping cursor (`Cursor::skip`, `lib.rs:2766-2830`) that understands *all* MessagePack markers (including floats, extensions, and maps it never interprets) so it can locate the payload slot in a Go-emitted frame without decoding it. That is the boundary: the envelope tolerates general MessagePack; the payload is checked as Snowpack.

### Sequence numbers, resequencing, and exactly-once

Requests are prepared once with sequence 0 and then *resequenced* just before sending: `resequence_call` (`lib.rs:265-305`) re-parses the frame, replaces only the second element, and copies the argument bytes verbatim. The doc comment gives the reason: "preserves signed payloads while allowing an authenticated connection to serve more than one request." A response with the wrong sequence is `Error::Sequence`.

In the client (`crates/foks-client/src/transport.rs:285-335`, `call_current`), each connection carries `next_sequence`; it increments only after a complete response frame (success *or* application status error) has been read, and any other failure invalidates the connection so "an unread or partial response must never satisfy a later call" (`transport.rs:265-277`). Idle pooled connections are probed with a non-blocking `peek` before reuse (`is_reusable`, `transport.rs:100-120`) so a server-closed keep-alive is discovered before anything is written; the comment explains this keeps RPCs "exactly-once."

### Status (error) encoding: one logical shape, two wire spellings

A FOKS `Status` is a Go `toarray` struct `[Sc, Switch]` where `Switch` is a one-arm union keyed by a codec tag character: `"1"` for a detail string, `"a"` for KV permission `[op, resource]`, `"b"` for the stale-cache path-version vector, `"3"` for method-not-found `[proto, method, name]`, `"4"` for a nested OAuth2 status. The Rust server builds this through Snowpack (`crates/foks-rpc/src/response.rs:168-243`), which reproduces go-foks's bytes exactly including `[Sc, {}]` for no payload. The comment at `response.rs:169-176` explains the go-codec union mapping.

But Go's *RPC layer* may also encode a status as a named map (`{Sc: 8012, f11: {...}}`), so the client's `check_status` (`crates/foks-rpc/src/status.rs:8-104`) branches on the first byte: `0xc0` = success, `0x92` = positional, otherwise a named map. Both paths recognize the special codes: `METHOD_NOT_FOUND_ERROR = 211` becomes `Error::MethodNotFound`, `KV_STALE_CACHE_ERROR = 8012` becomes `Error::KvStaleCache(vector)` (the server is handing the client the versions it needs to resync), and `NOT_IMPLEMENTED = 1020` must carry no payload. Status codes are generated constants (`generated/status_codes.rs`), and `status_phrase` (`lib.rs:115-147`) maps the human-provocable ones to plain sentences.

### References

- go-snowpack-rpc `protocol.go` method types [VERIFY line numbers in upstream].
- Keybase framed-msgpack-rpc protocol description: https://github.com/keybase/go-framed-msgpack-rpc/blob/master/README.md [VERIFY].
- MessagePack-RPC spec (the ancestral 4-element request/response array): https://github.com/msgpack-rpc/msgpack-rpc/blob/master/spec.md.

---

## 5. Protocol and method identifiers: extracted, pinned, generated

### Problem

Each go-foks protocol has a 32-bit unique ID (e.g. `Reg = 0xf7ab85f3`) and each method a small integer position (e.g. `Reg.signup = 2`). Positions are not contiguous (Reg has 0..=23 with gaps), and they are the dispatch key on the wire. Hand-copying them is error-prone, and upstream can change them.

### How it works: AST extraction, not execution

`tools/foks-protocol-sync` is a standard-library-only Go program that parses the go-foks source with `go/parser` and `go/ast` and never imports or runs upstream packages (README). `internal/extract/extract.go:31-166` walks `proto/rem/*.go`:

- Protocol IDs come from `var XxxProtocolID = ...` declarations (`extract.go:60-84`), evaluated with a tiny constant evaluator (`integer()`, `extract.go:651`).
- Methods come from finding the single `NewMethodV2(protocolVar, position, "Proto.method")` call inside each generated client function (`clientMethods`, `extract.go:233-298`).
- Whether a protocol uses DataWrap is *inferred from the generated client code*: `bindingUsesDataWrap(function.Body, "warg")` and `(…, "tmp")` look for the local variables the generator names `warg` and `tmp` being typed as `rpc.DataWrap[...]` (`extract.go:284-287`, `338-382`). That's how `argument_header`/`result_header` in the artifact are determined, and hence the headerless predicates.
- Client methods are cross-checked against the server handler tables (`compareHandlerMethods`, `extract.go:471`).
- Status codes and service types come from `proto/lib/status.go` (`StatusCode_*`) and `proto/lib/common.go` (`ServerType_*`) (`extract.go:130-137`).
- Every `.snowp` schema source gets two SHA-256 digests: the raw file, and a "semantic" digest of a token-normalized form (`semanticSnowpack`, `extract.go:168-226`) that strips comments and whitespace and length-prefixes each token. This lets the drift tool tell "someone reformatted a comment" from "the schema changed."

The output is a JSON artifact (`crates/foks-server/protocol/upstream-v0.1.9.json`) whose collections are sorted (`model.go:57`, `Normalize`) so extraction is deterministic. `tools/foks-server/generate-protocol.sh` runs the extractor **twice** and `cmp`s the results before accepting them, a cheap determinism check.

### Pinning

The artifact's SHA-256 is pinned as a Rust constant: `PINNED_PROTOCOL_METADATA_SHA256` in `crates/foks-protocol-metadata/src/lib.rs:12-15`. The source identity (module, version, `go.sum` hashes, commit) is embedded in the artifact (`lib.rs:28-37`); the upstream module is checksum-locked through `tools/foks-v019-oracle/go.mod` / `go.sum`. Ordinary Cargo builds never touch Go; they compile checked-in generated files.

### Merging with local policy and rendering

`foks-protocol-metadata::merge` (`lib.rs:289-545`) combines the artifact with a handwritten TOML policy (`crates/foks-server/protocol/policy-v1.toml`) that says which protocols and routes the Rust server supports, on which listener, with what authentication, and which status codes each may return. Validation is strict: every referenced upstream method must exist, no dispatch key `(protocol_id, position)` may repeat, principal-bound routes must be on the `authenticated` listener, etc.

Local extensions are ring-fenced so they can't collide with upstream: local protocol IDs must be in `0xf04b0000..=0xf04bffff` (`lib.rs:323-333`), and local method positions in `65536..=131071` with names starting `foks` (`lib.rs:393-414`). The generated file shows the result: `IDENTITY_PROTOCOL_ID = 0xf04b0001` with positions 65536+ (`generated/protocol_ids.rs:141-144`) and the realtime extension `RT_CHAT_CAPABILITIES_METHOD_POSITION = 65536` (`:128`).

`render_protocol_ids` / `render_status_codes` / `render_routes` (`lib.rs:670-876`) write `crates/foks-rpc/src/generated/*.rs`, the server's route table, and a merged `protocol-v1.toml` contract. `generate-protocol.sh --check` fails CI if any is stale.

### Drift classification

`tools/foks-protocol-sync/internal/drift/drift.go` compares the pinned artifact with one extracted from an arbitrary upstream commit (`tools/foks-server/diff-upstream-protocol.sh` resolves upstream `HEAD` to an immutable SHA and does a depth-1 fetch). Every difference is classified (`drift.go:15-23`):

- `wire_breaking`: an ID, position, header flag, or result type changed on a *supported* protocol/method, or an ID/position was reused for a different name;
- `behavior_review_required`: a `.snowp` source whose *semantic* digest changed and that feeds a supported protocol (with the affected coverage IDs listed);
- `additive`: new protocol/method/status;
- `outside_local_slice`: a change to something Rust doesn't implement;
- `source_only`: formatting-only change (raw digest differs, semantic digest same).

The classifier uses the policy's `coverage` IDs to say which tests would catch a given change (`drift.go:410-437`). This is a nice example of turning "did upstream change?" into a triaged report.

---

## 6. Differential testing against the Go implementation

This is the heart of the compatibility story and deserves its own chapter. There are four rings.

### Ring 1: checked-in golden fixtures from Go

`tools/foks-v019-oracle` is a Go program that links the *official* go-foks v0.1.9 modules and writes byte fixtures under `crates/foks-snowpack/tests/fixtures/foks-v0.1.9/{foks.app,signup,user,user-mutations,realtime,invitations,sso,account}`. Two file classes exist (`main.go:69-98`):

- `.snowp` files go through `writer.bytes`, which first calls `core.AssertCanonicalMsgpack` (Go's own canonical checker) and refuses to write anything that fails (`main.go:75-80`). So every `.snowp` fixture is Go-certified canonical.
- `.frame` files (`writer.raw`) are ordinary RPC frames "intentionally not classified as a canonical Snowpack object" (README).

Each directory has a `manifest.json` with size and SHA-256 per file, plus provenance: for the probe capture, the host ID, hostchain tail, Merkle epoch, root-node hash, and the independently computed *prefixed* hash of the whole root (`main.go:288-303`). Before writing, the oracle verifies the hostchain, public-zone signature, Merkle-root signature, and the root-to-hostchain binding with Go's verifiers (`main.go:193-231`).

On the Rust side, `crates/foks-snowpack/tests/official_fixtures.rs` checks every manifest entry's size and digest, requires that the on-disk file set equals the manifest (no stray files), and round-trips every `.snowp` byte-exactly (`:134-196`). `foks-proto` then decodes them into typed structs and re-encodes to the same bytes (`foks-proto/src/lib.rs:462-487`). `foks-rpc`'s tests do the same for request frames: the Rust encoder must reproduce Go's `probe-request.frame` (see `TestCheckedProbeRequestUsesGeneratedDescriptor`, `tools/foks-v019-oracle/rpc_fixture_descriptor_test.go:107-122`, which pins the Go side too).

The mutation fixtures are generated with deterministic randomness ("sha256-counter-v1" generator, `official_fixtures.rs:75`) and the Go test suite generates the whole corpus twice requiring byte-identical output (README). The README also documents a deliberately preserved Go quirk: `TeamCreator` emits a founding membership link with `Seqno` unset (zero) even though the team link is sequence one; fixtures retain it so Rust can decode Go history losslessly, but Rust is not permitted to emit it.

The fixtures also encode a negative space: `.frame` responses for error cases (`merkle-leaf-not-found-response.frame`, `kv-stale-cache-response.frame`, `reg-probe-key-not-found-response.frame`) so status decoding is tested against real Go bytes, not a Rust reimplementation of Go's encoder.

### Ring 2: the oracle re-verifies Rust output with Go code

Beyond emitting fixtures, the oracle *reads* Rust-produced objects and pushes them through Go's validators. Examples from the README and test files:

- `run-host-rotation-compat.sh`: the Rust server generates host-key add/revoke probe blobs; the Go oracle replays and verifies each chain with `core.PlayChain`.
- `TestGeneratedWireArgumentSelectsExactEnvelope` (`rpc_fixture_descriptor_test.go:49-74`) confirms, using Go's own generated descriptors, that `Probe.probe` uses `DataWrap` and `TeamLoader.loadTeamChain` is bare, which is the ground truth for the Rust headerless predicates.
- `TestGenerateYubiFixtures` uses Go's mock PIV bus so YubiKey encodings are covered without hardware.

### Ring 3: live mixed-implementation runs

- **Rust client vs. Go server**: `run-live-compat.sh` builds `foks-client --example live_compat` and runs `TestRustClientHappyPath` (`tools/foks-v019-oracle/live_compat_test.go:20-60`), which boots the unmodified Go v0.1.9 integration environment (Postgres in Docker via testcontainers) and hands the client side to the Rust binary: signup, chain load, PUK, KV writes, cache-check sync, realtime chat, KEX. One notable accommodation: the Go test helper's 1024-bit RSA probe leaf is replaced with a production-strength leaf because rustls correctly rejects it (`live_compat_test.go:49-60`).
- **Go client vs. Rust server**: `run-go-client-compat.sh` and `TestGoClientAgainstRustServer` (`go_client_rust_live_test.go:34+`) drive the official Go client library (`core.NewProbeClient`, `core.NewRegClient`, …) against a Rust server, verifying the Rust hostchain, Merkle root, and public zone with Go's `PlayChain`/`CheckZoneSig`, then registering, KEX-relaying, log-sending, and so on.
- Further scripts cover chat, teams, invitations (both directions across mixed hosts), SSO, MCP, and account flows. All are opt-in via environment variables so `cargo test` and `go test ./...` stay hermetic.

### Ring 4: property tests and rejection matrices

`crates/foks-snowpack/tests/properties.rs` generates arbitrary canonical values with array lengths chosen at the fixarray/array16/array32 boundaries (`:76-84`), checks round-trips, and checks that every shrink remains canonical. `tests/marker_matrix.rs` and `tests/boundaries.rs` (not read in detail) exhaustively cover marker classes. `crates/foks-rpc/tests/properties.rs`, `server_rejections.rs`, and `control_messages.rs` do the same for the envelope.

### Why this shape

The README for the oracle is candid about what each ring does and does not prove: fixtures are "encoding oracles rather than one coherent mutation against a Postgres-backed server"; the live gate "exercises SDK/core code; it is not a claim that every interactive Go CLI invitation flow is covered." The manifest "deliberately carries no self-asserted verification booleans"; a manifest is only written after Go's checks pass. That's a disciplined way to keep the fixtures trustworthy: the *presence* of a manifest is the assertion.

---

## 7. Transport: TLS, host-chain-pinned roots, and client certificates

### Problem

How does a client know it is talking to the real host, and how does the server know which device is calling?

### Server identity: two trust models

- **Probe** (first contact): the probe service is reached over ordinary WebPKI TLS (`FoksClient::webpki`, `crates/foks-client/src/transport.rs:392-400`, using `webpki-roots`). Its response, the `ProbeResponse`, contains the signed hostchain, the signed public zone, and the signed Merkle root, all independently verifiable (Section 2). This call is "intentionally not pooled and is suitable only for discovery data that will be independently authenticated" (`transport.rs:717-729`, `call_unpinned`).
- **Everything else**: the hostchain delegates a *TLS CA* key (`ENTITY_HOST_TLS_CA = 11`, `foks-proto/src/lib.rs:228`). After the probe is verified, the client builds a `RootCertStore` containing *only* the host's own delegated CA certificates (`authenticated_tls_roots`, `crates/foks-client/src/host.rs:256-266`) and uses it for the registration, user, Merkle, KV, and realtime services. So the host's identity chain, not a public CA, roots service TLS. On the server, `build_host_tls` (`crates/foks-server/src/pki/host_tls.rs:21-67`) creates a self-signed Ed25519 "FOKS delegated TLS CA" from a host key and issues the server leaf from it; the same CA DER is what gets committed into the hostchain.

Both sides pin the crypto provider (`aws_lc_rs`) explicitly because both ring and aws-lc can appear in the dependency graph (`transport.rs:657-660`). `TCP_NODELAY` is set on both sides because "RPCs exchange small TLS records" (`transport.rs:620-623`, `session.rs:1351-1352`).

### Client identity: mutual TLS with Ed25519 device certificates

Authenticated services require a client certificate. The flow:

1. A device's Ed25519 key is already enrolled in the user chain. The device calls the public, unauthenticated `Reg.getClientCertChain(uid, device_id)` (`crates/foks-rpc/src/lib.rs:596-611`; position 1 in `protocol_ids.rs:13`).
2. The Rust server (`crates/foks-server/src/net/session.rs:806-899`) checks the device is an active credential owner, issues (or reuses, if more than a day remains) a 7-day X.509 certificate whose subject public key *is* the device's Ed25519 key, signed by the host's "FOKS client identity CA" (`host_tls.rs:24-25`), records it, and returns it. Clock skew of 5 minutes is tolerated on `not_before`.
3. The client wraps its device seed as a PKCS#8 Ed25519 private key and presents the certificate chain via rustls's `SingleCertAndKey` resolver (`transport.rs:653-691`), after checking that the key matches the certificate (`keys_match`).
4. The server's authenticated listener uses `WebPkiClientVerifier` rooted at the client CA (`host_tls.rs:45-58`). After the handshake, `Principal::authenticate` (`crates/foks-server/src/auth/principal.rs:23-63`) parses the peer certificate, requires an Ed25519 SPKI (OID 1.3.101.112), looks up the certificate in the database to find the bound credential, and **re-checks that the certificate's public key equals the key inside the credential's entity ID** (`principal.rs:36-40`). The credential kind (software device, delegated subkey, backup key, bot token) then gates which routes are allowed (`principal.rs:76-108`).

So there is no signed-challenge login for ordinary RPC; identity is carried by the TLS client certificate, and the certificate is just a WebPKI-shaped wrapper around the chain-enrolled Ed25519 key. (Challenge/response does exist for specific flows: `Reg.getLoginChallenge`/`login` for passphrase login, `getUidLookupChallenge`, and team view-token challenges signed with the PUK, `foks-crypto/src/lib.rs:820-830`.)

### Connection pooling keyed by security material

`PoolKey` (`transport.rs:178-208`) includes hostname, port, host ID, a fingerprint of the client credential, a fingerprint of the trust roots, and the "selection protocol." A connection is only reused for the same peer, same trust roots, same client key, and same virtual-host selection. `checkout_connection` rebuilds and validates the TLS config even on a pool hit "so a caller cannot borrow an authenticated connection with a key that does not match the supplied certificate chain" (`transport.rs:895-898`). Limits: 2 idle per key, 16 total (`transport.rs:29-30`).

### Virtual-host selection as a session prologue

Multi-tenant FOKS hosts require a `selectVHost(host_id)` call as the *first* call on a connection (Reg position 15, Merkle position 8, KV position 18, RT position 9). The client treats it as part of connection setup: `checkout_connection` sends it only when `next_sequence == 0`, i.e. on a fresh connection (`transport.rs:943-951`), and the pool key includes which protocol's selector was used. The README for the oracle notes fixtures "include the official virtual-host selection exchange."

### Timeouts, cancellation, retry

- Every operation has a deadline and a cancellation token, enforced by wrapping the TCP stream so reads/writes poll every 250 ms and check both (`OperationControl`, `transport.rs:52-93`, `ControlledTcpStream`).
- Chain loads are retried at most 3 times with a tiny exponential sleep (`10ms << attempt`) and a fresh probe/pin between attempts, but *only* for errors that mean "the Merkle root moved under me" (`retry_chain_load`, `crates/foks-client/src/auth.rs:1142-1166`; `retryable_chain_load_error`, `:1255-1272`). Everything else fails fast. `TX_RETRY_ERROR` (1014) is the server's generic "transaction conflicted, try again" and the client maps both `StaleRoot` and `TransactionRetry` to it.
- **Long polls are sliced.** KEX receive asks the relay to block for up to 30 s per RPC; Go's relay actually blocks 5 s and answers `TX_RETRY`, so the client loops until its 5-minute pairing budget is spent (`crates/foks-client/src/kex.rs:30-43`, `660-700`). The comment explains why slices are bounded: "a lost connection leaves at most one slice occupying server resources instead of the whole pairing wait." Realtime chat uses a dedicated poll connection with a 20 s connect budget and a 60 s poll budget (`crates/foks-client/src/realtime.rs:7-58`); it is `isolated_with_timeout` so ordinary requests never borrow a socket that is sitting in a long poll. On the server, `serve` (`session.rs:1342+`) recognizes `KexReceive`, `RealTimeRtPollInbox`, and `RegPollOAuth2SessionCompletion` routes and awaits them asynchronously rather than occupying a blocking worker (`session.rs:1490-1523`), and closes the connection promptly if the socket becomes readable mid-poll (which, since the protocol is request/response serial, can only mean a cancel or shutdown).

### References

- TLS 1.3: RFC 8446 (client certificate authentication in section 4.4.2).
- rustls: https://docs.rs/rustls (`ClientConfig::with_client_cert_resolver`, `WebPkiClientVerifier`).
- Ed25519 in X.509: RFC 8410 (OID 1.3.101.112).
- PKCS#8: RFC 5958.

---

## 8. Local IPC: the agent protocol

`crates/foks-agent-proto` is a different animal: it is the *local* protocol between the standalone agent and its frontends, not a FOKS wire format, so it prioritizes simplicity and bounded resource use over canonicality.

- **Framing**: JSON inside a 4-byte big-endian length prefix, capped at 1 MiB (`src/frame.rs:7`, `51-61`, `97-109`). Every message carries `version` (`PROTOCOL_VERSION = 33` as of 2026-10-01, `src/message.rs:4`) and a correlation `id` (`message.rs:69-73`); a response binds to a request ID and contains either a value or a stable `ErrorCode` (`message.rs:2309+`). `request_id` (`frame.rs:72-75`) can recover the ID from a length-valid frame even if the version is wrong, so a mismatched peer gets a targeted error rather than silence.
- **Sizing bytes in JSON**: byte payloads are base64 strings, not integer arrays, and the module doc computes why: a uniformly random byte costs about 3.57 JSON characters as an integer-array element but a fixed 1.33 as base64 (`src/base64_bytes.rs:1-20`). From this and a 32 KiB envelope reserve, the maximum KV payload per frame is derived as 700 KiB (`frame.rs:9-30`), and a test proves the worst-case envelope still fits (`frame.rs:154-189`). A payload past the cap is refused, never truncated (`frame.rs:191-212`).
- **Sensitivity**: encoded frames and decoded requests live in `Zeroizing` buffers; secret fields are redacted from `Debug`.
- **Build-time constants shared with TypeScript**: `build.rs` compiles `chat-limits.json` into Rust constants so the desktop UI and the agent agree on limits by construction.

---

## 9. Reading Go's on-disk state: `foks-go-interop`

This crate lets the Rust agent discover and (on macOS) import credentials from an existing Go FOKS installation. It is relevant here because it is another place where Rust must decode Go-produced Snowpack:

- The Go secret store (`~/.config/foks/foks-secrets` or the macOS equivalent) is decoded with `foks_snowpack::decode` into typed candidates (`src/lib.rs:359-456`), with size caps (16 MiB file, 256 candidates), `O_NOFOLLOW`, and a same-inode check to detect concurrent modification.
- Go's SQLite "hard state" is opened read-only with `trusted_schema = OFF` and row/value caps (`src/hard_state.rs:47-101`), and the scoped key is recomputed with SHA-512/256 and a type ID (`hard_state.rs:15`, `LOCAL_USER_INDEX_AT_HOST_TYPE_ID`) to cross-check that a row really belongs to the identity it claims.
- The macOS Keychain bundle is an XSalsa20-Poly1305 secret box whose 24-byte nonce is `be64(SECRET_KEY_BUNDLE_TYPE_ID) || 16 random bytes` (`src/lib.rs:21`, `179-181`), the same domain-separated-nonce convention as the wire crypto.

---

## 10. Signed compatibility leases: `foks-compat-artifact`

### Problem

Hosted CI "canaries" run destructive mutation and read tests against real FOKS services with the Rust implementation. Clients want to know "is the Rust implementation currently known to be compatible with this host?" without trusting a mutable web page.

### How it works

`crates/foks-compat-artifact/src/lib.rs` defines a small JSON document (`:22-38`) binding: a schema version (2), a strictly increasing `generation`, the target host, the run ID, `generated_at`/`expires_at` (at most 7 days apart, `:72-73`), the pinned protocol-metadata SHA-256 (tying the lease to exactly one extracted upstream contract), digests of the mutation and read results, an `outcome`, a set of `capabilities`, and a `drift_reason`. Validation encodes the policy structurally (`:91-104`): a `Compatible` artifact must have non-empty capabilities and no drift reason; a `Drift` artifact must have *empty* capabilities and a reason. So "a drift artifact is structurally unable to grant capabilities" (README), and applying one revokes the hosted mutation surface.

The signature is Ed25519 over `b"foks-hosted-compat-canary-v2\0" || serde_json(artifact)` (`:13`, `:149-155`); `key_id` is the first 16 hex chars of SHA-256 of the public key (`:121`). Note this uses JSON, not Snowpack, as the signed encoding, which is only safe because the *same* Rust code with `deny_unknown_fields` serializes on both sides; the domain string and schema version guard against cross-protocol reuse. The README describes the CI separation: untrusted third-party actions run in an unprivileged build job, the signing job has no action steps and read-only permissions, and publication receives only public bytes. Generation allocation (`main.go`'s `AllocateGeneration`) folds the CI run number and attempt into a value above the authenticated previous artifact so a key never reuses a generation.

---

## 11. Cross-cutting invariants and clever tricks (summary list)

- **One encoding per value.** Snowpack's minimality rules plus "no empty arrays, no floats, no maps except one-entry variants" give canonical uniqueness; property-tested (`foks-snowpack/tests/properties.rs`).
- **Signable is stricter than decodable.** `validate_signable` mirrors Go's checker (rejects `array16` of 16-31) so Rust never signs something Go would refuse (`decode.rs:261-267`).
- **Sign the delivered bytes.** Signed objects wrap inner encodings as opaque binaries (`SignedBlob`, `UserLink.inner`) so verification never depends on re-serialization; forward-compatible decoders keep unknown trailing fields *and re-encode them identically* (`foks-proto/src/lib.rs:379-440`).
- **Type-ID domain separation everywhere**: hashes, signatures, HMACs, and even AEAD nonces are prefixed with a 64-bit object type (`foks-crypto/src/lib.rs:708-767`).
- **Stacked signatures cover prior signatures** (`chain.rs:662-672`, `host.rs:58-80`).
- **Two codecs, one boundary.** The RPC envelope is general MessagePack skipped by a permissive cursor; the payload is validated Snowpack; the exact bytes cross the boundary untouched (`foks-rpc/src/lib.rs:2168-2203`).
- **Resequencing without re-encoding** keeps signed request bodies stable across connections (`lib.rs:265-305`).
- **Exactly-once RPC discipline**: increment the sequence only after a complete frame; invalidate on partial reads; peek before reusing an idle socket (`transport.rs:100-120`, `249-335`).
- **Headerless protocols detected from Go's generated code**, not documented by hand (`extract.go:338-382`), rendered as predicates keyed by protocol ID.
- **IDs are extracted by AST, pinned by hash, generated twice and compared** (`generate-protocol.sh`), with local extensions in reserved ranges so they can never collide with upstream (`foks-protocol-metadata/src/lib.rs:323-333`, `393-414`).
- **Semantic vs. raw source digests** turn upstream churn into a triaged drift report (`extract.go:168-226`, `drift.go:302-360`).
- **Fixtures are Go-certified**: written only after Go's canonical checker and verifiers pass; manifests carry hashes but no self-asserted booleans (`main.go:75-80`, README).
- **Both live directions are exercised** (Rust client on Go server, Go client on Rust server), opt-in via environment variables.
- **Bounded long polls** (KEX 30 s slices with `TX_RETRY`, realtime 60 s on an isolated connection) so an abandoned client costs the server at most one slice.
- **Frame budgets derived from encoding math** in the agent protocol (base64 ratio, envelope reserve), with a test that the worst case fits and an explicit refuse-not-truncate rule.
- **Status codes are one logical shape with two wire spellings** (positional Snowpack vs. named go-codec map), and the special codes (211 method-not-found, 8012 stale cache) carry structured payloads the client acts on.

---

## 12. Suggested chapter mapping and further reading

- Chapter on canonical serialization: Sections 1-3. Readers with a CS background will recognize the "canonical form" problem from XML Canonicalization (RFC 3076) and DER (X.690); MessagePack minimality is the same idea in a binary format.
- Chapter on RPC and framing: Section 4, with a sidebar on the msgpack-rpc lineage (MessagePack-RPC spec; Keybase framed-msgpack-rpc; go-snowpack-rpc).
- Chapter on keeping two implementations in lockstep: Sections 5-6. Useful external framing: "differential testing" (McKeeman, 1998, "Differential Testing for Software"), golden/characterization tests, and property-based testing (QuickCheck, Claessen and Hughes 2000).
- Chapter on transport security: Section 7 (RFC 8446; RFC 8410; the idea of a self-certifying host chain that delegates a TLS CA).
- Local IPC and CI trust: Sections 8, 10.

Items marked [VERIFY] above are external references I am confident exist but whose exact URLs, line numbers, or lineage claims I did not confirm from within this repository.
