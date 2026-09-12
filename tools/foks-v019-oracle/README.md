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

The Rust server's two-link host-key rotation has a dedicated differential
gate. It generates fresh add and revoke probe blobs locally and asks this
pinned oracle to replay and verify each complete chain:

```sh
./run-host-rotation-compat.sh
```

The command needs no network service or account. It does require the locked Go
module dependencies and is run by the standalone FOKS CI workflow.

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
frames. Registration and Merkle fixtures include the official virtual-host
selection exchange and explicit HostID arguments. The PUK parcel carries an
officially sealed generation-1 seed-chain box beneath generation 2, and both
owner and member-role request encodings are recorded. Randomized commitments,
nonces, and timestamps mean regeneration
produces a new valid transcript; checked-in bytes remain the stable
differential oracle.

The user fixture command also creates a named-team eldest chain with the four
v0.1.9 PTK roles, authenticates it in a separate 995→996 Merkle transcript,
and boxes every PTK to the rotated owner PUK. It emits byte-exact view-token
challenge, activation, and team-load request frames; the challenge fields and
MAC are deterministic fixture values, and the official PUK implementation
produces the activation signature.

It also builds a deterministic team KV tree under the member-min PTK. The
tree contains a small file, symlink, and chunked file, and emits exact root,
directory, list, node, chunk, host-selection, and authenticated request
fixtures. It also emits an exact path-version vector, `kvCacheCheck` request,
and structured `KV_STALE_CACHE_ERROR` RPC response. All keys, MACs, boxes,
padding, and chunk nonces are produced by the official v0.1.9 implementation;
no browser, account, or running KV server is required.

The same fixture set covers every v0.1.9 KV mutation method used by the Rust
client: root/directory creation, dirent puts, small-object puts, large-file
upload initialization and continuation, and lock acquire/release. Mutation
objects use fixed fixture nonces so their byte-exact frames are reproducible.

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

The software-eldest registration substrate has its own fixture set so it can
be regenerated without replacing the randomized user/team transcript:

```sh
go run . \
  --host foks.app:4430 \
  --probe-file ../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp \
  --out /tmp/foks-v019-verified \
  --signup-out ../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/signup
```

This creates an exact two-signature eldest link, initial PUK box set,
reservation, hidden-location and commitment-key inputs, HEPK set, and byte
exact `reserveUsername`/`signup` RPC frames. It uses only the checked probe
fixture and the official Go crypto/protocol packages; it does not contact the
registration service or consume an invite code.

Exact device-provision, device-revoke, standalone PUK-rotation, and
single-owner ad-hoc-team request frames can likewise be derived from the
authenticated user fixture without a server:

```sh
go run . \
  --probe-file ../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp \
  --out /tmp/foks-v019-verified \
  --mutation-user-dir ../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/user \
  --mutation-out ../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations
```

The fixture reuses officially constructed provision/revoke links and a valid
PUK parcel, then constructs an official membership-preserving PUK rotation,
complete ad-hoc and named-team creation requests, and one named-team local-user
addition followed by removal with the exact two-role PTK rotation, recipient
boxes, historical seed chain, removal MAC, and TeamAdmin edit frame. It also
emits an independently randomized, officially replayed member demotion and the
complete TeamAdmin removal-key retrieval exchange: inert bearer-token request,
typed PTK signature, activation request, scoped box request, and returned
historical admin box. The same command also covers backup-key HESP derivation,
enrollment, signed account lookup, certificate request, PUK delivery, and
backup-countersigned permanent-device recovery. All of those steps use the
official Go v0.1.9 implementation and require no browser, agent, or live
account. Mutation randomness and link times are deterministic,
and the Go test suite generates the full corpus
twice to require byte-identical output. The generator opens the team eldest,
addition, removal, and creator membership through the official server-shared
validators, then checks
their team, host, owner, roles, PTK/box counts, and hidden-location bindings
against the RPC argument. The manifest records hashes and generator identity;
it deliberately carries no self-asserted verification booleans. A manifest is
only written after all of those checks succeed.

