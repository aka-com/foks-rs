# Scalar allocation audit for single-transaction List

Read-only proposal, 2026-10-16, main after `6662bcc8`/`ac9d3720`. No source changes or tests in this audit. This concerns the existing scalar transport; no SQL/catalog/public wire or cryptographic change is proposed. The new strict context decoder and the List workspace trace are separate components.

## Selected calls and bounds

Activation uses scalar43 ReadDependency and44 ReadSchemaCookie. List adds16 AuthorityState,18 AuthorityClock, and33..37 personal current-certificate/current-device/anchor/SSO/key-generation calls. List body generations use page10; directory/List payloads use pages2..8 and are not scalar copies. The existing successful whole-operation call ceiling remains1160, including repeated current-credential/profile checks; no allocation scope may reset it.

Sources: `src/catalog.rs` Statement/spec/validate_parameters and READ_DEPENDENCY_SQL; `src/authority/catalog.rs::spec`; `src/personal/catalog.rs::spec`; `src/personal/mod.rs::authenticate_peer`, `recheck_current`, `current_key_generation`; `src/authority/mod.rs::load`. All paths below are within foks-metadata-core unless explicitly qualified.

| ID | Parameters from the selected caller | Result columns; important storage bounds |
|---|---|---|
|16|none|identity TEXT, activation TEXT, owner BLOB, eight integer-text columns; existing transport admits4096 bytes per TEXT/BLOB, then authority decoding requires16-byte owner and typed JSON identity/activation|
|18|one nonnegative signed-i64 clock value|direct affected count encoded as Value::Integer; normal singleton update is1; write has no row|
|33|serial1..20 bytes (each zero-padded probe), DER1..4096 bytes, nonnegative i64 microseconds|three identifier BLOBs then two integer-text columns; valid identifiers33 bytes, generic transport currently allows4096 before domain validation|
|34|three identifier BLOBs, valid33 bytes each|two integer-text columns|
|35|one user identifier BLOB, valid33 bytes|constant integer1 or no row|
|36|none|constant integer1 or no row|
|37|user identifier33 bytes and two integer values (role type/visibility)|one integer generation or no row|
|43|none|typeof string, length integer, nullable32-byte incarnation, typeof string, revision integer; CASE suppresses malformed/oversized incarnation before crossing engine boundary|
|44|none|one integer schema cookie; admitted guard requires its qualified range|

These caller bounds do not replace transport validation. Statement::validate_parameters currently permits4096 bytes for every blob/text parameter and does not use Spec.maximum_blob_bytes as a per-field limit. Native query and the JS cell helper likewise use the global4096 payload bound. Do not silently tighten generic qualification calls to caller-specific33/20-byte limits as an allocation fix. The new guarded phase allowlist is separately reviewed.

Serialization inventory for compact existing Value JSON: integer is14+decimal_length bytes (at most34); nonempty BLOB is11+sum(byte decimal widths+commas) =4*N+10 at all255; empty BLOB is11. TEXT is11+escaped_UTF8_length, at most11+6*N; Value::Null is the6-byte string `"null"`, distinct from a missing query's4-byte bare `null`. Nonempty row/parameter arrays add count+1 bytes. Execute returns one Value, not a row array.

| ID | Max parameter bytes accepted by current generic catalog types | Conservative max row JSON admitted by current global cell bounds |
|---|---:|---:|
|16|2|65852 (two4096-byte text values,4096-byte blob,eight20-digit integers)|
|18|36|34 change-count Value|
|33|32826|49256|
|34|49186|71|
|35|16396|36|
|36|2|36|
|37|16466|36|
|43|2|65642 using generic type caps; actual fixed projection <=326 bytes for valid32-byte incarnation/i64 revision|
|44|2|36; qualified nonnegative u32 cookie <=26|

These are encoding bounds, not allocations or claims all maxima can coexist in a valid typed record. Actual selected33 parameters are <=16521 bytes (20-byte serial,4096 DER,nonnegative19-digit clock); selected34 parameters <=430. An AuthorityState row can exceed the existing64KiB scalar envelope under generic malformed-data caps. The Wasm host already rejects an encoded response above65536; a correction must reject before that output allocation, not expand the ABI or force an analogous serialization cap on native's existing Vec<Value> API. Semantic malformed-state classification remains infrastructure failure, never a public success.

