# First restricted hosted metadata profile — proposed, not implemented

The user explicitly chose to qualify a restricted hosted profile before pursuing
physical segmentation for the entire native range. Native behavior, formats and
limits stay unchanged. This document proposes an operator-visible hosted contract;
none of its limits or serving paths is enabled until plan/source/runtime reviews.
F1's unchanged full-native-range framing regression remains useful, but is no
longer a prerequisite for this first restricted profile.

## Source inventory and proposed limits

Pinned celld 0.6.2 `crates/celld/storage.rs:848` sets SQLITE_LIMIT_LENGTH to
2,200,000 bytes. This is an engine value/record limit, not a transport knob. Its
Cargo.lock pins libsqlite3-sys0.28.0; qualification must use the same official
binary already hashed in the celld evidence. Do not substitute the newer native
SQLite source as proof of the deployed engine implementation.

The native schema59 trace snapshot contains 99 FOKS tables, with at most fifteen
columns in a table. `/private/tmp/foks-restricted-profile-inventory.json` records
all table/column names and the source artifact hash. This is an inventory, not a
completed admission audit. Add the compiled private authority tables and the
exact platform schema separately; platform bookkeeping is engine-owned, not an
application write capability. The currently observed application expression
indexes are `kv_dirent_file_reference` (16-byte node-ID slice),
`team_members_identity` (nullable scoped host ID normalized to empty BLOB), and
`sso_single_host_policy` (fixed integer expression1).
Their bounds must be explicit, not lost by assuming every index is plain columns.

Proposed immutable profile `hosted-metadata-small-v1`:

| Dimension | Proposed ceiling | Relation to native behavior |
| --- | ---: | --- |
| Any application TEXT/BLOB value, measured in bytes | 1,048,576 | Native general blob/config and receipt limits may permit 16 MiB |
| Conservative encoded table record and each index record | 2,097,152 | Leaves 102,848 bytes below pinned celld's 2,200,000 engine limit |
| Parent exact directory | 1,048,576 | Native default remains 16,777,216 |
| KV node exact bytes (small files and symlinks) | 65,536 | Existing native bound retained; directories are separate records |
| Dirent exact bytes | 4,096 | Existing native bound retained |
| Peer certificate DER / serial magnitude | 4,096 / 20 | Existing hosted personal-auth contract retained |
| Scalar/page transport, logical List count/frame | Existing reviewed caps | Storage restriction does not silently lower public List4096 or change framing |

The effective field limit is the minimum of its existing semantic limit and the
new hosted ceiling. TEXT uses encoded UTF-8 bytes, never character count. Only the
compiled schema's actual types/nullability are admitted. UTF-8 database encoding
is part of the profile/connection contract; unsupported encoding fails activation.
The profile and its limit/schema/index descriptor digest join the compiled profile
and artifact activation identity. Operators must be able to inspect the exact
profile before import or deployment; a generic "SQLite-compatible" label is
insufficient.

## Complete-record bound, including indexes

