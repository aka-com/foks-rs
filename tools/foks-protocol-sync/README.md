# foks-protocol-sync

This standard-library-only Go tool parses generated go-foks source with
`go/parser` and `go/ast`; it never imports or executes upstream packages. It
extracts protocol IDs, method positions, statuses, service types, and exact
plus token-normalized Snowpack source hashes into a deterministic JSON
artifact. Discovery covers every generated remote protocol and every library
and remote Snowpack definition, including definitions outside the local server
slice. Its `diff` command
classifies a second artifact as wire-breaking, behavior-review-required,
additive, outside the local slice, or source-only drift.

A supported route in `policy-v1.toml` may list
`accepted_upstream_results`: result types that a newer upstream release
returns for that method and that the Rust client and server already handle.
`diff` reports a change to one of those types as `method_result_accepted`
(additive) instead of `method_result_changed` (wire-breaking). The pinned
artifact is not changed; regenerate it when a release carrying the new type is
tagged.

With `--structs-out`, `pinned` and `extract` also write a struct-shape
artifact: the positional field list of every generated library and remote
struct (the `XInternal__` types go-codec encodes as arrays). It is a separate
file, `crates/foks-server/protocol/upstream-v0.1.9-structs.json` for the
baseline, so the pinned protocol artifact and its digest do not depend on it.
Given `--baseline-structs` and `--candidate-structs`, `diff` compares layouts:
fields appended to an unchanged prefix are `struct_grown`, reported as additive
(`struct_grown_tolerated`) when the policy lists the struct in a
`[[tolerant_struct]]` table (its Rust decoder accepts appended fields; see
`crates/foks-proto/src/trailing.rs`) and as review-required otherwise; a
removed, renamed, retyped or reordered field is `struct_changed`. A
`[[tolerant_struct]]` entry that names no baseline struct is reported as
`tolerant_struct_unknown`.

Pinned regeneration is checksum-locked through `tools/foks-v019-oracle/go.mod`
and `go.sum`. Although the extractor itself targets Go 1.19, resolving the
pinned upstream module requires Go 1.25 or newer because that is the version
declared by go-foks v0.1.9:

```text
tools/foks-server/generate-protocol.sh --check
tools/foks-server/generate-protocol.sh --write
```

The networked mainline audit resolves an immutable commit from upstream's
remote `HEAD`, checks it out in a temporary directory, and writes JSON and
Markdown reports without changing the baseline:

```text
tools/foks-server/diff-upstream-protocol.sh --out-dir /tmp/foks-drift
```