## Exact current gaps and lifetimes

1. `native.rs::NativeSql::admit` validates types/call count/write mode, then collect allocates a Vec<rusqlite::types::Value> and clones every blob/text; integer n.to_string allocates too. None calls reserve_allocation before doing so. The input &[Value] remains owned by the domain caller during this copy. rusqlite then binds/copies into SQLite, which is a distinct engine allocation domain.
2. `native.rs::NativeSql::query` allocates Vec<Value> then clones accepted row text/blob values without admission. Each integer canonicality check allocates n.to_string. Its second-row rejection happens after the first row was copied. Borrowed ValueRef permits inspecting sizes/types before Rust payload copies. SQLite query/statement/row storage itself is not charged by adding Rust Vec hooks.
3. `wasm.rs::Host::invoke` serde_json::to_vec allocates/grows encoded parameters with no precharge and no Rust-side64KiB encoded bound before the import. Value's decimal serializer (`catalog.rs::decimal::serialize`) allocates n.to_string. Owned original parameters and encoded JSON coexist through the import; encoded unnecessarily remains live while decoding the returned row.
4. `wasm.rs::response_alloc` allocates an initialized Vec up to64KiB without admission. Replacing a cleared RESPONSE can overlap old capacity with the new allocation. Host::invoke clears rather than drops RESPONSE before SQL; an import returning an error leaves partially allocated response storage retained in TLS. Success takes it, but generic serde_json::from_slice<T> allocates row containers, byte Vecs, text Strings and decimal Strings without a column/cell-bound visitor or precharge.
5. `catalog.rs::decimal::deserialize` owns a String then n.to_string for canonicality. Generic Value/Vec serde can materialize wrong-length arrays or wrong-kind values before the shared host/domain checks. Since scalar JSON contains numeric byte arrays, pinned serde_json feature behavior matters: arbitrary_precision parse_any_number can allocate a16-byte String per numeric token; float_roundtrip can grow scratch for malformed long numbers. Do not treat total reply bytes as equal to cumulative decoder charges.
6. `tools/foks-metadata-spike/host.mjs::sql/respond` materializes SQL arrays, JSON.parse results, values.map bindings, Array.from blob values, JSON.stringify text and TextEncoder bytes before response_alloc. respond checks64KiB after stringify/encode. Those are host allocations, not covered by a Wasm Snowpack hook. The current host is qualification-only; no native Rust precharge proves a production JS bridge bounded.
7. Upstream `personal/mod.rs::blob` clones inputs (including DER for each serial probe); certificate row conversion clones credential/owner EntityId bytes. `authority/mod.rs::load` parses identity/activation JSON into additional owned structures while the full scalar row remains live. These are domain allocations outside NativeSql/Host::invoke. Storage's workspace trace must report these separately; a scalar transport correction cannot claim the whole invocation accounted.

## Smallest coherent correction groups

### S1: native scalar ownership admission (separate source review)

Add a private scalar allocation helper shared with the following group. Native admit retains the existing validation/call-count/mode checks first, then computes checked parameter container bytes + exact owned blob/text bytes + bounded integer formatting bytes and reserves before allocating bindings. Prefer an allocation-free canonical i64 byte check/stack formatting helper to repeated to_string, while preserving current IntegerRange versus Storage classifications. No dynamic SQL, catalog IDs or payload bounds change.

For query, borrow and validate each first-row ValueRef and its type/UTF8/length without copying; compute checked Vec<Value> container plus all copied payload sizes; reserve before creating that Vec or any owned cell. An allocation-free first pass may be followed by a second borrowed pass over the same row; do not allocate a temporary Vec of descriptors. Existing zero/one-row and second-row rejection stay intact. Parameters/input ownership and returned row ownership remain with the caller's current phase; the helper does not create/reset a budget or release it early.

Explicitly qualify Rust-owned allocations only. SQLite internal buffers, rusqlite prepared-statement machinery and result materialization need separately measured host reservation. Do not label a guessed fixed per-column constant as a proof of SQLite memory.

### S2: existing scalar JSON codec and Wasm response ownership