SQLite records have a header containing a varint header length and per-field
serial-type varints, followed by field payloads. Strings/BLOBs contribute their
byte lengths; integer/real payloads occupy at most eight bytes. The length limit
also covers complete encoded rows. These stable format facts are described by
SQLite's [record format](https://www.sqlite.org/fileformat2.html#record_format)
and [length-limit documentation](https://www.sqlite.org/limits.html).

Use a checked conservative formula, not sum-of-BLOB-lengths alone. For each stored
record with k fields, define payload(NULL)=0, payload(integer/real)=8, and
payload(TEXT/BLOB)=byte length. Require every field's own limit, then:

`R_upper = 9 + 9*k + sum(payload(field)) <= 2,097,152`.

Nine bytes bound each serial-type varint and the header-size varint. Charging
integers as eight bytes also covers compact special encodings. This intentionally
rejects some records the engine could fit; it is an explicit conservative hosted
contract, not a claim of exact minimal SQLite serialization. B-tree page headers
and overflow-page pointers are separate physical storage accounting and are not
silently counted as part of the SQLITE_LIMIT_LENGTH record value.

Generate a versioned descriptor for every table record and every physical index
record, including implicit unique/primary indexes and rowid or WITHOUT ROWID
primary-key suffixes. Evaluate indexes independently; repeated columns or
expressions must not be assumed to fit because the table record fits. The three
known expression forms get explicit typed bounds: substr(node_id,2,16) at16 bytes,
coalesce(scoped_host_id,X'') at most33 bytes, and the fixed integer expression
(1) in sso_single_host_policy at most8 bytes. Each requires an exact descriptor
match. Any additional expression, generated column,
virtual table, unrecognized suffix or schema drift fails profile qualification
until separately reviewed. Do not infer the descriptor from caller-supplied SQL.

The pure bound helper accepts borrowed typed field dimensions plus a compiled
record descriptor and returns a structured violation without copying field bytes.
All arithmetic is checked, including UTF-8 byte lengths and index suffixes. A
native development generator may inspect exact canonical schema metadata; the
runtime uses reviewed immutable descriptors tied to the schema/profile hash.
The helper does not authorize a write or stand in for SQL transaction ownership.

## Existing database, import, backup and restore admission

Qualification scans all application history and side tables, not just current
heads or the tables reached by the first List request. Old receipts, retired
certificates, chain history, inline chunk ciphertext and log blocks can exceed
these limits even when current KV rows fit. On a native private copy, enumerate
rows with deterministic keysets and type/byte-length projections before copying
payloads; compute each table/index record bound. Record a bounded diagnostic
(table, field/record class, observed dimensions, required limit and a bounded
opaque row locator) without plaintext/ciphertext dumps. Report the first bounded
set of violations and total examined counts. Do not scan under a live serving
connection or treat a partial scan as readiness.

Only a quiesced, schema-verified source and a completed private-copy audit may
produce an import manifest. Bind the source identity/incarnation/revision, schema,
profile descriptors, exact artifact hashes and completion result. A source change
invalidates admission. Migration occurs on the private copy first, then the
current canonical schema and all row/index bounds are checked. No implicit
truncate, split, rewrite, re-encrypt, drop-history or metadata segmentation occurs.
A rejected source remains usable with its native backend.

The hosted destination is private and unservable until all import writes, full
schema/data admission, authority/dependency incarnation establishment, checkpoint
publication and required sync finish. Resume only the same manifest/profile;
otherwise restart the private import. Existing celld state is likewise unready
until its full admitted inventory is verified; engine-limit query failures are
failures, never evidence that the absent rows were safe. Do not load a raw native
SQLite snapshot as an already-admitted hosted database. Restore, restore-to-new,
backup hydration, clone/promotion and rollback recovery must all use this gate;
current native `standalone.rs`/`operations/backup` behavior remains unchanged.

## Writer and query coverage required before route enablement

First qualify the bound helper, existing-data scanner and typed personal-KV slice.
This does not claim that all server routes have a shared hosted implementation.
The profile manifest must enumerate enabled operations and reject every unported
or unaudited route before effects. Every enabled mutation validates complete
post-image rows and generated index records in its owning SQL transaction before
publication. Exact retries preserve existing durable-receipt behavior; a new
mutation cannot bypass profile checks by calling a maintenance or replay helper.
Derived trigger rows must have explicit fixed bounds or be included in the
post-image checks. Unknown raw SQL/writers remain unavailable.

The source coverage ledger must explicitly include these groups:

| Group | Source/operation ownership to audit |
| --- | --- |
| Personal KV | server-db `kv.rs`, shared metadata PutDirents, root/directory/node/upload/chunk helpers; namespace creation hidden in reads/preflight; projections, Usage counters and dependency triggers |
| Identity and key material | identity/user_mutation/generic/team/passphrases/yubi/host/host_rotation modules; chains, parcels, boxes, certificates and bootstrap/root records |
| Auxiliary mutable routes | SSO, realtime, invitations/capabilities, web-admin/audit, logs/waitlist; receipt/outcome insertion and retention |
| Maintenance and storage accounting | upload cleanup, chunk binding/publication/GC ledgers, cursor updates, retirement, authority clock floors/epochs; every trigger/cascade destination |
| Import/restore/control | private-copy migration, backup hydration, standalone restore, promotion, reset/recovery and any trusted controller writes |

For the first personal-KV writer group, full post-images include all existing
unchanged variable fields, not only the submitted changed field. An UPDATE that
combines individually acceptable values into an oversized row must reject before
commit and preserve head/receipt/quota/dependency state according to the existing
transaction contract. Multi-statement mutations and trigger errors roll back the
same scope; no remote object publication may be made irreversible before its
existing durable protocol and profile admission are satisfied.

Hosted bulk-file qualification must explicitly choose reviewed external chunk
storage or reject over-profile inline rows; it cannot silently externalize an
imported inline row. Native log blocks allow4MiB and therefore are a concrete
restricted-profile rejection. General16MiB chain/box/receipt/directory payloads
and rows with multiple near-limit values are other explicit rejected native-valid
records. A1/A2 transport may impose narrower operation limits than physical row
fit; a storage-profile pass never automatically enables an opaque-value read
whose decoder/transport is not qualified for that dimension.

Every enabled SQL statement also needs a projected/intermediate-record audit:
joined values, sorting/temp records, concatenation, JSON or RETURNING data may
exceed limits even when each stored row fits. Existing List pages and parent slices
stay bounded through their reviewed descriptors. Unsupported statements do not
receive an exemption merely because the final host response is small.

Activation violations are operator-visible profile errors and prevent readiness.
Proposed new-mutation mapping uses the existing quota/size failure class, retaining
exact existing status/error ordering where applicable; the concrete per-method
mapping is a route source-review gate before enablement, not a new wire enum.
Post-activation invariant corruption fails closed and invalidates readiness rather
than being returned as missing data or silently omitted.

## Qualification and ownership split

1. Storage owns this row/value contract and source inventory, then a pure checked
   dimension helper and native private-copy scanner proposal. Freeze complete
   table/index descriptors and all exceptions before code. Root and recovery
   independently review the plan and source before tests.
2. Recovery owns profile/artifact binding and activation integration after the
   descriptor/manifest interface is frozen. Root owns trusted native adapter,
   restore/controller integration and operation-enablement ledger. No ordinary
   serving switch is implied by the fixture work.
3. Native tests cover exact limits +/-1, multibyte TEXT, NULL/integer/real shapes,
   serial-header boundaries, multiple near-limit values, index expressions/suffixes,
   overflow, historical-only violations, source change and interrupted import.
4. Actual pinned celld tests insert accepted boundary records and deliberately
   native-valid rejected records, preserve rollback/state parity, and independently
   observe engine rejection above2.2MB without raising limits. Include empty/multi-
   value rows and index records; a successful single-BLOB test is insufficient.
5. Run real-client compatibility for accepted records and explicit operator/RPC
   rejection for the documented restricted cases. Read/write/restore coverage,
   connection ownership/configuration, fencing, durable outcomes, deadlines,
   memory and capacity remain separate required gates. C3 provides inspected
   runtime prerequisites, not complete readiness by itself.

No segmentation implementation is required to qualify this explicitly restricted
first profile. Expanding its limits or adding an operation is a new reviewed
profile/artifact version with renewed inventory and runtime evidence.

## Review corrections before descriptor implementation

Root high-level review passed; independent review requires these explicit gates.
The full-source freshness authority is a quiesced, immutable, complete SQLite
backup/checkpoint artifact and its required companion artifacts, with their exact
hashes. Never hash main.db alone while a WAL contributes state. Schema and
metadata_read_dependency incarnation/revision are supplementary bindings only:
that revision covers24 tables and cannot fence all99 application tables, logs,
receipts or side tables. Every audit chunk reads that same immutable artifact.

Rowid tables use bounded integer rowid keysets. WITHOUT ROWID tables use exact
primary-key tuples with the declared collations and ordering. Before constructing
a cursor, probe all locator types and byte lengths; oversized, malformed or
unorderable keys stop with an explicit violation/incomplete result. No oversized
key copy, skipped row or falsely completed scan is allowed. Compile the precise
locator descriptor with each table descriptor before implementing the scanner.

The operator-facing profile also publishes canonical intent, batch and outcome
ceilings. A public count limit of4096 remains unchanged, but a batch whose
canonical intent or durable outcome exceeds the1MiB value/2MiB record profile
can be rejected even when every individual row would fit. Freeze exact per-route
limits, status precedence and retry behavior with the enabled-operation ledger.

## Concrete implementation sequence (each separately reviewed)

R0 freezes generated descriptors from trusted fresh schema59, separate authority
and pinned platform artifacts. Enumerate table_xinfo/index_list/index_xinfo,
autoindexes, partial predicates, all three expressions, and WITHOUT ROWID suffixes.
Include allowed types/nullability, effective field caps, exact locator collations,
index operands and schema/profile digests. Runtime never derives its expected
contract from the untrusted target.

R1 implements only the pure borrowed-dimension helper: checked9+9k+payload,
wrong type/count rejection, then field and complete table/index limits. Tests
include UTF8/NUL, header boundaries, suffixes and all expressions. Two1MiB values
already exceed the2MiB conservative row ceiling after record headers are added.
No writer or readiness capability is granted by this helper.

R2 separately freezes the native immutable-private-copy scanner: bounded type/
length projections, exact cursors, diagnostics and interruption state over all
history. Resume only the same finalized artifact and descriptors; partial scans
never create readiness. R3 owns import/readiness integration, and R4 owns each
enabled route's post-images, trigger/cascade and query-intermediate closure.
Current scalar transport limits (including4096-byte values on relevant contracts)
remain independent; a storage fit does not qualify their transport or decoder.

## R0 trusted descriptor generator — concrete proposal for review

R0 produces a development-only, reproducible inventory and storage-dimension
contract. It does not implement the R1 bound helper, scan an imported database,
install authority, change native limits or enable a hosted operation. The first
implementation should remain in test/development modules of metadata-core, which
already has a development dependency on server-db; no new runtime dependency from
server-db to metadata-core is needed.

### Trusted inputs and actual layouts

Use the existing `read_schema/tests/generator.rs` provenance pattern: create a
fresh native Database with the current schema, open a read-only connection to
that private generated database, and require the existing schema59 canonical
contract hash. Create private qualification authority DDL in a separate in-memory
connection. Load the exact ten pinned platform DDL objects in another connection;
platform rows are explicitly engine-owned and excluded from the application
storage contract. R0 must neither accept a target database path nor learn expected
DDL, expression definitions, policy limits or schema hashes from target state.
The output directory is the only external generator argument and must be new.

Read-only inventory of the existing trusted native fixture gives99 FOKS tables,
189 `index_list` entries, and five WITHOUT ROWID tables. The latter primary-key
indexes are aliases for the table b-trees, not five additional physical records;
the current FOKS physical record inventory is99 tables plus184 secondary/rowid
indexes, or283 descriptors. All observed index collations are BINARY, maximum
column count is15, maximum primary-key width is8, and maximum index_xinfo width
is13. There are97 STRICT tables and two non-STRICT tables:
`chunk_storage_binding` and `chunk_storage_revision`. Their declared types still
receive explicit restricted-profile type admission; non-STRICT does not mean
arbitrary values are admitted. These counts are reviewed snapshot observations,
not permission to silently accept later additions. The auxiliary source inventory
is `/private/tmp/foks-restricted-index-layout-inventory.json`; its Python engine
version is recorded and is not a deployed celld compatibility proof.

The fresh Rust generator independently enumerates `table_list`, `table_xinfo`,
`index_list`, and `index_xinfo`. Fetch exact sqlite_schema SQL for explicit
objects and link automatic indexes back to their owning table and origin.
Require exact coverage of the trusted component's objects, duplicate-free names,
known declared types, no virtual/generated/hidden application columns, no
unrecognized expressions, and no unexplained physical index fields. Unknown
layouts stop generation for review rather than inheriting a generic rule.

### Descriptor shape and physical record mapping

Use test-only typed DTOs initially; runtime borrowing/static arrays are a later
R1 integration decision. Stable IDs are assigned by canonical component/table/
index names, with no dependence on PRAGMA enumeration order. Each component and
manifest has a version and independently hashed source provenance.

A table descriptor includes component, exact name, schema object digest, STRICT
and WITHOUT ROWID flags, ordinal columns, physical table record order, and locator
kind. Each column includes declared type, allowed runtime storage type, effective
nullability, primary-key ordinal, any integer-rowid alias, and an explicit byte
policy source. Global BLOB/TEXT limit1MiB is the default; reviewed narrower
storage policies include exact_node64KiB and exact_dirent4KiB. Existing SQL CHECK
and protocol semantic validation remain separately required. R0 does not pretend
to parse arbitrary CHECK expressions or prove every semantic constraint. A
manually declared override is a reviewed exact table/column match and is rejected
if its target disappears or changes type. Every column gets a policy; an absent
mapping cannot silently mean unlimited.

A record descriptor is an ordered sequence of operands, not a set of column
names. Operands are `Column(ordinal)`, `SyntheticRowid`, or one of the three fixed
expression identities. Include all auxiliary `index_xinfo` fields, even when
`key=0`; those suffixes are part of the index record. Preserve ordering,
descending flags and collations. An ordinary rowid-table record uses declared
column order; its INTEGER PRIMARY KEY alias may be encoded as NULL in the actual
record, but counting its admitted integer as8 bytes is an explicitly conservative
bound. Rowid index suffixes are charged as8 bytes before a real rowid is assigned.
WITHOUT ROWID table physical order comes from its PK index_xinfo, including
non-key payload columns; that PK object is recorded as a table alias instead of
being double-counted. Secondary index suffixes come directly from index_xinfo,
with no guessed duplicate-column elimination. PK not-null semantics are recorded
explicitly, including when `table_xinfo.notnull` alone is insufficient.

Every partial index retains its exact predicate/SQL digest. The first restricted
contract conservatively checks its record bound for every row, even when the
predicate would omit that row. This avoids an unreviewed SQL predicate evaluator;
it may reject additional native-valid rows and is part of the published profile.
R0/R1 must not claim it computes exact on-disk byte size. Expanding acceptance by
conditional predicate evaluation would require a later reviewed descriptor
version. Every complete table and index record uses the already reviewed
`9 + 9*k + payload` upper bound with checked arithmetic; SQLite b-tree page
bookkeeping remains separate from the record-length engine limit.

The expression registry has exactly these entries, matched against exact trusted
DDL and the index_xinfo expression slot:

- `kv_dirent_file_reference`: BLOB `substr(node_id,2,16)`; dimension is at most16,
  with exact bounded length computed from the admitted source dimension.
- `team_members_identity`: `coalesce(scoped_host_id,X'')`; NULL maps to an empty
  BLOB, otherwise use the admitted BLOB dimension, at most33 under this explicit
  field override. No copy or JavaScript decoder is involved.
- `sso_single_host_policy`: fixed integer1; conservative payload charge8.

Missing, reordered or newly added expression slots fail generation. Collation
and suffix matching is as important as source column identity. A SQLite version
change that alters PRAGMA layouts requires reviewed regeneration and actual
engine qualification; the generator records its native engine version, and the
profile separately binds the pinned celld build.

### Locator descriptors and admission implications

Rowid tables use an unshadowed, explicitly recorded hidden rowid spelling and
signed-i64 keysets. Validate that the chosen spelling is not a declared column;
all current tables have an available spelling. WITHOUT ROWID tables use their
exact PK tuple, directions and collations. Only BINARY is supported by this first
version; a new collation fails generation. A future scanner must type/length
probe locator fields before copying, validate the row/profile bound, and then
retain only one bounded cursor. A malformed or oversized PK stops with a bounded
violation/incomplete result. It cannot be skipped to produce a false completed
scan. The native R2 locator budget may cover the profile's full2MiB record bound;
it must not silently impose the private scalar transport's4096-byte field cap.
Precise cursor memory/query APIs remain a separate R2 plan gate.

### Outputs, resource limits and version binding

Proposed output files are `foks-records-v1.json`, `authority-records-v1.json`,
`engine-ownership-v1.json`, and `storage-descriptors-manifest.json`. Authority
output is explicitly labelled qualification-authority-v1; synthetic principals
are not a production permission path. Engine output contains exact pinned DDL
identities and the engine-owned exclusion classification, never an application
write capability or a claim that platform bookkeeping rows were audited.

The manifest binds source DDL/contract hashes, exact generator format version,
profile limits and overrides, descriptor file hashes, native SQLite version,
record/table/index/alias counts and scope `trusted_descriptors_only`. It excludes
its own hash to avoid cycles and contains no timestamps, random database IDs or
source filesystem paths. Existing schema-profile296-byte activation descriptors
and catalog artifacts remain unchanged in R0. A later activation version must
explicitly bind this storage contract; generating these files alone is not an
admitted profile.

Use canonical typed JSON with fixed field ordering, sorted records and exactly
one terminal newline. Output limits are explicit: at most256 application tables,
1024 physical records,32 columns/operands per record,256 UTF8 bytes per identifier,
64KiB per DDL object and8MiB per generated component artifact. Checked aggregate
counts precede allocation/serialization where relevant. These are development
artifact limits, not a runtime memory allowance. Write with create-new semantics,
reject existing destinations and sync files before declaring generation complete.
Do not overwrite a reviewed descriptor artifact on a failed regeneration.

### Ownership, tests and source gates

Storage owns new development files under
`crates/foks-metadata-core/src/read_schema/tests/restricted_generator/` and the
minimal test-module declaration. Keep the existing trusted-source generator,
scalar/page catalog encodings and their hashes unchanged. Freeze exact proposed
paths before implementation; generated outputs are qualification artifacts first,
not silently installed runtime constants. Root/recovery review this plan and the
source before any test or regeneration command. Cargo stays with the assigned
owner until an explicit handoff.

Planned tests regenerate twice from separate fresh databases and require exact
bytes, current schema golden,99/189/5/283 inventory counts, all three expressions,
and exact file hashes. Targeted private miniature schema fixtures cover integer
rowid aliases, INTEGER PRIMARY KEY DESC non-alias behavior, shadowed rowid
spellings (select an unshadowed alias or reject), autoindex origin, composite
WITHOUT ROWID physical order and
secondary suffixes, descending keys, partial-index conservative classification,
NULL coalesce, and all expression slots. Rejection fixtures cover unknown or
changed expressions, generated/virtual columns, unknown types/collations,
identifier/output bounds, missing/duplicate components, and existing output
paths. Each fixture tests the descriptor mapping, not merely reproducing the
implementation's arithmetic. Compare selected expected physical layouts directly
with SQLite's reported index_xinfo rows, with independent hand-written expected
operands for the three expression cases.

R0 does not yet insert boundary values or claim cross-engine row-size behavior.
R1 independently qualifies arithmetic and dimensions, R2 qualifies complete
immutable-copy scans, and later actual celld tests prove accepted record/index
boundaries and rejected native-valid records under the unchanged engine cap.

R0 independent plan review passed (recovery), with the explicit miniature
INTEGER PRIMARY KEY DESC and shadowed-rowid cases above. Alias detection must
use actual table/index metadata, not assume that the declared INTEGER type
alone makes a primary key a rowid alias. Root review remains pending.

R0 root plan review passed with two explicit refinements. Effective nullability
must distinguish explicit NOT NULL, STRICT primary keys, WITHOUT ROWID primary
keys and genuine INTEGER rowid aliases from ordinary nullable primary-key
columns in non-STRICT rowid tables. Add a non-STRICT TEXT/composite nullable-PK
fixture; a PRIMARY KEY label alone cannot mark every field non-null. Field
policies match the exact `(component, table, column)` triple: in particular,
`kv_nodes.exact_node` is64KiB and `kv_dirents.exact_dirent` is4KiB; a same-named
`merkle_nodes.exact_node` must retain its separate global profile policy. Include
an explicit no-cross-table-override regression.

Concrete R0 source ownership is six paths: existing
`crates/foks-metadata-core/src/read_schema/tests.rs` (native-only module declaration)
and new files under `src/read_schema/tests/restricted_generator/` named `mod.rs`,
`descriptor.rs`, `introspection.rs`, `policy.rs`, and `tests.rs`. Implement in an
isolated worktree based on committed F1; do not edit existing generator.rs or
runtime profile/SQL/catalog modules. Root and independent source reviews precede
all compilation, tests and artifact generation.

R0 SQLite-owned refinement passed root and independent review: the fresh schema
contains `sqlite_sequence` because admin_audit uses AUTOINCREMENT. The ownership
artifact separately inventories exact main-schema internal objects, labels this
table's data unadmitted for raw import pending R2/R3, links automatic indexes to
physical descriptors, and classifies platform automatic indexes with their exact
pinned platform owner. Statistics absence is explicit; unknown internal objects,
TEMP objects and attached namespaces fail generation. This is not a wildcard
exclusion or a claim that99 application tables cover SQLite-owned data.

The author/root source audit also corrected the WITHOUT ROWID PK alias distinction:
its index_list entry has no sqlite_schema row at all. R0 requires that absence
for only the verified WITHOUT ROWID/PK alias; normal automatic indexes must have
a real schema row with NULL SQL. The miniature fixture checks both cases.
`kv_nodes.exact_node`64KiB covers both small-file and symlink nodes; it is neither
a directory limit nor a general same-column-name metadata limit.

R0 source freeze at baseline `4842499c`: six paths in
`/private/tmp/foks-restricted-descriptors-manifest.txt`, hashes in the matching
`-sha256.json`, isolated `metadata-restricted-descriptors-validation` tree.
Author audit and root/recovery independent source reviews passed before tests.
The only runtime-facing file change is a native-only test-module declaration;
all generator/descriptors/policy code is compiled under tests. Native validation
and two create-new artifact exports began after that gate; results are pending
at this entry. No generated artifact is an admission capability.

R0 native qualification passed: focused9 tests, full metadata155 tests (two
explicit exporter tests ignored in the ordinary run), strict native all-target
Clippy, and two separate one-test exports to fresh directories. The sole lint
correction was reviewed `max(256).min(8MiB)` → `clamp(256,8MiB)`, preserving the
same growth bounds; original lint evidence and pre-correction hashes remain.
No descriptor semantics changed. Final artifacts from two independent processes
are byte-identical under native SQLite3.53.2:99 application tables,189 index-list
entries, five WITHOUT ROWID aliases and283 physical records; private authority
adds three tables/four physical records. Frozen evidence is
`/private/tmp/foks-restricted-descriptors-qualification.json`; artifact roots are
`/private/tmp/foks-restricted-descriptors-a` and `-b`. Final six source hashes
match the reviewed freeze after the one-line correction. Root commit is pending
at this entry. No celld record boundary or imported-data admission is claimed.


R0 was committed as `87c0f97e` after root verified all six source hashes and both
exports. The qualification ledger now binds that commit; the original failed
lint log and pre-correction source hashes remain available. All Markdown stays
unstaged. The next R1 proposal below is separate from the completed R0 slice.

## R1 concrete pure UTF-8 record-bound API — proposed, review required

R1 installs a reusable, allocation-free runtime helper in
`foks-schema-contract::storage`. Both metadata-core and server-db already depend
on this crate, so R2's scanner and later typed postimage writers can call exactly
the same implementation without a dependency cycle. This slice grants no database
admission, transaction, authorization, publication or serving capability. It does
not change the existing296-byte profile format, SQL catalog or native limits.

### Fixed descriptors and reproducible generated Rust

The input authority is the committed R0 trusted generator, not a target database.
Freeze the four R0 output SHA-256 values from
`/private/tmp/foks-restricted-descriptors-qualification.json`: FOKS
`7addbf7e7b75d33908036c9b7280b246c4a5c2b6a65508ba277199be7c139df2`, authority
`0de8a187678b889de5d7b9c4552aed8e44d7fedf1be5f849180d9c5e78bbf518`, engine inventory
`b96142c9c163ccf9f0dda87e3adda2fdf64d2c2b4ace94614c0074d5e5ed378e`, and manifest
`6c3628d4acd708b519150e7f4a6f6255adb185c97973ce42da60724fb35203e2`.

A new native development-only renderer consumes the same trusted R0 Component
values after verifying these exact four encodings. It emits one checked-in Rust
module, `foks-schema-contract/src/storage/generated.rs`, using deterministic
ordering/escaping, borrowed static arrays and fixed-width integer literals. No
runtime JSON parser, build-time reference database or external crate-file
include is introduced. Keep R0's four existing output bytes unchanged. A separate
ignored exporter writes the Rust source to a fresh destination with the existing
bounded/create-new/fsync discipline; it never replaces a file in place.

Generated source contains profile/version/UTF8 requirement, all four provenance
hashes, the FOKS/authority schema hashes, exact column policies, table and physical
record layouts, index SQL hashes/partial policy/alias links, and locator
collations/directions. Include the exact SQLite-owned/platform classification as
read-only inventory, without giving their rows application TableIds. Its source
format has a separate version. Generation rejects missing/duplicate/orphan
references, unknown operand kinds and any loss of table/index/locator metadata.
The public provenance identifies this version plus the frozen R0 hashes; a later
activation artifact must pin its own compiled binary and any final profile digest.
No self-hash is embedded in the generated source.

Tests regenerate the source twice from independent fresh trusted sources and
compare exact bytes with the checked-in module using a native test-time filesystem
read. This is not a compile-time include across crate packaging boundaries.
Additionally compare every compiled descriptor field with the R0 object graph,
including all283 FOKS and four authority physical records and five alias links.
The source writer is an explicit development tool, not a new parameter allowing
runtime callers to supply expected layouts. It has no production constructor.

### Exact public types, inputs and lifetimes

Expose `HOSTED_SMALL_V1: StorageProfile` with private fields and read-only methods.
`Component` is the closed enum `Foks | Authority`. `TableId` and `RecordId` are
small Copy opaque IDs with private constructors. `profile.table(component, name)`
returns an optional ID; descriptor access accepts only these pinned IDs. No API
accepts caller-created descriptors, SQL or a target-derived default. Descriptor
getters return static borrowed views, including locators for R2. Internal tests
may construct miniature descriptors through a private test-only seam.

The proposed public checking surface is:

```rust
pub enum Dimension {
    Null, Integer, Real, Blob { bytes: u64 }, TextUtf8 { bytes: u64 },
}
pub enum BorrowedValue<'a> {
    Null, Integer(i64), Real(f64), Blob(&'a [u8]), TextUtf8(&'a str),
}
pub fn check_utf8_row_dimensions(
    table: TableId, values: &[Dimension],
) -> Result<RowBounds, Violation>;
pub fn check_utf8_row(
    table: TableId, values: &[BorrowedValue<'_>],
) -> Result<RowBounds, Violation>;
```

Values use table column ordinal order, not physical record order. Exact column
count is mandatory. The borrowed convenience path derives dimensions without
copying data; both paths use the same private evaluator and fixed stack scratch
of at most32 dimensions. Inputs are borrowed only for the call, never retained.
Result and error are Copy, bounded and allocation-free. `RowBounds` exposes the
table record bound, maximum index bound plus its optional RecordId, and physical
records checked. It is a measurement, not a proof object. `Violation` carries a
closed reason (count/type/nullability/field bytes/record bytes/arithmetic overflow),
the relevant ordinal or RecordId, and numeric observed/limit values where defined;
it carries no arbitrary input strings or payloads.

Database dimensions are a caller assertion: R2 must obtain them from guarded
SQL type/byte-length probes on the immutable admitted-encoding source. A forged
Dimension can yield a meaningless successful measurement and cannot authorize
any operation. Typed R4 postimages should use borrowed actual values where possible.
No constructor takes a connection, revision or artifact hash and pretends to bind
the measurement to that source.

### Evaluation, limits and deterministic errors

Validate exact count first, then all columns in ordinal order: storage type,
nullability, then per-field byte cap. NULL is allowed only by R0's effective
nullability. Require exact storage class; do not apply SQLite affinity coercions.
A REAL column's observed INTEGER storage is rejected unless the frozen descriptor
explicitly permits it; R1 does not infer new accepted classes. Input represents a
completed postimage: a rowid alias auto-assigned from an INSERT NULL must already
be represented as Integer. NULL is not silently treated as generated identity.

After field checks, evaluate the table record then each distinct physical index
in canonical index-name order; WITHOUT ROWID PK aliases do not duplicate the table
record. Every partial index is checked conservatively for every row, independently
of its predicate. This can reject rows whose partial index would be absent and is
part of the stated restricted profile, not accidental native behavior parity.

Every addition/product uses checked u64 arithmetic. The bound remains
`9 + 9*k + sum(payload_upper_bounds)`: NULL0, integer/real8, TEXT UTF8/BLOB exact
byte dimension. Column operands select their admitted dimensions; Rowid is8;
`NodeFileSlice` is the BLOB suffix from byte2 capped16 (length saturates at0);
`ScopedHostOrEmpty` converts allowed NULL to BLOB0, otherwise uses the admitted
BLOB dimension capped33; `IntegerOne` is8. Wrong-type expression inputs have
already failed column validation. Preserve ordered suffix/auxiliary operands,
including repeated columns, from index_xinfo; do not deduplicate payloads.

Each field must fit its exact R0 cap and every complete record must be at most
2,097,152. Two individually valid1,048,576-byte fields fail once even the minimal
conservative header is added. Checking only max field length is insufficient.
The helper does not model statement intermediates, CHECKs, uniqueness, triggers,
foreign keys, codecs or semantic validity. Those remain R3/R4 gates.

### UTF-8 and SQLite-owned scope

The named API assumes UTF-8 SQLite record encoding, as already required by the
profile. R2 validates source PRAGMA encoding before probes; R3 constructs and
verifies a UTF-8 destination. UTF-16 native databases are explicitly unsupported
unless a later separately reviewed logical conversion is performed. Do not infer
encoding from ASCII schema names or SQLite's returned Rust String. SQL TEXT byte
probes must include embedded NUL bytes; length(TEXT) is insufficient. Guarded
length(CAST(value AS BLOB)) is appropriate only after the encoding prerequisite.

R1 exposes no application TableId for sqlite_sequence, sqlite_schema, statistics
or platform bookkeeping. Inventory getters preserve their explicit ownership and
unadmitted policy. R2 must return incomplete/unsupported for raw snapshots with
unadmitted internal data rather than treat the99 application tables as complete.
Automatic application indexes are already covered by the owning table's physical
record descriptors. Logical import/rebuild policy remains R2/R3 work.

### Independent record-size and boundary qualification

Pure tests cover all type/null/count errors; caps at minus1/exact/plus1; UTF8
multibyte and embedded NUL; integer/real/NULL bounds; complete-record boundaries
with multiple individually valid fields; the three expressions; rowid and
WITHOUT ROWID suffixes; repeated operands; alias nullability; and checked
arithmetic overflow through the private formula seam. No test asserts that the
conservative bound equals SQLite's smaller minimal encoding.

A native oracle in metadata-core uses existing rusqlite/tempfile dependencies.
Create one-row miniature tables/indexes with known layouts, checkpoint and close,
then inspect their SQLite database-file root leaf page. Parse only the bounded
page header/cell pointer and at-most-nine-byte payload-size varint. Assert table
leaf0x0d or index/WR leaf0x0a and one cell; do not implement a general scanner or
read overflow chains. SQLite's logical payload-size varint covers overflow data,
so even near-limit BLOB fixtures require only a bounded page read. Compare that
independently observed record size to the helper's upper bound for every present
physical table/index record. Root-page IDs come from sqlite_schema while the
trusted miniature database is open, not from helper descriptors.

Include INTEGER PRIMARY KEY alias versus DESC non-alias, unique/nonunique indexes,
WR order/suffixes, partial index included/omitted, all three expressions, empty and
large payloads, UTF8 NUL and multibyte text, and integer sign/size extremes. Add a
UTF16 source fixture proving why the R2 encoding precondition is necessary; its
raw record size is not passed off as a supported UTF8 measurement. Production
compiled descriptors are separately exhaustively compared to R0; miniature
fixtures qualify arithmetic/layout behavior without target-learned production
policy. Native engine-version evidence is recorded. No deployed celld boundary
claim follows; actual celld record/index admission and intermediate queries remain
separately reviewed gates before restricted-profile activation.

### Ownership, source gates and R2 reuse

Proposed paths are schema-contract `src/lib.rs` (module declaration),
`src/storage.rs`, `src/storage/generated.rs`, `src/storage/tests.rs`; metadata-core
R0 generator `mod.rs` (typed ReferenceBundle extraction below), new `runtime.rs` (renderer and
reproduction tests), and `record_oracle.rs` (native file-format qualification).
The generated module is reviewed with its source generator and full artifact
comparison. No dependency or Cargo feature change is expected. Freeze the exact
manifest before source review if this split changes. Runtime helper compiles for
native and Wasm; source reviews precede any generator execution or tests.

R2 will iterate these same static tables/locators, produce one bounded dimension
row at a time, call check_utf8_row_dimensions, and bind aggregate scan completion
to the immutable complete source artifact plus this pinned descriptor provenance.
No parallel arithmetic implementation or unchecked expected-descriptor input is
allowed. R4 uses the same checker on every explicit/derived completed postimage;
that integration must separately cover trigger closure and engine intermediates.
R1 remains useful non-test code without prematurely enabling either consumer.

Author plan audit complete; root and independent review are required before code.


### R1 narrow review refinements: bootstrap, bundle and completed postimages

Root's initial review and recovery's independent review passed the arithmetic/API
architecture; the following exact staging resolves the remaining pre-code gates.
There is no placeholder generated profile and no generation before renderer source
reviews. Stage R1a owns only the existing native test generator's `mod.rs` and new
`runtime.rs`. Refactor the existing generate body into private `reference_bundle()`
returning `ReferenceBundle { foks: Component, authority: Component,
ownership: serde_json::Value, files: BTreeMap<String, Vec<u8>> }`. Keep all existing
trusted source setup, schema checks and output construction in the same order.
Name the existing inline engine-ownership JSON value so it can be both serialized
and retained; return the two typed Components only after the same four files are
encoded. Existing `generate()` becomes `Ok(reference_bundle()?.files)`, preserving
its signature and four output bytes. No new Component Deserialize implementation,
external artifact input, target database or policy inference is introduced.

Stage R1a renderer emits Rust text from that private bundle and validates all four
pinned artifact hashes before rendering. It has no reference to a future
`foks_schema_contract::storage` module, so it compiles against today's unchanged
schema-contract crate. Its private tests check reference graph validity, emitted
escaping/order/counts/bounds, create-new behavior and unchanged R0 artifacts.
Root and independent source review gate this exact refactor/renderer before any
focused R1a tests or fresh artifact generation. Generated maximum column/operand
count must be <=32 explicitly, before producing source for the fixed stack API.

After R1a passes, generate the candidate Rust file in a fresh temporary artifact
directory. Record its hash and preserve it. Stage R1b then adds the runtime module,
helper, tests and that exact generated source in the isolated worktree. Root and
independent reviewers inspect the generated layout/policies and helper together
before compiling or testing any runtime helper. This source handoff may add the
cross-crate compiled-descriptor comparison tests to runtime.rs, now that storage
exists. The generated file is never an unchecked bootstrap placeholder. R1b tests
repeat generation from two fresh trusted sources, compare exact checked-in bytes,
and exhaustively compare compiled fields. Keep R1a and R1b evidence distinct even
if root elects to integrate them as one final implementation commit.

Inputs to R1 are the completed SQLite storage classes after applicable affinity
and value normalization. R2 observes those classes from stored rows. R4 must
model or derive the completed postimage before calling the helper; passing raw
SQL binds is insufficient (for example numeric affinity or NaN-to-NULL conversion).
R1 must not create a new hosted rejection merely because the raw bind's Rust type
differs from the final permitted stored type. Conversely it does not perform
SQLite conversion itself, guess a coerced value, or treat an INSERT NULL rowid as
an already completed identity. That boundary remains explicit in helper docs and
R4's later source gate.

Recovery independent architectural plan PASS is recorded. This narrow staged
refinement is awaiting root and independent review before R1a implementation.


R1 final staged plan passed root and recovery independent review. R1a is authored
in isolated `metadata-storage-bound-validation` at `a777d0a9`, exactly two paths:
ReferenceBundle extraction in mod.rs and new runtime.rs renderer. Author audit,
rustfmt and diff checks passed; no build, test or artifact generation has run.
Frozen manifest/hashes are `/private/tmp/foks-storage-runtime-generator-manifest.txt`
and `-sha256.json`. The generated module is self-contained private static DTOs;
complete inventory/manifests use compiled borrowed enum trees, not runtime JSON.
Its future public storage facade remains R1b. Five focused source tests plus one
explicit ignored exporter cover pinned/reproducible output, bundle tampering,
graph/32-field bounds, literal/inventory/output bounds and create-new publication.
Root and independent source reviews are requested before R1a qualification.

R1a source-review refinement: generated.rs contains one crate-private `data`
module with a scoped rustfmt::skip attribute, covering only machine-generated
DTOs/static descriptors/inventory. There is no dead-code lint suppression. R1b
must expose/use metadata through explicit borrowed getters; handwritten helper
code remains normally formatted. This gives deterministic checked-in bytes
without invoking a formatter during trusted generation. Literal tests additionally
cover maximum u16/u64 emission and reject the oversized ordinal at graph admission.
Both source reviewers are rereviewing this narrow change before execution.

R1a final two-path source gate passed root and recovery, including the formatter
and maximum-literal amendment. Cargo handed over after W1 native/build gates.
Focused existing R0/new R1a tests, strict native lint and two fresh candidate Rust
exports are now authorized. R1b has no runtime source yet and its compiled-field
review remains a separate gate; no generated candidate is admitted by generation.

R1a generator qualification passed without source corrections:14 focused tests
(five new plus nine R0), two intentional ignored exporters, strict native
all-target Clippy, and two separate create-new source exports. The490,341-byte
candidate generated.rs is identical in both exports, SHA-256
`b117937fad909a736e6e7769ce7717bf573f49c208bccd545b36659a68b03539`.
Paths: `/private/tmp/foks-storage-runtime-source-{a,b}/generated.rs`; source/log
hash evidence: `/private/tmp/foks-storage-runtime-generator-qualification.json`.
All four original R0 pins and both source hashes remained exact. Rustfmt check
preserves the candidate's generated-module exclusion. No runtime generated module
has been compiled; R1b's generated-source/helper review remains outstanding.

R1b source freeze is ready in the same isolated `a777d0a9` tree: seven total paths
in `/private/tmp/foks-storage-bound-manifest.txt` and matching `-sha256.json`.
The exact generated candidate remains SHA b117937f... with no edits. Added shared
pure checker/opaque IDs/borrowed getters, seven pure tests, complete compiled-field
and exact-source reproduction comparison, and three independent native file-record
oracle tests. Author source audit/rustfmt/diff checks passed; no R1b compilation,
tests, or regeneration has occurred. R1a passed before these additions.

The file oracle's boundary fixtures deliberately copy only exact trusted table
and index DDL into fresh databases, without triggers. Foreign keys and CHECKs are
disabled for physical-only seeds with large/malformed semantic IDs; CHECK mode is
restored/verified before measurement. This is explicitly not domain admission or
schema readiness. Completed row classes are read back after SQLite affinity.
Partial file-reference and active-generation index inclusion/absence are asserted
against their actual predicates. A separate INTEGER PRIMARY KEY versus DESC file
fixture validates payload decoder behavior; UTF16 shows the required unsupported
encoding distinction. Full runtime layout equivalence is independently checked
against all R0 fields, not inferred from these miniature data fixtures.

R1b recovery independent seven-path source review passed with exact artifact and
source hashes verified. Count/type/null/field-before-record ordering, partial-index
conservative checks, aliases, complete compiled metadata and bounded physical
oracles were reviewed. Root source review and Cargo handoff remain pending; no
R1b test has run.

R1b root and recovery source reviews passed, then the isolated tree advanced to
`6e6b275d` with all seven reviewed hashes unchanged (W1/S3c paths disjoint). Root
independently verified the exact union before Cargo. Final qualification passed:
15 pure crate tests;18 focused generator/oracle tests with two explicit exporters
ignored;188 full metadata tests with three intentional ignored exporters; strict
native all-target lint and pure schema-contract Wasm lint/check. No actual celld
storage admission is claimed. Evidence is
`/private/tmp/foks-storage-bound-qualification.json`.

The first pure run6 passed/1 failed because a test counted aliases across both
components but expected only FOKS's five. Frozen R0 has five FOKS and two authority
aliases; both reviewers approved per-component5/2 assertions before rerun. The
failed log and old source hashes remain in evidence. No helper/generated behavior
changed; final generated source is still the exact R1a b117937f... artifact. All
seven final source hashes match. Cargo is released; root integration/commit is
pending at this entry. R2 still needs whole immutable-source admission, encoding
checks, bounded scans and SQLite-owned handling before these measurements can
contribute to any readiness decision.

R1a+R1b committed together as `5a1f4bc6` after root independently verified all
seven final hashes, logs and generated artifact. Both qualification ledgers now
record that commit; original alias-test failure evidence remains preserved.
All Markdown remains unstaged. R2 scanner work is plan-only until two reviews.

### R2 finalized-package storage scanner — concrete plan for review

R2 will produce an owned `StorageScanComplete` for the FOKS component of
`HOSTED_SMALL_V1`. It means that the captured, exact-schema UTF-8 database's 99
application tables and their conservative physical index records passed R1,
with the separately specified SQLite sequence rule below. It does not mean
native authentication semantics, cryptographic validity, hosted readiness,
physical SQLite integrity, or complete external ciphertext availability. R3 must
establish those properties before importing into fresh trusted DDL. Authority and
celld platform snapshots are not accepted inputs to this first scanner.

**Source and ownership.** The only input is a finalized native backup package,
with the existing `BackupArtifacts` layout. Live installations and archives with
nonempty `-wal` or `-journal` files are unsupported. There is no custom WAL parser,
no replay, no migration, and no silent omission of sidecars. Nonempty `-shm` is
also unsupported; absent or zero-length named sidecars are recorded distinctly
in the package inventory. The database is never opened at its original path.

Capture retains a canonical, no-symlink root directory handle and opens each
fixed child relative to retained handles with no-follow/create-new rules. Only
regular files and the one `keys` directory are allowed; unexpected directories,
symlinks, special files, names, and duplicate identities fail. The database cap
is the existing native default 64 GiB, optionally lowered by the caller. Manifest
size is at most 4096 bytes. Key names are at most 96 bytes, there are at most 4096
key files, encrypted key files are exactly 96 bytes and the wrapping key exactly
80 bytes. The key-count ceiling is an explicit first-package restriction, not an
inferred native protocol limit. Relative names are bounded before retention.

All allowed files are copied to a newly owned private directory (0700/0600), using
32 KiB streaming buffers, checked lengths, create-new publication and fsync. A
bounded inventory plus complete streaming digests is taken before and checked
against the source after capture; a mutation fails, never refreshes the scan's
identity. This check detects change rather than proving that an adversarial
concurrent producer supplied an atomic snapshot: finalized/quiesced production
is an input requirement. The captured package is independently validated even if
the source subsequently changes. Its identity is a domain-separated digest of
ordered relative name, byte length and SHA-256 for **every** file, including the
manifest, all keys and any accepted empty sidecars. It is not main.db alone, a
path identity, or the 24-table dependency revision. Final completion rechecks the
private inventory and hashes before moving the owner into the receipt.

The final closed key inventory is derived from the bounded manifest and scanned
host/capability generation rows using the existing backup naming/state rules.
Unknown, extra, missing or duplicated generation files fail. Traversal is bounded
and streaming; R2 must not call an existing unbounded generation-Vec collector
before admission. Key decryption and binding to an operator root remain R3.
External chunk references are reported in the receipt and block a later
self-contained import unless R3 supplies and verifies their companion closure;
R2's closed inventory covers the native backup package, not arbitrary external
object storage. No ciphertext availability claim follows from storage fit.

The acceptance fixture must use the genuine existing completed backup path
(`RunningStandaloneServer::backup` or finalized standalone backup), preserving
its durable manifest-last convention. If it leaves nonempty sidecars, the source
producer must instead finalize a new private package through SQLite's backup API
under the existing exclusive owner, close/checkpoint it, then publish its
manifest. R2 itself must not clear sidecars to manufacture acceptance. Qualification
must demonstrate an actual accepted package, not merely handwritten files.

**Concrete runtime API and lifetime.** A new `foks_server::restricted_storage`
module owns capture and scan. Proposed non-test interface:

- `RestrictedScanner::capture_finalized_backup(&BackupArtifacts, CaptureLimits,
  &ScanControl) -> Result<RestrictedScanner, ScanError>` captures the private
  package; neither a raw connection nor a claimed digest can construct it.
- `advance(&mut self, &ScanControl) -> Result<ScanProgress, ScanError>` processes
  at most 128 rows or bounded schema/companion units. `ScanProgress` contains only
  Copy counters/stage information, never key bytes or mutable cursors.
- `finish(self, &ScanControl) -> Result<StorageScanComplete, ScanError>` refuses
  until every table reached actual EOF and all final checks passed. The receipt
  owns the private files and read-only connection, has no Clone/Deserialize/public
  constructor, and exposes only profile/source digests and bounded summaries.
  R3 will consume this owner through a separately reviewed private importer seam.

Capture and full hashing run as an explicitly declared maintenance/import job,
not an ordinary 15-second RPC. Their finite job deadline is selected before capture
and never renewed by progress calls. `ScanControl` borrows a full
`foks_local_deadline::Deadline` and cancellation flag;
checks surround every read/query/hash chunk and final completion. Capture retains an owned clone of the original deadline; subsequent controls
may only intersect/shorten it, so `advance` cannot extend the deadline. In-process resume retains the same owned source
and private cursor. There is no serialized resume token or caller-supplied cursor;
process loss requires a fresh scan. SQL/IO/cancellation faults latch failure;
a failed scanner cannot later produce completion. Budget exhaustion can be
reported as incomplete but cannot be treated as EOF. Diagnostics carry a bounded
reason/table ordinal/column ordinal and numeric dimensions, not source payloads.

This adds explicit server dependencies on the existing shared schema-contract
and local-deadline crates. Rusqlite's `limits` and `hooks` features are proposed
for this native scanner, with the exact Cargo/lock delta separately reviewed.
No change to schema-contract's pure helper or runtime descriptor bytes is planned.

**Opening raw SQLite before trust.** Schema metadata inspection necessarily
initializes an untrusted source schema. The private connection is read-only,
no-create, with extension loading never enabled and no custom functions/modules.
Before schema-bearing reads it enables defensive mode, disables trusted schema,
sets query_only, and verifies those settings. It installs a bounded progress hook
that checks the same deadline/cancellation and refuses a statement after one
million VM operations (1000-op callbacks); refusal never becomes successful
validation. SQLite length is capped at 2,200,000 bytes, SQL text at 64 KiB,
columns at 128, variables at 64, attached databases at zero, and worker threads
at zero. Application records admitted by R1 remain at most 2 MiB; an oversized
native record may therefore fail before dimensions are available, with an
explicit unsupported-size/error outcome rather than a completeness claim.

These controls are not a sandbox or a bound on engine parsing, filesystem stalls,
or native heap/RSS. Metadata predicates do not prevent SQLite from first parsing
the schema. The initial source remains a finalized native backup, not an arbitrary
hostile database upload. Root's C4 trusted-schema/engine qualification is a separate
prerequisite for stronger imported-file/production claims. Unsupported settings,
corrupt schema, or engine errors fail closed; no fallback to trusted_schema ON.

**Schema before row data.** Require user_version 59, UTF-8 encoding, no TEMP or
attached objects, and exact native FOKS schema matching the frozen R0 contract
hash and SQLite-owned inventory. Compare through the shared canonical checker
with exact-union semantics, not native extension-permitting required-subset mode.
The expected digest/descriptors are compiled trusted data; the target cannot
provide expected DDL. Names/types/table names are type- and byte-probed before
copying; malformed NULL/BLOB names remain visible and reject. Limits are 1024
schema objects, 256-byte names, 64 KiB per DDL and 2 MiB aggregate DDL. Literal
SQLite-owned classification replaces wildcard exclusions. Exact autoindex links
and sqlite_sequence are checked; statistics and unknown internal objects reject.
No application data scan begins until this comparison succeeds.

**All-history row and index checks.** Traverse all 99 compiled FOKS tables,
including historical and otherwise unreachable rows. Use completed SQLite
storage classes: no raw-bind approximation, conversion, normalization or repair.
For each field project typeof and byte length; TEXT uses UTF-8 byte length via
CAST AS BLOB (including embedded NUL). Feed at most 32 borrowed dimensions into
R1, which checks type/nullability/field caps followed by the entire table record
and every conservatively applicable index. The application payloads are not
materialized just to measure them. Record checked row counts and maxima; total
work scales with the complete source, not the dependency coordinate.

Rowid tables use the compiled unshadowed rowid spelling and signed i64 keyset.
Use separate initial and continuation statements, not nullable-cursor OR scans.
The five current WITHOUT ROWID locators use their exact BINARY ASC PK tuples;
unknown direction/collation/layout is unsupported rather than guessed. Identifiers
come only from compiled descriptors and are quoted, never from source values.
The query returns one candidate row at a time; initial/continuation use indexed
ORDER BY and LIMIT 1. A step advances at most 128 candidates.

A WITHOUT ROWID next locator is copied only after type/field probes and R1 row
acceptance. Its query-side guarded projection must suppress malformed components
and any aggregate locator payload over 2 MiB before materialization. A suppressed
or invalid locator causes explicit failure, never omission or EOF. At most the
old and next locators coexist (4 MiB payload upper bound, plus separately counted
containers/statement/engine ownership). Fixed dimension arrays and bounded
parameters remain explicit. This is a requested-copy bound, not a complete
memory allowance. Copying a locator uses fallible pre-reservation; allocation
failure cannot yield a completion receipt.

**SQLite-owned sequence rule.** R0 explicitly excludes sqlite_sequence data from
R1 table IDs. R2 supplies one narrow rule for the exact trusted
`CREATE TABLE sqlite_sequence(name,seq)`: zero or one row, name exactly the
11-byte TEXT `admin_audit`, seq INTEGER in 0..=i64::MAX. Unknown/duplicate/null/
wrong-type rows reject. If admin_audit contains rows, require the sequence row and
seq >= max(0, maximum event_id). An empty audit table may retain a larger sequence
from deleted history; preserve it exactly, including i64::MAX exhaustion. No
synthetic reset or recomputation from current maximum is allowed. The fixed
2-field sequence record is separately bounded using R1's conservative formula.
R3's fresh-DDL import will preserve explicit audit IDs and this exact high-water;
it cannot import arbitrary internal tables or infer raw-snapshot readiness.

**Semantic boundary and later import.** StorageScanComplete binds the complete
immutable package and physical storage results, not native user authority. Before
List readiness, R3 must validate every conversion/error invariant of
`user_authority_inner`, including all device integer/blob cells, all shared-key
roles/generations, stale-parcel role_type unsigned and visibility i64 conversions,
and present chain/tree/root integer and width conversions, including unrelated
rows. Device exact_hepk/exact_name and shared-key verify_key/exact_hepk retain
native blob/type/size handling without implying cryptographic decoding. Native
stale-parcel handling does not apply stored_role to those roles; any stronger
restriction is explicit unsupported-profile policy, not native parity. Missing
chain/tree/root joins retain native None/permission behavior rather than globally
becoming corruption. Relevant-key lookup alone is insufficient. R3 must
also establish cryptographic/key/external-chunk closure and physical integrity,
then bind its evidence to the exact imported artifact and subsequent typed-writer
mutation coverage. Fresh destination DDL and derived data are trusted/generated;
source DDL is never installed. R2 does not rotate authority/dependency identity or
mint a hosted capability. R4 postimage and query-intermediate admission remains
separate from both scanner and readiness.

**Qualification and implementation staging.** R2a capture/API/configuration and
R2b schema/rows/sequence may be reviewed as separate source groups, but no successful
receipt exists before both are implemented and reviewed. Proposed owned paths:
server Cargo.toml, workspace Cargo.lock, server lib.rs, and new
restricted_storage/{mod,capture,scan,tests}.rs; add further private files only in
an explicit source freeze. No standalone restore or native serving behavior changes.

Tests must cover a genuine finalized backup accepted unchanged; nonempty WAL with
otherwise valid main bytes; journal/SHM refusal; manifest-last incomplete package;
symlink/extra/missing/changed companions; key count/size/name ceilings; source and
private copy hash mismatch; UTF-16 and wrong schema; extra authoritative triggers
and unknown sqlite-owned objects; malformed schema type/length; every table's
history traversal; empty/final rows and i64 extremes; WR boundaries and malformed
oversized locators; cancellation/refusal at every phase; and no receipt after
partial/faulted scan. Sequence cases include deleted high-water, missing/extra/
duplicate/negative/NULL and i64::MAX. An unrelated-table mutation with unchanged
read-dependency coordinate must still change package identity. VM cost probes use
actual first/continuation SQL and distinguish per-step from whole-scan cost.
SQLite operation-limit errors retain their classification; no fixture or engine
limit is raised to make an unsupported source pass. Source reviews precede all
builds/tests. No actual celld admission or production route is enabled by R2.

Author plan review: the finalized-package limitation, source-closure rules,
completed-postimage storage checks and unforgeable owned receipt are explicit.
Root and independent review are requested before implementation.

Root concrete R2 plan review PASS, including declared maintenance/import deadline
classification rather than an ordinary RPC budget. Recovery independent plan
review remains pending; no R2 implementation or tests have begun.

Recovery independent R2 plan PASS received. Narrow retained-handle implementation
uses safe nix fs/dir features (both reviews PASS). Opening refinement also passed
both reviews: only the complete private finalized copy may use a percent-encoded
file URI with immutable=1 and READ_ONLY|URI. Original/live paths never receive that
flag; nonempty sidecars are rejected first. Captured SQLite header read/write modes
are retained diagnostically and final complete hashes/absence of added sidecars
are rechecked. This relies on private ownership, not arbitrary producer correctness.

R2 source is being authored in isolated metadata-storage-scan-validation at
b837bd0d. Seven paths include exact Cargo/lock edges and four new private module
files. Companion generation closure is accumulated while scanning each admitted
row, retaining only required file names, so revoked history gains no new row-count
limit. Sequence maximum is likewise collected through the normal audit-table scan.
No Cargo build or test has run; author review and frozen source gates precede them.

R2 Unix-only module gate passed root and recovery: this first native import tool
uses cfg(unix) and safe nix APIs; non-Unix scanner unavailability is explicit,
with no weaker fallback or change to native serving. Author source audit is
complete. Seven frozen paths are listed in
`/private/tmp/foks-storage-scan-manifest.txt`, hashes in matching `-sha256.json`,
isolated tree `metadata-storage-scan-validation` at b837bd0d. The four new module
files total1,870 lines including14 authored test functions. Cargo changes are
only the reviewed existing-crate edges/features. Root and recovery source reviews
are requested; no compilation, tests or source-derived qualification has run.

Root and recovery full R2 seven-path source reviews passed, including the explicit
local-deadline `clock` dependency feature. The isolated baseline advanced to
3d992330 with all seven reviewed hashes intact. First focused compilation stopped
before tests on a test-only rusqlite API name (`DatabaseName` was removed in
0.40); the correction uses `is_readonly("main")`, preserving the exact assertion.
The original failure is retained in `/private/tmp/foks-storage-scan-focused-a.log`;
this narrow correction receives both reviews before retry. Storage-fit success
still grants no semantic, cryptographic, physical-integrity, or serving readiness.

### R2 prerequisite: finalize the real backup producer before its manifest

Focused R2 run `focused-c` passed eight tests and failed six; the genuine backup
was refused before scanning. Reviewed fixed-name metadata diagnostics in
`/private/tmp/foks-storage-scan-diagnostic-a.log` prove the producer left a zero-byte
WAL and a 32,768-byte SHM after its final readonly integrity check. Database,
manifest, and encrypted-key lengths were otherwise valid. This is not permission
to ignore or delete a sidecar in the scanner. All failure evidence remains.

Proposed separate implementation commit, before R2: change only the private final
publication stage of `standalone::create_backup`. The source/live database and its
lease remain untouched. After hydration, integrity verification and dropping its
reader, finalize the new, exclusively owned backup destination before writing
`key-manifest.txt`. This is a genuine producer operation, not test preparation or
an import-side repair. Existing `BackupArtifacts` API and format remain unchanged;
the completed backup uses DELETE journal mode instead of retaining WAL mode.

Own two paths: `src/standalone.rs` and a new private
`src/standalone/backup_finalization.rs` with its focused tests. Move the existing
manifest write/fsync stage into `finalize_and_publish` (visible only to parent
module), so its tests exercise exactly the failure-before-marker ordering used by
`create_backup`, without an injected production callback or public finalization
API. Preserve create-new mode0600, manifest-last order and destination directory
fsync. No new package/dependency, migration, schema, or R2 code change.

The helper checks cancellation before opening and between SQLite/filesystem
stages. Open only the known regular private destination with SQLite
READ_WRITE|NO_MUTEX|NOFOLLOW, explicitly omitting CREATE and URI. No
`Database::open`, migrations, or source/companion rewrites. Configure busy timeout
zero; there is no retry or renewed deadline. Run `PRAGMA wal_checkpoint(TRUNCATE)`
and require exactly one integer result row with busy=0, log/checkpoint values both
nonnegative and equal or both -1 for non-WAL mode. Then set journal_mode=DELETE,
require the returned mode and an independent readback both be `delete`. Close the
connection explicitly and propagate close failure. SQLite, not manual unlinking,
owns sidecar cleanup. After close require every fixed WAL/SHM/journal path either
absent or a nonsymlink regular zero-byte file, and fsync the regular database;
any busy/mode/close/type/nonempty-sidecar/I/O/cancellation failure prevents opening
the completion manifest. Check cancellation once more before manifest creation.
This bounds busy waiting, not checkpoint I/O duration or kernel fsync latency;
existing scheduled-job cancellation remains cooperative and is not relabeled a
15-second ordinary RPC guarantee.

Tests use genuine public backup production to assert closed nonempty-sidecar
absence, unchanged logical rows/key manifest, restorable contents, and scanner
acceptance in the later R2 group with no fixture finalizer. Private final-stage
tests cover WAL with committed frames, an existing reader pinning an older frame,
an active writer, DELETE-mode idempotent finalization, missing/symlink database,
preexisting completion marker and cancellation. Busy/finalization failures must
leave the marker absent and preserve readable data. No nonempty sidecar is
manually removed to pass a test. Existing backup restore, rotation and external
chunk hydration/compatibility suites remain regression gates. R2's explicit
nonempty SHM/WAL/journal refusal fixtures remain unchanged; its fixed diagnostic
may stay as evidence until a separately reviewed cleanup.

Author plan review: this closes the producer lifecycle that the actual tests
exposed, preserves the scanner's stricter finalized-package contract and makes
that contract obtainable through the real API. Request root and independent plan
reviews before implementation; source reviews then precede all tests. R2 remains
paused at its original reviewed source plus the readonly-test and diagnostic
amendments.

Root prerequisite plan PASS with two precisions: reject any preexisting completion
marker before opening/mutating the database (this is not a repair API), and preserve
existing marker-write failure semantics. The no-marker guarantee concerns failures
before marker creation; write/fsync failure after create-new can leave an incomplete
marker just as the existing producer does. No stronger atomic-publication claim.

Producer source two-path reviews passed before focused execution. First run
`/private/tmp/foks-backup-finalization-focused-a.log` compiled and passed two tests,
with three failures from SQLite NOFOLLOW rejecting the symlink ancestor `/tmp` on
macOS (extended code1550). Proposed narrow correction keeps NOFOLLOW and uses the
existing native `DatabasePathIdentity::existing` canonical-path/regular-single-link
identity validation, then rechecks before open and after close/fsync. This does
not open/migrate or follow a symlink leaf; a supported parent alias resolves to the
same checked dev/inode. Add an explicit parent-alias positive and hardlink negative
while preserving symlink-leaf refusal. Root and independent review precede source
correction and retry. Original producer and R2 failures remain in the ledger.

Canonical-identity plan passed both reviews. Source refinement includes checks
before and immediately after SQLite open (before checkpoint/journal SQL), after
explicit close, and after database fsync. One canonical path supplies SQLite,
sidecar inspection and fsync. Parent-alias success and hardlink refusal are the
sixth focused test; leaf-symlink negatives remain. Recovery final source rereview
passed after requesting the immediate post-open check. No test acceptance rule
has been weakened; producer and R2 pre-correction failures stay preserved.

### R3 scope and ownership direction (concrete implementation plan still pending)

Root approved the first integration checkpoint as a whole finalized native package,
with personal-only imported scope and explicit profile refusal of unsupported
team/SSO/federation data. The exact table/predicate roster needs source audit and
review before code. There is no selected-user graph extraction. Organization64
remains a required later expansion; this checkpoint cannot claim the complete
restricted hosted profile or M1.

The proposed server-owned, non-Clone/non-deserializable `NativePersonalListImport`
is minted only by a real fresh-trusted-DDL logical importer consuming the R2
receipt and operator key material. It owns destination connection, private package,
path identity and physical/key/storage leases; no public raw connection/path or
readiness constructor is exposed. F3 consumes it and retains ownership through
pending work and output cleanup. A private internal connection bridge is permitted,
without adding a server-db→metadata-core dependency. Native-target readiness must
not be relabeled as eventual celld-target import/readiness.

Native List conversion/precedence closure includes every device/shared-key/stale
parcel conversion and joined head width, and every shared-key stored-role check.
This is distinct from existing backup key-provider/host-key consistency checks.
Ordinary native List does not cryptographically replay every historical signed
link; this plan must neither invent that parity requirement nor claim such proof.
R2 storage fit remains insufficient for any R3 semantic or serving capability.

Producer prerequisite final qualification passed at its two frozen hashes:
focused6; full server169 passed/4 ignored; two existing external-chunk backup
regressions; strict server all-target Clippy. Exact source/log hashes and explicit
exit codes/counts are in `/private/tmp/foks-backup-finalization-qualification.json`.
The initial macOS NOFOLLOW failure remains retained. This separate producer change
is ready for integration; R2 remains paused until its baseline contains the fix,
then reruns its unchanged sidecar refusal and genuine-package admission tests.

Producer prerequisite committed7c0f1d47 at exact tested two-path hashes. R2 advanced
to that baseline with all seven frozen scanner hashes intact. Final R2 gates pass:
focused14, full server183 passed/4 ignored, and strict server all-target Clippy.
Evidence `/private/tmp/foks-storage-scan-qualification.json` records exact source,
producer and log hashes plus explicit exit/test counts. Prior compile, sandbox,
and nonempty-SHM failures are preserved. There was no scanner acceptance relaxation.
The result remains a Unix native owned storage-fit receipt, not semantic/crypto/
physical-integrity readiness or celld/serving admission. Cargo has been released;
R3 remains a plan/source-review gate.

R2 committed55dec5e4 after root verified all seven source hashes, both producer
hashes and every retained/final log. The qualification ledger is bound to that
commit. R3 concrete plan follows; no implementation or Cargo work is authorized
until its own reviews and later source/test handoff.

## R3 concrete native personal List import — plan for review

This is the next implementation proposal after R2 `55dec5e4`, not permission to
serve requests. It imports a whole finalized package into a new private native
SQLite database and produces the single consumed owner required by F3. It does
not install source DDL, migrate a source, extract a selected-user subgraph, enable
writers, or manufacture a celld readiness receipt. Organization64, SSO, federation
and actual-cell import/recovery remain later mandatory work. Native behavior and
native storage limits remain unchanged.

### Inputs, job lifetime, and the private handoff

Add a Unix-only child `restricted_storage::import` with a crate-private entry:
`import_personal_list(source: StorageScanComplete, root_key: Zeroizing<[u8; 32]>,
config: foks_server_db::Config, control: &ScanControl) -> Result<NativePersonalListImport>`.
The implementation consumes the genuine R2 owner; a digest/path/summary cannot
substitute for it. The passed control must intersect the captured control, with
the same cancellation owner and no deadline extension. This remains an explicitly
admitted maintenance/import job, not an ordinary 15-second RPC. Check before and
after each bounded copy/query/key operation and before returning the owner;
interruption, allocation, SQLite, integrity or comparison failures are sticky and
never return partial readiness. Existing cooperative limits do not promise bounded
kernel I/O, SQLite parsing, fsync or process RSS.

`NativePersonalListImport` is crate-private, non-Clone and non-deserializable. Its
private fields own the fresh connection, validated native Config, fixed native
schema/storage/personal-policy identity, exact source-package digest, fresh target
dependency coordinate, path identity, database writer and exclusive storage leases,
validated key provider and private destination directory. Keep the source receipt
until all source rechecks/comparison finish; it is not exposed with the result.
The connection drops before key/lock/directory resources. The only consuming
handoff is a crate-private F3 constructor/parts move reviewed together with F3;
there is no public connection/path getter, callback loan, serialized witness or
constructor accepting caller-provided hashes. R3 itself exports no public route.
F3 must retain the moved owner and the independent physical execution/output permit
through commit/sync/fresh-check and final cleanup; a memory owner is insufficient.

The imported witness is explicitly `NativeSqlitePersonalListV1`. Its schema hash
is the compiled FOKS59 contract; its storage descriptor and personal-policy hashes
are compiled/reproducibly generated, not selected by the caller. This is not yet
an ABI/artifact serving profile. F3 must bind its separate trusted authority schema,
catalog, connection policy and production artifact before returning public bytes.
The source dependency coordinate is evidence about the captured source only. A
fresh target coordinate is mandatory even when importing the identical package
again; neither old tokens nor old owners can attest the new import. Target schema
cookie need not equal the source cookie and must never be forced to match it.

### Exact first-scope policy

Freeze the policy as a closed constant roster against the 99 compiled FOKS tables;
regeneration/source tests fail if any table is unclassified. The following tables
must be empty (one bounded EXISTS each, no payload read):

* `teams`, `team_names`, `team_members`, `team_chain_heads`, `team_chain_links`,
  `team_tree_locations`, `team_shared_keys`, `team_parcels`, `team_seed_chain_boxes`,
  `team_admin_tokens`, `team_invitation_certificates`, `team_local_join_requests`,
  `team_local_view_permissions`, `team_remote_join_requests`,
  `team_remote_member_view_tokens`, `team_removal_boxes`, `team_removal_proofs`,
  `team_view_challenges`, `team_view_tokens`;
* `sso_access`, `sso_binding_receipts`, `sso_identity_challenges`,
  `sso_migration_cohort`, `sso_policy`, `sso_sessions`;
* `federation_team_view_permissions`, `federation_user_view_permissions`;
* `rt_channel_sets`, `rt_channels`, `rt_messages`, `rt_user_channels`,
  `rt_user_inboxes` (the last restriction deliberately excludes opaque realtime
  reconciliation state in this first checkpoint).

All three `generic_*` tables permit only chain_type 2 (user settings); their
entity_id and every `subchain_tree_location_seeds.entity_id` must equal an imported
local user UID. Chain_type 4 (team membership), even historical, is unsupported.
Every `kv_namespaces` row must have party_kind 1, namespace_id equal to an imported
local user UID, and host_id equal to this package's validated host. All users must
have well-formed personal-user entity IDs. No fixed role-count premise is added:
Member visibility spans the full i16 range and history is streamed. Empty accounts
and incomplete signup/authority-anchor states remain admissible.

The other classified tables are copied completely, including historical rows and
opaque receipts/ciphertext. This policy does not claim that opaque archived bytes
contain no historical team references; those bytes grant no enabled operation.
R2's external-reference count must be zero and the source chunk-storage binding
must already be absent (as in the real finalized self-contained producer). No
external store is attached and no collection/upload/maintenance job is enabled.
Retained orphan object ledgers are inert copied data, not authority to read or
collect an external object. Host and capability generation inventories each have
an explicit first-profile limit of 4096 rows, including revoked history, so use
of the existing typed validation snapshots is bounded. Excess native-valid history
is a declared unsupported-profile result, not truncation. These additional policy
choices need root and independent approval before source work.

### Trusted fresh schema and exact logical copy

The bootstrap is exclusively a newly created private destination, never the R2
source path. Acquire `DatabaseWriterGuard`, initialize through its existing
`open_database` using trusted native code, close that temporary handle, then open
the destination with a server-private noncreating/no-follow connection under the
retained writer/storage/path-identity ownership. Recheck identity immediately
after open, before SQL, and at completion. Configure defensive/no-extension,
trusted_schema OFF, bounded SQLite limits, cancellation/progress hooks and zero
busy wait before inspecting schema. The fresh bootstrap code and pinned FOKS59
schema are part of the importer provenance; opening a caller-supplied target is
not an alternative path.

Before using any DDL text, verify this fresh database against the exact compiled
schema contract and the closed expected trigger identities. Only its own verified
trusted trigger definitions may be retained, under R2's schema-text bounds; none
come from the source. Disable FK enforcement before the private copy transaction,
verify that state, drop the exact trusted triggers, and remove all fresh seeded
rows. Preserve the freshly generated nonzero dependency incarnation separately.
Copy every source row, including all derived columns/projections, with triggers
absent; otherwise usage/dependency/counter triggers would replay and double-count
already-derived state. Reinstall the identical verified trusted triggers only
after the copy and comparison; full schema verification must pass again. This
explicit fresh-bootstrap/drop/reinstall approach avoids a public raw server-db
connection API or new runtime source-derived DDL authority.

Use the same compiled R1 table/locator descriptors and R2 rowid/WR keyset order.
Before any owned cell copy, inspect borrowed SQLite type/length, enforce field and
complete table/index record bounds, and validate the bounded locator. Copy one
row at a time with a checked sum of owned cell bytes no greater than the admitted
record payload; old/new locators retain the existing 4 MiB combined bound. Keep
statement/binding/SQLite engine allocations separately declared. Identifiers come
only from compiled descriptors and are quoted; all values are bound parameters.

Preserve the true signed hidden rowid for ordinary rowid tables, including gaps,
negative values and extrema. An INTEGER PRIMARY KEY alias uses its column value
without a second conflicting rowid bind; DESC non-alias and shadowed rowid cases
follow the actual generated locator. WITHOUT ROWID copies use the complete exact
PK tuple. Never renumber rows. Null/integer/real/blob/text storage classes are
preserved; TEXT requires actual valid UTF-8 before constructing a Rust str, while
embedded NUL remains valid and unchanged. Encoding=UTF8 alone does not validate
malformed stored TEXT. Do not cast, stringify numeric cells, decode/re-encode
opaque blobs, or use lossy UTF-8. Destination affinity/CHECK/uniqueness application
may reject unsupported source data; readback comparison must also reject any
storage-class or byte change rather than treating raw bindings as postimages.

Before any intended coordinate change, walk source and destination in the same
bounded locator order and compare every row, locator, storage class and exact
BLOB/TEXT bytes; compare numeric values exactly (REAL representation handled
without converting through decimal text). Require both EOFs and per-table counts.
Restore `sqlite_sequence` from its separately admitted source state after all
AUTOINCREMENT inserts, including deleted-row highwater and i64::MAX exhaustion;
never recompute it from max(rowid). Absence of an admin_audit sequence row is also
preserved. sqlite_sequence is the sole admitted SQLite-owned data table; unknown
internal rows/statistics remain unsupported. Target physical page layout, root
pages and schema cookie are intentionally not source-equality claims.

After exact comparison, replace only metadata_read_dependency's source coordinate
with the fresh target incarnation/revision1, checking it differs from the source.
The seed came from the existing trusted fresh native initializer, not operator
input. Require exactly one matching old row updated and verify the new coordinate.
No serving handle exists during this transformation. Reinstall trusted triggers,
commit, restore FK ON and verify it, run bounded/cancellable foreign_key_check and
integrity_check, then reverify full schema, all R1 record dimensions and source
package digest. Any failed check destroys the unexposed destination. These checks
establish SQLite integrity/constraint closure for this import, not cryptographic
verification of opaque application content or a hard execution-time proof.

### Keys and native authority semantic closure

Copy the already closed encrypted key/manifest inventory into a separate private
destination key directory with create-new/no-follow bounded copies and exact hash
comparison. Do not open a key provider against the immutable R2 source directory:
its lock creation would change the package. Add a narrowly scoped crate-private
`DirectoryKeyProvider::open_existing` that requires an existing private directory
and wrapping file, takes the ordinary provider lock, and calls the existing
`load_wrapping_key` rather than load-or-create. It must not generate missing keys,
bootstrap a wrapping key, clean rotation temporaries or repair an import. Purpose
and generation loading uses existing decrypt/authentication routines; root input
and decrypted keys retain Zeroizing ownership. Missing/wrong keys fail closed.

Reuse the existing manifest/purpose-generation, nonrevoked host generation
public-key equality and capability-ledger validation semantics from standalone
backup, on the bounded private destination. Validate the immutable bootstrap host
and manifest against the imported package and key provider. No full historical
signed-link replay is added or claimed. Any additional key/host validation needed
by the actual F3 TLS constructor must be named and reviewed, not silently replaced
by R2 filename presence. Keep the provider/leases with the imported owner.

For each imported user, first evaluate the exact native joined-head condition in
`read.rs::user_authority_inner` (user head + matching tree location + current root
head/root). If absent, retain that absent state and skip its subsequent authority
conversion audit, as native does. If present, stream the following native checks,
with bounded rows rather than building the whole history Vec:

* head seqno/root epoch are nonnegative integers; tail hash, next tree location
  and current root hash have the exact native fixed widths;
* every device's device_id/exact_hepk/exact_name is BLOB, subkey_id is NULL/BLOB,
  active/role_type/visibility are INTEGER, and role_type converts to unsigned;
  native active==1 semantics are not strengthened into a new boolean rule;
* every shared key has INTEGER role_type/visibility/generation, unsigned role_type
  and generation, BLOB verify_key/exact_hepk, and its full role passes the same
  `auth::team::stored_role` conversion used by native `key_generations`;
* the exact native stale-parcel query's rows convert role_type to unsigned and
  visibility to i64. Do not impose stored_role on stale rows: native does not.

Native schema CHECKs/foreign keys and restricted policy can independently refuse
a package, but the claimed conversion equivalence must retain these distinctions.
Current/headless users are not required to have a device/key/anchor merely to
enter the import. Public request outer decoding, clock/credential checks and later
Permission result remain F3's responsibility. This audit grants no authorization
for a particular peer. The source-defined row converters/predicates must have
independent native-oracle tests before witness minting; do not infer semantic
validity from lengths or the schema hash alone.

Only public List is contemplated. Full exact copy preserves native observable
projections; it does not attest arbitrary future write/Usage/maintenance invariants.
Before any such route is enabled, independently verify its derived invariants and
typed writer/postimage closure. F3's sole private clock-floor write must have its
separate reviewed proof that it cannot invalidate this personal semantic witness
or restricted record bounds. Any data writer/connection escape/schema change
invalidates the witness; it is not lazily refreshed by a matching 24-table revision.

### Source groups and validation gates

Proposed ownership is server-only: existing `restricted_storage/mod.rs`,
`capture.rs` and `scan.rs` receive minimal private consuming/helper seams; new
`restricted_storage/import/{mod,copy,policy,semantic,tests}.rs` owns the importer;
`keys/directory.rs` adds the existing-only provider mode and focused tests. Split
files further only for readability, with the final exact manifest reviewed before
builds. No normal server-db→core dependency or public raw API is added. F3's module,
normal core dependency, exact consuming handoff and authority union belong to its
separate integrated source review. Source tests pin the closed policy and descriptor
hashes; no dynamic table discovery becomes expected authority.

Tests must begin with genuine finalized public backups through R2, then import
without a test finalizer or fabricated receipt. Cover: exact all-table copy and
source immutability; missing anchors with malformed later auth data and native
outer-error precedence; each conditional authority conversion and full i16 roles;
all unsupported table/predicate families; embedded-NUL UTF8, invalid UTF8 refusal,
integer/rowid extrema and gaps, WR identities, sqlite_sequence deleted highwater
and exhaustion; trigger-free derived-row copy without duplication; affinity
change/corrupt CHECK/FK refusal; wrong/missing wrapping/purpose/generation keys;
source replacement/mutation, target alias/symlink and lock contention; cancellation
and injected failure in every stage; no owner on incomplete work; fresh differing
incarnations across identical imports and exact source coordinate preservation.
Compare bounded semantic audit results against native user_authority/key conversion
oracles for joined-head-present and absent cases. Keep native physical integrity,
crypto-provider checks, storage fit and public List parity as separate assertions.

Root and independent plan reviews precede implementation; author/root/independent
source reviews precede tests. F3 must consume the genuine result in real request,
SQL, framing and output-lifetime tests before the combined factory can claim a
ready native List path. Actual celld import/sync/recovery and org64 remain explicit
future gates. Author plan audit complete; concrete policy additions, trusted fresh
bootstrap sequencing and the private existing-only key-provider seam are submitted
for root and quota review now.

### R3 review refinements: exact key validation, bounded diagnostics, and staging

Root approved the policy additions and the fresh-native-bootstrap alternative.
The following precisions close the remaining pre-code contracts; they do not
change native startup or call `load_or_bootstrap` on the import.

The key/bootstrap adapter is new private `import::keys::validate_existing_import`.
It has a closed call inventory. Before calling any typed collector, the importer
uses borrowed type/byte probes on the already R1-admitted destination to require:
host_metadata's singleton host_id33, canonical_name UTF8 <=255 bytes, ASCII
key_manifest_blob <=4096 bytes equal to the copied manifest, bootstrap_blob <=1 MiB;
all host/capability generation IDs16, host public IDs33 and UTF8 filenames <=96
bytes, <=4096 rows in each ledger; active rotation result <=1 row with its fixed
integer/ID columns. Before decoding/verifying the probe, require <=4096 stored
hostchain links and checked sum of exact_link byte lengths <=1 MiB. These are
additional declared first-profile ceilings; do not let a typed Vec collector
perform the counting. The immutable destination prevents a preflight/read race.
The provider directory still has R2's <=4096 closed filenames, <=96-byte names and
fixed encrypted-file sizes. No unbounded filesystem list is introduced.

The exact allowed typed reads are `ReadDatabase::host_bootstrap`,
`host_key_generations`, `capability_key_generations`, `active_host_rotation`,
`roots_at(&[probe_epoch])` and `current_root` on this private destination. Preflight
both selected root rows' exact_root/exact_signed_root <=1 MiB each before their
single-row typed reads. A temporary ReadDatabase may be opened only after trusted
schema/row copy/constraint verification and is closed before the owning connection
is handed to F3. It cannot migrate or repair the source. Bound and check each
collector's returned row count too. Do not call `hostchain_links()` or
`user_authority()` in the production importer: their whole-history Vecs are
unnecessary. Their use as independent test oracles is confined to small fixtures.

Call `KeyGenerationManifest::decode` on the bounded ASCII manifest, then
`validate_existing(provider, require_genesis_host, require_genesis_capability,
false)`. The booleans are derived from nonrevoked canonical generations, never
input flags; SSO blocked-recovery bypass is unavailable. Use the existing
`standalone::host_key_backup_files` state/filename closure after the bounded ledger
preflight, compare its required files to the closed inventory, then load every
nonrevoked host generation with `load_existing(Host)` or `load_generation(Host,id)`
and require the recorded generation ID and exact Ed25519-derived public entity.
Call `keys::validate_capability_key_generations` on the same bounded private
reader/provider. Its list_generations is bounded by the already verified private
closed directory. No rotation cleanup/create/remove function is called.

For public-host/key consistency, call
`foks_verify::verify_public_host(stored.canonical_name, stored.probe_response)` and
`ProbeResponse::decode` only after the 1 MiB preflight. Require the verified host
ID equal to the stored host ID. Reuse
`host::bootstrap::validate_generation_ledger_against_probe` through a narrow
crate-private visibility change; it receives the bounded generation/probe data.
It verifies host additions/revocations, not historical user-chain signatures.
Compare the single referenced probe root and current root using the same equality,
epoch ordering and signable-hash checks as the existing read-only branch of
`load_or_bootstrap`. Stream stored hostchain rows one at a time against the bounded
probe hostchain, requiring exact sequence/encoded-link/hash equality and both EOFs;
never collect all stored links. Use `restore_public_host_identity` on the verified
parts and `pki::host_tls::delegated_tls_ca_der` with the existing delegated key and
stored canonical name, requiring that CA among the verified host certificates.
Do not manufacture endpoint input: this import validates the self-contained
verified stored host identity; the future serving frontend must separately bind
its configured endpoints/TTL at F3 activation. Normal native startup is unchanged.

All of these operations are read-only with respect to copied SQL/key contents.
No `load_or_bootstrap`, `bootstrap`, provider `get_or_create`, source migration,
rotation cleanup, SSO block-policy mutation or key-file repair is permitted. The
private provider's lock is an importer-owned operational file, excluded from the
copied encrypted inventory equality but not confused with a source companion.
Root input and decrypted material stay zeroized on success/failure. Finite bounded
input and cooperative allocation/deadline checks are not a hard heap/RSS proof
for the existing crypto/protocol decoders; keep that limitation in evidence.

Re-run all personal-scope predicates on the completed destination before minting
the owner, as well as schema/storage checks. For FK checking read only the first
failure row and return a fixed classification without its table/key payload.
Integrity checking requests at most one reported failure, accepts exactly the
literal successful result, checks type/byte length before copying, and does not
echo SQLite diagnostics. Both retain progress/deadline cancellation. No diagnostic
collector can grow with corrupt rows.

The deliberate-difference ledger is explicit: fresh initializer seed rows are
removed under disabled triggers; source rows and sqlite_sequence are first copied
and compared exactly; source dependency coordinate is captured before replacement;
only that row's incarnation/revision then changes to the initializer's independently
sampled nonzero incarnation/revision1. Require fresh != source, verify the intended
old/new tuple and single-row mutation, and recompare all other source rows (or an
equivalently exact bounded second pass with just that single typed exception).
Target schema-cookie/root-page differences are physical metadata, not data changes.
Afterward no unreviewed reconciliation rewrites are allowed. A stale/source receipt
can never stand in for this newly owned target.

REAL comparison is storage-class-preserving and uses SQLite's retrieved numeric
representation, with no decimal conversion. A source REAL must be finite for this
first profile (nonfinite is explicitly unsupported); preserve the f64 bit pattern,
including sign of zero, across bound insertion/readback, refusing if SQLite
normalizes it differently. SQL NULL remains NULL, including any earlier SQLite
NaN-to-NULL coercion; do not reinterpret it as REAL. Integer extrema use i64 binds.
The source/destination byte-exact claim applies to TEXT/BLOB; numeric comparison
has this separate explicit rule and tests. This is a conservative import policy,
not a claim about every native-valid REAL value.

Initial implementation staging is `cfg(any(test, feature="celld-proof"))` for the
new importer and its existing-only/host-adapter additions. Add one feature-only
qualification caller that performs real finalized-backup capture, R2 completion
and R3 import, then explicitly drops the resulting owner before returning a fixed
bounded diagnostic summary (source/profile hashes, target coordinate, counts and
success classification only). It returns no path, connection, key or readiness
capability. Native unit tests can consume the genuine private owner to exercise
ownership, failures and row comparisons. There is no blanket dead_code allowance
or public proof constructor. This group is explicitly an import qualification
checkpoint; F3 promotes the same importer into the default-private build only
with its real consuming constructor at a later union source review. Qualification
entry points remain excluded from serving artifacts.

Source manifest therefore also includes the narrow host-bootstrap helper visibility
and the real qualification caller (location under the existing celld-proof
qualification surface, frozen before edits). Policy/hash constants remain compiled;
no new artifact generator or target-learned default is needed. Request root and
quota narrow plan rereview before implementation, especially the explicit hostchain
ceilings and qualification-only staging.

Final roster precision from the schema audit: `user_local_view_permissions` is not
an opaque archived blob; require its target_id to be a well-formed personal-user
entity ID (its viewer UID is already an imported-user FK). A target user may be
absent, so this does not invent a new target-existence requirement. Team-type view
targets are unsupported even if the teams table is empty. Recovery challenges and
other inactive opaque capability payloads remain copied history with no enabled
recovery/capability operation; no claim is made that they grant personal List
permission. Add a direct non-user target negative to the policy matrix.

R3 key-read ownership refinement (implementation audit, narrow review): do not
open the proposed temporary ReadDatabase after all. Its independent connection
would not carry the original importer progress hook. Private `import::keys`
adapters execute the same fixed host/bootstrap/generation/rotation/root projections
on the already hooked owned connection, after the same preflights, and construct
the existing typed row structs with identical integer/enum/width conversions.
Small fixtures compare them directly with the native typed APIs. This replaces
only the read mechanism; all input limits, validators and no-repair rules above
remain. Implement CapabilityGenerationReader privately using the hooked adapter.
No extra connection, raw public helper or unhooked opening is introduced.

Extract `standalone::host_key_backup_files` post-read state/filename closure into
a crate-private helper taking the already loaded host generations, optional active
rotation and a bounded capability-generation supplier. The native wrapper still
reads host generations then active operation, calls the same helper, and its
capability supplier reads only at the original point after host closure validation.
Thus ordinary producer error/query order is unchanged. Import supplies its bounded
already-read typed capability vector; it does not copy/reinterpret the ledger
closure. The exact fixed SELECT projections and conversion tests are part of the
source review. Root direction PASS; quota independent narrow review requested
before authoring key adapters/refactor.


R3 committed `5d6d5fb7` after author/root/independent source reviews and validation.
The private fresh importer preserves exact source rows, rowids and sequence state,
restores only trusted triggers, validates personal metadata and existing keys, and
retains the database with its physical leases. Final default and celld-proof server
library suites each passed169 with4 existing intentional ignores; both strict
all-target lints and2 real backup tests passed. Focused R2/R3 coverage passed32 and
is included in the final library runs. The intentional-corruption fixture and
test-only import corrections were reviewed before reruns; original failures remain
in /private/tmp/foks-personal-import-qualification.json. Root verified13 source
hashes and20 evidence artifacts before commit. This is native import qualification,
not serving readiness, celld activation, recovery latency or capacity proof.
All Markdown remains unstaged. The reviewed F3 group now connects this owner to
guarded personal List execution and real TLS output.
