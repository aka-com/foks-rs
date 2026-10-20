# Single-transaction List feasibility and implementation proposal

Date: 2026-10-16. Plan only; no serving route or new ABI is authorized by this
document. This complements `HOSTED_METADATA_READ_BUNDLE_PLAN.md`. Existing
schema activation, current authenticated certificate resolution, fencing and
physical representation prerequisites remain mandatory.

## Proposed decision

Qualify one bounded List invocation inside one owner-held SQL transaction before
implementing the earlier multi-call Begin/Next/Finish path. Keep private bounded
SQL pages, but consume them synchronously inside that transaction and return only
the complete encoded public response. This removes revision-restart starvation
between pages under continuous writes. It does not prove a 15-second bound or
make the current authority transaction read-only: `Prove` advances its clock
floor and requires a writable transaction.

The native server remains the byte/error oracle. Preserve public count
normalization to 4096, existing MAC/time predicates, full parent decoding,
optional-body omission semantics, and `final = returned_count < requested_count`.
Reaching the requested count means nonfinal even if the database has no next row.
Public FOKS framing and cryptographic formats remain unchanged. A new private
single-call request/result ABI needs separate review; the current 1 MiB page ABI
and 64 KiB scalar result ABI cannot carry a 16 MiB assembled result implicitly.

## Transaction and ownership sequence

1. Admit the original deadline and working-memory reservations before entering
   the existing physical owner/actor. Use its connection-affine verified schema
   capability; no public connection-plus-hash constructor or second writer.
2. Within one transaction, verify the schema cookie as required by that capability,
   prove the current owner/fence/profile and dependency coordinate, resolve the
   exact authenticated credential, and enforce the personal-profile policy. The
   invocation cannot choose a synthetic principal or supply a parent key.
3. Probe and assemble the selected parent through the existing 512 KiB slices;
   decode its complete exact bytes using the native decoder. Check its decoded
   read key. Drop exact bytes, decoder AST and all slice pages before creating
   the response encoder. Preserve decoded-ID/version mismatch behavior and parent
   error precedence over an unrepresentable unsigned time cursor.
4. Read at most 15 entries per private page. Keep one encoded page and at most
   15 decoded dirents/optional bodies, never an entire result collection. Decode
   optional bodies selected by the stored node ID before permission decisions;
   missing/unreadable bodies are omitted, while malformed payloads or
   infrastructure failures abort the entire response. Do not substitute decoded
   dirent node IDs for the native SQL selection keys.
5. Obtain eligible body-role generations in batches of at most seven. Apply
   permissions and append in original entry order with global extended-entry
   positions. Drop decoded rows and the page before fetching another page.
   Retained encoder allocations are charged independently of reusable page work.
6. Finish encoding, verify the exact public response/frame limits, perform the
   final same-transaction schema-cookie check, and commit the transaction. Do not
   open a fallback transaction to refresh credential or page state. Complete
   required durability and recheck the original deadline
   before releasing output. Commit/sync/expiry failure publishes no success.
   The physical worker and its completed result retain all relevant reservations
   until payload destruction or transport ownership transfer, including cancellation.

No object-store I/O or network wait occurs while assembling metadata in SQL.
Durability after commit can still wait. Read linearization is the transaction's
authorized consistent state, rather than a fresh independent Finish transaction.
A later revocation does not invalidate that already-authorized snapshot. This
does not establish safety against an unfenced separate owner or database clone.

## Role semantics and a bounded SQL-call budget

Member visibility is i16; a 64-member organization cap does not limit possible
key roles. A full-role cache alone cannot bound 4096 distinct body requirements.
Use the separately reviewed `RoleGenerations` page10 contract: seven indexed
latest-generation point lookups in one statement, preserving each slot, missing
rows and malformed values. Generation zero remains `Some(0)` and leads to ordinary
read denial; it must not become a corruption error. Negative/malformed stored
generations remain errors. NONE or roles above the credential's role need no
generation lookup, but their bodies must still be decoded first.

An optional transaction-local cache keys the complete role type and visibility,
including absent/zero results. It is a performance optimization, not the bound.
Its capacity/allocation must be explicit and it must never survive the transaction.
Retaining only the current page's decisions is also sufficient for the hard bound.

For 4096 entries, the current 15-row contract needs at most 274 page queries,
including an EOF query when required below the public limit. Three seven-role
batches per page cost at most 822 queries. The maximum parent costs one probe and
32 slices. These total 1129 calls, leaving 919 of the existing 2048-call ceiling
for authentication, schema/fence proof, parent-key permission and bookkeeping.
Before implementation, enumerate that fixed overhead from the actual shared
authority call graph and assert a conservative complete maximum in tests. Do not
reset counters between pages or role batches. Full schema activation is a separate
bounded lifecycle operation; a cookie change cannot silently trigger an unlimited
activation scan inside this request.

Batching must not turn corrupt generation data or host errors into optional-body
permission omission. Decode rows in order; if a later row fails decoding, retain
that failure only until earlier rows' required generation checks have been
resolved, then abort without publishing any prefix. Differential tests must cover
multiple simultaneous malformed/body/key conditions. Exact native key-map parity
continues to rely on bootstrap validation and complete typed-writer coverage for
unrelated roles; batching does not add a new claim of validating every role.

## Memory feasibility gate

The isolated 16 MiB parent decoder reached 52,494,336 bytes of Wasm linear memory.
It must finish and release its allocations before response construction. This is
not evidence that the combined operation fits: linear memory does not shrink,
and allocation layout/fragmentation determines whether subsequent buffers reuse
the freed regions.

The current encoder can retain approximately 32 MiB across its two geometrically
grown bodies, then allocate a final response up to 16 MiB before dropping those
bodies. Add one approximately 1 MiB encoded page, its at-most-15 decoded small
bodies/dirents, transient schema/encoding copies, role decisions and host buffers.
Those allocations require an explicit live-ownership audit and measurement;
adding independent phase peaks or assuming a 64 MiB fit is insufficient.

Reuse the separately reviewed parent-held fixed workspace with an exclusive
physical lease for page/decoder scratch. Do not cumulatively charge 274 page
allocations as if they all remained live, and do not release a lease while an
abandoned worker or borrowed row still exists. Encoder/output allocations must
remain separately charged until their actual lifetime ends. Exact workspace
size, borrow API and permitted allocation paths are pre-code review gates.

The dedicated output boundary must charge the complete response copy before it
leaves Wasm, plus frontend framing/queue retention. Measure Wasm linear highwater,
declared JavaScript page working bytes and process/array-buffer residency
separately; none is a substitute for the others. Keep the 64 MiB Wasm limit.

## Liveness comparison and acceptance

Multi-call bundles can overlap other SQL work, but every relevant intervening
write invalidates the saved global dependency revision. Sustained writes can
therefore prevent completion despite each individual page being fast. A single
transaction removes those inter-page invalidations, at the cost of holding the
tenant's writable owner through decoding and encoding. Measure writer queue/tail
latency as well as read completion; do not describe this as concurrent read-only
WAL access. Changing clock-floor ownership to obtain that behavior is a separate
design, not an implementation shortcut.

Both designs retain the original absolute 15-second deadline through admission,
SQL, assembly, commit, durability and output. Checks before/after synchronous work
do not interrupt a long SQLite statement or sync. A frozen celld clock cannot
measure latency. Qualification needs a trusted advancing host clock and explicit
interrupt/worker-termination behavior before advertising a hard recovery or
completion target.

Required gates, after author and independent source review:

* Native exact response/error differential tests: all cursor forms and count
  boundaries, empty/full/nonfinal pages, duplicate sort values, missing/denied
  optional bodies, malformed exact data, generation zero/missing/extremes, and
  4096 distinct supported roles. Check total SQL calls without increasing 2048.
* Native/Wasm repeated and fragmented allocation sequences: maximum parent then
  maximum response, all small-body sizes, exact frame boundary, failed allocation
  at each retained/scratch boundary, and traps/cancellation without partial output
  or early permit release. Stop and revise the design if 64 MiB is exceeded.
* Actual celld page10/guarded invocation qualification with exact catalog/profile
  artifacts, physical row/type corruption, SQL/commit/sync faults and owner/fence
  changes. Existing celld 2,200,000-byte stored-row limit still blocks the native
  16 MiB directory range; no successful smaller profile cures that compatibility gap.
* Matched single-transaction versus multi-call continuous-write experiments,
  using a real monotonic clock and original deadlines, across personal tenants and
  hot tenants. Record read success/retry, writer tails, calls and retained memory.
  These are local qualification measurements, not host/AZ recovery evidence.

## Proposed work split and review order

Root owns page10 plus the typed generation decoder and its independent query-cost
gates. Recovery owns immutable profile/activation, connection/host proof and the
reviewed eventual single-call entry ABI. Storage owns shared List orchestration
and native differential fixtures after those interfaces and the workspace/output
ownership contract are reviewed. No ordinary route is enabled by these groups.
Each implementation remains separately frozen, reviewed twice and tested before
integration. The production choice follows combined memory/liveness evidence;
the previously qualified token primitives remain useful but are not needed to
authorize an intermediate page inside one transaction.

Author review completed against the current native List, shared personal key
checks, parent decoder and bounded encoder. Parent and independent architecture
review are required before selecting the final orchestration/ABI.

Independent architecture review (recovery agent): PASS for this feasibility
proposal, including writable ownership, conservative 1129-call subtotal,
decode/permission ordering and sequential-memory qualification. Exact fixed
overhead, final ABI/workspace contracts and implementation reviews remain gates;
this review does not authorize orchestration source changes.

## Proposed domain-helper interface and fixed-call audit

Root architecture review also passed. The next independently reviewable source
group is a domain helper under `personal/list.rs`, with no host dispatcher,
activation constructor, serialized credential or new Wasm entrypoint:

```rust
pub enum ListCursor { Initial, Mac([u8; 32]), Time(u64) }
pub struct ListRequest { /* private normalized parent/cursor/count/load fields */ }
impl ListRequest {
    pub fn new(parent: [u8; 16], cursor: ListCursor,
               requested_count: u64, load_small: bool) -> Self;
}
pub fn read_personal_list(
    sql: &mut impl Sql,
    credential: &CurrentCredential,
    request: &ListRequest,
    now_micros: u64,
    maximum_directory_bytes: usize,
) -> personal::Result<Vec<u8>>;
```

`ListRequest::new` normalizes only count (zero or above4096 becomes4096).
`Time(u64)` deliberately retains values above i64::MAX until after parent decode
and parent permission, matching native error ordering. It is not the signed
token cursor DTO. The directory maximum is trusted configuration, nonzero and at
most the currently qualified16MiB, never a caller-requested increase. Invalid
configuration is UnsupportedProfile. The helper returns exact public
KvListResponse bytes, not `wire::Reply`'s scalar JSON wrapper. RPC envelope/framing
and output ownership remain the future adapter's responsibility.

As with `read_personal_point`, entry rechecks the SQL-created credential against
its current row and expiry, then requires the personal profile. `CurrentCredential`
has no public field/deserialization constructor; qualification must obtain it
with real `authenticate_peer` inside the same transaction. The helper itself is
not a guard, transaction owner or fence proof. Its return type cannot be presented
as production authorization. A later sealed factory invokes it only after current
authority/schema admission and retains the physical request/output owner. The
initial native domain tests may call it directly only under explicit qualification.

Current successful-path call audit, based on `authority::run_resolved(Prove)`,
`PersonalResolver::resolve` and the existing personal helper entry pattern:

| Work | Maximum Sql calls |
| --- | ---: |
| AuthorityState load + AuthorityClock update | 2 |
| Issued-certificate serial probes (width1 through20) | 20 |
| Resolver SSO + personal anchor | 2 |
| Domain credential recheck + SSO + anchor | 3 |
| Schema cookie before/after | 2 |
| Current read-dependency coordinate | 1 |
| Parent-key generation | 1 |
| Parent probe +32 slices | 33 |
| List pages including required below-limit EOF | 274 |
| Body-generation batches, at most3 per page | 822 |
| **Total** | **1160** |

This leaves888 calls below2048 without any role cache. The count includes the
deliberate domain-entry recheck rather than assuming the resolver's private
credential becomes directly available. Actual integration can remove redundant
checks only through reviewed same-transaction ownership; it cannot add per-row
checks without updating the bound. Transaction BEGIN/COMMIT, durability and host
admission are separate operations, not hidden inside this Sql-call count. If the
final guard adds another coordinate/proof query, update this table and assertions;
the current table is not permission to renew a budget or omit that check.

Implement the domain helper with page-local role decisions rather than a global
cache initially: fixed7-slot batching and at most15 decoded rows already meet the
bound and avoid cache allocation/eviction complexity. Preserve each earlier row's
permission resolution before exposing a later decode failure, with no partial
response publication. Root's reviewed page10 decoder is the only new generation
query path. Qualification initially uses existing allocation hooks with explicit
reported cumulative charges; reusable fixed workspace and the production16MiB
result ABI still require their own ownership/source review before integration.

Storage owns this domain source and native exact-oracle tests after root and
recovery approve these interfaces. A Wasm/full-result qualification export is a
separate reviewed harness change, not part of this interface authorization.


### Reviewed node-type projection correction

Author/root/recovery review found that native `kv_node` converts stored
`node_type` from SQL i64 to unsigned before returning exact bytes. The prior
private List projection did not represent this check. The approved correction
adds `typeof(n.node_type)='integer' AND n.node_type>=0` to the existing guarded
payload CASE. Presence and actual payload type/length remain observable; a
present invalid node has suppressed NULL payload and causes Corrupt, rather than
being mistaken for an absent optional body. Zero and positive mismatched types
remain accepted as the native conversion permits. No extra column, row limit,
public format or permission rule changes.

Qualification adds exact native service comparisons, malformed-type projection
fixtures and three immutable page vectors. Only those named page vectors may
seed CHECK-bypassed values, with `ignore_check_constraints` restored OFF in
finally and verified before kernel entry. Full snapshot comparison remains. The
pinned celld authorizer explicitly supports this test pragma; this is not an
ordinary admission option. Root and recovery plan reviews both passed. Catalog
and profile artifact hashes change deliberately and require reviewed regeneration
and actual page transport requalification before integration.


Domain interface review record: root and recovery independent reviews passed the
exact helper interface, retained three entry rechecks, and1160/1133 wrapper/helper
ceilings. Source group is isolated at `metadata-list-domain-validation`; its
frozen eight-file manifest/hash is `/private/tmp/foks-list-domain-manifest.txt`
and `/private/tmp/foks-list-domain-sha256.json`. Author review completed with no
build/test execution. Parent and independent implementation review are pending.


### Single-transaction workspace refinement (root proposal; independent review pending)

The earlier page-workspace ownership proposal assumed each page could cross an
async join. The preferred single synchronous transaction keeps parent decoding,
all List pages, role lookups and encoding in one physical worker. Preserve the
ownership guarantees, but do not introduce an individual blocking task per page.
A detached worker must retain the original Request through all SQL and output
preparation; the returned result must retain it until physical output cleanup.
No generic child Request or early extraction may release its allowance.

Keep the original Request CURRENT, RPC response budget and request deadline
installed for the whole worker. Only the Snowpack/SQL allocation hook changes for
scratch work. Use one fixed reservation charged to the parent before starting.
Its exclusive page lease counts cumulative temporary allocations within that
page; it has no public reset method. A new page can start only after the old
lease and every scratch-owning value are physically dropped. A refusal latches
the parent failure so fallback encoding cannot hide ambiguous admission.

The page boundary includes query parameters, binary page transport, up to15
decoded rows, all role batches and their temporary results, and per-row temporary
encoding. It ends only after these values are destroyed. Encoder retained-buffer
growth and final output are different lifetimes: charge them explicitly to the
original parent, not to the temporary page budget. The existing ListEncoder
charges scratch encoding and retained growth through the same hook, so changing
only the caller scope would undercount retained memory. Split those admission
points before enabling reuse. Parent-directory assembly/decoding remains a
separate phase that completes before the retained List encoder is allocated.

Do not select a workspace constant by increasing limits until a test passes.
First audit every allocation site and collect cumulative charge traces for one
maximum page, each malformed boundary, parent decoding and retained encoding.
Then propose a conservative fixed allowance and rerun both reviews on its formula
and interfaces. The proof must cover4096 distinct roles, maximum valid node
payloads, the exact public response-frame ceiling, parent-plus-result allocator
fragmentation and a64MiB Wasm maximum. A native admission ceiling is not itself
a Wasm peak-memory proof. Full native-range parent storage remains separately
blocked on celld's stored-row limit.

Deterministic tests retain the earlier requirements: no second live lease, reuse
only after physical drop, cancellation while blocked, drop-before-allowance order
for unstarted and completed work, error and panic unwind, restoration of nested
budget scopes, and output retention through a slow socket. Add an assertion that
SQL call/deadline counters are never reset between pages. This proposal authorizes
design/audit work only; exact APIs, allowance and implementation require review.


Workspace refinement independent review: recovery agent PASS for audit/design
only. Explicitly destroy the parameter Vec before lease reuse as well as page and
decoded rows. SQLite internal allocation remains separately measured host memory;
the Rust allocation hook does not account for it. Exact allowance/API review is
still pending. Existing RPC success encoding peaks at raw List + envelope content
+ final framed copy, approximately48MiB near the public limit, before allocator
fragmentation and host buffers. Its response hook reserves the two new buffers;
it does not erase the original List buffer. Preserve the existing exact framing
limit and independently qualify the whole path under64MiB Wasm.


Second independent workspace audit (quota agent): PASS for design/audit. Parent
retained-growth admission remains cumulative through final output, even when old
Vec allocations are freed; scratch reuse must not silently reset those charges.
Frame qualification will cover sequence widths at0/127/128/255/256/65535/65536/
u32::MAX/u32::MAX+1/u64::MAX because the exact payload cutoff changes with RPC
sequence encoding. Existing sequence256 coverage alone is insufficient for the
new generic ABI gate.


### Domain-helper qualification completed (2026-10-16)

The reviewed nine-file snapshot in `metadata-list-domain-validation`, based on
7730051f, passed focused List tests (8), full metadata native tests (88 passed,
1 explicit generator test ignored), the live native RPC oracle and issued-certificate
List-helper oracle (2), and the existing pinned Go full-flow/certificate-authority
test (1). The Go test does not invoke the new domain helper. Native core and
selected testkit strict lint, default Wasm strict lint/release build, manifest
rustfmt and diff checks passed. Node actual-Wasm tests passed 8; actual celld
passed all 37 private page vectors with exact reply and complete state equality,
including negative, zero and positive-mismatch node_type cases.

The first focused run exposed four test fixture raw payload rewrites that failed
the existing schema58 revalidation trigger. The reviewed test-only correction
sets ciphertext_bytes=NULL alongside exact_node; all triggers and intended
payload/status assertions remain intact. Root and two independent agents reviewed
that correction before retry. No production correction was needed after source
freeze.

Evidence: `/private/tmp/foks-list-domain-qualification.json`, source manifest and
hashes `/private/tmp/foks-list-domain-{manifest.txt,sha256.json}`, and actual celld
report `/private/tmp/foks-list-domain-celld-a/qualification.json`. Immutable default
Wasm is `/private/tmp/foks-list-domain-default.wasm`, SHA256
`c32f961daddcda0c3c8aa632e6865c047cfdb46f86c15ca36a84f049f8939519`;
its declared memory is 18..1024 pages (64MiB maximum). Page artifacts are under
`/private/tmp/foks-list-domain-page-vectors`; page catalog SHA256 is
`a2b83eb458c38a39b3245fbcaa669bd9bd85255db0be08c4124813248b4f955b`.
All nine source hashes were checked after runtime qualification.

This qualifies domain logic and page SQL/transport, not a complete hosted List
entry point, production authority guard, fixed reusable workspace, response
lifetime, latency, or failover. The celld stored-row limit still excludes native
16MiB directory records. These limitations remain prerequisites to enablement.


Commit record: root integrated the exact tested nine-file snapshot as **8ec3a044**.
The failed focused-a fixture run and initial listener-sandbox failure remain in
the evidence logs; neither is omitted from the successful rerun record.

### Concrete allocation ownership/API proposal (audit only; review required)

This proposal changes neither List semantics nor its 1160-call upper bound. It
introduces no serving route. First qualify ownership and charged work in the
existing synchronous domain helper, then connect the reviewed guarded factory.
No implementation or tests are authorized by this text alone.

**Owned result and admission.** Add a small core `ListMemoryOwner: Send + Sync`
interface with `reserve_retained(bytes: usize) -> bool` and `fail()`; the server
implementation delegates to the original Request's working admission and sticky
failure flag. Construction and use of this interface do not constitute an
authority guard. A qualification implementation supplies deterministic capacity
and failure injection. `ListMemory::try_new(owner: Arc<dyn ListMemoryOwner>,
limits: ListWorkspaceLimits) -> Result<ListMemory>` reserves the fixed scratch
allowance once before SQL and retains the same owner. Limits have private fields
and only the reviewed profile/qualification constructors; no caller-selected
unbounded limit. The request CURRENT, response hook, deadline and SQL-call counter
remain installed unchanged; only the Snowpack scratch hook is scoped.

A new memory-aware domain call returns `OwnedListBytes { bytes, owner }`, with
payload first and owner last. It exposes a borrowed byte slice, not an unguarded
`into_vec`. The future native adapter must transfer the complete owner into its
existing retained worker/result and physical output path; the Wasm/JS adapter
must explicitly retain its host admission until the last output copy is dropped.
The old unscoped helper may remain an explicitly qualification-only oracle; the
production factory must use the owned result path. Rust's Vec type alone does
not prove response ownership.

**Concrete private page boundary.** Extract a private `process_page` that takes
mutable SQL, encoder and scratch workspace plus immutable credential/request and
Copy cursor/count. Its only success result is `PageProgress { added, last,
exhausted }` (all Copy). It acquires an exclusive lease before allocating
parameters and returns only after parameters, PageBuffer, decoded rows,
role-query parameters/results, encoder temporary schemas/bytes and error-local
values have been dropped. It must not expose a generic closure returning `T` or
a caller-resettable budget. Lease/restore guards are declared before scratch
payloads so unwind destroys payloads first; the private budget becomes reusable
only when that lease ends. A second live lease fails. A bounded fixed `PageFault`
contains only the error variants used here; any heap-bearing generic error is
consumed/dropped within the lease, never carried out under a released allowance.
In particular do not use the general `PersonalError::StaleCache(Vec...)` as an
unchecked scratch escape. SQL/panic/error/cancellation paths obey the same drop
order. Budget refusal latches `owner.fail()` before returning.

**Encoder split.** Keep `ListEncoder::new` as the current qualification oracle.
Add an explicit memory-aware constructor carrying the retained owner. Only
`grow(entries)`, `grow(extended)` and `finish` call that retained owner directly;
`append` schema clones, Snowpack normalization and position encoding use the
active scratch hook. Growth continues charging the full new capacity before it
coexists with the old buffer; do not refund the old cumulative retained charge.
Final output is retained too. Reserve both possible growths before appending
bytes/counts, preserving the current atomic accepted-prefix rule. A refused
second growth must not silently clear the parent's sticky failure.

**Allocation-site inventory and formula, before selecting a constant.** Let
`S_P(k)` be a page descriptor's checked maximum row bytes, `r_P` its effective
limit, and `V/T/C` the target's sizes of core Value, rusqlite Value, and borrowed
Cell. Let `E(x)` be Snowpack's checked owned encoding capacity for the concrete
schema. Derive the scratch ceiling from these actual sites, not measured RSS:

| Site | Charge/lifetime to include |
| --- | --- |
| List parameters | `6*V + 33+16+32+16`, current helper precharge; owned until page end |
| Native query_page | parameter payload (20 per integer), `params*T + columns*(512+(r+1)*C) + S_P`, then PageEncoder's `24+r*S_P`; SQLite internal caches/work remain separate |
| Wasm page call | parameter header24 + Cell array + checked encoded parameter allocation, imported response allocation up to `24+r*S_P`; the JS host's declared budget/cursor charge is separate from Wasm admission |
| Decoded row container | `r*sizeof(Decoded)` before its Vec |
| Each dirent/node | helper `4*exact_len+4096`, plus the existing decoder's charges: each binary/text `4*len+64`, array `4*n*sizeof(snowpack::Value)+4096`, variant `4*sizeof(Value)+128` and tag charge |
| Dirent normalization | `KvDirent::decode` also calls `encode(value)` to retain its exact canonical bytes: `E(value)+4096`; include this independently of helper precharge |
| Role batches | up to3 batches/page: UID33, result `n*sizeof(Option<u64>)`, native/Wasm transport and PageEncoder charges for PageStatement10; fixed stack arrays need no heap reservation |
| Encoder append scratch | ciphertext lengths +512+40*sizeof(snowpack::Value), then `E(dirent)+4096`, optional `E(small)+4096`, optional `E(position)+4096` |
| Encoder retained | each actual growth's complete new capacity, plus exact final output length; never part of page reuse |

Thus a page ceiling is the checked sum of one List query/parameter charge, the
15-row container, at most15 valid-schema decode/normalization/append charges,
and three seven-role transport/result charges, taking the larger relevant native
or Wasm-side formula for the selected adapter. Container widths are measured per
target, not assumed equal. A trace collector must compare each category against
this formula for maximum canonical schema variants and under error injection
before the fixed numeric allowance is reviewed. There is deliberately no new
magic 16/32MiB scratch constant in this proposal. The existing byte limits alone
do not bound cumulative decoder charges for arbitrary malformed nested input:
malformed values may hit the cap and return infrastructure/admission failure,
without allocating beyond it. That is not a reason to grow the cap or weaken
native malformed-input error-order tests below the cap.

**Parent phase is separate.** Parent input assembly owns the one exact directory
buffer before its decoder; slice pages/parameters are short-lived but the exact
buffer is not. Either initially retain all existing parent charges on the original
owner, or add a separately reviewed parent-only scratch lease returning only
Copy RoleAndGeneration after exact input and decoded seed are destroyed. Never
reset a shared page budget while the assembly Vec survives. Full maximum parent
allocation traces and repeated parent→List allocator-fragmentation tests precede
any64MiB qualification claim. Parent allocation reuse must not bypass the
existing decoder or trust denormalized SQL key columns.

**Gaps requiring explicit treatment.** NativeSql scalar `admit/query` currently
clones parameters/rows without the page path's precharge. Wasm scalar `invoke`
uses serde parameter/response copies without explicit precharge. The entry
credential/profile/key checks use these paths; the new proof must add bounded
scalar admission before those copies, or keep a separately audited conservative
parent reservation covering them. A SQL row's borrowed type/length must be
checked before Blob/Text allocation. Static scalar parameter/column bounds and
actual JSON expansion matter; do not claim the page hook covers them.

Current Wasm TLS `clear()` retains Vec capacity, and a failed `sql_page` import
can leave PAGE_RESPONSE allocated. Before permitting a scratch lease to end,
consume/drop page-response capacity on success and ordinary error, and audit
all early returns. A trap cannot run Rust destructors: the existing host must
poison, wait for the outermost export to unwind, wipe/drop the instance, and only
then release that invocation's physical allowance. Do not refund a scratch
reservation while a retained TLS buffer or a trapped instance still owns it.
The normal output buffer remains retained until host output consumption, never
cleared as part of page reset.

**Proposed implementation and review split.** First add charge tracing and a
static formula test on qualification paths; freeze the numeric native/Wasm limits
for a second plan review. Then implement the private page scope, explicit encoder
retained hook and owned output, with no production dispatcher. Keep scalar and
TLS buffer corrections separately reviewable. Root owns native Request/output
integration, storage agent owns List/encoder allocation boundaries, recovery owns
Wasm import/output cleanup and host physical ownership. Before tests, both source
reviews cover each slice. Tests include all4096 distinct roles, maximum bodies,
every admission failure, late cancellation/unstarted work/panic/drop checks,
constant SQL/deadline counters across reuse, exact framing boundaries and slow
output. Native highwater and Wasm repeated large→small→large requests must show
actual retained lifetime and fragmentation behavior; page-only peaks do not
substitute for that gate. Full parent-size celld storage compatibility remains
blocked independently.


Allocation proposal independent review: recovery PASS for audit/design. Retain two
explicit gates: an actual allocation/try_reserve failure must latch the owner just
like a logical admission refusal; ListMemoryOwner is not the physical worker
permit, so root's retained wrapper must carry both through output cleanup.

### First qualification slice: charge traces and static bounds (plan review pending)

Scope: establish reproducible **native cumulative charge** evidence and a checked
static formula before implementing reusable scratch scopes. This slice neither
resets budgets nor selects a production numeric allowance. It does not add a
Wasm export or claim Wasm peak proof; its target-width formula is compiled for
wasm32, with runtime Wasm traces reserved for the later guarded/qualification
entry-point review. Keeping this first slice native avoids inventing an
unauthenticated List ABI solely to measure allocations.

Proposed owned files are new child test modules beside personal/list.rs and
list_encoder.rs, their cfg(test) declarations, and a new native-only integration
trace fixture. No Sql trait, Snowpack codec behavior, catalog/SQL, server Request,
production allocation hook or public response changes. Existing native List
oracle fixture builders should be shared only inside tests; avoid exporting
CurrentCredential constructors. Use a real issued-certificate/current-row fixture
or existing authenticated test setup for the complete helper trace.

The recorder implements the existing AllocationBudget. Allocate its fixed storage
before entering the hook: checked u64 total/call counters, fixed phase bins and a
bounded diagnostic event prefix. Recording never allocates or formats. Overflow
or a full diagnostic prefix is recorded explicitly; totals continue without an
unbounded event Vec. The prefix is diagnostic, never the basis of the correctness
assertion. Run serialization/reporting after the hook is removed. A refusal mode
rejects at a selected byte total/call index without changing the underlying codec.
Trace overhead is not claimed as payload memory or RSS.

Use three complementary observations:

1. Private unit tests call existing List parameter construction and row decode
   under the recorder, then drop all results before switching phase. The row
   fixtures come from the actual PageEncoder/NativeSql path, not hand-built
   mismatched page metadata. Enumerate all valid role/status layouts, maximum
   4096-byte dirent and65536-byte node records, missing/denied bodies, generation0,
   and malformed fixed prefixes. Record generic decoder, normalization and helper
   precharge totals together and compare them with the independently stated
   valid-schema formula from the allocation inventory.
2. Encoder child tests inspect the two private buffers before/after append. Compute
   the expected retained growth from old capacity and needed exact public payload
   length; compare each actual growth reservation and cumulative retained sum with
   recorded charges. The remaining append charges are scratch normalization.
   Finalization's exact output length is separately retained. This requires no
   production trace tags or a new ambient hook: expected growth is observed through
   existing private test access and checked against the charge event sequence.
   Test both-buffer growth in one append, no-growth append, finalization, and
   failure between the first and second growth without accepted-prefix mutation.
3. Wrap NativeSql in a test Sql delegator that records each query's statement and
   before/after cumulative counters. It does not reset SQL counters, catch errors
   into success, or change the transaction. Complete List runs record actual call
   counts/bytes and output oracle parity. Their charge totals must fit the sum of
   separately checked phase bounds plus retained encoder growth. This is an
   independent end-to-end check on the decomposition, not a substitute for the
   lifetime proof planned later.

The static formula is a checked test-only calculation over actual immutable
PageSpecs plus explicit valid-schema shapes. It uses target sizeof values for
Value/SqlValue/Cell/Decoded, descriptor row caps and parameter layouts, Snowpack's
published per-binary/array/variant admission rules, and exact encoder capacities.
Store a human-readable term table in the emitted report, not just a single total.
Every production charge site in the audit must map to a formula term; a new site
or changed descriptor fails the expected inventory/fixture review rather than
silently increasing a guessed cap. Tests independently sum observed totals and
assert <=formula for each case. Do not claim bounds for scalar allocations that
currently have no hook: list them as explicit uncovered terms and propose their
precharge correction separately before calling the whole workspace complete.

Concrete matrix:

* Empty and1/14/15/16/30/4096 entry results; include-small false/true, all distinct
  roles across i16 visibility extremes, absent and generation0 roles; exact native
  bytes and SQL counts retained (4096-role fixture currently1100 helper calls).
* One maximum page of valid dirents/small boxes, plus multiple maximum pages up to
  the existing exact public response ceiling; no larger test limit or truncation.
  Include each encoding prefix/capacity transition and a final partial page.
* Parent maximum valid16MiB and malformed bounded parent, traced separately from
  page/encoder lifetime. Distinguish admitted cumulative bytes from observed native
  allocator or Wasm memory; this test does not remove celld's stored-row blocker.
