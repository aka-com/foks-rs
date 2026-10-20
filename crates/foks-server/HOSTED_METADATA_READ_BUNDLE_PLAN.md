# A2: bounded personal read bundles and native/Wasm binary transport

Draft 2026-10-15. Follows reviewed A1 and the vertical-slice plan. Markdown stays
unstaged. This is a plan, not a claim that a persistent hosted read path exists.
No public FOKS message, cryptographic bytes, historical verifier, List page cap,
or Go client cursor changes. Separate plan review and two code reviews precede
implementation qualification and each implementation commit.

## Why another boundary is required

The fixture ABI returns one row through JSON number arrays, caps each imported
SQL response at64KiB and counts2048 statements. Public List supports4096 entries,
optionally carrying encrypted small-file bodies. Raising one fixture limit does
not provide bounded memory or preserve List semantics. Assemble one public page
from bounded private pages and prove a fixed revision before publishing it.

The current historical Get query is also not history-independent. A read-only
Python SQLite probe using the actual query and table/index definitions measured
72 /1656 /160056 VM steps for1 /100 /10000 versions of one name. SQLite used
`kv_dirent_listing` plus a temporary ordering B-tree. Evidence lives in
`/private/tmp/foks-historical-name-cost.{py,json}`. These are preliminary engine
measurements, not native/celld qualification or disk latency claims.

## A2a: bounded historical lookup

Review a schema57 index on historical rows covering
`(uid,parent_id,name_mac,directory_version,version DESC,dirent_id ASC)`.
Keep historical lookup separate from the live-name projection: tombstones and
old directory versions retain their current meaning. Do not silently replace
Get with a head-only lookup. Preserve the existing join to the exact historical
directory row and the public Get omission of creation time.

The current query orders only by entry version, leaving duplicate-name version
ties implicit. The current index walks dirent IDs ascending before sorting by
version. Before changing the query or allowing the optimizer to use a new index,
qualify duplicate names/equal versions against the supported native baseline and
pinned Go behavior. An explicit `version DESC,dirent_id ASC` tie rule is proposed
only if those tests confirm equivalence; otherwise stop and revise this plan.
Do not describe an unspecified SQL tie order as a protocol guarantee.

Measure actual catalog/native SQL at1/100/10000 versions, matching and absent
name/directory-version combinations, and duplicate-name ties. The expected shape
is an index seek and first matching row, with no retained-history scan/sort.
Include index write amplification, schema56→57 atomic migration/backup restoration,
projection consistency and downgrade rejection. Update authoritative schema and
activation catalog hashes together. Retained encrypted content is unchanged.

## Public Usage is distinct from quota charging

A1 review found that the existing four namespace charging counters are not the
public KvUsage result. A1 names its private point `QuotaCounters`; it must never
be encoded as public Usage. Native KvUsage counts all retained small/symlink nodes
and their decoded ciphertext lengths, then all large uploads and their existing
chunk_count/chunk_bytes sums. Current native small-node and large-file enumeration
is also history-dependent.

A2 requires a separate reviewed migration for exactly those five semantic totals
(small count/ciphertext bytes, large count/chunks/chunk bytes), independent of quota
charging policy. Proposed design: backfill by bounded native decoding under the
quiesced migration transaction; add validated small ciphertext length per node;
maintain per-namespace aggregates transactionally through every node/file/chunk
insert, update, replacement, cascade and reclamation path. Rust decodes and binds
the per-node length; JavaScript never parses FOKS ciphertext containers. Typed
writers must update exact bytes and derived length atomically. An unchecked legacy
SQL writer cannot coexist with this invariant. Group U preserves malformed in-bound historical payloads as an explicit NULL
validation sentinel; public Usage returns the existing native corruption error
until typed exact-byte revalidation succeeds. Oversized stored payloads fail
migration before decode. No guessed length contributes to the totals.

Keep the native enumerating implementation as the qualification oracle. Compare
exact public values after replay, conflict rollback, replacement, cancelled/partial
upload, retained versions, deletion and actual reclamation; exercise cascade
trigger ordering to prevent double subtraction. Include zero namespaces, integer
overflow, backup predecessor restoration and Go public Usage checks. Freeze the
exact schema/trigger/typed-writer inventory before implementing; do not squeeze
this into A1 or assume quota counters are an acceptable wire behavior change.

## A2b: bounded private page ABI

Keep scalar catalog statements and write-domain rollback behavior intact. Append
explicit catalog entries; never renumber existing IDs or route a numeric range
implicitly. Add page-query support to the shared Sql abstraction with a declared
row/column/byte contract, enforced by native and JavaScript hosts before copying
payloads. Fixed internal pages contain at most64 rows, with a separate byte cap;
large individual rows outside the already qualified profile fail explicitly.

Replace JSON number arrays for page results with a versioned private binary row
codec. Header carries magic/version, response kind and row count. Each cell has an
explicit type plus checked fixed-width integer or length-prefixed bytes. Signed
SQLite integers remain exact i64 throughout; celld's SQL-facing CAST-to-text adapter
parses canonical decimal to BigInt before binary encoding. No JS Number round trip.
UTF-8 text, nullable cells, widths, count, total length and trailing bytes are checked.
Reject a claimed length before reserving/copying it; test arithmetic overflow and
truncated input. Freeze concrete byte-order/header/cell tags in a codec-specific
subplan and fixtures before coding. Catalog fingerprint includes the row contracts.

SQL must return length before a CASE-bounded blob, so an oversized stored value
cannot first allocate an unbounded JavaScript blob. Start with a1MiB private-page
ceiling, subject to the concrete64-row column accounting and memory measurement.
Native and Wasm must use identical predicates, private continuation and role checks;
no JavaScript authorization or domain business logic. An unknown ABI/catalog version
fails activation. This is private adapter compatibility, not FOKS wire compatibility.

An invocation returns only one bounded read page; the native frontend accumulates
at most the requested public response. Keep the64MiB Wasm maximum and enforce
actual host/runtime memory in qualification. Do not build4096-entry JSON trees in
Wasm and then another full JSON tree in JavaScript. Binary body transport must
preserve exact encrypted/signed payloads and all existing public frame limits.

## A2c: fixed-revision bundle orchestration

A bundle binds installation, schema/kernel/catalog/storage/key activation,
owner/epoch, tenant domain revision, authenticated exact peer context, route,
public arguments and original absolute deadline. The private continuation is
opaque frontend state, never a public FOKS cursor or caller-selected authority.
Each page authenticates current credential/profile and validates the same fence
and domain revision inside its own transaction. No database snapshot/reader lease
is held across HTTP or object I/O. Any intervening mutation rejects the whole
bundle; no automatic switch to a newer snapshot or mixed file/page result.

List uses schema56 live indexes. Initial/MAC public cursors preserve name order
and strict public MAC boundary; Time preserves inclusive public timestamp boundary.
Private continuations use the complete `(sort_key,dirent_id)` tuple so duplicate
names and timestamp ties cannot be lost between internal pages. Determine public
`final_page` exactly as native (`returned_count < requested_limit`), including an
exactly full final page. Preserve extended-entry positions relative to the full
public page, not each internal batch. Missing or unauthorized optional small-file
bodies are omitted while their dirents remain; malformed stored bodies retain
existing infrastructure/error behavior.

Get preserves up to64 supplied historical name alternatives in their original
order, historical directory-key authorization, follow modes, and absent/tombstone
semantics. Root/GetDir/GetNode/Usage and cache/precondition decisions reuse A1 scalar
primitives, adding no silent limit or unsupported status to an ordinary client flow.
Large-file/external-node support remains explicitly outside the initial small-KV
profile until its complete route and object-store contract is implemented.

Before publishing any bytes, perform a final transaction-local current-authority,
fence, activation and fixed-revision proof, then wait for required durability/sync
and check the original deadline. That proof is the read linearization point; do
not claim promotion can retroactively cancel an already-proven read. Discard the
whole candidate on any failed proof. Hold native working/output permits through
final frame construction and socket transmission, including detached physical work.
Charge internal HTTP buffers and assembly copies before allocation. Existing16MiB
public frame ceiling and response admission rules remain authoritative. A larger
logical result fails as native would; never truncate it to manufacture success.

## Qualification and commit groups

1. Historical-query evidence and index/migration group, if equivalence is proved.
   Author/root and independent reviews, then SQLVM/native/backup/Go regression gates.
2. Concrete binary codec/Sql-page ABI subplan review; shared codec/host implementation
   with malformed/oversized/negative/exact-i64 tests and native/Wasm differential
   rows. Exercise real celld, memory growth/detached views and existing rollback
   origin/fault tests. No production route is enabled by this group alone.
3. Personal bundle resolver plus native assembly, two reviews then tests. Cases:
   empty/single/64/65/4095/4096/4097 entries; duplicate names; timestamp ties;
   maximum valid encrypted small bodies; optional unreadable/missing bodies;
   malformed stored fields; historical Get alternatives; exact public bytes and
   final-page flags versus native and pinned Go. Pause between private pages and
   revoke/demote/rotate unrelated key/change directory/promote owner: the whole
   public candidate must fail without mixed output. Test final-proof and slow-output
   deadlines with barriers. Record peak Wasm/native RSS and all copy/reservation
   ceilings, not only codec payload sizes.
4. Integrate into B/C only after these gates; qualify end-to-end15s ordinary and
   recovery success using persistent state, then capacity. Fast fixture results
   cannot stand in for complete route coverage or host/AZ recovery.

Author review: this plan preserves the public4096 limit and cursor semantics,
separates historical name lookup from live projection, identifies its measured
scan instead of raising an operation budget, and requires a final consistent
revision proof plus allocation ownership. Open review gates: historical tie
semantics; exact binary ABI/cap accounting; native error/overflow parity; A1
credential/SSO/activation dependency completeness; production endpoint timing.


Independent staged-plan review (storage): conditional PASS with public Usage
semantic counters kept separate (now recorded above), and a mandatory cap audit
before the binary codec subplan. A1's4096-byte scalar fixture cap is not blanket
proof that every native directory/node fits. Inspect actual decoder/storage/public
limits and configured variants, including64KiB node policy, arbitrary directory
seed payloads, and public4096-entry server List versus the Rust client's chosen
100-entry pages. Support the valid range or explicitly retain a hosted-profile
restriction through B/C; do not silently reject native-compatible values while
claiming the profile complete. Concrete codec layout/cap accounting remains a
separate pre-implementation review gate.


## Concrete A2b codec contract (reviewed and qualified)

H is committed as 9e8632e9 and Group U as a01bf92c. A2b was qualified
on that tested schema58 baseline. This preparatory
commit adds a private page transport and qualification exports. It does not enable
a production route, authenticate a frontend service, or claim deadline interruption
of synchronous SQLite. A2c and the persistent B frontdoor remain separate gates.

The cap audit finds native Config rejects node limits above64KiB and dirent limits
above4096 bytes (`server-db/connection.rs`). Directory exact bytes instead use
Config.maximum_blob_bytes (default16MiB) in validate_directory; the earlier draft
incorrectly grouped them with small nodes. Public List clamps requested counts to4096; the Rust client's
100-entry choice and8MiB client decoding policy are not server limits. A2b must
therefore transport full64KiB small-node values and4096-byte dirents. A2b
can qualify directory payloads only up to its declared64KiB profile; native-valid
larger directory values remain a mandatory A2c compatibility gate. It must not reuse A1's4096-byte scalar-host cap for these payloads.
Native public response framing/allocation remains the final limit; a4096-entry
request is not permission to allocate an unbounded response.

### Binary envelope and row contracts

Use a separate `page` module with no serde/JSON representation of row payloads.
Version1 is little endian. The fixed24-byte header is:

| Offset | Field |
| --- | --- |
| 0 | Four bytes `FKPG` |
| 4 | u16 codec version, exactly1 |
| 6 | u16 kind:1 parameter row,2 result rows |
| 8 | u32 pinned catalog statement ID |
| 12 | u16 row count (parameters exactly1, result0..64) |
| 14 | u16 column count, equal to catalog contract, at most16 |
| 16 | u32 body byte length, exactly total bytes minus24 |
| 20 | u32 reserved, exactly0 |

Cells are row-major: tag0 NULL (no payload), tag1 signed i64 as eight
little-endian two's-complement bytes, tag2 BLOB (u32 length then exact bytes),
tag3 UTF-8 TEXT (u32 byte length then validated UTF-8). No floats, booleans,
indefinite lengths, padding, trailing bytes or unknown tags are accepted. Null is
accepted only by a declared nullable column. Integers include i64::MIN/MAX;
unsigned domain fields still use checked conversion after decoding. Statement ID,
kind, column count and per-column maximum are validated before exposing rows.

Total result envelope is at most1,048,576 bytes including header. Parameter
envelopes remain at most64KiB and each parameter has its own catalog bound. Check
all count/length arithmetic before reservation or copying. Native/Wasm decoders
first validate the complete envelope and offsets, then expose borrowed row/cell
views; owned copies occur only where the shared FOKS decoder or public assembler
requires them. There is no Vec<Vec<Value>> mirror of a whole page. The native
encoder writes directly to one capacity-checked buffer; the JS encoder also
preallocates only the declared bounded buffer and uses DataView.setBigInt64.

Append explicit `PageSpec` contracts without changing existing scalar statement
IDs or scalar/domain behavior. A contract pins SQL, parameter types/bounds, result
types/bounds, maximum rows and maximum encoded bytes in the catalog fingerprint.
New `Sql::query_page` accepts only page statement IDs; a host without page support
fails closed. No implementation translates an arbitrary raw SQL string. Existing
2048 logical-call accounting remains per invocation; one page query consumes one
call in both hosts, and existing nested-domain control charges stay unchanged.

Every variable SQL payload column has a preceding length column and an exact
`CASE WHEN length(...) <= column_cap THEN ... ELSE NULL END` projection. The
shared kernel rejects a positive oversized length or an unexpected NULL before
FOKS decoding. The JS host never receives an oversized payload merely to reject
it. SQL LIMIT is chosen before execution from the worst-case encoded row size:
`min(64, floor((1048576-24)/maximum_row_bytes), remaining_requested_rows)`.
Consequently a page with64KiB optional bodies generally contains fewer than64
rows. A shorter private batch caused by this byte contract is not end-of-list.
Do not fetch64 maximum-size rows and trim them after allocation. Cursor iteration
must remain bounded and must not call toArray on an unbounded result. Read one
extra cursor row only when needed to reject a violated SQL row limit; that row
has the same bounded projection.

### Wasm/host boundary and first qualification statements

Add a distinct synchronous `foks.sql_page(id, ptr, len)` import and a
`page_response_alloc(len)` export with a dedicated owned PAGE_RESPONSE buffer.
Rejected allocation clears that buffer, and the host clears previous results
before an invocation. Rust holds no RefCell borrow or slice into that buffer
across an import; parameter bytes are separately owned and immutable until the
import returns. JS copies/validates request bytes before any Wasm allocation,
then obtains fresh memory views after allocation/growth. Page read exports cannot
reenter top-level execution or invoke domain callbacks. Imported exceptions and
codec/budget errors remain infrastructure failures that abort the outer
transaction; they cannot become durable domain-rejection receipts. Keep the
64MiB declared Wasm maximum and actual artifact verification.

First add a qualification-only type probe and bounded actual current-list row
queries (initial/MAC/time plus private tuple continuation) against initialized
schema58. Shared selection predicates must be exact counterparts of native
current-list SQL; the page output additionally carries dirent ID and sort key.
Keep authorization out of JS: the A2b probe is fixture-only and is not a personal
read endpoint. Extend the existing actual-cell fixture host only for this distinct
probe, never selecting it through an ordinary personal command.

Native and Wasm compare exact encoded page bytes, then decoded native values,
for0/1/63/64 rows; null versus empty blob/text; invalid UTF-8; all integer extrema
and values above2^53; exact per-column and1MiB boundaries; malformed header,
truncation, overflow, extra rows/columns, unknown statement/version/tags and
trailing bytes. Large fixtures use genuine encoded FOKS directory/small-node
payloads, including valid legacy values above the new-write2064-byte limit.
Exercise memory growth/detached JS views, repeated rejected allocations, injected
SQL errors, rollback and traps. Measure bytes/copies and peak native/Wasm memory
at maximum valid pages, separately from wall-clock latency. Run all prior scalar,
rollback-origin, authority and personal qualification regressions unchanged.

### Concrete A2c handoff

After A2b passes and is committed, author a separate fixed-revision bundle change.
The shared kernel authenticates the real peer and authorizes the parent/key in
each read transaction. The native assembler retains one original caller deadline
and an opaque bundle binding containing activation, owner/epoch, domain revision,
exact peer digest, route, canonical arguments, requested public count and consumed
position. Private continuation contains the complete sort tuple and is bound to
that bundle, never accepted as a free-standing authority token. In the fixture
this is an owned typed value; before the persistent HTTP frontdoor it must become
an authenticated capability or server-owned state with a bounded lifecycle.

The first complete route is personal List, including optional readable type3
bodies, exact extended positions and the4096 public cap. Optional missing or
unreadable bodies preserve dirents. Symlinks are not included as optional List
file bodies, matching native. Use full sort tuples for ties; preserve strict MAC
and inclusive public timestamp boundaries. Fetch another internal page only when
the previous reached its effective SQL row limit and the public requested count
is not met. At requested count, final_page is false even if no further row exists,
matching native. A short final internal page yields final_page=true only when the
public requested count was not met.

A final same-revision/current-authority/activation/fence transaction precedes
publication, followed by durability and the original deadline check. Any page or
proof mismatch discards the whole candidate; no implicit restart at a newer
revision. Test revocation, role/key changes, directory mutations and promotion
between pages and before final proof. Read linearization does not retroactively
cancel an already proven response after later promotion. Preserve native public
frame/memory admission through assembly/transmission; do not encode a4096-row
JSON tree in Wasm. Public Get alternatives, large scalar directory/node reads,
precondition/cache vectors and error-vector bounds remain explicit subsequent
route gates before claiming a complete small-KV hosted profile.

### Ownership, review and test order

Recovery owns `metadata-core/src/page.rs`, shared Sql/native/Wasm page plumbing,
new private page fixtures, and metadata-spike host/Worker additions. Storage owns
new catalog SQL/page contracts and native query oracle/cost fixtures; coordinate
catalog declarations before edits. Root owns the eventual native public assembler,
publication/memory/deadline integration and pinned Go public route oracle. No
server listener/backend selection is enabled in A2b.

1. Review this exact codec/host contract independently before implementation.
2. Author review and independent implementation review before builds/tests.
3. Shared native codec/property/malformed tests, actual schema row/cost tests,
   Wasm build/lint, actual-Wasm Node tests and real celld differential pages.
4. Preserve failed runs and exact artifact/catalog/source hashes; commit only the
   tested A2b files, leaving Markdown unstaged.
5. Review A2c route implementation plan against the passed page artifacts before
   coding. Its acceptance includes native+Go public bytes/cursors,4096 entries,
   bounded copies, interruption barriers and all-page authority proof. Persistent
   B and real frontend/cell co-failure remain separate prerequisites for HA claims.

Author review: the fixed envelope uses exact signed integers and checked lengths,
the SQL LIMIT is bounded by worst-case payload before materialization, and page
budget semantics cannot accidentally alter public final_page. Full small-node/dirent
sizes are supported by the codec without silently widening the scalar fixture;
larger native-valid directory payloads remain an explicit route gate.
The persistent capability, complete route inventory and synchronous-SQL deadline
limits are explicitly deferred gates, not completed by this preparatory codec.
Root and storage independent plan reviews passed after the refinements below.
A2b author and independent source reviews passed. The frozen16-file implementation
passed40 native tests,37 Node/actual-Wasm tests, strict native/Wasm lint and112
actual-celld vectors (26 pages,17 differential,69 authority) on2026-10-16.
See the qualification record below. No production List route is enabled.


Root A2b plan review requires the following concrete refinements, incorporated
before implementation. Variable SQL TEXT caps use UTF-8 byte length
`length(CAST(value AS BLOB))`, not SQLite's character-counting length(TEXT).
Payload projections check `typeof(...)` as well as length before materializing a
BLOB/TEXT. A separate bounded type probe distinguishes an actual SQL NULL from
wrong-type or oversized payload suppressed to NULL by CASE; none may silently
become an absent optional body. Native and JS validate the same declared probe,
byte length and payload contract.

PageSpec construction validates checked worst-row arithmetic and requires that
at least one maximum legal row fits the page; an impossible contract fails
activation. Computed SQL LIMIT must never become zero and masquerade as an empty
list. `Sql::query_page` returns one owned bounded encoded byte buffer; validated
borrowed row views borrow only that returned value. No SQL host/RefCell borrow
crosses an import. The assembler retains at most one private page plus explicitly
charged public assembly and required bounded decoder temporaries; it does not
accumulate a vector of private pages. A new private ABI version/fingerprint binds
the page import/export signatures, codec version and every PageSpec. Hosts lacking
the exact fingerprint reject activation rather than attempting scalar fallback.

The optional extra cursor row used to detect a violated row limit has an explicit
transient reservation for one maximum encoded row before advancing/materializing
it, even though it is never copied into the accepted page. The memory accounting
therefore includes the owned page buffer plus one bounded cursor row and decoder
temporaries. If that reservation is unavailable, fail closed before advancing;
do not exceed the advertised working-memory budget for a validation check.

A2b implementation accounting clarification: native query-page admission calls
`foks_snowpack::reserve_allocation` before binding clones, query preparation or
cursor advance. Its conservative charge includes binding payloads/containers,
one maximum cursor payload and per-row cell containers. The result encoder reserves and charges header+requested_rows*maximum_row_bytes
once, before allocation, then appends without growth. Other codec allocations are
charged separately before allocation/growth (including replacement capacity), so
these are cumulative admission figures, not an assertion about actual peak RSS.
A rejecting-budget test uses a database without list tables to establish that
budget rejection precedes SQL preparation and returns an error, never an empty
successful page. No unused byte vector is allocated merely as a reservation.
The private JS host reserves a checked declared working bound before parameter
copies/encoder/cursor work, including UTF-16/UTF-8 temporaries and containers,
and releases it on every return/error. Its declared peak is reported separately
from Wasm memory and measured RSS. This is a per-invocation qualification bound;
node-wide pooling and SQLite engine working memory remain production gates.
A host exception or trap escaping page allocation/execution exports poisons that instance;
both page and scalar entry then refuse reuse because Rust destructors may not
have run. Recovery constructs a fresh instance. Normal returned Rust status
errors remain reusable, and tests distinguish these paths.
Page fixture setup/snapshot may carry a65,537-byte corrupt legacy node solely to
prove SQL suppression of a value one byte above the65,536-byte result cap;
ordinary fixture seed limits and the page transport bounds remain unchanged.

## Concrete A2c List subplan — awaiting independent review

Prepared 2026-10-16 against a01bf92c and the in-progress A2b API. This group
qualifies personal List; it does not silently declare Get alternatives, team/SSO
profiles, the persistent frontdoor, or whole-route coverage complete. Implement
only after the parent-authority continuation decision below is reviewed. Keep
Markdown unstaged and commit tested implementation groups separately.

### Native oracle and parent record limits

The native List implementation decodes the complete stored parent directory,
then checks its decoded key. Unlike the current A1 scalar Directory helper, it
does not additionally require decoded ID/version to match the selected SQL
columns. Preserve that distinction in differential cases. The record's seed
ciphertext is variable length. Config.maximum_blob_bytes, default16MiB, bounds
the directory; the64KiB node policy does not. A2b's List rows never contain the
parent directory, so its1MiB page cap does not resolve this issue.

Add a pinned directory-size/type query and bounded substring query to the
private catalog. Probe before payload materialization; require a BLOB and a
length within the activated database limit. Read chunks of at most512KiB in one
SQL transaction into a single exact-capacity buffer. Reject a short, extra, or
inconsistent slice. This is one selected record, not a database reconstruction.
Decode using the existing Rust KvDirectory decoder, extract the key, and drop
the full decoded directory and input before querying List rows. No JavaScript
Snowpack parser and no tightened historical verifier. The configured directory
limit is part of activation; deployments above the qualified limit fail
activation explicitly rather than silently changing per-request semantics.

Qualification must show the default16MiB valid parent fits the64MiB Wasm maximum
including input, decoder/tree/schema copies, allocator high-water, and host-page
buffers. A payload-only sum is insufficient. Malformed large records must return
the existing infrastructure/corruption outcome with transaction rollback and
bounded memory. If the existing decoder cannot meet that gate, stop this slice
and review a shared decoder allocation refactor; do not raise Wasm memory or
lower the accepted directory range to conceal the failure.

### Bundle identity and authority continuation

Proposed interface: BeginList, NextListPage, FinishList. The native frontend
owns a non-public bundle handle that fixes exact peer/certificate context,
installation and activation, owner/epoch, revision, namespace, route, normalized
public arguments, original deadline, requested count and derived parent key.
Only BeginList constructs it. It is never accepted from a public RPC or encoded
as a public cursor. Each page and final proof reauthenticates the current peer,
checks the fixed fence/revision and supported profile, and checks current access
to the original parent key. Optional small-file access is checked by the shared
Rust kernel against the freshly resolved credential, never JavaScript.

Use an authority-issued opaque token as specified below. The existing frontend
already proves mTLS possession and supplies exact PeerCertificate parts, but this
does not by itself make an arbitrary deserialized parent key authoritative. A
plain digest proves equality, not provenance. The ephemeral token key introduces
no secret persistence or metadata-backend dependency.
Repeatedly decoding a16MiB parent for every15-row page is a fallback to measure,
not an assumed15-second solution: a4096-entry page needs up to274 private pages.

Every mutation of a dependency must advance the compared revision, including
credential/key/profile changes and maintenance. Fixture-only tests must explicitly
advance it and identify that assumption. Persistent activation remains blocked
until the complete write inventory enforces it; direct local SQL or mixed old
writers cannot coexist with this proof. Rechecking credentials helps revocation
but does not replace revision coverage for directory/content changes.

### Page processing and response ownership

Preserve the native zero-or-over4096 count clamp, strict MAC start, inclusive Time
start, complete private continuation tuple, and exactly-full final_page=false.
Fetch no more than min(remaining_count, contract.effective_rows()). Do not read an
extra public entry just to change the native final_page behavior. Enforce forward
continuation and response binding before advancing. A deadline, fence/revision
change, revoked credential, malformed page or out-of-order page discards all
accumulated output; never restart into the same output buffer.

Decode each dirent using the existing Rust schema and choose an optional small
node using the stored SQL node_id, as native does. Missing or unreadable small
bodies are omitted; malformed present bodies fail. Re-encode validated schema
values where native normalizes fields; blindly concatenating stored bytes is not
necessarily native-response equivalence. Extended positions count entries across
the entire public page, including entries whose optional body was omitted.

