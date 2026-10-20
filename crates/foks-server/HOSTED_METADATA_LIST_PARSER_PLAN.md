# Shared native List argument parsing

This is the next small implementation group in the single-transaction List plan.
It extracts the existing native List field/cursor decoder into foks-rpc so the
future guarded kernel can reuse the same public argument rules and status text.
It changes no public encoding, authorization rule, schema, endpoint or crypto.
Markdown stays unstaged; the implementation receives its own reviewed commit.

## Boundary and invariants

Add `arguments::KvListArguments` with owned fixed-width parent/cursor fields,
normalized u64 count, and load-small flag, plus
`decode_kv_list_arguments(&[u8]) -> Result<KvListArguments, RpcStatus>`.
Reuse the existing public `KvListCursor` enum. The decoder is explicitly only the
post-authority List-field decoder: it does not authenticate the ignored first
argument field. It is not an authorization capability or a new RPC route.

Move the behavior of services/kv.rs List's initial fields/parent/pagination/cursor
checks exactly, including these rules and their ordering:

1. Snowpack decode errors retain their existing displayed BadArguments message.
2. The outer value must be a three-field array; parent must be16-byte binary.
3. Pagination must be a three-field array with unsigned count and bool load flag.
4. Normalize count0 or above4096 to4096. Retain Time(u64), including u64::MAX.
5. Cursor is the exact existing positional union: None(0,nil variant), Mac(1,
   matching tag1 and32-byte binary), or Time(2,matching tag2,unsigned value).
   Preserve every existing failure string and which structural error wins.

Native dispatch must continue resolving authority before calling this decoder.
Keep `resolve_authority` unchanged, including its clock query position and current
whole-history/native authority behavior. Parent decoding/key permission still
precedes database Time-to-i64 conversion. Keep the native usize conversion at its
existing position after parent permission, even though the normalized count fits.
The other KV routes keep their existing helpers. Remove only the now-unused local
List cursor decoder, not generic fixed-width/fields helpers used elsewhere.

Do not extract KV authentication in this group. That parser currently calls the
clock between validating its outer auth array and interpreting token fields;
moving it wholesale could change clock-failure/error precedence. The later guarded
adapter needs a separately reviewed auth-resolution boundary and current SQL
credential checks. Sharing List fields alone is not full native status parity.

## Implementation and validation

Implement the pure decoder in a new foks-rpc arguments module and export it from
arguments/mod.rs. Native `list` calls it in the same post-authority position and
maps its cursor to the existing database enum without narrowing Time. No metadata
core dependency or Wasm export is added in this group.

Tests use explicit expected results/status strings, rather than another copy of
the implementation. Cover all valid cursor variants, normalization0/1/4096/4097/
u64::MAX, Time above i64::MAX, every shape/tag/type/width failure and combinations
that establish parent-before-pagination-before-cursor precedence. Include malformed
Snowpack and acceptance of an opaque first field to document the post-auth boundary.
Extend the existing raw authenticated List oracle's malformed-request matrix with
literal expected statuses so socket-versus-oracle equality is not the only assertion.
Retain missing-parent-before-large-Time and existing valid List byte comparisons.

Gates after author and independent source review: focused new parser tests, the
complete foks-rpc suite, existing raw authenticated List oracle and shared-domain
service comparison, strict all-target RPC/server/testkit lint with relevant feature,
Wasm foks-rpc compile/lint, and formatting/diff checks. Use the shared Cargo lock
protocol, incremental disabled/jobs2, exact worktree input refresh and fixed source
hashes. No celld rerun is needed solely for this native decoder extraction because
it changes neither the catalog nor the deployed Wasm; the eventual guarded export
must qualify this parser on actual Wasm separately.

Author plan review: this removes a duplicated future parsing boundary while keeping
native authorization ordering intact. The main misuse risk is calling a post-auth
field parser as authentication; docs/type naming and later guarded integration
must make that separation explicit. Keeping native auth resolution unchanged avoids
introducing a callback API merely to preserve clock-failure precedence. Fixed
expected status assertions and unchanged raw socket tests cover the extraction.
Independent plan review is pending; no source edits/tests have started.


Independent plan review (quota agent) PASS against native List/fields/cursor
source. Root implementation author review PASS on five frozen paths in isolated
list-parser-validation: native authority/generic fields remain byte-identical,
argument rules/messages unchanged, and raw RPC cases include authority-before-
pagination. Source manifest/hashes: /private/tmp/foks-list-parser-manifest.txt
and /private/tmp/foks-list-parser-sha256.json. Independent code review is pending;
no tests or Cargo builds have run for this group.


Completed as6662bcc8 after independent source review PASS and exact five-file
hash verification. All106 RPC tests (including four new parser tests), two native
List integration checks with22 raw authenticated RPC cases, strict all-target
RPC/server/testkit lint, and Wasm RPC lint/release compilation passed. No source
corrections were needed after tests. Tested on8ec3a044; ac9d3720's independent
schema fixture-only commit was integrated without overlapping source changes.
Evidence: /private/tmp/foks-list-parser-qualification.json. This is a shared
post-authority parser, not authentication or actual guarded Wasm execution.
