# Generated protocol metadata and upstream drift design

Status: design only. This does not change the pinned v0.1.9 contract or add Go
to any Rust build, test, or runtime dependency.

## Outcome

Replace manually repeated upstream facts with one checked, deterministic
metadata artifact generated from `github.com/foks-proj/go-foks`, while keeping
server policy explicit and reviewable. Continuously compare the pinned v0.1.9
artifact with upstream's resolved default-branch commit, but never silently
move the compatibility baseline.

The source of protocol truth remains the Go module pinned by
`tools/foks-v019-oracle/go.mod` (`github.com/foks-proj/go-foks v0.1.9` plus its
`go.sum` checksum). The useful upstream inputs are:

```text
proto-src/rem/{probe,reg,user,merkle,team,kv,realtime}.snowp
proto-src/lib/{status,config,*.snowp}
proto/rem/{probe,reg,user,merkle,team,kv,realtime}.go
proto/lib/{status,config}.go
```

The generated Go is an important cross-check: it contains the compiler-resolved
protocol unique IDs and every explicit/gapped `rpc.NewMethodV2` position. The
Snowpack source supplies source identities and content hashes. The extractor
must parse files as data; it must not run upstream generators, tests, init
functions, or server code.

## Ownership split

Upstream-owned metadata:

- protocol names and 32-bit unique IDs;
- method names and numeric positions, including gaps and historical spelling;
- status constant names and numeric codes;
- service-type enum names and values;
- referenced Snowpack source files and SHA-256 hashes;
- source module version, module sum, and resolved Git commit when available.

Local policy remains handwritten:

- listener assignment and authentication class;
- supported/unsupported decision;
- local request/result adapter name;
- per-method request bound;
- deliberately exposed status subset and local semantic aliases;
- implementation/test coverage identifiers.

Generation must fail if local policy names an upstream method/status/service
that does not exist. An upstream addition is reported but does not become a
supported Rust route by default.

## Proposed files

```text
tools/foks-protocol-sync/
  go.mod                         # extractor-only dependencies; publish never
  go.sum
  main.go                        # generate and semantic-diff subcommands
  internal/
    module.go                    # go list -m -json resolution and checksum data
    goast.go                     # generated-Go AST extraction
    snowp.go                     # source inventory and content hashes
    normalize.go                 # stable ordering and duplicate checks
    diff.go                      # compatibility classification
  testdata/
    generated/                   # small synthetic Go/Snowpack inputs
    expected/                    # golden normalized metadata and diff reports

crates/foks-protocol-metadata/   # no Go dependency
  Cargo.toml
  src/
    lib.rs                       # serde schema, validation, semantic diff types
  tests/
    pinned.rs                    # validates checked artifacts and policy merge

crates/foks-server/protocol/
  upstream-v0.1.9.json           # generated, sorted, checked in
  policy-v1.toml                 # handwritten local policy only
  upstream-v0.1.9.sha256         # artifact digest + module/sum/source digests
  mainline-diff.schema.json      # versioned report contract

crates/foks-rpc/src/generated/
  protocol_ids.rs               # generated constants only
  status_codes.rs

crates/foks-server/src/rpc/generated/
  routes.rs                     # merge of upstream metadata and local policy

tools/foks-server/
  generate-protocol.sh          # pinned, offline generation/check wrapper
  diff-upstream-protocol.sh     # networked mainline audit wrapper

.github/workflows/
  foks-protocol-drift.yml        # scheduled/read-only upstream audit
```

`protocol-v1.toml` can remain the human-facing complete contract, but becomes a
generated merge product rather than an independent input. The existing
`protocol_matrix.rs` test still compares it with `ROUTES` and `SERVICES` exactly.
It also compares both with the policy/upstream merge, preserving the current
enforcement while removing duplicate manual upstream numbers.

Generated Rust is checked in. `cargo build`, Bazel builds, normal tests, and the
released server therefore need only Rust and the checked artifacts. Go and
network access are limited to explicit regeneration/drift jobs under `tools/`.