The assembler owns one private page at a time and a charged public candidate.
Charge encoded capacity, schema trees, copies and envelope/framing before each
allocation; retain permits with detached work and through actual socket flush.
Use the existing16MiB public framing rule, including envelope overhead. Do not
allocate4096 maximum bodies and discover the frame limit only afterward. A
bounded encoder may be needed; review it independently against KvListResponse::new
before using it. No public byte is published until FinishList has re-proved
current authority, the exact original binding and revision, completed required
durability synchronization, and checked the original deadline. This is a read
linearization point; later promotion does not retroactively cancel a proved read.

### Review, tests and commit boundaries

1. Directory transport/decoder qualification and native List oracle. Review twice
   before tests. Test type/length guards,512KiB boundaries,64KiB-plus and16MiB valid
   parents, malformed fields and mismatched decoded ID/version, full native/Wasm
   error parity and measured memory. No ordinary route activation in this group.
2. Reviewed bundle provenance plus shared List processing/native bounded assembly.
   Review twice before tests. Compare native exact bytes and pinned Go fixtures
   for0/1/14/15/16/64/65/4095/4096/4097 entries; duplicate name/time ties; both cursor
   modes; missing, unreadable, malformed and maximum small bodies; frame boundary.
   The15-row boundary comes from the concrete A2b maximum-row contract, not a
   replacement public page limit.
3. Deterministic barriers between pages and before final proof: revoke/demote,
   rotate keys, change parent/dirent/body, change profile, promote owner, change
   activation, expire deadline, reorder/replay a continuation and exchange bundle
   identities. Assert no mixed or partial public output. Test late physical work
   and slow socket output retain memory ownership. Run actual celld native/Wasm
   differential cases, then commit implementation only after both reviews/gates.

Direct plan review: identified the16MiB parent compatibility gate, native decoded
ID/version behavior, private continuation provenance, complete revision dependency
inventory, and per-allocation response cap. Independent review is still pending;
the continuation provenance decision is not approved for implementation yet.

### A2c token contract for second plan review

Use a separate private HMAC-SHA256 read-token domain; this changes no public FOKS
crypto, signature, Snowpack message or Go client contract. The shared Rust kernel
owns MAC generation/constant-time verification, using the existing locked HMAC
dependency and sha2. JavaScript treats token bytes as opaque. A non-serializable
ReadTokenAuthority owns a zeroizing32-byte key and16-byte key-generation nonce.
The authority host generates both with its OS cryptographic RNG when activating
the kernel. A dedicated host-owned setup path injects them before request entry;
the private frontend request schema has no key/nonce setter. No automatic all-zero
or deterministic fallback. Native qualification supplies explicit test keys;
production construction requires a distinct RNG-backed constructor.

Erase every setup-key copy, including the host RNG buffer and dedicated Wasm
setup input, after transfer; zeroizing only the retained Rust field is insufficient.
An imported host exception or Wasm trap escaping secret/token work poisons the
instance. Destroy and recreate it with a fresh key/nonce; do not depend on Rust
destructors or RefCell guards unwinding across that trap.

The host serializes kernel invocations and forbids setup/reentry while an
invocation is active. Setup clears old secret and response state. A Wasm restart,
authority-host restart, activation replacement or explicit key rotation creates a
new key/nonce. Tokens do not survive that event: report a typed read-bundle-lost
error and discard the entire candidate. A frontend may start a whole new read
only within the original operation deadline and with empty assembly. Do not load
a token key from S3, replicate it, retain previous keys, or add a durable replay
ledger for these read-only continuations. This deliberately keeps the design
independent of the production metadata ownership backend.

The token is exactly180 bytes:148 payload bytes followed by32 MAC bytes. All
multibyte token integers are little endian. Reject any other length before
allocation or parsing. MAC input is the ASCII domain
`foks-metadata-list-token-v1` followed by a zero byte and all148 payload bytes.
Validate the MAC with a constant-time library operation before consuming the
derived authority fields. The payload layout is:

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 4 | Magic `FKLT` |
| 4 | 1 | Version1 |
| 5 | 3 | Reserved, all zero |
| 8 | 16 | Authority key-generation nonce |
| 24 | 16 | Random bundle identity supplied by the host |
| 40 | 32 | Exact request-binding digest |
| 72 | 8 | Derived parent-key role protocol value |
| 80 | 4 | Derived parent-key signed visibility |
| 84 | 8 | Derived parent-key generation |
| 92 | 2 | Remaining requested entries |
| 94 | 2 | Entries already returned |
| 96 | 2 | Next expected page ordinal |
| 98 | 1 | Cursor kind:0 none,1 MAC,2 Time |
| 99 | 1 | Flags:bit0 complete,bit1 native public final_page; others zero |
| 100 | 32 | Last MAC, or8-byte timestamp followed by24 zero bytes |
| 132 | 16 | Last dirent ID; all zero when cursor kind is0 |

The binding digest is SHA256 over Rust's versioned canonical serialization of a
fixed typed struct containing: domain/version, complete Fence (identity,
activation including limits, owner/epoch), compared revision, exact peer context,
SQL-selected namespace and parent ID, route discriminator, normalized initial
public cursor/count/load_small, original deadline value and trusted clock-domain
generation. Use typed structs and the existing Number decimal representation;
never hash arbitrary frontend JSON member ordering. Pin golden bytes/digests.
Current qualification's trusted host time is not a production cross-host clock
proof; D still must qualify that deadline envelope and its clock-error margin.

Begin verifies authority and reads/decodes the SQL-selected parent itself. Only
then may it issue a token with that derived key; never MAC a key supplied by the
frontend. SQL parent identity is bound separately from decoded parent ID/version,
preserving native mismatch behavior. Next verifies MAC, nonce, all request binding
fields and tuple/count/ordinal invariants, then rechecks current SQL authority and
key access before obtaining one page. Return a page envelope containing bundle
identity, consumed ordinal, binding digest and the next token. Frontend checks
these against its owned expected state before appending any entry. Finish accepts
only a complete token and repeats the current proof, including for an empty list.

Counts satisfy returned+remaining=original normalized request count, each at
most4096; ordinals advance with checked arithmetic. The token starts at ordinal0
with no private continuation. A short private page sets complete and
final_page=true. Exhausting the requested count sets complete and
final_page=false, including an exactly full last page. Cursor kind must match
the original ordering after a nonempty page. Enforce canonical zero padding and
valid role/generation encoding. No fields are ignored for token binding.

Stateless tokens are not single-use. An exact page retry at the same revision
may return the same page. The frontend's expected ordinal/token prevents duplicate
append and detects out-of-order or cross-bundle responses. No retry may advance
from an older token into already accumulated output. Promotion, dependency writes,
key loss and expiry invalidate the whole read. Tests must cover every byte-field
tamper, wrong secret/nonce, forged frontend key, different peer/namespace/parent/
args/deadline/activation, page replay/reordering, final-token misuse and restart.

Direct token-plan review: fixed-size parsing bounds allocations; the MAC supplies
provenance that the binding digest alone lacks; the SQL-derived parent key is never
asserted by a public caller; key loss aborts only read work; stateless retry behavior
is explicit. Storage and recovery independently reviewed the concrete contract and
passed it, with setup-copy erasure and poisoned-instance disposal incorporated.
Canonical flag/count combinations and the original public cursor remain explicit
implementation checks. The clock-domain envelope and complete revision write
inventory remain qualification gates, not assumptions proved by these fixtures.

### A2c bounded response encoder subplan

The existing KvListResponse::new builds a whole Value tree and retains decoded
entries alongside the final bytes. Keep it unchanged as the compatibility oracle.
Add a separate incremental Rust List encoder for the hosted assembler. It owns
two bounded byte buffers (entry bodies and extended-entry bodies), entry/extended
counts, and a total encoded-length ledger. It does not retain decoded pages or
per-entry Vecs. Normalize each validated dirent and permitted small body using
the existing schema encoders, append their bytes, then drop those temporary values.
This retains native field normalization and keeps JS out of FOKS encoding.

Before each reservation/append, check the combined List length with checked
arithmetic, including the outer array marker, actual minimal entry-list prefix,
boolean, extended-list prefix, every extended position/container prefix and both
bodies. Empty lists encode as Null, exactly as the oracle. Length accounting must
handle array-prefix changes at15→16 and integer-position changes at127→128 and
255→256. Reject totals above the existing16MiB public maximum before adding the
next entry. The final RPC framer still enforces its precise envelope overhead;
do not invent a smaller arbitrary payload cap that rejects an otherwise valid
native response. A16MiB logical candidate may fail final framing just as native
does, but cannot cause unbounded candidate allocation.

Reserve geometric buffer growth only up to the remaining explicit capacity cap,
charging old+new coexistence through the request's existing allocation admission.
At finish allocate the exact final List length once, append minimal prefixes and
the two bodies, then release intermediate storage. The assembler/response owner
retains its admitted working/output permits through physical completion and
socket transmission; the encoder alone is not a substitute for that lifetime.
No caller can request more than4096 entries or supply an extended position: the
encoder derives positions from accepted entry count.

Append admission is atomic with respect to builder state: encode/validate both
candidate pieces and reserve both destination capacities before modifying either
body or count. On any error, either leave the accepted prefix unchanged or poison
the builder so it cannot finish; never emit an entry with a mismatched side table.

Review this encoder separately before tests. Differential tests compare exact
bytes with KvListResponse::new across all prefix boundaries, empty/full pages,
omitted optional bodies, normalized schema variants and maximum valid small
nodes. Inject allocation refusal before growth/finalization and assert no partial
published response. Test just-below/at/above the calculated payload limit, then
the actual RPC envelope boundary. This is an internal encoder/API addition, with
no public FOKS format or historical verification change. Direct review complete;
independent implementation-subplan review remains pending.

Bounded response encoder plan independent review passed (storage): aggregate capacity admission must cover both buffers, old/new coexistence and final output; normalization uses existing schema encoders and append is atomic or poisons the builder. Existing oracle and exact RPC framing remain authoritative.

A2c0 directory component author and root independent reviews passed on the eight-path isolated source snapshot. It retains one SQL transaction, probes guarded current-head dimensions and copies bounded slices into one exact buffer before the unchanged full decoder. Decoded ID/version mismatch parity is deliberate. Tests through the default16MiB limit and the fixed64MiB Wasm cap are still required; SQLite internal BLOB materialization/CPU is a separate limitation.

A2c response encoder author/direct and storage independent reviews passed in metadata-list-encoder-validation. It normalizes via existing encoders, derives extended positions, accounts minimal prefixes and both capacity buffers, and admits final output while intermediates remain live. Failure leaves accepted entries intact. Native exact-byte, allocation-failure and real RPC-envelope gates are still pending.

A2c1 trap cleanup amendment: once a synchronous export/import trap unwinds, poison the instance, wipe its current capped linear memory and explicit setup/key buffers, then drop the instance reference in a finally path. Never re-enter trapped Rust for cleanup. This makes no erasure guarantee for runtime/allocator copies outside the explicit buffers. Review and deterministic trap/setup-failure tests plus separate wipe-cost measurements are required.

## A2c production dependency revision — concrete plan for review

Prepared 2026-10-16 from the read-only inventory at
`/private/tmp/foks-list-revision-inventory.md`. This is a separate implementation
slice after the current qualification components, not permission to enable a
persistent route. Proposed native schema version is59, following current58;
if another reviewed migration consumes59 first, rebase this migration and its
fixtures/catalog hashes explicitly. No public FOKS message, signature, key
encoding, HostID/UID or Go wire format changes. External zero-loss ownership,
replication and host/AZ recovery remain separate decisions.

### Two revisions with different responsibilities

Retain `metadata_authority.revision` as the authority intent/activation revision.
Its Execute/SetPrincipal/Promote/Activate behavior, receipt replay, rejected-domain
semantics and existing `bump()` contract remain unchanged. That table is currently
qualification-only, and native Database writers do not touch it. Do not attach
new dependency triggers to it: `authority/mod.rs::bump` returns cached
`s.revision+1`, which would be wrong if row triggers had already advanced the
same counter inside a domain command.

Add one schema-owned dependency coordinate per tenant database. Here a tenant
means one standalone FOKS server database, including all its personal namespaces
and users; this is deliberately not one row per UID. Global Merkle/SSO/key
lifecycle dependencies make the single row a conservative, complete first
boundary. It costs one indexed singleton update per triggering row change and
can invalidate a personal bundle after an unrelated user's write. Measure that
abort cost before deciding whether finer revisions are worth the added proof.

Use a STRICT table named `metadata_read_dependency`:

```sql
CREATE TABLE metadata_read_dependency (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    incarnation BLOB NOT NULL
        CHECK(typeof(incarnation)='blob' AND length(incarnation)=32),
    revision INTEGER NOT NULL
        CHECK(typeof(revision)='integer' AND revision BETWEEN 1 AND 9223372036854775807)
) STRICT;
```

An incarnation is a fresh32-byte OS-CSPRNG value, rejected if all zero or equal
to its predecessor. It is private state, not a replacement public installation
identity. Native creation/migration uses an audited OS RNG adapter and propagates
failure before commit; celld initialization receives host-generated CSPRNG bytes
through a privileged initialization path, never request-selected bytes or SQLite
`randomblob`. Deterministic test values are confined to qualification constructors.
“Nonreused” means no deliberate reuse plus cryptographic collision resistance;
it is not a claim that raw copies of all private state are detectable.

Initial revision is1. It is an equality token, not a count of operations. Multiple
row/derived-trigger updates inside one transaction may advance it more than once.
Persisting a revision must never wrap, become REAL, or silently stop advancing.
There is no automatic live reset at exhaustion. Offline incarnation rotation can
start revision1 again only through the controlled lifecycle described below.

Expose `ReadDependency { incarnation: [u8;32], revision: u64 }` via a bounded
read-only Database/ReadSnapshot method and a fixed shared catalog point. Reject
missing row, wrong width/type, zero incarnation, nonpositive/overflow value, or
extra row as corruption; no default0 and no repair on read. Add
`ReadDependencyChanged` as a private bundle failure distinct from permission,
unsupported profile, missing row and infrastructure failure. Never turn this
into a public cursor or a caller-selected authority coordinate.

### Exact trigger coverage and explicit broader-KV choice

Install three AFTER INSERT/UPDATE/DELETE triggers for **each** of the following
persisted tables. UPDATE covers the whole row, including exact encoded blobs and
primary-key changes; do not use only numeric key columns or only projection
columns. All triggers use the same generated/pinned SQL body and unique names.
The first implementation deliberately does not optimize equal-value UPDATEs.

* Certificate and current credential: `issued_certificates`, `devices`.
* Role/generation and supported profile: `shared_keys`, `sso_policy`.
* Anchored personal identity and lifecycle: `users`, `user_chain_heads`,
  `tree_locations`, `merkle_root_heads`, `merkle_roots`.
* Namespace and all public KV data: `kv_namespaces`, `kv_roots`,
  `kv_directories`, `kv_directory_heads`, `kv_nodes`, `kv_dirents`,
  `kv_dirent_heads`, `kv_dirent_current`, `kv_file_uploads`, `kv_file_chunks`.
* Physical descriptor/placement and activated native key metadata:
  `chunk_objects`, `chunk_storage_binding`, `host_metadata`,
  `host_key_generations`, `capability_key_generations`.

This chooses **root, public Usage, large-file metadata/chunks and descriptor
changes now**, conservatively along with List. Root replacement, usage sentinel
revalidation, chunk upload/reclamation, inline/external conversion and storage
adoption therefore invalidate bundles even though personal small-KV List may
not use their changed fields. This closes a predictable later read-data gap;
it does not qualify or enable large-file/Get/Usage bundle routes or their object
I/O/pin contracts. Current optional List bodies remain only inline small files.

The List closure is complete through these tables: exact certificate row plus
same active canonical device/subkey/role/visibility; current PUK generation;
absence of any SSO policy; anchor joins through current user/Merkle heads;
selected parent head and full directory bytes; live membership/order tuple and
historical exact dirent; optional node presence/bytes/key. See
`personal/catalog.rs`, `personal/mod.rs::{authenticate_peer,
require_personal_profile,require_key}`, `page_catalog.rs`, and the unchanged native
`services/kv.rs::list` for the actual predicates. Pure root changes are included
by choice rather than misrepresented as a current explicit-parent List lookup.

Explicit exclusions: `kv_locks`, `kv_upload_maintenance`,
`chunk_storage_revision`, `chunk_object_totals`, `chunk_object_attempts`,
`host_rotation_operations`, authority intents/principal fixture/clock bookkeeping,
SSO browser sessions/access/refresh rows, team/federation grants, challenges,
reservations, receipt expiry, logs and metrics. The included tables already catch
the data/descriptor transitions caused by these operations. Excluded counters,
inventory cursors and unresolved-attempt timestamps do not themselves change
selected List bytes/access, root/Usage totals or stored file content. A later
route that relies on an excluded status must expand this catalog before activation.
Team/SSO profiles stay explicitly unsupported. Account-name/nag and staging-key
updates may cause conservative invalidation through included broad tables; that
is acceptable and measurable, not a reason to silently weaken the closure.

Every trigger body first executes a SELECT CASE guard that raises ABORT with a
fixed private invariant error if singleton1 is absent, has invalid incarnation/
revision, or revision is already i64::MAX. It then updates exactly singleton1
with revision+1. The table's PK/CHECK restricts it to at most one row; the presence
guard prevents an apparently successful zero-row UPDATE. The typed table's CHECK
is an additional overflow defense, not the only one. The source mutation and
revision increment share the same SQL statement/transaction. If any cascade or
later trigger fails, all source changes and increments roll back together.

Add protective triggers on the dependency table rejecting DELETE and singleton
key changes, and rejecting a decreasing revision while incarnation is unchanged.
Incarnation rotation is a separate trusted lifecycle operation with expected-old
coordinate CAS; no normal request/catalog method can perform it. The SQL table
is not a security boundary against arbitrary local SQL: direct counter/nonce
edits, removed triggers, writable_schema and live file replacement are unsupported
external mutation, explicitly denied by the serving deployment contract.

Cover both source and derived rows so direct exact_dirent edits, direct projection
edits/rebuilds, tombstone changes, head relocations and FK cascades cannot bypass
revision. This intentionally produces multiple bumps for a Put. Pin
foreign_keys=ON; qualify recursive trigger behavior used by both engines. A
REPLACE still bumps through its INSERT even when SQLite suppresses the implicit
DELETE trigger; tests must also prove derived projection/counter consistency.
Do not rely on trigger execution order or connection-local SQL UDFs.

### Migration, initialization and hook verification

Add a server-db schema module defining the table, complete trigger manifest and
validation. Fresh initialization builds existing schema/backfills first, then
creates one coordinate and all dependency hooks in the same exclusive migration
transaction before publishing user_version59. Predecessor58 migration leaves all
FOKS records unchanged, inserts the fresh coordinate, installs hooks, validates
FK/projection/current schema contracts, and commits the version atomically.
Older supported predecessors first run their existing migrations and finish with
the same59 step. No reader/writer may serve between backfill and hook installation.
Any RNG/DDL/validation error rolls back table, triggers, version and prior migration
work; retry does not adopt partial state. Genuine predecessor fixtures must remove
all new objects, not just lower user_version.

A schema-owned immutable Rust manifest lists each expected table/event/trigger
name and exact qualified SQL definition. Native and celld use the same artifact;
include it and the new table definition in schema/catalog/activation fingerprints.
Validate the actual `sqlite_schema` objects, not only their names, a supplied hash,
or user_version. Canonical comparison must preserve SQL string-literal semantics;
no blanket whitespace removal. Pin and test the actual SQLite/celld representation
of stored definitions. Missing/modified/duplicate unexpected dependency hooks,
missing coordinate or wrong schema fails activation and serving admission.

Full definition hashing happens at exclusive activation and new connection setup,
not by scanning all SQL objects on every page. Cache only after exact verification.
Bind the verified SQLite schema cookie to that connection/activation; each bundle
transaction checks it using a fixed catalog query before dependency access. A
changed cookie causes refusal/revalidation under exclusive activation, never
continued serving from the cached hook proof. Qualify that query on actual celld;
if unavailable, the host must supply an equivalent qualified schema-change gate
before production use, not omit it. Deliberately rewriting the cookie to conceal
DDL is unsupported arbitrary SQL, as is deleting triggers after validation.

Native writer exclusion remains required. `Writer::start` and
`OfflineWriterGuard` must hold their existing host-local exclusion through migration
and activation. Audit every direct `Database::open/open_existing` operator path;
unsupported old raw writers must be stopped before upgrade. A schema59 tag makes
old newly opened binaries reject, but does not stop an already-open58 process.
Triggers cover supported DML even without wrapper awareness, yet old-process
coexistence is still prohibited because its invariant/migration assumptions were
not qualified. Freeze/verify hooks on the actual physical DB each reader uses.

### Bundle binding and transaction behavior

Extend the private canonical List binding with the complete `ReadDependency`
coordinate and version the binding domain/fixtures. Keep the existing authority
Fence and intent revision separately. The fixed token's binding digest covers the
new coordinate; the opaque token need not grow if it still stores only that digest.
Old bindings fail nonce/domain/catalog checks, never silently acquire the new
coordinate. Bind the opaque frontend bundle to the coordinate obtained by Begin;
never accept a public request's asserted incarnation/revision.

Begin checks activation/schema verification, resolves current peer/profile, reads
this coordinate and the parent in **one SQL transaction** before issuing its token.
Next and Finish each check the expected coordinate plus current peer/key/fence and
schema verification within their own SQL transaction. No reader snapshot crosses
network/object I/O. Finish's successful proof is the read linearization point;
then perform the required durability/output/deadline gates before publication.
A dependency mismatch discards the entire accumulated candidate. A new attempt
uses an empty candidate and original deadline; no mixed-prefix restart.

For authority Execute, dependency increments happen only inside the nested domain
transaction and persist only if that domain commits. A rejected domain rolls them
back while the outer authority intent outcome may still advance its separate
revision. A late infrastructure/outer commit failure rolls both domains back.
Known-outcome replay does not reexecute dependency writes. Exact ordinary native
replay can still conservatively bump if it performs a real equal-value UPSERT
(e.g. namespace ensure) or validation-sentinel repair; no replay path may reset
or reuse a coordinate. Bookkeeping-only reads/clock-floor updates, Register,
Settle and Prune do not bump dependency revision and cannot self-invalidate a
multi-page read. Wrapper intent revision remains allowed to cause conservative
read invalidation under its existing independent contract.

### Incarnation lifecycle and limits of local fencing

Fresh databases and migration each generate an incarnation once inside their
publication transaction. Ordinary restart preserves it and the revision; the
read-token authority still generates a new ephemeral key/nonce, so in-flight
bundles from the previous process cannot resume. No deterministic fallback, no
loading an old token secret from backup, and no persistence of issued read tokens.

The official restore path (`standalone.rs::read_validated_backup_artifacts` and
`restore_backup`) must rotate incarnation on the private validated copy before it
is published, using a typed helper that CASes the observed old coordinate and
sets fresh nonce/revision1 atomically. Use the same restricted private-copy pattern
as clearing chunk storage binding; no online RPC/normal mutation method exposes
this helper. Preserve HostID/UID and signed records. Sync/checkpoint the copy and
publish via the existing fresh-destination protocol; on failure leave the source
backup untouched and no admitted new activation. An already59 backup gets rotation
as well as migrated58; the restored old numeric revision is never sufficient proof.

Backup creation may preserve the source coordinate in the snapshot, but snapshot
hydration/validation never publishes an online authority capability. Every
subsequent official restore rotates it. Export/import, snapshot delta installation
and any future migration path that replaces a live database must follow the same
quiesce→private-copy→rotate→validate→activate sequence, or remain unsupported.
Do not reuse the older proof_meta.revision as this coordinate: its full-snapshot
proof is a different authority model.

Promotion/activation always invalidates existing bundles through owner/epoch or
activation comparison and token-authority reinitialization. A plain same-state
owner promotion need not reset dependency revision; a restored/replaced database
must rotate incarnation even if public identity/owner fields otherwise match.
Host-key generation, storage-binding and config/limit activation must be tied to
current native state and actual key-file digests before serving; dependency bumps
alone do not authorize a stale activation. Replacing activation also replaces the
secret token authority. Certificate expiry and clock/deadline validity still need
fresh checks even when every SQL coordinate is unchanged.

These measures do not detect malicious whole-DB+private-authority co-rollback,
unsupported raw cloning, direct SQL edits, or simultaneous owners bypassing the
local lock. They do not choose a replication/fencing provider or establish
zero-loss failover. A persistent host/AZ authority must provide those guarantees
separately; a local random incarnation and WAL FULL sync are not substitutes.

### Qualification and implementation order

1. Schema module, typed coordinate reads and lifecycle helper; author review then
   root/independent review before tests. Test fresh creation and genuine43..58
   migrations, failure rollback, missing-row and malformed-row rejection, exact
   hook manifest, modified/dropped hook, schema-cookie change, old opener refusal,
   concurrent/offline upgrade exclusion and RNG failure. Verify unchanged FOKS
   record bytes and signed historical verifier behavior.
2. Real native writer matrix under barriers, reading coordinates before/after:
   identity creation, certificate issuance, credential addition/revocation, PUK
   rotation, user/global Merkle publication via generic/team/host rotation, SSO
   activation/transition, mkdir, node insertion, dirent update/tombstone, root,
   chunk append/finalization, public-Usage revalidation, reclaim, externalize/
   hydrate, storage adoption and object-descriptor transitions. Where no typed
   writer exists (demotion/subkey replacement/raw repair), explicitly exercise
   schema DML as a trigger test, not invent production API coverage.
3. Table-event coverage generated from the manifest: INSERT/UPDATE/DELETE,
   same-value writes, exact-blob-only updates, key relocation, REPLACE with pinned
   recursive_trigger settings, source/derived ordering, namespace/user cascades,
   optional body missing→present and permission-generation changes. Check rollback
   after a later trigger/domain failure restores both bytes and coordinate; test
   i64::MAX abort before effects and overflow midway through a cascading batch.
   Fault injection may temporarily drop the protective DELETE trigger to create
   missing singleton corruption; serving must refuse, not repair it.
4. Authority/native/Wasm integration tests prove the two revisions independently:
   successful domain, rejected domain, infrastructure rollback, known receipt replay,
   clock/read/Register/Settle/Prune, mismatched incarnation, forged/stale binding,
   exact retry and parent/body/revocation ABA between pages. Barrier tests require
   no partial output and no accepted old token after Finish failure. Final proof
   remains valid if a mutation happens strictly after its linearization point.
5. Actual celld runs the same schema/trigger artifact and mutation matrix, including
   nested transaction rollback and cascade overflow, with outer transaction+sync
   before visible replies. Fixture seed inserts occur before hooks only in an
   explicit initialization transaction that establishes revision1; post-activation
   test mutations must use hooks, not manually bump a counter. Keep native and
   Wasm result/coordinate parity, while checking monotone change rather than
   asserting one increment per public operation.
6. Restore/copy tests start an old bundle, produce/restore a backup, activate a new
   token authority, and reject the old bundle even with equal public identity and
   overlapping numeric revision. Check source artifacts unchanged, new private
   incarnation, crash-before-publication recovery, preserved healthy current DB,
   and refusal to auto-adopt raw copied/live-overwritten state.