This exercises canonical signed bytes, arguments, RPC framing, and the
stateless validation used by `CreateTeamAdHoc`, including v0.1.9's retained
deprecated provision fields. It is not a live handler transaction: the reused
parcel makes the user-mutation request frames encoding oracles rather than one
coherent mutation against a Postgres-backed server. A full handler test still
requires the official integration environment, but never a browser.

## Live Rust compatibility test

The opt-in successful-flow harness builds the Rust client and starts the
unmodified official v0.1.9 Go integration environment with Postgres 17. The
full path drives account creation, authenticated user-chain and PUK loading,
personal KV initialization, a file write, incremental cache-check
synchronization, realtime chat, SQLite projection, and KEX. A focused chat path
creates a named team and channel, sends and decrypts text, synchronizes inbox and
read state, verifies its preview, and polls the Go realtime server:

```sh
./run-live-compat.sh
./run-live-chat-compat.sh
```

This requires a working Docker daemon because the official Go test environment
uses testcontainers for Postgres. It requires no browser or interactive input.
Ordinary `go test ./...` skips the live test unless `FOKS_RUST_LIVE_DRIVER` is
set, so the existing fixture suite remains command-line-only and hermetic.

## Realtime phase-1 fixtures

`TestRealtimeFixtures` generates deterministic synthetic identities, RT keys,
message/name/description boxes and the six initial generated-client RPC frames.
Ordinary execution compares immutable fixtures and opens the encrypted payloads:

```sh
GOPROXY=off GOSUMDB=off go test -run '^TestRealtimeFixtures$' -count=1
```

To intentionally regenerate, set `FOKS_RT_FIXTURE_OUT` to
`../../crates/foks-snowpack/tests/fixtures/foks-v0.1.9/realtime`.
The fixture seeds are public test data. No live account or server is contacted.
Rust crypto and RPC tests assert exact equality with these Go-produced bytes.

## MCP adapter comparison

`./run-mcp-compat.sh [all|rust|go]` runs the same independent stdio and Go-SDK
scenarios through Rust MCP against Rust and pinned Go services. The Go SDK also
unmarshals stat output into upstream `lcl.KVStat`. The Go service direction reuses
`TestRustClientHappyPath` setup and its ephemeral production-strength TLS leaf.
The Go MCP command is a reference comparator, not a peer of Rust MCP.

The unmodified upstream comparator is run with:

```sh
go test -C "$(go env GOPATH)/pkg/mod/github.com/foks-proj/go-foks@v0.1.9" \
  -mod=readonly ./integration-tests/cli -run '^TestMCP' -count=1
```

Ordinary `go test ./...` skips the opt-in Rust MCP process test unless its driver
supplies `FOKS_MCP_CLI` and `FOKS_MCP_STATE_DIR`. No real hosted account is required.

## Invitation compatibility

`run-invitation-compat.sh` runs the pinned Go test fleet (Docker/PostgreSQL) and
`foks-server-testkit/tests/invitations_live.rs`. It tests Rust local invitation
acceptance, duplicate-pending rejection, approval and PTK opening on Go; both
mixed-host directions for user and team applicants; and the unmodified Go RPC
SDK/core signing a local Requested link, granting scoped view permission, then
proving membership to Rust after administrator approval. Team applicants use
explicitly separated index ranges. The fixture accounts and their keys exist
only in private temporary test directories. This gate exercises SDK/core code;
it is not a claim that every interactive Go CLI invitation flow is covered.

For only the Go-client-to-Rust half, set `FOKS_GO_ORACLE_DIR` to this directory
and run this from the repository:

```sh
cargo test --locked -p foks-server-testkit --test invitations_live \
  official_go_invitation_client_against_rust -- --nocapture
```
The normal Rust suite skips external Go/Docker gates unless explicitly enabled.
`TestInvitationCertificateFixtures` remains the immutable wire/crypto gate and
records the pinned Go client's reversed verifier order for rotated certificates;
initial certificates work in the live gate. The existing `run-live-team-compat.sh`
remains a fixture/Rust conformance gate, not a substitute for this mixed-host gate.