* Declared-overlong binary, wide/nested malformed arrays, negative/wrong node type,
  late corrupt body and missing SQL row. Record the first refusal/error and all
  admitted prefix charges. A malformed value may exceed the valid-schema bound
  and fail admission; it cannot induce uncharged allocation or a successful
  partial response. Error precedence remains compared with the existing native
  oracle where no admission refusal occurs.
* Fail every observed allocation index for representative maximum-page/growth
  transitions, retaining unchanged DB state and a subsequent successful exact
  result. A fixed trace run is not a replacement for ownership/drop/failure-latch
  tests in the later reusable-workspace implementation.

Evidence output is an optional environment-selected new JSON file containing
source/catalog hashes, target pointer width, sizeof table, formula terms, observed
per-phase/retained totals, SQL count, typed outcome and response hash. Default tests
assert the same facts without writing files. Output is create-new or explicit test
artifact path and is never read as an authority input. Preserve failing traces.
No runtime-capacity or latency statement is derived from instrumented runs.

Author review: this slice measures the current cumulative policy without changing
runtime behavior, and keeps the scalar precharge/Wasm transport gaps visible.
Root plus independent plan review precede implementation; author plus two source
reviews precede any tests. Once results exist, propose the exact fixed allowance
and scalar/TLS correction API as a separate plan amendment before implementing
scratch reuse. Native totals and target-size arithmetic alone cannot qualify
Wasm64MiB or physical permit/output ownership.


### Trace slice amendment: explicit native/Wasm qualification artifact

Root requested runtime target layouts and full-helper Wasm traces in the first
slice. This amendment replaces the preceding native-only/no-new-Wasm-export
scope restriction. It does not enable a production export. Plan review must cover
this exact fixture mechanism before code.

Add Cargo feature `list-allocation-qualification = ["dep:foks-rpc"]` and an optional
normal foks-rpc workspace dependency (the existing dev dependency remains usable).
This is the real existing response encoder, not a new framing implementation.
Default builds do not enable this feature. Qualification-only modules implement
fixed counters/phase guards and a small child Wasm adapter; default code has no
trace storage, exported fixture entry or changed SQL/codec behavior. Any trace
phase annotations in List/encoder are cfg-gated and do not move allocations or
change error ordering. Root's production artifact allowlist rejects this feature
artifact, regardless of its catalog/schema identity.

**Input and provenance.** The sole fixture request is compact JSON capped at
20,480 encoded bytes before allocation/copy. All fields are required and unknown
fields are denied: version (integer1), peer (existing bounded PeerCertificate
parts), now_micros (canonical nonnegative i64-range decimal string), parent
(exact16 bytes), cursor (explicit initial/mac/time DTO with required shape), count
(canonical u64 decimal string), load_small (bool), frame (bool), rpc_sequence
(canonical u64 decimal string), and fail_at (canonical u64 decimal string0..1e6,
0 means observe only). No supplied UID, credential role, CurrentCredential, fence,
SQL, limit override, scope-reset command or parent key. Time cursor keeps fullu64
for the existing post-parent conversion precedence. Parent maximum is the existing
16MiB constant. Cursor kind uses a strict string visitor, not serde's unit-enum
map alternative. Peer DER/serial bounds and no-leading-zero serial are checked.
The bounded fixture parser is explicitly **not** the upcoming strict production
context parser or a proof of TLS possession; fixture preparation obtains parts
from an issued certificate and authenticated test account. The fixture executes
existing authenticate_peer against real current SQL rows, then the exact same
read_personal_list helper. It does not fabricate CurrentCredential or invoke a
public serving path. Native and Wasm use identical fixture semantics.

Parsing/input setup is outside the charge-observation interval and is separately
reported as fixture overhead; input is dropped before parent materialization while
the bounded peer remains live. The interval starts before SQL authentication and
ends after complete List bytes and, when frame=true, the actual
foks_rpc::encode_success_response_at call. Install both AllocationBudget and
ResponseAllocationBudget recorders. Record the latter's charges separately so
Snowpack validation of the complete List cannot be missed or double-described.
Frame failure at the exact existing limit is a typed fixture outcome, not a reason
to enlarge that limit. Hash response/frame bytes in place, then drop them before
serializing diagnostics; no16MiB JSON output or full response copy to JS.

**Exports and imports.** Exactly three function exports are added in the feature
build: `list_charge_input_alloc(length:u32)->u32`,
`run_list_charge()->u32`, and `list_charge_layout()->u32`. Layout/run results use
existing output_pointer/output_length, maximum8,192 output bytes. The allocator
accepts only1..20,480 bytes and invalidates stale fixture input/output on refusal;
run consumes its input once and checks the bound again so the ordinary input_alloc
export cannot bypass it. Reentry is refused. The phase/counter recorder itself
uses bounded preallocated storage and checked totals; no host output/logging calls
occur from reserve callbacks. Layout returns target pointer width and the exact
sizeof table used in the formula, never frontend values. The normal artifact must
lack all three exports; the fixture must contain exactly the prior23-export
qualification set plus these three. Pin names and kinds, not just membership:

`memory`; functions `input_alloc`, `output_length`, `output_pointer`,
`page_abi_version`, `page_catalog`, `page_input_alloc`, `page_response_alloc`,
`read_personal`, `read_token_clear`, `read_token_setup`, `read_token_setup_alloc`,
`response_alloc`, `run`, `run_authority`, `run_directory_key`, `run_page`,
`run_personal`, `small_box_ciphertext_length`, `small_box_input_alloc`, `trap`;
globals `__data_end`, `__heap_base`.

Imports remain exactly foks.sql, foks.sql_page, foks.domain, with their existing
qualification signatures. This fixture uses only the first two; no List call may
invoke domain. No WASI/new network/time import is introduced. This is deliberately
the old fixture host, not the new foks_read_v1 production ABI. Reuse its trap
poisoning and copied-output rules. A trapped instance cannot be reused; discard
it and record the failure. The fixture artifact keeps the explicit64MiB maximum.

**Counters and phases.** A fixed phase enum distinguishes authentication, List
entry recheck/profile, parent assembly/decode, parent authorization, List page
transport, row decode, role query, append scratch, retained growth, List finish,
and RPC framing. Feature-only scoped phase guards restore prior phase on errors.
They do not reset budgets, SQL calls or deadlines. Nested PageStatement10 transport
is attributed under role query. Each phase reports admitted calls/bytes and the
first refused index; the total sums are checked independently. Retained growth
and finish are tagged exactly at existing charge calls, without introducing reuse.
Encoder private tests still independently derive capacity changes and compare
them with tags so the labels cannot hide retained growth as scratch. Recorder
retains a bounded event prefix only; checked category/total counters cover every
call. Failure to record or numeric overflow refuses admission, never wraps.

Result JSON has version, fixed outcome code, target layouts, per-phase and overall
call/byte totals, SQL calls, list/frame lengths and lowercase SHA256 hashes,
first-refusal index, and Wasm memory pages (native null). All integer counters use
canonical decimal strings, no JavaScript precision loss. It contains no raw cert,
SQL row, response bytes or error formatting. Ordinary helper/decoder errors map
to fixed fixture categories; an import exception/trap is a separate failed test,
not a comparable domain result. No partial result hash is presented as successful.
The returned counters describe charged cumulative work, not live allocation/RSS.

**Wasm qualification cases.** Export real native schema/data fixtures and exact
native response/frame hashes plus charge formula inputs. The Node SQLite host can
hold a valid16MiB parent without putting it into giant JSON: use immutable binary
assets (length+SHA pinned) and one bounded blob bind during fixture seeding; validate
state unchanged afterward. Run one maximum15-row page,4096 distinct roles with
bounded bodies, exact List/RPC cutoff cases at sequence0/127/128/255/256/65535/
65536/u32::MAX/u32::MAX+1/u64::MAX, and large→small→large plus malformed/refused→valid
cycles in the same nontrapped instance. Record linear-memory size after every call
and peak, separately from JS/SQLite asset ownership and declared host page budget.
Explicitly include maximum parent followed by near-limit response; do not infer
combined memory from separate successful primitives. If this exceeds64MiB, preserve
the failed case and revise ownership/design through review rather than increasing
memory or shrinking public response/directory limits. Actual celld still cannot
store a16MiB parent; do not rebrand Node's SQL memory qualification as a celld pass.

**Ownership split and gates.** Storage owns core trace counters/phase annotations,
formula/native fixtures and the three feature-only Wasm adapters. Recovery reviews
and may own the small JS fixture host extension under an agreed manifest; root
owns Request/physical worker integration later. Quota separately audits scalar
precharge: observed scalar byte counts remain explicitly incomplete until that
correction is implemented and reviewed. Freeze the entire source manifest and
both default/fixture export/import sets before native/Wasm tests. No artifact
runtime or test execution begins before author plus root/independent source
reviews and the shared Cargo slot. Default full regression and strict lint remain
gates alongside focused trace tests. Numeric workspace allowance is proposed only
after static terms, native/Wasm traces and uncovered scalar terms agree.


Trace-slice independent plan review: recovery PASS. Preserve three source gates:
pre-bound diagnostic serialization before allocation, enforce fixture input/phase
state on every new export, and physically dispose trapped instances rather than
merely retaining a poisoned instance. Default/feature import/export parity and
immutable binary fixture hashes are explicit tests. Root final plan gate remains
pending; no trace source or tests have been started.

### Trace source freeze for review (no tests run)

Root subsequently approved the trace amendment and authorized isolated source
implementation. The prior pending-plan paragraph records the earlier state only.
The Rust source now resides in `metadata-list-allocation-validation`, base
`6662bcc8`, with fifteen owned paths in
`/private/tmp/foks-list-charge-rust-manifest.txt` and exact SHA256 values in the
matching `-sha256.json`. Recovery owns the separate two-file JS host/harness;
its returned-zero page-allocation handling is undergoing final narrow review.
No Cargo, native test, Wasm runtime, or Node test has been run for this source.

The compact maximum fixture input is 17,100 bytes: a 4096-byte exact DER,
20-byte serial magnitude, 32-byte key, 16-byte parent, and 32-byte MAC cursor all
rendered as all-255 decimal arrays; the two bounded signed-range times use
19 digits, count/sequence use 20 digits, failure index uses seven digits, and
all required field names/JSON punctuation and longest boolean spellings are
included. Rounding to the next 4096-byte boundary gives 20,480. The strict native
fixture test pins that exact derivation and also tests padding to the bound,
plus one, duplicates, missing fields, and cursor-shape violations. The owned
Wasm input is explicitly dropped after parsing and before authentication/parent
materialization. This is a fixture decoder, not the production context codec.

Reporting separates Snowpack and RPC response-hook origins, including the RPC
control flag in a fixed sixteen-event prefix. Per-origin, per-phase, and total
counters retain cumulative admitted bytes/calls, refusal counts, and first refusal
ordinal. Full page scopes include nested decode/role/append charges; a separate
scratch maximum excludes retained Growth charges. Final encoder output is outside
that page scope. Fixed diagnostic serialization occurs into an 8192-byte array
before allocating the returned diagnostic Vec. Actual fallible allocator failure
at instrumented page/directory/encoder sites is distinct from hook refusal;
allocation abort/trap remains a host failure requiring instance disposal.

The checked formula module exposes ten named charge-site terms and six explicit
uncovered categories. Its target-specific valid-schema page/row terms are checked
against native and Wasm observations, including the complete page aggregate.
The retained cumulative `4*M` bound is derived from both buffers' geometric growth,
including the final clamped capacity, rather than treated as a guessed live-memory
allowance. Parent and full RPC framing are measured separately but their complete
static formulas remain pending, as do S2/S3 coverage and allocator/host internals.
S1 requested native copies are admitted, but their aggregate static formula remains pending.
The report therefore keeps `complete=false`; no production workspace allowance is
selected by this source group.

Native fixtures compare each actual cursor and load-small setting with the real
native service oracle, including MAC/Time and unsigned-Time overflow. Failed
operations expose no List/frame hash or length. Exact public frame cutoffs and
maximum-parent/near-maximum-response combinations remain mandatory runtime gates;
Node SQLite's ability to seed those records does not remove celld's physical-row
compatibility failure.

The trace validation tree was subsequently advanced without discarding its owned
source to committed `9442ef8d`, incorporating context codec `235b8c95` and S1's
three native scalar files. The lib module merge preserves both read_context and
the feature-gated trace module. S1's requested native binding/result copies are
now covered by the trace; S2/S3 remain excluded. A fixed role_generation_edges
fixture adds both i16 visibility extrema, an absent current key, malformed stored
current generation0, and a separate body-generation0 with a valid current key.
Only seeding that immutable malformed fixture may bypass CHECK constraints;
the setting is restored and verified OFF, with foreign_key_check, before queries.
Root's narrow source review passed; independent narrow review precedes tests.


### Trace native and artifact gate record (2026-10-16)

Final Rust and JavaScript source reviews passed before execution, including the
fixed role-edge seed and origin-separated accounting. The exact seventeen-file
union is frozen in `/private/tmp/foks-list-charge-validation-manifest.txt` and
`/private/tmp/foks-list-charge-validation-sha256.json`. Native focused trace tests
passed 7/7; the full feature suite passed 117 tests with one existing ignored
generator. The explicitly invoked issued-certificate/native-service exporter
passed and produced 36 fixtures and 58 oracle cases. Their outcomes comprise 44
successes, ten expected frame-size refusals, two expected corrupt-record errors,
and two injected trace refusals. This is not a count of Wasm runtime successes.

Strict native core/exporter lint, default and feature Wasm lint, and both release
Wasm builds passed. A lint-only autoderef correction was separately reviewed by
root and recovery before the lint rerun; functional tests did not require repetition.
Formatting, diff whitespace, and all seventeen source hashes were checked.
Both artifacts declare memory minimum 18 pages and maximum 1024 pages (64 MiB):

- Default: `/private/tmp/foks-list-charge-default-a.wasm`, SHA-256
  `b48324caf40a7c315310891329bfa9c8e5122841cdf9483912b5ec0bb92a725f`.
- Feature: `/private/tmp/foks-list-charge-feature-a.wasm`, SHA-256
  `c3dbfde62ab8d98e72c4598431bca3544755b4e1a2251b6cc97053c297bd00c8`.

Native evidence is `/private/tmp/foks-list-charge-native-qualification.json`;
immutable fixture input is `/private/tmp/foks-list-charge-vectors-a`.
Actual-Wasm qualification subsequently failed the combined maximum-parent/frame
case as recorded below. No complete workspace bound or 64 MiB feasibility claim
follows from these successful native/build gates.


### Trace actual-Wasm gate record (2026-10-16): full-range failure retained

The reviewed seventeen-file snapshot and immutable default/feature artifacts
above were checked before running the Node actual-Wasm harness. The run completed
41 of 42 tests successfully. The `large_parent_cycles` test failed on its first
case with a Rust allocation-abort trap in `foks_rpc::codec::frame`, called by
`encode_success_response_at`. The native oracle returned a valid 16,750,868-byte
List response and 16,750,904-byte RPC frame for that case. Neither the public
frame limit nor the declared 64 MiB Wasm maximum was enlarged.

The failing instance had 52,559,872 bytes of linear memory when it was retired;
this is the observed disposal size, not a claim that memory reached 64 MiB.
The allocator could not satisfy the framing allocation within the artifact's
maximum. The host wiped the complete current memory in approximately 0.93 ms
and dropped the instance after the outer export unwound. No Wasm trace report or
successful response was returned for the trapped operation. The two later
large-parent reuse-cycle cases were not reached and remain unqualified.

The report contains 56 of 58 native-oracle cases: 55 matching results and this
one trap. All thirty sequence/frame-cutoff fixture groups, maximum-page and
4096-role cases, role-generation edges, exhaustive small-call refusals, targeted
page/growth/finish refusals, and import/allocator-trap lifecycle tests passed.
These successes do not override the combined maximum-parent/response failure.
No complete production workspace bound or full-range 64 MiB feasibility follows.

Preserved evidence: `/private/tmp/foks-list-charge-node-a.log`,
`/private/tmp/foks-list-charge-node-a.json`, and
`/private/tmp/foks-list-charge-runtime-qualification.json` (overall `passed=false`).
The existing physical-row incompatibility in celld remains separate; this run
used Node SQLite and does not qualify celld storage of a 16 MiB parent.

### Page-response ownership correction and S3 inventory — proposed, not implemented

Recovery author review: PASS for the boundary below. Root and independent plan
review are required before source edits. This component follows the reviewed S2
scalar-response state machine and its trace integration; it does not change the
page wire format, SQL, catalog digests, public List semantics, or memory maximum.
The framing allocation failure recorded above remains a separate blocker.

**P1: one owned page delivery, with no retained empty TLS allocation.** Extend the
private S2 response state with `Phase::Page`. Replace the independent
`PAGE_RESPONSE: RefCell<Vec<u8>>` with the same phase-owned optional response used
by scalar delivery. An internal typed `DeliveryKind::{Scalar, Page}` selects the
allocator; public ABI names/signatures remain `response_alloc(u32)->u32` and
`page_response_alloc(u32)->u32`. Scalar delivery retains its 64 KiB cap; page
requires `HEADER_BYTES <= length <= MAX_PAGE_BYTES` (24..1,048,576). Neither
allocator accepts the other's active phase. Length errors, duplicates, missing
delivery, out-of-phase calls, and phase/generation mismatches latch a fixed
infrastructure failure; they never manufacture an empty successful page.

`Host::query_page` validates the descriptor/parameters and consumes the existing
aggregate call budget. It serializes the parameters under the current charge
hook, then enters `Scope::begin(Page)` immediately before `sql_page`. No Rust
reference or RefCell borrow spans the import. Only the matching result allocator
may reenter that scope. It marks the delivery attempted before invoking admission,
then releases the state borrow, charges the requested bytes, and uses
`try_reserve_exact` before initialization. It rechecks the unchanged epoch/depth/
phase and sticky failure before installing the owned Vec. A recursively invoked
admission hook cannot install a second buffer or clear an existing failure.
Allocation or admission failure returns zero with a sticky Bounds failure; it
must not unwind Rust or call another host import from the allocator callback.

After the import, parameters are dropped, and `Scope::finish(failed != 0)` either
moves the sole response Vec out or physically drops it. It never calls clear()
as a release operation. On success `PageBuffer::decode` consumes the Vec and
validates the original descriptor, row count, reserved header bytes and exact
payload. Decoder/row-limit failure drops the Vec and latches the failure. On
success the PageBuffer owns all bytes until its consumer drops it; borrowed rows
and cells cannot outlive it. No extra page copy is needed for validation.
The scope is only an import-delivery scope, not a scratch lease: taking the Vec
out does not refund its allowance or prove that the enclosing page has ended.

The exact allowed Page transitions are depth1 Idle→Page and depth2 Domain→Page
inside the already established nested Invocation. No other depth, previous phase,
or reentrant top-level entry is accepted. Scope drop restores only that recorded
previous phase; it cannot clear sticky failure or restore a stale generation. The shared sticky error
prevents caller code from catching a bad page and publishing an apparently
successful outer result. All entry, setup, pure-helper and output guards continue
to reject invalid phases. A normal structured error may reuse an otherwise clean
instance; an export/import exception or trap still poisons it, waits for the
outermost export to unwind, wipes the capped current memory, and drops it. Do not
call Rust cleanup exports after a trap.

The existing qualification `run_page` deliberately moves its page bytes into
OUTPUT for the host to copy. Those bytes remain owned and charged as output;
they are not evidence that a page scratch lease may reset. The future serving
adapter instead uses the separately reviewed private `process_page` with a
Copy-only progress result. A PageBuffer or heap-bearing error must not escape
that lease. Output copies, retained encoder growth and a READY instance's grown
linear memory retain their original owners independently of page scratch reuse.
This correction introduces no output-release ABI, production admission, or
numeric reusable workspace constant.

**Host origin handling.** An allocator's ordinary zero return and an allocator
trap are different events. The trace-only host may translate only its proven
local normal-refusal sentinel into nonzero `sql_page` status, so Rust can produce
a diagnostic refusal report. A general KernelError, identical message/code, SQL
exception, or thrown allocation export is not that sentinel. Preserve the S2
subclass `invokeExport` delegation so the base wrapper does not accidentally
classify this private logical sentinel as a trap. The ordinary qualification
host may conservatively retire an instance on its thrown allocation failure;
this slice does not promise normal-refusal reuse for every host implementation.

**P1 tests and ownership.** Recovery owns the private response-state/page bridge
changes after quota's S2 merge, plus narrow host/harness amendments where needed;
quota reviews state-machine compatibility and storage reviews page-consumer
ownership. Root reviews the complete merged manifest before tests. Required
native state tests cover scalar↔page wrong allocator, duplicate/missing/zero/
oversize delivery, charge refusal before allocation, reentrant hook, sticky
failure, success transfer and all early-return drops. Actual-Wasm tests cover
large→small→large page delivery, malformed page→valid reuse, nonzero import after
allocation, out-of-phase callback, true allocation-export trap, host SQL failure,
and fresh-instance recovery. The same-state snapshot and page byte parity remain
unchanged. Allocation observability must inspect owned capacity/drop state, not
only Vec length; Wasm linear memory need not shrink after Vec deallocation.
Default/feature import-export sets, all existing authority nested-domain vectors,
page and directory vectors, trace refusal sites and strict lint remain gates.

**S3: explicit allocation inventory before a workspace claim.** S1 covers native
scalar requested bindings/results and S2 covers private Wasm scalar JSON and
response ownership. Neither covers every caller-side value or JS/SQLite object.
The remaining audit is organized by actual allocation and owner, not by counting
which helper was called:

| Remaining boundary | Required ownership and accounting evidence |
| --- | --- |
| Trusted context and peer | Hold the existing bounded context reservation through exact DER/serial/current credential use. Inventory moves versus clones; do not count the codec reservation as permission for later unchecked clones. Old fixture serde entry paths remain qualification-only. |
| Authentication/profile/key callers | Enumerate temporary Value vectors and byte copies before each scalar call, returned credential/owner IDs, certificate comparison buffers, hashing/normalization allocations, and errors. Charge caller copies independently of S1/S2 transport copies; preserve exact microsecond authentication and current-record checks. |
| Authority and proof callers | Inventory identity/fence/binding construction, command/context serialization, schema descriptors and retained proof state. List's selected Prove path may not claim coverage of mutation receipt/outcome or all raw fixture authority APIs. |
| Parent assembly/decode | Include one exact directory Vec, every slice parameter/page, Snowpack AST/schema/exact copies and temporary normalization. Parent-only lease may return only Copy RoleAndGeneration after all heap owners drop. The existing cumulative charges and observed fragmentation are not interchangeable with live peak memory. |
| List page decode and role access | Retain the static formula and trace by target layout; close page-response TLS ownership through P1, then prove the Copy-only page lease. Any malformed-input admission refusal stays infrastructure failure rather than a forged native domain status. |
| Encoder/result/frame | Keep encoder growth and final output on retained admission. Audit all three simultaneous buffers in the current borrowed raw List→RPC content→framed path, including exact framing overhead and sequence-width matrix. The observed maximum-parent/framing OOM prevents selecting a successful 64 MiB full-range profile today. Any consuming-frame redesign needs its own API/error/parity review. |
| JS parameter/binding/result bridge | Inventory TextDecoder strings, JSON.parse arrays/objects, hex/byte copies, SQL binding buffers, cursor row wrappers, binary PageEncoder, JSON.stringify/TextEncoder, memory views and output copies. Existing declared page budget is a bounded qualification contract, not a measured process/tenant RSS cap. No claim that Rust admission accounts for JS allocation. |
| Engine and persistent instance | SQLite caches, query workspace, runtime/allocator metadata and resident grown Wasm memory require separate host admission and capacity measurement. Vec destruction does not reduce linear-memory size; hold instance reservation or destroy/reinstantiate under a separately reviewed pool policy. |

For each row, the implementation proposal must list exact allocation sites,
maximum dimensions, target-width terms, refusal classification, physical drop
boundary and owner retaining the charge. Checked sums must distinguish cumulative
retained admission, reusable scratch admission, and actual live allocations.
Preserve the current Request/deadline/call counters across every scoped hook;
no lease may reset them. Source inspection and target-local traces complement
each other; neither a successful small trace nor a large conservative multiplier
establishes the missing whole-operation bound. S3 and P1 do not resolve celld's
physical row cap, connection readiness, activation/factory proof, full route
coverage, or synchronous SQL deadline qualification. Actual C2 evidence shows
the exact empty-TEMP query succeeds but pragma_database_list() attachment
inventory is denied; the run stops before the TEMP candidate matrix. Earlier
C1 showed qualified TEMP creation and shadowing are allowed. Consequently no
positive runtime TEMP-detection matrix or complete connection readiness is
qualified by these probes.

**Trace diagnostic exception is not domain success.** S2/P1 sticky Bounds must
remain observable by the feature-only trace entry. After an ordinary refused
operation has unwound its response scopes, that entry may serialize its bounded
fixed diagnostic report with `outcome=trace_refused`, refusal counters and no
List/frame bytes or hashes. It must not clear the sticky failure to resume domain
work or reinterpret the report as successful List publication. The diagnostic
path must explicitly reconcile recorder first-refusal and response-state failure;
a non-refusal infrastructure failure must not become trace_refused. Normal
bounded diagnostic extraction occurs after the Invocation guard ends and the
state has no live response allocation. A trap still yields no Rust report and
requires full host retirement. Freeze this narrow integration against S2 before
code; replacing every sticky refusal with an opaque export error would discard
the evidence required by the existing failure-site matrix.

### Preserved trace failure and next framing proposal (review required)

The seventeen-file trace group is committed as `5aa3a7ad`; the independent C2
fixture is `fef8f861`. Actual-Wasm evidence is
`/private/tmp/foks-list-charge-node-a.log` and
`/private/tmp/foks-list-charge-node-a.json`. The suite passed 41/42 test functions.
It recorded 56 of the 58 native-oracle cases: `large_parent_cycles-0` trapped in
`foks_rpc::codec::frame` while requesting the final approximately 16 MiB buffer.
The host wiped all 52,559,872 currently allocated Wasm bytes and dropped the
instance. The later small and repeated-large cases in that same fixture were not
reached. No diagnostic result or partial output hash was published for the trap.
The native oracle for that first case succeeds with 16,750,904 framed bytes.
This is a failed 64 MiB combined-parent/response qualification, not an expected
error converted into a pass. All fixed memory and input limits remain unchanged.

The largest successful observed instance reached 53,673,984 bytes. That is an
observation, not a safe allowance: the failed case demonstrates that sufficient
aggregate free bytes need not provide the next contiguous allocation after the
large parent decode. The trace records cumulative admission separately from
physical memory, and neither substitutes for an allocator/lifetime proof.

#### F1: owned direct framing, before any workspace-cap change

Propose a new `foks_rpc::encode_success_response_owned_at(Vec<u8>, u64)` beside
the unchanged `encode_success_response_at(&[u8], u64)` oracle. Its result remains
`Result<Vec<u8>>`. The owned API consumes the exact complete logical response;
no raw bytes escape on failure. Keep the original API and ordinary production
callers unchanged during this qualification group. Only the feature-only trace
adapter opts into the new API after computing the raw List hash and length.

The first implementation should encode directly into one final framed buffer,
without first constructing an intermediate `content` Vec. This removes one
whole-response copy without introducing encoder headroom, a new public wire
format, or a cursor-dependent buffer layout. An in-place/headroom API is a later
alternative only if this smaller change fails the unchanged runtime gate.

Preserve the original observable ordering: `reserve_success(raw.len())`, complete
Snowpack validation, then exact envelope/frame-length admission. Retain the
existing conservative response-hook charge `2*(L+128)` for this first group;
reducing a budget charge is a separate ownership review. Calculate envelope
length using checked arithmetic and the canonical unsigned sequence width, then
apply the existing `DEFAULT_MAX_FRAME_LENGTH` content limit. On oversized content,
return the same `FrameTooLarge { received, maximum }` as the old encoder. The
frame prefix is the existing canonical unsigned content length, followed by the
same four-element response envelope, nil status, Data/Header map, exact response
bytes, and pinned response header. Use fixed stack prefix/suffix staging or
checked direct byte writes; never allocate a second envelope-sized buffer.

Admit before a single `try_reserve_exact(final_length)` and distinguish actual
allocation failure from hook refusal (proposed `Io(OutOfMemory)` versus the
existing hook `Io(WouldBlock)`, with the trace marking only the former as actual
allocator failure). The owned raw Vec remains alive while copying into the final
buffer, then is explicitly dropped before returning. The future retained owner
must latch both refusal and allocator failure and hold the output reservation
alongside the physical worker/connection permit through transport cleanup; F1
alone does not create that owner or a serving route.

Expected live payloads during framing become raw+final (approximately `2L`),
compared with raw+content+frame (approximately `3L`). Snowpack validation still
creates and drops its AST before framing allocation. This change cannot be
claimed to solve fragmentation until the same full matrix, including all three
large-parent cycles, succeeds under the unchanged 64 MiB artifact limit.

F1 tests must compare exact bytes/errors with the unchanged generic encoder at
all ten existing sequence-width cases and the thirty below/at/above frame
cutoffs, including u64::MAX. Cover empty/general valid Snowpack, malformed input,
raw-length overflow precedence, envelope-only overflow, response-hook refusal,
and an injectable final-allocation failure. Re-run the existing native service
and full actual-Wasm trace matrix with unchanged inputs/hashes; preserve baseline
failure files and write a distinct report. No test may redefine the failed
large-parent case as an expected refusal. The source group needs author, root,
and independent review before tests. F1 may build on the trace baseline
independently of S2; their later union must separately review invocation guards
and allocator-refusal provenance.

#### Parent and framing formulas to freeze with F1

Use checked arithmetic throughout. Let `N<=16,777,216` be the exact parent length,
`S=524,288`, `q=ceil(N/S)`, `V=size_of::<SnowpackValue>()`, and `Q(spec)` the
already audited target-specific single-page transport charge, using the immutable
DirectoryProbe/DirectorySlice descriptors and bounded parameter bytes.

The parent cumulative requested-charge formula is:

`(49+3*size_of::<Value>()) + Q(probe) + N
 + q*((49+6*size_of::<Value>())+Q(slice)) + D_directory(N)`.

For a valid directory, the longest role shapes are Member for both key and
write role. There are five arrays with thirteen elements in total, two tagged
role variants with one-byte tags, and the ID/seed binary fields. The decoder
formula therefore bounds `D_directory(N)` by `4*N + 60*V + 21,064`; using the
whole record length for the seed overestimates its payload. This counts requested
schema-copy admission, not retained bytes. Tests must derive the same shape
independently from the existing codec and check Member extrema and all other
roles. The full-parent assembly stays alive during decoder AST and typed seed
cloning, so a physical peak model also includes approximately three parent
payload copies, target container overhead, and allocator fragmentation. Each
slice page is dropped before full decode. Invalid records retain the decoder's
existing global depth/value limits and fail closed; the valid-schema formula
must not be advertised as a complete malformed-input allocation bound.

For a valid normalized List with logical length `L`, `n` entries and `m` optional
bodies (`0<=m<=n<=4096`), derive the validation AST charge using the same independent
`shape` recurrence: the outer/list containers, `n` empty-payload maximum-role
entry shapes, `m` extended-pair plus empty small-box shapes, and `4*L` for all
binary/text payload bytes conservatively. This gives a checked formula dependent
on actual counts/length rather than multiplying every row by its maximum size.
Pin it against all trace output shapes and actual decoder charge; do not infer a
whole-invocation allowance from observed maxima alone. The response-hook formula
is exactly `2*(L+128)`. The old physical output model is raw capacity + `(L+64)`
content capacity + `(content_length+5)` frame capacity; F1 replaces the latter
two with one `final_length` capacity request. Actual allocator capacity, native
host/SQLite memory, S2 scalar work, and S3 domain copies remain separate terms.

A later workspace proposal may reset page scratch only after a private
`process_page` scope drops decoded entries, role results, parameter Vecs, encoded
page buffers and schema temporaries, returning only Copy cursor/count facts.
Retained encoder/output reservations cannot be reset with that scratch. No
numeric production workspace cap is selected by this formula or F1 proposal.

P1 narrow review refinements: native tests explicitly exercise both
response_alloc-during-Page and page_response_alloc-during-Scalar/Domain, as well
as every inactive/setup phase. Test-only capacity/drop instrumentation records
TLS response ownership, Scope-to-PageBuffer transfer and final PageBuffer drop;
it is absent from default production builds and creates no cleanup bypass. A
synthetic allocator-export trap proves trap retirement, not actual allocator
exhaustion. The separate real trace OOM remains the only observed physical
allocation-abort evidence here. Root's plan review is conditional PASS with
these transitions frozen; quota's independent compatibility review follows its
S2 union freeze. P1 implementation waits for that reviewed/committed baseline in
a distinct worktree, without concurrent edits to S2's wasm.rs.