7. Measure amplification before production activation: actual trigger invocations,
   SQLite VM steps, rows changed, WAL bytes, transaction latency, bundle abort rate
   and successful public List latency for64/4096 entries, with1/64-dirent Put batches,
   many retained historical versions, concurrent unrelated-user writes, upload
   append/reclaim and idle inventory. Compare the exact existing-schema baseline
   with revision hooks at fixed cardinalities and declared hardware. Track repeated
   kv_namespaces accounting/projection bumps rather than hide them in RPC counts.
   The per-row algorithm is bounded independently of retained history, but large
   legitimate cascades remain proportional to changed rows. No scale/15s success
   claim until these measurements and full persistent route/ownership gates pass.

Direct self-review: separate revisions preserve current authority receipt math;
fail-closed row hooks cover actual selected payloads and indirect/cascade writers;
broader root/Usage/file mutations are explicitly included; excluded bookkeeping
cannot cause read-clock self-invalidation. Restore rotates private identity while
preserving public FOKS identity. Remaining production gates are explicit activation,
old-writer exclusion, actual-engine behavior, allocation/deadline ownership and
external HA authority, not unmentioned mutation paths. No implementation or tests
are authorized by this draft until parent and independent plan review pass.


### A2b qualification record (2026-10-16)

A2b was committed as19ac886f. The exact16-file snapshot based on a01bf92c
passed all reviewed gates. Evidence is
`/private/tmp/foks-a2b-final-qualification.json`; source hashes are in
`/private/tmp/foks-a2b-validation-sha256.json`. All main/isolated source bytes and
all eight actual-celld input/artifact hash manifests were verified after the runs.
The26 page vectors compare exact binary replies and complete SQL state, including
0/1/63/64-row probes, signed64-bit boundaries, leading UTF-8 BOM, nullable values,
maximum bodies, missing optional nodes and suppressed oversized legacy payloads.

The Wasm artifact declares18 initial/1024 maximum pages (64MiB maximum). Page
vectors observed at most1,703,936 bytes of linear memory and1,266,750 declared JS
page working bytes. Those measurements exclude SQLite/V8 RSS and fixture snapshot
heap; they are not tenant-density or production memory bounds. The22MiB JSON
fixture bundle ran with the existing64MiB JS heap limit unchanged. Native and
actual-Wasm failures verify admission before SQL, poison/discard after exported
traps, ordinary-error reuse and exact bounded envelope behavior.

For a two-row requested page, initial/MAC/time start queries used153/158/160
SQLite VM instructions and their continuations195/187/190; unchanged native
queries used49/52/53. Counts remained identical with0,100 and10,000 additional
retained versions, tombstones and unrelated current heads. The query-cost test
records indexed plans without temporary ordering. This demonstrates the tested
query work, not constant wall latency or freedom from B-tree/cache costs.

The first native compile failure (temporary descriptor lifetime) is retained in
`/private/tmp/foks-a2b-native-a.log`; the reviewed correction uses identical named
constant slices. The initial cell launch was denied loopback binding by the
sandbox before any server started; the authorized local launch and all subsequent
runtime suites passed. No assertion, heap limit or protocol compatibility rule
was relaxed. This remains private semantic qualification: persistent service,
complete current authorization/revision coverage, bounded synchronous SQL
execution and combined host/AZ recovery are separate gates.

Dependency-plan parent review clarification: activation and new-connection
verification cover the complete relevant existing schema contract as well as the
new revision hooks. In particular verify live-head projection, public-Usage and
quota accounting, FK/cascade definitions and associated triggers; a correct
revision counter cannot certify a damaged projection or missing subtraction
trigger. Cache that full verified contract once per activation/connection and
check the qualified schema cookie inside every bundle transaction. This does not
mean scanning schema objects for every page. Root direct plan review passed with
this clarification; independent review remains pending.


A2c1 token source is author-reviewed and frozen as12 files in the isolated
metadata-read-token-validation tree. It adds the mandatory validated dependency
incarnation/revision to canonical binding domain-v2 without changing the180-byte
token layout. The coordinate is a validated value, not evidence it was obtained
from SQL; reviewed Begin/Next/Finish must obtain/compare it in their own authority
transaction. Secret setup/clear exports are host-only; issuance/advancement is
crate-private and the public fixture façade/export requires the explicit
read-token-qualification feature. Default artifacts cannot select that issuer.
Native/Wasm golden, tamper, tuple/retry, expiry, key loss and trap-wipe tests are
authored. Root and storage independent source reviews passed, including the
mandatory dependency binding and entropy-failure regression. Isolated execution
gates passed:46 native tests (six new token tests), strict native/default-Wasm/
fixture-Wasm lint, and nine actual-Wasm host tests. Both immutable modules declare
18 initial/1024 maximum memory pages; the default module has no issuer export.
Evidence: `/private/tmp/foks-a2c-token-qualification.json`. Twelve frozen source
hashes remained unchanged through qualification. Integration with directory and
response-encoder components still requires its own merged-source review/checks.

### A2c0 actual-celld directory qualification extension — proposed execution plan

Prepared 2026-10-16 after the native45-test and actual-Wasm eight-case directory
qualification passed. This extension qualifies DirectoryProbe/DirectorySlice on
actual celld storage; it enables no ordinary route, authentication, token or List
bundle. The existing native eight binary cases and expected replies/errors/call
counts are the oracle. Keep both the declared64MiB Wasm maximum and existing64MiB
celld JS heap configuration unchanged. SQLite/JS total memory, SQLite substr work
and a production15-second success guarantee remain outside this qualification.

Use a new dedicated `directory-worker.mjs` fixture entrypoint and an explicit
`run-local.py --directory-fixtures <directory>` mode. Reuse the existing launcher,
local Moto/celld deployment, private capability, pinned Wasm/parser/page-catalog,
node heap settings and child cleanup. Ordinary metadata/page/authority modes and
their body limits remain unchanged. The directory mode requires the eight-case
`directory-vectors.json`, its exact page-catalog and all eight `.bin` files; it
refuses mixed modes, unexpected filenames, symlinks, missing files, lengths above
16MiB, or unsupported case counts. Before deployment the controller copies those
inputs into its private output directory, verifies copies byte-for-byte/hash, and
constructs a small immutable manifest with each file's exact length and SHA-256.
It also pins the original native expectations and small common schema snapshot.
Record original and copied input hashes plus all host/worker/controller artifacts
in the qualification evidence. No binary file becomes a JSON integer array or
base64/hex string, and no deployment module imports all eight binary bodies.

For each case, the controller sends that one immutable binary file as the body of
a private authenticated POST to its fresh case cell. The indexed manifest, never
request metadata, determines the allowed case, expected body length/digest and
command. Authenticate and validate the index before reading a body. The worker
reads at most the manifest length into one owned Uint8Array, rejects short/extra
bytes, cancels the reader on failure, and verifies SHA-256 before seeding or SQL.
No caller can choose a directory/key/command/expected result through this route.
The HTTP request and body copies are fixture transport, measured/described
separately from kernel execution; this is not the production large-record path.
If actual celld rejects a16MiB fixture body or its unchanged heap cannot support
this bounded transport, stop and review a chunked fixture-loading alternative;
do not raise a limit or silently omit maximum cases.

Seed the same small native snapshot/schema in an initialization transaction,
then replace only the selected exact_directory using the raw binary bind. Check
foreign keys and synchronize before starting the execution measurement. One
fresh cell accepts only one attempted case; any transport/seed/execute failure
makes that case fail, not a reusable partially seeded cell. Construct the kernel
with the exact pinned page artifact. Execute `runDirectory` inside one
`storage.transactionSync` so the current head/probe/slices share a snapshot, with
the same15-second original execution budget used by the Node qualification.
This is a fixture abort bound, not a hard production runtime guarantee.

Compare exact key reply bytes, typed failure class, SQL call count, unpoisoned
normal-error state and zero remaining declared page reservation against the
native vector. Record initial/final Wasm linear-memory sizes (memory only grows,
so final is its observed peak), declared page working peak and elapsed execution.
Require linear memory <=64MiB and the immutable module's declared maximum <=64MiB;
retain the distinction between declared JS page admission and measured process
memory. The valid and malformed16MiB cases are mandatory independent fresh cells.

Preserve complete SQL-state assertions without materializing the large directory
as JSON. Reuse the bounded complete snapshot comparison for every table/column
except the selected exact_directory cell. At that one exact row, check SQLite
`typeof`, exact byte length, unchanged key/head metadata and a fixed sentinel in
the normalized snapshot. Independently compare the selected BLOB byte-for-byte
against the retained immutable request bytes using512KiB SQL substr reads; check
that all expected bytes were compared and an extra slice is empty. Thus the
sentinel is backed by a complete binary equality proof, not a blind exclusion or
caller-supplied digest. Run this assertion both after seed and after execution;
all other snapshot cells, rows, tables and schema objects must match the native
fixture. Also assert unchanged `total_changes()` across kernel execution and
sync before returning the private result. Assertion SQL and source fixture
ownership are outside the kernel call/working-memory metrics and explicitly
labeled. No after-state bytes are returned as giant JSON arrays.

Implementation ownership: storage agent owns the new directory worker and narrow
launcher mode, plus focused fixture-controller tests; recovery reviews the host
and launcher changes, root reviews the full delta. Do not modify the frozen
A2c0 Rust/source codec or existing Worker route behavior merely to fit fixtures.
Two source reviews precede testing. Add deterministic transport tests for missing,
short, oversized, wrong-hash and wrong-capability bodies; fail before SQL, with
fixture cell reuse forbidden. Add assertion tests proving a changed middle/last
BLOB byte, head metadata, another row or another table fails parity. Then run all
eight immutable native-oracle cases on actual celld with unchanged limits, save
all failures as evidence, and rerun the existing launcher-mode argument tests.
Success requires eight passing cases, exact input/artifact hash verification
before and after the run, and no skipped/memory-relaxed case.

Author review: the binary POST is a private immutable fixture transport, not a
new upload API; one bounded case avoids giant JSON/deployment assets. Snapshot
normalization remains sound only with complete binary equality for its sole
excluded payload. The cell must reject input before any SQL and must not mask
transport/host failures as expected malformed-directory results. Existing
modes remain unchanged. Await root plan review before implementation or tests.


A2c1 explicit trap cleanup samples measured2/16/64MiB current linear-memory wipes
at approximately0.014/0.538/2.977ms on the local Node host. These are single
observations of the bounded fill operation, excluding verification scans,
allocation, runtime copies and cryptographic-library stack copies; they do not
qualify a hard request deadline. The deterministic tests require the instance
reference to be dropped after the outermost trapped export unwinds, every later
entry to refuse, and current memory bytes to be zero. Setup allocation failure,
normal returned errors, entropy failure and default-artifact issuer exclusion
remain separate tested paths. No actual-celld token-service or SQL Begin/Next/
Finish route was introduced or qualified by these component tests.

### A2c0 actual-engine stop gate and proposed commit qualification amendment

The first immutable actual-celld run (`/private/tmp/foks-directory-celld-a/qualification.json`)
failed all eight cases before kernel execution. Preserve its original inputs,
responses and logs. Six representable cases reached a fixture-only forbidden
`PRAGMA user_version=58`; both16MiB cases failed binding the large exact directory.
The pinned celld0.6.2 source explicitly sets `SQLITE_LIMIT_LENGTH=2_200_000` in
`crates/celld/storage.rs` (around848), covering SQL values and encoded rows rather
than just transport buffers. Its `allowed_sql_pragma` function (around519) excludes
user_version writes. This is direct pinned-source evidence, not an assumption
that splitting one bind or raising a runtime heap limit solves storage.

Proposed narrow correction: remove only fixture user_version assignment, retaining
the native schema label in the immutable manifest as previous qualification modes
do. Add a negative fixture test forbidding that SQL and prove ordinary seed/state
assertions still work. Add stage-specific error text around the directory payload
bind and pre-execution state check to distinguish setup failure from a kernel
status. Keep every native expected outcome and all eight cases unchanged; do not
turn storage failures into expected malformed-directory results. After two source
reviews, rerun Node/Python fixture gates then all eight actual-celld cases. The
expected diagnosis is six qualified representable cases plus two retained
unsupported large-record failures; the original eight-case qualification must
still report failure, never “eight passed.” No cap is raised and no case is skipped.

Commit qualification amendment for root review: the independently reviewed
one-record decoder, response encoder and private token primitives may be committed
with their native/default-Wasm/fixture-Wasm gates and accurate actual-engine
limitations. This does not enable an ordinary/persistent route or claim default
native16MiB directory compatibility on celld. Production activation remains
blocked by the storage cap even though the decoder itself fits64MiB Wasm.

The next design decision is physical record segmentation (wire-identical exact
bytes stored across bounded rows with atomic ordered reconstruction and the same
native decode/error semantics), or an explicitly restricted hosted profile that
refuses these records at bootstrap/admission. Concatenating chunks into one SQL
BLOB does not bypass the engine's encoded-row limit. A segmented representation
needs its own reviewed schema/migration, completeness/digest/length invariants,
transactional revision coverage and read/write/backup compatibility tests before
implementation; this qualification extension does not introduce it. Uncontrolled
local SQL-limit overrides are not an approved deployment choice.

Author review of this amendment: the test-only pragma correction removes no
payload, reply, hash, state or memory assertion. The storage failure remains a
material compatibility finding. Root and independent review are required before
rerunning the changed fixture code or changing the component qualification claim.

Root direct review approved the actual-engine amendment and the narrow fixture
correction. The combined19-file component snapshot passed58 native test functions
and10 Node test functions (eight directory vectors plus nine token tests), strict
native/all-target and both default/qualification Wasm Clippy, release Wasm builds
with64MiB maximum, and workspace formatting. Earlier progress counts of59 native
and17 Wasm tests mixed expected counts/vector cases; these parsed final counts are
authoritative. Evidence: /private/tmp/foks-a2c-primitives-qualification.json.
The16MiB directory decoder reached52,494,336 bytes of Wasm linear memory; declared
JS page working allowance peaked2,630,213 bytes. Neither measures SQLite or V8 RSS.

Restore integration author/direct review and storage-agent independent review
passed. Rotation occurs after private-copy migration/storage-binding clearance and
before checkpoint/publication. A new backup test restores one archive twice,
requires distinct nonzero private incarnations at revision1, rejects the archive
coordinate, preserves the new coordinate on ordinary reopen, and checks unchanged
archive bytes. Existing predecessor43..58 fixtures remove59's hooks before older
DDL. All public identity/history checks remain. This four-file slice awaits the
combined schema59 gates; it has not been committed or tested independently.

Independent recovery-agent review approved the actual-engine amendment and
fixture correction. The second actual-celld run retained both16MiB bind failures;
six smaller cases reached a separate fixture schema-comparison mismatch, under
investigation. No actual-celld directory case is yet counted as passed. The
19-path native/Wasm primitive commit is independently qualified under the reviewed
amendment; the six-file celld fixture extension remains separate until its own
assertion corrections are reviewed and tested. Full production compatibility
remains blocked, with all failed qualification evidence retained.

Schema59 implementation scope clarification: connection.rs/error.rs additionally
own the per-handle verified schema proof and typed coordinate/schema failures.
SQL equality preserves string literal spelling and only normalizes identifier
quoting for the explicitly qualified ALTER-renamed chunk tables and SSO columns.
The singleton also rejects INSERT OR REPLACE, independently of SQLite's optional
recursive DELETE trigger behavior. A hook's RAISE(ABORT) rolls back its failing
statement including cascades; repository transaction error propagation/drop rolls
back earlier statements in the same domain operation. It does not automatically
abort an arbitrary surrounding transaction.

The schema component's drafted tests cover actual identity/certificate/directory,
root, small-node, inline/external chunk, full rename, and user-key rotation writers;
identity/user mutation failure-point rollback; exact replay where repository work
is a no-op; raw credential ABA; all 72 generated event bodies; exhausted counters;
private-copy rotation; schema-cookie rejection; existing projection-hook damage;
genuine58 migration and the existing43..57 fixture chains; and measured VM steps
against retained history. Successful typed revocation is not separately asserted
here (its credential DML and a rejected stale-root revoke are covered). Authority
nested transaction/replay bookkeeping, old-writer deployment exclusion, actual
celld trigger depth/opcode limits, and production bundle token integration remain
explicit subsequent integration gates. None of those gates is implied by these
native schema tests.

## Concrete shared List transaction and assembly plan — review required

Prepared2026-10-16 after A2b19ac886f and the individually qualified directory,
token and response-encoder components. This section supersedes the earlier
assumption that unchanged native directory rows are physically representable on
celld. It is an implementation plan, not route activation authorization. The
schema59 dependency component is still being implemented independently.

### Hard prerequisites and qualification boundaries

Pinned celld0.6.2 `crates/celld/storage.rs` sets SQLITE_LIMIT_LENGTH to2,200,000
bytes, a value/encoded-row limit. The16MiB native directory oracle cannot be
stored in the unchanged physical schema. Bounded `substr` reads and concatenating
pieces into one final SQL BLOB cannot repair that incompatibility. The original
eight-case actual-engine qualification remains failed if either maximum case
fails; six smaller successes are not full native compatibility. Native/Wasm
memory feasibility and physical celld storage feasibility are separate results.

The recommended prerequisite for unrestricted compatibility is a separately
reviewed physical directory segmentation adapter: bounded rows plus a versioned
manifest, atomic completeness/length/digest invariants, exact reconstruction,
shared mutation/import/backup/restore behavior and dependency hooks. Merely
segmenting the read fixture would be a false fix. It must preserve both malformed
legacy bytes and successful native directory semantics, public wire bytes and
Go compatibility. That representation needs its own plan before implementation.
An explicitly restricted deployment profile is an alternative product decision,
not a silent per-request cap or a successful full-compatibility gate. This plan
may qualify List semantics on representable cells while recording that blocker;
it must not enable the claimed production default profile until resolved.

Directory segmentation alone is insufficient for the full backend. Before claiming
full route coverage, inventory every physical table/encoded row, including
multiple BLOB/TEXT fields in one row, native maximum_blob_bytes and
maximum_idempotency_response_bytes (both16MiB), generic links/identity/passphrase/
team/host rotation records, roots and retained outcome payloads. A sum of fields
can exceed the engine row limit even when each field fits. Freeze a route-by-route
physical representation/import/write/backup compatibility matrix; a qualified
small personal List subset does not settle those other storage contracts.

Other required gates are actual schema59 hook/coordinate validation, an
actual-engine schema-cookie query, complete native fixture status parity,
production frontend/owner capability binding, bootstrap to actual installation/
key/storage/artifact state, and end-to-end deadline/output ownership. The current
authority fixture's controller-supplied activation labels do not prove these.
Pinned celld explicitly allows read-only `PRAGMA schema_version`, but a proposed
`pragma_schema_version()` table-valued query still needs an actual-authorizer
probe. It does not allow the fixture's former user_version write. No hidden
privileged SQL or provider limit changes are planned.

### Owned context, exact requests and sealed proof results

Add `read_bundle` to the shared metadata crate. Keep it separate from the
synthetic authority caller and from JSON fixture dispatch. Proposed internal
Rust types, with private fields and bounded constructors, are:

* `ListArguments { parent:[u8;16], cursor:InitialCursor, count:u16,
  load_small:bool }`, constructed by the unchanged native public argument parser;
  retain the original zero/over4096 clamp and exact cursor validation. The public
  Time cursor initially retains its full unsigned64-bit value. Native parsing
  accepts values above i64::MAX; database conversion later returns IntegerRange,
  mapped to TransactionRetry. Begin must preserve this error and its ordering:
  resolve/read/decode/authorize the parent first, then reject an unrepresentable
  time before constructing the signed Number in ListBinding. Do not introduce
  BadArguments or truncate/wrap. Test i64::MAX, i64::MAX+1 and u64::MAX against the
  native oracle, including missing and unreadable-parent precedence.
* `ReadContext { peer:PeerCertificate, fence:Fence, intent_revision:Number,
  original_deadline:Number, clock_domain:[u8;32] }`. The host supplies fresh
  trusted `now` separately on every call. Repeated calls cannot replace the
  original deadline or peer. The actual mTLS certificate parser remains outside
  the kernel; serialized peer parts alone do not prove possession.
* `VerifiedSchema`, a host-owned non-serde guard obtained only by checking exact
  schema/trigger artifacts on the actual connection/activation. It retains the
  checked schema cookie and qualified physical representation. It is affine to
  that actual SQL connection or sealed storage activation, not a transferable
  cookie/hash credential. Native proof construction borrows the same connection
  used by its transactions; adapter APIs must make using a different connection
  impossible. Wasm setup seals the guard to the particular instance, storage
  adapter and activation; replacing any of them invalidates it. Equal cookies in
  two databases do not establish equivalence. A public request cannot create it.
  Native and celld constructors have separate qualification fixtures, not a
  request-selectable bypass.
* `ReadBundle`, a frontend-owned non-serde handle with context, arguments, the
  kernel-returned full binding/derived namespace/dependency, expected bundle ID,
  next ordinal, token, accumulated count and terminal state. Only a successful
  Begin receipt constructs it; no frontend field supplies the parent key.
* `PageBatch`, one owned bounded binary response with validated borrowed entry
  views. `FinishReceipt` and `PublishableList` have private constructors. These
  do not implement serialization as public protocol objects.

Proposed kernel calls are `begin_list(sql, schema, authority, context, arguments,
host_bundle_id, now) -> BeginReceipt`, `next_list(sql, schema, authority, binding,
token, now) -> PageBatch`, and `finish_list(sql, schema, authority, binding, token,
now) -> FinishReceipt`. `authority` is the existing host-owned ephemeral
ReadTokenAuthority. The host generates a fresh16-byte random bundle ID, rejects
zero and supplies it only through its internal call boundary. No public selector
can choose the token key, nonce, parent role, namespace or dependency coordinate.
The explicit fixture feature may supply deterministic values solely for tests.

The binding is the reviewed domain-v2 ListBinding: complete Fence and intent
revision, full dependency incarnation/revision, exact original peer, SQL-derived
namespace, selected parent, normalized arguments and original deadline/domain.
The180-byte token remains unchanged. Different certificate DER, even a valid
renewal for the same device, requires a fresh whole read; an old read's exact peer
context cannot change in flight. The stable identity rule for write-outcome
resolution does not silently apply to this read continuation.

### SQL proof and transaction ordering

Refactor the sealed A1 PersonalResolver only enough to expose a crate-private
`prove_current_personal_read` returning the currently queried CurrentCredential.
It reuses the exact owner/activation/revision checks and current certificate,
ordinary-device/profile/anchoring logic; it does not read synthetic principals.
Do not clone the algorithm into a second authorization implementation. The
existing `run_resolved(Request::Prove)` updates the authority clock floor; these
bookkeeping writes remain excluded from the separate dependency coordinate.
Expected failure must preserve current personal/infrastructure classifications.

Each Begin/Next/Finish executes in a fresh host-owned SQL transaction. Inside it:

1. Check trusted time and the original budget, the verified schema cookie, exact
   activated fence and intent revision. A cookie change refuses serving and
   requires exclusive schema verification; no implicit repair or hash refresh.
2. Resolve current peer/profile using the shared resolver. Derive namespace from
   CurrentCredential.uid(); compare any carried binding namespace to that value.
3. Query schema59's singleton dependency using a pinned point statement returning
   exact BLOB32 incarnation and canonical decimal i64 revision. Require exactly
   one valid nonzero coordinate; never substitute0 or bootstrap on read. Compare
   both fields for Next/Finish, even if the numeric revision matches.
4. Perform the operation below while retaining this transaction. Return no bytes
   from the host until the transaction succeeds and the required durability/
   original-deadline gate succeeds. There is no transaction or SQLite snapshot
   held across the next frontend call or object I/O.

Begin reads and fully decodes the selected current parent through the qualified
physical directory reader. It preserves native behavior when decoded ID/version
or denormalized key columns disagree with the selected row: access derives from
the decoded key. `personal::require_key(Read)` must succeed. Drop the full parent
input/schema tree before issuing the token; Begin returns no List entries and
does not retain a16MiB parent between calls. Only this checked SQL path may call
the crate-private issuer. Begin returns its actual typed binding, opaque token,
random bundle ID, ordinal0 and normalized count. The frontend checks the returned
context/arguments against its own originals before constructing ReadBundle.

Next verifies token MAC/canonical fields and the entire supplied binding before
using its derived parent key or cursor. It also performs the current SQL proof
above and rechecks read access to that original parent key. The dependency proof
is what permits avoiding repeated full parent decode; without verified complete
hooks this optimization is disabled, not justified by the token alone. A changed
parent/body/credential or activation invalidates the entire read.

Choose the existing PageStatement from the original cursor and token position:
initial order by(mac,id), public MAC start strict `>`, public Time start inclusive
`>=`, subsequent private tuple `>` in the same ordering. Set limit to
min(token.remaining, statement.effective_rows()), currently at most15 for the
qualified List row contract. Never fetch an extra public entry to alter native
final_page. One query result is retained at a time. Check every probe before
materializing/decoding payloads, distinguish NULL/missing from wrong type or
suppression, and decode every exact dirent with the existing Rust decoder.

Optional small bodies are chosen by the stored SQL node_id type3, not decoded
dirent node_id, matching native List. A missing body is omitted. A present body
is decoded even if its key later proves unreadable, so malformed present content
still fails as native does. Only an ordinary Permission result from the shared
key predicate causes omission; an SQL/query-bound/clock/corruption failure aborts.
Do not blanket-catch every personal error as an unreadable optional body. Parent
permission failure aborts the whole call. No per-dirent key check is invented.

Derive the last tuple from stored ordering columns and advance the token in the
same transaction. Short page means complete/final_page=true; consuming the exact
remaining count means complete/final_page=false. Empty completion advances the
ordinal once and preserves the previous tuple. No Next on a completed token.
A replay of the same token at the same coordinates may return identical bytes;
this stateless property is explicit and never permits duplicate frontend append.

Finish accepts only a MAC-verified complete token, repeats all current proof and
parent-key-access checks, and returns a receipt for the exact binding/bundle/
ordinal/count/final_page. It executes no List query and performs no new mutation
other than existing authority clock bookkeeping. Its successful transaction is
the read linearization point. Mutation/promotion strictly afterward does not
retroactively invalidate that observation, and this is not a claim that socket
publication is atomically fenced against a later promotion.

### Private transport and error origin

Use new versioned bounded binary read-bundle input/output exports, distinct from
the existing64KiB scalar JSON output and from raw SQL page ABI. Pin their complete
layout/signatures in the artifact fingerprint; old hosts must fail activation.
Inputs are at most64KiB, with bounded fixed-order context serialization and
exact180-byte tokens, never giant JSON integer arrays. The exact proposed layout follows below and needs the same plan review; no API
may allocate using an unvalidated claimed length.

