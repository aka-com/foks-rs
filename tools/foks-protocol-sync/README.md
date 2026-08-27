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