S3 initial source-site audit (not a completed charge proof):
`personal::blob` performs `to_vec` before SQL admission. `authenticate_peer`
allocates a padded serial and clones exact DER for each of up to twenty fixed
serial-width probes; its matched row also copies credential/owner into EntityId.
`recheck_current_credential` and profile/key helpers use the same caller-side
blob constructor. These allocations precede, and are distinct from, S1 native
binding clones or S2 encoded-parameter storage. `PersonalResolver::new/resolve`
currently serialize peer/principal digests and allocate namespace/encoded role;
the future guarded List entry must explicitly choose and budget its resolver
path rather than infer coverage from the transport codec. Parent slice parameter
copies and generation-batch/result arrays already have local charge sites, but
must be counted under their correct parent/page owners. Existing fixture serde
and authority mutation/outcome paths are not automatically covered by the
bounded serving context decoder. These concrete sites anchor the next audited
formulas; they are not authorization to add a blanket reserve or alter error
precedence.

### S3a: limited List authentication caller admission — proposed independent slice

Recovery author plan review PASS; root and independent review required before
implementation. This slice covers only `authenticate_peer`,
`recheck_current_credential`, `require_personal_profile`, and the scalar
`current_key_generation` reached by parent-key authorization. It changes no SQL,
catalog digest, public arguments/status, permission order, or credential API.
It does not cover PersonalResolver's serialization/Principal construction,
mutation authorization, arbitrary point operations, generic authority receipts,
JS bindings/results, or complete List/page/parent/frame ownership.

**Exact private helpers.** Introduce private fallible byte construction used only
at these selected sites: `admitted_copy(&[u8]) -> personal::Result<Vec<u8>>` and
`admitted_zeroed(usize) -> personal::Result<Vec<u8>>`. Each calls the current
Snowpack admission hook for exactly the requested payload bytes before
`try_reserve_exact`; only after successful reservation may it copy or resize.
Both hook refusal and allocation failure return `PersonalError::Kernel(Bounds)`;
the feature trace separately marks actual fallible allocation failure. No error
contains input data. There is no reusable local allowance or new owner: these
charges remain on the already installed invocation owner, and existing request
sticky failure behavior remains unchanged. Infallible caller `to_vec`/`vec!`
at the selected sites are replaced, not merely counted after allocation. Arrays
of Value parameters remain stack arrays, with no heap container charge invented
for them. Scalar transport charges from S1/S2 remain additional and unchanged.

**Serial probes and retained credential.** Preserve time conversion and exact
certificate-expiry checks before any probe allocation. For canonical serial
magnitude length `s` (1..20), exact DER length `D` (1..4096), the original ordered
loop still executes widths `s..=20`, with `n=21-s` probes. Each attempted probe
admits `w` bytes for its padded serial and `D` for the exact DER copy, then calls
the unchanged SQL with those owned buffers. Do not reuse a mutable DER buffer
across imports, alter probe order, stop at the first match, or omit duplicate-row
detection. The integer now value has no heap payload and can be freshly formed
without clone ambiguity. Cumulative caller bytes for all probes are exactly
`n*D + (20+s)*n/2`, bounded by 82,130; all arithmetic is checked. Fewer probes
only follow an existing error/early return, not a new optimization.

After a matching row, keep the existing exact five-column shape, UID-kind,
credential/owner validity, Ed25519 key, role, credential-kind and owner-binding
checks in their current order. Consume the row's owned Blob values into
`EntityId::from_bytes` rather than cloning credential and owner payloads. That
constructor takes ownership and creates no replacement Vec. UID is copied into
a fixed `[u8;33]`. The retained CurrentCredential therefore inherits the row's
already admitted ID buffers; it requires zero additional ID payload allocation.
A helper may consume the five owned values only after the current row-count
check; it must not use a collecting map/intermediate Vec. Preserve duplicate
match detection before taking a second credential. This is a move of retained
ownership, not a refund of the scalar-row charge or proof that an authentication
scratch lease can end. A future reusable auth workspace must explicitly transfer
those retained owners or copy under separate retained admission.

**Remaining selected calls.** `recheck_current_credential` retains its expiry
check first, then admits exact owner-ID + UID + credential-ID copies at the
original parameter construction sites: `O+33+C`, at most 100 for a valid queried
credential (`O<=34`, `C=33`). `require_personal_profile` still rejects backup keys
before SQL, queries SSO first with no caller heap payload, and only if appropriate
admits the 33-byte UID copy before the anchor query. `current_key_generation`
retains UID/role validation before its 33-byte copy; two integer parameters are
stack values. `require_key` still rejects role mismatch before lookup and applies
the existing generation/read/write predicates afterward. No admission may move
a query or permission check earlier merely to simplify aggregation.

For the current trace path (one authenticate, one List-entry recheck, one profile
check and one allowed parent-key scalar lookup), selected caller charge is at
most `82,130 + 100 + 33 + 33 = 82,296` bytes. This is a cumulative selected-site
payload bound, not a live-memory ceiling or the operation budget. If a wrapper
performs additional profile/recheck/authentication calls, each adds the exact
per-call formula above; do not advertise 82,296 as covering a different call
graph. Scalar row/binding/JSON allocations, fixed stack layout, context ownership,
role-batch pages, and the rest of the List pipeline remain separately accounted.

**Tests and ownership.** Freeze only the selected personal module/helper plus
focused tests and any narrow feature trace assertions. Native unit tests with a
recording Sql and admission hook should verify s=1/20, D=1/4096, exact probe order,
all twenty DER/serial charge pairs, first/middle/last refusal before SQL, and
matched-row ID ownership without a new copy/admission. Test duplicate matching
serials, malformed ID/owner/key/role combinations, zero/maximum times, expired
certificate/credential, backup/SSO/anchor failures and parent permission denial;
assert existing public result classes and which SQL calls have occurred.
Inject the fallible allocation path through a test-only helper without assuming
that a synthetic failure proves a real OOM. No default allocator override.

Re-run unchanged personal/native List oracles and the existing default metadata
suite, strict native/Wasm lint and artifact builds. On the reviewed union, actual
Wasm small-call exhaustive refusal must report new caller refusal sites without
poisoning ordinary failures; real host/allocator traps still retire. Existing
trace formulas must add this explicit selected caller term instead of silently
absorbing it into S1/S2 transport charges. The original full-range framing failure
stays failed until F1 or another reviewed correction passes the unchanged runtime
matrix. No serving route, production schema/connection guard, numeric workspace
allowance, or broader domain coverage is enabled by S3a.


F1 source freeze: eight paths in the isolated
`.claude/worktrees/metadata-owned-framing-validation` tree at `5aa3a7ad`, recorded
in `/private/tmp/foks-owned-framing-manifest.txt` and
`/private/tmp/foks-owned-framing-sha256.json`. Author review checked both the
encoding/error-order implementation and a second ownership/bounds pass: fixed
raw digest remains unpublished on error, only explicit OutOfMemory marks physical
failure, ordinary callers retain the borrowed encoder, and parent/frame formulas
remain valid-schema cumulative bounds. Recovery independently reviewed all eight
frozen hashes and passed. Root direct review and Cargo scheduling remain required
before tests. No F1 tests have been run at this freeze.

### Hosted milestone scope update (2026-10-16)

The user selected a restricted hosted profile as the first serving milestone.
Its separate admission plan must bound individual values and complete stored rows
and explicitly reject unsupported records that remain valid in native storage.
Native behavior and limits remain unchanged. The preserved full-native-range
memory failure and regression matrix remain useful qualification evidence, but
passing that broader range is no longer a prerequisite to the first independently
reviewed restricted hosted profile. P1 and S3a ownership/admission corrections
are still required; this choice does not itself qualify a workspace allowance,
connection/authority guard, deadline, or production route.


F1 final union validation began on `5b6f86d6` (committed S2) in
`metadata-owned-framing-union-validation`; eight source paths/hashes are frozen
in `/private/tmp/foks-owned-framing-union-{manifest.txt,sha256.json}`. Root and
recovery reviewed the only overlap: the added parent/frame bound assertion in
the final S2 JavaScript harness. Native focused tests exposed an invalid empty
Snowpack array fixture before either framing oracle ran; reviewed corrections
use a valid `[Null]` array, preserve raw `0x90` rejection parity, and use canonical
Null for zero List entries/bodies in the formula fixture. No production change
was needed. The failed first log remains `foks-owned-framing-focused-a.log`.
Focused3, full RPC109, full metadata134 (one existing ignored generator), and
the issued-certificate exporter1 passed. Lint/Wasm/runtime gates are still
pending at this entry; this does not close the preserved64MiB baseline failure.

F1 final union qualification passed: native RPC109/core134/exporter1; all strict
native and default/feature Wasm lint plus release builds; actual-Wasm44 tests
and all58 oracle cases. The original large→small→large same-instance sequence
now completes unchanged: memory grows from1,441,792 to52,559,872 bytes on the
first large call and remains there for both following calls. The maximum across
all cases is52,559,872 bytes under the unchanged64MiB cap. Both boundary failures
and successful bytes/hashes still match the borrowed native oracle. This closes
the specific preserved framing regression, without selecting a numeric workspace
allowance or qualifying celld's separate2.2MB physical row constraint.

Final source evidence is `/private/tmp/foks-owned-framing-union-sha256.json`;
all hashes matched before and after runtime. Native/artifact evidence is
`/private/tmp/foks-owned-framing-native-qualification.json`; runtime evidence is
`/private/tmp/foks-owned-framing-node-a.{log,json}`. The prior failed trace remains
unchanged at `/private/tmp/foks-list-charge-node-a.{log,json}`. Root integration
and commit are pending at this entry.

F1 integrated as commit `7ec81ffe`, exactly the eight reviewed and qualified
source hashes above. All planning/evidence Markdown remains unstaged.


### S3a integration after F1 — selected caller formula

Integrate the original three reviewed S3a files byte-for-byte onto F1 commit
`7ec81ffe` in the isolated `metadata-auth-integration-validation` worktree.
The narrow additional trace change adds `Bounds.selected_auth_caller` and one
explicit term to the feature-only formula. Checked arithmetic computes
`20*4096 + (20+1)*20/2 + (34+33+33) + 33 + 33 = 82,296`.
It applies only to one authenticate, one List-entry credential recheck, one
profile anchor check and one allowed parent-key scalar lookup in the current
trace call graph. It is cumulative selected payload admission, not retained
memory, a total operation allowance, or a bound for any other call graph.

Keep `complete=false`; scalar transport aggregate formulas, remaining resolver/
authority/domain allocations and host/engine memory remain explicit exclusions.
Do not compare this term with entire Authentication, Entry or ParentAuthorization
phase totals: those phases also contain independently charged scalar transport.
The existing seven S3a tests independently measure the selected caller charges.
Add exact value/scope assertions to the native formula test and an exact layout
field assertion to the actual-Wasm trace suite. The unchanged exhaustive
small-call refusal matrix exercises the new caller refusal sites; no new phase
instrumentation, SQL, catalog or default export is introduced.

This integration owns six files: the original personal module/admission helper/
tests, trace formula, trace formula tests and `list-charge.test.mjs`. Root narrow
plan review passed; independent plan review and both final source reviews precede
any tests. Cargo remains with P1 until explicit handoff.


### P1 page-response ownership qualification (2026-10-16)

The six-file P1 slice was validated on committed F1 `7ec81ffe` after author, root, and independent source review. All frozen source hashes remained unchanged during testing. The shared response owner now distinguishes scalar and page delivery, transfers the sole page buffer into the decoder, and drops owned capacity on ordinary errors. It does not refund the caller's admission reservation. Physical reserve failure and logical admission refusal retain the same infrastructure failure class while only physical reserve failure marks the qualification allocator diagnostic.

Evidence: `/private/tmp/foks-page-ownership-qualification.json`, with immutable source, build, runtime, and artifact hashes. Gates passed: 12 focused response tests; 139 full native tests (one existing ignored generator); strict native all-target lint; default, trace-feature, and token-feature Wasm lint and release builds; 25 host/page/scalar Node tests including all 37 exact native page vectors; 10 actual-Wasm token tests; and all 44 trace tests covering the unchanged 58 native/Wasm cases. Default export/import names and kinds match F1; linker ordering of three token exports changed without changing the set.

The large → small → large trace sequence remains successful, with observed linear-memory high-water 52,559,872 bytes retained across the small call. Moving or dropping a Vec is not evidence of shrinking resident Wasm memory. Private injected reserve failure tests establish diagnostic classification, not forced production allocator exhaustion. This is Node actual-Wasm qualification, not celld qualification, a complete S3 workspace formula, or production activation. The original pre-F1 failed trace evidence remains preserved.

P1 was committed as `5a014c98` with the exact six tested source hashes; the qualification ledger records this commit. Markdown evidence remains unstaged.


### S3a final integration qualification — committed4842499c

Six reviewed source paths were tested on committed P1 baseline `5a014c98`;
manifest and final hashes are `/private/tmp/foks-auth-integration-manifest.txt`
and `/private/tmp/foks-auth-integration-sha256.json`. Root and independent source
reviews passed before tests. The initial compile rejected an ambiguous empty
array assertion in the new test; the twice-reviewed `.is_empty()` correction
changed no production behavior. Its failed log remains preserved.

Focused admission tests7, full native metadata tests146 (one intentional exporter
ignored), real TLS List compatibility tests2, strict native and default/trace
Wasm lint, and both release artifacts passed. Actual-Wasm Node scalar5, page11
(including37 page vectors), and trace44 tests/58 domain cases passed, including
all small-call allocation refusal sites and repeated full-range large/small/large
calls. The immutable F1 native vectors compare unchanged domain outputs and SQL
call counts; their historical charge counters are not treated as current S3a
measurements. Current native exact-charge tests independently pass.

Both Wasm artifacts retain18 initial and1024 maximum pages (64MiB). Observed
linear memory reached52,559,872 bytes and remained resident on a later small call.
This is fixture observation, not a production live-memory bound, celld runtime
qualification, serving-route activation or capacity claim. Selected caller
admission remains82,296 bytes for the named call graph only. Full evidence,
source/artifact/log hashes and retained failure details are recorded in
`/private/tmp/foks-auth-integration-results.json`.


### W1: close the page and retained-output ownership boundary (next concrete slice)

This proposal uses committed S3a `4842499c` as its baseline, including F1
`7ec81ffe` and P1 `5a014c98`. It supersedes the old statement that scalar copies and
PAGE_RESPONSE ownership are wholly unaccounted: S1/S2 now admit selected scalar
transport copies, P1 owns imported page buffers, and S3a admits the selected
personal-auth caller copies. None of those changes makes the existing List loop
safe to reset a scratch allowance. The remaining immediate obstacle is that
`ListEncoder` currently sends both temporary schema work and retained buffer
growth through the same ambient Snowpack hook.

**Implement W1 first, without any refund or new numeric workspace ceiling.**
Extract the exact private page function and give encoder growth/final output an
explicit retained owner. Its qualification mode remains cumulative pass-through
to the original invocation budget. This closes an actual ownership boundary;
it is not permission to reset a counter, install a nested server Request, lower
native limits, or expose a serving route. A later W2 may enable reuse only after
the remaining terms below and the concrete request/instance owner are reviewed.
No obsolete Begin/Next/Finish route is introduced.

#### Source audit: owners that must remain distinct

| Current source/site | Actual objects and proposed owner |
| --- | --- |
| `read_context::HostListContext::decode` | The caller's raw context bytes, fixed-capacity DER/serial/host vectors, Fence and parser allowance belong to the whole invocation. The reviewed precharge includes input size plus4,149 variable bytes,4,096 layout bytes,256 error bytes and32 short-number temporary bytes. It is a retained logical allowance, not a page reservation; the current codec does not itself carry its lifetime token. Borrow the decoded context and exact `now_micros`; never clone Peer/Fence into each page. |
| `authority::PersonalResolver::new/resolve` | Current `encoded(...)` makes an unadmitted temporary JSON Vec for peer hashing and another for principal hashing; `namespace.to_vec()` and `Role::encoded()` allocate too. The stored CurrentCredential owns IDs moved from SQL rows under S1/S2/S3a admission. These objects outlive page scratch. The existing resolver's millisecond conversion cannot substitute for the future List context's exact microsecond authentication time. Selected Prove state decoding also creates owned identity fields from JSON in `authority::load`. These are explicit S3b entry-work gaps, not covered by the new W1 page boundary. |
| Public argument parsing | `decode_kv_list_arguments` returns Copy fields but its Snowpack AST and BadArguments formatting allocate beforehand. Preserve the shared post-authority error ordering. Raw argument storage and error-status bytes remain invocation/control-output owned; a page lease cannot cover them. The future guarded adapter must admit them before invoking this parser. |
| `personal::list::parameters`, List query, decode and role batches | Parameter Vec/payloads, native bindings/cursor row copies or Wasm encoded parameters/imported page, Decoded Vec, decoded dirent/exact canonical bytes, optional small boxes, role parameter/result vectors and temporary errors all belong to one page call. P1 moves the one imported response Vec into PageBuffer and drops it on ordinary failures; there is no additional TLS page-capacity owner after the move. |
| `directory::read_list_parent_key` | Probe/slice pages and parameters are short-lived, but the assembled exact directory Vec survives all slices and its decoder. Keep all parent charges on the invocation in W1. A later parent scope may return only Copy RoleAndGeneration after both exact bytes and decoded directory die. Restricting the first hosted profile to1MiB does not change the ordinary native16MiB maximum. |
| `ListEncoder::append` | `entry.to_value()` / `small.to_value()` schema clones and containers, the three temporary encoded Vecs (entry, optional small, optional position), and their Snowpack encoder work are page scratch. The two encoder accumulation Vecs are retained across pages and must never be paid from the reusable page allowance. |
| `ListEncoder::grow/finish` | Charge each requested replacement capacity before reallocation, while the previous allocation may still coexist. Both entries and extended growths are reserved before changing the accepted prefix. `finish` charges its exact new List output before allocation while the two accumulators remain live. These charges remain cumulative and owned through output use, even after prior Vecs are dropped. Requested capacities do not include allocator metadata/rounding. |
| F1 `encode_success_response_owned_at` | The owned raw List remains live during validation and construction of the final frame. Validation AST is frame-phase temporary work. The final frame and unchanged conservative response reservation `2*(raw_length+128)` belong to the original response owner, not page scratch. F1 drops raw bytes before returning the frame; it does not refund the original reservations. |
| JS bridge, SQLite and the instance | JS parameter decoding, arrays, strings/BigInts, row conversion, encoded response copies and SQLite bindings/caches/query work are outside Rust page hooks. Existing page working-byte accounting is a declared fixture budget, not a complete JS allocation/RSS admission. Grown Wasm linear memory remains charged to a host instance/tenant reservation after Vec destruction, until eviction/destruction under a separately reviewed policy. |

No input/response/output TLS buffer may be counted as released just because its
length becomes zero. P1 specifically removes the independent PAGE_RESPONSE
capacity; other generic input/output buffers and JS output copies retain their
existing invocation/instance owner. No W1 lifetime claim extends to those other
raw qualification exports.

#### W1 private API and exact page exit

Add crate-private `read_personal_list_owned(sql, credential, request,
now_micros, maximum_directory_bytes, owner: Arc<dyn ListRetainedOwner>)
-> personal::Result<OwnedListBytes>`. The existing public unowned helper and
new owned path share the same private loop/encoding implementation; only a sealed
internal encoder/admission strategy differs. Both remain transaction-neutral.
Only the qualification trace calls the new entry in W1. No publicly implementable
encoder strategy or generic page result is introduced.

The concrete production-neutral core interfaces are:

- A private `ListRetainedOwner: Send + Sync` with
  `reserve_retained(bytes: usize) -> bool`, `failed() -> bool`, and `fail()`.
  The owner must be retained by every owned result. It is memory admission only,
  never a schema/credential/transaction capability. Refusal and arithmetic or
  allocation failure latch failure; success later cannot clear it.
- `OwnedListEncoder` uses the same encoding implementation as the existing
  ListEncoder, with explicit retained admission only at the two growth sites
  and final-output allocation. Keep `ListEncoder::new` and its ambient behavior
  as the unchanged compatibility oracle. Do not duplicate encoding semantics.
  Store byte buffers before the retained-owner field so payloads drop first.
- `OwnedListBytes { bytes, owner }` exposes only `as_bytes()` and `len()` outside
  its module. No public `into_vec`, `into_inner`, Clone, generic closure-return
  escape, or serialization that detaches ownership. Empty results still carry
  the owner. Result construction checks sticky failure before success.
- Private `process_page(sql, credential, request, after, remaining,
  encoder: &mut OwnedListEncoder, work: &mut PageWork)
  -> Result<PageProgress, PageFault>`. `PageProgress` contains only Copy
  `added: usize`, `last: Option<Position>`, and `exhausted: bool`.
  Position remains fixed id/MAC/time arrays and integers. `PageFault` is a Copy
  enum preserving the reachable fixed personal/kernel classifications, including
  static Invalid text. Heap-bearing stale-vector errors cannot escape this API;
  any future callee introducing one requires an explicit new ownership review.

`PageWork` is private and has an exclusive RAII active-page state. There is no
public method to reset it and no public generic `with_page<T>` API. The guard is
declared before page-owned values; its destructor observes their drop before
making the state available again. No await occurs in this function. W1 forwards
every scratch charge to the same original cumulative budget, without resetting
it. The boundary can later install a private fixed-scratch budget around exactly
this function, but W1 adds no reset-enabled constructor or budget constant.

Preserve the existing loop's ordering, not just successful bytes: validate a
row's position before accepting it; stop decoding at the first bad row; query
roles and append the previously decoded prefix before returning that recorded
failure, as the current code does. Missing or unauthorized optional small bodies
remain omitted. Both growths must succeed before an entry changes count/bytes.
A failed page may have grown retained capacities, but cannot publish any prefix.
The caller updates continuation/count only from successful Copy progress.
Finishing still sets `final_page = count < requested_count`; hitting the requested
count is not EOF, even when it happens to equal remaining rows. Time-u64 range
failure keeps its current post-parent-permission position.

The page scope must physically end after dropping, in particular: outer List
parameters, encoded/native bindings, the List PageBuffer, Decoded values and
canonical exact bytes, each generation result and its query's parameters/page,
append Values and encoded temporaries, and any converted error. Retained encoder
capacity is deliberately excluded. P1's shared response scope must already be
idle at every normal exit. On a Rust unwind RAII releases the page state only
after local payloads; on Wasm trap no destructors are assumed. The established
host must retire the instance after the outermost export unwinds and hold its
physical reservation until then. A trace-only failure report may describe the
failure but contains no successful domain bytes or hashes.

#### Retained output and frame transfer

The owned path is initially private to core/qualification code; no native service
or production Wasm route switches to it in W1. The trace harness exercises it
and retains the old unowned native helper/encoder as an independent oracle.
`OwnedListBytes` must remain attached through inspection and framing. A private,
non-generic `frame_owned_list` in the existing allocation-qualification feature
may destructure it, hold its owner in a local declared before frame work, invoke
the unchanged F1 consuming encoder under the already-installed original response
budget, and return `OwnedFrameBytes { bytes, owner }`. It checks the same sticky
owner after framing. There is no externally callable raw-Vec transfer helper.
No-frame trace mode drops bytes before its owner after computing only fixed-size
length/digest diagnostics. Frame mode keeps the complete owner through the last
frame hash/read and drops frame bytes first. No public RPC dependency or feature
change is needed for the feature-only bridge.

The retained owner must represent the original invocation's full working and
response ownership, not merely a new counter recording encoder bytes. A future
server bridge must retain the original Request/permits across detached work,
commit/sync, pending output and final socket use. Do not use a nested Request or
`RetainedValue::into_inner` to release a child owner while its raw/frame bytes
survive. W1's fixture owner proves ordering and sticky failure; it is not that
production bridge. Constructing its Arc/control state itself remains fixed
qualification overhead until the serving bridge's control allowance is reviewed.

#### Why a reset is still gated, and the exact next closure work

Once W1 passes, all values allowed to survive a page are either fixed Copy
progress or explicitly retained encoder buffers/owner. That is the required
mechanical precondition for a reusable allowance. W2 still needs all of:

1. Freeze checked target-specific page terms after S3a and P1: parameter/container
   storage, one List query, at most15 decodes/normalizations/appends, and at most3
   seven-role queries. Verify every requested allocation has exactly one physical
   lifetime owner, including bindings/temporary decimal strings and the P1 import
   allocation. Existing trace formulas bound cumulative valid-schema charges;
   they are not an assertion that every term is simultaneously live.
2. Specify a fixed parent-held page reservation and a private per-page counter
   bounded by the reviewed sum. Counter reset is permitted only after the W1
   scope exits and response delivery is idle. The parent retains the fixed
   reservation for the whole invocation. SQL call count, deadlines, cancellation,
   parent failure and retained-growth accounting never reset. No public callback
   or generic return type may carry scratch allocations out.
3. Keep entry/auth/context, parent decoding, frame validation and final output on
   their original non-reusable owners initially. Their costs must be bounded for
   the full request before calling the whole workspace complete. A reusable
   parent/frame phase, if later useful, is a separate typed lifetime boundary;
   it cannot be inferred from a page allowance.
4. Complete S3b for the selected personal Prove/entry path. Inventory bounded
   canonical peer/principal serialization, namespace/role construction, loaded
   authority identity/activation fields and every clone. A near-term narrow
   proposal should either pre-admit exact bounded writer capacities or stream
   the same canonical bytes into SHA-256 with a checked byte ceiling, retaining
   independent old/new digest goldens. Preserve existing A1 invocation semantics;
   the future List resolver must borrow HostListContext and authenticate with its
   exact microsecond sample. This is not authorization to silently broaden the
   generic authority/receipt codec or to treat successful context decoding as
   proof. S3b gets its own concrete formula and source review before code.
5. Review a real parent memory-owner bridge and host lifecycle: physical output
   ownership through commit/sync/write, TLS/JS copies, bounded admission metadata,
   runtime/SQLite headroom, and resident instance high-water. Requested-byte
   accounting alone does not establish a node RSS/density bound.

The first hosted profile's1MiB parent ceiling reduces its selected parent term;
ordinary native behavior and the16MiB regression remain unchanged. Admission of
that profile still depends on the separate R0/R1/R2 value and complete-row checks,
authority/schema/connection activation and actual runtime/deadline qualification.
No W1/W2 success substitutes for those gates.

#### Ownership, tests and review gate

Proposed W1 owned files: `list_encoder.rs` plus a private
`list_encoder/owned.rs` (and tests), `personal/list.rs` plus private page-boundary
tests, and the existing allocation trace bridge/tests. A new small private
memory-owner module is allowed if it avoids an encoder/personal dependency cycle;
freeze exact paths before code. No `wasm.rs`, scalar codec, catalog SQL, public
argument parser, server Request implementation or default export changes are
needed. Coordinate trace/formula files after the S3a commit; formulas continue to
say `complete=false`. No new numeric reservation is introduced in this slice.

Required evidence before merge:

- Old/new exact List bytes and error/SQL ordering across existing native service
  and maximum fixtures, sparse/no/all bodies, duplicate/distinct roles, malformed
  row after a valid prefix, MAC/time boundaries and count/EOF behavior.
- Fixed owner/drop instrumentation proves page values die before page-exit;
  retained capacities and raw/frame results keep their owner after page exit.
  Weak-reference/marker tests cover success, allocation refusal, late frame
  failure, cancellation/drop before use and native panic. No global allocator
  override or claim that synthetic failure proves actual physical OOM.
- Independent scratch and retained budgets: refuse each scratch call, each of
  the two growths and finish; no result on sticky failure. Successful retained
  growth does not debit a future scratch counter, and page completion cannot
  clear parent failure. W1's cumulative traffic must not be reported as reuse.
- The unchanged actual-Wasm58-case matrix and all callback/trap lifecycle tests,
  including maximum parent large→small→large and complete RPC sequence-width
  cutoffs. Report resident memory separately from logical scopes; retain the
  original failed pre-F1 run. No live celld route is added by this qualification.

Author source audit of this proposal: complete. Root and independent plan review
are required before implementation. Numeric page allowance selection and S3b
implementation remain separately gated, as specified above.


W1 independent plan review: PASS. Source-review precision: PageFault conversion
must retain the existing late-prefix and role-query error precedence. Ownership
checks do not replace a prior domain error merely because cleanup is running;
only an actually latched admission/infrastructure failure has the existing
sticky failure effect. In the raw→frame bridge, declare the held owner before
raw/frame payload locals and retain field-order checks on owned results, so
failure and unwind destroy payloads first. Test empty List, no-frame/drop-only,
and failed/no-output paths as well as nonempty output ownership. Root review
remains pending; no W1 code or tests have started.


### S3b proposal: bounded personal resolver digests and retained principal bytes

Plan only; not implementation authorization. Baseline `4842499c`. This is a
small successor to S3a, separate from W1 page/encoder ownership. It covers only
`authority::personal::PersonalResolver::new/resolve` and the serializer/helpers
those two functions directly need. It preserves the exact private context and
principal digest bytes, current permission/time ordering and all A1 wire formats.
It does not make `PersonalResolver` a production List resolver or replace exact
HostListContext microseconds with its existing A1 millisecond sample.

The concrete current sources are `authority/personal.rs:26` (temporary peer JSON
Vec before SHA256), `:54` (temporary principal JSON Vec), `:63` (retained33-byte
namespace clone), `:64` (`Role::encoded`), and `personal/mod.rs:98` (hidden
`expires_at_micros.to_string()` inside the derived PeerCertificate serializer).
`foks-proto/src/role.rs:47` constructs a Snowpack Value array, optional member
tag Vec/Box, encoder path entries and output. Those allocations are not removed
by merely charging the final role byte length.

**Digest helpers.** Introduce two private functions, not a public arbitrary
Serialize hashing API: `peer_context_digest(&PeerCertificate)` and
`principal_digest(&CurrentCredential)`. Each invokes pinned serde_json1.0.151
`to_writer` over the exact existing typed tuple and a stack SHA256 writer. Keep
the literal tags `foks-personal-peer-v1` and `foks-personal-principal-v1`, derived
PeerCertificate field order, array-number representation, tuple delimiters and
quoted decimal expiry unchanged. No JSON Value tree, intermediate JSON Vec or
String is constructed. The serializer's finite sequence/tuple/struct paths keep
state on stack; its byte-number formatter uses stack itoa buffers. The enabled
arbitrary_precision/float_roundtrip parser features do not change these typed
serialization paths. Source-review the pinned formatter and all reachable custom
serializers before asserting no heap temporaries.

The writer uses a checked byte counter and fixed maximum per helper. It never
formats input or creates an io::Error on bound failure: it latches failure,
returns the consumed input length to the serializer, and refuses to return a
digest afterward. This avoids manufacturing serde_json's boxed error merely to
report a bound. With the finite validated serializers, writes are otherwise
infallible. Unexpected serialization failure maps to static Bounds; no error
Display/Debug call is permitted. It must never expose a prefix digest. Tests
inject a smaller cap to prove refusal and later reconstruction behavior.

For the peer tuple, the conservative exact all-max-byte ceiling is
`4*(4096+20+32)+119 = 16,711` bytes, including the maximum19-digit signed-range
expiry string and all syntax. For the principal tuple it is
`4*(33+33+34)+36 = 436` bytes. These are serialized-byte/work ceilings, not heap
reservations; the digest state and counters are fixed stack values. Independent
legacy serializer tests must verify those maxima, minima and the digest bytes.
Actual entity type prefixes may reduce principal length; the bound deliberately
does not depend on that reduction.

Replace only `micros::serialize`'s temporary String with a20-byte stack decimal
formatter and `serialize_str` borrowing its canonical digits. Preserve the full
u64 formatting behavior of that serializer; validated PeerCertificate values
remain <=i64::MAX. Do not alter `micros::deserialize`: its existing owned String
and canonicalization temporaries are a separate input-codec scope. No broad
change to authority Number/encoded/json is part of S3b.

**Retained principal owner.** Namespace retains an exact admitted33-byte Vec.
The role uses a private `[u8;8]` stack encoding of the existing Role schema and
then an exact admitted copy of its used3..8 bytes. The helper has no public
protocol surface: NONE/ADMIN/OWNER use the existing three-byte array/empty-variant
encoding, MEMBER uses the existing tag0 variant plus canonical i16 integer.
Compare all65,536 member visibilities and the three other roles byte-for-byte
against unmodified `Role::encoded` in tests, including every integer marker
boundary. Do not call `Role::to_value`, `encoded` or a general encoder on the
production path, since that would reintroduce the temporary AST/path allocations.