A successful Next output is at most1MiB including its fixed header, binding
SHA256, bundle/served ordinal, row count, next token and rows. Rows contain only
canonical Rust-encoded dirent bytes and optional permitted canonical small-body
bytes, each with checked lengths. Reserve header overhead in PageSpec/effective
limit arithmetic; do not append it to an already1MiB response. Recompute that
at least one maximum row fits, and record whether the existing limit15 remains.
The last SQL tuple is incorporated by the kernel into the next token; JS neither
decodes Snowpack nor decides access nor supplies the continuation tuple.

The concrete input envelope uses24 bytes: magic FKLI(4), ABI u16=1, operation
u8(1 Begin/2 Next/3 Finish), reserved u8=0, total_length u32, context_length u32,
binding_length u32, token_length u16, reserved u16=0. All integers are little
endian. Begin carries one bounded strict typed context/arguments JSON value and
no binding/token; Next/Finish carry one canonical typed binding JSON value and
exactly180 token bytes, with no separate context. Fresh trusted now is supplied
out of band on every operation by the sealed host invocation, alongside the
connection-affine VerifiedSchema and original owner capability; it is not the
old time in a replayed Begin context or a frontend-supplied binding field. The
same host invocation retains the original deadline/domain from the authenticated
operation context. Never trust public now or renew the deadline on continuation. Body lengths must add exactly to total<=64KiB before allocation;
unused fields must be zero, unknown/trailing metadata rejected. The repeated
binding carries no secret and its digest must match the token; JSON member order
is never the source of the authoritative canonical digest.

Success output has80-byte header: FKLB(4), ABI u16=1, operation u8, flags u8
(complete/final only), total_length u32, ordinal u16, row_count u16, normalized_count
u16, cumulative_returned u16, remaining u16, token_length u16, binding_length u32,
row_payload_length u32, bundle_id16, binding_digest32. Then binding bytes, token
bytes, and rows. Begin has canonical binding+180-byte token, ordinal0/no rows;
Next has no binding bytes,180-byte next token and the *served* ordinal; Finish
has no binding/token/rows and the complete token's next ordinal. Each row is
u32 dirent length+bytes, optional flag0/1, then if1 u32 small-body length+bytes.
Lengths are bounded4096/65536, with no ignored bytes or row padding. The worst
row is69,641 bytes, fixed Next overhead260, so15 maximum rows still fit1MiB.
Header count arithmetic and token metadata must agree. A separate returned
nonzero status uses the bounded existing typed error codec, never a success
header plus partial rows. Pin native/Python or independent fixture goldens for
every header, integer and optional-body boundary before host integration.

New exports are read_list_input_alloc, read_list_begin, read_list_next and
read_list_finish with the existing bounded output pointer/length convention.
Production activation requires an explicit reviewed export allowlist. Reject all
raw unauthenticated fixture entry points, including run_authority, run_personal,
run_page, run_directory_key and run_read_token_fixture/its allocator, as well as
any future fixture issuer or mutation export, even if built through all-features/
feature unification. Only required memory/setup and real authenticated/proven List
entry points may be present; an unrecognized function export refuses activation. An authenticated fixture
capability is not permission to install such a module in production. Keep an
explicit export/ABI/artifact allowlist; test the rejection of the otherwise valid
qualification artifact. Default-Cargo-build absence alone is not this gate.

Host JS copies opaque bounded bytes, wraps exactly one storage.transactionSync,
and then awaits storage.sync before releasing a successful receipt. Existing
shared secret setup and poison/wipe/discard lifecycle apply to every export,
including input allocation. Normal returned Rust errors are reusable; imported
exceptions/traps abort and poison. No read error becomes a durable write-domain
rejection. Typed bundle errors include LostAuthority, DependencyChanged,
StalePreparation, Expired, Unauthenticated, Permission, Missing, UnsupportedProfile,
Corrupt and separate infrastructure/budget failures. Errors contain no partial
entries. Public status mapping is a server adapter responsibility tested against
real native dispatch; in particular Missing parent is KvNoEnt and malformed
stored data remains TransactionRetry. Retryable bundle restart is private and
must start with empty output and the original deadline. A typed new internal
error does not create a new public RPC status.

The first host qualification remains a private fixture service. Production B
must authenticate each owner-specific frontend capability and construct the
actual fence/schema/key bindings; accepting a shared token plus a caller-asserted
owner is not sufficient. Frozen fixture time, guarded between-SQL callbacks and
storage.sync do not establish interruption of synchronous SQL or a hard15-second
response. Late/failing sync releases no response; a hanging sync remains a
production deadline gate until bounded physical work is qualified.

### Bounded frontend assembly and publication

Implement the shared Rust assembler over a narrow internal backend trait with
Begin/Next/Finish methods taking an immutable original operation context. Supply
a native direct-transaction adapter and an in-process Wasm qualification adapter
first. A network adapter requires the separate authenticated persistent B plan.
The assembler owns the only mutable ReadBundle and ListEncoder; no caller can
append a standalone PageBatch without matching the owned state.

Before accepting a page validate ABI/total length, exact context digest, bundle,
served ordinal, row count, count deltas/terminal flags and next token metadata.
A duplicate, reordered, cross-bundle or truncated response destroys the candidate.
Backend transport authentication supplies response provenance; frontend parsing
of an opaque token without the secret is not MAC verification. The next kernel
call independently verifies that MAC. Borrow the validated one-page buffer,
decode one returned canonical entry/body at a time, call the already reviewed
ListEncoder::append and drop those values before the next row. Its repeated
normalization is intentionally retained for the first implementation and measured,
not replaced with unchecked concatenation. Extended positions derive from the
encoder's global accepted count, including omitted bodies.

When complete, finalize the candidate and check actual public framing size before
Finish. This keeps large allocation/encoding ahead of the final observation and
retains the unchanged RPC framing oracle (payload16MiB alone is not sufficient).
Check original deadline/cancellation before and after local work and every call.
Only a matching FinishReceipt creates PublishableList. Immediately before handing
that value to the existing response path, check the same deadline/cancellation;
transport permits remain owned through the physical socket flush. No public
byte or partial status/data frame is emitted before final proof. Delayed in-flight
responses after a later promotion are classified by the earlier linearization,
not misreported as a divergent write.

Do not retain all274 private pages needed for a4096-entry worst-case page. The
existing Request budget is cumulative and holds reservations until request/output
drop; naively nesting every temporary page under it can accumulate hundreds of
MiB of conservative maximum-page charges despite small actual results. Qualify
full4096 behavior rather than assume the512MiB default working pool makes it safe.
Prefer a separately reviewed fixed-capacity page workspace reserved once for
this request, with exclusive RAII occupancy for one physical page operation at a
time. The final ownership design must account for transport, encoded page,
decoder and SQL-binding capacities before allocation, including overlap during
handoff. The reservation remains held until all physical work and borrowed views
finish, even after caller timeout; no second page can reuse that workspace while
the first worker is still active. Retained candidate and RPC output allocations
stay independently charged to the parent. Append/growth of ListEncoder must not
consume a disposable page credit. Ordinary Request admission must still reject
other allocations and propagate its failure latch.

Neither this preferred workspace nor a nested Request scope is approved merely
by this overview. L2 requires a concrete type/lifetime and allocation-accounting
plan before source changes. In particular, existing RetainedValue::into_inner
can detach a value from a scope's permits; borrowing or moving any page value
must not release its reservation before the last owner is gone. If a child scope
is ultimately necessary, shared failure state, physical task lifetime and retained
value extraction must be redesigned and tested together. Test allocation refusal,
late detached completion, cancellation, parent failure and append promotion; do
not enlarge pools or lower public count to hide overcharging.

Wasm holds the token authority, one transaction and one bounded page, never the
whole public candidate. Native frontend owns the final candidate and its framed
response. Directory decoding temporarily reaches its separately measured high
water and must finish/drop before page work. JS declared admission, Wasm linear
memory, native retained permits and engine RSS remain separate measurements.

### Groups, ownership and acceptance gates

L0 is the prerequisite decision/qualification: schema59 hooks and physical
representation, exact schema-cookie query on actual celld, activation guard and
original deadline source. A bounded schema-guard constructor must not accept a
frontend-supplied hash/cookie as proof. Test same-cookie different connections,
reopened/replaced storage and instance activation changes. Keep native/shared semantics work separate
from resolving full-size celld physical representation; report blocked gates.

L1 adds the shared proof resolver and ListBegin/Next/Finish kernel plus exact
private transport codec. Recovery owns read_bundle and codec/host glue; storage
owns pinned dependency/schema SQL additions in coordination with its schema59
owner (quota); root owns narrow server native-oracle helpers and eventual public
status mapping. Freeze exact interfaces/statement IDs and binary layout before
editing shared catalog/wasm modules. Author and independent review precede tests.
Native differential gates cover0/1/14/15/16/64/65/4095/4096/4097 and zero/large count
clamp; empty/nonempty initial/MAC/Time, duplicate names/times, missing parent,
mismatched decoded parent ID/version, selected-vs-decoded node IDs, omitted unreadable
or missing bodies, malformed present bodies and historical normalized encoding.
Include actual Rust and pinned Go account fixtures with unchanged public calls.

L2 adds frontend assembly, proof ownership, framing/status adapter and any reviewed
page workspace ownership/budget design. Root owns server memory/publication integration;
recovery owns private assembler and deterministic fake/direct backends. Test
truncation, oversized declared page, duplicate/reordered/mixed tokens, exact retry,
missing/incorrect Finish, full/public frame boundary, allocation refusal at every
ownership boundary, disconnect, expiry, slow socket, detached work and no partial
publication. One error must discard the whole candidate and prevent reuse.

L3 qualifies actual native and celld transactions with schema59 mutation hooks,
not manual counter increments. Pause after Begin, between pages and before Finish;
mutate/revoke/demote/renew/rotate peer keys, parent bytes/head, dirents/order/tombstone,
optional body missing→present/content/key, SSO profile, dependency incarnation,
schema cookie/hooks, owner and activation. Test ABA where visible bytes revert
but dependency revision differs. Clock bookkeeping/Observe/Register/Settle/Prune
must not self-invalidate the dependency coordinate, though the separate intent
revision may conservatively invalidate. Verify nested domain/outer failure rolls
back coordinates and bytes; no successful Finish accepts stale data. Test a
mutation strictly after successful Finish as valid earlier linearization.

Actual-cell tests use persistent initialized SQL across a bundle, not one reseeded
snapshot per page; exact oracle snapshots occur only at fixture boundaries. Test
host-key loss/instance trap and restart using fresh token authorities. This still
does not qualify cell/node/AZ failover until integrated with the persistent owner/
replication service. Keep maximum native16MiB parent cases in the suite and record
physical storage failures until a full compatible representation passes them.

Measure Begin, each page, encoding, Finish and total latency; total SQL calls,
actual/cumulative/peak admitted bytes, Wasm high water, JS working bytes and RSS.
Run4096-entry reads with maximum representable and ordinary bodies, unrelated
writes,64 concurrent personal tenants and repeated invalidation/restart under the
original deadline, within declared local resources. Requests that exceed public
framing are expected errors, never reduced successful pages. Preserve every
15-second miss; many sequential private calls may themselves be a bottleneck.
No result from these groups establishes thousands-of-tenants density, full KV
route coverage or persistent production HA. Each group gets separate reviewed
source, gates and commit; Markdown remains unstaged.

Author review: this design does not trust a frontend parent key or dependency
coordinate, does not keep snapshots across network calls, and does not decode
permissions in JS. It preserves native optional-body/error ordering and exact
public pagination/framing while making transient admission ownership explicit.
The celld physical BLOB limit, schema guard, real principal/owner binding and hard
SQL/deadline completion are named prerequisites rather than silently assumed.
Exact transport layout, scope ownership and production activation still require
root and independent plan review before implementation.

The diagnostic run-c identified ten additional celld-owned schema objects and zero
missing FOKS definitions. The reviewed fixture correction pins their exact DDL in
`tools/foks-metadata-spike/celld-directory-schema.json`: `_cf_KV`, `_cf_ALARM`,
`_cf_METADATA`, `_cf_FACETS`, `_cf_WAKE`, `_litestream_lock`, `_litestream_seq`
and the three `_cf_WAKE` triggers. The worker explicitly supplies this immutable
platform baseline; native fixture helpers default to an empty baseline. Actual
schema must equal the full FOKS-plus-platform union, with missing, altered or
additional reserved-name objects rejected. This is not a wildcard exclusion or
an observed schema blessing itself. The JSON is included in controller pre/post
artifact hashes. Assertions cover every FOKS data row and all DDL; celld-owned
bookkeeping row contents are outside the FOKS oracle and this is not a durability
proof. Native expected directory outcomes and all eight cases remain unchanged.

Native KV namespace admission currently performs an equal-value UPSERT outside
its later domain transaction. Consequently even upload preflight and exact native
KV retries can advance the conservative dependency coordinate. Concurrent retries
can invalidate an otherwise unchanged List bundle and require a bounded restart;
this is a responsiveness/throughput qualification concern, not data loss. Receipt
lookups which execute no domain DML (identity/full-rename known success) remain
coordinate-preserving. The schema change deliberately does not optimize these
existing admission semantics.

### L2 concrete page workspace ownership proposal (root review; independent review pending)

The existing Request allocation budget is cumulative through physical output.
Repeated bounded List pages must not charge and retain a new maximum page allowance
for every call. Do not introduce a child Request: existing RetainedValue::into_inner
assumes the awaiting request still owns its scope, which would be unsound if it
released a child's permits while returning the child's live bytes.

Proposed simpler ownership: reserve one fixed page-working allowance from the
existing parent Request working pool before Begin/Next orchestration. Keep that
reservation in the parent's existing permit vector until the entire physical
request/output owner drops. A PageWorkspace holds Arc<Request>, the fixed allowance,
and one exclusive occupancy flag. The exact allowance must be derived from the
bounded transport/SQL-page/decoder/temporary-encoding allocation audit and tested
at maximum valid inputs before selecting its constant; do not increase node pools
to make tests pass. Directory decoding is a separate one-time phase and final
candidate/output allocations remain separately charged to the parent.

A non-public PageLease acquires that occupancy with a nonblocking atomic operation;
there is never a queue or second live page lease. Its allocation budget counts
cumulative temporary bytes for that page against the already charged allowance.
Failures latch the parent Request failure state. Clones share the same count and
occupancy; only the last physical owner dropping permits a new page. The lease
holds the workspace and thereby the original Request, so cancellation cannot
release the pool reservation while a blocking query or network buffer remains.
Lease counters are fresh for the next page only after the previous lease and all
of its values are gone. There is no public reset-counter operation.

Install the lease only as the snowpack/SQL allocation budget, leaving the existing
Request CURRENT and RPC response budget bound to the parent. A page scope must
not become an alternate response-memory owner or override the original deadline.
Use a private PageValue<T> wrapper whose value field drops before its Arc<PageLease>.
It exposes borrowing/validated iteration, not into_inner or an unrestricted owned
extraction. The page backend constructs it before returning from the physical
worker; the worker retains a lease before its first allocation and also on error.
The wrapper crosses any join/network boundary intact. Dropping a caller/waiter
leaves the worker and its eventual value owning the lease until physical cleanup.
Do not rely on a timeout or on the generic RetainedValue extraction convention.

For frontend assembly, borrow one checked row from PageValue, decode its transient
values under the lease, switch explicitly to the parent allocation budget while
ListEncoder::append copies/encodes retained output, then drop the temporary row.
Drop the whole PageValue before acquiring the next lease. Parent failure, deadline
expiry, cancellation or a malformed page destroys the candidate. At most one page
workspace exists per List operation; creating it is not an unbounded new admission
mechanism. Keep all helpers private until the actual List adapter uses them, and
do not apply this optimization to unrelated existing response paths.

Required deterministic ownership tests: two sequential pages whose cumulative
historical charge exceeds one workspace both succeed after physical drop; a
second live lease refuses; page allocation refusal latches parent failure; retained
encoder allocations are charged separately; a blocked worker plus canceled caller
retains occupancy and parent permits; a queued unpolled task/value drops payload
before lease release; error and panic unwind release exactly once; nested budget
scopes restore the parent; a slow socket still owns candidate and output permits.
Use barriers and observable pool counts, never sleeps. Full4096-entry native/Wasm
List tests and allocation/refusal traces must validate the derived allowance and
absence of whole-page accumulation before enabling this path. This is a concrete
proposal awaiting independent review, not implemented or qualified behavior.

Actual-celld directory qualification record (2026-10-16): the seven-file fixture
extension passed author, root and independent source reviews, six Node negative/
parity tests and three Python artifact tests. Run-d preserved all eight native
oracles and finished with six passing cases and two failures: `maximum` and
`malformed_maximum` both fail at the unchanged SQL value/row limit before kernel
execution. Overall qualification remains false. The passing cases compare exact
replies, all FOKS data, complete payload bytes and the exact FOKS/platform DDL
union; no production List route is enabled. Maximum observed Wasm memory among
these representable cases was 3,145,728 bytes and declared page working bytes
2,630,213; these exclude fixture transport/assertions, SQLite internals and JS RSS.
The native/Wasm-only 16 MiB decoder result does not establish celld storage
compatibility. Evidence: `/private/tmp/foks-directory-celld-d/qualification.json`,
`/private/tmp/foks-directory-celld-final-report.json`, and the immutable seven-file
manifest/hashes `/private/tmp/foks-directory-celld-{manifest.txt,sha256.json}`.

Final fixture hygiene also passed the three existing Wasm-limit tests (six Python
tests total with artifact checks), launcher/helper Python syntax and worker/helper
JavaScript syntax. The cell's `execution_ms=0` values use frozen fixture time and
are not latency evidence. No native codec or production source changed for this
fixture qualification.

### L1 exact schema authority, binding decoder and SQL interface subplan

Root and storage independently reviewed the preceding List plan. Root approved
its24-byte input and80-byte output layouts, with strict binding-wire decoding and
current schema authority mandatory. The fixed-workspace overview passed review,
but its allocation/type API remains a separate pre-code gate. This narrower L1
proposal is not yet implementation approval.

**Schema comparison ownership.** Schema59 already gives each Database and
ReadDatabase a private ReadDependencySchema; ReadSnapshot borrows its own handle's
proof and checks the schema cookie within its transaction. Preserve that native
contract. Do not turn its public cookie/hash getters into a cross-connection
capability or copy a proof into arbitrary NativeSql instances.

Propose a small pure `foks-schema-contract` crate shared by server-db and the
metadata kernel. It owns only ObjectKey, SQL lexical comparison, canonical
contract hashing and comparison policy. It has no SQLite access, expected schema
bootstrap, caller authentication, activation constructors or token issuance.
Move the existing lexer/hash semantics without broadening identifier or literal
normalization: preserve case-sensitive literal bytes and the exact existing
historical ALTER identifier exceptions. A bounded borrowed token iterator avoids
allocating a Vec per SQL token; compare/hash by streaming with checked lengths.
Pin old/new hashes and comparison decisions against genuine historical fixtures
before replacing the native helper. Native verification keeps its current policy
(required exact FOKS objects, reject extra dependency objects and extra triggers
on authoritative tables); this extraction must not silently reject unrelated
extension tables that native currently permits.

Hosted activation uses a stricter explicit union: exact generated FOKS contract,
the immutable private authority schema required by the sealed resolver, and exact
pinned engine-owned object definitions. No wildcard prefix exclusion,
live-schema-as-expected fallback or caller-uploaded expected manifest. Generate
the FOKS manifest offline from the same initialized schema/reference connection
used by native verification, including schema59. Check it into or embed it in the
qualified artifact with its canonical hash; a regeneration test requires exact
agreement. The celld platform manifest is separately source/observationally
qualified and hash-bound to its pinned version. Its allowed objects are not
silently included in the native FOKS contract hash. Activation binds both hashes,
physical-profile ID and ABI/artifact hash. Other engines require their own explicit
platform profile rather than reusing the celld labels. The private authority DDL
has its own pinned hash/version and is part of hosted equality; do not mistake
native acceptance of unrelated extension tables for verification of this required
private authority state. Freeze the hosted schema fingerprint as a new private
versioned domain over the FOKS hash, private-authority hash, engine-profile hash
and physical-profile identifier, using explicit length-delimited typed fields.
Activation.schema_hash holds that composite for the new hosted ABI; native
ReadDependencySchema.contract_hash remains its existing FOKS-only hash. Old
activation values/tokens do not silently acquire the new meaning: activation
version/ABI validation must reject them. Pin a golden composite-hash vector.

**Native and Wasm proof factories.** A native pooled `VerifiedNativeConnection`
owns the admitted rusqlite connection and private contract/cookie metadata. Its
constructor performs verification on that connection in a transaction using the
shared checker. Its `transaction` method lends a `VerifiedReadTransaction<'_>`
containing NativeSql and the same connection's proof; the guard is not Clone,
Serialize or publicly constructible. Beginning that transaction checks the cookie
again, and its kernel-facing API supplies the proof without taking a separate
caller-provided connection. Its wrapper withholds the callback result until COMMIT
succeeds; callback errors roll back, and a DDL-capable callback requires another
cookie check immediately before commit. A successful Rust callback return alone
is never a publishable result. The existing server-db handle may expose an analogous
verified transaction callback after separate API review; it must lend its actual
connection and proof together. Prefer reusing its already-admitted connection
rather than reverifying per RPC, but do not use an unchecked callback/hash adapter
to claim that reuse. The qualifier can initially own its independent verified
connection. Full schema scans occur on admission/reactivation only. The shared
current authority Prove operation currently updates its clock floor: this path
requires a writable transaction and cannot be wrapped in a query-only ReadSnapshot
or silently treated as a pooled read lease. Qualify with a host-owned writable
connection; any production integration must use the existing writer ownership/
admission boundary or separately review a clock-proof redesign. Measure this
write serialization rather than claim concurrent read-only throughput.

The Wasm host owns an immutable storage object and one kernel instance. Activation
is a separate export/state machine: enumerate actual schema through pinned typed
SQL, compare against the artifact's expected contract, and store cookie/profile/
contract metadata inside that instance. No request or setup argument can assert
"verified=true" or substitute the expected contract. Replacing storage/instance,
changing deployment activation or losing the ephemeral token authority destroys
the guard and all outstanding bundles. It is forbidden to rebind the JS storage
object after activation. Before every operation, the same instance queries the
cookie through the same storage object inside the operation's transaction. Equal
cookies from two databases do not authorize swapping them. Activation and request
exports share the existing trap poisoning/wipe/drop discipline.

Activation enumeration is private setup work, not an ordinary List page. Proposed
bounds: at most1024 objects,64KiB UTF-8 SQL per object,256 UTF-8 bytes each name and
table name,8-byte object kind and2MiB aggregate schema text. Check lengths and types
before materializing text; consume one bounded SQL page at a time, retain at most
one bounded schema snapshot and its borrowed token views. Qualify these limits
against the generated full schema and actual engine union before adopting them;
a fixture outside them fails activation, never truncates. Native existing admission
need not acquire new hosted-only extension-table caps. Measure activation memory
and discard all actual-schema buffers before any16MiB directory decode. The exact
activation binary framing and allocation accounting require their own source/API
review before the export is implemented.

**Reserved SQL interfaces.** Scalar catalog IDs1..42 and page IDs1..8 remain
unchanged. Reserve43 `ReadDependency` and44 `ReadSchemaCookie`; storage owns their
statements and tests.43 has no parameters and returns one row with these five
columns, in order:

1. `typeof(incarnation)` as Text;
2. `CASE WHEN typeof(incarnation)='blob' THEN CAST(length(incarnation) AS TEXT)
   ELSE '-1' END` as canonical signed decimal Integer;
3. `CASE WHEN typeof(incarnation)='blob' AND length(incarnation)=32 THEN
   incarnation ELSE NULL END` as NullableBlob;
4. `typeof(revision)` as Text;
5. `CASE WHEN typeof(revision)='integer' THEN CAST(revision AS TEXT) ELSE '0' END`
   as canonical signed decimal Integer.

The table predicate is `metadata_read_dependency WHERE singleton=1`. Require one
row, incarnation type exactly blob, length32, nonzero bytes, revision type exactly
integer and positive<=i64::MAX. Missing, NULL, wrong type and suppressed payload
are distinct invalid-schema outcomes; the sentinel0 is never a valid coordinate.
Do not allocate a malformed stored blob before its type/length probe. Coordinate
construction in core remains private validated ReadDependency; do not expose a
"SQL proof" constructor accepting a frontend coordinate.