Keep exact existing JSON bytes and64KiB envelope. Add checked exact encoded-length calculation for the finite Value representation (including JSON escaping and byte decimal widths); reserve exact output plus audited serializer scratch before constructing it. Use pre-sized Vec and a bounded writer that refuses growth past the calculated length. Do not use serde_json::to_vec's geometric growth and claim only final length was charged. Independently compare new encoding to current serde serialization across integer extrema, all byte widths, empty/null, control/quote/backslash/Unicode text, all44 catalog parameter shapes and length boundaries. This need not alter Value's general public serde implementation.

Replace Host::invoke's generic DeserializeOwned return with query/change-count-specific bounded decode: expected column kinds/count, per-cell4096 caps matching current generic host, exact missing-row representation, and bounded integer parsing. A strict DeserializeSeed or private equivalent can reserve row/container/vector capacities before fill and reject extra fields/elements without buffering them. Every malformed-value diagnostic must be static/capped, following the separately reviewed context decoder principle.

This scalar decoder must still accept escaped TEXT payloads, unlike the context's unescaped-ASCII restriction: AuthorityState carries JSON inside JSON. Audit serde string scratch/growth and numeric-token paths explicitly. An allocation-free lexical preflight can cap outside-string raw numeric byte tokens to0..255/three digits and reject signs/floats/exponents before serde; signed SQL integers are already quoted canonical decimals. Reserve bounded escaped-string scratch and error storage from the actual pinned parser behavior, or use a reviewed no-scratch reader. Do not choose a scalar workspace constant until the trace/source audit establishes its formula. The new context's26,997-byte allowance cannot be reused for this different payload.

At each import boundary, physically take/drop preceding RESPONSE storage before a replacement allocation. response_alloc reserves before allocating; no Rust RefCell borrow spans the host import. After return, drop encoded parameters before decoding the response. Success moves returned row values into their owning phase; every returned-error path takes/drops partial response storage. Trap/import exceptions still require host instance destruction, not imagined Rust unwinding. Normal TLS cleanup must not erase a different nested operation; existing domain() reentry and qualification APIs require specific regression tests. The future guarded allocator phase state machine remains separate; this patch does not make existing raw fixture exports production-safe.

### S3 prerequisite, not hidden in S1/S2: domain and host bridge

Storage's trace must include all pre-List scalar/domain phases and persistent authority-state allocations; certificate/owner/profile material may outlive individual scalar calls. Prefer separate credential/authority scratch lifetime or move-only retained context rather than charging it to a List page that can reset. Any caller precharges are separately reviewed after the source trace.

The JS/SQLite bridge must reserve/validate before owned copies/serialization. Existing SQL scalar16 has unbounded TEXT columns at the engine boundary; native can borrow before copying, but celld may materialize oversized corrupted values before JS sees them. Trusted persisted-value invariants or a separately reviewed bounded scalar projection are prerequisites to claiming complete host-memory bounds. This audit does not change SQL/catalog just to hide that gap. Exact-response serialization can avoid unbounded grow/Array.from through a later bounded host codec, but its object/string/typed-array peaks require explicit measurement. Keep host SQL engine memory outside the Rust allowance and retain physical ownership during cancelled work.

## Tests before integration

After author and independent source review, S1 tests use actual selected statements, an allocation recorder/refuser and unchanged real schema. Refusal at binding admission prevents SQL execution; refusal at row-copy admission creates no owned payload; oversized/malformed borrowed cells fail before clone; successful output/status and transaction rollback remain equal. Include no-row, two-row host-double fault, every integer boundary/noncanonical form and admitted row ownership after return. Test native32/64-bit layout assertions as supported, not hardcoded host sizes.

S2 pure native codec tests independently compare old serialization and strict decoded values; injected refusal at each owned allocation boundary leaves no callable stale response. Wasm tests exercise actual scalar import/response_alloc with success, returned error, thrown import, zero/oversized allocation, two consecutive large/small responses, repeated instance use, nested legacy domain path and no counter reset. Malicious long numeric token, escaped string, unknown enum label, excess columns/blob entries and wrong types cannot allocate before their applicable admission. Preserve all existing native/Wasm authority/differential vectors and actualcelld scalar/page behavior. No new public/production endpoint or unreviewed scalar ABI is required for qualification.

The List trace must be rerun after S1/S2 and show previously missing charges as a distinct change. Its existing uncharged trace remains evidence, not a passing production budget. Full successful maximum List/RPC framing, repeated fragmented64MiB memory and output/permit cleanup remain separate combined gates. No tests are authorized by this audit alone.


## S1 concrete implementation freeze for review