Use the established S3a exact-copy reservation/try_reserve_exact discipline,
with only a crate-private wrapper/visibility extension if needed. Keep namespace
copy before role copy, at their existing position after successful authentication
and personal-profile checks. Hook refusal or actual reserve failure returns
PersonalError::Kernel(Bounds); only real reserve failure marks physical allocation
failure in the qualification feature. Both allocations remain with the original
invocation owner; CurrentCredential's existing moved credential/owner buffers
remain retained in the resolver. Neither digest helper refunds them. Principal
namespace/role live through their caller's authority action, with namespace moved
into existing SQL bindings where already done; no new cloning or scratch reset.

Per successful resolve, the new retained requested payload is at most
`33+8=41` bytes, additional to S3a and scalar transport. Hashing introduces zero
requested heap payload under the pinned serializer audit; this is not a claim
about stack layout, allocator metadata, full entry-path traffic or node RSS.
The current List allocation trace bypasses PersonalResolver, so do not add41 to
its82,296 selected caller term or claim that trace covers this resolver.
Qualification belongs in focused resolver tests and existing personal-authority
exact-replay/native/Wasm/celld oracle suites.

**Tests and file ownership.** Proposed files: authority/personal.rs, a new
private authority/personal/admission.rs and tests, personal/mod.rs serializer/
private admission visibility only, and personal/admission.rs only if visibility
is required. Freeze the exact list before edits. Tests preserve old JSON bytes/
digests across min/max DER/serial/expiry, entity IDs including delegated/Yubi,
all role values, and certificate renewal (principal stable/context changes).
Use independent legacy tuple/struct serialization, with its own old expiry
serializer rather than the newly changed micros::serialize, and capture reviewed
fixed golden digests before pinning. Recording admission tests assert33 then exact
role bytes, refusal before copying, retained owner/drop on a role failure,
permission/expiry/SSO failure order and no late SQL. Existing authority Register/
Execute replay, proof and error fixtures must retain exact outcomes/bindings.
A before/after full default export comparison and native/Wasm strict gates are
required. Any captured golden/test correction receives narrow review before
rerun. No Cargo or source edit is authorized by this proposal alone.

**Remaining entry-work blockers.** `authority::load` at mod.rs229–230 decodes
Identity/Activation through generic serde_json, owns the Identity.host Vec and
custom decimal Strings, and clones State data through separate reply/binding
paths. Generic encoded/json, intent/domain parsing, receipt binding/outcome
serialization, fixture-principal copies and stale-vector error hex formatting
remain outside S3b. They need a separate concrete admission/codec slice before
claiming selected Prove/entry closure or enabling W2 whole-request reuse. The
strict HostListContext decoder's retained allowance is already reviewed and
unchanged; it is data-only and no proof or transferable memory-owner capability.

S3b independent plan review PASS: exact compact maxima16,711/436 and the
latched discard-on-overflow writer were checked independently; the legacy hash
oracle must not reuse the new expiry formatter. Root plan review also passed; implementation is separately source-reviewed before tests.

S3b five-file source freeze: isolated `metadata-resolver-admission-validation`
on4842499c, `/private/tmp/foks-resolver-admission-{manifest.txt,sha256.json}`.
Author review confirms bounded streaming hash and independent legacy expiry
oracle, fixed role encodings, exact retained admission and unchanged error/SQL
ordering. Seven new focused tests are drafted (including full-u64 decimal
formatting boundaries); formatting/diff checks only have
run. Root and independent source review are required before execution.

#### W1 source gate and evidence boundary

W1 is authored in `metadata-list-workspace-validation` at base `4842499c`;
its eight-file frozen source manifest is
`/private/tmp/foks-list-workspace-manifest.txt`, with hashes in the matching
`-sha256.json`. Author, root and independent source reviews passed. The final
union is `metadata-list-workspace-union-validation` at `a777d0a9`, with the matching
`foks-list-workspace-union` manifest/hashes. Seven files are byte-identical; the
eighth adds only the reviewed owned-helper re-export to S3b's personal module.
Root and independent union reviews passed before testing. Focused ownership/page
checks and full native172 tests (two explicit exporters ignored), plus strict
native all-target lint, have passed. Runtime qualification remains in progress;
the first local export attempt was blocked by sandbox loopback EPERM and preserved
before an authorized loopback rerun into a fresh output directory.

The page closure and fixed `Copy` return types establish the source-level
ownership boundary. The ten authored tests cover scope state/unwind, retained
owner `Weak` lifetime, empty/nonempty raw and framed output, refusal and error
precedence. They do not instrument physical allocator deallocation or RSS.
Neither these tests nor dropping a page/Vec authorize a scratch-budget reset or
release the resident Wasm instance reservation. Framing failures latch the
retained owner while preserving the original F1 error; the old public encoder
continues using its original ambient charge and accepted-prefix behavior.

#### S3b final qualification — committed `a777d0a9`

The exact five reviewed source files passed validation on R0 base `87c0f97e`.
Evidence is `/private/tmp/foks-resolver-admission-results.json`, with the exact
manifest and hashes in `/private/tmp/foks-resolver-admission-{manifest.txt,sha256.json}`.
Native tests passed162 with2 intentional exporters ignored; the final focused
rerun passed15 including all7 new cases. Strict native and default Wasm lint,
release compilation, formatting, diff and final source-hash checks passed.
Fresh real Rust and explicitly enabled offline pinned-Go personal-authority
tests both passed and exported their own schema59 vectors. The final default
Wasm passed all33 Rust and31 Go cases on actual pinned celld.

The immutable artifact `/private/tmp/foks-resolver-admission-default-a.wasm` has
SHA256 `cbe6b6e0852cccf8a97d9d399c89fbd64ae26cbe40d08a77d8d017055e70454f`,
18 initial and1024 maximum memory pages, and the unchanged23 default export
name/kind pairs. An initial sequence comparison detected only export ordering
changes; normalized set comparison passed. The first test compilation failure
was an unsupported diagnostic LowerHex formatter, retained in `focused-a.log`;
the reviewed correction and later independently reconstructed golden pins
changed test assertions only. Full native evidence precedes the stronger pins;
focused tests and strict lint were rerun on the final exact snapshot.

These results qualify resolver hash parity and the selected admitted retained
copies. They do not close generic `authority::load` deserialization, establish
a complete live-memory bound, or authorize workspace resets.

### S3c proposal: pre-admit preserved typed authority-state decoding

Plan only; no implementation/tests authorized. Baseline `a777d0a9`. After root
requested a simpler alternative, the custom Identity/Activation visitor proposal
is withdrawn in favor of retaining both existing derived serde implementations.
Only the two persisted JSON values read by private `authority::load` are in scope.

**Source and compatibility inventory.** `authority/mod.rs:223..240` owns the11
Values returned by AuthorityState and decodes Identity, Activation, owner and
eight integers in that order. S1 native / S2 Wasm already admit scalar row/TEXT
copies; the additional typed parsers currently allocate without caller admission.
Identity.host is the only variable retained Vec. Number's custom decimal decoder
owns one String and builds one canonical decimal String, then returns a fixed i64.
Activation Limits (`dirents.rs:7..13`) are raw usize/u64 JSON numbers, unlike the
quoted decimals in HostListContext. Derived structs accept maps and positional
sequences, reordered/escaped keys and legal whitespace. Keep all these semantics,
public types, serde derives, decimal helper and writers unchanged. In particular,
do not introduce host-length33 or activation-positivity checks into load: those
are different semantic/error-order boundaries. Current legitimate Initialize
already checks host33; load itself does not.

**Small private wrapper.** Add `authority/state_decode.rs` with concrete private
`identity(&str)->Result<Identity>` and `activation(&str)->Result<Activation>`.
Each rejects input over the existing real scalar4096-byte limit before parsing,
reserves the checked envelope below, runs a small allocation-free numeric guard,
then calls unchanged `serde_json::from_str::<Identity/Activation>`. No generic
public Deserialize wrapper, custom object/field/sequence visitors or shared codec
refactor. Replace only load's two parsing expressions, maintaining their order.
Structural/parser failures remain Storage; admission/checked arithmetic refusal
is Bounds. Never format/Debug the serde error. Retained Identity.host moves into
State; its reservation remains in the original invocation, with no refund/reset.

Do not tighten the input cap to2048 UTF-8 bytes: authority DDL limits SQLite TEXT
characters, while canonical writers separately limit their output to2048 bytes.
The existing scalar hosts cap TEXT bytes at4096. Empty/malformed bounded input
still follows the old parser rejection once admission succeeds. The already
owned SQL strings are borrowed, not recopied or included again as input ownership.

**Why a tiny guard is still needed.** A length-only wrapper reaches pinned
serde_json float_roundtrip's lexical Bigint Vec/long multiplication for invalid
long numeric tokens (`de.rs:677..717`, `lexical/bignum.rs`, `lexical/math.rs`).
A finite envelope is possible, but auditing that invalid-only algorithm is much
larger than this slice. Scan string/escape boundaries without allocating; outside
strings, reject negative/fraction/exponent numeric syntax and digit tokens over20
bytes or whose checked u64 value overflows. Do not interpret field names or JSON
structure. All successful raw numeric fields are unsigned bytes, usize or u64;
signed Numbers are quoted. Thus rejected tokens cannot be accepted by either
existing target type. Leave other syntax/shape validation to unchanged serde.
Escaped digits inside quoted Number values remain accepted. Raw negative zero is
already rejected by typed unsigned decoding (`parse_number(false,0)` produces a
float). This guard avoids the long-number/arbitrary-precision map/float paths
without writing another JSON parser or changing accepted object/sequence forms.

The exponent/fraction compatibility claim has a pinned source chain:
serde_json1.0.151 `de.rs:1504,1507` dispatch typed u8/u64 to
`deserialize_number` (`:323`), then `parse_integer/parse_number` (`:462..537`).
A dot or e/E produces ParserNumber::F64 even for mathematically integral1e0;
ParserNumber::visit (`:118..125`) calls visit_f64. serde_core1.0.229
`de/impls.rs:414..447` unsigned/usize visitors implement integer visitors only;
`de/mod.rs:1494` default visit_f64 rejects. arbitrary_precision does not replace
this typed-number dispatch. Negative zero takes F64 too (`de.rs:521..527`).
Future tests must assert these exact tokens against unchanged serde under both
feature combinations, not merely test the new guard against itself.

**Pinned live allocation envelope, not an arbitrary multiplier.** Let N be the
bounded input UTF-8 length. Hold one uniform conservative reservation per decode:

| Requested Rust heap term | Bound and source reason |
| --- | --- |
| reusable parser escaped-string scratch |3N+8: pinned SliceRead/RawVec old+new requested capacities, same source proof as S2 |
| Identity host Vec including growth coexistence |3N+8: serde VecVisitor has zero size hint for JSON sequences, pushes u8, at most N bytes; Activation does not need this term but uniform reservation retains it |
| one live Number owned String and canonical decimal temporary |N+68: StringVisitor visit_str copies at most N; one Number is completed/dropped before the next, canonical i64 is at most20 bytes and3*20+8 covers even amortized formatting growth |
| arbitrary_precision numeric temporary on wrong-kind error paths |64: guarded raw token<=20 digits gives String16 growing to32, old+new48; no bigint/string-number map |
| two conservatively overlapping formatted serde errors and boxes |6M+256, M=10N+512: each message can coexist with old/new String or shrinking boxed-str storage at3M; four64-byte records cover end-map/end-sequence/position repair overlap |

Total `B(N)=67N+3476`, maximum277908 at4096 bytes. Two maximal load texts reserve
555816, not including the separately admitted scalar row. This deliberately
includes mutually exclusive terms and charges the host term even for Activation.
It is a conservative **live requested-allocation envelope**, charged once per
decode under the original cumulative invocation hook. It is NOT measured RSS,
allocator size classes or cumulative internal parser allocation traffic. Fixed
stack layouts are reported separately; there is no page-workspace reset.
The parent ledger is cumulative in **reservation events**: it reserves B(N1),
then B(N2), and retains both until original invocation disposal. Internal serde
allocations consume the held envelope without additional per-allocation events;
the envelope can be reused internally as buffers die, but no credit is returned
to the parent or a later invocation. Repeated load calls reserve new envelopes.
Do not label the resulting admission-event sum as exact cumulative allocator
traffic; the physical pool still holds the sum of the conservative reservations.
This slice does not change the parent's pool accounting or install a nested hook.

The error bound must be independently source-checked before code: serde_json
`error.rs:436..491` formats custom messages into String, then into_boxed_str;
`de.rs:1755,1806,1843,1854` may hold visitor and closing syntax errors together.
`serde_core/de/mod.rs:245..296,401..425` formats unknown/missing/duplicate fields,
fixed expected descriptions and Unexpected string Debug. A Rust Debug character
escape is at most10 ASCII bytes; decoded characters cannot outnumber N input
bytes. The512 static allowance covers punctuation, all expected field lists and
fixed type/array/integer descriptions (pin/assert their maximum, including up to20
printed count digits). No user-provided Display implementation participates.
Raw error strings are never reformatted by this wrapper. The pinned alloc String
format growth/reboxing old+new capacity proof is the same max(2*old,required)
source contract as S2; fmt::Arguments' initial literal estimate also stays within
2M. Pin serde, serde_json, serde_derive and toolchain source assumptions.

Serde's fixed arrays use stack storage; map field identifiers borrow/copy through
the one parser scratch, not separate owned keys; deny_unknown_fields stops before
unknown value deserialization. Partial Identity.host may coexist with an error,
and completed Identity.host remains while Activation parses: the first reservation
is still held, and each B(N) already includes its own complete envelope. Number
String errors, long escaped unknown names, wrong-kind strings and repeated fields
are therefore covered without custom error visitors. Public derive behavior is
the compatibility oracle, not newly reproduced test-only field definitions.

**Important allocator limit.** Unmodified serde uses infallible allocation APIs.
Pre-admission prevents logical overcommit but does not make a real global-allocator
OOM recoverable as Bounds or reliably mark allocator_failed. Native process OOM
and Wasm trap still require their existing physical process/instance ownership
and disposal policy. Do not promise an injected try_reserve failure test for
allocations this wrapper does not control. If the production requirement becomes
recoverable allocation failure for every parser copy, that is the reason to return
to specialized visitors, under a new reviewed scope.

**Tests and exact source ownership.** Only `authority/mod.rs`, new
`authority/state_decode.rs` and its tests. No W1, personal/mod, scalar/context,
SQL/catalog or Wasm ABI changes. Test wrapper against unchanged serde for canonical
writer values, reordered/escaped maps, positional forms, i64/u64/targetusize
boundaries, host-length variants, array lengths and malformed data. Test the
numeric guard independently against tokens accepted by the concrete unsigned
fields, including negative zero, exponent/float/overflow and escaped quoted forms.
Near-cap unknown names, control/Unicode escapes, wrong-kind long strings, duplicate
fields, Number parse errors and later Activation/owner failures cover the envelope.
Use source/layout pins and a scoped allocator-measurement qualification to compare
peak outstanding requested bytes (not totals) with B(N) for those adversarial
cases under default and arbitrary_precision+float_roundtrip builds. Measurements
are evidence supporting the source bound, not its sole justification.

A load mock tests exact11-column evaluation order, one charge per decoder and
refusal before that decoder begins; no AuthorityClock/resolver work may follow
refusal. Retained Identity and SQL row drop normally after later failures.
Native/full-authority/personal Rust+Go and actual celld replay vectors retain exact
outcomes, state and private hashes; default exports/catalog stay unchanged.
Two plan reviews and both source reviews precede tests. This smaller preserved-
serde option is preferred only after the explicit error-format bound passes review.

Generic `authority::encoded/json`, Initialize/Activate serialization, intent command
parsing/binding buffers, stored Outcome decode, fixture-principal copies and private
invocation parsing remain excluded. The direct List trace bypasses load, so its
selected82296 formula is unchanged. This is not full authority/route/JS/SQLite
memory closure. Author audit favors this option over custom visitors; root and
independent review are pending. No code has been written.

S3c root and recovery independent plan reviews PASS. Implementation authorized
only for this narrow wrapper; source review precedes all tests. Current lock pins
serde/serde_core/serde_derive1.0.229 and serde_json1.0.151. The inspected integer,
String/Vec visitor and error-format paths are unchanged from1.0.228; the only
impls.rs delta is unrelated in-place collection mutation.

S3c author-reviewed four-file source freeze: isolated
`metadata-state-admission-validation` at `a777d0a9`, exact paths/hashes in
`/private/tmp/foks-state-admission-{manifest.txt,sha256.json}`. The fourth path is
an isolated integration-test binary owning its own fixed-capacity thread-local
tracking allocator; it does not install instrumentation in other test binaries.
Five unit tests cover codec/guard/error/order/source terms; two integration tests
qualify actual authority::run parser peak and pinned formatting/rebox overlap.
The ErrorRecord assertion is an audited source-layout surrogate, not runtime
introspection of serde_json private ErrorImpl; source pins and allocator evidence
are separate checks. Author review verified the conservative realloc old+new model, no heap allocation
inside tracker bookkeeping, input/fence/mock creation outside measurement, and
error/retained owners dropped before returning. No tests/builds have run. Root
and independent source reviews plus final W1 baseline merge precede tests.

### W2 proposal: one request-held page allowance, exclusive page occupancy

Plan only; no W2 implementation or reset is authorized by W1. W1 has both source
reviews and is validating on `a777d0a9`. W2's first consumer remains the private
allocation qualification path. It must demonstrate a page-only reduction in
parent admission without changing native List bytes, errors, SQL ordering or
production routing. A real server-owner adapter is a separately reviewed part
of this slice, not an implicit consequence of the qualification recorder.

#### Exact owners and APIs to implement after review

Keep W1's `process_page -> Result<PageProgress, PageFault>` private. A request owns
one `PageWorkspace`; each call borrows it exclusively. Replace its marker-only
entry with `with_page`, whose closure returns only this exact fixed result type.
Do not expose a generic `with_page<T>` that could return an owned page, error
payload, closure, iterator, or reference. The closure destroys query parameters,
returned pages, decoded entries/bodies, role query results and append temporaries
before the hook is restored and occupancy becomes idle. No asynchronous closure
or child work is permitted. Page queries may synchronously deliver P1 response
buffers while the page hook is installed; their allocators consume that same
allowance and cannot start a second page occupancy.

A single invocation-level `Arc<PageBudget>` implements Snowpack AllocationBudget;
allocate it once, before any page work. Its fixed state contains occupancy,
nonwrapping generation, current-page charged bytes, invocation cumulative bytes,
refusal count/first refusal, and sticky failure. `with_page` resets only the
current-page counter after successful exclusive entry. The same Arc is reused;
no per-page Arc/Mutex/permit/container allocation is hidden in entry or exit.
Do not export its Arc or generic callback to callers. Reentrant entry, wrong
phase, overflow, parent failure or a previous refusal fails closed. Restore the
previous ambient hook before ending occupancy, including error/unwind paths.
A trap skips Rust cleanup: retire the instance after outermost unwind; that
workspace is never reactivated. W1's returned output retains its separate owner.

Introduce an explicit allocation-owner bridge, not another Request. Its reviewed
interface must distinguish `reserve_page_workspace(bytes)` once,
`reserve_retained(bytes)` cumulatively, `failed()` and `fail()`; it is allocation
ownership only, never authentication or a transaction capability. The core's
private W1 retained-owner adapter delegates to this parent directly, bypassing
the active page hook for encoder growth and final bytes. A refused local page
charge latches both local and parent failure before returning Bounds. SQL call
counts, admission event ordinals, first refusal, host deadline, cancellation,
P1 invocation epoch/failure and retained/output counters never reset.

For native `net::memory::Request`, the adapter keeps the original Arc<Request>.
Its one workspace reservation uses Request's existing working-pool reservation,
which stays in the original Request's reservation vector until that Request dies.
Add a narrow explicit sticky-failure method rather than trying to force failure
by issuing a bogus allocation. Retained charges still reserve from the original
working pool; the installed RPC response hook still reserves from its original
output/control pools. No new pool, nested Request or child permit is released by
`RetainedValue::into_inner`. Any future public adapter must make this ownership
interface callable across crates deliberately; W1's crate-private trait alone is
not a server adapter. Until that source gate, use the qualification parent and
keep server routes unchanged.

Frame validation runs after page occupancy under the original ambient request
hook, not the page allowance. F1's raw/frame transition and original response
reservation stay separate and follow the retained output to commit/sync/output
release. Entry authentication, context, authority load, public argument parsing,
parent assembly/decode and final encoding are not charged against the page pool.
An instance's grown linear-memory reservation remains held independently until
instance destruction, even when every Rust page Vec has dropped.

#### Target-specific checked terms and callsite reconciliation

Derive terms on the actual target using `size_of`/checked arithmetic and pinned
PageSpec fields; do not copy native values into Wasm. Let V be catalog Value,
Q native rusqlite Value, C page Cell, D decoded List row, S Snowpack Value, and P
Snowpack PathSegment sizes. Let r be the requested page rows (1..15), d=4096 and
n=65536. The List page's complete encoded maximum is
`24 + r * ListSpec.maximum_row_bytes()`; it is not r times only the ciphertext.
Role pages have their own specification and at most7 rows. The existing
`formula::full_page_scratch` is a *valid-schema cumulative charge* formula and
must remain labeled that way until these additional physical terms are audited.

| Site and lifetime | Existing charge to reconcile | W2 owner |
| --- | --- | --- |
| list::parameters Vec and UID/parent/MAC/id clones | `6*V + 97`, including time/MAC alternative overestimate | page |
| native query_page binds, decimal strings, cursor cells | parameter payload + parameter count*Q + columns*(512+(r+1)*C) + maximum row | page; SQLite internals excluded |
| native PageEncoder | one `24+r*maximum_row` preallocation; no per-row capacity growth | page |
| Wasm encode_parameters |24-byte initial header, bounded parameter cells and one full encoded growth | page |
| Wasm P1 page result | exact admitted response allocation moves into PageBuffer; no copy/refund | page |
| Decoded row Vec | `r*D` before capacity allocation | page |
| list::decode | `4*d+4096`, and when loaded `4*n+4096`, before typed decoders | page |
| Snowpack parse and typed schema extraction | array `4*len*S+4096`, binary/text `4*len+64`, variant `4*S+128`; dirent exact re-encode capacity+4096; typed ciphertext copies covered explicitly | page |
| role batches | up to3 batches: UID33, `7*sizeof(Option<u64>)`, RoleGenerations query terms; fixed16 Values/15 role slots are stack | page |
| append temporary schema/encoded entry/body/position | `d+n+512+40*S` plus each owned encode capacity+4096; no temporary escapes append | page |
| encoder entries/extended replacement capacities | unchanged full new-capacity charges, old and new may coexist | retained parent, never page |
| encoder finish output | exact output charge | retained parent, outside page |
| frame validation + F1 final buffer | unchanged Snowpack and response-hook charges | parent working/output, outside page |

Before computing a reservation, audit native decimal-canonicality temporary
`n.to_string()` per returned integer against the existing per-column512 term;
record its simultaneous and cumulative bounds explicitly. Audit borrowed row
views/headers and all parameters for each List cursor shape and the full16 role
parameters; no guessed "six parameters" formula may cover a role query. The
Wasm host's JS parameter/result arrays, BigInts, SQL rows and encoded ArrayBuffer
are a distinct host allowance with their own admission timing. A Rust page
allowance does not reserve JS heap, SQLite query memory or allocator overhead.

The additional fixed owner/control allocation is admitted once outside the page
counter before creating its Arc. Its formula must include target-sized state,
Arc reference-count/alignment overhead and any native adapter object, with an
explicit fixed-layout source audit; allocator metadata/RSS remains host headroom.
Do not allocate one permit or accounting Vec per page. Counters use fixed storage.

#### Malformed data cannot borrow the valid-schema assumption

Keep physical ownership closed even when malformed Snowpack returns before a
typed row exists. The existing valid-schema shape formula is not a decoder-wide
bound. Snowpack has depth64 and a1,000,000-value ceiling, but a small encoded
input can request a large array or declare a u32-length binary. `bytes()` requests
its allocation allowance before checking available input; that huge *demand*
need not be admitted. W2 rejects an over-budget demand before allocation, latches
Bounds and returns no partial List. Preserve existing bounded-hook ordering;
do not claim exact unbudgeted-native Corrupt versus Bounds parity for every
malformed allocation-amplification input.

Audit unhooked auxiliary allocations separately from admitted AST/container
bytes. In current decoder, the path can reach65 entries, a growing path Vec may
hold128 slots, variant path tags are at most31 bytes, and one cloned Error path
may coexist with the original path. A conservative requested-live envelope is
`(128+65)*P + 2*65*31`, plus fixed error/schema diagnostic objects and explicitly
bounded expected/found variant-tag copies. This is a candidate source-derived
headroom term, not an approved constant: verify all selected decoder/encoder
paths and typed Role/dirent/body errors before freezing it. No Display/Debug or
formatted diagnostic is permitted inside page processing. Error conversion must
consume diagnostics before W1's page scope ends. Encoder path temporaries also
need their own live bound; their known valid schemas do not authorize treating
arbitrary decoder paths as shallow.

Reserve fixed auxiliary headroom separately from dynamic page credits so an
attacker cannot spend it on AST payload and then require it for error cleanup.
The per-page credits may use the checked valid-schema cumulative upper bound
once every valid callsite is accounted for; rejected malformed demands then stop
before the corresponding allocation. Demonstrate this with maximum-depth paths,
long variant tags, arrays of tiny scalars, truncated u32-length payloads, wrong
schema after a valid AST and late malformed rows following a valid prefix. Any
unhooked allocation that cannot be bounded under the held headroom blocks reset;
source inspection alone must not relabel it covered.

#### Narrow implementation and qualification gate

First freeze a `PageAllowanceTerms` implementation and allocation-site ledger,
including native/Wasm layouts, auxiliary/error bounds and parent adapter state.
Root and independent review must approve those terms before selecting a numeric
reservation. Then implement one page-only owner/hook slice and native/actual-Wasm
qualification. Keep whole-request `complete=false`; S3b removes selected resolver
copies but generic authority::load, other authority JSON, caller context/entry,
frame/output, SQL engine and resident-memory accounting remain separate.

Tests compare cumulative traffic and parent pool reservations separately: more
pages increase total charge/SQL counts while only one page reservation is held;
retained growth/output still increase original parent reservations. Force every
page charge refusal, nested occupancy, wrong allocator/import, real export trap,
late role/prefix error, cancellation/unwind and retained refusal. Verify no next
page starts until pages/decoded/temporaries are destroyed, no failed invocation
recovers by counter reset, detached work and outputs hold the original parent,
and unrelated concurrent requests cannot use its occupied capacity. Repeat all58
trace cases and full page/refusal/callback lifecycle suites with original native
and Go-derived byte oracles, both target layouts and large/small/repeated resident
memory observations. This qualifies page-only reuse, not a hard15s SQL deadline,
whole-request memory closure, production activation or a smaller native cap.

W2 independent design review passed for this staged plan, not for a numeric
allowance or reset implementation. The allocation-term source gate also requires
reserve-control failures and actual local allocator failures to latch the parent
before another page can enter, even if no hook refused a charge. A trap or detached
operation retains the fixed workspace reservation until physical owner disposal;
no early refund follows only from marking occupancy failed.

#### W1 completed qualification (`345247ce`)

All eight union source hashes remained unchanged during validation. Evidence is
`/private/tmp/foks-list-workspace-qualification.json`: native172 tests passed
(two explicit exporters ignored in ordinary execution), focused ownership/page
filters7+5, fresh issued-certificate trace export1 and live native List oracle1.
Strict native all-target and default/trace Wasm lint plus64MiB release builds
passed. Node host/page/scalar25 tests (37 exact page vectors) and trace44 tests
passed; all58 trace cases matched native outcome, raw/frame lengths and hashes.
This is actual Wasm running in Node; W1 did **not** rerun actual celld replay.

Default23 export and three import name/kind sets match committed S3b's immutable
artifact; import order is unchanged, while the linker moved the three token
setup/clear exports after trap. Immutable W1 artifacts are
`/private/tmp/foks-list-workspace-{default,feature}-a.wasm`, with hashes recorded
in the ledger. Observed linear memory reaches52,559,872 bytes and remains there
through the large/small/large cycle; host-declared page working bytes peak at
2,630,213. Neither observation is a measured process RSS bound or refund proof.

The initial vector export's sandbox loopback EPERM log is preserved; a fresh
output directory and authorized loopback rerun passed without source changes.
No serving route, production proof, restricted-profile admission, numeric page
reservation or scratch reset was enabled. Root committed the eight exact tested sources as `345247ce`; the qualification ledger now binds that commit.

For W2's failure-latching requirement, an actual allocator failure means a
fallible allocation returning Err to the owned code path. Existing infallible
Snowpack/serde allocations may abort or trap instead; this proposal does not make
those failures recoverable or promise execution of a Rust failure hook afterward.
Their process/instance retirement keeps the parent/instance reservation until
physical disposal. Tests must label synthetic failures separately from real
fallible-reserve errors and from actual Wasm export traps.

W2 root staged design review passed after the independent review. Numeric
allowances and reset implementation remain unapproved pending exact target term
and auxiliary allocation source review. No W2 tests or implementation gates ran.

#### S3c final qualification — committed `6e6b275d`

The exact four source paths passed author/root/independent reviews and clean
W1 union verification at `345247ce`. Evidence is
`/private/tmp/foks-state-admission-results.json`, with the matching manifest/hash
files. Native179 passed/2 intentional exporters ignored; focused5 and scoped
requested-peak2 passed under both default and arbitrary_precision+float_roundtrip.
Strict native/all-target and default Wasm lint/build passed. Fresh real Rust and
explicit offline pinned-Go personal tests both passed. Actual celld passed all
133 fresh cases: personal Rust33/Go31, authority28, errors5, settlement10, faults7,
bindings9 and normalization10. No source correction or failed gate occurred.

Immutable `/private/tmp/foks-state-admission-default-a.wasm` SHA256
`9529feb8a8d0f3dec78593569b17dfad2519ef119cd0694e422b9395ec1f88c4`
retains the23 W1 export name/kind pairs and18 initial/1024 maximum memory pages.
Final source hashes, formatting and diff checks passed. This is requested-live
parser-envelope evidence; cumulative parent reservations, actual allocator
traffic, resident Wasm memory and process RSS remain distinct. ErrorRecord is
a pinned-source surrogate. Global serde OOM remains nonrecoverable by this wrapper.
Other authority codecs and production route/workspace closure remain excluded.

### W2 exact term/source gate (proposal; no budget/reset implementation)

The proposed private Rust term source is
`/private/tmp/foks-w2-page-allowance-proposal.rs`. It has not been compiled or
executed and is not a repository module. It is intended as a child of
`personal/list.rs`, using actual target layouts rather than caller-supplied
sizes. This gate freezes the calculation and its ownership assumptions before
selecting/activating a reservation. It replaces no production adapter and adds
no endpoint. Baseline is W1 `345247ce`; S3c can merge independently.

The concrete next implementation is one bounded group: private page allowance
terms/owner integrated at W1's existing `process_page`, the native Decimal
substitution below, and qualification parent/trace tests. The production adapter
will consume the same allocation-owner interface, with its original Request and
immutable connection/fence context. No additional qualification-only protocol or
new Wasm export is needed. Current trace exports can exercise the terms and owner;
production readiness remains false until the authenticated guarded adapter exists.

#### Exact dynamic credits

Use checked `usize` arithmetic on each actual target. The term source constructs
valid schema shapes from fixed stack arrays of `Shape { decode, encode }`; it
must not instantiate `KvDirent`, call `to_value`, create a Vec, or serialize JSON
while calculating the allowance. This removes setup allocations from the formula
itself. A shape's binary/text charge is `4*bytes+64`, array charge is
`4*children*sizeof(SnowpackValue)+4096` plus children, and variant charge is
`4*sizeof(SnowpackValue)+128+68` plus its child (valid selected tags are `b"0"`).
Owned encode capacity is9 per scalar/container/tag header plus payload. The
complete fixed dirent and small-box trees mirror current protocol constructors.
Ciphertext lengths use the full exact-record caps4096/65536 as conservative
upper bounds, preserving the original successful-schema formula.