44's first candidate is exactly `SELECT CAST(schema_version AS TEXT) FROM
pragma_schema_version`, no parameters, one canonical Integer column. Before
merging it, run a separate actual celld-authorizer probe inside a transaction,
including repeated reads and an actual DDL change, and compare native behavior.
Do not infer permission from allowed read-only PRAGMA. If the table-valued form
is denied, stop that candidate and review a narrowly typed host `schema_cookie`
primitive executing only read-only `PRAGMA schema_version`. The native and JS
implementations must check an exact nonnegative32-bit cookie and convert to exact
canonical decimal/integer representation; do not introduce caller-selected SQL,
floats for general i64 values, or a supplied-cookie fallback. Record denied probes
as evidence rather than quietly changing catalog semantics.

Reserve page ID9 `SchemaObjects` for activation enumeration, separate from public
List. It uses keyset order `(type COLLATE BINARY,name COLLATE BINARY)` with an
explicit first-page flag or after the last verified tuple and a checked positive effective row limit. Return explicit
type/UTF-8-byte probes and guarded payloads for type/name/tbl_name/sql; text lengths
use `length(CAST(value AS BLOB))`. Exclude only the literal SQLite-internal `sqlite_` prefix, using a reviewed literal
prefix predicate rather than copying SQL LIKE's underscore wildcard. In particular,
a custom sqliteX object must not disappear from hosted verification. Preserve the
existing native selection policy unchanged during pure checker extraction. Then
compare every other observed object against
the complete declared profile. The SQL, descriptor and cursor sentinel need a
separate frozen catalog review; no unbounded schema JSON through the old scalar
interface. Host rows use canonical decimal integer strings and the existing binary
page ownership model.

**Binding-wire decoder.** Add `ListBinding::decode_wire(&[u8])` using a private
strict DTO, separate from the public-count-normalizing `ListBinding::new`. Enforce
the32768-byte input bound before deserialization, exact domain-v2 and
personal_list route, exact field widths, validated peer/fence/dependency fields,
nonnegative signed Time, count strictly1..=4096, positive revisions/epochs and
canonical Number decimal strings. Reject duplicate fields at every object level,
unknown fields, unknown enum variants, missing fields, trailing content and
noncanonical integer text. Never pass an incoming zero/oversized count through
the Begin clamp. Construct the private binding only after all checks, then compute
the canonical typed digest independent of JSON member order. Tests must exercise
nested duplicate identity/activation/peer/dependency/cursor fields, wrong domain,
wrong route, zero/4097 counts and precision around2^53/i64::MAX. Begin retains
its separately documented full public unsigned Time semantics and error precedence.

**L1 gate and ownership.** Recovery owns the strict decoder, core read_bundle,
private native factory and host activation state machine; storage owns43/44/page9
and native SQL/oracle tests; quota reviews any changes touching schema59's native
proof/checker extraction; root owns the native server transaction borrowing seam
and public status oracle. The pure-checker extraction, actual schema-cookie probe,
activation framing/memory and native callback interface are individually reviewed
subcomponents before implementation. Keep qualification token issuance behind its
existing explicit fixture feature until Begin genuinely performs all current SQL
proofs. Production activation rejects all fixture issuer exports even when the
artifact also exposes valid real Begin/Next/Finish. No flag turns synthetic fixture
principals or caller-supplied activation labels into real owner authentication.

L1 tests include two distinct databases with equal cookies, schema changes after
admission, reopened/replaced connections, altered/missing/extra FOKS and platform
DDL, literal-preserving normalization, malicious overlong schema text, poisoned
activation and fresh-instance recovery. Exercise actual schema59 hooks and rollback
rather than manually bumping revision. Native schema factory tests and actual celld
small-row List qualification do not resolve the separately blocked physical row
range, current production capability/bootstrap, SQL interruption, or L2 final
publication ownership. Those limitations remain in reports and activation gates.

### L1 storage-owned SQL and schema-cookie probe — concrete proposal

Author review completed 2026-10-16; root and independent review are required
before source changes. This is fixture and interface planning only. L1's guard
and shared-checker design passes storage's independent review subject to the
explicit activation/memory/native-callback gates already listed above. Production
artifact admission must reject every raw unauthenticated qualification entry
point, including `run_authority`, `run_page`, `run_directory_key` and the token
fixture issuer/allocator, not only the latter. A correct real List export does
not neutralize another exported bypass in the same instance.

**Probe first, no automatic fallback.** Add a separate small private fixture mode
or standalone fixture worker/controller to the existing local celld lab. It must
use immutable pinned source, the same celld binary, local Moto, unchanged heap and
storage limits, and one fresh cell. A random private capability is checked before
SQL or cell acquisition. The fixture accepts no SQL, schema cookie, activation
label or arbitrary command from HTTP. Hard-code the candidate query and a fixed
DDL table name. Return a bounded report with raw result type, canonical decimal
value, phase and any denial; retain source/runtime hashes and failures. No Wasm
module or normal RPC route needs changing merely to test this engine capability.

Within separate synchronous transactions: (a) run the exact candidate twice and
assert one canonical decimal TEXT column, identical values, range0..u32::MAX;
(b) read, CREATE a small fixed table, read again, throw a fixture sentinel and
then prove the original cookie and absent table after rollback; (c) read, CREATE
and commit that same fixed table, read again and prove the cookie changed, with
repeated reads stable; (d) DROP the table in a committed transaction and prove
another cookie change and absence. Compare these relations and SQL result types
with the same statements on native SQLite. Do not require identical initial
cookies across engines or monotonic wraparound behavior. All cookies in this
fixture remain small; reject rather than truncate an out-of-range value.

Attempt the table-valued form first, exactly:

```sql
SELECT CAST(schema_version AS TEXT) FROM pragma_schema_version
```

If celld denies it, preserve the denial as a failed candidate and stop adoption
of scalar44. A separate reported diagnostic may execute only the fixed read-only
`PRAGMA schema_version` to confirm its permission/type; success does not qualify
an integer ABI change. A typed host-cookie import requires its own review and
must validate the bounded32-bit value before exact conversion. Never catch a
candidate denial and silently continue through an alternate path. Negative
fixture tests cover wrong capability causing zero SQL, unexpected row/column or
noncanonical result, rollback sentinel provenance, and controller failure when
any candidate phase fails. A local native relation test and actual celld report
are both acceptance gates; no latency or failover claim follows.

**Scalar43.** Freeze `ReadDependency`, no parameters, read-only, maximum BLOB32,
with SQL and columns as follows (SQL spelling becomes catalog artifact input):

```sql
SELECT typeof(incarnation),
       CASE WHEN typeof(incarnation)='blob' THEN CAST(length(incarnation) AS TEXT) ELSE '-1' END,
       CASE WHEN typeof(incarnation)='blob' AND length(incarnation)=32 THEN incarnation ELSE NULL END,
       typeof(revision),
       CASE WHEN typeof(revision)='integer' THEN CAST(revision AS TEXT) ELSE '0' END
FROM metadata_read_dependency WHERE singleton=1
```

Columns are Text, Integer, NullableBlob, Text, Integer. `typeof` text is bounded by
SQLite's fixed type vocabulary. The caller requires exactly one row, BLOB type,
length32, nonzero incarnation and positive integer revision; sentinels cannot
construct a valid coordinate. Test missing row, zero/NULL/wrong-type/oversized
incarnation and invalid revision using isolated malformed-schema fixtures,
including refusal before copying an oversized BLOB. Scalar44, conditional on the
probe, is the exact candidate query above, no parameters, read-only, one Integer
column and zero BLOB allowance. No scalar/page IDs1..42/1..8 change.

**Page9 SchemaObjects.** Use one immutable PageSpec with parameters:
`initial` Integer0..1, `after_type` Text(max8 UTF-8 bytes), `after_name` Text(max256),
`limit` Integer1..effective_rows. Initial calls require empty cursor fields;
continuations require a previously validated nonempty type/name tuple. The
initial flag avoids an unproved empty-string sentinel excluding malformed rows.
The selection is:

```sql
FROM sqlite_schema
WHERE (typeof(name)!='text' OR substr(name,1,7) != 'sqlite_' COLLATE NOCASE)
  AND (CAST(?1 AS INTEGER)=1 OR
       (type COLLATE BINARY,name COLLATE BINARY) > (?2 COLLATE BINARY,?3 COLLATE BINARY))