Scope is only Rust-owned scalar allocations in the native host. Proposed paths:
`crates/foks-metadata-core/src/native.rs`, a private
`crates/foks-metadata-core/src/native/scalar.rs`, and native scalar tests (private
unit module or one integration target using existing test DB helpers). No Cargo,
catalog, SQL, Wasm, schema, RPC, context or caller changes. S2/S3 remain explicitly
unimplemented and their terms remain uncovered in the List trace.

`ScalarDecimal` is a private20-byte stack array plus a start index. Conversion uses
unsigned_abs and reverse base10 digits, prepending minus only for a negative i64;
zero emits one digit. It exposes only the populated ASCII slice. Parameter integers
still bind as SQL TEXT, not SQLite INTEGER. Conversion to owned String occurs only
after admission. Canonical row parsing keeps the existing order: byte length/type
branch, UTF8 (Storage), i64 parse (IntegerRange), then exact comparison with the
stack encoding (Storage if noncanonical). This eliminates both n.to_string heap
allocations without changing their accepted values or error categories.

After existing parameter validation, call-count increment/2048 check and mode
check, compute:

`B = parameters.len()*size_of::<rusqlite::types::Value>() + sum(blob.len | text.len | decimal.len | 0)`.

Every add/multiply is checked; failure is Bounds. Call the existing Snowpack
allocation admission hook before constructing the bindings Vec or owned payloads.
Use explicit try_reserve_exact for the Vec and exact-size byte/String copies; any
fallible allocation failure becomes Bounds. Skip a zero-byte admission because an
empty Vec allocates no heap memory. No charge or lifetime is reset inside admit.
The parameter input values were allocated by the caller and are separate terms.
The binding Vec/payloads are consumed/dropped by rusqlite parameter binding; SQLite
may retain its own copies, which are outside this Rust formula.

For a returned first row, perform one borrowed validation/size pass in original
column order, without a descriptor Vec. The private borrowed cell representation
can hold i64, &[u8], &str or null and allocates nothing. Compute:

`R = spec.columns.len()*size_of::<Value>() + sum(accepted blob.len | accepted text.len | 0)`.

Retain the current global4096 per TEXT/BLOB acceptance, including NullableBlob;
do not substitute spec.maximum_blob_bytes or selected-caller identity widths.
Reserve R before constructing the result Vec or copying any cell. A second pass
may reconstruct borrowed validated cells and copy into the preallocated Vec; it
holds the same row/statement and performs no intervening SQL. Integer values have
no heap payload. The existing extra-row probe occurs afterward and discards the
first row on conflict, preserving Bounds. No-row queries perform no row charge.
The returned Vec remains owned by the existing caller/phase; no new budget reset,
transferable proof or permit release API is added.

The charge is cumulative logical Rust allocation admission, not engine RSS. It
includes actual requested container/payload allocations and excludes allocator
rounding, rusqlite statement internals and SQLite copies. The implementation must
use exact reserve/copy paths so geometric Vec growth does not exceed the formula;
if std String/Vec helpers cannot establish that path, use explicit pre-sized copy
helpers rather than silently assuming final len equals allocated capacity.

Meaningful tests, after two source reviews:

* Both signed extrema,0 and decimal-width transitions preserve SQL TEXT binding
  and canonical row values. Wrong UTF8, malformed/overflow numeric text, leading
  plus/zero, negative zero, excessive20-byte text and wrong SQL types retain the
  prior error categories, tested with the actual helper/query branch.
* Actual selected queries against the existing initialized native test DB record
  exact B/R charges using target size_of values. Empty parameters/no-row show no
  invented row allocation; representative blob/text/integer results match exactly.
* Refuse binding admission for an actual AuthorityClock update and prove the row
  did not change. For query pre-prepare ordering, use valid parameters against a
  deliberately missing selected table: refusal must return Bounds before the
  prepare would return Storage. This demonstrates the early gate without adding
  a SQLite extension/function dependency.
* Refuse row admission after successful binding and prove no result is returned;
  record that oversized/invalid first-row cells fail before any row-copy charge.
  Use fixed malformed-state fixtures, never a lower limit in production SQL.
* A deliberately broken host fixture with two AuthorityState rows exercises the
  real scalar SQL's extra-row check and retains Bounds after first-row admission.
  Normal tests use the genuine schema; the broken fixture is explicitly a host
  contract test, not supported schema admission.