The returned terms are `parameter_copies`, `list_transport`,
`decoded_container`, `three_role_batches`, `fifteen_row_decode`,
`fifteen_row_append`, `auxiliary_live`, `dynamic_credits`, `workspace_bytes`, and
`control_bytes`. Their fields are fixed-size and Copy. `workspace_bytes` is
`dynamic_credits+auxiliary_live`; auxiliary bytes cannot be spent by a hook.
The terms assert the pinned List maximum row69782/effective rows15 and role
maximum row39/effective rows7. Any catalog/layout drift requires a reviewed
formula update rather than silently selecting different limits.

| Query shape | Parameters | Payload allowance including20 bytes per integer |
| --- | ---: | ---: |
| initial List |4|89|
| public MAC start |5|121|
| private MAC continuation |6|137|
| public time start |5|109|
| private time continuation |6|125|
| seven-role batch, including all unused fixed slots |16|333|

The maximum List transport uses six parameters/137 bytes and15 rows. The three
role queries each use their own sixteen parameters/four columns/seven rows;
UID33 and seven `Option<u64>` result cells are charged separately. Native query
credits remain exactly payload + parameter count*SqlValue + column count*
`(512+(rows+1)*Cell)` + maximum row + encoded page. Wasm query credits remain
old24-byte parameter header + full replacement (`24+payload+5*parameters`) +
parameter Cell vector + encoded reply. Integer parameter widths in this Wasm
formula deliberately overestimate their9-byte encoding. The P1 reply Vec moves
into PageBuffer; it is not copied a second time.

`decoded_container` covers15 typed rows. Per-row decode includes the two explicit
`4*exact_length+4096` reservations, generic dirent/body AST charges, and dirent
exact re-encode capacity+4096. Per-row append includes the existing
`d+n+512+40*SnowpackValue` precharge, three encoded outputs (entry/body/position),
and each output's4096 overhead. The constructors allocate at most26 Value slots
for entry+body, fixed binary/tag copies below512, and the two bounded ciphertext
copies;40 slots/512 remains the existing conservative charge, not a new cap.
Encoder retained-buffer replacement and final output do not consume these
credits. Their W1 owner directly charges the original parent.

#### Native Decimal and unhooked auxiliary audit

Fold a narrow native query_page correction into this group: reuse the reviewed
`scalar::Decimal` stack formatter for Integer binding bytes and returned-integer
canonicality. Copy only the exact decimal slice into the precharged binding
String; preserve old SQL, parameter order, overall charge and error order.
Canonicality then compares borrowed bytes without allocating a temporary String.
This avoids relying on an undocumented generic `to_string` capacity. Do not call
`scalar::bindings` and add its separate charge on top of the existing page charge.
The existing payload20 allowance covers each signed decimal copy. Existing
per-column512 is retained unchanged; it is not a justification for unbounded
SQL-library allocations.

The unhooked Snowpack path/error audit covers both decode and encode:

* `MAX_DEPTH=64` is checked before processing a value; one pushed path may reach65.
  PathSegment is nonzero and between2 and1024 bytes on accepted layouts. Rust1.95
  RawVec starts at4 elements for that layout, doubles, and requests at most128.
  During the last growth old64+new128 can coexist; one original128-slot path
  plus a65-slot error clone is193 slots and bounds both cases.
* A variant path tag is at most31 bytes. The original path and error clone can
  each retain65 tags. `Error::new` clones the path; it does not box the Error.
  `foks_proto::Error` adds heap payload only for the selected wrong-tag case:
  expected `"0"` String1 and found tag Vec at most31. Other selected schema
  errors contain only fixed fields/static strings. None is formatted in page work.
* Generic decode has returned and destroyed its Decoder path before typed schema
  extraction or dirent re-encoding begins. No two Decoder/Encoder path owners
  overlap. A completed or failed diagnostic is consumed inside W1's Copy-only
  scope. Therefore reserve the conservative separate live term
  `(128+65)*sizeof(PathSegment) + 2*65*31 + 1+31`. This includes growth coexistence
  and simultaneous original/error tags without assuming length0 frees capacity.
* AST array/Box/tag/payload and typed ciphertext copies are already preceded by
  their dynamic hooks. Malformed requests that exceed remaining credits fail
  before allocating that requested payload. This does not promise unbudgeted
  native Corrupt/Bounds equivalence for allocation-amplification inputs.

The local installed Rust1.95 documentation source was inspected at
`.../share/doc/rust/html/src/alloc/raw_vec/mod.rs.html` (`min_non_zero_cap`,
`grow_amortized`) and `alloc/sync.rs.html` (`ArcInner`, repr(C,align(2))). The
implementation source gate must pin the toolchain and test these requested-live
bounds with allocator instrumentation, including realloc old+new coexistence.
The auxiliary term concerns requested allocations, not allocator rounding/RSS.
Wasm uses the same reviewed Rust source but its own actual layouts.

#### Control owner and real adapter seam

One Arc allocation contains the parent Arc, fixed terms and fixed atomic state:
occupied/failed flags; nonwrapping generation; local charged bytes; cumulative
attempts/traffic/refusals/first-refusal. Use fixed atomics rather than an accounting
Vec or per-page synchronization allocation. Calculate its exact requested layout
with `Layout::array::<AtomicUsize>(2).extend(Layout::new::<Budget>()).pad_to_align()`
for the pinned Arc header; the two-byte minimum alignment is already dominated
by AtomicUsize on supported targets. Reserve control+workspace from the parent
before allocating the Arc. Arc clones/coercion to an existing Request or Recorder
parent do not create another payload allocation. No new adapter-box allocation
may be omitted from this formula.

The final interface is allocation-only: `reserve_workspace`, `reserve_retained`,
`failed`, `fail`. A native implementation on the existing Request holds its
original pools/reservations; it does not create a child Request. The existing
Request reservation vector/control bookkeeping is request-owned, outside page
reset, and needs its own bounded reservation as part of adapter entry admission.
The W2 core can be implemented/tested with the fixed qualification parent first,
while the actual Request bridge is a narrow explicit server integration gate.
Do not expose arbitrary reset or generic result-returning scope APIs. Only the
private fixed page callback installs these credits and only Copy exits it.

The parent and local failure latch must cover a hook refusal, counter overflow,
invalid occupancy, reserve-control failure, and fallible allocation failure.
Unwinding restores the prior allocation hook before releasing occupancy; actual
Wasm traps retire the instance after the outer export exits. SQL counts, deadline,
cancellation, event ordinals and sticky response state never reset. Global
infallible allocation failure is still abort/trap, not a recoverable local error.
The allocation hook itself swaps a fixed TLS Option<Arc>; it does not grow a stack
Vec. Retained growth bypasses the page hook via W1's existing owner.

SQLite/rusqlite statement/cursor/error/cache internals and JS bindings, BigInts,
rows, encoded ArrayBuffers are host-owned separately; this term never reserves
them. Connection caches and the full grown Wasm linear memory keep their host
reservation across page exits and idle periods. Entry context/auth/authority
state, parent assembly/decode, retained List/frame/output and actual sync/deadline
remain distinct from this page-only allowance. S3c closes only its two preserved
state decoders, not generic authority command/outcome JSON.

Review/test inventory before reset is enabled: compare these allocation-free
terms to prior valid-shape formulas on native/Wasm; preserve all existing58 trace
cases and exact hashes; full15-row/three-role maximum; native Decimal extrema and
unchanged query charge/error ordering; malformed depth65/tag31/truncated u32/large
scalar-array/late typed wrong-tag cases under a requested-live allocation tracker;
local refusal after exhausted dynamic credits while auxiliary cleanup still fits;
parent reservation and cumulative traffic observed separately; wrong phase,
retained refusal, counter overflow, unwind and actual trap with no reentry/refund.
Numeric term results, claimed parent working-pool savings and concrete reset
implementation require root and independent source review. No source in this
proposal has been compiled, no W2 allowance selected, and no W2 tests have run.

One trace-integration precision for this gate: replacing the ambient hook must
not erase its logical event stream. Under the existing qualification feature,
PageBudget forwards each local admission attempt to a fixed, non-allocating
Recorder observation method through the existing ACTIVE TLS owner; it does not
call the parent's reservation method for each page charge. Recorder combines the
local admissibility bit with its existing fail_at injection, preserves the same
invocation-wide event ordinal/phase/origin, and records a refusal when either
condition refuses. No observer Arc or per-event container is allocated. The
observer must return before any local failure calls parent.fail (no nested lock).
Workspace/control reservations have separate fixed reservation counters, not
extra Snowpack allocation-attempt events that would silently renumber fail_at.
Retained/frame/entry events still reach the original Recorder directly. Native
parent-pool tests inspect actual reservation count/bytes independently of this
logical traffic stream. The production build has no qualification callback and
no second parent reservation for each local charge.

### Next authority integration proposal: borrowed context to owned List (review pending)

S3c committed `6e6b275d` closes the two persisted state decoders needed by the
selected read. The remaining `authority/mod.rs::encoded/json/domain/binding/
stored/finish` allocations belong to initialization, controller operations or
mutation receipts. `wasm.rs::{run_authority,run_personal,read_personal}` still
parse generic qualification JSON and serialize generic replies. They are not the
proposed guarded List ABI and must not become its entry path. No further general
mutation-codec rewrite is proposed for this milestone.

The next bounded group should join the existing pieces in one private core
backend, ready for the restricted authenticated adapter: borrow one decoded
`HostListContext`, perform the selected authority gate and exact-microsecond
personal resolution, then call the W1/W2 owned List helper under the same original
request owner. No peer/fence clone, generic Invocation/Reply JSON, new frontend
DTO, new Wasm export, transaction constructor or transferable guard is needed.
This is a proposal only; implementation waits root and independent plan review.

**Concrete interface and ownership.** Add a crate-private selected-List entry
next to `authority/personal.rs`, taking `&mut impl Sql`, `&HostListContext`,
`&personal::ListRequest`, and the existing retained/request-owner Arc. Return
owned List bytes plus fixed-size authority-observation metadata, or a typed
error distinguishing authority/infrastructure failure from later domain failure.
The original context/input reservation remains held outside this borrowed call;
the owner continues through frame construction and physical publication. The
helper cannot COMMIT, sync, publish or mint a public error capability. Its typed
ListRequest is the backend input, not evidence that the public authority field
was parsed or authorized. Keep the helper crate-private and exercised by native
in-crate qualification; promote it into the production factory only with that
factory's reviewed connection/transaction capability.

Use the current state's revision from its owner-checked row, never a new
frontend revision parameter; this preserves the approved single-call context
format. The authority intent revision remains distinct from schema59's read
incarnation/revision. Do not fabricate a dependency observation from the former.
A fixed returned observation is tentative transaction data, not a guard and not
a proof reusable after leaving the transaction.

**Common gate and exact time.** Extract only the existing validation/state-load/
clock-floor sequence in `authority/mod.rs::run_resolved` into a private borrowed
helper so existing request behavior and the new selected path share it. Preserve
existing deadline-range checks, S3c load ordering, floor rejection and clock SQL
write, then exact fence identity/activation/owner/epoch checks. The new selected
path obtains the current revision from that row; existing Prove keeps its caller
revision comparison unchanged. Any error requires outer rollback unless the
future guarded adapter explicitly classifies an allowed public outcome.

Refactor PersonalResolver's private time representation to accept an exact u64
microsecond sample. Existing personal/point qualification entrypoints retain
their checked Number-milliseconds conversion at the same original failure point;
new selected entry uses `context.now_micros()` directly. Authority floor/deadline
math still uses `context.now_ms()` and execution_deadline_ms. Retain existing
personal authentication/profile, bounded digest and admitted namespace/role
behavior rather than silently changing the established principal semantics.
Credential recheck/profile and parent authorization inside the owned List helper
remain unchanged; removing repeated checks is outside this group.

**Public adapter boundary is explicit.** The new backend does not call the
post-authority public parser early. The eventual adapter must preserve native
`services/kv.rs::resolve_authority` (including the original outer authority field
and shared-key failure precedence) before `foks_rpc::arguments::
decode_kv_list_arguments`, then invoke this backend without treating typed input
as an authorization proof. Exact raw public argument parsing, CandidatePublicError
construction and raw RPC framing are not replaced by generic PersonalError or
JSON. This backend group must not claim complete native public-status parity;
existing native List oracle remains the required adapter gate. If this layering
would force a duplicate authentication or alter error precedence in the concrete
factory, root should choose the factory call boundary before coding rather than
add a second unguarded dispatcher.

**C4 seam.** The enclosing factory owns the immutable host capability, actual
connection settings/name-resolution seal, schema proof/cookie, writable outer
transaction, original deadline and physical instance lifetime. It admits the
restricted profile and reads the schema59 dependency in that transaction. The
backend receives that already-owned Sql borrow; it does not establish those
properties itself. COMMIT/durable sync and a fresh sealed post-sync check produce
the previously planned PendingFrame→PublishableFrame transition. An authority or
schema failure, deadline, trap or uncertain transaction outcome discards output;
no old admission survives transaction exit. This proposal does not implement C4
or select the external ownership/fencing provider.

**Owned files and validation.** Proposed production files are authority/mod.rs,
authority/personal.rs and one private selected-list child module; tests live in
that child. No W2 page/list/encoder/native Decimal/trace-owner files are changed.
Before implementation, confirm the retained-owner type exposed by W2 so no
adapter Box or second accounting owner is introduced. Tests use real native SQL
transactions, issued certificates and decoded existing strict contexts: expiry
one microsecond before/at/after a boundary (including within one millisecond),
wrong identity/activation/owner/epoch, state-floor/deadline rejection before List,
rollback of floor writes on failure, original qualification error ordering,
context/fence borrow identity, owner retention through output/frame drop and
admission refusal. Compare successful backend raw List bytes to the existing
native oracle for valid personal requests; do not claim this proves outer-field
or malformed-argument precedence. Preserve existing Rust/Go personal authority
vectors and actual celld regression suites. No new dynamic JSON fixture format
or export is justified merely to test this private composition. Final adapter
parity, connection seal and publication tests remain named serving gates, not
implicit consequences of this group's tests.

### Guarded List factory extension — concrete integration design for review

Root selected folding the preceding composition into the factory itself. The
backend-only commit proposal is therefore superseded: no separately shippable
unguarded dispatcher is planned. This section is a design gate, not permission
to implement or enable serving. R2/R3 readiness and C4's connection qualification
remain mandatory prerequisites, not feature-flag assumptions.

**Native ownership choice.** Place the native physical owner in a new internal
`foks-server-db/src/hosted_list.rs` module. A `HostedListDatabase` consumes an
already-open `Database`; ordinary repository access is unavailable after that
move. It owns the connection, its real handle-bound schema verifier, path identity
and nonwrapping operation generation. No connection/transaction getter, arbitrary
closure, migration/configuration method, or raw SQL loan is exposed. This is an
opt-in owner for an explicitly imported/admitted hosted database, never an
implicit conversion of the ordinary standalone server database. Creation cannot
initialize private authority rows or silently enroll an unqualified database.

The dependency direction is server-db→metadata-core(native), behind an explicit
hosted-List feature, and server-db→foks-rpc for typed status/frame handling.
Metadata-core's server-db edge is dev-only today; the production graph remains
acyclic, but the existing reverse dev edge/feature unification must be checked
with locked Cargo metadata and the relevant tests before accepting the manifest.
If that graph is not supported, move the affected core integration fixtures to
testkit rather than expose a raw connection to avoid the dependency. No new crate
or public raw-connection adapter is proposed merely to bypass the graph gate.

Server `hosted_list.rs` owns the actual TLS/frontend provenance and original
`net::memory::Request`, deadline/cancellation and writer/execution permit. The
request remains the original owner; it is not reconstructed as a fresh child
request. It installs existing budget hooks and lends the DB owner one fixed
operation input. The W2 parent interface is implemented directly for Request,
with its reservation bookkeeping explicitly admitted before growth; no uncharged
adapter Box or accounting Vec is permitted. The server route is left disconnected
until the final gates below. Qualification may call the same owner through
feature-only testkit plumbing, never through a different SQL authority path.

**Proposed API/state contract.** The names below are proposed internal API names,
not transferable public proof constructors:

* `HostedListDatabase::admit(database, imported_profile, deployment_binding)`
  consumes the DB and validated R2/R3 evidence. Evidence is bound to its actual
  identity/incarnation, descriptor/artifact versions and current dependency; raw
  caller-provided hashes or a boolean cannot construct it.
* `execute_list(&mut self, authenticated_context, original_argument, sequence,
  original_request_owner)` performs one complete synchronous transaction and
  returns `PendingList`, or an infrastructure/refusal result. No callback can run
  between seal and transaction completion. Core execution uses borrowed strict
  context and the existing raw public argument, not generic authority JSON.
* PendingList privately retains frame bytes, request/instance owner, operation
  generation, committed transaction observation, original sequence/deadline and
  exact fence. It offers no public bytes/deref/into_vec. The matching owner's
  `finish_pending(pending)` performs the platform durability completion and fresh
  sealed ownership/deadline check, producing a publishable owned frame. It cannot
  accept a pending value from another connection or an earlier reused generation.
* Publication lends bytes only to the existing physical response writer while
  retaining all owners. Cancellation, disconnect and late completion drop the
  candidate only after actual work exits; they never release permits merely
  because a future was abandoned. Drop without finish disposes pending output.

For native SQLite, successful COMMIT under the verified durable policy is the
platform's local durability step; it is not an S3/availability-zone guarantee.
For celld, successful transactionSync completion is followed by the existing
platform durability sync and then fresh sealed verification. An interface must
not call both implementations identically durable without recording this
qualification difference. Pending output is already committed when sync fails;
read retry is safe but no stale success may escape.

**Exact authority/parser ordering.** Freeze a shared pure outer-auth parser in
foks-rpc from `services/kv.rs::resolve_authority`, preserving its full argument
Snowpack decode, first-field/header distinction and exact status strings. Split
only its SQL-free stage: native keeps its clock read at the existing point after
auth-array shape validation and before the auth-variant match. For the hosted
route, the trusted context's original captured micros supplies that clock value;
owner/deadline/schema admission failures remain nonpublic refusals.

After current certificate authentication, the public personal branch must retain
this sequence: outer argument/auth shape → clock sample semantics → personal
variant selection → active credential owner → user-authority existence/current
device role and full-history validation semantics → post-authority List parser →
parent/key permission → cursor DB-range conversion → List pages. The shared
post-authority parser deliberately ignores field0; it cannot replace the first
stage. Team auth, malformed team tokens and unsupported profiles require an
explicit restricted-profile outcome policy; this first route may refuse valid
team requests as unsupported rather than claim personal parity, but it cannot
publish a personal success by ignoring a team token. Literal malformed-input
statuses must be compared to the native oracle, not remapped generically.

Native `user_authority_inner` also reads all device rows, shared-key rows and
stale-parcel role rows, plus chain/tree/root widths. Consequently existing
PersonalResolver plus targeted key queries alone is insufficient for full error
precedence. The admitted profile must certify the complete semantic validity
actually used by these reads, not merely SQLite record sizes. R2/R3's readiness
contract must explicitly cover each returned column's integer/blob/type/width
conversion, stored device/key role validity, anchor joins and stale-role query
conversions; mutation/restore/import gates invalidate it atomically with the
existing dependency revision. A malformed or unclassified historical row causes
profile refusal before public results. If that semantic evidence is absent or
stale, execute_list refuses; it must not perform an unbounded per-request history
materialization or silently skip native validation. This precise validator roster
must be agreed with R2/R3 before coding this shortcut. The native full-history
oracle remains unchanged and independently exercises corruption precedence.

**Transaction sequence and provenance.** With exclusive physical ownership,
verify construction-only configuration then BEGIN one writable transaction.
Inside it read all C4 settings, TEMP/attachment absence and exact approved
schema/artifact profile, using the actual same connection. Check R2/R3 evidence
and schema59 dependency. Run the existing selected authority state/floor/fence
checks without cloning the context and derive its current intent revision from
SQL, never from a request. Use exact context micros for certificate expiry and
millis only for authority-floor/deadline math. Perform the public ordering above,
then W1/W2 owned List and existing raw RPC framing. Check unchanged dependency,
schema cookie, ownership and deadline before COMMIT. Every statement, including
entry/exit checks, shares the original 2048-call cap and deadline; no probe gets
a fresh counter. The exact fixed SQL schedule must be counted before a route is
qualified (including R3 checks), not inferred from the old 1160 domain-only count.

Only a typed public-domain outcome reached after the current gate may become a
candidate FOKS error. Schema/authority/profile/admission failures, malformed
private context, SQL failures, traps and uncertain commit all discard. A generic
PersonalError/RpcStatus conversion cannot grant publication. Known domain-error
clock-floor commit policy is explicit in the owner state machine. At transaction
exit its admission dies; post-sync proof is a new sealed transaction, and any
mismatch invalidates pending output. No checking-empty-TEMP-after-callback
substitutes for the prohibition on foreign callbacks and raw aliases.

**Celld adapter and code inventory.** Use a dedicated deployment-owned module
`tools/foks-metadata-host/guarded-list.mjs` and its static worker composition,
not the qualification Kernel subclass as the production boundary. Private storage
ownership is enforced by whole-worker reference audit: no retained alias,
reentrant callback or generic SQL endpoint. It uses the already approved fixed
FKLQ input and strict context import, a guarded artifact with an exact export
allowlist, and the same pending-generation state machine. Existing spike workers
remain test harnesses. The module's introduction needs the exact import/export
and platform-build audit already specified in the read-bundle plan; no new JSON
frontend format is introduced.

Expected owned source groups are server-db hosted_list/connection/lib/Cargo;
metadata-core authority borrowed gate/exact-time resolver plus a guarded List
orchestration module and its feature/exports; rpc outer-auth argument parser;
server hosted_list and original Request integration; dedicated celld host module;
and testkit factory/native oracle tests. Native `services/kv.rs` changes only to
call the shared pure parser while retaining its current SQL/clock path. W2's
page implementation stays separately owned; any required public owner-trait
visibility change gets a narrow union review. Precise path/hash freeze follows
agreement on the R2/R3 validator evidence and factory API, before any source work.

**One integration acceptance gate.** Require real native factory SQL tests and
actual celld factory tests, not merely a fake exclusive host: malformed outer auth
and pagination precedence, delegated/current/revoked credentials, full-history
invalid rows, exact-micros expiry edges, unsupported team/profile refusal,
namespace/parent denial, original sequence/status/raw bytes, admission/retained
refusals, wrong owner/activation/incarnation, settings/TEMP/ATTACH interference,
reentry/raw-alias attempts, clock-floor rollback, commit/sync/fresh-proof faults,
wrong pending generation and cancellation during physical output. Preserve pinned
Go/Rust vectors and native oracle comparisons. R2/R3 import/readiness and C4
engine/registered-function qualification must pass before a serving switch; this
group may be committed with the route unreachable, but never labeled production
ready from test-only construction or from size descriptors alone.

Factory/R2-R3 coordination: storage confirmed R2 yields only
`StorageScanComplete` (storage types/nulls/bytes/physical records and immutable
schema/artifact binding). That receipt cannot construct the factory's semantic
readiness. R3 must add the full native authority-conversion/error invariant roster
and bind it to the imported artifact plus subsequent typed-writer and dependency
coverage. Neither same-size valid records nor a matching descriptor digest prove
that historical authentication data is semantically admissible.

Two constructor details remain explicit source gates: the native physical owner
also receives the deployment's existing exclusive writer/lifecycle lease and
validated file identity; consuming Database alone does not manufacture that
lease. Native qualification and celld use explicit target-tagged schema/platform
contracts. Never omit celld's pinned platform objects or relax ExactUnion merely
to make a native fixture pass; each artifact/contract pairing must be separately
identified and checked. The crate dependency graph and platform contract selection
are plan checks, not yet validated implementation facts.

### Factory subgroup F0: native-used outer-authority parser (committed 9285f2ca)

Root approved the factory direction subject to its remaining exact gates and
selected this independently useful first subgroup. F0 refactors the real native
call site now; it does not wait for R3 and does not claim to implement readiness,
a hosted transaction owner, exact-time resolver or publication. All factory gates
above remain. No source or tests have been changed for F0.

Freeze this ownership API in `foks-rpc::arguments`:

* `KvAuthorityArgument::decode(argument: &[u8]) -> Result<Self, RpcStatus>` owns
  exactly the one existing Snowpack decode result. Private fields are the root
  Value and a fixed direct/header path discriminator. It validates only the
  current pre-clock operations: outer array, nonempty first field, first-field
  array, the direct-vs-header selection, header nonempty when selected, and final
  auth-array shape. It does not classify the variant or inspect team token bytes.
* `classify(&self) -> Result<KvAuthoritySelection, RpcStatus>` runs the existing
  post-clock match and returns only `Personal` or `Team([u8;16])`. It preserves
  exact unsupported-auth, token-not-binary and wrong-width status text. No
  namespace, role, principal, readiness or authorization proof is returned.
* Both public types have private data where applicable; no Clone, Serialize,
  raw-Value getter or borrowed interior result is added. The selection is a
  fixed Copy enum. The decoded owner lives through native resolve_authority's
  existing SQL branch and drops on its return, as the current root Value does.
  Input bytes are borrowed only for decoding, never copied into a second raw
  argument buffer. No full-AST clone or new per-field container is introduced.

Native `resolve_authority` becomes exactly: decode owned prefix; call existing
`clock.now_micros()` once with its existing TransactionRetry mapping; classify;
then run the unchanged Personal/Team SQL and role/key logic. Classification is
not folded into decode: unsupported variants and malformed team tokens must
still lose to a clock error, while malformed pre-clock structure must still win.
The native server's public post-authority List parser stays in its current later
position. No SQL, team-token hash, role comparison, key-generation accumulation,
permission string or Principal construction changes. Since resolve_authority is
shared by all KV routes, F0 is a native KV parser refactor, not a List-only route
switch. It cannot silently reject a native-valid team branch.

The future factory uses the same two stages with its already captured exact
context microsecond sample between them. That use is not implemented by F0.
It must retain the full factory error-provenance policy: unsupported valid team
requests are a typed restricted-profile refusal, never personal authorization;
private infrastructure failure is never synthesized into public BadArguments.
The name classify does not itself prove a clock was sampled or SQL authority was
resolved; caller sequencing is covered by the actual native tests and future
factory ownership code.

Owned source list is five paths: `foks-rpc/src/arguments/mod.rs`, new
`arguments/kv_authority.rs`, new `arguments/kv_authority/tests.rs`,
`foks-server/src/services/kv.rs`, and
`foks-server-testkit/tests/native_list_oracle.rs`. Keep any native-only clock
sequencing unit tests inside the existing services module to avoid adding a new
production test hook. No dependency, lockfile, schema, catalog or Wasm export
change is planned. Existing foks-rpc native/Wasm builds cover the shared module.

Tests must use fixed expected statuses and encoded arguments, not merely compare
two paths both using the new helper: malformed root/empty/missing-header/auth
shape; direct and wrapped personal/team success; unsupported type/tag/arity;
nonbinary/15-byte/17-byte team token; ignored unrelated fields and valid token
copy; malformed full trailing Snowpack. The native clock-failure matrix proves
pre-clock shape failure does not call the clock, while malformed post-clock
variant/token returns TransactionRetry from the clock before variant errors and
never touches SQL. On an ordinary clock, variant errors remain exact literals.
Extend the existing real authenticated TCP/native List oracle matrix for direct
and wrapped forms and wrong outer auth combined with bad pagination, keeping
original certificate-before-public-argument behavior. Existing successful team
KV conformance and pinned Go KV tests protect the shared call site's non-List
scope. All original22 List cases remain. Source review must compare the moved
syntax against the original resolve_authority block and verify no accidental
secret-token Debug/log formatting; test results do not claim new memory bounds.

F0 author plan review: the only intended observable change is code location.
Keeping the whole decoded owner and borrowing it for fixed selection avoids an
unreviewed lifetime change; leaving the clock between stages preserves the
otherwise easy-to-miss error precedence. The later concrete factory still needs
R3 semantic evidence, exact C4 configuration/inventory, actual exclusive lifecycle
lease, target-tagged contracts and original Request reservation bookkeeping.
Root and independent plan approval are required before implementation.

### W2 source freeze and qualification entry

W2 implementation is frozen in the isolated `metadata-page-allowance-validation`
worktree, eleven paths listed in `/private/tmp/foks-page-allowance-manifest.txt`
with exact hashes in the adjacent `-sha256.json`. Author, root and quota source
reviews passed before any test. The tree then advanced from W1 `345247ce` to
`b837bd0d` without changing any of those eleven hashes, preserving S3c and R1.

The private owned List path reserves one checked workspace plus its fixed Arc
control allocation. Its Copy-only page scope restores the original hook before
releasing occupancy; only local credits reset. Attempts, traffic, refusal,
generation, parent failure, SQL and deadline counters do not reset. Unhooked
Snowpack diagnostic/path headroom cannot be spent as dynamic credits. Retained
encoder growth and output ownership remain with the original parent. The public
Ambient helper retains its original cumulative admission behavior. Native page
integer formatting now uses the existing stack Decimal while preserving exact
charges and canonical bytes. Trace workspace reservations are separate from the
original logical event stream and do not renumber fault injection.

The source includes requested-live allocator overlap tests for malformed page
schemas, source/toolchain pins, actual native decimal extrema and charge order,
sticky control/local failures, and late-row role-query precedence. These are not
RSS or complete request/engine budgets. The Arc reservation still follows Rust's
process/instance disposal policy for unrecoverable allocation failure. Parent,
entry/authority, frame/output, JS/SQLite and resident linear-memory owners remain
separate. No production adapter or restricted-profile admission is enabled.

Qualification began only after both reviews. The first focused allowance gate
passed eleven filtered tests without source correction; full native and immutable
Wasm/runtime gates are in progress. Final results and any failures will be recorded
separately; this entry does not claim the remaining gates passed.

### Factory F1 prerequisite/API freeze (design only; distinct from earlier framing F1)

The following refines the factory extension after reading concrete R2 and native
owner APIs. No factory source change is authorized yet. F0 is independently
reviewed and does not confer any of these properties.

| Required input | Owning component and meaning | Cannot substitute |
| --- | --- | --- |
| StorageScanComplete | R2 server-owned immutable full-package/storage scan | semantic readiness, a path/digest tuple |
| ImportedListReady | Proposed R3 server-private owner of freshly imported DB, key/chunk closure and complete native authority-semantic evidence | R2 receipt alone, row-size success, frontend flag |
| Exclusive native lifecycle | Existing server DatabaseWriterGuard plus required storage/import lifecycle leases, rechecked identity | moving Database, an advisory bool, lock acquired on another inode |
| Connection configuration/profile | Compiled target-tagged fixed contract, C4/C4a qualified engine and extension inventory; re-established per transaction | user-supplied settings/hash, cached main schema cookie alone |
| Frontend context | Server-private authenticated connection/peer provenance, exact HostListContext data borrowed under original request | deserialized asserted principal or context-as-capability |
| Memory and execution | Existing Request Arc, its pools/physical permit and immutable deadline/cancellation | child Request, fresh per-probe timeout or refunded page reservation |

**Dependency-safe ownership split.** DatabaseWriterGuard, package/key closure
and Request currently live in foks-server, not server-db. Therefore the complete
`HostedListFactory` and `ImportedListReady` remain server-private. The lower
server-db module owns only the consumed Database and fixed closed transaction
operation. It cannot accept a server-owned type through a reverse crate edge,
and its result does not claim TLS provenance, complete readiness or publication.
The server composite owns DB before its lease fields in drop order and retains
all leases until the connection/worker/pending output have actually stopped.
This supersedes interpreting the earlier `HostedListDatabase::admit` sketch as
a complete lower-crate readiness constructor. Moving writer-lock APIs into DB
solely for this feature is not proposed.

Proposed server-private calls are
`HostedListFactory::from_import(ImportedListReady, DeploymentContract, NativeLeases)`
and `execute(AuthenticatedListCall, Arc<Request>) -> PendingList`. There is no
public constructor from booleans/raw hashes. `ImportedListReady` is noncloneable,
not serializable, and consumed exactly once; it internally owns the lower DB
handle created during R3 import. `AuthenticatedListCall` retains original raw
argument and sequence with the existing connection's verified peer and original
deadline. It is not constructed by parsing a public HostListContext string.
Borrowing strict context into core preserves data format without confusing data
validation with origin proof. Closed lower operations expose no SQL or arbitrary
callback and return tentative bytes/status with coordinate metadata only. A
caller outside the server private composite gains no publication capability.