ORDER BY type COLLATE BINARY,name COLLATE BINARY
LIMIT CAST(?4 AS INTEGER)
```

The seven-character reserved prefix comparison is literal and ASCII case-insensitive;
do not use `LIKE 'sqlite_%'`, whose underscore also matches non-internal names.
SQLite-generated internal object names are excluded; every other row is returned,
including a NULL SQL value so it cannot silently hide an unexpected object. Root
is correcting the same wildcard bug in the uncommitted schema59 native verifier;
pure-checker extraction preserves that corrected predicate and the separately
reviewed native extension-table policy, not the accidental wildcard behavior.
Stricter hosted equality is not a silent native admission change. Test an allowed non-internal `sqliteX`-prefixed extension is visible and
rejected by the hosted expected union.

Project each of `type,name,tbl_name,sql` as three columns: `typeof(value)` Text7;
`CASE WHEN typeof(value)='text' THEN CAST(length(CAST(value AS BLOB)) AS TEXT)
ELSE '-1' END` Integer; and `CASE WHEN typeof(value)='text' AND
length(CAST(value AS BLOB))<=CAP THEN value ELSE NULL END` nullable Text(CAP).
CAPs are8,256,256,65536 respectively. Thus12 columns retain wrong-type versus
suppressed/NULL distinction and bound bytes before materialization. Unexpected
NULL SQL, unknown kind, empty name or inconsistent type/length fails activation;
none is treated as EOF. Accepted kinds are the explicit table/index/trigger/view
set, with no hash input omitted. Cursor advancement uses validated exact type/name
text, never canonicalized SQL or caller-provided names.

Proposed PageSpec maximum64 rows,1MiB encoded bytes; codec computes its effective
row limit from these exact twelve descriptors and parameter-limit contract.
Do not hard-code an assumed effective row count until that arithmetic is checked.
Activation accumulates at most1024 objects and2MiB aggregate SQL text, counts all
returned rows, checks unique tuple ordering, and completes only after a genuinely
short SQL page. `sqlite_schema` enumeration can scan/sort its bounded object set;
this is activation work, not a claim of history-independent hot-query cost.
Measure full schema union cost and actual memory, including the one returned page
plus retained actual/expected contracts and lexical comparison workspace. No
request can supply a larger object/page/aggregate allowance.

Acceptance includes exact SQL/descriptor native-vs-host equality;0/1/effective/
effective+1 objects; multibyte/BOM names and literal-preserving SQL; maximum and
one-over names/SQL; missing/changed/extra platform, FOKS and authority objects;
wrong-type probes where writable-schema test fixtures can represent them; complete
keyset enumeration with no skipped/duplicated objects; and guard reuse/rebinding
across same-cookie different connections refused by the separate guard factory.
Each valid database can still be independently verified and admitted. These statements do not
create that factory or establish owner/certificate proof on their own.

### Independently separable L1 binding-wire decoder

Author-reviewed proposal: implement only `ListBinding::decode_wire(&[u8]) ->
read_token::Result<ListBinding>` in a private `read_token/wire.rs` child module,
plus focused tests. It creates a structurally validated binding, never SQL proof,
a capability or a verified token. No new Wasm export, host activation or catalog
change is part of this commit. Existing public-count normalization and existing
golden token bytes remain unchanged.

The input must be nonempty and at most32768 bytes. Reserve a documented bounded
parse-working allowance before serde allocation; decoding, reconstructed peer
vectors and canonical serialization remain charged, with refusal returning Bounds.
Use a private strict top-level DTO with required domain/fence/revision/dependency/
peer/namespace/parent/route/cursor/count/load_small/deadline/clock_domain fields.
Reuse only audited strict nested structures: Fence, Identity, Activation and
PeerCertificate already deny unknown struct fields and validate decimal integers
or peer parts. Use a dedicated exact-width dependency DTO with strict fields and
ReadDependency::new. Namespace is an exact33-byte array on wire. Validate the
entire existing constructor invariants, exact binding domain-v2 and route, and
count1..=4096 *before* any call to the normalizing constructor. Incoming count0
or4097 is an error, not4096. Recanonicalization must exactly preserve the typed
values rather than accept a mismatched domain/route and overwrite it.

Cursor decoding must not rely on internally-tagged enum flattening. Use an explicit
strict object with required `kind` and a presence-tracked `value`: none requires
absence; mac requires exactly32 bytes; time requires canonical Number text>=0.
Reject value:null, missing values for mac/time, extra values for none, unknown
fields/kinds and repeated kind/value in any order. A scalar/array-only value
visitor avoids buffering an arbitrary nested JSON map. Top-level and nested
struct deserialization must reject duplicate known fields, not merely unknown
fields. Keep the original wire's JSON integer types for activation Limits; do
not route them through JS Number. Audit target-width usize fields explicitly so
native/Wasm acceptance cannot diverge silently; reject bounds beyond u32::MAX for
blob_bytes/dirent_bytes in this private decoder before conversion, while retaining
exact u64 counter fields. This is a private hosted ABI admission condition, not a
change to the native public List count or database configuration.

Native tests cover the existing independent binding/token golden, every cursor
arm, reordered object fields yielding the same digest, escaped known field names,
nested duplicate and unknown fields at every object level, invalid domain/route,
wrong widths, zero/negative/oversized counts and revisions, all-zero incarnation,
noncanonical decimal strings, integer precision2^53/i64::MAX/u64 counter limits,
malformed/trailing/overlong input and allocation refusal before parsing. Run the
same acceptance vectors through a qualification-only Wasm test path if needed;
production exports remain unchanged. Source ownership is recovery's module plus
its declaration/tests; explicit manifest, author and independent source reviews
precede any Cargo or actual-Wasm tests. This proposal awaits storage's independent
plan review before source edits.

### Schema59 integrated qualification and final review

The native schema/restore group passed 237 distinct native tests (one existing
ignored), including three explicitly enabled pinned Go flows. On the combined
metadata branch, 58 kernel tests and strict all-target lint passed. All eight
actual celld 0.6.2/Moto suites passed 112 schema59 reply/state vectors; source
and artifact hashes were uniform. This qualifies the hooks under actual engine
limits for these bounded fixtures, not production activation or recovery latency.

The combined differential oracle initially failed because native namespace
admission performs one equal-value UPSERT before four rejected KV operations,
whereas the kernel's outer transaction rolls everything back. The reviewed test
correction asserts the complete kernel snapshot is unchanged, validates the
private coordinate, requires the native oracle's exact same-incarnation +1 delta,
and normalizes only that revision in a comparison copy. Exported raw snapshots
remain intact; no public row or production behavior is normalized away.

Final root review and independent quota/storage review found and fixed two schema
inspection gaps before commit: LIKE's underscore wildcard could hide sqliteX
objects, and SQLite preserves mixed-case trigger target spellings. Literal
reserved-prefix filtering and ASCII-case-insensitive authoritative-table/reserved
name matching now reject these extras. The canonical expected contract/hash and
unrelated extension-table policy remain unchanged. Eleven focused dependency
tests (including the new regression), strict DB lint, workspace formatting and
diff hygiene passed after those reviews. No public FOKS wire changes.

Frozen implementation: 32 paths in
`/private/tmp/foks-read-dependency-final-manifest.txt`, hashes in the adjacent
`-sha256.json`, results in `/private/tmp/foks-read-dependency-final-qualification.json`.
Actual celld evidence: `/private/tmp/foks-schema59-celld-results.json`. All Markdown
remains unstaged. The independent schema-cookie probe, strict binding decoder
and pure checker extraction remain separate subsequent groups.

### L1a pure schema-contract extraction — concrete component plan

This component extracts the corrected schema59 SQL equality/hash machinery into
`foks-schema-contract`. It changes no database ownership, snapshot, transaction,
callback, guard constructor, activation, catalog, or public request API. The
native `ReadDependencySchema` stays private to its admitted handle, and its
schema-cookie checks remain in the existing transaction. Pure content equality
is never a database, storage, ownership, or authorization proof.

The baseline is commit `3ef60ab7`, including the literal `sqlite_` filter and
ASCII-insensitive rejection of extra triggers targeting authoritative tables and
extra `metadata_read_` names. Root owns native transaction borrowing and recovery
owns hosted activation/composite fingerprints; this component only supplies the
content checker they can call from their separately reviewed trusted factories.

**API and storage ownership.** The new crate has no SQLite, server-db, metadata-core,
randomness, filesystem, clock, Wasm-host, or network dependency. Its only runtime
dependency is the already locked workspace SHA-256 implementation. Proposed API:

```rust
pub struct SchemaObject<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub table: &'a str,
    pub sql: &'a str,
}
pub struct ContractObject<'a> { /* four Cow<'a, str> fields */ }
impl<'a> ContractObject<'a> {
    pub fn borrowed(object: SchemaObject<'a>) -> Self;
    pub fn owned(kind: String, name: String, table: String, sql: String) -> Self;
    pub fn as_object(&self) -> SchemaObject<'_>;
}
pub struct Contract<'a> { /* ordered expected objects and cached content hash */ }
impl<'a> Contract<'a> {
    pub fn new(objects: Vec<ContractObject<'a>>) -> Result<Self, ContractError>;
    pub fn hash(&self) -> &[u8; 32];
    pub fn verifier(&self, policy: MatchPolicy) -> SchemaVerifier<'_, 'a>;
}
pub enum MatchPolicy { NativeRequired, ExactUnion }
impl SchemaVerifier<'_, '_> {
    pub fn observe(&mut self, object: SchemaObject<'_>) -> Result<(), ContractError>;
    pub fn finish(self) -> Result<(), ContractError>;
}
pub fn sql_equal(left: &str, right: &str) -> Result<bool, ContractError>;
pub fn is_sqlite_internal_name(name: &str) -> bool;
```

`ContractObject` permits native ownership of SQLite-returned strings and borrowed
immutable generated artifact strings in Wasm without cloning all SQL text. The
contract owns a single ordered object vector; it does not own token vectors.
Constructor validation requires strict `(kind,name,table)` byte order, rejects
duplicate `(kind,name)` identities and SQLite-internal names, validates every SQL
string and computes the existing hash. Caller-created expected contracts remain
ordinary data: the native adapter obtains its contract only from the internal
reference schema, and the hosted factory later obtains its exact union only from
its immutable qualified artifact. No frontend supplies this argument.

The observed stream also has strict byte ordering and unique `(kind,name)`
identities. A verifier retains only the expected cursor and one reusable previous
key (three strings), plus failure state. It retains no observed SQL text or actual
schema snapshot. `observe` performs a merge against the expected ordered objects;
missing expected objects fail immediately when the stream passes their key, and
`finish` rejects any remaining expected objects. Any observation error poisons
that verifier; a caller cannot ignore the error and later obtain a successful
finish. Methods return only `Result<()>`, never an opaque proof/capability.

SQLite-internal names are excluded by the existing native literal SQL predicate;
the helper implements the same ASCII-insensitive literal seven-byte prefix for
other adapters. `observe` rejects unexpected internal rows rather than silently
allowing a caller to smuggle an unqualified exclusion policy. The future hosted
SchemaObjects reader must apply the explicitly reviewed exclusion before feeding
this stream. Null-SQL autoindexes are likewise the reader's existing exclusion.
These rules keep SQL enumeration and verifier inputs explicit and testable.

`NativeRequired` preserves the current behavior exactly: every expected object
must match; extra `metadata_read_` names fail case-insensitively; extra triggers
whose target matches any expected table fail case-insensitively; unrelated extra
tables/indexes/views and triggers on unrelated tables remain permitted. Permitted
extension SQL is still lexically validated. This is the existing native extension
policy, not certification of those extensions or their effects. `ExactUnion`
rejects every extra object; metadata-core separately constructs its FOKS plus
private-authority plus pinned-platform union and composite activation hash.
No wildcard prefix exclusion, live-schema-as-expected fallback, or policy default
is provided. Required exact object identities remain case-sensitive, as today;
case-insensitive checks only prevent extra-object bypasses.

**Borrowed lexical representation and unchanged hash.** A private fused lexer
returns borrowed byte spans with an Exact or AsciiFold tag. No token allocates a
Vec, String or decoded text. It recognizes the same whitespace/comments, ASCII
identifier/number runs, single-byte punctuation and quoted spans as the current
native implementation. Single-quoted literals and all unqualified quoted spans
retain their complete original spelling, including delimiters and doubled quote
escapes. Only these existing ALTER-introduced identifier spellings normalize:
`chunk_objects`, `chunk_object_attempts`, `kv_file_chunks`, `blocked_reason`, and
`source_hash`. They normalize only in identifier quote forms, never single-quoted
strings. A doubled quote cannot occur in those five names, so their borrowed raw
inner span can be compared without allocating an unescaped copy. All other quoted
content stays Exact. Non-ASCII unquoted bytes retain the current one-byte token
behavior; this is equality canonicalization, not a new SQL parser.

One-token lookahead suppresses only the final semicolon, including when followed
by comments/whitespace, matching the old final-token removal. Truncated quotes or
block comments remain errors. Streaming equality compares paired canonical bytes
and consumes both streams through completion on equality. Hashing makes two
bounded lexer passes per SQL object: first count tokens, then feed their bytes.
ASCII-folded spans use a fixed stack scratch buffer for hash updates; no temporary
lowercased String is created. A fixed byte chunk size is an implementation detail
and cannot affect the digest.

The FOKS contract digest must remain byte-for-byte identical: SHA-256 over
`foks-read-dependency-schema-v1\0`, then each object in existing BTreeMap order;
for kind/name/table, u64 little-endian byte length and exact bytes; then u64
little-endian token count; then each token's u64 little-endian canonical length
and canonical bytes. No new version/domain, normalization, trailing separator or
policy identifier enters this existing hash. The hosted composite hash remains a
separate recovery-owned domain; `ExactUnion` does not redefine the FOKS hash.

**Native adapter and ownership.** `read_dependency.rs` retains private cached
`ReadDependencySchema`, per-connection admission, per-transaction cookie checks,
coordinate validation and private-copy rotation untouched. Its expected-schema
OnceLock/initial-build mutex becomes a cache of `Contract<'static>` assembled from
the same private reference connection. Actual SQLite rows feed the verifier one
at a time through the existing budgeted column reads. Full scans still happen
only at connection admission; no new per-request schema scan is introduced.
The query stays ordered by type/name (SQLite guarantees unique schema identities);
the checker additionally rejects duplicate/out-of-order input. The corrected
literal SQLite-prefix SQL predicate remains unchanged. Checker errors map to the
existing typed `ReadDependencySchema` failure, with no swallowed error or fallback.

Owned implementation paths after both plan reviews: new `crates/foks-schema-contract`
Cargo manifest/source/tests; workspace membership/dependency edge and Cargo.lock;
server-db Cargo edge plus `src/read_dependency.rs` and its focused tests. The
metadata-core Cargo edge/reexport, generated manifest and activation consumer are
recovery-owned later work; coordinate that interface before either consumer edits.
No production guard or transaction API changes belong in this extraction commit.
Use an isolated worktree under QMUX_WORKTREE_ROOT based on `3ef60ab7`, with exact
file manifest/hashes, author review and independent review before any build/test.

**Validation and compatibility gates.** Preserve a test-only copy of the corrected
old canonicalizer/hash algorithm as an independent migration oracle. Compare the
new streaming tokens/equality/hash against it across the actual reference schema,
actual43..59 migrated definitions, every quote/comment/semicolon boundary, explicit
DQS numeric/keyword strings, escaped quotes, non-ASCII literals, malformed tails
and long64KiB SQL/literal spans. Capture a fixed full-FOKS golden hash from that old
oracle and pin it alongside a small manually inspectable fixture vector. The
oracle stays test-only; neither serving path keeps per-token allocations. Tests
must also confirm borrowed token slices point into the source and fixed scratch
bounds, rather than claiming allocation freedom from elapsed time alone.

Pure streaming-policy tests cover missing/changed/duplicate/out-of-order objects,
unknown trailing objects, caller ignoring an error before finish, multiple observe
pages, harmless native extension tables, forbidden sqliteX/SQLITEZ lookalike
triggers, mixed-case authoritative targets/reserved names, exact-union rejection,
and canonical hash independence from observed extensions. Native schema59 tests
must retain both older migration parity and actual same-handle cookie rejection;
run the complete DB suite and strict Clippy after the focused extraction gates.
Build/check the pure crate for wasm32 and qualify memory/throughput only for its
own bounded inputs. Hosted activation framing/memory, actual celld schema-cookie
queries and ownership proof construction remain separately reviewed L1 components;
a pure comparison pass does not complete those gates.

L1a root-review precisions: accepted native extension objects still consume the
entire lexer, so malformed trailing SQL cannot escape validation. The one retained
previous-key copy is covered by native `budgeted_get`'s existing eight-copy plus
record-overhead reservation; hosted activation must charge this retained key in
its separately bounded name/page allocation envelope. The currently measured
schema59 plus private-authority fixture contains317 objects,123853 total SQL
bytes and a1395-byte largest SQL definition. These are fixture measurements, not
permission to omit activation bounds or certify an arbitrary extension schema.

Schema-cookie probe qualification (2026-10-16): the standalone five-file fixture
passed author/root/independent source reviews, six native Node tests, two Python
controller tests and syntax checks. Pinned celld accepted the exact scalar44
candidate without a fallback. Eleven canonical TEXT observations showed cookie10
stable, transactional create11 rolled back to10, committed create11 stable, and
committed drop12 stable; the fixed fixture table was absent at completion. The
controller finished successfully and verified source/copy hashes unchanged.
Evidence: `/private/tmp/foks-schema-cookie-celld-a/qualification.json`; frozen
source manifest/hashes `/private/tmp/foks-schema-cookie-probe-{manifest.txt,sha256.json}`.
This qualifies the fixed query and transaction relations on that pinned engine,
not the connection-affine VerifiedSchema factory, full schema contract, activation,
production ownership or response deadline. Native cookies need not start at10;
only type/range and relational behavior are compared across engines.

### Actual celld schema-cookie capability gate passed

The standalone fixed-SQL probe passed author, root and independent recovery source
reviews before tests. Six Node tests, two Python tests and syntax checks passed.
On pinned celld 0.6.2 with local Moto and unchanged runtime limits,
`SELECT CAST(schema_version AS TEXT) FROM pragma_schema_version` returned canonical
decimal TEXT in all eleven phases: stable10; uncommitted create11 then rollback10;
committed create11 stable; committed drop12 stable and table absent. Source/copy
hashes stayed unchanged. No diagnostic fallback was needed. This clears the
conditional scalar44 query capability only, not a schema guard, cross-host owner
proof or recovery guarantee. Evidence: `/private/tmp/foks-schema-cookie-celld-a/qualification.json`;
frozen five-file manifest/hashes: `/private/tmp/foks-schema-cookie-probe-{manifest.txt,sha256.json}`.

### L1 native List oracle helper — root implementation subplan

Add a qualification-only `hosted_personal::native_list(certificate, reader,
clock, argument)` next to native_usage. It parses/authenticates the actual peer
through the unchanged native Principal path, then invokes a celld-proof-only
service helper. The service helper requires an ordinary credential, opens one
native snapshot, resolves authority from the original public argument, and calls
the existing private List implementation unchanged. Return its exact bytes or
existing RpcStatus; do not duplicate parsing, permissions, pagination, optional
body handling, status precedence or encoding. This helper installs no route and
does not prove TLS possession, a hosted schema contract or ownership. Its reader
lease ends at function return. Certificate authentication failures remain the
existing outer QualificationError, matching the Usage oracle convention.

Add one feature-gated testkit integration test using a real Rust-created account,
a small encrypted file and an authenticated TLS socket. Extract the exact public
argument from each encoded List call; compare the helper result, framed using
the existing success/status encoder, against the actual socket response with the
same sequence. Cover count0/1/4096/4097, load-small true/false, initial/MAC/Time
cursors (including i64::MAX+1 and u64::MAX), missing parent and malformed pagination.
Explicit assertions check expected outcomes so two identical accidental successes
cannot validate each other. This narrowly qualifies the oracle itself; the full
4096-row, mutation, malformed stored-body and Go-originated corpus belongs to the
shared List implementation's differential gates. No production List code changes.

Owned source: `crates/foks-server/src/hosted_personal.rs`, a short feature-only
helper in `services/kv.rs`, and a new testkit integration test. Root author plan
review: this reuses the real native route and status/framing semantics rather
than creating an independent interpretation, while the TCP comparison verifies
the qualification entry follows public request behavior. Independent plan review
and then author plus independent source review precede tests and a separate commit.

### L1 activation framing and native factory refinement — review required

The fixed actual-celld cookie probe passed the exact table-valued query, including
its11 canonical-TEXT observations across rollback and committed DDL changes.
Scalar44 can therefore use the reviewed query; no host-PRAGMA fallback is needed.
This qualifies the query permission and transaction relation, not the entire
activation factory. The pure schema-contract implementation is now quota-owned;
this section consumes its reviewed borrowed objects/stream verifier API.

**Activation framing and state.** Propose `activate_read_schema() -> status` as an
internal setup export with no caller input buffer. A future guarded production
module contains one immutable generated schema profile: ordered FOKS objects, private
authority objects, platform objects, hashes/versions and physical representation
identifier. Different profiles require separately qualified artifacts; no request
uploads expected DDL or chooses a weaker policy. Activation gets actual rows only
through page9 in the existing pinned Sql host, and reads scalar44 before and after
inside one transaction. The streaming exact-union verifier holds one previous
key/expected cursor, not all observed SQL. Track cumulative object and UTF-8 SQL
counts against the reviewed1024/2MiB limits; page payload/probe/name bounds apply
before copying. Finish must see a genuinely short page and unchanged cookie.
Validate the dependency singleton through43 as well. No SQL proof is created from
a manifest hash comparison alone.

On entry, invalidate any previous schema state and read-token authority before
allocating or querying. During activation, ordinary request entry is refused.
Success produces a fixed64-byte setup receipt: magic FKSA(4), ABI u16=1, flags
u16=0, total_length u32=64, cookie u32, observed_object_count u32,
observed_SQL_UTF8_bytes u32, composite_schema_hash32, profile_id u32 and reserved
u32=0. All integers little-endian; counts must obey the known limits. This receipt
contains diagnostics for the trusted host, not a transferable authorization token.
The composite hash uses a separately frozen private domain and the exact hashes/
profile described above; it does not change native's FOKS-only contract hash.

The host wraps the activation export in one storage.transactionSync, then awaits
storage.sync under its original activation budget. Do not mark the service ready
or install a fresh token authority until both complete. Any callback, commit,
sync, timeout or receipt-validation failure discards the instance; a guard that
was tentatively constructed by the export cannot survive a failed host commit.
Once ready, no external route exposes activation/setup or the instance directly.
The immutable host storage reference, artifact/profile and owner activation are
sealed at construction; replacing any requires a new instance and setup. Every
read export still checks actual cookie/coordinate/fence/current principal inside
its own transaction. A setup receipt is never a substitute for those checks.

The current default Wasm module remains an explicitly qualified fixture artifact
with raw exports; this plan neither calls it production nor silently breaks its
existing test interface. New production admission remains disabled until guarded
Begin/Next/Finish exist and the exact signatures, composite-hash domain and profile
encoding pass review. Artifact admission uses an explicit function-export allowlist,
not a blacklist. Freeze the exact required memory/output, trusted setup/activation and real
Begin/Next/Finish functions with their signatures. Reject raw run_authority,
run_personal, run_page, run_directory_key, generic run, trap/testing selectors and
fixture token issuer/advance exports even if authenticated fixture code could
reach them. A module with extra unknown function exports refuses deployment.
Qualification artifacts have a separate clearly marked harness policy and are
never silently accepted by this production constructor. Default/all-features
artifacts are both inspected; a Cargo feature name does not prove export safety.
The actual production allowlist remains disabled until all listed real paths
perform their SQL proof; this plan creates no transitional bypass.

**Native qualifier factory without a dependency cycle.** Metadata-core's native
feature may own `QualifiedReadConnection` with private fields: the actual
rusqlite::Connection, verified composite metadata/cookie, current ephemeral token
authority and activation state. Its admission constructor takes ownership of a
connection plus a typed selector for a compiled-in immutable profile; there is
no public constructor from a supplied hash/cookie or a freely constructed profile.
It verifies its actual connection using the shared pure checker in a transaction,
commits successfully and only then becomes usable. Its named begin/next/finish
methods start transactions on that connection and privately create NativeSql plus
the matching borrowed proof. They call the common kernel, recheck the cookie
before commit, and return the result only after commit succeeds. All failure
paths roll back and retain no publishable result. The raw connection and proof
are never exposed independently; no arbitrary public callback can move a proof
to another connection. The clock-floor write requires writable ownership, as
already noted; this factory is not a parallel read-pool optimization.

This qualifier can be implemented without a normal metadata-core/server-db edge.
Do not force the production server to open a second uncoordinated writer just to
reuse it. The later server integration must instead bridge its existing writer
actor through one opaque `VerifiedTransaction<'_>` lent by Database, combining
the actual writable Transaction and cached per-handle FOKS proof; successful
callback values remain withheld until commit. The server layer already depends
on both DB and core and is the appropriate bridge owner. It must additionally
admit the private-authority/platform composite once, and may not pretend the
native FOKS-only guard verifies extension DDL. Root will freeze that exact bridge
trait/API after the common kernel's guarded transaction interface exists; no
production raw-connection-plus-hash constructor is an acceptable interim seam.

Activation tests pin the64-byte receipt independently, exact profile/composite
hashes and export signatures, actual schema enumeration/rejection, missing or
corrupt singleton, independent admission of two valid same-cookie databases but
rejection of guard/token reuse between them, attempted host storage rebind,
DDL during/after activation, callback/commit/sync failures, deadline expiry,
poisoned instance cleanup, and fresh activation losing all old continuations.
Use observed memory high water and declared admission separately; fixed-memory
activation must discard page/lexical buffers before directory decoding. Actual
celld qualification runs on persistent SQL with real schema59 hooks, while the
2.2MB physical-row incompatibility and complete owner/bootstrap qualification
remain independent blockers. Author review completed; root and independent
review of this exact framing/factory proposal are required before code.

### Strict binding decoder qualification

The separable decoder passed author, root and independent storage reviews before
testing. Review fixes made the cursor kind string-only (serde's default enum
deserializer also accepts object-form tags), corrected test array construction,
and made JS mutation tests assert that their target substring exists. No fixes
were needed after those reviews. Six focused decoder tests, all64 native metadata
tests and10 actual-Wasm token/decoder host tests passed. Strict native and both
Wasm feature-profile lint/builds and formatting passed; default artifact still
excludes the fixture issuer. Both Wasm artifacts retain the64MiB declared maximum.
Golden binding/token bytes are unchanged. This is structural private binding
validation only; SQL authorization/ownership still belongs to Begin/Next/Finish.
Evidence: `/private/tmp/foks-binding-decoder-qualification.json`; frozen five-file
manifest/hashes: `/private/tmp/foks-binding-decoder-{manifest.txt,sha256.json}`.

Page9 source-review correction: malformed non-text names, including NULL and BLOB
names resembling the reserved prefix, are explicitly retained by the filter so
the typed probes can reject them. SQL three-valued NULL comparison must never
turn an invalid schema object into apparent EOF. Regression fixtures use
writable_schema only to represent these otherwise impossible ordinary-DDL rows.

### L1a native enumeration hardening amendment

During independent page enumeration review, the native query's `sql IS NOT NULL`
and SQL NULL propagation through the name predicate exposed an admission hole
for malformed schema rows. Remove the SQL-NULL exclusion and select rows using
`typeof(name) != 'text' OR substr(name,1,7) != 'sqlite_' COLLATE NOCASE`. Valid
SQLite autoindexes already have the excluded literal reserved prefix, so this
does not change the qualified contract or its hash. A nontext/NULL name or
nontext/NULL SQL on any selected object now reaches the existing budgeted String
conversion and rejects admission. Preserve valid unrelated native extensions.
The pure comparator API and private connection proof remain unchanged.

Native regression fixtures alter otherwise-valid schema objects through explicit
`writable_schema` test-only corruption: NULL/BLOB name (including a reserved-looking
BLOB), NULL/BLOB SQL, with explicit typeof assertions to avoid TEXT-affinity
coercion hiding the intended fixture. Each
must reject explicit native verification, while a legitimate UNIQUE autoindex
and normal unrelated extension still admit with the unchanged contract hash.
This small hardening is subject to root and independent plan/source review before
execution; it is not a wider claim that arbitrary external schema rewriting is a
supported database lifecycle operation. The native read admission enumeration is
a separate trusted adapter from the pure content checker.

### Native List oracle qualification passed

The three-file oracle slice passed root author/direct review and independent
quota review before testing. Its real-account authenticated TLS integration test
passed all15 exact byte/status comparisons, including the missing-parent versus
overflowing-Time precedence and malformed pagination. An explicit malformed DER
case retained the outer certificate error. Strict server/testkit all-target lint
with celld-proof and workspace formatting passed. No production List code changed.
Evidence and exact source hashes: `/private/tmp/foks-native-list-oracle-qualification.json`.
The broader hosted List semantic corpus and full production coverage remain open.

SQL43/44/page9 native qualification (2026-10-16): four implementation/test files
passed author/root/independent source reviews. The eight new schema catalog tests
and full72-test metadata suite pass, as do strict native all-target and default
Wasm lint, default release Wasm build with64MiB maximum, and eight existing Node
page-codec tests consuming26 freshly exported native page oracles. The only
test correction advanced the old unknown-page-ID assertion9→10 after assigning9;
existing IDs1..8 are unchanged. Full schema enumeration observes311 objects and
actual production SQL VM work9986 steps for the first page and5700 after the last
tuple: bounded activation scanning, not constant-work or latency evidence.
The new twelve-column row maximum is66,160 bytes, yielding15 rows under1MiB.

Evidence: `/private/tmp/foks-schema-catalog-qualification.json`; source manifest/
hashes `/private/tmp/foks-schema-catalog-{manifest.txt,sha256.json}`; immutable
default artifact `/private/tmp/foks-schema-catalog-default.wasm` SHA256
`048c0bff6506e7c4d1f4d4a9b2ecc7a41d3752a3a7a43047b5a4b0d2fc460c93`.
Scalar44 separately passed actual celld's authorizer/transaction probe. Actual
page9 celld enumeration must still use an explicitly pinned platform/FOKS/authority
union fixture; generic preexisting page tests do not qualify that union. No
VerifiedSchema factory, activation export or ordinary List route is enabled.

### Read starvation and single-transaction alternative — assessment, not a design change

The global dependency coordinate can invalidate a List after any covered tenant
write, including an unrelated user's write in the same hosted database. A4096-row
result can require274 private15-row calls. A deterministic test that one mutation
correctly rejects a bundle establishes safety, not eventual successful reads under
sustained organization traffic. Repeated restarts may consume the entire original
15-second budget without completing. Qualification must include sustained writers,
read success/error rates, retries and complete latency distributions; record
starvation and retain the unchanged success target rather than relaxing it.

A separately reviewed single-transaction List may be feasible and reuses these
foundations: resolve/authorize once, fully decode/drop the parent, then fetch one
bounded SQL page at a time and append the public response inside one kernel call.
Existing ListEncoder capacities total at most32MiB; finalization temporarily adds
at most16MiB, plus one~1MiB page, transient decoded/schema values and runtime state.
The parent's measured52.5MiB phase occurs before assembly and its allocations drop,
but Wasm linear memory never shrinks and allocator fragmentation/reallocation
peaks can still exceed64MiB. These arithmetic bounds are a candidate, not measured
combined feasibility. The final JS output copy/Response ownership and64MiB V8
budget must be measured separately. Native16MiB directory cases cannot qualify
on current celld physical rows without the already identified representation fix.

A single transaction replaces cross-call invalidation with one SQLite snapshot and
274 local SQL imports, reducing frontend round trips while holding the tenant's
transaction/write serialization for longer. Prove currently writes its clock floor,
so it is not automatically a concurrent read-only transaction. Native/Wasm shared
MAX_SQL_CALLS=2048 is another concrete gate:4096 uncached optional-body read-key
checks can alone exceed it. Transaction-local caching by exact role can remove
repeated generation queries, but a bound on distinct role decisions must come from
real supported semantics rather than an invented hosted cap. Any revised query
budget or key-inventory query needs its own cost and deadline review.

Before selecting this alternative, prototype actual64MiB Wasm assembly at the public
frame boundary, fragmented allocation histories, repeated requests, maximum counts,
optional-body role diversity and malformed data. Compare single-transaction and
multi-call paths under the same sustained-write load and original deadline. Measure
write tail latency as well as read completion, SQL/import count, physical retained
memory and output copies. Neither approach can claim a hard deadline while native
SQLite work/sync remains uninterruptible. No current List/token/assembly source is
changed by this assessment; full production rollout waits for the measured choice.

### Bounded role-generation batches — root proposal before implementation

The single-transaction List feasibility assessment exposed a hard query-count
problem: 4,096 distinct member-visibility roles can exceed the existing 2,048 SQL
call limit. A role cache does not solve that worst case. Add one private bounded
catalog query and a typed decoder before changing List orchestration. This helper
only reads current generations; it neither authenticates nor grants access.

Reserve page ID10 `RoleGenerations`. Its sixteen parameters are namespace BLOB33,
active count1..7, and seven (role type, visibility) integer pairs. A typed helper
accepts a user namespace and1..7 non-NONE Roles, pads inactive slots with(0,0),
and validates the namespace entity, length, count and full role representation.
The statement uses seven fixed VALUES rows with ordinal0..6, selects only ordinal
less than the active count, and left-joins shared_keys by rowid selected from a
correlated `(uid,role_type,visibility)` latest-generation lookup. Existing schema
uses a rowid table and a covering primary-key index for that lookup; rowid lookup
then selects exactly that stored record. There is no role/history table scan.
Use CAST for each incoming integer and outgoing decimal integer; no JS Number
round trip. Keep duplicate requested roles and original order observable.

Each result has ordinal, presence(0/1), generation typeof text and a guarded
integer value. Missing row yields presence0/type'null'/value0; present integer
retains its exact signed value; present non-integer is visible and rejected by the
typed decoder. This distinguishes a missing row from a malformed NULL generation.
The descriptor caps7 rows and512 encoded bytes, with no caller LIMIT. The decoder
requires exactly the requested count and sequential ordinals, rejects malformed
presence/type combinations, and returns a fixed-size array plus active length
(or a bounded Vec charged before allocation). Nonnegative integers including0
map to Some(u64), matching existing current_key_generation; negative values are
Corrupt. Missing maps to None. No normalizer repairs invalid data.

Plan review requirements: independent review of rowid portability, exact scalar
semantics, descriptor/host assumptions, and typed namespace binding. Implementation
review twice before tests. Tests compare each result with the unchanged scalar
helper for present/missing/duplicate roles, all count boundaries, i16 visibility
extremes, generation0 and i64::MAX. Malformed fixtures cover NULL, text, negative
and oversized generation data without copying the invalid payload. Fake host
responses cover omitted/extra/reordered rows and invalid marker combinations.
Production SQL VM tests vary unrelated users/roles and selected-role history;
report measured query work rather than claiming elapsed latency. Native metadata
suite, strict lint and actual Wasm page-codec vectors must pass. Actual celld
execution of the new statement remains required before the List integration gate.

Author plan review: preserve0 semantics rather than imposing a new restriction;
use row identity to distinguish NULL from absence; keep seven slots within the
existing16-parameter ABI; no cache or organization-member-count assumption.

### L1 exact compiled profile and guarded artifact contract — proposed freeze

This section freezes the proposed private encoding and signatures for review. It
adds no serving route. The existing default metadata Wasm remains a qualification
artifact; its raw exports are not accepted by the guarded constructor described
here. L1 can qualify schema admission in that harness before the guarded List
module exists. Its result must continue to say `qualification_only`, and the
production constructor remains unavailable rather than accepting a temporary
weaker export set.

**Compiled profile.** Introduce an immutable `ReadSchemaProfile` whose constructor
is private to a generated, reviewed profile module. No network input, environment
variable, observed database, receipt, or general public `Contract` constructor can
create one. Generation consumes pinned source DDL, not a caller's database. A
profile contains three independently ordered contracts (FOKS, private authority,
engine), the exact sorted union, and these fixed-width canonical bytes. All
integers are unsigned little-endian; all digests are exactly32 bytes:

1. magic `FKSP`(4), encoding_version u16=1, reserved u16=0;
2. profile_id u32, FOKS schema version u32, physical_representation u32,
   schema_checker_version u32=1;
3. activation_abi u32=1, guarded_read_abi u32=1, page_abi u32=1,
   private_binding_version u32=2, private_token_version u32=1;
4. FOKS contract hash32, authority contract hash32, engine contract hash32,
   sorted-union contract hash32;
5. scalar_catalog_digest32, page_catalog_digest32, guarded_abi_digest32;
6. schema_object_limit u32=1024, schema_SQL_UTF8_limit u32=2097152,
   schema_SQL_field_limit u32=65536, schema_name_UTF8_limit u32=256,
   page_byte_limit u32=1048576, page_row_limit u32=64,
   Wasm_memory_page_limit u32=1024.

This is a fixed296-byte record:44 bytes before the seven hashes,224 digest bytes,
and28 final limit bytes. The individual field list, not an implicit Rust struct
layout, is authoritative. The independent golden encoder must assert that length.
No padding, terminator, optional field, unknown enum, or native-size integer is
allowed. Any later added field requires a new encoding version and composite hash.

Each component hash uses the unchanged shared schema-contract algorithm/domain;
empty engine objects on a native qualifier use that algorithm's empty-contract
hash, never an all-zero sentinel. The union rejects every duplicate `(type,name)`
between components even when SQL happens to match, then sorts by the same bytewise
key used by the verifier. Native-required admission and hosted exact-union
admission remain distinct policies; this profile always specifies exact union.
The native-only FOKS hash remains unchanged and is never relabeled a composite.

Define `composite_schema_hash = SHA256("foks-hosted-read-schema-profile-v1\\0" ||
profile_bytes)` where the final domain byte is one NUL, not the two characters
backslash and zero. This new hash occupies the future guarded activation's
`schema_hash`; it cannot be used with the previous fixture's FOKS-only schema hash
without a separately activated binding/version change. The profile binds schema
and interpreter contracts, not the particular live tenant or its data revision.
Installation/HostID, tenant, owner/epoch, key generation/digest, storage identity,
actual artifact digest and current dependency coordinate retain their separate
runtime checks. A valid schema alone proves none of those values or key recovery.

Catalog digests pin the exact immutable catalog artifacts already consumed by the
host: SHA256 of the exact UTF-8 bytes of scalar `catalog.json` and
`page-catalog.json`, respectively. They are generated once from the compiled Rust
catalog, copied byte-for-byte into the deployment bundle, and checked before any
SQL. Do not parse/reserialize them when hashing. The artifact generator must
prove each descriptor equals the compiled catalog, including SQL, IDs, kinds,
nullability, byte and row limits, parameter ranges and limit-parameter position.
A semantically identical but byte-different JSON artifact is deliberately a new
profile; the database DDL comparator's lexical compatibility remains unchanged.

The module's own bytecode digest is **not embedded in its profile**, avoiding a
self-hash cycle. The sealed host computes SHA256 over the exact accepted Wasm
bytes before instantiation and compares it with the controller-approved artifact
record and the runtime authority activation's `kernel_hash`. Neither a frontend
supplied hash nor a successful schema receipt substitutes for that comparison.
The deployer also pins the host adapter/bundle bytes; their trust is not proved by
SQL schema inspection. The private authority schema currently provides fixture
semantics; complete controller/bootstrap and frontend-capability admission remain
B prerequisites before a service can advertise production readiness.

Reserve profile IDs0x80000001 for native schema59 qualification and0x80000002 for
celld0.6.2 schema59 qualification, with physical_representation=1 meaning the
unchanged native table layout. The high bit marks a qualification-only profile
and the production constructor rejects it. These IDs do not silently impose a
new public directory cap: the celld profile still fails the known native16MiB
payload cases at the platform's2.2MB physical row/value limit. No production
profile ID is assigned until the supported physical representation/range and
bootstrap prerequisites are reviewed. That decision requires the full row
inventory, including large receipts and non-directory metadata.

**Exact guarded ABI proposal.** `guarded_abi_digest` hashes a canonical binary
manifest under `foks-hosted-read-wasm-abi-v1` plus one NUL. Encode a u16 count of
imports followed by each import sorted by `(module,name)`, then a u16 count of
function exports sorted by name. A name is u16 UTF-8 byte length plus bytes. Each
function signature is u8 parameter_count, parameter type bytes, u8 result_count,
result type bytes; type bytes are Wasm's0x7f(i32) and0x7e(i64). Finish with u8=1
for one defined unshared32-bit memory, u32 maximum_pages=1024, and the fixed memory
export name `memory` encoded as above. Reject duplicate names, extra imports,
extra function exports, shared/imported/multiple memory, start functions and
exported tables. Permit only the two compiler diagnostic globals `__data_end`
and `__heap_base`, when present, each immutable i32; no other exported globals.
The manifest records those two as a fixed allowed-global section (u8 count=2,
name/type/mutability tuples) after the memory section. Actual minimum memory may
vary but must not exceed1024; actual maximum must be present and exactly1024.

Required imports are exactly `foks.sql: (i32,i32,i32,i32)->i32` and
`foks.sql_page: (i32,i32,i32)->i32`. The guarded read module has no `foks.domain`,
WASI, entropy, clock or arbitrary SQL import. The sealed host supplies trusted
clock values at entry and checks the original monotonic budget at every import;
a frontend cannot call the instance or choose those values. This still does not
make a synchronous SQL operation interruptible.

Required function exports are exactly:

| Name | Wasm signature | Meaning |
| --- | --- | --- |
| `activate_read_schema` | `()->i32` | No caller input; stream actual schema through the pinned host transaction. |
| `read_token_clear` | `()->()` | Invalidate ephemeral authority and clear explicitly owned request buffers. |
| `read_token_setup_alloc` | `(i32)->i32` | Accept exactly48 host entropy bytes only after schema admission. |
| `read_token_setup` | `()->i32` | Consume/wipe entropy and install fresh authority after host commit+sync. |
| `input_alloc` | `(i32)->i32` | Bounded FKLI input allocation; clear old input/output on rejection. |
| `response_alloc` | `(i32)->i32` | Bounded scalar-import response allocation, never a request dispatcher. |
| `page_response_alloc` | `(i32)->i32` | Bounded binary-page import response allocation. |
| `output_pointer` | `()->i32` | Pointer to the current validated output buffer. |
| `output_length` | `()->i32` | Exact bounded output length. |
| `list_begin` | `(i64)->i32` | Fresh trusted now_ms as signed nonnegative exact i64; consume FKLI Begin. |
| `list_next` | `(i64)->i32` | Fresh trusted now_ms; consume FKLI Next. |
| `list_finish` | `(i64)->i32` | Fresh trusted now_ms; consume FKLI Finish. |

The import helper exports remain host-private, and allocation/setup/dispatch state
must reject reentry. An output getter never publishes by itself: the host copies
only after a successful status, validates the framing, commits the transaction,
awaits required durability and checks the original deadline before releasing the
result. Activation uses the fixed64-byte FKSA receipt already specified. List
uses FKLI/FKLB and the reviewed typed-error codec; status0 means a complete framed
result, nonzero means no success result. Error publication still follows current
proof requirements and never converts infrastructure errors into durable domain
rejections. Setup failure, malformed receipt, import/export trap, commit/sync
failure or expiry discards the instance and explicitly owned buffers.

`list_*` exists only in the eventual guarded entry module with current proof and
SQL-derived parent-key issuance; no placeholder forwards to an unguarded fixture
selector. `activate_read_schema` can first be qualified through a clearly separate
fixture policy, but that policy is not the production constructor's allowlist.
The default core currently exports its qualification module for all wasm32
builds. A reviewed entry-module split must preserve those existing defaults while
allowing a separate guarded module to link shared kernel code without dragging
in raw exports. Feature unification/all-features must either fail compilation or
produce an artifact rejected by the exact constructor; a feature flag is not the
security boundary. Raw `run`, `run_authority`, `run_personal`, `read_personal`,
`run_page`, `run_directory_key`, `trap`, decoder probes and token issue/advance
selectors are not in this allowlist.

**Implementation groups and verification.** First implement only the pure profile
encoding/hash and immutable generated-profile plumbing, with native/independent
byte goldens, duplicate-union rejection, catalog-byte mismatch and ABI manifest
mutation tests. Root/storage review the source before tests. Next implement the
qualification activation adapter: shared streaming schema verification over9,
coordinate43, cookie44 before/after, cumulative bounds and native connection-owned
factory. No token key or request dispatcher is enabled by that group. Actual
celld tests must prove the exact FOKS+authority+pinned-engine union and fail for
missing/altered/extra/malformed objects, schema changes, profile/artifact mismatch,
transaction/sync faults, same-cookie cross-handle reuse and budget exhaustion.
Only a later separately reviewed group can connect guarded List entrypoints,
post-commit+sync token setup and the production export admission constructor.

The entry-module split, concrete allocation allowance and actor borrowing API are
source-review gates, not reasons to use a supplied hash as proof. This design is
compatible with investigating single-transaction List assembly: schema activation
and connection affinity remain necessary; changing the eventual request ABI or
removing multi-call exports would require a new reviewed ABI manifest/profile.
Author review completed for this proposal; parent and independent review are
required before implementing the profile or activation groups.

### L1 qualification profile generation and ownership

The dev-only generator is an explicitly invoked native metadata-core test/tool;
it has the existing dev-dependency on server-db, never a normal dependency. It
creates a new empty temporary `Database` through the reviewed native initializer,
then enumerates only that newly created database's non-internal schema using the
hardened literal-prefix predicate. It accepts no existing database path, snapshot,
source-directory override, or caller-supplied expected schema. Data, entropy,
schema cookie and dependency incarnation from this throwaway database are not
included in the expected contract. The version must equal the pinned59 and its
FOKS contract hash must equal the independently captured schema59 golden.

A separate new in-memory SQLite connection executes only the checked-in
`authority::SCHEMA`; it never selects an authority schema from the live target.
The celld engine objects come only from the already reviewed
`tools/foks-metadata-spike/celld-directory-schema.json`, with the exact pinned ten
objects. Native's engine list is empty. Sort each component by the checker key,
reject duplicate keys and invalid/malformed objects, compute each contract and
union hash through the shared pure checker, and reject collisions across the
three components. No extension observed in the generated FOKS database can be
silently assigned to a different component.

Checked-in assets live under `crates/foks-metadata-core/src/read_schema/profiles/`:
`foks-59.json`, `authority-v1.json`, `celld-0.6.2.json`, `scalar-catalog.json`,
`page-catalog.json`, `manifest.json`, and generated `objects.rs`. The schema JSON files
are bounded, deterministic ordered arrays/records with a final newline. Catalog
files retain the existing exact host-consumed encodings: scalar Spec array from
serde_json::to_vec without a newline, and page::catalog_artifact() including its
descriptor/fingerprint wrapper, also without a newline. No catalog is reformatted. `objects.rs` contains borrowed static
`SchemaObject` values with exact escaped SQL strings from those component arrays;
Rust string escaping is the only representation conversion. A regeneration test
requires byte-for-byte equality of all outputs, plus decoded JSON/borrowed-static
object equality. It writes only to a freshly requested output directory for
review; it does not overwrite its own source assets during ordinary tests.

The manifest includes generator encoding version, schema version, component
object counts/hashes, exact byte hashes of the other assets (not its own bytes),
profile bytes/composite digests and pinned source-input hashes (the authority SQL and engine JSON). It has no timestamp,
absolute build path, random data, observed target cookie, tenant or environment
identifier. Scalar/page catalog bytes are generated from the compiled catalog
specifications in the same process; native tests assert all SQL and descriptor
fields match. Final assets wait until the independently reviewed page10 role
batch is committed so no profile is frozen against a half-updated catalog.

Runtime expected DDL uses the compiled static strings, not JSON deserialization
or a fresh reference database. Creating the component/union `Contract` vectors
requires checked admission for the exact vector capacities, descriptor sizes and
fixed verifier state before allocation; every `Cow` borrows immutable static SQL,
so no per-object SQL copy or decoded JSON allocation is hidden in activation.
Schema observation still has its separately charged one-page workspace, bounded
previous-key storage, and cumulative1024-object/2MiB SQL limits. The allocation
API/constant calculation is reviewed with implementation; static module bytes,
linear-memory resident data and host copies are reported separately from dynamic
request admission.

Ownership: recovery agent owns `read_schema/**`, the one `lib.rs` declaration,
metadata-core's normal dependency on the pure schema-contract crate and its lock
edge, and generator/profile tests. Root owns page10/current-generation work;
storage reviews the profile and generation contracts. No normal dependency on
server-db is introduced. The pure profile group exposes only immutable compiled
profile selection/diagnostics; it provides no public constructor from arbitrary
DDL or hashes, no `VerifiedSchema`, no SQL connection proof and no enabled
production artifact. Native/host activation and transaction affinity are later
reviewed groups. Root has approved this source provenance; independent review of
this exact generation/bounds refinement remains required before code.

### L1 page9 actual-celld enumeration fixture — proposed narrow gate

This next component qualifies the committed scalar43/44 and page9 transports on
celld0.6.2 with the exact FOKS59/private-authority/pinned-platform union. It does
not implement `activate_read_schema`, the compiled profile, a VerifiedSchema
factory, a token, or a public List route. Recovery owns those production/private
factory decisions. The current default qualification Wasm with raw `run_page`
remains clearly a fixture artifact; no role10 feature is needed. The scalar
catalog may contain later reviewed entries without this fixture invoking them.

**Independent artifacts, not live-schema adoption.** Add a native-only integration
fixture exporter `crates/foks-metadata-core/tests/schema_enumeration.rs`. It creates
an empty genuine Database through the existing test helper, adds the committed
private authority SCHEMA, and creates the ten exact platform definitions from
`tools/foks-metadata-spike/celld-directory-schema.json` in that disposable native
fixture. It captures `(type,name,tbl_name,sql)` with the corrected literal-prefix
policy from those trusted definitions, before target deployment. The native
FOKS content hash remains the pinned L1a hash. Export the FOKS+authority seeding
DDL separately from the platform objects; the actual cell already owns its
platform tables and the fixture must never recreate or drop them. Emit the
single read-dependency row needed for43, not arbitrary account/domain data.

Exported immutable artifacts bind schema59, exact scalar/page catalog bytes, the
platform JSON hash, the exact expected union, and the fixed scenario list. There
is no constructor accepting expected rows from the target. Before seeding, read
the actual fresh cell's complete platform schema and require exact equality with
the pinned ten-object platform artifact. After seeding, every baseline object
must match the native artifact. Any unexpected platform object is a failure, not
a newly learned profile. The fixture has no generic SQL endpoint.

**Exercise the existing binary page boundary without another page codec.** The
native exporter uses `schema_objects_parameters`, `encode_parameters`, and
NativeSql::query_page to produce exact FKPG request/reply bytes for each fixed
page. Each exported page also records its decoded row count, validated final
(type,name) tuple, and whether a wrong-type/overlong field prevents continuation.
The exporter checks this metadata against PageBuffer::rows, not ad hoc byte offsets.
The fixture worker calls the unchanged Kernel.runPage with those precomputed
requests and compares each complete returned byte buffer against its native
oracle. It uses existing decodeParameters to check that an initial request has
empty cursor fields, and each continuation exactly matches the preceding
successfully compared page's final tuple. This qualifies a fixed test script,
not the future runtime activation cursor implementation. The fixture compares
exact stored object bytes; it does not execute the new pure schema checker inside
celld or qualify its future activation binding. That is a separate factory gate.
No JavaScript SQL lexer, new Wasm export, or second production FKPG result decoder
is introduced.

The effective limit is derived from the frozen PageSpec: currently15, not its
nominal64. A full enumeration must end on a genuinely short SQL page; an exact
multiple requires one final empty page. Accumulate row counts and UTF-8 SQL bytes
against1024 objects/2MiB; bound page calls to1025 for limit1. One continuous
transaction encloses each enumeration, with the exact scalar44 query before and
after. Require unchanged cookie and unchanged scalar43 coordinate; compare the
singletons' types/canonical decimal ranges, not SQLite-specific initial cookies.
The run has one cooperative15-second execution budget carried across all pages,
not a fresh budget per page. Report elapsed time separately; this small fixture
neither proves a production deadline nor permits raising it on a miss.

**An independent full observation remains a test assertion.** In the same
transaction, a fixed `SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY
 type COLLATE BINARY,name COLLATE BINARY` observes the fixture's entire schema.
Apply only the literal ASCII-insensitive sqlite_ internal-name exclusion in the
assertion code; nontext names are not exclusions, and NULL SQL is never silently
ignored. Bounds are checked before retaining assertion rows. Compare every typed
cell and the complete ordered set with the native full-snapshot oracle for that
fixed scenario, separately from page equality. The live snapshot is never used
to choose expected DDL or approve its own contents. These full test snapshots
are outside the eventual streaming production memory claim and reported as such.
UTF-8 byte ordering is authoritative; do not use localeCompare or JavaScript's
UTF-16 ordering for supplementary/BOM names.

**Scenarios.** Fix the scenario names and mutations in the native exporter and
worker module, not request arguments. Cover the full unmodified union at limits1
and15; derived suffixes of0,1,15,16 objects; exact-boundary and one-over256-byte
names and65536-byte SQL definitions; multibyte/BOM names and literal SQL bytes;
and a visible sqliteX-prefixed extension. Fixed safe mutations exercise missing
`kv_dirent_current_name`, changed `kv_current_head_insert`, missing
`metadata_terminal`, and changed `metadata_intent_reserve`. These intentionally
fail equality with the original expected union while their full page bytes must
still match the independently mutated native oracle. Do schema mutation only
inside a transaction that ends with a private sentinel rollback; afterward prove
the original full union, cookie and coordinate are restored. No mutation touches
celld-owned platform tables/triggers. Missing/changed/extra platform expectations
are fixed negative *artifact* variants compared against actual unchanged platform
rows; report that distinction, not a claim of live platform-DDL fault injection.

NULL/BLOB writable_schema cases remain native regression coverage because the
celld authorizer does not grant arbitrary schema surgery. The actual fixture
must report any unsupported fixed DDL as a failed case, not silently skip it or
change the authorizer. For oversized public names/SQL, assert the descriptor's
actual type/length plus suppressed NULL payload, and stop without inventing a
continuation from invalid text. Baseline and negative-artifact equality outcomes
are explicit assertions: identical wrong successes cannot pass the gate.

**Transport, ownership and files.** Add a mutually exclusive `--schema-fixtures`
mode to existing `run-local.py`, with `schema-enumeration.mjs`, a tiny worker entry,
Python preparation/verification module and focused Node/Python tests. Keep the
existing directory mode unchanged; do not reuse its old LIKE/NULL-filter snapshot
helper. The schema mode uses only empty POST bodies, a bounded immutable case
index, a random private capability checked before cell acquisition/SQL, one fresh
cell per scenario, and the existing local dummy credentials/random loopback ports.
Wrong capability must prove no SQL/cell work. No HTTP-supplied SQL, expected
manifest, cursor, cookie, deadline or profile selector is accepted.

Before deployment, bound the immutable fixture manifest to8MiB, require exact
known keys/case names/counts, bound each command/reply to the existing ABI limits,
verify source/copy hashes, and pin the exact platform ten-object hash. Native
oracle pages are fixture material only; their aggregate/static memory is reported
separately from one-page host/Wasm working measurements. HTTP result bodies remain
64KiB and contain bounded counts/hashes/outcomes, not the schema text or payloads.
The controller owns and terminates all children, preserves deployment and every
case failure, checks artifact hashes after execution, and exits nonzero on any
case/timeout/cleanup verification failure. Use the existing64MiB heap and unchanged
SQLite limits; no emulator or heap tuning to make a failing case pass.

Author plan review: trusted expected DDL precedes target setup; native encoded
pages and independent whole-schema observations check different failure modes;
all schema mutation is fixed and rolled back; platform expectations are pinned
without mutating runtime internals; no factory/capability can be mistaken for
production activation. Proposed ownership is the new native test/exporter plus
schema fixture modules/tests and narrow launcher branch only. No core runtime,
page catalog, pure checker, profile factory, role10, or SQL schema edits. Root and
recovery/storage independent plan approval precede implementation in an isolated
worktree at the then-current committed HEAD. Author and independent source reviews
precede native/Node/Python and actual-celld tests; keep all Markdown unstaged.

Page9 fixture review precisions (root and recovery PASS): scalar43/44 checks use
the exact immutable catalog SQL through fixed fixture assertions, not a newly
claimed scalar-Wasm ABI path; no generic scalar export is added. Only the exact
private sentinel object authorizes the deliberate rollback outcome. Unexpected
DDL, query, commit or sync errors fail the case and cannot become an expected
union mismatch. The isolated implementation base is6984a043; later catalog additions
remain independently owned and require an exact-artifact integration check.

### Role-generation batch qualification passed

Committed as `e47fbfd6` after root author/direct and storage-agent independent
plan/source reviews. All77 native metadata tests, strict native/default-Wasm lint,
release Wasm build capped at64MiB, eight Node actual-Wasm page-codec tests, and all
34 actual celld page vectors passed. The eight added role vectors preserve exact
large integer values, visibility extremes, duplicate order and missing namespaces.
Native malformed-data tests retain NULL/type probes without copying huge values.
The production SQL used386 VM steps at1,100 and10000 selected/unrelated historical
generations. This is a bounded query primitive, not a route or latency claim.

Frozen source/evidence: `/private/tmp/foks-role-batch-{manifest.txt,sha256.json,qualification.json}`;
actual-engine results: `/private/tmp/foks-role-batch-celld-a/qualification.json`.
The exact default Wasm is `/private/tmp/foks-role-batch-default.wasm`, SHA256
`75eb658dc3c6fd47e173f8d6cc425738bc583fa11833a929d21abdc5ef26d98c`.
The page catalog SHA256 is
`9e19c35cc1de83118c6d502c8b17ca503f564bd13be8dfc8156c3533ce6a4915`.
Local child processes were terminated and independently checked absent afterward.
All Markdown remains unstaged.

Profile source review correction: preserve both exact existing catalog artifacts;
adding a newline or emitting bare page specifications would break the pinned host
artifact contract. Generator assertions pin the already qualified role10 catalog
digests. The pure composite test now pins an independently computed Python
struct.pack/hashlib SHA256, not only changed-component inequalities. The first
source group emits only trusted-source artifacts and an intermediate
source-manifest.json; final profile/ABI manifests wait for the single-call ABI
review and generated-profile integration. It creates no deployed profile.

### Pure profile/source generator qualification

Commit7730051f contains the six reviewed source files. On basee47fbfd6, all80
native metadata tests passed (including three new profile/generation tests), with
one explicit source exporter ignored by default and then run successfully. Strict
native all-target and Wasm lint, release Wasm build and owned-source formatting
passed. The immutable default artifact declares18 initial/1024 maximum memory
pages. The only compile correction was an explicit generator BTreeMap key/value
type, independently reviewed before rerun. Catalog byte-format and independent
hash-golden corrections were completed during source review before testing.

`/private/tmp/foks-read-profile-qualification.json` records exact six source hashes,
logs and artifact hashes. `/private/tmp/foks-read-profile-generated-a/` contains
intermediate trusted-source outputs:311 FOKS objects,6 private-authority objects,
10 engine objects,317 native-union and327 celld-union objects. Both catalog hashes
match the qualified role10 artifacts. These are not checked-in serving profiles,
not actual-target schema admission, and not a guarded List implementation. The
upcoming node-type catalog correction requires a deliberately reviewed digest
update/requalification; do not silently regenerate and call the earlier profile
unchanged. Final profile/ABI assets still await the single-call ABI review.

### Connection configuration and name-resolution admission prerequisite

Exact main-schema DDL and a matching schema cookie do not prove connection
settings or SQL name resolution. Before a future activation transaction, the
trusted adapter must establish and query the reviewed settings on that actual
connection: foreign_keys=ON, ignore_check_constraints=OFF, and an explicit
recursive_triggers policy. Native connection.rs currently sets foreign_keys=ON
and trusted_schema=OFF for both writer and reader; recursive_triggers and
ignore_check_constraints are not explicitly set there. Qualification must measure
and pin the intended behavior rather than treating an unspecified default as a
schema guarantee. Set foreign_keys before entering a transaction because changing
it inside a transaction is ineffective. No caller endpoint can toggle settings.

Celld's pinned pragma policy permits foreign_keys, defer_foreign_keys,
ignore_check_constraints and recursive_triggers, but not a user trusted_schema
pragma. The platform's compiled/native equivalent for trusted_schema therefore
needs a separately verified capability/source/behavior gate; lack of access is
not permission to assume parity. Native synchronous/WAL settings and celld's own
commit/sync contract remain separate durability responsibilities. A list-only
fixture does not qualify all mutation constraints.

Admission must also address unqualified catalog names resolving to TEMP objects:
a same-connection TEMP kv_nodes can shadow main.kv_nodes without changing the
main schema cookie or verified DDL. The initial guarded adapter rejects any
non-internal TEMP schema object and any attached database other than the expected
main/temp pair, and seals all later DDL/ATTACH/PRAGMA entry paths. If exact platform
configuration requires exceptions, they need an explicit pinned contract, not a
wildcard. Native database-name and temporary-schema checks run on the owned connection.
For pinned celld, storage.rs denies Attach/Detach and Create/DropTemp operations,
sets SQLITE_LIMIT_ATTACHED=0, and installs its authorizer before user SQL. Its
user pragma policy does not permit database_list. Qualify that pinned equivalent
with actual negative TEMP/ATTACH tests instead of requiring a forbidden diagnostic
pragma or silently skipping the capability. The trusted_schema requirement likewise
permits a documented pinned platform equivalent tested against the exact admitted
DDL; it does not automatically require an engine fork. These capability facts
remain separate from schema hashes. An alternative migration
to fully qualified main table names would change catalog artifacts and needs
separate SQL review; it is not silently substituted here.

Tests must change each relevant setting, add a TEMP shadow or attach a database
while preserving main's cookie/DDL, and prove readiness is invalidated/refused.
Native connection ownership prevents a second raw handle from reusing its proof;
these checks cover state on the same handle. Trusted callbacks that retain DDL or
configuration capabilities must revalidate before commit and publication, or be
removed from the serving surface. This prerequisite is added before any factory
implementation and does not change the already-qualified pure profile component.


### Single-call guarded List ABI and connection lifecycle — concrete proposal

This section replaces the proposed Begin/Next/Finish serving dispatcher, not the
already qualified private token primitives. No continuation token, token entropy,
setup key, token issuer or token advance function belongs to this single-call
route. The pure profile/source generator remains reusable; its final read-ABI
identifier and ABI digest must be deliberately revised before generating a serving
profile. The current default Wasm stays a qualification artifact. This proposal
adds no production flag and does not clear the physical-row/bootstrap blockers.

**Host ownership and input.** One exclusive owner of a Wasm instance also owns its
SQL connection/cell transaction capability. It installs one immutable context for
the duration of an invocation, after authenticating the frontend's per-owner
capability and verifying peer-certificate provenance. There is no RPC parameter
that selects the synthetic resolver or asserts a principal, UID, namespace, role,
parent key, dependency coordinate or schema cookie. The exact original public List
argument is the only public argument. The original request sequence is transport
metadata, not part of the authorization decision.

Use a dedicated binary input with a32-byte header: magic FKLQ (4), ABI version
u16=1, flags u16=0, total length u32, argument length u32, reserved u64=0, and
original RPC sequence u64; all integer fields little-endian. The header is followed
by exactly argument-length bytes, without trailing data. Checked arithmetic and
exact total equality precede parsing. The argument is an exact slice of a native
accepted request frame; its maximum is the existing public frame bound, not the
old2MiB qualification command bound. A request whose argument cannot fit its
original public frame is never admitted. The private allocator accepts at most
32+the public frame bound and reserves before allocation. No blanket normalization
or reserialization changes the public argument before its native-compatible parser.

The trusted context arrives through a separate fixed host import, not the public
input. Freeze a strict bounded context DTO before implementation: exact peer
certificate fields already used by PersonalResolver; exact expected installation,
tenant, host, cell, activation and owner/epoch binding; trusted now_micros as a canonical unsigned-u64 decimal string, checked
now_ms=floor(now_micros/1000) and immutable execution_deadline_ms as canonical
signed-i64 decimal strings; and the original
absolute host operation deadline owned outside Wasm. No current authority revision
is taken from the frontend: the gate obtains it from the same transaction's
owner-checked authority row. No SQL-derived read dependency appears in this DTO.
All nested objects reject unknown/duplicate fields, integer widths are target
independent, and the finite byte/vector limits come from the existing reviewed
certificate/fence profile. The trusted host captures one exact microsecond clock
sample. Credential/certificate expiry uses that original sample, never the rounded
millisecond value multiplied back by1000. Existing PersonalResolver's ms-to-micros
qualification path must therefore be refactored/reused with an explicit exact-time
parameter for this route, not silently called with reduced precision. Authority
clock-floor arithmetic remains in its defined milliseconds, and checked conversion
failure aborts. The execution expiry conversion may round earlier, never later;
its relation to the original absolute monotonic host deadline needs the existing
cross-host deadline contract, not a fabricated universal wall/monotonic mapping.
Clock sampling/conversion errors fail closed before SQL/publication and retain the
native error classification where applicable. Tests cover one microsecond before,
at, and after credential/certificate expiry, millisecond rollover, overflow, clock
failure and rollback; no rounding grants extra credential validity. The proposed64KiB context ceiling requires an allocation
and maximum-valid-field audit before it becomes an implementation constant. If a
currently valid context exceeds it, report a profile limitation or revise the
bound through review; never truncate or silently coerce. TLS possession and owner
capability authentication remain prerequisites outside the shared kernel.

**Proposed exported/imported surface.** The future guarded module exports only
memory; read_input_alloc(u32)->u32; activate_read_schema()->u32;
read_schema_ready()->u32; invalidate_read_schema()->u32; run_list()->u32;
read_output_ptr()->u32; read_output_len()->u32; read_output_release()->u32; and
read_host_response_alloc(kind:u32,length:u32)->u32 for the legal import-result
callbacks described below.
Activation uses no caller input and produces the existing64-byte FKSA diagnostic
receipt only. Ready is a trusted host transition after the activation transaction
COMMIT and durable sync, not an instruction a public frontend can invoke. The
host never treats the diagnostic receipt as a reusable authority capability.
Run_list is permitted only in READY with one fresh immutable context and one open
owned transaction. Its output is the complete existing public RPC frame, with no
private wrapper that would copy another16MiB buffer. Return status0 means a
candidate success frame,1 a candidate known public-error frame,2 infrastructure
failure with no frame. Unknown status is infrastructure failure. Both0 and1 are
**tentative** until the host completes the publication protocol below.

The host import set consists only of the reviewed scalar/page SQL operations,
a bounded immutable-context read, allocation admission and deadline checks. Freeze
exact Wasm function signatures, namespace names and import fingerprints in the ABI
artifact before code; no generic SQL string, control-plane mutation, token-key or
arbitrary network import is allowed. The existing scalar/page transport bindings
remain shared, but raw run/run_authority/run_personal/read_personal/run_page/
run_directory_key and all fixture/issuer/trap exports are forbidden by the guarded
artifact's explicit export allowlist. Reject extra exports even if Cargo feature
unification accidentally links them. Qualification uses a distinct, explicitly
identified artifact and cannot authorize production admission.

**Transaction and error sequence.** After admission/deadline checks, configure the
actual connection before BEGIN, establish the reviewed configuration/name-resolution
capabilities, and enter one writable transaction. Check the connection-affine
schema guard/cookie, current owner capability/epoch/activation and trusted clock
floor. Derive current credential/profile and personal authority from SQL and the
exact authenticated peer. Parse the original public List argument in the same
observable order as native resolution, including its existing status text and
Time-u64 overflow precedence. The existing helper's typed request constructor is
not a substitute for this public parser. Reuse/extract the exact public argument/status boundary and compare it against
hosted_personal::native_list before using it; a new serde DTO or generic
BadArguments message is not parity. Native resolve_authority precedes List field
parsing and currently loads all shared keys. The bounded replacement must preserve
its observable failure precedence, not simply move key validation after parsing.
Parent read-key denial retains the exact text "active device does not authorize
this KV namespace"; optional-body denial omits that body. The normalized domain
helper and full frame codec alone do not establish these properties.
Read schema59 incarnation/revision from that transaction, derive the parent key
through the existing full directory decoder, and execute the shared List helper
with one SQL page at a time. There is no inter-call restart or token dependency.

A dedicated typed owner/schema gate result must precede any publishable public
error. Do not map an authority Permission/Expired/Storage failure to an ordinary
FOKS error merely because it shares an error name with a domain result. Only the
explicit native public-status adapter can construct CandidatePublicError after
the current owner gate. Failed schema/owner/deadline/admission, trap, malformed
private input, unsupported artifact/profile or SQL infrastructure failure aborts
and discards the candidate. Authentication/public-argument failures require an
explicit reviewed ordering relative to that gate, with native differential tests;
there is no generic catch-and-publish path. Known read errors may commit the clock
floor observation, but never convert an uncertain transaction outcome into success.

Encode the raw helper bytes with existing foks-rpc::encode_success_response_at,
or the exact status with encode_status_response_at, using the original sequence.
Check the final cookie/current guard before COMMIT; a callback capable of changing
DDL/settings between check and use is not admitted to the serving surface. All
SQL and host context remain exclusive during this interval. COMMIT, durable sync,
then recheck the original monotonic deadline and current host ownership generation
before releasing output. On failure/uncertain commit/sync or expiry, discard the
frame and retire the instance as appropriate. The host clock-floor durability
contract must preserve irreversible observed expiry; this is not implemented by
merely passing Date.now to Wasm. Sync work may outlive a response deadline; the
physical worker/connection permit stays held until work and cleanup actually end.

A final owner check linearizes the observation; it cannot atomically fence socket
bytes against a later promotion. A response whose observation completed before
promotion may still overlap it in flight. Reads do not create acknowledged writes;
write fencing/outcomes and combined frontend/cell host-loss qualification remain
separate production gates. Single-transaction List also holds a writable clock-floor
transaction, so measure writer delay and sustained-write read completion rather
than claiming a read-only WAL snapshot.

**Buffer and allowance ownership.** The Request remains the parent owner of the
absolute deadline, aggregate SQL-call count and cumulative retained allocations.
Reserve one bounded scratch workspace before work; a per-phase exclusive lease
covers every page parameter, imported encoded page, borrowed view, decoded row,
role batch and temporary decoder/encoder buffer. The exact sizes and all allocation
sites require the separate workspace API review before implementation. Retained
ListEncoder growth and final framing buffers use the original parent admission,
not a resettable scratch counter. Cumulative retained-growth charges are not live
RSS, and reusing scratch does not eliminate that cumulative bound.

Release the full public input allocation after strict parsing has retained only
bounded fields needed by this call; release the context's owned encoding after
validated typed context construction. The parent-directory allocation must be
physically dropped before ListEncoder growth. No Rust RefCell borrow or borrowed
page view spans an import. At each phase boundary, replace/drop owned Vec buffers;
clear() alone retains capacity and cannot justify returning its allowance. In
particular imported PAGE_RESPONSE/RESPONSE and input/output thread-local storage
must not retain uncharged capacities after an error or completed phase. A failed
import or trap poisons the instance; after outermost unwind, wipe explicitly owned
sensitive buffers/current capped memory and drop it, without calling Rust cleanup
on trapped state. A normal typed domain error may reuse an instance only after
all buffers/leases are demonstrably released or retain their original charges.

The current success encoder creates two new buffers while borrowing the raw List
response: content and framed output. Near the limit, raw+content+frame is roughly
48MiB, not32MiB; reserve_success charges2*(length+128) for the new allocations.
Prior parent-directory decoding reached about52.5MiB in the standalone64MiB Wasm
qualification, and freed capacity/fragmentation may still prevent this subsequent
phase. Combined repeated-call peak and cleanup tests are mandatory. The host must
keep output backing memory and its physical admission guard alive through the last
transport write/abandonment; release is host-controlled after that ownership ends.
Any host-side copy has its own bounded allowance and is included in peak evidence.
No extra JSON/hex/private-frame copy of the public result is permitted.

**Admission tests and implementation groups.** First freeze the strict context
codec/import signatures and native public parser/status adapter, then implement
connection-affine activation/configuration checks, then guarded List plus the
reviewed workspace, then persistent host/transport publication. Each group gets
separate author/independent source reviews and focused tests before integration.
No existing qualification endpoint becomes a production endpoint as a side effect.

The native/Wasm exact-frame matrix covers sequences0,127,128,255,256,65535,65536,
u32::MAX,u32::MAX+1,u64::MAX, with payload lengths immediately below/at/above each
actual public frame cutoff. Preserve the existing seq256 precise-limit regression;
16MiB of raw List bytes is not universally a legal16MiB RPC content frame. Compare
success and exact status frames against the existing native socket/oracle, including
malformed args, unsupported/team auth profiles, parent permission before Time
u64::MAX conversion, missing/malformed optional bodies, role generations and maximum
4096-entry results. Unsupported team coverage stays a declared profile boundary.

Lifecycle tests cover activation failure before/after COMMIT and held sync; run
before READY; second simultaneous entry; same-cookie other-connection guard reuse;
changed PRAGMAs/TEMP/attachments without cookie change; pinned celld negative TEMP/
ATTACH attempts; every allocation/SQL/import/frame/sync failure; stale owner before
and after computation; cancellation while blocked; output borrower lifetime; and
repeated large success/error/trap/reinstantiate cycles. Record actual Wasm memory,
host copy peaks, cumulative admission and physical permit occupancy separately.
Measure sustained concurrent writes, read/writer latency and full4096 output under
the unchanged15-second target without hiding retries, failures or frozen-clock
limitations. A failure is evidence to change the design through review, not a
reason to raise runtime limits or truncate public results.

Author review: the proposal keeps original wire framing and no new public crypto,
removes unnecessary single-call continuation authority, separates domain errors
from failed owner admission, and makes buffer retention/commit/publication explicit.
Hard prerequisites still include exact strict-context/import API, workspace charge
inventory, production peer/owner bootstrap, connection/platform capability checks,
physical-row coverage beyond celld's2.2MB limit and synchronous-SQL/deadline behavior.
Independent quota review passed after the lifecycle refinements below; final root
review also passed for the outlined lifecycle. Exact API/allowance subgates still
precede implementation; this draft alone authorizes no ABI code.


Single-call review refinements (root and quota):

* The host-owned nonconstructible PendingFrame binds the instance identity,
  monotonically checked invocation generation, original sequence, transaction and
  deadline. Wasm ptr/len getters are private **tentative host access**, never a
  public response or a publication capability. Only PendingFrame::finish after
  successful COMMIT, sync and final ownership/deadline checks can yield an owned
  PublishableFrame. An abandoned frame cannot be revived by a late sync callback.
  Exhausted invocation generations retire the instance rather than wrap. The host
  lifecycle is READY→EXECUTING→CANDIDATE/SYNCING→OUTPUT_BORROWED→READY; returning
  READY requires host-controlled output release and actual worker cleanup. A
  failed/abandoned generation stays unavailable until its work terminates; uncertain
  transaction or trap retires it. Second run, new input/context, activation and
  unrelated SQL are rejected during every pending/borrowed phase. Wasm itself
  retains a pending-output state so a second export cannot overwrite its buffer.
  Only the host can advance the external durable/publication state. Tests attempt
  stale completion against a later instance/generation and prove refusal.
* A host may inspect tentative bytes to validate framing before COMMIT, but must
  neither send nor externally expose them. No raw Wasm typed-array view survives
  a memory.grow or reentrant call. Once output is borrowed, all growth/import/entry
  paths are frozen through its last use; alternatively make a separately admitted
  bounded host copy while retaining its physical output owner through transmission.
  The latter's additional peak is measured, not hidden by releasing the Wasm view.
* The explicit result allocator closes the export-list omission. kind1 is the
  immutable-context reply (proposed64KiB audited cap), kind2 the existing bounded
  scalar reply, and kind3 one binary page (1MiB cap). During activation only scalar
  and page replies are legal; context is allowed only during the single List
  context import. Each allocation is permitted exactly once while the matching
  synchronous import is outstanding, after its byte reservation and before host
  copy. It cannot reset context, call counters, deadline, authority state or another
  pending buffer. It rejects unrelated kind/length/phase rather than creating an
  unauthenticated general allocator. Existing scalar/page host implementations need
  an explicit adapter to these signatures; they cannot assume their old allocator
  exports remain available. Exact import argument/return signatures and scalar cap
  are frozen in the next API inventory before implementation.
* This result allocation is the one intentional reentrant callback. No outer Rust
  mutable borrow or RefCell guard spans the import; the callback owns separate
  response storage, and the host refreshes memory views after allocation/growth.
  Second top-level execution, recursive queries or context changes are forbidden.
  Normal consumed buffers are moved out and dropped; failed import buffers remain
  charged until poisoned-instance destruction. An allocation trap poisons just
  like an execution trap. Activation must exercise this same page-return path.
* Native resolve_authority currently loads all shared-key history. The bounded
  replacement can preserve valid-data behavior and touched-record status precedence,
  but cannot promise discovering every corruption in unrelated history while doing
  only bounded point/batch reads. Admission must establish the reviewed full role/
  generation invariants once, and every supported mutation must preserve/invalidate
  that admission with the dependency coordinate. If those invariants and mutation
  coverage are absent, record the corruption-equivalence gap as a production
  blocker; do not silently scan all history per List or claim exact malformed-data
  parity from touched-row tests. Native malformed argument/status differential
  coverage remains required independently of this data-validity prerequisite.
* Vec destruction releases allocator blocks, not Wasm linear-memory pages. The
  instance/pool retains a reservation for its full grown memory while it remains
  resident, independently of request scratch/output allowances. Releasing that
  reservation requires instance destruction; an optional high-water eviction policy
  is a separately explicit lifecycle choice. Measure repeated calls and dormancy
  density as well as active-call peak. READY never implies return to initial memory
  size, and allocator reuse is not RSS reclamation.

### Single-call context and import inventory — exact subplan for review

The preceding lifecycle plan has root and quota independent PASS. This inventory
freezes the next proposed private ABI details, not an enabled dispatcher. The root
owns extraction of the original public auth/List parser and exact status boundary;
storage owns the scratch allocation-site audit. Those components remain independent
review gates before wiring this ABI. All integer sizes below are explicit on both
native and wasm32. No JavaScript Number carries a u64 or i64 value.

**Context v1 shape.** The private host serializes one object with exactly these five
fields: version (JSON integer1), peer, fence, now_micros, execution_deadline_ms. The
last two are decimal strings. now_ms is derived by checked floor division of the
exact now_micros by1000; it is not a second asserted input. This supersedes the
earlier provisional two-clock-field wording. The host's monotonic operation budget,
instance/invocation identity and authenticated capability are owned host state, not
serialized frontend claims. There is no request-supplied current authority revision,
read-dependency coordinate, namespace, role, route or parent key in this record.

Peer has exactly exact_der, serial_magnitude, public_key and expires_at_micros:

| Field | Exact accepted representation and bound |
| --- | --- |
| exact_der | JSON byte array,1..4096 bytes, same existing MAX_PEER_CERTIFICATE_BYTES |
| serial_magnitude | JSON byte array,1..20 bytes, no leading zero unless its length is1 |
| public_key | JSON byte array of exactly32 bytes |
| expires_at_micros | Canonical nonnegative decimal string,0..i64::MAX |

These are the existing PeerCertificate structural limits, not proof of TLS
possession or DER parsing. The trusted native certificate parser and authenticated
transport must establish those facts. Invoke the existing constructor after strict
bounded decode; certificate expiry is checked against exact now_micros, never its
rounded millisecond counterpart. now_micros is likewise0..i64::MAX because the SQL
authentication path uses checked signed-i64 time. execution_deadline_ms is positive
and must satisfy0 < deadline_ms-now_ms <=15000 with checked arithmetic; the original
host monotonic budget can expire earlier and always wins. A future cross-host clock
conversion must not extend that budget. Expired or unrepresentable time does not
become a public List status by default.

Fence has exactly identity, activation, owner, epoch. Identity has installation16,
tenant16, host33 and cell32, all exact-length byte arrays. Owner is16 bytes; epoch
is a canonical positive signed-i64 decimal string. Activation has the same ten
fields as the existing authority Activation: version, schema, schema_hash,
kernel_hash, catalog_hash, key_generation, key_digest, storage_namespace,
storage_placement and limits. Version,
schema and key_generation are positive signed-i64 decimal strings. All four hash/
digest fields and storage_placement are32-byte arrays; storage_namespace is16 bytes.
Limits has exactly blob_bytes, dirent_bytes, namespace_bytes, namespace_objects,
directories and dirents. In this new private wire DTO all six are canonical unsigned
decimal strings; blob_bytes and dirent_bytes are checked u32 before converting to
usize, and the other four are u64. This removes the native/wasm usize acceptance
difference without reusing the old Limits JSON format. An activation outside this
qualified target-independent range is unsupported, never clamped. The complete
typed fence is compared with current SQL authority state and the immutable admitted
profile; merely passing its shape checks grants no authority. Profile validation,
not these wire-width bounds, determines permitted operational limits.

Every level denies unknown fields and tracks duplicate fields. Decimal values reject
sign aliases, leading zeros, whitespace, exponent/floating representations and
negative zero. Byte visitors check u8 and stop at the exact maximum before pushing
an extra element. No flattened enum or arbitrary JSON map is used. Fixed arrays
cannot accept short/long sequences, and all fields are mandatory, including zero
values when permitted. The host emits compact unescaped ASCII JSON. The decoder
rejects any backslash or non-ASCII byte before serde parsing: none is needed for
fixed ASCII field names, decimal strings or byte arrays. This avoids hidden owned
string/scratch expansion from JSON escapes. Ordinary JSON structural whitespace is
accepted only within the same total bound; it does not change numeric canonicality.

The conservative maximum compact field-wise representation is18,496 bytes, computed
from the exact
field inventory with every byte255,4096 DER bytes,20 serial bytes, positive i64
maxima, u32 maxima for the two allocation limits and u64 maxima for four counters.
Peer contributes16,685 bytes, fence1,700 bytes, and root framing/time fields111.
The joint15-second deadline relation reduces the largest structurally time-valid
example to18,493 bytes (deadline_ms has at most16 digits for an i64-microsecond
clock); retaining the three extra bytes is an explicit wire ceiling, not extra time.
Pin this independent size calculation as a fixture; the context allocator ceiling
is18,496, replacing the provisional64KiB context ceiling. Native and Wasm reject
one byte above that bound before allocation/copy. 
A bounded context parser reserves its encoded input N, at most4096+20+33=4149
bytes of variable DER/serial/host storage, and two fixed DTO/typed-context layouts
whose size must each be asserted <=2048 bytes on both targets. Thus successful-value storage is N+4149+4096. The bounded256-byte error
allowance detailed below makes the total N+4149+4096+256 (at most26,997), excluding
the separately retained instance stack and allocator overhead. Decimal/field-name visitors borrow ASCII
strings and retain no String; fixed arrays are inline, and the three variable
vectors reserve once at their fixed cap before filling. Conversion moves those
vectors into PeerCertificate/Fence rather than cloning them. The implementation
review must verify serde's unescaped string path and every allocation site against
this inventory; if it allocates additional storage, amend the formula before tests.
Do not present this logical allowance as whole-runtime RSS or the final request
workspace formula. Context input is dropped before parent-directory materialization.

**Exact proposed imports.** All live under module foks_read_v1; Wasm parameters
and results are i32 bit patterns for u32, except admission bytes uses i64/u64 and
must arrive at JavaScript as BigInt. No WASI/domain callback/arbitrary-SQL import:

| Import | Signature | Contract |
| --- | --- | --- |
| context | () -> u32 | Once per List, fills kind1 through the legal callback; never during activation |
| sql | (statement:u32, pointer:u32, length:u32, write:u32) -> u32 | One pinned scalar call; write exactly0/1 and must match the restricted statement policy |
| sql_page | (statement:u32, pointer:u32, length:u32) -> u32 | One pinned binary page call with existing PageSpec validation |
| admit | (bytes:u64) -> u32 | Cumulative parent retained-allocation admission; checked target width, no release/reset |
| checkpoint | () -> u32 | Check original host deadline/cancellation/owner generation without refreshing any budget |

SQL scalar parameters use the existing Value JSON codec but are capped at64KiB
before any host copy, sufficient for the selected4096-byte DER binding plus fixed
parameters. Scalar responses retain the existing64KiB cap and typed row bounds;
fixed canonical integers are never read through JS floating point. Page parameters
retain64KiB, page responses1MiB and the exact FKPG descriptor/fingerprint. Host typed
cell bounds apply before materialization; error/type/NULL probes remain unchanged.
Context/scalar/page response kinds are respectively1/2/3. Their one response callback
is read_host_response_alloc(kind:u32,length:u32)->u32; maximums are18,496/65,536/
1,048,576. Page minimum length is its existing24-byte header. No zero-length success
is meaningful for these three response formats. Invalid size/kind/phase yields0
and a latched invocation failure; it cannot leave an earlier response callable.

Imports return0 only after completing their contract. Nonzero codes are1 bounds,
2 expired,3 cancelled,4 owner invalid,5 host/SQL failure,6 invalid phase. Unknown
codes are host failure. Every nonzero import aborts this invocation and causes
host retirement/discard after outermost unwind; none constructs a public error or
returns an empty successful row. A thrown import/allocator/export also poisons the
instance. The SQL allowlist is additional to the complete immutable catalog:
activation reads scalar43/44 and page9 only; List reads scalar16 (AuthorityState),
33..37 (current certificate/device/anchor/SSO/key checks),43/44, and pages2..8/10.
Its only write is scalar18 (AuthorityClock). No transaction-control or other
Authority mutation ID can be invoked through this module. A parser extraction
needing another read must receive a reviewed allowlist amendment, not a wildcard.
The host owns BEGIN/COMMIT/ROLLBACK/sync and connection settings outside these
imports. The compiled kernel and host independently check phase/write/ID bounds.

`admit` is not a scratch-reset API. The one scratch workspace is reserved by the
host before invocation and its Rust phase leases use the separately reviewed
fixed allowance; no new imports are needed to renew a page budget. SQL-call count
is aggregate across activation or List invocation and cannot be reset by context,
allocator callbacks or workspace reuse. Activation and List have distinct admitted
operations, not a hidden reset partway through either one. Checkpoint and context
calls do not consume SQL-count slots, but are structurally bounded by the guarded
control flow. Before each SQL call and every publication transition the original
host budget is checked. Synchronous SQLite interruption remains separately unproven.

Only the matching response allocator is callable reentrantly while its import is
outstanding. The host records kind and consumed status before calling the export;
a second allocation, recursion, unrelated entry or callback after return is a
phase failure. The kernel sets a matching pending-response state without holding
a RefCell borrow across the import. After allocation the host obtains a fresh
memory view, checks pointer+length in current memory, copies synchronously and
returns; Rust then takes ownership of the complete initialized buffer and decodes.
The host cannot call a second SQL import while delivering a response. Pointer/len
arithmetic is checked and never converts an unchecked JS Number to a memory range.

**Setup versus runtime capability.** Before activate_read_schema, the native host
sets/rechecks the reviewed actual-connection PRAGMAs and rejects TEMP/attachments;
pinned celld uses its qualified authorizer/limit equivalent plus actual negative
capability tests. This is a sealed host precondition, not an extra user-selectable
import or field in context. No callback or other owner may mutate connection
configuration between that check and transaction use. Wasm schema admission then
streams the exact immutable union via page9 and checks43/44, while runtime calls
check the same admitted connection/cookie. Host readiness requires both capabilities
and successful COMMIT/sync; neither substitutes for the other. Explicit unsupported
capabilities fail closed and are reported, with no engine-limit changes.

Native/Wasm tests for this subplan include maximum context and each one-over field,
all duplicate/unknown/missing/nested wrong types, numeric limits/aliases, invalid
serial padding, byte-range errors, backslash/non-ASCII refusal, input-size and
allocator failures, wrong/repeated callback kinds and phase transitions, legal
activation page callbacks, scalar/page output-size edges, and import status/trap
provenance. Pin exact maximum JSON bytes and native/Wasm field/error parity; prove
DER/serial/host buffers move rather than duplicate in the ownership audit. Separate
clock tests use exact microsecond expiry edges and one millisecond rollover. These
tests do not prove real TLS possession, production owner authentication or a hard
cross-host deadline.

Author review: field inventory follows current PeerCertificate, Identity, Fence,
Activation and Limits types, but uses fresh strict width-stable transport types.
The only variable context byte vectors have explicit small caps, and parent/scratch
admission is not conflated. Root and independent review are required before code;
no production route, schema factory or context implementation is added here.

Context review precision: check the incoming byte length before the ASCII/backslash
scan, parser or allocation. The host also checks its encoded context length before
calling the result allocator, and that allocator independently enforces the same
ceiling. The exact microsecond sample survives into the credential resolver; only
its checked quotient is used for the authority clock floor.

Successful-value storage alone is insufficient for parsing failures. Use custom
bounded visitors for all root/nested object, field-name, decimal and byte-array
positions, with fixed static error strings (at most64 bytes) for every rejected
kind/value. In particular do not call serde unknown_field/invalid_value/invalid_type
with an attacker-controlled field name or string: those helpers can format the
entire input into an owned diagnostic. Dispatch through deserialize_any and
explicitly override wrong-kind visitor methods so serde's default Unexpected::Str
formatter cannot reintroduce that path. Duplicate/missing/length errors are also
fixed constants. No arbitrary map buffering, raw input echo, reserialization or
formatted diagnostic leaves the context decoder; it maps the parser failure to a
fixed internal Invalid code and drops its error immediately.

Audit the pinned serde_json syntax-error paths separately: they must retain only
fixed error codes/static messages and bounded position values, not source excerpts.
Add a256-byte error allowance: one error representation with an asserted target
layout no larger than128 bytes, a static custom-message allocation capped64 bytes,
and64 bytes of explicit formatting/allocation headroom. No error is Display/Debug
formatted inside the kernel. The revised logical context allowance is therefore
N+4149+4096+256, at most26,997 bytes. If the implementation or pinned parser violates
this inventory, the static formula must be amended and independently reviewed
before tests; do not assume the visitor limit controls a library-internal copy.
Near18,496-byte unknown names, invalid decimal strings, wrong string-for-byte/root
values and malformed syntax must exercise failure admission and prove no diagnostic
copy proportional to their length. Native/Wasm tests also cover every visitor kind,
including null, bool, float, map and nested array. This is a new strict decoder,
not a claim that derive(deny_unknown_fields) alone has bounded errors.

The entire context allowance is reserved once before the context import and held
through the invocation; its DER/serial/host and typed context do not borrow a page
lease that can later reset. Physical input storage is still dropped after parsing.
For response kind1, the allocator consumes that existing context reservation.
Kinds2/3 consume the already reserved scalar/page scratch workspace for the active
phase; credential/owner scalar checks before the List page loop need an explicit
phase in the workspace audit. No response allocator invokes admit, checkpoint,
SQL, another allocator or any other host import. Its bounded local accounting
checks are sufficient because the parent reservation was acquired before entry.
Retained encoder/frame growth invokes admit only after any response callback has
returned. This distinguishes legal single-level host-result allocation from a
recursive admission callback and keeps the phase allowlist enforceable.


### Page9 actual-celld enumeration qualification passed

The seven-file fixture was committed as `ac9d3720` after author, root and recovery
agent source reviews. Its final validation base was `8ec3a044`, including the
reviewed List node-type projection correction. Native export (one test), seven
Node/actual-Wasm tests, three Python preparation tests, strict native all-target
metadata-core lint, formatting and diff checks passed. Actual local celld passed
all19 schema cases and all37 existing page vectors through the modified controller.
Both controllers exited successfully after child-process cleanup.

The native artifact pins327 named schema objects: FOKS59 plus private authority
and exactly10 independently pinned celld platform objects. The6,262,079-byte fixture
manifest is below its8MiB bound. Cases cover complete limit15/limit1 traversals,
EOF suffixes0/1/15/16, Unicode,256/257-byte names,65536/65537-byte SQL definitions,
literal sqliteX names, missing/changed FOKS and authority objects, and three wrong
platform expectation profiles. Actual runtime platform DDL is never mutated.
Every case checks exact native FKPG bytes, cursor progression, an independent full
schema observation and rollback restoration of schema/cookie/dependency coordinate.

The final default Wasm is `/private/tmp/foks-list-domain-default.wasm`, SHA256
`c32f961daddcda0c3c8aa632e6865c047cfdb46f86c15ca36a84f049f8939519`.
Exported catalog bytes match the List artifacts; the page-catalog SHA256 is
`a2b83eb458c38a39b3245fbcaa669bd9bd85255db0be08c4124813248b4f955b`.
Source manifest/hashes and the complete gate/failure ledger are
`/private/tmp/foks-schema-enumeration-manifest.txt`,
`/private/tmp/foks-schema-enumeration-sha256.json` and
`/private/tmp/foks-schema-enumeration-results.json`. Raw actual-runtime ledgers are
`/private/tmp/foks-schema-enumeration-celld-a/qualification.json` and
`/private/tmp/foks-schema-enumeration-page-regression-a/qualification.json`.

One initial Node test failed before its assertion because defensive NodeSQLite
forbids writable_schema modification. The reviewed test-only correction retains
real SQLite sqliteX/UNIQUE-autoindex coverage and feeds four fixed malformed raw
rows to the actual snapshot validator. It qualifies wrapper rejection of NULL/BLOB
name/SQL, not engine-corruption handling. The original failure log is preserved at
`/private/tmp/foks-schema-enumeration-node-a.log`; the corrected run is `-node-b.log`.
No production/runtime implementation changed in response to that failure.

Maximum observed schema-case Wasm linear memory was1,441,792 bytes, the largest
binary page72,275 bytes and the largest declared JS page working allowance1,255,672
bytes. These omit full fixture snapshots, engine/host RSS and allocator overhead;
they do not establish streaming activation memory or tenant density. Scalar43/44
were checked through their exact pinned SQL, not their Wasm scalar ABI. Celld's
performance clock returned zero within each case, so these runs establish no
latency or hard-deadline bound. The fixture enables no production route, activation
factory, authenticated frontend/cell ownership handoff, zero-loss replication or
host/AZ recovery. Moto is local ephemeral emulation with dummy credentials.

Context/import review record: quota and storage independent reviews PASS after the
static-error and callback-reservation refinements. Storage independently reproduced
the18,496/18,493 byte arithmetic and checked the selected catalog IDs. Final root
approval and the named implementation/source audits remain required; no code or
tests have been added by this plan.

### Strict context codec implementation contract and pinned parser audit

Root approved the context/import design; this is the isolated pure-codec slice,
not the host or Wasm export implementation. Base6662bcc8 includes the committed
shared public List argument parser. Owned files are core src/lib.rs (one module
declaration), new src/read_context.rs and its private decoder/tests submodules.
No Cargo dependency or Wasm export changes are needed. Tests/builds still require
author, root and independent source reviews and the shared Cargo slot.

Proposed Rust API: `HostListContext::decode(bytes: &[u8]) -> Result<Self,
ContextError>`, with private fields and borrowed `peer()` / `fence()` getters plus
`now_micros() -> u64`, `now_ms() -> i64` and `execution_deadline_ms() -> i64`.
ContextError has only InputBound, Admission and Invalid, with fixed code strings.
It contains no owned diagnostic and implements no raw-input Debug/Display path.
The returned value is structurally checked trusted-host *data*, not an owner,
credential, schema, transaction or publication capability. It is not accepted as
such by any production API. The decoder reserves the complete context allowance
through the existing Snowpack admission hook before parsing; callers must retain
that reservation while the owned decoded data remains live. A future host factory
will enforce that ownership, not this isolated public data type.

Pinned serde_json1.0.151 audit found two feature-dependent paths: with
arbitrary_precision, parse_any_number builds a String with capacity16 even for a
short byte; with float_roundtrip, a long raw number may grow its scratch buffer
before a visitor rejects it. Therefore an allocation-free lexical preflight runs
after the byte-length gate and before serde. Unescaped quotes allow a fixed
inside/outside-string state machine. Outside strings, only structural punctuation,
JSON whitespace and digit tokens are accepted; numeric runs have1..3 digits and no
leading zero except0. Signs, decimal points, exponents and literal identifiers are
rejected before serde. Every valid unquoted number in this context is a byte or
version1, so this excludes no valid context. Inside strings, only printable ASCII
without backslash is allowed. A fixed four-entry bracket stack checks matching
braces/brackets and the maximum nesting of the specified object/array shape.
Any open string, unclosed/mismatched container or excessive depth fails with a
static code. The parser still performs the full JSON grammar and field checks.

The quoted decimal fields remain canonical and at most20 digits through custom
borrowed visitors. Fields/values use deserialize_any with explicit wrong-kind
methods, so serde's default Unexpected::Str formatter is not called. From_slice's
unescaped SliceRead strings are borrowed; no raw_value or generic map/Value DTO is
used. The preflight prevents arbitrary_precision long-number maps and
float_roundtrip scratch growth regardless of Cargo feature unification. For short
arbitrary_precision numbers, the16-byte String is dropped before visiting the
parsed U64. Reserve an additional32 bytes of reusable parser temporary capacity
(16 bytes plus16 explicit headroom), not zero and not a new cumulative reservation
for every byte. It lives under the single invocation-owned context reservation.
The whole logical context allowance is now N+4149+4096+256+32, maximum27,029 bytes.
It bounds live parser workspace; cumulative allocation traffic is separately
observed and is not falsely described as zero allocations.

The pinned error.rs has an ErrorImpl containing ErrorCode and two usize positions;
ErrorCode's only payloads are Box<str> and io::Error. Audit those target widths
against a64-byte error-record ceiling. Error::custom allocates the fixed message
and converts it into Box<str>; syntax errors retain no input. Position fixing and
end_map/end_seq may briefly coexist with the previous error record, so the256-byte
reservation covers two64-byte records plus two64-byte static-message blocks, not
only one error. No Display/Debug formatting runs inside decode. A source change
adding a payload or a new parser allocation requires this inventory to be reviewed
again; size_of(Error) alone measures only its outer pointer and is not the proof.
Tests assert the conservative audited payload/layout terms on both targets and
exercise static errors at each visitor/syntax boundary.

Native focused tests cover exact fields, field-width limits and joint time
arithmetic; every wrong kind/unknown/duplicate/missing field; long names/decimals;
raw numeric overflow/fractions/exponents; escapes/non-ASCII; depth/grammar errors;
precharge refusal and exact maximum reservation; and vector ownership transfer.
Run the same suite with the dependency features arbitrary_precision and
float_roundtrip enabled together to exercise short-number allocation behavior;
no current workspace request for those features is needed to test unification.
Also check raw_value/preserve_order/unbounded_depth compatibility where supported,
without changing this codec's four-level bound or introducing generic Value parsing.
Compile strict native and wasm32 targets and retain the no-new-export assertion.
This component supplies no runtime codec fixture export: actual guarded-context
Wasm invocation waits for the later reviewed host/ABI slice, and a Wasm build must
not be reported as an actual-Wasm runtime decode test.

Author audit record: exact length/lexical checks precede potentially allocating
serde paths; per-number temporary storage is explicitly reusable under one held
reservation; both error-record overlap and static message storage are bounded;
returned vectors move into existing PeerCertificate/Fence values. Root and
independent narrow plan approval precede implementation of these refinements.

### Runtime connection gate supersedes source-only TEMP equivalence

The connection probe committed as cb01959f found actual TEMP shadowing on the pinned
local celld binary. CREATE TEMP TABLE was denied, but CREATE TABLE temp.NAME
succeeded and an unqualified query selected the TEMP value over a same-named main
table. Configuration qualification remains FAILED; the earlier source-audit claim
that the authorizer is sufficient is not an admitted runtime capability. See
HOSTED_METADATA_CONNECTION_QUALIFICATION_PLAN.md and
/private/tmp/foks-connection-config-celld-b/qualification.json. A sealed connection
TEMP invariant or reviewed main-qualified catalogs must close this before activation
can grant readiness. No production artifact or route is enabled by the probe.

### Pure context codec qualification record

Commit235b8c95 contains the four-file codec source, which passed author, root, and
independent source review before qualification on base6662bcc8. Exact frozen hashes are in
/private/tmp/foks-read-context-sha256.json; results and artifact hashes are in
/private/tmp/foks-read-context-qualification.json. The focused11 tests passed;
the full native metadata suite passed100 tests with one existing ignored export
fixture. The same11 context tests passed with arbitrary_precision, float_roundtrip,
raw_value, preserve_order, and unbounded_depth together, and separately with
float_roundtrip without arbitrary_precision. Strict native all-target lint,
default and optional-feature Wasm lint/build, and owned-file formatting passed.
No source correction was needed during qualification.

Both Wasm artifacts declare18 initial/1024 maximum memory pages and retain the
prior default artifact's exact23 export and three import name/kind sets. An initial
order-sensitive comparison differed only by linker export ordering; the canonical
sets are identical. This is compilation/export qualification, not runtime Wasm
context decoding. Successful structural parsing grants no identity, schema,
ownership, or publication authority, and the default artifact remains a fixture
artifact with raw exports. The connection-readiness failure above remains open.