* Run focused native tests, complete metadata native/differential tests and strict
  native lint. Shared source import/name changes must also compile on wasm32,
  but this native-only correction does not require claiming Wasm runtime coverage.
  Existing actual-celld catalog behavior is unchanged; S2 will independently
  requalify scalar transport when implemented.

Author review: all new admissions occur before their corresponding Rust-owned
allocations; validation/call/mode and canonical-error order stay intact; no-row
and extra-row behavior remain; no physical permit is released by the helper. The
remaining uncharged domain/JS/engine paths are named blockers to a complete List
allocation proof, not zero-valued trace terms. Root and storage independent plan
reviews passed before implementation. The three-file source snapshot in
`native-scalar-admission` passed author, root, and storage independent reviews;
its manifest and hashes are `/private/tmp/foks-native-scalar-manifest.txt` and
`/private/tmp/foks-native-scalar-sha256.json`. All nine focused tests passed, as did the full native metadata suite (98 passed;
one explicit source-profile generator remains ignored), strict native all-target
lint, default wasm32 compile check, formatting/diff checks and frozen-source hash
verification. No source correction or test failure occurred. Evidence is
`/private/tmp/foks-native-scalar-results.json`; these are native logical-allocation
and compatibility gates, not Wasm runtime or engine-RSS qualification.

## S2 proposed implementation boundary (awaiting review; no code)

S1 does not change the Wasm host. S2 will change its private scalar transport,
without changing `Value`'s public serde representation, pinned SQL/catalogs, or
FOKS frames. Move only the stack decimal helper into a shared private module; the
native helper keeps identical behavior. Add a private scalar JSON codec and tests,
then integrate it with `wasm.rs` and the existing JS qualification host. This is
separate from context decoding and from S3's domain/JS/engine allocation gaps.

### Outgoing parameters

Retain parameter validation, call increment/limit and read/write checks first.
Compute the exact UTF-8 JSON size in a checked borrowed pass. For an array, the
size is 2 plus commas plus each value's size; an empty array is exactly 2.
`Null` is the six-byte JSON string `"null"`; integer values are the fixed object
prefix/suffix plus stack-produced canonical signed decimal inside quotes. Blob
values use decimal bytes with exact one/two/three-digit widths and commas. Text
uses the exact current serde_json escaping: quote/backslash and the named short
control escapes take two bytes, remaining U+0000..001F take six ASCII bytes, other
UTF-8 bytes pass through. No intermediate JSON Value, String or decimal heap
allocation is permitted. Native/Node tests compare exact bytes to the unchanged
serde representation across all byte values, Unicode/control strings and signed
extrema.

Reject encoded size greater than 65,536 before allocation/import, matching the
existing JS scalar range limit. Admit exactly the encoded Vec's requested size,
use try_reserve_exact, and write into a fixed-capacity private writer that rejects
any length mismatch or overflow rather than growing. Drop these parameter bytes
immediately after the synchronous SQL import returns, before parsing its reply.
The caller-owned `&[Value]` remains outside this encoded buffer term.

### Incoming scalar data and parser allocations

Replace the generic DeserializeOwned entry with two explicit functions: a
catalog-aware `decode_query(spec, bytes)` returning Option<Vec<Value>>, and
`decode_changes(bytes)` returning u64. Both retain the 65,536-byte response bound.
Query accepts bare JSON null or exactly the declared columns in order. Per-cell
kinds and global4096 TEXT/BLOB bounds match NativeSql, including NullableBlob's
string `"null"`. Change count accepts the existing singleton integer object,
canonical signed i64 decimal text, then the existing nonnegative conversion.
Unknown/duplicate object fields, extra columns/fields, wrong kinds and malformed
canonical decimals fail with the existing host-data Storage classification;
negative change count retains IntegerRange. Allocation refusal/size arithmetic
uses Bounds. Tests must pin these distinctions rather than catch all errors.