**Readiness freshness.** R3's semantic witness identifies the exact fresh import,
profile/validator versions, target artifact contract and schema59 incarnation/
revision at completion. A subsequent typed writer must preserve the certified
semantic invariant and restricted postimage/physical bounds, or atomically
invalidate readiness with its dependency-changing transaction. A bare larger
revision is not evidence that a writer preserved it. Until every relevant writer
is covered, the first factory is read-only with respect to public domain mutations
and refuses reopening after an unsupported mutation; the clock-floor update is
explicitly excluded from the domain dependency and must preserve the witness.
Restore creates new evidence/incarnation; no raw-clone/rollback witness reuse.
R4's postimage/query-intermediate work is needed before permitting mutation
routes, not silently assumed from R2/R3 import success.

R3's exact native authority-semantic roster is the source-defined conversion
closure in `read.rs::user_authority_inner`: head sequence/root epoch unsigned,
link hash/tree location/root hash fixed widths; all device ID/subkey/blob cells,
active/role/visibility integer handling and current selected-role validity; every
shared-key role/generation and verify_key/exact_hepk storage conversion plus
`services/kv.rs::key_generations` stored_role validity; stale-parcel query role_type
unsigned and visibility i64. Stale roles are not themselves fed to stored_role in
native code. Missing head/tree/root joins retain None/permission, not corruption.
Any stronger imported-profile exclusion is named as such. Blob fit is not DER,
HEPK or other cryptographic validation. Storage's R2/R3 plan now records this
closure; the actual R3 validator/source gate remains outstanding.

**Original Request accounting.** W2's owner interface reserves one page/control
allowance through the original Request; retained growth/frame and entry decoders
continue through that same request's designated pools. The current Request stores
one OwnedSemaphorePermit per accepted charge in a growing Vec. Proposed bounded
replacement is five inline Option<OwnedSemaphorePermit> slots keyed to the actual
working/small/output/control/bulk pools. Merge only same-pool permits using pinned
Tokio1.53.1 merge, with checked combined counts before its internal u32 addition.
Current SessionLimits already caps byte pools at u32::MAX and semaphore limits;
retain explicit checks rather than relying on that fact alone. Failure drops only
the just-acquired permit and latches the original Request; earlier permits remain
held. Pool fallback order and small/bulk classification remain unchanged.

This removes per-charge bookkeeping allocation without inventing a new request
or refund. Request/Arc/Mutex and fixed control storage must be covered at original
request admission (or separately reported existing bounded connection-control
ownership); it is not charged again per page and not hidden in SQL credits.
Qualification asserts pool availability and drop ordering under success, refusal,
cancellation-before-start, detached physical work and pending publication, as well
as failed checked merge and wrong-pool prevention. This source change would affect
all users of Request, so its existing network/transfer tests are required even
though it is motivated by the factory.

**Pending operation state.** The server owner and celld owner both remain Busy
through pending output/durability/fresh verification. They reject another domain
entry rather than reuse the instance while a prior result awaits sync. Pending
contains the nonwrapping operation generation, exact context binding, committed
observation, original sequence/deadline and original memory owner. Finish consumes
it; duplicate, wrong-owner, expired or invalidated completion cannot produce a
publishable frame. A canceled waiter does not end physical ownership. Native
connection confinement stays in its synchronous worker; an async Pending handle
never contains an unsynchronized raw Connection. The fresh post-sync sealed check
uses a new transaction and cannot revive the old proof. A native COMMIT/local-sync
receipt and celld COMMIT+platform-sync receipt are different target-tagged types
or enum variants with separately qualified policies, never a caller `durable=true`.

The exact lower-core trait visibility, worker return/drop type, fixed C4 SQL
schedule and R3 witness constructor are still source/API review gates. This freeze
chooses their ownership and prohibits shortcuts; it does not declare those
unimplemented APIs already available. No factory code/tests/serving switch follows
from F0's parser validation or from this design discussion.

### Factory F1 concrete subgroup: fixed Request permit bookkeeping (committed 3d992330)

Root requested this bounded subgroup after F0. It changes only
`crates/foks-server/src/net/memory.rs` and tests in that module; no numeric limits,
configuration defaults, pool sizes, request classification, timeout, protocol,
SQL, factory readiness or serving switch changes. It does not yet implement W2's
Request trait bridge. It removes allocation proportional to accepted charge count
from the already-used Request owner, providing the fixed bookkeeping needed by
that later bridge. Author, root and independent plan reviews passed. The isolated one-file implementation is frozen for two source reviews before tests.

Replace `Mutex<Vec<OwnedSemaphorePermit>>` with a private fixed `PermitSlots`
containing five `Option<OwnedSemaphorePermit>` fields. A private Copy `PoolId`
selects both the actual pool and exactly one slot; callers cannot provide a
semaphore Arc independently of the selected slot.

| PoolId/slot | Existing source | Existing acquisition policy |
| --- | --- | --- |
| Working | pools.working | Snowpack/working admission first |
| Small | pools.small | fallback for eligible small working or noncontrol output only |
| Output | pools.output | noncontrol response first |
| Control | pools.control | control/status output, no Small fallback |
| Bulk | pools.bulk | one permit before bulk working admission |

Input's existing `_input` permit remains a separate field, never merged into any
of these pools. Zero-byte accepted reservations may initialize/merge a zero-count
slot; they acquire no capacity and cannot allocate a bookkeeping Vec. Keep all
five field owners until final Request drop, including zero-count ownership.

`reserve_from` still converts the requested count to u32, tries the selected pool
without awaiting, then takes the same request bookkeeping mutex. The new helper
checks Arc::ptr_eq of the acquired permit's `semaphore()` against the selected
pool, and of an occupied slot against that pool, before using merge. It checks
`existing.num_permits().checked_add(new.num_permits()) <= u32::MAX` explicitly.
Pinned Tokio1.53.1 `OwnedSemaphorePermit::merge` then consumes the new permit,
adds its u32 count and zeros the consumed count, so Drop does not release an
intermediate reservation. No allocation occurs in this merge. Existing configured
pool ceilings make legitimate count overflow impossible, but the helper must
reject it rather than panic/wrap if misused or if future configuration changes.

Only after all checks pass does an empty slot take the permit or an occupied
slot merge it. Poisoned mutex, wrong pool or overflow leaves the existing slot
unchanged, drops the just-acquired permit to roll back that attempt, and latches
Request.failed. Use an internal distinction between ordinary pool-capacity
refusal and bookkeeping failure so fallback cannot hide an invariant failure.
No existing reservation is split, forgotten or eagerly released on either error.
A successful later control/status reservation does not clear a prior sticky
failure; existing fallback-error behavior remains supported.

**Multi-pool ordering and rollback.** Preserve working→Small and Output→Small
fallback exactly; Control never falls back. A capacity-refused first pool acquired
nothing. If its permit was acquired but bookkeeping failed, only that permit is
released and the operation fails without trying another pool. If a later
independent reservation fails, all earlier slots stay held. In particular,
`admit_working` currently acquires Bulk before working; if working then fails,
the accepted Bulk permit remains with Request until Request disposal. Do not
change this into an eager refund or claim atomic all-or-nothing admission across
those two calls. Existing unstarted/detached physical work ownership decides when
Request is dropped. Tests explicitly distinguish failed-attempt rollback from
releasing earlier successful multi-pool reservations.

**Lifetime and accounting.** Pools::request still creates one Request Arc, and
Request::sync/run, TLS restoration, RetainedWork/RetainedValue/RetainedFuture and
physical response writer ownership remain unchanged. No new Arc/child Request
or per-reservation Box is introduced. The fixed slots may increase the Request's
fixed layout relative to a Vec header; record actual target size before/after.
This subgroup removes variable bookkeeping traffic, not the existing Request/Arc/
Mutex control allocation itself. Do not claim complete pool/RSS admission from
this change. All permits release exactly once on final Request destruction,
after retained payloads/physical work, including disconnect and late completion.

**Tests and gates.** Extend current memory-module tests to reserve repeatedly
through all five mappings and assert exact pool availability, per-slot merged
counts and constant five-slot storage; zero counts; eligible/ineligible fallback;
control refusal without Small usage; just-acquired permit rollback on poisoned
mutex/wrong-pool helper input; checked overflow without mutation; and Bulk-held
until-drop when later working admission fails. Overflow arithmetic gets portable
boundary tests; an actual >u32 combined-capacity fixture is only feasible on a
native target whose Semaphore limit supports it and must be labeled accordingly.
Wrong-pool injection is test-private and must exercise refusal before Tokio's
panic, not catch a merge panic as normal behavior.

Retain/run existing `unstarted_work_and_future_drop_payloads_before_allowances`,
`bulk_saturation_preserves_small_request_headroom_and_failure_is_sticky` and
`slow_socket_retains_response_capacity_and_control_errors_still_fit`. Add a
barrier-driven detached-worker case: drop the waiter/outer Request while the
physical worker is stopped inside owned work; every acquired slot remains held
until the worker's actual output/payload exits. No sleep-based ownership claim.
Test that output/control fallback success after an earlier refusal preserves the
failed flag. Run full native server unit/network tests relevant to memory and
response writes plus strict all-target lint; code review checks every reserve_from
call and fallback operator. No Wasm or catalog rebuild is warranted for this
server-only bookkeeping change. Source freeze and author/root/independent reviews
are required before any tests.

F0 validation began on the exact five reviewed hashes at b837bd0d: focused outer-authority parser tests and the full RPC suite passed; native oracle, shared Team/Go paths and strict lint remain in progress. No factory serving capability is enabled. F1 implementation is independently isolated at the same base; no F1 builds or tests have run.

Factory F1 final one-file source passed root and independent review, including the exact preceding Request layout surrogate used only for target-size qualification. Frozen manifest/hash: `/private/tmp/foks-request-permit-manifest.txt` and `/private/tmp/foks-request-permit-sha256.json`. No tests have run; F0 retains the shared Cargo slot until its compatibility/lint gates complete.

F0 final validation is PASS on all five unchanged reviewed source hashes at
b837bd0d: focused parser 3, full RPC 112, native oracle 1 test containing 32 exact
TCP/helper vectors plus 8 injected-clock assertions, Team KV isolation 1, pinned
Go live 3, native all-target strict lint, RPC Wasm strict lint, formatting and
source/hash checks. The focused tests are included in the full RPC total.
Evidence: `/private/tmp/foks-outer-auth-results.json`; individual log paths are
recorded there. No source correction was required during validation. This is
native parser compatibility evidence, not factory readiness or a serving switch.

W2 first native gates passed without implementation corrections:199 active full
metadata tests (three intentional exporter ignores),11 allowance-filter tests,
strict native all-target lint, one real authenticated native List oracle, and the
fresh real-certificate58-case trace export. Default and trace Wasm strict lint and
release builds passed, each declaring18..1024 memory pages. The target formulas
produced native dynamic12,559,716 + auxiliary8,694 =12,568,410 workspace bytes
plus168 control; Wasm dynamic12,435,975 + auxiliary6,378 =12,442,353 workspace
plus104 control. These are requested-allocation terms, not RSS.

The first actual-Wasm trace run passed all58 parity records but failed one of44
harness tests. The preserved failure was an obsolete single-refusing-phase
assertion: diagnostic-only rerun proved Decode event74 refusal followed by the
sticky owner's Roles parameter refusal at75 (one attempt, zero admitted bytes),
before role SQL. Root and quota approved a narrow test design that accepts only
this exact secondary sequence, retains the primary site and all zero-output,
state/reuse assertions, and leaves implementation unchanged. Final source review
and full harness rerun remain pending at this entry. The independent25-test
scalar/page host gate passed. Initial and diagnostic reports/logs are retained
under `/private/tmp/foks-page-allowance-trace-a` and `-trace-diagnostic-b`.

F0 committed as `9285f2ca` with the exact five validated hashes. The evidence ledger is bound to that commit. F1 validation now uses isolated baseline `9285f2ca` with its reviewed one-file hash unchanged.

W2 final qualification passed after both narrow test-source reviews:44 trace tests
cover all58 exact native/Wasm records, and25 scalar/page/host lifecycle tests pass
again. No Rust or Wasm correction was needed. Both original failed traces remain
in `/private/tmp/foks-page-allowance-qualification.json` together with final11
source hashes, logs, artifact/catalog hashes and target terms. Default export and
import sets remain23/3; linker export ordering changed, not names/kinds. Both
artifacts retain18..1024 declared pages. Observed linear memory remains52,559,872
bytes through the large/small/large sequence, and each invocation records one
held page reservation. The independent JS declared page peak is2,630,213 bytes.
This is Node actual-Wasm qualification, not a new actual-celld replay. Root
integration/commit is pending; no serving admission or complete pool/RSS bound is
inferred.

W2 committed as `0bf6eeb4` after root verified the exact eleven final source hashes
and qualification evidence. The preceding native outer-auth parser commit
`9285f2ca` was disjoint. The qualification ledger now records W2's commit; original
failed/diagnostic traces remain preserved. The next bridge must bind the existing
Request owner rather than create a parallel budget owner, and remains a separately
reviewed integration step.

Factory F1 final validation passed on isolated `9285f2ca` with its original
one-file reviewed hash unchanged: focused memory 12, real listener boundaries 4,
transfer leases 2, full server library 125 passed/4 ignored, strict server/testkit
all-target lint and formatting/hash checks. On aarch64-apple-darwin the prior
Request field-layout surrogate is 72 bytes and the fixed Request is 128 bytes;
five slots are 80 bytes versus the old Vec header's 24 bytes. This trades 56 bytes
of fixed owner size for removal of per-charge variable bookkeeping allocation;
it does not close Request/Arc/Mutex admission or establish an RSS bound.

The first full run exposed the preexisting maintenance rollback fixture installing
an unqualified trigger before Writer startup. An exact unchanged-baseline run
reproduced that failure. A separately reviewed test-only correction starts the
writer against its qualified schema, then installs the same fault trigger and
requires the exact late SQLite failure. Rollback rows, counters, checkpoint
suppression and retry assertions remain. The corrected focused test and full
suite passed; original failure/baseline logs are retained. Separate evidence:
`/private/tmp/foks-request-permit-results.json` and
`/private/tmp/foks-maintenance-fixture-results.json`. The fixture is a separate
commit group; no verifier or production maintenance code changed. Cargo was
handed to R2 after these gates.

The separately validated maintenance fixture correction committed as `cae8a3f5`; Factory F1 fixed Request bookkeeping committed as `3d992330`. Both result ledgers are bound to their exact commits. Main integration retained the disjoint W2 implementation; no unreviewed source changes were included. R2 now owns Cargo.

### Factory F2: original Request bridge to owned List (concrete plan; review pending)

This subgroup builds on W2 `0bf6eeb4` and fixed Request bookkeeping `3d992330`.
It establishes the actual server memory-owner bridge, without adding an unused
dispatcher, SQL borrowing factory, readiness constructor or route. Root requested
the direct bridge plus server ownership tests; the later PendingList wrapper
remains part of the guarded factory. R2 physical admission, R3 semantic/import
readiness and C4 connection/engine qualification remain serving prerequisites.
No provider selection is reopened.

**Minimal core API.** Add feature `owned-list = ["dep:foks-rpc"]`; make
`list-allocation-qualification` depend on it. Change only existing W1/W2 ownership
cfgs to `any(test, feature="owned-list")`; trace observation/exports stay under
the qualification feature. The server metadata-core dependency enables
`owned-list`, without enabling core `native` or any Wasm fixture export. Expose
an `owned_list` facade containing the existing helper/framer and opaque owners:

```rust
pub trait ListRetainedOwner:
    foks_snowpack::AllocationBudget + foks_rpc::ResponseAllocationBudget
{
    fn reserve_retained(&self, bytes: usize) -> bool;
    fn reserve_page_workspace(&self, bytes: usize) -> bool;
    fn failed(&self) -> bool;
    fn fail(&self);
}
pub fn read_personal_list_owned(
    sql: &mut impl Sql, credential: &CurrentCredential, request: &ListRequest,
    now_micros: u64, maximum_directory_bytes: usize,
    owner: Arc<dyn ListRetainedOwner>,
) -> personal::Result<OwnedListBytes>;
pub fn frame_owned_list(value: OwnedListBytes, sequence: u64)
    -> foks_rpc::Result<OwnedFrameBytes>;
```

The owners expose only `as_bytes()`, `len()` and `is_empty()`, with private bytes
before owner in drop order. No public constructor, Clone, serialization, bare Vec extraction,
DerefMut or encoder constructor. OwnedListEncoder, Budget, PageWork, target terms
and Copy page progress remain private. Both supertraits already require
Send+Sync. This API conveys accounting and lifetime, not SQL, schema, TLS, owner
fence, readiness or publication authority. The helper requires the caller's
original admitted owner and valid current-auth/SQL scope; it creates neither.

**Captured scopes at both boundaries.** The private framer currently relies on
ambient response hooks. Crossing a crate boundary must not let ambient Request B
pay for output retained by A. Before framing, clone/upcast the raw owner's
existing Arc to both `Arc<dyn AllocationBudget>` and
`Arc<dyn ResponseAllocationBudget>` and install those synchronous scopes around
complete validation/framing. These are reference-count/pointer operations, not
fresh Arc allocations. The pinned Rust toolchain supports trait-object upcasting;
native/Wasm builds verify the exact Arc conversions. There is no second supplied
owner. Restore both previous hooks on return/unwind, preserve the original RPC
error, and latch the captured owner on failure as W1 already does.

Apply the same captured-owner scopes at `read_personal_list_owned` entry, before
credential recheck/parent decode/SQL work. This closes a similar cross-owner gap
before W2's page scope starts. W2 overrides only the nested Snowpack page hook
and restores A before occupancy release; retained growth still calls A directly.
Outer helper scopes restore B afterward. This does not make SQL engine memory
admitted, nor does it create server CURRENT TLS or an execution/deadline scope.
The eventual server factory still enters original Request::sync around its full
entry/SQL operation. These synchronous core functions never await.

Framing preserves raw-length/admission/validation/precise-frame-cutoff order and
sequence-width semantics. Snowpack validation uses A's working budget; the RPC
encoder's unchanged `2 * (raw_length + 128)` conservative reservation uses A's
response budget. Do not charge a second retained frame reservation or spend page
credits on framing. Raw bytes coexist with the final frame until the consuming
encoder drops them. All original permits remain held through final owner drop.
Empty valid List outputs retain ownership. Ordinary Ambient List and ordinary
RPC encoder APIs stay unchanged.

**Direct server implementation.** Implement the trait for private
`net::memory::Request`, with no adapter allocation, child Request or new public
permit API. Both reserve_retained and reserve_page_workspace check the existing
sticky failed bit, delegate to Request's existing Snowpack AllocationBudget
implementation, then recheck that bit. fail sets it; failed reads it. The inherited
RPC response implementation is unchanged. Arc<Request> coercion preserves the
exact original owner. No request classification, fallback, Bulk permit, input
permit, execution permit, timeout, counter or no-refund policy changes.

| Allocation | Original admission |
| --- | --- |
| One W2 page workspace plus W2 control allocation | Working, existing eligible Small fallback, once per owned List call |
| Encoder retained growth and raw finish | Working/eligible Small, cumulative existing charges |
| Page temporaries | W2 local credits backed by the held reservation; no parent charge per page |
| Frame validation | Captured Request Working/eligible Small |
| Final success frame | Captured Request Output/eligible Small, existing conservative RPC reservation |
| Separate error/status encoding | Existing Control policy, unchanged |

W2 control already includes its Budget Arc layout. F2 adds no control allocation.
The existing Request Arc/Mutex/five slots remain original bounded overhead
reported by F1, not newly claimed as pool charged. SQL/deadline/attempt/refusal
counters remain monotonic. Vec drop is not a parent refund or Wasm RSS release.

**Actual server qualification and factory boundary.** Server tests call the real
owned helper under original Request::sync, with SQL-derived CurrentCredential
and bounded multi-page native fixture data. Prefer the existing NativeSql only
as a dev dependency; add no production connection accessor. The bridge does not
install a caller in the ordinary native List service. Full page execution must
exercise actual reuse; direct trait-method tests alone are insufficient.

The opaque raw/frame owner retains Request even after RetainedValue::into_inner
removes its redundant outer owner. A future pending wrapper holds that opaque
value, generation, transaction/deadline and target-tagged durability state. No
unused wrapper is added now. Borrowing as_bytes is an accounting API, not an
approval to publish before COMMIT/sync/current checks. Pure framing cannot acquire
SQL or depend on server CURRENT; if that changes, its captured-hook guarantee
must be reviewed again. Physical output tests borrow the frame while its owner
lives rather than unwrap/clone its Vec to fit an existing write helper.

Required focused tests use real Pools/Request and exact pool deltas: one workspace
across several pages; no per-page parent working reservation; retained growth
remains cumulative; workspace refusal before page SQL; sticky refusal/no frame;
unchanged domain-error precedence; output refusal retains earlier working
permits; and empty successful output ownership. Run owned helper and framer with
unrelated ambient B: only A's pools change, B hooks restore on success/error/unwind,
and B failure cannot replace A's result. A's refusal remains sticky even if a
later control/status reservation succeeds.

Barrier-driven detached work and a blocked physical writer drop the waiter/outer
Request and remove RetainedValue's wrapper while the opaque result remains.
Permits remain unavailable through readable raw/frame bytes and physical output,
then release exactly once. Assert payload-before-owner order, not allocator RSS.
Synthetic core owners gain both explicit supertrait implementations. Refusal
tests formerly using an unrelated ambient response hook must instead refuse
through the captured owner; separately assert ambient refusal cannot hijack it.
Existing trace Recorder already implements both traits, so no logical event or
fault-injection renumbering is intended.

Expected source scope: core Cargo/facade, ownership cfg/reexports in encoder and
personal List/allowance, owned helper/framer/tests; server Cargo, Request trait
implementation and separate focused memory-owned-list tests. Lockfile changes
only if dependency edges require them. No catalog/SQL/profile artifact, Wasm
entrypoint, public wire or server-route changes. Freeze exact paths after author
review; root plus independent source review precede tests. Validate focused and
relevant full server memory/network/core suites, strict native lint, default,
owned-list-only and trace Wasm builds/export sets, all58 native/Wasm trace records
and existing callback/refusal/RPC cutoff gates. Owned-list-only must add no
qualification exports beyond the existing default artifact's set. No new
actual-celld serving claim follows.

Author plan review: the missing concrete boundary is Request implementing W2's
contract plus both owned operations installing that same owner's hooks. This
subgroup is independent of readiness but not a replacement for it. Entry/context
closure, R3/C4, guarded factory, hosted sync/deadline and restricted writer coverage
remain explicit gates. Root and quota plan reviews are pending; no code/tests yet.

F2 root and quota independent plan reviews passed. Both also approved a
stack-only unwind guard retaining a clone of the existing owner Arc: budget
scopes restore first, then panic latches that owner; a normal helper return
(including an earlier domain error) disarms it, preserving existing precedence.
The owned framer retains its normal error latch. No adapter/control allocation
is added by that guard.

F2 author source review is complete in isolated
`metadata-request-bridge-validation` at `3d992330`. Thirteen paths are frozen in
`/private/tmp/foks-request-bridge-manifest.txt` with adjacent `-sha256.json`.
The source exposes only the accounting facade, binds both owned operations to
the captured budget supertraits, implements Request directly, and authors six
server ownership tests plus core scope/unwind coverage. Default production
server dependency enables owned-list only; NativeSql is an explicit test-only
feature edge. No build, test, route, constructor or serving activation has run.
Root and quota source reviews are requested before qualification.

F2 quota independent thirteen-file source review passed with all frozen hashes
verified: same-Arc scopes, unwind restoration/latching, unchanged pool policies,
opaque ownership through into_inner/physical output, and feature separation.
Root source review and Cargo handoff remain pending; no F2 tests have run.

F2 root full thirteen-file source review also passed. Both reviews cover the
same frozen hashes, including six server tests and core unwind/refusal coverage.
Qualification subsequently completed on the reviewed `55dec5e4` union. Only the
server Cargo merge differs from the original freeze: F2 owned-list normal/native
dev edges preserve R2's deadline/schema/nix/rusqlite settings. Both reviewers
passed that union. One test-only correction includes the existing three S3a
credential parameter copies in an exact pre-page pool expectation (116 rather
than 17); both reviewers passed it before rerun. No production correction.

Evidence `/private/tmp/foks-request-bridge-qualification.json` and
`/private/tmp/foks-request-bridge-union-{manifest.txt,sha256.json}` records all
thirteen final sources. Gates passed: focused core5/server6; full metadata200
(3 ignored exporters), default server151 (4 intentional ignores); strict native
all-target lint; real-certificate58-case exporter and native List oracle; strict
default/owned-list-only/trace Wasm lint and release builds with declared64MiB max;
Node host/scalar/page25 plus trace44, all37 page and58 trace vectors. Feature
graphs prove default core has no normal RPC dependency and server normal core
enables only default+owned-list, with native dev-only. Default and owned-only
Wasm export23/import3 sets and ordering match the prior default artifact.

Large/small/large cycles retain the same52,559,872-byte Wasm highwater, with
2,630,213-byte maximum host-declared page workspace. This is resident linear
memory observation, not RSS release after a call. The sandbox-only broad test
attempt failed OS clock/loopback access; unchanged source passed with approved
OS access. Both failed logs remain in evidence. There was no new actual-celld
replay, factory constructor or production activation. Committed as `b6ffacb9` after root
verified all13 sources and27 evidence hashes; Cargo has been released.

## F3 combined proposal: one imported owner, guarded List, and pending output

Plan only; author review complete, root and independent review requested. This
supersedes the earlier proposal to land a private dispatcher independently of a
real importer. F2 supplies accounting ownership; R3 supplies the fresh imported
destination; C4 supplies private connection checks inside this single owner.
None independently enables a route. The next useful factory implementation is
their integrated closed List operation with a genuine imported test package.

### Owner and source boundary

Use a server-private `restricted_list` module, with `mod.rs` owning the operation
and pending states, `connection.rs` containing C4's fixed checks, and `tests.rs`
containing actual-import tests. Keep the connection in the server: no new normal
server-db-to-core dependency, no public Database connection loan, and no second
writer actor. NativeSql is used internally against the owner's active transaction.
Enabling its normal server dependency is a deliberate F3 change, separate from
F2's dev-only native edge.

This is a new default-private server module, not an expansion of the public
`hosted_personal` qualification module currently gated by `celld-proof`. Existing
A1 arbitrary-Sql/certificate helpers remain feature-only and cannot construct the
factory or attest TLS possession. Tests may call the actual private constructor
through the real importer; no qualification bypass is compiled into serving.

R3's provisional handoff is a non-Clone, non-deserializable
`NativePersonalListImport`, privately constructed only by the real logical
importer consuming `StorageScanComplete` and operator key input. Its field/API
definition must be frozen with storage before source edits. It owns the fresh
target connection, validated Config, target-specific schema/profile binding,
dependency incarnation/revision, path identity, WriterGuard/lifecycle exclusion,
package/key ownership and physical storage leases. F3 consumes this complete
owner; it does not accept `(Connection, hash)` or a path plus a boolean. There is
no public raw connection/path getter or callback borrowing the connection. Drop
closes cursors/connection before releasing keys, path/lifecycle and physical
leases. Import replacement consumes a new handoff and destroys the old owner.

The dormant imported connection initially retains its finite maintenance-job
progress hook. F3 must replace that hook through the fixed SQLite API with the
newly admitted request's cancellation/deadline hook before any request SQL, under
exclusive ownership and without a connection loan or query in between. This is
not renewal of the import job and cannot run a request on an expired import
clock. Exact hook replacement/unwind/idle restoration belongs to the consuming
constructor source gate, including whether idle state rejects all SQL.

The first imported scope is the whole finalized native package, explicitly
personal-only: no selected-user graph extraction. R3 must freeze exact predicates
for unsupported team/SSO/federation data and audit native conversions, existing
backup/key-provider/host-key consistency, companions and required related history
off the request path. This does not promise full historical signed-link crypto
replay. Pre-route SSO access is excluded by this roster; there is no SSO network
fallback. R2 physical fit alone does
not mint this handoff. Legitimate absent account/signup/authority-head states are
preserved. In particular, native `user_authority_inner` returns None before
device/key/stale-row conversions when its joined head is absent; R3's conversion
audit must preserve that conditional distinction. Rejecting all missing anchors
would not justify changing public error precedence.

The native target is not a celld receipt. A later celld importer must construct
its own target-tagged persistent binding on the actual cell with fresh trusted
DDL and proven sync, before that target can serve. Native test readiness must not
be serialized or relabeled into hosted readiness. Production profile IDs, exact
schema/catalog/ABI/policy digests and immutable artifacts require their own
concrete freeze before implementation; the current high-bit qualification IDs
are rejected. No runtime caller chooses a profile, SQL catalog or expected DDL.

### One operation and one budget

The private entry consumes the original admitted call: raw bounded public List
argument and sequence, authenticated TLS peer evidence, original `Arc<Request>`,
original local Deadline/cancellation ownership and physical execution permit.
These fields come from the fixed frontend, not deserialization of a public
factory command. Keep one Request through queueing, context, SQL, frame and
physical output; enter its `Request::sync` around every synchronous core phase.
F2 additionally binds the same Arc at helper/framer boundaries. No nested Request
or admission-class renewal is introduced.

Add a minimal shared SQL-counter seam with the factory group: a fixed-size
request-held `SqlCallBudget` with private count/failed fields and checked,
monotonic `attempt()` capped at the existing 2,048. NativeSql gains a constructor
borrowing this budget for the closed path; its existing independent constructor
retains qualification semantics. A small enum/local-counter implementation can
avoid an allocation, but its exact API/layout is a source-review gate. C4 fixed
queries, controls, core scalar/page operations, precommit checks and post-sync
checks all use that same object. The factory path counts an attempted operation
before parameter validation/preparation, including failed validation, prepare or
execution. It must not count a single operation in both wrapper and NativeSql.
The ordinary constructor's old ordering is unchanged. No reset/subtraction based
only on successful return, and no recreating a fresh budget after COMMIT/sync.
Counter exhaustion is sticky infrastructure failure with no candidate output.

Original deadline/cancellation checks bracket every SQL operation and callback
transition. This is cooperative control: synchronous SQLite, fsync and Wasm can
overrun while retaining physical ownership. The shared request counter and
deadline do not become a demonstrated 15-second latency bound. The final source
gate must enumerate C4's maximum schema-page calls, core calls and all exit checks
against 2,048; no silent increased limit if the combined bound fails.

### Closed connection interval and public authority ordering

Adopt the corrected C4 plan in `HOSTED_METADATA_CONNECTION_QUALIFICATION_PLAN.md`
as the private `connection.rs` contract. Native construction establishes and
reads back WAL/FULL, foreign_keys ON, ignore_check_constraints OFF,
recursive_triggers ON and trusted_schema OFF, with existing validated size/WAL/
temp limits and bounded busy policy. Every transaction checks actual settings,
empty TEMP, no attachments, exact streamed target schema, cookie and R3 binding.
No callback, SQL text, extension/function registration or connection reference
escapes. C4's borrow-scoped interval is neither Copy nor stored in pending output.

Infrastructure admission may precede public argument processing, but cannot
perform resolve_authority's anchor/role checks early. Native session.rs performs
`Principal::authenticate` for every authenticated RPC before dispatch, not merely
at the TLS handshake. The admitted operation must therefore first verify current
certificate issuance/revocation/expiry and ordinary-device route eligibility,
before malformed public arguments can produce a List status. TLS possession is
additional evidence, not a substitute for that current database admission.
Native also runs `ServerData::authorize_principal` between authentication and the
service's ordinary-device restriction; it opens its reader and samples time again
before SSO access evaluation. Preserve that distinct clock-failure boundary even
under R3's no-SSO policy. The exact entry sequence is certificate admission clock/
current certificate -> authorize_principal clock/access -> ordinary-device route
restriction -> F0 decode -> resolve_authority clock -> classify -> active owner/
anchor/roles. The trusted frontend/factory contract must supply these stages at
their actual execution points, not collapse them into one HostListContext sample.
The native implementation/oracle preserves those separate calls and failures.
The distributed context contract must distinguish trusted infrastructure refusal
from public/domain parity under valid time evidence; a single validated remote
sample cannot reproduce every native clock-failure interleaving. Freeze this
target-specific provenance before code. No arbitrary clock callback runs in a
sealed celld transaction: any needed time delivery is a fixed, reviewed phase of
the trusted import interface. None refreshes the original execution deadline.
Within an admitted interval, after this per-RPC admission:

1. Run the shared F0 `KvAuthorityArgument::decode` on the original argument; keep
   its owned AST alive through authority as native does. Malformed outer shape
   retains its exact status/message before the public clock/classification stage.
