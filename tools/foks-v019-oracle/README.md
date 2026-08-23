# FOKS v0.1.9 fixture oracle

This command uses the official FOKS v0.1.9 Go implementation to capture the
public probe response and emit exact canonical Snowpack objects for Rust
differential tests. It also emits the ordinary MessagePack RPC request frame
produced by the official client, which is intentionally not classified as a
canonical Snowpack object.

It requires only Go and network access. It does not need a FOKS account, local
agent, browser, PostgreSQL, or interactive input.

```sh
go run . \
  --host foks.app:4430 \
  --out ../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app
```

The command verifies the hostchain, public-zone signature, Merkle-root
signature, and the binding between the Merkle root and hostchain tail before
writing anything. It then confirms that every emitted object passes FOKS's
canonical MessagePack validator. The manifest records both the Merkle tree's
root-node hash and the independently computed prefixed hash of the full Merkle
root object for differential verification.

The live host can advance, so regeneration is an explicit review operation.
Checked-in fixture bytes are immutable test inputs; they are not regenerated
during ordinary Cargo tests.

An existing probe fixture can be re-verified without network access:

```sh
go run . \
  --host foks.app:4430 \
  --probe-file ../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp \
  --out /tmp/foks-v019-verified
```

This uses the same official host-chain, signature, canonicalization, and hash
code as live capture.

The same command can generate a self-contained authenticated-user fixture set
without an account, browser, or running server:

```sh
go run . \
  --host foks.app:4430 \
  --probe-file ../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp \
  --out /tmp/foks-v019-verified \
  --user-out ../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/user
```

It uses fixed device and PUK seeds to create an eldest link, provision a second
owner device, revoke that device, and rotate the owner PUK. It constructs and
officially verifies multi-level presence/absence paths, emits a real 995→998
Merkle skip-pointer transcript, boxes and officially unboxes PUK generation 2,
and records exact registration, Merkle-query, user-chain, and PUK RPC request
frames. Randomized commitments, nonces, and timestamps mean regeneration
produces a new valid transcript; checked-in bytes remain the stable
differential oracle.

The YubiKey/subkey matrix is also command-line-only and uses FOKS's mock PIV
bus; it never searches for or prompts a physical token:

```sh
FOKS_YUBI_FIXTURE_OUT=../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/user/yubi \
  go test -run '^TestGenerateYubiFixtures$' -count=1
```

It emits a mock YubiKey EntityID and hybrid HEPK, a delegated Ed25519 subkey
and its self-box, the three-signature PUK/subkey/Yubi eldest stack, and
software→Yubi plus Yubi→software PUK parcels. The official Go implementation
verifies both signature and unboxing directions before any fixture is written.