Use strict DeserializeSeed/Visitor implementations with static errors (at most64
bytes), never derive-generated unknown-field or invalid-value formatting that
can echo a long string. Unlike the context codec, escaped JSON strings, Unicode
and legal whitespace remain accepted. The lexical preflight scans without heap
allocation, recognizes escapes and string boundaries, checks maximum nesting3
(row→cell object→blob array), and allows only canonical raw unsigned0..255 byte
number tokens (at most3 digits) outside strings. Bare null is allowed only as
syntax for the query result; normal seed validation checks position. This prevents
arbitrary_precision long-number maps and float_roundtrip long-number scratch from
being reached, while quoted i64 decimals remain subject to the integer visitor.
Escaped keys must remain comparable as decoded text; no String key is retained.

Before invoking serde, retain one conservative parser working allowance:
`P(N) = 3*N + 8 + 32 + 256 + 2048`, with checked arithmetic and N≤65,536.
The proposed scratch term covers serde's reusable decoded-string Vec plus an old
allocation during geometric growth: each requested next length is at most N,
so a growth allocation is at most max(2*N,8) and its previous allocation less
than N. The 32-byte term is the pinned arbitrary_precision short-number temporary;
256 covers two fixed error records and two fixed-message blocks; 2048 is an
explicit checked visitor/layout allowance. This is a retained live-working model,
NOT a claim that cumulative parser allocator traffic equals P(N). The source audit
must verify every string/escape/error growth path against serde_json1.0.151 before
this formula is approved; a changed implementation must revise the formula, not
silently assume it. Tests must enable arbitrary_precision and float_roundtrip,
exercise near-limit escaped unknown keys/invalid values, malformed escapes and
surrogate pairs, and inspect fixed error behavior outside the production path.

Result ownership has separate cumulative admission before allocation: exact
`columns*size_of::<Value>()` for a present row; each blob reserves/charges at most
4096 bytes once before consuming its bounded sequence; text reserves/charges its
validated decoded byte length before copying. Integer/null have no payload heap.
No present-row container is allocated for absent results. Blob fixed-capacity
slack is explicitly counted, not described as final-length-exact. Parser scratch,
response bytes and result buffers can coexist; their terms remain distinct. A
malformed result may consume admitted space and then fail; no allowance resets
inside the decoder. The caller retains its budget until resulting values drop.

### Shared RESPONSE slot and physical lifetime

`response_alloc` is shared by scalar SQL and the existing nested domain import;
S2 cannot silently break that reentry. Use a private response scope with explicit
Idle/ScalarWaiting/DomainWaiting phases and one allocation attempt per active
import. The only supported nesting is the existing DomainWaiting→ScalarWaiting
while nested `run` executes SQL; the previous domain slot must be empty and is
restored when the scalar scope finishes. No heap-allocated scope stack is needed.
All other reentry/duplicate/out-of-phase allocation is a failed invocation, not a
usable old buffer. Capture refusal in the active scope so a host ignoring a zero
pointer cannot make the Rust caller accept stale bytes.

Drop any previous buffer before making a new response allocation; do not use
clear() to retain its capacity. Admit the actual response length, use exact
fallible allocation and initialize every exposed byte. Take the initialized buffer
out after import and leave the slot empty. Normal error paths, including nonzero
import status, malformed JSON and admission failure, drop the buffer and restore
the enclosing scope. The existing domain reply also uses this bounded buffer
admission/lifetime guard because it shares the allocator; its serde reply decoding
and domain-owned data remain explicit S3 gaps. Domain wire/status behavior stays
unchanged and receives regression coverage.

A thrown JS import or Wasm trap does not unwind Rust destructors. Extend the
qualification host's existing page-export poison wrapper to scalar/domain entry
and allocation exports as well: any thrown export retires that instance, while a
normally returned typed status does not. The original transaction must still roll
back; no trap can be reclassified as a durable domain rejection. Tests inject
throw/trap after response allocation, assert subsequent use of that instance is
refused, and require a fresh instance for recovery. This does not release a physical
instance/worker permit early or claim a trap ran Rust cleanup.

### Qualification and exclusions

Author and independent source review precede tests. Run pure native codec tests
and comparison to unchanged serde bytes, exact allocation-refusal tests, native
metadata regressions, strict native/Wasm lint, default64MiB Wasm build, Node mocked
imports for duplicate/out-of-phase/missing/failed replies and nested domain
restoration, then existing actual-celld differential/authority/personal/page
vectors using the new exact artifact. Repeated large/small calls must leave no
response capacity retained across successful/error scope exit; traps prove instance
disposal rather than destructor behavior. Any qualification-only inspection API
must be feature-gated and absent from the default artifact.