2. Establish the exact-microsecond authority-time sample at the native
   resolve_authority clock boundary, then call `classify()`. This is distinct
   from session authentication's earlier clock sample. Both remain under the
   original execution deadline; sampling current time does not renew it.
   A trusted host's inability to
   supply valid time is infrastructure failure; do not synthesize a successful
   public error frame from an invalid context. The native clock-failure matrix
   remains the oracle for the public boundary. A well-formed Team selection is
   an explicit unsupported-profile path for this first imported scope, not a
   synthetic personal identity or claim of team parity.
3. Recheck active credential ownership and personal authority using SQL, then
   the anchor/maximum role and required key generations. Do not accept a UID,
   namespace, role, generation or parent key from frontend assertions.
4. Only after authority succeeds run the shared
   `decode_kv_list_arguments` and the owned List helper with its SQL-derived
   CurrentCredential. Preserve count normalization, Time-u64 conversion after
   parent permission, optional-body omission and exact public error text.

The existing A1 `PersonalResolver` performs `require_personal_profile` during
Prove, including anchor checks; reusing it before step 1 would be wrong. Extract
only the minimum private borrowed gate needed for this closed List path: checked
time, admitted state load, owner/activation/fence checks and revision observation,
then separately staged current-credential and personal-authority checks in the
order above. Derive authority revision from current SQL state, never an asserted
frontend revision. Borrow strict HostListContext peer/fence; do not clone it into
an owned Invocation/Request::Prove or use generic qualification JSON. Existing
A1/mutation resolver behavior stays unchanged and remains regression-tested.

New List authentication uses the checked session-admission microseconds for
certificate expiry. Only the authority clock floor uses checked `micros / 1000`; it must not
round certificate expiry into extra validity. Reuse S3b admitted digest/copy code
and S3c preserved state decoder. The floor update is the only allowed write in
this List-only owner. Freeze its exact order relative to fence rejection and
domain errors against existing authority behavior before code, and prove it
preserves restricted physical bounds, R3 semantic witness and the dependency
coordinate. If it changes the coordinate, publication records the resulting
transaction-local coordinate; it must not compare against a stale prewrite value.

R3's closed writer exclusion is essential for bounded authority equivalence.
Native loads unrelated device/shared-key/stale rows too; the factory must not
claim that targeted queries discover arbitrary corruption in every such row.
The imported invariant covers those conversions until replacement, while touched
record errors and legitimate absence retain native ordering. No unbounded history
scan is added to each request. Any write path able to invalidate that invariant
is excluded until R4. Root's native List oracle supplies original argument bytes,
exact response/status and paired malformed-argument/missing-anchor cases.
Initial Principal authentication reads unsigned role fields but does not perform
all stored_role checks; authenticate_peer also validates owner linkage earlier.
Any reuse equivalence is conditional on R3's explicit admitted history roster,
not unconditional native parity. Reject unsupported histories at import and test
the preserved legitimate absence cases. Do not simply move authenticate_peer
before F0: its stored_role/owner-link checks are stronger than the native early
certificate gate. Freeze an early gate matching actual Principal checks; defer
role materialization until after classification unless R3 explicitly excludes
that record combination. Include malformed argument + missing head + invalid
nonnegative role in the oracle if R3 admits that history. Current HostListContext carries only one
time sample: the exact guarded context/import API must represent these distinct
clock boundaries before code, without reusing a later sample retroactively or
cloning a new execution deadline. Post-sync fresh expiry is a publication guard
and can intentionally refuse an observation valid at initial admission.

### Commit, pending frame and physical publication

Use one nonwrapping operation generation in the owner. States are
Ready -> Executing -> PendingCommit/Sync -> PendingFreshCheck -> OutputBorrowed
-> Ready, with terminal Invalid for uncertain/trapped/replaced state. Checked
generation overflow retires the owner. A pending object is non-Clone and privately
constructed; it owns the opaque F2 success frame or an owned exact status frame,
original Request/Deadline/execution lease, sequence, target/generation and the
observed fence/dependency/schema/profile coordinates. It contains no C4 seal.
Status framing uses the existing response-budget hook and exact public codec: Control is selected only when status_capacity<=1024; larger statuses use Output with its existing fallback policy. It must retain its Request through physical output just as success does. The concrete
status-owner layout/drop order is part of the combined source gate.

Before commit, drop SQL rows/borrowed pages and repeat C4 exit checks plus fence,
dependency and deadline checks. Infrastructure/allocator/trap failures roll back
and discard; they cannot become publishable domain status. An expected public
domain rejection may become a candidate only through the exact classified path
and reviewed clock-floor policy. Failed COMMIT produces no durability receipt or
frame even if outcome is uncertain. Native success requires actual WAL/FULL
readbacks immediately before successful COMMIT under the closed owner; this is
local SQLite/VFS/filesystem durability, not celld S3 durability.

For celld, the fixed synchronous callback stores the internally constructed
pending object in a private slot and returns only a private singleton marker.
Its outer closed callback compares identity before allowing commit. A Promise,
Proxy or foreign object is rejected without accessing `then` or any getter.
There is no external callback. After transactionSync commits, root storage.sync
runs outside the callback with Busy ownership retained across await. A fresh
read-only synchronous C4 interval then checks current target/generation/fence,
dependency, certificate validity and original deadline before publication; it
cannot advance the clock floor or write. All its SQL spends the original budget.
An observation that requires a write must move before sync or require another
reviewed commit/sync, never be silently published.

Neither ptr/len nor as_bytes is external publication authority. Host-only pending
ownership gates the actual server transport write boundary. No subsequent
input, activation, SQL or memory growth may invalidate borrowed output. Native
borrows opaque bytes through the physical writer; Wasm uses a fresh bounded view
only while growth/reentry is prohibited, or an explicitly admitted owned copy.
Caller cancellation drops no physical permit early. Failure during sync/fresh
check/publication discards pending; stale completion cannot resurrect a reused
instance. Traps poison first, wipe capped explicit memory after outermost unwind,
then destroy the instance without calling Rust cleanup exports. Wasm highwater
remains charged to its instance owner until actual destruction.

For the first serving module, do not retain an idle per-tenant Wasm instance.
Acquire a slot and the artifact's declared maximum linear-memory reservation
(currently 64 MiB) from a finite process-wide instance-capacity owner before
instantiation. Hold both through active work, detached cancellation, sync and
physical output; destroy the instance on completion/abandon before releasing
them. Capacity exhaustion is bounded admission refusal, not an unbounded queue
or instance creation. Sharing immutable compiled Module bytes is distinct from
retaining a tenant's linear memory or activation state. No generalized instance
pool is required now; a later reusable pool would need its own isolation/eviction
proof. The exact process cap, reservation owner and refusal mapping must freeze
with the host source before code. Repeated-call/cold-activation latency and a
10,000-idle-tenant run must measure this actual destruction policy; SQLite/V8/module
cache and connection residency remain separate measured capacity obligations.
This requirement is not implemented by a JavaScript global semaphore or dropping
the final visible JS reference. Pinned celld's stateless max_requests setting is
not the cell-request limit; its logical resident-cell cap does not prove physical
retirement overlap or V8 reclamation. A real process-wide physical-capacity owner
and release evidence are still required for the celld target. Native F3 first
uses compiled Rust and existing real server Request/execution/connection leases;
it creates no Wasm instance and does not claim hosted10k density from those tests.

### Concrete code gate and qualification

F3 does not introduce a placeholder readiness constructor. Before source edits,
freeze together (a) R3 private handoff and exact personal-only semantic roster,
(b) production target IDs/immutable artifacts and guarded ABI, (c) counter API and
closed write/floor policy, (d) target frontend context/clock/lease provenance and
(e) remaining entry/status/host-copy admission terms. Native factory source may
then be authored with the actual importer and C4 private module in one reviewed
integration group; celld target construction requires its own real imported-cell
path. The current default qualification Wasm is not a production artifact.
Production allowlist excludes all unauthenticated fixture exports, including
run_authority/run_page/run_directory_key and token/trace fixtures. No token issuer
or Begin/Next/Finish exports are needed for this single-call route.

Implement no generic dispatcher, mutation callback or second connection owner.
Suggested bounded ownership split: storage owns R3 importer/receipt and physical
profile roster; quota owns private C4 checks/counter integration; recovery owns
borrowed authority staging, guarded List operation and pending output integration;
root reviews public error/time/factory composition. Exact source manifests and
two implementation reviews precede tests. R4 is not required to pretend List is
a general writer: it is required before any external domain mutation, maintenance,
credential rotation or restore route can use this owner. The private floor's
preservation proof is a prerequisite even for List.

Qualification uses genuine finalized backup -> R2 -> fresh R3 import -> F3, not
test-minted capability fields. Compare raw Rust/Go public List arguments and exact
frames/status against native, including malformed argument paired with unissued,
revoked or expired certificate, missing head plus malformed argument,
expired/revoked credentials, subkeys, absent/zero/extreme generations, permission
before Time overflow, sparse/maximum pages and sequence-width frame cutoffs.
Adversarial tests cover target/profile mismatch, malformed settings/schema/TEMP/
attachments, stale incarnation/fence, attempted counter reset, foreign callback
values without getter execution, wrong allocator/reentry, all admission failures
and COMMIT/sync uncertainty. Pause each physical boundary to prove no early frame,
no lease release on waiter cancellation, and no stale completion publication.
Actual celld tests must use the same immutable serving artifact/module and real
target import, exercise reactivation/replacement and stable frontend routing, and
measure repeated large/small calls plus original-deadline failure behavior.
Restricted parent 1 MiB does not change native 16 MiB semantics or waive retained
frame/JS/SQLite/instance-memory bounds. This first personal List checkpoint is
not org64, full backend coverage, HA recovery or a completed M1/M2 claim.

Author review: this plan resolves ownership location and error-order composition
without treating F2 accounting or C4 diagnostics as readiness. Remaining exact
R3/artifact/entry-admission contracts are real pre-code gates. Work that can proceed
now is F2 qualification, R3's real importer/roster and the combined contract review;
no detached private dispatcher or fake production guard is needed meanwhile.

F3 combined direction received root review PASS after the explicit native
per-RPC/SSO/ordinary/F0/authority stages and target-time distinction above. Storage
independently passed the consumed R3 owner/scope/witness direction with the
historical-crypto precision incorporated. Quota's combined independent review
also passed after comparing the revised stages with the native source.
All exact pre-code gates remain mandatory; no factory source or route was added.

Root and quota also passed the narrow first-instance residency direction:
process-wide bounded slot plus maximum-memory reservation, no idle per-tenant
instance, physical destruction before release. The real process/isolate scope,
numeric process cap and engine/module residency qualification remain source gates;
64 MiB is the current per-instance artifact maximum, not a chosen process cap.

## F3 C4/counter source contract — proposed exact freeze

Quota author proposal; no detached implementation. This section fixes the C4
portion of the combined F3 group. Recovery owns the staged authority/floor/time
and production artifact contract. Storage owns the actual R3 handoff. Root and
independent review precede edits; the auxiliary authority-call ledger below is
an explicit remaining combined freeze item, not an assumed allowance.

### One monotonic counter and bounded abort ownership

Add accounting-only `SqlCallBudget` in `foks-metadata-core/src/sql_budget.rs`,
re-exported from lib.rs. It is not a database/authority capability. Proposed
`#[repr(C)]` fields are `charged: u16`, `attempts: u16`, `cleanup_credit: u8`,
`failed: bool`; expected size/alignment is 6/2 on native and Wasm, pinned by tests.
It is neither Clone nor Copy. `new()` starts zero; read-only `charged()`,
`attempts()` and `failed()` expose diagnostics, never setters/reset/refund.
`attempt(&mut self) -> Result<()>` first refuses a failed budget, then checked-
charges one of the existing 2,048 credits and increments attempted operations,
before parameter/mode validation, binding allocation or prepare. At exhaustion
it latches failure and performs no SQL; repeated denied calls cannot overflow or
clear it. Attempted operations include parameter/prepare/execution failures;
they are not represented as successfully executed SQL in evidence.

The original F3 operation owns this six-byte value inline with its original
Request/execution lease through main transaction, sync, fresh publication check
and output decision. It may move with the owned async job but is never shared
concurrently; no Arc/Mutex allocation or TLS fallback is needed for the counter.
NativeSql temporarily borrows it. `NativeSql<'a>` retains its existing public
lifetime shape and internally replaces `calls: usize` with
`Counter<'a> { Local(usize), Borrowed(&'a mut SqlCallBudget) }`.
`new_with_budget(connection: &'a Connection, budget: &'a mut SqlCallBudget)`
requires an already active transaction and an unfailed budget; it does not reset
it. Existing `new()` and Local-mode validation/count ordering remain unchanged.
Borrowed mode calls attempt before validation; `calls()` reports the selected
counter's monotonic operation count. Fixed C4 operations borrow the same budget
between NativeSql lifetimes. No wrapper counts an operation already counted by
NativeSql. Failed construction that executes no SQL consumes no SQL attempt.

The6-byte budget is only its own inline representation, not total per-operation
control overhead. Original F3 entry admission must also include the target-sized
NativeSql wrapper and Counter enum (including pointer/discriminant/padding), the
operation/pending state and Request-owned control fields, plus the schema scratch
hook/ArcInner layouts and fixed cursor storage. Stack-only values still belong in
the target stack/control envelope; they are not separate heap charges. Do not
charge overlapping fields twice when they are contained in the operation object.
F3's source-size ledger must name each actual owner/size/alignment and its live
phase, with layout tests on native/Wasm; no claim that6bytes closes that ledger.

Reserve one rollback credit **before** each outer BEGIN through private factory
control methods on this accounting object. Reserving increments charged, not
attempts; it never refunds. `begin_cleanup_reservation()` refuses an existing
credit, exhaustion or failure. `attempt_rollback()` may run after ordinary failure
but only consumes that precharged credit once and increments attempts; it cannot
execute arbitrary SQL. Successful transaction completion clears the unused credit
without decreasing charged. Native transaction guards and celld transactionSync
host integration must avoid issuing a second explicit SQL ROLLBACK after a manual
rollback: disarm/consume the matching guard at the explicit cleanup boundary, with
no retry after rollback failure. Accounting covers explicit host attempts, not
SQLite internal cleanup. SQLite may implicitly roll back on IO/FULL errors or
connection close; do not promise to prevent those engine actions. A failed explicit
rollback destroys/invalidates the owner while retaining physical work.
Two successful outer transactions therefore charge two unused rollback credits;
report them as reserved cleanup, not executed SQL. BEGIN and COMMIT each use
ordinary attempt. If BEGIN fails, account its attempted operation, clear the
unused credit and fail the operation. This initial schedule exposes no nested
transaction; introducing mapped nested controls with F3 floor policy requires an
explicit corresponding cleanup reservation ledger before its source freeze.

Celld uses the same numeric semantics in the private operation-owned host state;
all fixed C4 SQL and every Wasm SQL/page/control import enter that one counter.
The existing Wasm Host local limit may remain a redundant stricter check, but is
not the authoritative shared budget and cannot permit a reset at phase boundaries.
Host import admission counts before decoding/validating caller bindings/IDs;
unknown/invalid calls fail and poison without reaching SQL. Budget arithmetic is
bounded exact integer arithmetic, not caller-supplied numbers. No budget fields
are added to a public wire DTO. Physical SQLite implicit work/trigger instructions
are not separate host SQL calls; they retain the independent engine limits.

### Exact C4 schedule, two full schema passes

Main and FreshPublication are the only two sealed transactions. Each has one full
schema pass. Precommit exit uses fixed settings/cookie/coordinate checks, **not**
a third schema pass. FreshPublication is the post-sync transaction on celld and
the separate final publication-check transaction on native. It is read-only and
does not advance the floor. No third post-check transaction is scheduled; all
output follows its successful exit while the same F3 physical owner remains busy.

Construction (outside the request schedule) applies the C4 native WAL/FULL and
validated Config policy. Request entry only reads it; no repair PRAGMAs. Fixed
settings SQL, each requiring one correctly typed row and EOF:

| Target | Fixed statements | Required result |
| --- | --- | --- |
| Both | `PRAGMA foreign_keys`; `PRAGMA ignore_check_constraints`; `PRAGMA recursive_triggers` | INTEGER 1; 0; 1 |
| Native additionally | `PRAGMA trusted_schema`; `PRAGMA main.journal_mode`; `PRAGMA main.synchronous` | INTEGER 0; TEXT `wal`; INTEGER 2 |

These are internal fixed C4 operations, not a caller-addressable raw SQL API.
Do not send them to current celld Scalar ABI as INTEGER results without conversion:
its strict scalar codec expects canonical decimal TEXT. The private host check
reads the platform integer directly with exact type/range and consumes it locally;
no guessed coercion, array collection or new generic scalar export. Native reads
borrowed SQLite cells directly. No trusted_schema read is attempted on celld.

Each entry executes settings, then exactly:

* TEMP: `SELECT CASE WHEN EXISTS(SELECT 1 FROM temp.sqlite_schema) THEN '1' ELSE '0' END`, one TEXT `0` and EOF.
* Attachments: `SELECT CASE WHEN EXISTS(SELECT 1 FROM pragma_table_list WHERE schema NOT IN ('main','temp')) THEN '1' ELSE '0' END AS value`, one TEXT `0` and EOF.
* Existing catalog `ReadSchemaCookie` (44), typed canonical nonnegative cookie.
* Existing `SchemaObjects` page statement (9), with **eight rows per request**,
  only trusted initial/last-validated `(type,name)` continuation. Complete page
  type/length validation precedes verifier observation. The exact-union verifier
  retains no SQL text. Enforce 1,024 objects, 2 MiB aggregate and existing per-field
  limits; request one final page to establish EOF even at exactly 1,024 objects.
  An extra object is refusal. The maximum is 128 nonempty pages plus one EOF page:
  **129 SQL attempts**, including the malformed/over-limit terminating page.
* Catalog44 again, require same cookie, then existing ReadDependency (43), require
  one valid singleton/incarnation/revision. R3/fence comparisons themselves do not
  create another independent schema receipt or query.

Each precommit exit executes settings, TEMP, attachments, catalog44 and catalog43
again. Settings/TEMP checks remain defenses, not evidence that an earlier arbitrary
callback was safe. New fixed C4 SQL/readback expressions must join the immutable
production policy inventory and C4a corpus; the old corpus does not cover additions
by implication. Native and celld profiles remain distinct.

| Credits per complete interval | Native | Celld |
| --- | ---: | ---: |
| Entry: settings + TEMP/attachments + two cookies + schema129 + dependency | 140 | 137 |
| Exit: settings + TEMP/attachments + cookie + dependency | 10 | 7 |
| BEGIN + COMMIT | 2 | 2 |
| Precharged abort slot, unused on success | 1 | 1 |
| Both complete intervals | **306** | **294** |

An error never restarts a page/interval. Validation failures charge their attempt
and terminate, with at most the already reserved rollback. Malformed settings,
schema or bindings cannot exceed the full successful schedule by retrying. Root
must review any retry policy as a new callgraph; none is part of this contract.

The current owned List helper's source upper bound is 1,133 calls: three entry
credential/profile rechecks, one directory probe + at most32 slices (16 MiB
legacy ceiling), one parent-key query, and at most274 List pages with at most
three role batches per page (`ceil(4096/15)*4=1096`). This is conservative for the
restricted 1 MiB parent: only two slices, yielding **1,103**. Keep the 1,133 bound
in combined arithmetic so no accidental native-limit assumption creates a hole.
Thus C4 + unchanged helper cost at most **1,439 native / 1,427 celld** charged
credits before the staged F3 auxiliary calls. Authentication can need all20 serial
probes, not stop at first match. Recovery must freeze its exact early/late/fresh
certificate, state/floor/fence and optional control ledger before implementation;
its maximum must fit the remaining609/621 credits and must include its own cleanup
reservations if it introduces nesting. This is not permission to spend the whole
remainder: the source contract must name a much smaller actual finite schedule.
Until that final ledger lands, only these C4/helper subtotal bounds are claimed.

### Exact schema scratch admission and lifetime

Do not accumulate a maximum-sized PageEncoder charge on every schema page. For
page9, the maximum encoded row from the compiled columns is **66,160 bytes**:
four type tags/texts, four integer lengths and four bounded field payloads. Eight
rows plus the24-byte header require **529,304 bytes**. Both constants are derived
and asserted against `PageSpec::maximum_row_bytes()`; never hand-maintained alone.
The maximum encoded parameters are316 bytes (header24, two integers18, kind13,
name261). The verifier and continuation cursor must outlive a page, but borrowed
rows/SQL bytes must not.

Add a private `SchemaScratch` in the F3 C4 module, owned by the original Request,
with one admitted scratch allowance and exclusive lexical page occupancy. It uses
the established W2 ownership rules, not its List-specific numeric formula: one
precharge before allocating its hook/control, local page accounting only while
occupied, sticky refusal/panic, all page buffers/row borrows/parameter temporaries
and diagnostic values dropped before exit, hook restored before reuse. Only the
scratch traffic counter resets after successful physical exit. SQL count,
Request failure, deadline, verifier progress and retained/output charges never
reset. A Copy-only page outcome carries cursor bytes/EOF, not a Vec or row borrow.
No generic public scratch factory or callback API is added.

Native per-page allowance uses the existing NativeSql/PageEncoder charge sites:
`N = 264 + 4*sizeof(SqlValue) + 304 +
12*(512 + 9*sizeof(Cell)) + 66160 + 529304`.
The264 covers owned kind/name parameters, 304 the maximum binding payload
(two20-byte decimal strings plus kind8/name256); remaining terms are the exact
existing binding/row-container/max-row and PageEncoder reservations. Descriptor
and fixed scalar checks use stack storage/borrowed rows, not uncharged owned
String conversion. Rust/Wasm schema decode uses the page's borrowed row/cell
iterators, not a new decoded schema Vec.

Wasm Rust per-page allowance is
`W = 264 + 24 + 316 + 4*sizeof(Cell) + 529304`:
parameters, parameter-header and growth charges, cells, and one P1-owned response.
Scope::finish moves that response into PageBuffer without a copy. Use the relevant
target term, never sum incompatible native/Wasm layouts as a runtime claim.
Hook/control allocation is admitted separately using exact target size/alignment
and Arc-inner layout as W2 does; no uncharged observer Arc. Refusal before the
first page must execute no schema SQL.

The schema verifier's three growing previous-key Strings are not covered by SQL
row admission. Before each complete scan reserve4*(8+256+256)=2,080 requested
bytes under the original Request to cover their bounded geometric growth traffic;
keep that reservation through Request completion. The independent continuation
uses fixed arrays `[u8;8]`/`[u8;256]` plus lengths, so replacing a page cannot leave
a borrowed cursor. Pin the standard String growth/source assumptions and test
extreme ascending name/table lengths; allocator rounding/RSS remains excluded.
If the current pure verifier changes growth behavior, update this bound or replace
its private storage through separate source review, never silently assume it.
Expected contracts/artifacts are immutable deployment-owned bounded objects,
admitted at construction, not reparsed from target data in the request path.

The private JS host has its separately admitted page workspace, using the existing
codec's declared terms: `4*316 + 1048576 + 3*66160 + 16*512 = 1,256,512` bytes for
parameter conversion, writer/delivery and row/container scratch. PageWriter still
allocates spec.maximum_bytes (1 MiB), even for eight rows; its finish returns a
subarray rather than another owned page. Do not substitute the smaller Rust
PageEncoder capacity in this JS term. Hold this before
copying bindings or advancing the cursor; release only after cursor/writer/import
completion and all views are gone. This is declared logical JS allocation coverage,
not a proof of V8 string/allocator or SQLite engine RSS. Instance64MiB, bounded
process slots, module/connection cache and engine memory are separate owners and
qualification obligations. Strict malformed row/type/length failure must not
allocate echoed diagnostics or retain rejected SQL text.

### Source ownership and required tests

Quota's integrated F3 paths: core `sql_budget.rs`/tests, core lib export, NativeSql
counter seam and tests; server private `restricted_list/connection.rs` plus its
private schema scratch/tests. Recovery owns restricted_list operation/pending
state and staged authority calls, and the serving JS host embedding the matching
private counter/checks. No edits to the public native route, fixture Kernel or
qualification IDs silently substitute for this integration. F3 source manifest
must include the exact serving host and generated production policy artifacts.

Tests pin counter size,2048 boundary, pre-validation charge, sticky refusal,
borrowed-mode continuation across multiple NativeSql instances, unchanged Local
semantics, unused abort charge vs actual attempt reporting, rollback after
exhaustion and no second rollback. Verify real settings/cookie queries, schema
8-row/1024-object/EOF boundaries and both intervals' exact call ledger. Adversarial
invalid rows/IDs never trigger a retry or extra scan. Scratch tests assert one
parent reservation, no leaked page/borrow across reuse, refusal before SQL, hook
restoration after normal/error/panic, unchanged original Request retained/output
charges, and target-specific exact formula/source pins. Run real full schema,
maximum List and all refusal boundaries under one budget; tests must compare the
final integrated counts to the completed auxiliary ledger, not only these
subtotals. This plan creates no new reusable connection proof or serving route.

Author source cross-check: the JS writer's actual1MiB allocation is included
above, separately from the529,304-byte maximum eight-row Rust delivery. The
finite process-wide instance reservation must be provided by the real native/
platform owner: a per-realm JS variable is not a node-wide semaphore. celld's
stateless request cap, per-cell drive permit and logical residency eviction do
not prove that physical Wasm instances/output buffers have been destroyed. F3's
process-owner seam remains an explicit platform implementation gate.

Root review of the exact C4/counter proposal: **PASS** for the described native
seam and scratch terms, with explicit-host rollback accounting and full control-
layout clarification incorporated above. Recovery independent exact-delta review
is requested. The1,439/1,427 values remain C4+helper subtotals; neither complete
authority/control SQL accounting nor complete entry/control allocation admission
is inferred. Combined F3 freeze remains required before implementation.

Recovery independent exact C4/counter review: **PASS** after checking the fixed
parameter array, NativeSql charge sites, PageBuffer borrowed iteration and verifier
String growth against source.129 schema attempts and306/294 two-interval credits
include unused cleanup reservations, not executed SQL. Explicit/implicit rollback
and full control/layout distinctions remain intact. Final AUX ledger uses the
separately reviewed45/46 staged queries; no code is authorized before combined
F3 contracts and source manifests freeze.


## F3 native contract freeze proposal — authentication, SQL schedule and bootstrap

This is the concrete native-first contract for joint review, not permission to
construct readiness from an R2 report. It narrows the combined proposal above:
no serving Wasm artifact or celld process semaphore is part of the first source
group. The native owner consumes the genuine R3 import and retains its connection,
keys, Config, private directory, WriterGuard/path/storage leases and original
Request/output owners. The normal native route and A1 qualification behavior stay
unchanged. Root and independent review of the exact remaining parser/envelope and
bootstrap fields below precede implementation.

### Native early certificate and current-owner queries

Reserve scalar45 `ListIssuedCredential` and46 `ListCurrentOwner`. They are fixed
catalog operations, not frontend-selected SQL. Proposed45 parameters are padded
serial BLOB, exact TLS DER BLOB, and initial authentication microseconds INTEGER:

```sql
SELECT c.uid,c.credential_id,CAST(d.role_type AS TEXT),
       CAST(d.visibility AS TEXT),CAST(c.not_after AS TEXT)
FROM issued_certificates c
JOIN devices d ON d.device_id=c.device_id AND d.uid=c.uid
WHERE c.serial=?1 AND c.exact_certificate=?2
  AND c.not_before<=CAST(?3 AS INTEGER)
  AND c.not_after>CAST(?3 AS INTEGER) AND d.active=1
```

There is deliberately no current-device/subkey linkage predicate and no DER-expiry
predicate here. Compare actual public key to the typed credential ID and preserve
native supported credential-kind and unsigned role conversion behavior. Do not
materialize Role or parse issuance owner as EntityId at this stage. The five
returned columns are bounded by admitted native constraints;45 does not need to
return or retain issuance owner. Missing/invalid early binding is the native
Principal authentication failure class, not a synthetic BadArguments response.
Probe every zero-padded serial width through20, including widths after a match;
refuse multiple matches. The R3 certificate extension described below is the
prerequisite that makes this bounded search equivalent to exact-DER lookup.

After F0 decode, the native authority clock and classification,46 is:

```sql
SELECT device_id FROM devices
WHERE uid=?1 AND active=1 AND (device_id=?2 OR subkey_id=?2)
ORDER BY device_id LIMIT 1
```

This tests existence and chooses the same first device as native's ordered device
snapshot. Each equality arm has at most one logical candidate because of its own unique index;
cross-column uniqueness is NOT assumed. This is not yet a bound on rows visited:
actual target EXPLAIN/cost qualification must show indexed searches rather than
an OR/ORDER full scan. If it does not, review two forced indexed equality arms
merged with ORDER/LIMIT before freezing the final SQL. That SQL joins the C4a
policy corpus and catalog artifact; no optimizer assumption substitutes for it. Absence is permission denial before the
anchor. Keep the admitted raw owner bytes, then query35 for the joined anchor;
only after it exists query34 for the selected owner's role/visibility and map an
invalid selected role to native permission denial. R3 proves the native complete
head-present history conversions; targeted queries do not re-audit that history.
Current-owner selection is independent of the certificate's historical device_id:
archived delegated certificates, current subkey rotation and cross-owner rebinding
must retain native outcomes. Missing head still precedes selected-role decoding.

The private CurrentCredential owner representation may become an admitted Vec
instead of EntityId. Existing A1 still validates its owner EntityId and linkage
before moving the same buffer; the new native staged constructor keeps raw SQL
owner bytes without adding an EntityId-prefix restriction. Its width33/34 follows
from R3's fresh native CHECK-constrained insertion and final integrity check
(identity.sql), not R0's broader generic BLOB limit. Defensively validate that
width before copying/retaining it. No public caller can construct this credential.

The staged credential retains SQL `not_after`, not floored DER not_after. Pass the
initial successful certificate-admission microseconds to the helper's existing
credential recheck. Native samples a later authority clock but does not recheck
certificate expiry there; using that later time would change a boundary outcome.
DER validity remains TLS-handshake evidence. A persistent TLS session can cross
DER's floored second boundary while remaining inside stored issuance microseconds;
that is not an import error or early List rejection. Fresh publication checks use
fresh time deliberately and may refuse an initially valid observation.

### Certificate import invariant and peer ownership

The proposed R3 extension is a complete two-pass streaming audit under the same
immutable imported owner and original finite maintenance deadline. First validate
every issued row's bounded supported DER (<=4096 bytes), supported Ed25519 key,
positive representable serial magnitude<=20, and equality of stored serial to a
zero-padded representation of that DER magnitude. Negative/unrepresentable serial,
trailing/unsupported DER or oversized DER is explicit restricted-profile refusal.
Second, for each row, probe all at-most20 padded serial PK candidates and compare
exact DER, requiring no duplicate exact certificate. Identical DER necessarily
has the same magnitude after pass one, so this proves uniqueness without a retained
inventory/hashset, new index or certificate-table scan per request. Both passes
must finish before minting the stronger import owner. No stored-vs-DER timestamp
equality and no issuance-owner/current-owner linkage invariant is added.

The native frontend lends original TLS-owned DER under its real session/execution
ownership, bounded before parsing. It does not accept public PeerCertificate JSON
or promote feature-only parse_hosted_peer_certificate into a serving constructor.
Serial/key extraction must use a jointly reviewed bounded parser reservation; a
Request::sync scope does not intercept x509/ASN.1/OID or ordinary Vec allocations.
Borrow DER rather than clone it into an Invocation; fixed serial/key fields are
stack values. Any owned parsed metadata/copies retain their original admitted
owner. The shared R3/request parser source and allocation envelope remain a
specific pre-code gate, including malformed DER, long OIDs/extensions and error
paths; source-unjustified constants are not permitted. TLS possession and current
SQL certificate admission remain separate facts.

### Exact clocks, floor policy and auxiliary SQL credits

Native performs three actual trusted Clock reads at the original semantic points:
certificate admission, authorize_principal access, and (only after successful F0
decode) authority classification. Keep their respective native failure classes.
The no-SSO profile removes SSO network work, not the second clock boundary. None
refreshes the original local execution Deadline. The existing single-sample
HostListContext is not used as a substitute for these native stages.