## Pinned generation flow

1. Resolve the module exclusively through `tools/foks-v019-oracle/go.mod` using
   `go list -m -json`; require version `v0.1.9` and the checked module sum.
2. Parse the selected generated `.go` files with `go/parser` and `go/ast`.
   Recognize protocol-ID declarations, `rpc.NewMethodV2` calls, status constants,
   and service enums structurally rather than with regular expressions.
3. Inventory the corresponding `.snowp` sources, reject symlinks and files
   outside the resolved module directory, and hash exact bytes.
4. Cross-check every extracted method's protocol variable and position against
   the generated protocol handler map. Reject duplicate `(protocol ID,
   position)`, duplicate `(protocol, method)`, unknown expressions, integer
   overflow, or an extraction count change without an explicit golden update.
5. Serialize a schema-versioned, lexicographically sorted JSON artifact. Run
   the extractor twice in fresh temporary directories and require byte equality.
6. Merge with `policy-v1.toml`; generate Rust constants/routes and the complete
   `protocol-v1.toml`; run formatters.
7. Run `cargo test -p foks-protocol-metadata -p foks-rpc -p foks-server` and the
   existing Go-oracle fixture tests.
8. Regeneration CI runs the command and then `git diff --exit-code`. A developer
   cannot hand-edit a generated protocol ID, status, route, or manifest without
   failing the gate.

The extractor should initially cover only metadata needed by the current Rust
surface. Expanding it to full Snowpack type generation is a separate project;
handwritten canonical encoders/decoders and official byte fixtures remain the
wire-level authority until then.

## Mainline drift workflow

The scheduled job discovers upstream's default branch from the remote `HEAD`
symbolic ref and records the resolved commit SHA. It downloads that immutable
revision into an empty temporary module cache, parses it without executing code,
and compares normalized metadata with `upstream-v0.1.9.json`.

The report groups changes as:

- **wire breaking:** changed/reused protocol ID, method position, status value,
  service value, or changed source definition for a locally supported type;
- **behavior review required:** supported method/type changed while its numeric
  identity remained stable;
- **additive:** new protocol, method, status, field, or service value;
- **outside local slice:** change with no dependency path from local policy;
- **source-only:** comments/formatting changed and normalized metadata did not.

For every locally supported method the report includes its policy and test
coverage IDs. A changed dependency hash with no available structural detail is
classified as review-required, never source-only.

The scheduled workflow is informational: it uploads JSON and Markdown reports
and opens or updates one tracking issue on breaking/review-required drift. It
does not edit the baseline, generated Rust, `go.mod`, or lockfiles. Updating the
baseline is a normal reviewed change that regenerates fixtures, runs the live
Go audit, and states whether ordinary upstream clients can still interoperate.

## Tests and acceptance gates

- AST fixture tests cover hexadecimal IDs, explicit gaps, renamed Go symbols,
  comments, aliases, malformed expressions, duplicates, and overflow.
- Golden extraction from v0.1.9 matches every manually registered current
  protocol ID, method position, service value, and status code before manual
  constants are removed.
- Mutation tests alter one ID, position, status, source hash, and additive method
  and assert the expected drift class.
- Offline regeneration works with a prefilled module cache and no network.
- Normal Cargo/Bazel dependency graphs contain no Go tool, upstream source, or
  `foks-protocol-sync` runtime edge.
- The existing `protocol-v1.toml`/route-table exact comparison and route-coverage
  tests remain mandatory.
- A baseline bump cannot merge unless fixture manifests name the same upstream
  revision and the opt-in official Go compatibility audit passes.

## Deliberate limitations

This detects upstream protocol drift; it does not prove semantic compatibility.
Unchanged IDs can conceal changed validation or authorization behavior, so live
oracle tests and byte fixtures remain necessary. Mainline is inherently moving
and is never a production dependency. v0.1.9 remains supported until a separate
version decision and compatibility review explicitly replace it.