S2 does not precharge JS JSON.parse/stringify/TextEncoder/Array.from, engine
materialization, rusqlite/SQLite workspace, or personal/authority higher-level
clones/serde data. S3 owns these terms. The same host file may receive a narrow
poisoning fix, but that is not a declaration that its JS heap is admitted. The
trace must name covered scalar Rust terms and uncovered S3 terms separately.

### S2 pinned scratch audit and failure-provenance refinements

The pinned parser source was inspected before proposing code: serde_json1.0.151
`de.rs:938` allocates String::with_capacity(16) in arbitrary_precision's
parse_any_number. Raw canonical byte tokens have at most3 digits, so that String
cannot grow and cannot reach the long-number map. `de.rs:1427` clears one reusable
scratch Vec before string parsing, and dispatches borrowed or copied strings to
our visitor. `read.rs:494..537` shows SliceRead's only escaped-string growth paths:
extend_from_slice for ordinary byte runs, then parse_escape. `read.rs:873` and its
unicode helper append at most the UTF-8 representation of the consumed escape;
with validated strings the decoded length never exceeds consumed input length.
No IoRead, raw-value capture, serde_json::Value tree or deserialize_bytes/raw-string
entry is used. The lexical scanner's escape handling must not mistake escaped
quotes for string boundaries or admit floating/sign/long raw-number paths.

The installed stable rustc and repository-pinned toolchain both identify as
1.95.0 (59807616e). The matching local rustdoc source for alloc/raw_vec/mod.rs
(grow_amortized at lines501..523) computes max(old_capacity*2, required_capacity)
with the u8 minimum8. While growing, old_capacity < required_capacity ≤ N;
therefore new requested capacity ≤ max(2*N,8), and old plus new requested storage
≤ 3*N+8. Vec extend_from_slice delegates through the ordinary reserve path. This
is an audit of requested Rust allocations for these pinned sources, not allocator
size classes or resident Wasm pages. Tests must pin both rust-toolchain.toml and
serde checksum, and qualification must be repeated when either implementation
changes; no stable Vec API promises this growth factor forever.

`error.rs:315` uses one boxed fixed ErrorImpl for syntax failures; no input text is
copied. `de.rs:1443..1470` can retain an existing visitor error while constructing
an end-map/end-sequence error; `error.rs:337` fixes position by moving the code into
a replacement record. Static custom-error message conversion at error.rs:436
can hold a String and its boxed-string replacement during shrinking. The existing
256-byte allowance covers two ≤64-byte records and two ≤64-byte static-message
allocations. Production code must neither format nor Debug these errors. Every
visitor entry, including copied escaped strings and wrong types, overrides default
invalid_type/invalid_value errors; those default formatters can include the input.
The proposed2048 fixed-layout allowance will have explicit compile-time size
assertions for the actual seed/visitor/refusal state; no map or owned key object is
part of it. This completes the source basis for the proposed P(N), subject to the
independent plan and source reviews of the eventual implementation.

Visitor allocation failure must not disappear into the static serde error. Pass a
borrowed per-decode refusal latch through seeds. Failed admission, checked sizing
or try_reserve_exact sets Bounds before emitting a static parser error. The outer
decoder first checks this latch, then maps ordinary syntax/type/canonical failure
to Storage. Use a separate structural latch only if needed to retain a documented
IntegerRange distinction; never infer provenance from error strings. There is no
thread-local error classification shared between independent calls.

Response-slot phase violations require an invocation-level sticky failure bit,
not merely a failing nested scalar result. A duplicate/out-of-phase allocation,
forbidden reentry, or ignored allocator refusal poisons the entire active top-level
invocation. Nested DomainWaiting→ScalarWaiting cannot save/restore this bit to a
previous healthy value. Before accepting any scalar/domain result and before the
top-level export publishes output, check the bit and return infrastructure failure.
Only a new top-level invocation with no outstanding scope and no trapped instance
may initialize a fresh bit. A foreign import may deliberately ignore a nested
error; tests must prove it still cannot produce a successful outer result. Traps
retire the instance in JS; resetting this bit is never permission to reuse one.

## S1 committed evidence