Read authority state16 at Main entry and compare its entire binding to the
connection-owned bootstrap values. Check nonnegative initial authentication time
and `auth_micros/1000 >= clock_floor`; rollback-clock/fence failures are private
infrastructure refusal. Build a complete success frame or exact public-domain
status candidate first. Only then execute18 once with that initial time's checked
millisecond quotient, followed by state16/C4 exit checks and COMMIT. Thus an early
unauthenticated/infrastructure/trap/admission failure commits no floor and emits no
candidate; a known public domain error can commit this one private floor write.
No savepoint, generic Prove/Invocation, principal registry, intent or revision
mutation is executed. Existing A1's clock-before-fence behavior is unchanged;
this is the explicitly separate native List policy. FreshPublication is read-only,
checks state/fence again, and never advances the floor or schedules a third sync.
The authority table is outside schema59's data-dependency trigger set; verify this
and coordinate stability in tests instead of assuming it from table names.

| AUX operation family | Main maximum | FreshPublication maximum |
| --- | ---: | ---: |
| AuthorityState16 entry and exit | 2 | 2 |
| ListIssuedCredential45 serial probes | 20 | 20 |
| PersonalSsoPolicy36 | 1 | 1 |
| ListCurrentOwner46 | 1 | 1 |
| PersonalAnchor35 | 1 | 1 |
| PersonalCurrentCredential34 selected role | 1 | 1 |
| AuthorityClock18 deferred floor write | 1 | 0 |
| Total | **27** | **26** |

FreshPublication compares only the authority prefix actually observed. Pending
holds a fixed, non-forgeable internal decision journal, not a replayable public
proof: InitialRoute, OuterSyntax, AuthorityClock, NoOwner, NoAnchor,
InvalidSelectedRole, or Authorized. It retains bounded UID/credential, selected
owner bytes+length where observed, raw i64 role/visibility for an invalid selected
role, and typed role only after successful role conversion. Payloads contain no
unbounded AST, SQL row or diagnostic text. Frame/status ownership stays separate.

All branches require the same current certificate/UID/credential/kind, no SSO,
full fence/profile/dependency observations and fresh unexpired stored issuance.
InitialRoute/OuterSyntax/AuthorityClock stop there; they do not require an owner,
anchor or role that the original operation never established. NoOwner requires46
still absent. NoAnchor requires the same ordered46 owner and35 still absent.
InvalidSelectedRole requires the same owner, anchor present and same raw34 pair
still yielding the original permission result. Authorized requires the same
ordered owner, anchor and successfully converted role. An unexpected change is
infrastructure discard, not a newly substituted public error. The original input,
sequence and parser/clock error candidate are immutable; do not parse again or
invent a second failing authority-clock event to justify them. Exact dependency
comparison also prevents changes in other data the imported witness covered.
This prefix journal allows BadArguments+missing-head and owner-absent Permission
candidates to publish, while fresh certificate revocation/expiry discards them.
The26 fresh AUX calls are the longest prefix, not unconditional calls for every
candidate. Fresh expiry is an explicit stricter publication guard. The
helper's own three checks remain in its1,133 subtotal, not removed or counted
again here. AUX53 + native C4/helper1,439 = **1,492 charged credits**; the analogous
celld arithmetic is1,480, not evidence of an implemented celld contract. Unused
rollback credits are already inside C4's306/294. No retries, third interval or
extra status SQL are hidden in this sum. Any added query changes the reviewed
ledger before source/test execution.

### Entry trees, diagnostics and retained status

Run F0 under the captured original Request. Explicitly drop its owned complete AST
after authority resolution and before decode_kv_list_arguments; NLL alone does not
destroy an owned root. The second decode receives the original borrowed input,
then drops its tree before parent/List work. No cloned argument or simultaneous
full trees are required. Snowpack's ordinary allocation hooks remain in force.

Before EACH existing parser invocation reserve a separate source-derived diagnostic
envelope for Error::new path/tag clones, Error::Display's lossy tag temporaries and
error.to_string growth. These allocations occur before status framing. The exact
formula/source pin is under review with quota; input<=16384, depth64 and tag<=31
bound it, but no arbitrary multiplier or completed total is claimed yet. All error
text and ordering remain byte-equivalent. Malformed nested/high-tag inputs must
measure old/new realloc overlap, not only final String length. Once a status exists,
use existing encode_status_response_at and its actual Control-versus-Output
selection; retain original Request through final physical output. Framing failure
is infrastructure refusal, not replacement success or partial bytes.

### Private R3-to-authority bootstrap and native profile artifact

R3 owns native59 only. F3's consuming construction must install the exact trusted
authority DDL in that private destination, initialize its binding using trusted
R3/deployment ownership, verify the entire native59+authority union, and commit
under actual WAL/FULL before Ready. Never install target-observed DDL or call a
public generic Initialize handler. The existing authority schema comment marks it
qualification-only: its deliberate reuse requires this exact source review and
immutable policy artifact; it is not already a serving schema by implication.
Principals/intents start empty and no operation in this owner can populate them.

Proposed native serving artifact is a separate400-byte fixed record, not the
296-byte qualification/token profile. Header `FKNP`, u16 version1 and u16 length400;
then six LEu32 fields: profile_id1, target_native1, native_schema59,
restricted_physical1, personal_semantic1, native_entry1. Then ten32-byte SHA256
components in fixed order: native DDL, authority DDL, exact union, scalar catalog,
page catalog, compiled storage descriptor, personal import policy, connection
policy, native entry contract, and certificate import policy. Finally six LEu64
bounds:16384 argument,1048576 parent,65536 node,4096 dirent,4096 entries,2048 SQL.
Total8+24+320+48=400. Composite domain is
`foks-native-personal-list-profile-v1\0` followed by these exact bytes. Only the
compiled reviewed artifact can construct the private native profile; reject
high-bit qualification IDs and all other target IDs. Generate from trusted source
DDL/catalogs/policies, never the imported connection. The actual executable hash
belongs in an external immutable deployment manifest, avoiding self-reference.
No Wasm export ABI or token issue/advance authority is implied by native_entry1.

The exact authority Identity/Activation initialization mapping remains one concrete
joint R3/F3 source gate: host and validated key inventory come from R3; private
installation/tenant/owner/storage labels come from the consuming deployment owner;
schema_hash pins union, catalog_hash pins domain-separated scalar+page bytes, and
kernel_hash pins the native entry contract. Key generation/digest and placement
must have real R3/deployment meanings, not dummy caller-provided values. If those
existing qualification fields cannot represent the native owner honestly, review
a native-only typed binding before code rather than fill placeholders. Config
limits are mapped from validated R3 Config and the restricted physical bounds,
encoded and compared exactly for this owner. Bootstrap must fail closed before
Ready on any missing identity/key/config mapping, DDL/schema mismatch, failed
COMMIT or expired construction job. No previously minted R2/R3 diagnostic receipt
alone authorizes this transition.

Required combined oracle additions include real issued positive certificates;
malformed args paired with expired/revoked/unissued credentials; missing head with
invalid full-role encoding; two cross-column current-owner matches; real subkey
rotation and cross-owner rebinding; persistent TLS across floored DER expiry;
stored-expiry crossing between authentication and authority; exact three-clock
failure order; floor commit for domain status versus rollback for infrastructure;
fresh-time refusal; explicit F0-tree drop; near-limit error formatting; exact1,492
SQL upper schedule; and genuine backup→R2→R3→bootstrap→request/output ownership.
No service constructor/source tests proceed until the remaining named envelope,
bootstrap mapping and generated artifact source gates have both reviews.


Root review of the native schedule/floor direction and400-byte record arithmetic:
**PASS in direction**, with actual indexed46 EXPLAIN and the decision-prefix fresh
guard above required. The key_generation field has a concrete private meaning:
the monotonically numbered imported immutable key-bundle activation generation,
starting1 for this freshly created owner. It is NOT HostKeyGenerationId or a
hostchain sequence. Its digest is domain-separated canonical encrypted inventory
plus validated manifest, not plaintext key material or a claim that public keys
alone identify the bundle. Storage is defining the exact R3-owned bounded digest
handoff/encoding; future rotation would require atomic generation/digest change
and separate review. Native inline storage namespace/placement name the actual
private import binding, never an S3 namespace. The kernel_hash is a native
contract/version binding; external immutable deployment evidence identifies the
executable. No constructor accepts a caller-supplied hash to establish either.


### Proposed exact native bootstrap values and consuming handoff

Storage supplied this bounded key-bundle digest format for the joint handoff
delta (not a claim that current R3 already computes it): SHA256 of
`foks-native-personal-list-encrypted-keys-v1\0`, then sorted validated key entries
and key-manifest.txt, each `kind:u8 || name_len:u64LE || name_bytes ||
file_len:u64LE || SHA256(file)`. Freeze kind0=manifest, kind1=encrypted key and
sort by raw ASCII name bytes, rejecting duplicate names. The inventory is the
existing captured bounded roster, not a fresh caller directory listing. Reuse
captured content hashes only after final source-package verification and exact
copied-key equality plus existing decryption/ledger checks. Keep the digest inside
the consumed import owner; no public getter or deserializable hash constructor.
The receipt binds encrypted bytes and manifest, not plaintext key digests. The
original maintenance owner pays this construction traffic.

Proposed bootstrap construction is one private consuming function, with no
connection callback: `RestrictedListOwner::from_import(import, construction_job)`.
The job is a real finite admitted construction owner; replacing import's hook is
explicit, does not extend either job, and cannot expose an idle raw connection.
It fails if the stronger certificate audit or immutable compiled profile is not
present. Exact field moves preserve database-first drop order and all leases.
Before any SQL, replace the old maintenance hook through the fixed C API while
holding exclusive ownership; install the construction job's hook. No SQL runs in
that transition. After successful initialization and construction completion,
install an idle hook that refuses SQL until the next admitted operation replaces
it. Rejection/panic closes the owner; there is no half-ready recovery constructor.

For this first fresh private native owner, draw independent installation16,
tenant16, owner16 and storage_namespace16 labels from the trusted OS entropy
source, failing construction on entropy failure. They identify this actual owned
inline import instance and its whole-package tenant mapping; they are not claims
of an existing global installation or S3 resource. Retain these exact values only
inside the real owner. `Identity.host` is the verified R3 host ID. `Identity.cell`
is SHA256(`foks-native-personal-list-cell-v1\0` || target_incarnation32 ||
tenant16). `Activation.storage_placement` is SHA256(
`foks-native-personal-list-inline-placement-v1\0` || target_incarnation32 ||
storage_namespace16 || source_package_digest32). Path identity and WriterGuard
are checked/retained independently: a matching digest cannot authorize rebinding
to another connection/path.

Initialize the existing authority fields with version1, schema59,
schema_hash=compiled exact native+authority union digest,
kernel_hash=compiled native entry-contract digest,
catalog_hash=SHA256(`foks-native-personal-list-catalog-v1\0` || scalar_digest32 ||
page_digest32), key_generation1 and the validated encrypted-bundle digest above.
Set epoch1, revision1, high_water0, retained_rows0, reserved_bytes0, max_rows1,
max_bytes4608 and clock_floor0. Zero is meaningful only for this fresh owner with
fresh incarnation and no prior List observation; no reactivation/resume accepts a
zeroed old floor. The first successful domain candidate establishes its committed
floor. The single unused intent slot is a schema requirement, not permission to
register an intent; all intent/principal mutation operations are absent from the
native serving allowlist.

Activation Limits come from the actual validated import Config:
blob_bytes=min(maximum_kv_node_bytes,65536),
dirent_bytes=min(maximum_kv_dirent_bytes,4096), namespace_bytes and
namespace_objects from their Config fields, directories=maximum_kv_directories,
dirents=maximum_kv_dirents. Checked usize/u64 conversions and nonzero limits are
required. The separate parent transport bound is1MiB. These fields describe the
closed owner/config and do not authorize mutation or retroactively reject native
readable data merely for exceeding a write quota. Its request path compares the
entire expected typed Activation, not a caller-selected subset/hash.

The initialized identity/activation are emitted once with the existing typed
canonical serialization under a construction allowance, bounded to actual schema
ASCII/byte limits before insertion. The ordinary generic initialization API is
not invoked. Verify empty principal/intent tables and the exact initialized row,
then exact union/settings/dependency and COMMIT. Require actual WAL/FULL commit
success before Ready. On uncertain commit destroy the private owner and publish
nothing; a new genuine import is the recovery path for this checkpoint. Constructor
SQL/DDL has its finite construction-job ceiling, separate from per-request1492;
it never spends or refreshes a future request's deadline.

These exact values/encodings are proposed for root and storage review. They add
real private binding to the existing import owner, not a publicly constructible
readiness token. Native deployment executable provenance remains external; this
checkpoint does not establish a completed hosted deployment identity.

### Final F3 native qualification gate closure and source groups

The following exact decisions supersede the remaining alternatives above. Root
and independent review agree that `Activation.kernel_hash` is the composite
400-byte profile hash, including its domain, rather than only its entry-contract
component. The entry-contract digest remains one of the ten components: there is
no recursive hash and no claim to have hashed the running executable. Persisting
the composite makes the physical, import, connection and certificate policies
part of the full authority binding checked by every request and publication.

Construction consumes the original imported owner and **the same ScanControl**.
Before moving the connection, require the original Control to be valid and call
Control::intersect: cancellation Arc identity must match, deadlines take the
minimum, and sticky interrupt/failure cannot be reset. Certificate audit,
encrypted-key digest, authority DDL and bootstrap finish under that original
finite maintenance job. After success, the private owner enters an Idle state
with no SQL, Connection or cached-statement escape; all statements have dropped.
An idle progress hook at cadence1 is a supplemental short-VM defense, not proof
that arbitrary SQL/PRAGMA preparation could never run. Only a private admitted
state transition may replace the hook and enter SQL. There is no fresh
construction deadline capable of reviving a cancelled or expired import. A
separately admitted construction memory owner does not refresh time. The key
digest merges the manifest into the captured key roster by **raw ASCII name**,
not the capture roster's `(kind,name)` order; no second owned inventory is needed.
R3's stronger private handoff retains verified host33, canonical host name and
bundle digest32 before the source package drops. These values have no public
hash-based constructor or readiness getter.

For this checkpoint, both import and `restricted_list` remain
`cfg(any(test, feature="celld-proof"))`; `celld-proof` enables metadata-core's
native host dependency. Default native routes and A1 helpers are unchanged.
The public feature-only qualification entry consumes actual finalized backup
artifacts, operator root, Config and ScanControl; it cannot take a caller-created
import receipt, peer JSON, expected hash, arbitrary SQL callback or TLS config.
After genuine import and bootstrap, it owns a loopback qualification listener
and constructs its authenticated rustls configuration from the validated
canonical name and retained existing keys. A private read-only HostKeyProvider
adapter maps load-or-create to load-existing and refuses key creation/removal.
Thus TLS construction cannot repair or change the encrypted key bundle.

The qualification listener performs the real authenticated TLS handshake and
obtains the peer DER only from that negotiated stream. Before its one bounded
session copy, admit DER length (1..=4096), the exact Vec payload and Arc-inner
layout of the private SessionPeer under a physical session memory owner. No
request or Invocation makes another DER copy. Blocking SQL work borrows this
retained original through an Arc whose permit remains held until all detached
work and output owners release it. A waiting task cannot return the connection,
execution permit or request/output allowances while its blocking work continues.
The request input, exclusive connection and original execution lease move into
the worker; the returned Pending owns them through the fresh publication interval
and socket write/flush or deliberate abandonment. There is no connection loan
to caller code and no current-thread synchronous SQLite path that can block the
async listener. The source layout review pins the exact worker/result/hook/Arc
allocations before implementation is tested.

The closed owner is a whole imported personal server, not one end user's UID.
Its private tenant label names that actual one-package boundary. Schema59,
Config and key provenance come from the consumed import; none is supplied again
at request time. Restricted parent/node/dirent bounds are checked before owned
copies and cannot be increased by native Config. Config's namespace quotas are
bound owner settings, not retroactive read-denial rules. Only personal List is
allowed on the qualification listener, using the existing framed FOKS request
and response encoding. Unsupported routes do not gain a generic SQL path. This
checkpoint still does not enable a production hosted listener or prove celld.

#### Exact bounded certificate extraction

Use the locked `x509-parser 0.18.1` builder with
`with_deep_parse_extensions(false)`. This preserves the fields and relevant
acceptance of the native parser: its deep extension parser records recognized
extension failures in `ParsedExtension::ParseError`, rather than rejecting the
certificate; unsupported OIDs otherwise create owned copies that are unnecessary
here. Never construct/format a name, OID, parsed extension or parser error. The
only public-key test uses the borrowed `Oid::iter()` and numeric equality with
`[1,3,101,112]`, followed by zero unused bits and key width32. This is the same
iterator used by the locked no-bigint native `to_id_string()`, including its
acceptance of nonminimal arc encodings, with no string/BigInt formatting. Pin the
actual ASN.1 feature set (datetime/default/std/time; no bigint, debug or trace)
and compare nonminimal, huge and unterminated OID cases against the native oracle.

Accept a **nonnegative** serial magnitude, preserving the single-byte zero
representation already supported by the hosted helper. Negative raw serials and
magnitudes above20 bytes are explicit restricted-profile refusal. A leading DER
sign zero is stripped; stored serial must be one of the zero-padded width<=20
representations. No timestamp equality with stored SQL values is introduced.
The shared parser returns DER not-after as its original signed i64 seconds; the
import audit does not add a stored-vs-DER time equality or nonnegative DER-time
policy. At request time, after SQL binding, key equality and supported-kind
validation, perform the native Principal::authenticate nonnegative u64
microsecond conversion (principal.rs51-57), before the authorize clock and F0.
This preserves its failure precedence even though List's credential stores SQL
expiry. Borrow original DER and return fixed serial20+length, key32 and signed
not-after seconds; do not keep a parsed X509 tree.

For N=DER length<=4096, define A=floor(N/6), R=floor(N/8), E=floor(N/6), and
target sizes a=sizeof(AttributeTypeAndValue), r=sizeof(RelativeDistinguishedName),
e=sizeof(X509Extension). Before the shallow parser executes, reserve exactly the
following source-derived **upper allowance** under the original memory owner:

`X(N) = 4*A*a + 4*(R+2)*r + 4*(E+1)*e + N + 2*round_up(N,8) + 2*53`.

The vector factors come from nom7.1.3's initial capacity4 and pinned Rust Vec
doubling: cumulative requested allocation bytes are <=4*n elements for a
nonempty vector, or4 for an empty many0 vector. An allocated attribute requires
at least six distinct DER bytes; a successful RDN at least eight; an extension
at least six. Two possibly empty name vectors and one possibly empty extension
vector explain the constants. Failed partial children cannot be reparsed as
another field: DER containers advance past their entire content. This accounts
for allocations that are dropped on malformed input as well as retained trees.
The BigUint parser first reverses an N-byte copy, collects at most round_up(N,8)
digit bytes, and can shrink/relocate at most that many again during normalization;
include the latter even though canonical small serials usually need less. The
two ASN.1 time fields each have at most one static error-string allocation, whose
longest reachable literal is53 bytes. Any/OID/bitstrings otherwise borrow input;
the shallow path avoids the lazy extension registry and nested extension heap.
Exact target layout assertions and locked dependency/source pins accompany the
formula. Allocation tests measure cumulative requested bytes and old/new realloc
overlap for many attributes/extensions, malformed partial children, long leading
zero serials, invalid time literals and unsupported OIDs. No allocator/RSS bound
is inferred. Root review and independent review must approve changes to this
callgraph or formula before tests.

The import certificate audit uses the same parser and finite original Control,
with one admitted reusable parser workspace released only after parsed values
are physically dropped. Its two complete passes and at-most20 serial probes per
row are unchanged. It never accumulates certificate trees or a certificate set.
The ordinary request uses one parser allowance, retained through request/output
completion; fresh SQL publication reuses the immutable extracted peer context
instead of parsing or copying DER again.

#### Exact Snowpack diagnostic allowance

Before **each** F0/List decoder invocation, reserve
`(128+65)*P + 2*65*31 + (3*M+8) + (9*31+8)`, where
P=sizeof(PathSegment) and M=6179. At native P32 this is29,038 bytes. Depth65
includes the path pushed before the MAX_DEPTH64 rejection. The first term covers
the path growth overlap (old64+new128) and the later original128+error-clone65;
the second covers original and cloned31-byte variant tags at each depth. The
formatted error ceiling is23 prefix+5 offset+65*(1+3*31) path+2 separator+39
reachable message bytes. The last terms cover default String formatter growth
and one lossy tag Cow, including old/new allocations. AST/Box/value payloads
remain charged by their existing allocation hooks. Byte-equivalent fixed decoder
status strings also fit this allowance; framing/output charges remain separate.
Rust1.95 String/RawVec source pins and hostile formatting tests guard the formula.
Explicitly drop F0's AST before the second decode. Retain a resulting status with
the original Request through framing and physical transmission; no refund occurs.

#### Integrated source groups and review boundary

One shared worktree based on the committed R3 importer will contain the following
cooperating groups. Each author's source review and root plus independent full
integration review precede Cargo/tests and the single integrated F3 commit. This
is one group because a detached counter, fabricated Ready constructor or separate
fake peer adapter would not exercise the ownership claim.

* Core counter: `foks-metadata-core/src/sql_budget.rs`, its tests,
  `native.rs`, native budget tests and `lib.rs` reexport. Root owns this group.
* C4: `foks-server/src/restricted_list/connection.rs`, schema scratch and their
  tests. It uses the reviewed shared counter and both full native intervals.
* Integration: server `restricted_list/{mod,bootstrap,certificate,operation,
  output,profile,tests}.rs`, narrow memory/session and module/feature wiring;
  core catalog45/46 and staged native personal-credential API/tests; immutable
  source-generated profile/policy artifacts and reproducibility tests.
* R3 private consuming handoff: import owner/key audit/capture seams retain
  verified host/canonical name/digest, strengthen certificate inventory and move
  connection, leases and keys together. It cannot mint readiness independently.

The exact path manifest and shared APIs are frozen after R3 commit and before
source work. Additional paths require explicit integrated review; generated
production policy SQL must also join C4a's immutable corpus. No Markdown is
staged. The1,492 native SQL ceiling, two complete C4 intervals, request/physical
output ownership, native status ordering and genuine backup-to-TLS test remain
the integration acceptance criteria. Actual celld import/guarded host and process
memory admission follow this checkpoint; native tests do not stand in for them.

C4a qualification tooling closure (independent review PASS): the closed eight-entry
connection-policy JSON is the common SQL/readback inventory. The five shared
programs join native ON/OFF diagnostic equivalence and the actual celld corpus.
The trusted-schema, WAL and FULL readbacks are explicitly native-only: pinned
celld SQLite 3.45 diagnostics cannot qualify F3's rusqlite 0.40.2 / SQLite 3.53.2
connection. Actual native Rust connection tests prove all eight settings on that
engine. Normalization admits TEMP root 1 only for the exact empty-temp program,
and the pragma-table-list virtual table only for its exact no-attachments
program; unrelated IDs, SQL, multiplicity, writes and operands remain rejected.
This is an evidence-target distinction, not a worker waiver.

Native source ordering clarification, before integrated source review:
`net/session.rs` performs initial certificate authentication even when routing
fails, but a route error or unsupported route returns before
`authorize_principal` and its second clock. A recognized List route then runs
that second clock/no-SSO check, ordinary-credential restriction, F0 decode,
third authority clock and classification. This supersedes shorthand that placed
SSO authorization before every routing result. A Team selection in the admitted
R3 profile returns native `Expired` after classification: R3 proved the entire
team-token ledger empty. The private owner preserves that witness because it
retains exclusive writer ownership and exposes only the private authority/floor
write; no team-token writer is reachable. The schema59 dependency triggers do
not cover team_view_tokens, so dependency equality alone is not evidence of
continued emptiness. This reaches the AuthorityClock journal prefix and does not require
an unobserved personal owner/anchor. It does not enable team serving.

Ingress diagnostic amendment (root and independent review PASS): packet decoding
runs generic Snowpack validation before the routed 16 KiB method cap. Reserve
29,083 requested bytes on the original Request before decode_message, extending
the reviewed 29,038 formula by 3*(20-5) for the maximum usize decimal byte offset.
Input and decoded argument retain their separate 2*frame permit; generic AST
allocation hooks remain additional. Ingress errors are dropped without formatting
outer RPC diagnostic wrappers. F0 and List decode still each reserve 29,038.
The disposable native listener destroys its owner after infrastructure or early
authentication failure; this qualification does not establish production
availability or tenant recovery. Domain-status replies retain normal reuse.

Final source ownership/layout amendment (root and independent review PASS):
construct one private Work aggregate before spawning Main, with owner, call,
peer, control and execution in that field order. Consume it whole into the task;
errors, panic unwinding and an unstarted task drop the physical database and
argument before the original Request and release execution last. Pending has
the corresponding ordered ownership through Fresh and physical write/flush.
Ingress retains its earlier original Request Arc while control admission is
fallible, so rejected admission cannot release argument/input memory early.
Control deadline checks compare the captured Instant directly and allocate no
discarded std::io::Error.

Before each Main/Fresh spawn, admit max(size of actual closure payload, size of
Result<Pending>) from that original Request; reject over16KiB or alignment over16
and latch Request failure without spawning. Each actual task has its own retained
reservation. The capture and result are alternative phases; contained fields are
not charged twice. This is typed payload admission, not the complete Tokio task
allocation: Option<F>/Stage tags, padding, headers, scheduler storage and runtime
allocation retention remain process-capacity qualification work. Drop completed
JoinHandles promptly, without claiming this proves final runtime Arc release.
The qualification report now names20 source type sizes/alignments and live phases,
including NativeSql/Counter, Work/Pending, Request and control Arc layouts,
progress closure, peer, certificate workspace, scratch budget and schema cursors.
Inline stack observations are nonadditive and do not claim complete compiler
stack-frame or RSS bounds. Native counter/layout assertions and C4 actual Layout
checks accompany this ledger; a future Wasm adapter must qualify its own types.

The physical-output seam remains private and generic over AsyncWrite; production
still passes the negotiated TLS stream. Deterministic tests gate actual Pending
writes and flushes, check retained Request/execution/physical ownership, and
exercise completion, abandonment and the original transport timer. Dev-only
Tokio test-util advances the transport timer after genuine import/operation setup;
it does not advance std::Instant or claim that the separate control check expired.
The I/O phase cap exceeds the original deadline in these tests, and Pending never
receives a new deadline. A separate detached blocking-worker barrier proves its
leases survive an aborted waiter until the worker physically drops the result.


### F3 validation correction: actual native diagnostic layout

The first native execution found that Rust1.95's native `PathSegment` occupies
24 bytes with alignment8, rather than the assumed32. The implementation already
uses `size_of`; its unchanged formula is `193*P + 4030 + 18545 + 287`. The correct
native diagnostic allowance is27,494 bytes and the twenty-digit ingress offset
allowance is27,539. These values supersede earlier29,038/29,083 numeric examples.
The immutable entry descriptor and independent allocation assertions now pin
size24/alignment8 and use the tighter formula-derived totals. Parser growth,
variant-tag, formatter, source-toolchain and hostile-input terms are unchanged.
This is a native qualification layout, not a Rust ABI or Wasm-layout promise.

Root and author reviews passed for validation-c's five exact paths; independent
review precedes rerun. The original failed build/layout and sandbox-denial logs
are retained. Actual pinned celld completed the expanded469-program corpus
(466 direct and3 mapped), with13 Python and10 JavaScript policy tests passing.
These policy diagnostics do not qualify the complete celld serving path.

### F3 validation-d plan: retire corrupt page work without poisoning its status

The unsandboxed frozen-source run completed47 tests and found two status-path
failures: an out-of-range List time cursor and admitted malformed List metadata
returned private `Bounds` instead of the native `TransactionRetry`. Both enter
the actual owned List helper. `personal/list/allowance.rs::Budget::run` currently
calls `fail()` for every returned `PageFault`; a normal typed `Corrupt` therefore
irreversibly fails the original Request before the caller can frame its status
and run Fresh. This is independent of the corrected diagnostic-layout number.

Apply one private exception, only for `Err(PageFault::Corrupt)` when neither the
child nor its original parent is already failed after page work completes:
set the child's existing failed latch, retiring that page workspace, without
calling `parent.fail()`. Do not clear a latch, reset counters, refund admission,
or create a replacement owner. All reservations and cumulative retained debt
stay with the same original Request through the status and physical output.
Every other PageFault remains parent-fatal. Permission and Missing are decided
before PageWork construction in the current List call graph, so they receive no
new exception. An ignored child or parent allocation refusal followed by Corrupt
must still leave the parent failed; the earlier typed error provenance is retained.

The existing `Budget::run` entry guard must reject any attempt to reuse a retired
page and irreversibly fail the parent. Panic, reentry, counter overflow, SQL,
expiry, admission and fallible allocation errors keep their current behavior.
No new public state, readiness proof, status mapping or wire format is added.

The return path is part of this exception's proof. Page-local SQL buffers,
decoded values and diagnostics are physically destroyed before the Copy
PageFault leaves the hook scope. `read_with_encoder` immediately returns on that
error and drops its PageWork and partial encoder; it cannot finish partial List
bytes. OwnedListEncoder and ListEncoder have no failure-on-drop implementation.
`read_personal_list_owned` disarms its unwind guard on normal return only. Its
panic path remains parent-fatal. The server keeps its existing Corrupt-to-retry
mapping, checks the original Request, and must complete the same Fresh and
physical-publication gates. A concurrent parent failure remains sticky.

The exact source scope is five files in the shared F3 worktree:

* `foks-metadata-core/src/personal/list/allowance.rs`: the Corrupt-only child
  retirement branch; no allocation formula, counter layout or charge changes.
* `personal/list/allowance/tests.rs`: retain refusal, panic, overflow and provenance
  checks; prove healthy-parent/retired-child behavior, retained debt, fatal attempted
  reuse, every non-Corrupt error, and ignored local/parent allocation refusals
  followed by Corrupt. A poisoned owner cannot finish owned output.
* `personal/list/allowance/tests/allocations.rs`: keep all adversarial input,
  allocation-traffic, old/new overlap and physical-cleanup assertions. Distinguish
  normal Corrupt retirement from actual fatal refusal; every failed page remains
  locally unusable and all workspace admission stays held.
* `personal/list/workspace_tests.rs`: preserve late-row role/append precedence and
  no-second-SQL checks; assert the parent stays healthy only for ordinary Corrupt,
  then becomes failed on attempted reuse of the same retired work.
* `foks-server/src/restricted_list/operation/tests.rs`: retain both failing native
  status matrices and their successful Fresh requirement; assert original-owner
  health, add a corrupt later row after a valid prefix, and prove a real original
  Request allocation refusal cannot publish a retry candidate.

Root and independent plan reviews precede edits. Save before/after hashes and
perform author plus independent source reviews before any rerun. Root retains
the sole Cargo slot. The focused core allowance/workspace tests, original F3
regressions and full guarded integration remain the runtime acceptance gates;
prior expected statuses and error ordering must not be weakened. This Markdown
and the validation evidence remain unstaged.


### Guarded native List qualification committed (5967a5b2)

Committed `5967a5b2` after direct and independent reviews of the complete F3
source and every validation correction. The private route now consumes a genuine
validated personal import, authenticates the issued and current credential in
native order, executes Main and Fresh under one original deadline and shared
2,048-attempt SQL budget, and retains the original allocation/physical owner
through local socket write and flush or disposal. Ordinary listeners and public
FOKS wire encoding remain unchanged.

Final validation passed: core161; server228 in both default and celld-proof
builds, with4 existing opt-in S3/benchmark ignores each; external allocation5;
client/server integration12; explicit live Go v0.1.9 authority/compatibility1;
official RPC fixtures8; strict core/server lint and41-file format check. The live
Go flow does not invoke the private F3 List route. The pinned actual celld/Moto
SQL-policy run passed469 programs (466 direct SQL and3 mapped controls) plus19
behavior checks. Celld cleanup required SIGKILL and was reaped; that is not a
graceful runtime-termination proof.

The final49 source hashes,10 tested binary digests, review closure, initial
failures and final evidence are recorded in
`/private/tmp/foks-guarded-list-qualification.json`; commit binding is in
`/private/tmp/foks-guarded-list-commit.json`. The measured native PathSegment
layout correction tightened diagnostic allowance to27,494 bytes (27,539 with
ingress). The reviewed Corrupt-only page-retirement correction permits the native
retry status after dropping partial output, while infrastructure/allocation
failures and attempted page reuse stay fatal. A malformed-node fixture clears
its derived usage value while retaining actual database guards.

This qualifies the native personal List slice and pinned SQL compatibility. It
does not qualify production metadata mutation coverage, actual celld closed List
execution, distributed ownership/durable acknowledgment, process memory capacity
or successful15-second host/AZ recovery. Remote S3 reclamation stays report-only.
All Markdown remains unstaged. The separately reviewed A0 transport extraction
now precedes the actual celld runtime integration.