Commit9442ef8d contains the exact three reviewed/tested implementation files.
The isolated native gate passed98 tests plus nine focused tests (included in that
full count), with one explicit profile-source generator ignored. Strict native
all-target lint, default Wasm compile check, formatting and hash checks passed.
Root then integrated S1 with context codec235b8c95 and the combined native suite
passed109 tests, with the same one ignored generator. No test failure or source
fix was required. Evidence and source hashes are retained in
`/private/tmp/foks-native-scalar-results.json`, including the combined run log.
Only requested native scalar Rust allocations are covered; S2, S3 and the observed
TEMP-shadowing connection gate remain unfinished.


### S2 implementation and qualification evidence (2026-10-16)

The reviewed eighteen-file implementation is frozen in
`/private/tmp/foks-scalar-codec-manifest.txt` and
`/private/tmp/foks-scalar-codec-sha256.json`, on trace baseline `5aa3a7ad`.
Implementation commit: `5b6f86d6` (all eighteen exact frozen source hashes).
Root and recovery independently reviewed the codec, response state, physical
retirement, trace integration and the subsequent test/diagnostic corrections
before their affected gates ran. The full evidence ledger is
`/private/tmp/foks-scalar-codec-results.json`; no Markdown is staged.

The encoder preserves existing Value JSON bytes and admits the exact requested
buffer first. The strict borrowed decoder preserves escaped TEXT and canonical
i64 behavior, preholds the reviewed parser allowance and separately admits owned
rows/payloads, with explicit Bounds provenance. The allowance `3*N+2344` is a
conservative **live parser** bound, not measured or cumulatively charged parser
allocation traffic. Scalar/Domain response scopes own their buffers, drop empty
capacity on normal completion/failure, and retain invocation-sticky errors.
Shared helper guards reject invalid callback phases. A thrown export retires
only after the outermost call unwinds; setup and runtime tests verify actual
memory wiping and a null instance, rather than only a poisoned flag.

The feature trace explicitly emits failure diagnostics, never domain payloads:
any sticky error clears List/frame sizes and hashes and forces a non-success
outcome. A recorded Bounds refusal retains `trace_refused` and its first index.
Private scalar and page refusal tokens can originate only from a normal allocator
zero return and are caught inside their matching import. A matching-code host
exception or real allocator trap still retires the instance.

Validation results:

- Native pre-trace suite: 124 passed. Final trace union: 132 passed, one intentional
  exporter test ignored. Optional serde arbitrary_precision/float_roundtrip scalar
  matrix: 24 passed; corrected boundary fixture rerun: one passed.
- Strict native all-target lint and default/trace/token Wasm lint passed. Three
  release artifacts have initial 18 and maximum 1,024 Wasm pages (64 MiB maximum),
  with immutable paths and SHA-256 digests recorded in the ledger.
- Node: base host 9, scalar 5, token 10, page 8 (including 37 vectors), and directory
  1 (including eight sizes through 16 MiB) passed. Trace: **43 passed, one failed**.
- Actual local celld: 123 selected cases passed across differential, authority,
  errors, faults, settlement, bindings, normalization and pages. Six suites retain
  their original passing worker hash. The two rerun suites include normal and fault
  cases under the final diagnostic-only worker hash; exact hashes are in each
  qualification output. This remains local emulator semantic qualification, not
  production storage, recovery, distributed ownership or latency qualification.

Failures remain visible. Native strict lint initially rejected an unnecessary
clone in a test; the reviewed `slice::from_ref` correction passed strict rerun.
The first differential/fault celld attempts stopped because the fixture accessed
`kernel.instance.exports` after correct retirement set instance to null. The
reviewed worker now reports nullable live memory and distinct retired-byte/wipe
completion evidence and strengthens its pass predicate with retirement validity.
Both original failed logs remain; fresh differential 17 and fault 7 case reruns
passed without changing expected replies, outcomes or full database snapshots.

The full-range trace still fails `large_parent_cycles-0` in the existing RPC frame
allocation under the unchanged 64 MiB maximum. It recorded 56 of 58 native oracle
cases; two later cases in that same fixture were not reached. Physical retirement
wiped all 52,559,872 currently allocated bytes. The trace report and log are
`/private/tmp/foks-scalar-trace-node-a.json` and `.log`. S2 does not resolve that
failure or establish a complete List memory allowance. F1 framing, P1 page
ownership and the explicit S3 domain/JS/engine gaps remain separate work. The
user's subsequent choice of a restricted hosted profile does not change this
unchanged full-range test result or native protocol limits.
