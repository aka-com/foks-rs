# Actual-celld guarded personal List qualification

Status: active qualification, 2026-10-16. Native F3 and transport extraction A0
are committed. A1 is committed as `8e31944080e03651caaa3512169fe468f8375342` after
source reviews, harness tests, actual build/lint/probe and final evidence review.
A2 is committed as `dca38eecb08263a3594f70025df7a309ed3682a5`; its results are
in [the lifecycle plan](HOSTED_CELLD_RUNTIME_LIFECYCLE_PLAN.md). The next reviewed
phase is [the actual node baseline](HOSTED_CELLD_NODE_BASELINE_PLAN.md). Its
ordinary-binary approach supersedes the earlier proposed A3 process extraction
in this ledger; guarded integration follows under the reviewed
[node ownership plan](HOSTED_CELLD_GUARDED_NODE_PLAN.md), after baseline acceptance.
Keep this
file unstaged. The production metadata adapter,
backend choice and broader replication design remain deferred.

This is the next target-specific slice after native F3. It follows the ownership,
error-order and two-transaction contracts in
[SingleTransactionList](HOSTED_METADATA_SINGLE_TRANSACTION_LIST_PLAN.md), the
[connection qualification plan](HOSTED_METADATA_CONNECTION_QUALIFICATION_PLAN.md)
and the private context/import inventory in
[ReadBundle](HOSTED_METADATA_READ_BUNDLE_PLAN.md). The latest native F3 contract
supersedes older provisional interfaces in those documents; in particular,
SQL45/46 and native error-prefix replay must replace the older A1 authentication
shortcut in any new guarded List artifact.

The native F3 six-file production snapshot has independent source review evidence
in `/private/tmp/foks-guarded-list-native-operation-import-review.json`, bound to
manifest SHA-256
`1b22aa1145d0ee0008e957ac097c0c2d65929047e1fad8428c45d1157dc8c51e`.
That source receipt is not execution evidence. Native F3 subsequently passed
its qualification and was committed as `5967a5b2e3ff827f445c45b3304281c52d8e0870`,
including SQL46 indexed-work checks and actual Go interoperability. Its results
are recorded in `/private/tmp/foks-guarded-list-qualification.json`; they provide
the native reference and do not establish actual-celld qualification.

## Smallest complete outcome

One finalized personal-only backup is validated, streamed into a fresh actual
celld cell, validated on that target and durably finalized. A native mutual-TLS
frontend forwards only one admitted List call, its original sequence and bounded
trusted context. A dedicated closed worker/Wasm path executes bounded SQL on the
cell, commits, awaits root `storage.sync()`, performs FreshPublication and returns
the exact FOKS frame through an owned physical transport path. Repeat calls and
cell reactivation use persistent target metadata, with no whole-state snapshot,
temporary native database or native history reconstruction per request.

The first target remains restricted personal List, with the existing restricted
storage descriptor and 1 MiB parent-directory ceiling. Native-valid records outside
the restricted profile fail import. Team/SSO/federation, org64 qualification,
ordinary mutations, credential rotation and maintenance are separate work. The
only serving write here is the reviewed authority clock-floor update. There is
no FOKS network wire or cryptographic change.

Assume the configured strongly consistent S3 service is durable and reliable, as
requested. Still require celld's real bucket-mode durability/ownership proof;
local COMMIT, a successful HTTP exchange or an emulator is not that proof. Local
Moto exercises the real integration and failure paths, not AWS latency or AZ
recovery performance.

## Current gaps and reusable pieces

| Area | Current evidence | Missing boundary |
| --- | --- | --- |
| Native F3 | Committed qualification of the consumed R3 owner, two guarded intervals, owned response path, indexed SQL and Go interoperability | Apply and qualify these contracts on the actual celld runtime |
| Existing live proof | `foks-server-testkit/src/celld_proof/bridge.rs` downloads a snapshot, materializes SQLite and computes a delta | A direct bounded cell request; do not extend this projection path into the new adapter |
| Wasm SQL fixture | `tools/foks-metadata-spike/host.mjs` has working scalar/page transports | Its public storage/catalog/instance, fault controls, generic exports and per-entry counter resets are unsuitable for the closed owner |
| Durability fixture | `durability.mjs` commits and awaits `storage.sync()` | Fresh C4/auth/fence checks, original-operation ownership, and physical response lifetime |
| Import | R2/R3 validate and logically copy a finalized native package | A real celld destination, target-specific binding/finalization and reactivation contract; no `(hash, ready=true)` constructor |
| ABI/profile | Strict `HostListContext`, bounded codecs, shared List helper and immutable schema descriptors exist | Guarded artifact/export allowlist, target profile, phase state machine and native-compatible staged authentication |
| Capacity/output | Celld has residency controls, output gating and stream backpressure | A process-wide physical Wasm reservation and an explicit output lease that survives producer completion/cancellation |

The inspected local runtime is
`/private/tmp/foks-celld-lab/source-v0.6.2`; the previously qualified binary is
`/private/tmp/foks-celld-lab/bin/celld`, SHA-256
`92ededd2bcfebeb1e78cc2d97219f00214a79eb6606f0eef48208fca3f98c15a`.
This is an artifact pin, not a claim about current upstream releases. A patched
runtime needs a new source/binary pin and the affected policy/runtime gates.

## Runtime boundaries to resolve before enabling the slice

| Exact source boundary in the pinned celld tree | Required contract |
| --- | --- |
| `crates/celld/storage.rs`: `Cells`, `finish_open`, `close`, `storage_identity`, `sync_sample` | A JS storage object or schema cookie is not a stable physical connection identity. Use closed synchronous intervals and reconstruct the owner after replacement/reactivation. Keep celld's actual length/record limits and pinned platform objects. |
| `crates/celld/js/harness.js`: `transactionSync` | Its callback result is returned unchanged, including a Promise. Use only a closed non-async callback, an owner-private pending slot and strict identity comparison against a private completion marker before COMMIT. Never inspect `then`, prototypes or foreign getters. |
| `crates/celld/js/storage_ops.rs`: `op_storage_sync`; `storage.rs`: `sync_sample` | Sync must run after the outer transaction commits, on the root storage of the running event. It samples committed position plus epoch and uses the output gate's durability/ownership proof. Await it with Busy, deadline and resource ownership retained. |
| `crates/celld/runtime.rs`: `pool_limits`; `crates/logic/isolate.rs`: `admit`, `may_free`; `crates/celld/machine.rs` | `max_requests` bounds stateless affiliations; resident-cell limits and RSS pressure are separate policies. Neither establishes a per-operation reservation for Wasm backing memory, including retirement overlap. Add or prove the required native process owner rather than a JS-global semaphore. |
| `crates/celld/js.rs`: `read_response`, `op_response_stream_write`, `op_response_stream_close`; `http_streams.rs`: `response_stream_write`, `response_stream_close` | Buffered output is copied to Rust; stream writes complete after channel enqueue and close reports producer completion. Those events do not prove final transport consumption. Transfer a real output-memory/execution lease into the native response owner. |
| `crates/celld/engine_api.rs`: `HttpResponse`; `main.rs`: `runtime_response` and forwarding paths | Follow the lease through body/stream queues, local or forwarded responses, disconnect and actual final write/disposal. A Hyper body being polled or a JS stream being closed is not by itself a socket-flush acknowledgment. |

For the local qualification profile, propose two process-wide execution slots and
128 MiB of Wasm maximum-memory reservations: two times the pinned 64 MiB artifact
maximum. Count active and retiring instances together. These are initial test
limits, not a production sizing recommendation. Host input/output copies,
SQLite, V8 heaps, compiled modules, transport buffers and runtime tasks require
separate finite admission or explicitly measured capacity; 128 MiB is not an RSS
bound. Freeze their exact terms before the corresponding source group.

There must be no idle per-tenant Wasm instance. A slot is acquired before
instantiation with immediate refusal when full. It remains owned through detached
work, sync, Fresh and physical publication/disposal. Its memory reservation is
released only after the backing memory is actually destroyed. Wiping bytes,
setting `instance=null`, `WeakRef` observations or dropping a JS reference do not
establish that event. Freeze a concrete native lifecycle hook or isolated-runtime
destruction mechanism and demonstrate its release edge first. If the pinned
platform cannot supply it, record this as a blocked capacity prerequisite; do not
replace it with a timeout or optimistic credit return.

An admitted owned output copy is an acceptable transfer of memory ownership, not
an excuse to release all owners. Freeze the overlap and every copy before making
it; retain the original invocation/execution identity through the transport, and
retain the Wasm reservation through its actual destruction. A frontend waiter
timing out cannot release a still-running cell's resources. This may require a
narrow celld runtime patch. It does not choose the production metadata backend.

## Proposed phases and review gates

### A. Freeze and qualify the missing runtime contracts

Write the exact request/instance/output lease API, numeric admission limits,
destruction acknowledgment and cancellation/drop order against the boundaries
above. Include native-owned context/clock provenance, cross-process deadline
propagation and transport ownership. Start the operation deadline at the original
frontend first-byte ingress; neither admission, cell activation, HTTP retry, sync
nor Fresh can grant a fresh 15 seconds. A remote monotonic clock is not the same
clock: freeze an explicit conversion/remaining-budget contract with transport time and clock
uncertainty accounted for. The frontend's original monotonic cutoff always wins.

Pinned celld freezes JS time during synchronous execution. Date checks and a
Promise race cannot interrupt a long SQLite statement. Qualify checkpoint and
physical cancellation behavior independently; withhold late output and retain
permits when work overruns. `CELLD_OPERATION_DEADLINE_MS` is not a replacement for
this end-to-end contract, and the existing deadline diagnostics explicitly do not
claim a hard production deadline. Do not use a successful timeout as termination
evidence.

Use an immutable diagnostic worker to prove global saturation across different
cells/isolate realms, retained retirement credits and slow-output ownership.
This capability gate may precede the full importer. It does not enable FOKS
serving. If it needs runtime changes, review and pin that runtime separately and
requalify affected C4a policy/engine behavior.

### B. Construct an actual target-cell import

Use the real finalized package and existing source R2/R3/key validation under one
finite maintenance job. Copy logical rows in bounded, ordered batches into fresh
trusted DDL on the target, with no execution of source SQL and no whole-package
JSON materialization. Preserve the necessary rowids, scalar types and integer
precision. Ordinary List requests never invoke this importer.

Freeze the target validation roster: restricted value/whole-record limits,
complete native authority-conversion semantics, certificate inventory and key/
host binding, FK/integrity checks, exact FOKS59 + authority + pinned platform
schema, and dependency/placement identity. Define which checks run on source,
which run after target copy, and the exact-copy argument between them; do not
assume a source scan proves a partially copied destination.

Only trusted successful finalization creates the persistent target binding and
fresh incarnation, with key digest/source digest/profile/namespace placement
bound together. Root sync must prove that finalization durable before activation.
Interrupted/duplicate/out-of-order imports remain unservable. Reactivation reads
and validates this actual binding with the compiled target contract; it cannot
reuse a native receipt or prior JS admission. A partially imported target is
discarded or resumed only by a separately reviewed maintenance protocol.

### C. Build the closed List artifact and host together

Proposed FOKS boundaries are `tools/foks-metadata-host/guarded-list.mjs` and its
static worker composition, a new private guarded Wasm module under
`foks-metadata-core/src/wasm`, and a separate private native frontend/testkit seam.
Freeze exact paths and all import/export signatures before edits. Do not subclass
the spike Kernel or route through `RpcProof::execute`/NativeBridge projections.

Use the existing fixed binary List argument/sequence envelope and bounded trusted
context. The frontend must derive peer evidence from actual mTLS and bind the
authenticated private transport to the imported tenant/host/activation. A bearer
string supplied by an ordinary client cannot assert a principal. Context decoding
is structural validation only. Reconcile the existing context's expiry/time
fields with native F3's SQL45 admission, SQL46 current-owner selection, three
clock boundaries and checked DER-expiry conversion; do not call A1's stricter
historical-owner resolver and claim native parity.

The artifact exports only the reviewed allocation/response callbacks and closed
guarded phases. Exclude fixture, domain-dispatch, trace, arbitrary page, token and
directory-decoder entries. Use immutable compiled catalogs and exact target
profile hashes; no caller chooses SQL, DDL, profile or expected hash. A local
qualification profile remains explicitly distinct from production profile IDs.

Freeze the serving SQL allowlist separately from construction/maintenance:
scalar reads 16, 34–37 and 43–46; the sole scalar write 18; List/parent/role pages
2–8 and 10; schema page 9 only within C4. SQL33 and unrelated point/mutation
programs are unavailable. C4's five fixed host checks require `foreign_keys=1`,
`ignore_check_constraints=0`, `recursive_triggers=1`, empty TEMP and no
attachments, with exact local types/EOF. Preserve the target-specific schema
union and complete eight-row page-9 scan, including mandatory EOF. Do not claim
native-only trusted-schema/WAL/FULL checks on celld or expose transaction controls
as generic imports. Unsupported routes follow the reviewed route/error prefix;
they cannot manufacture authenticated public statuses before SQL admission.

The single owner holds Ready -> Main -> PendingSync -> Fresh -> Output -> Ready,
plus terminal Invalid, with a checked nonwrapping generation. It privately owns
root storage; no storage/cursor getter, callback, alarm, admin mutation or retained
context alias can escape. The same immutable worker handles positive and negative
qualification paths; failure injection must not introduce a serving callback API.

Main performs the complete C4 check, native-compatible admission/error ordering,
existing bounded List helper and exact frame/status generation, floor update and
exit checks before COMMIT. Then root sync. Fresh is a new read-only synchronous
transaction: repeat C4, compare target/fence/dependency/generation and replay only
the authority prefix actually observed, using fresh certificate validity and the
original deadline. It neither reparses the public call nor advances the floor.
Sync failure, uncertain COMMIT, changed binding, trap or expired deadline cannot
publish any candidate success/error frame.

One sticky 2,048-call budget spans Main, sync boundary and Fresh, including fixed
host checks, controls and prepaid cleanup. The existing 1,480 celld arithmetic is
a proposal, not a completed host count. The fixture's write adapter executes an
extra `SELECT CAST(changes() AS TEXT)` because `rowsWritten` includes triggers,
but counts only one import. Charge this companion attempt for the floor write,
or review a proven equivalent, and include its SQL in C4a. Count attempted
validation/preparation failures; no counter reset at an export or new transaction.
Every new reachable fixed SQL program joins the immutable policy corpus.

Reserve bounded context, argument, parser diagnostics, reusable page/schema
workspace, frame/status, host copies and task/control terms before allocation.
Use original-operation owners; never refresh a cumulative allowance after sync.
The 64 MiB instance bound is independent of these payload ledgers. No memory
growth or Wasm reentry may invalidate a borrowed frame during transport.

### D. Qualify the complete vertical slice on local celld

Use the same artifact/module, real target import and mTLS frontend. Reuse native
and pinned-Go raw argument/frame oracles, C4a tooling and local process management,
without inheriting the old snapshot bridge's performance claims. Freeze sources,
binary, Wasm imports/exports/memory limits, catalogs, profile and deployed bundle
before each run. Record before/after hashes, failures, exact response bytes,
SQL attempts, deadlines, resource acquisition/release events and child cleanup.

| Case | Decisive assertion |
| --- | --- |
| Positive repeat/cold/reactivated List; maximum restricted parent; sparse/maximum pages; sequence-width boundaries | Exact native/Go frame and status parity, persistent target use, no per-request whole-state reconstruction |
| Unissued/revoked/expired credentials; subkey rebinding; malformed outer/pagination; missing owner/head; invalid selected role; Time overflow after permission | Native F3 error precedence and Fresh prefix matching; no success with stale or fabricated principal |
| Truncated/replayed/reordered import, wrong source/key/profile/namespace, one-over value or physical record | Target remains unservable; source package unchanged; no readiness minted from supplied hashes |
| Settings/TEMP shadow/attachment/schema corruption; wrong incarnation/fence; cell replacement across sync | Refusal before publication, correct rollback/discard and reconstruction rather than cached-cookie reuse |
| Foreign callback result, Promise, Proxy/getter, recursion, wrong allocator phase, extra export/SQL ID | Closed owner rejects without invoking getters/traps or exposing storage; poisoned instance cannot be reused |
| SQL46 direct/subkey/miss with growing device population; schema maximum and mandatory EOF; failed prepare and exhausted counter | Bounded indexed work on actual pinned celld, one original counter including companion SQL and cleanup |
| Pause before COMMIT, during sync, before/after Fresh, before response and during output; then cancel/expire/fail/rebind | No early/stale frame; no early execution/memory release; committed floor may remain, but late publication is refused |
| Slow reader, disconnect, buffered/streaming/forwarded output and host-copy refusal | Byte bounds survive every queue/copy; original deadline is not an inactivity timeout; physical output owners release exactly once |
| Saturate two slots across cells/realms, retain one cancelled worker and one retiring instance, attempt another | Prompt refusal while both physical reservations remain; reuse only after demonstrated destruction/disposal, with no leaked or prematurely returned credit |
| Repeated large -> small -> large calls, traps, frontend/runtime shutdown, source reactivation | Cumulative charge correctness, actual high-water and retirement overlap recorded; no hidden idle per-tenant instance |

Prefer deterministic runtime barriers for lifecycle assertions. Diagnostic-only
fault hooks must be outside the public request surface and included in the exact
source review. Time-based polling is suitable for the external controller's
bounded observation, not as evidence that a worker or PUT terminated.

### E. Capacity baseline after D passes

First measure one/two concurrent active cells under the proposed fixed capacity,
then a reviewed larger process cap. Measure cold activation, Main, sync, Fresh,
transport and physical retirement separately; report p50/p95/p99/max, rejects,
timeouts, SQL calls, Wasm reservations/actual high-water, host-copy bytes, SQLite
and V8 residency, module cache, RSS, files and outstanding work. Repeated-call
traces must cover cancellation and retirement overlap.

Then run increasing idle-tenant populations, culminating in 10,000 only if setup
and machine capacity permit, followed by simultaneous reactivation at fixed
admission limits. Do not extrapolate a successful one-tenant or old snapshot
bridge run into a density result. A local 15-second observation is a measured
qualification result under its recorded load; hard worst-case responsiveness and
host/AZ recovery require their separate fault/load/environment qualification.

## Review, commits and claims

Each source group requires an exact reviewed plan/API/source manifest, author
source review and independent review before tests, followed by its separate
implementation commit. Keep Markdown unstaged. Group C cannot be committed as an
unguarded dispatcher; it may remain an unreachable qualification entry until B
and the runtime ownership contracts are satisfied. Preserve failed evidence and
re-review any source correction before rerunning affected tests.

Without the runtime/output lease work, permissible results are component
diagnostics or functional SQL/sync experiments only. They cannot establish the
reviewed process memory bound, resource release after cancellation, output memory
through socket transmission, 10k tenant density or 15-second worst-case recovery.
Even the complete local slice does not establish AWS/AZ timing, zero-loss
multi-owner replication, general FOKS write coverage, org64, receipt retirement,
or S3 object reclamation. Those remain separate plans; S3 reclamation remains
report-only until outstanding PUT termination is proved.

Author plan review: PASS after comparison with the current native F3 contract,
fixed SQL catalogs and pinned celld source boundaries. This is source/plan review,
not execution evidence. Concrete runtime destruction/output ownership APIs remain
phase-A prerequisites, not assumed capabilities. Root and independent plan review:
pending. No source work is authorized by a diagnostic success or by changing
these status labels.

## Reviewed outline and concrete phase-A proposal

The outline above, SHA-256
`c0e727056c65dd07408a731b4aa6bd6537a7c7ed857f042e2b225cd0d85db784`,
passed author, root and independent plan review. Receipts are
`/private/tmp/foks-celld-guarded-list-plan-{author,root,list}-review.json`.
This appended proposal narrows the first qualification topology; its own review
is required before code. Earlier HTTP/forwarding test rows apply only to a later
extension, not to the first result below.

### Select a co-located, disposable-isolate qualification profile

Run the native mutual-TLS FOKS frontend and celld runtime in one process. Hand
off the actual admitted request owner and original monotonic `Instant` through
typed native values. This avoids a new cross-process clock/lease protocol. The
cell still uses real celld storage, bucket-mode `storage.sync()` and lifecycle
coordination. It does not qualify a remote frontend, forwarded HTTP, generic
celld HTTP/stream buffers, Cloudflare Workers, or the eventual production topology.
All activation paths for this class require the native admitted invocation;
ordinary HTTP, alarms, WebSockets, service bindings and background warmup cannot
instantiate an uncharged guarded runtime. Maintenance construction has its own
finite reviewed owner. Reject unsupported ingress before placing this class.

Use one dedicated celld isolate for one cell and one invocation. The immutable
guarded Wasm has exactly one non-shared memory, with **initial = maximum = 64 MiB**;
validate its memory section, imports/exports and artifact hash before loading.
No positive memory growth, second memory, memory64, shared memory or escaping
memory alias is permitted. Check any zero-page grow path against the pinned
engine's backing/alias behavior as part of this gate. This prevents old/new
backing-store overlap from invalidating the two-times-64-MiB reservation. Immutable compiled module reuse
may be investigated separately; no tenant instance survives completion.

The narrower alternative of observing one Wasm memory inside an ordinary shared
celld isolate is not the first implementation. The pinned Rust V8 API exposes
backing-store shared ownership, but no production nonblocking last-reference
handoff for an existing Wasm memory; its public count assertion is a diagnostic
that can spin. A weak JS wrapper or custom ArrayBuffer allocator is not a proven
Wasm deallocation callback. That route needs a separately reviewed V8 binding/
engine change and growth/alias proof. See the pinned
[backing-store API](https://raw.githubusercontent.com/denoland/rusty_v8/v152.1.0/src/array_buffer.rs)
and [shared-reference API](https://raw.githubusercontent.com/denoland/rusty_v8/v152.1.0/src/support.rs).

Whole-isolate disposal has a concrete existing boundary: `Pool::free_retired`
takes and drops its `Worker` only after the slot drains. `WorkerIsolate` owns the
`SharedIsolate`; its final native destructor calls V8 isolate disposal. That is
stronger than `RuntimeManager::stop_cell`, which deliberately keeps the shared
isolate alive. The pinned
[SharedIsolate implementation](https://raw.githubusercontent.com/denoland/rusty_v8/v152.1.0/src/locker.rs)
also makes surviving lockers and deferred globals relevant to this proof.

The guarded profile must exclude backing-store escape: no structured clone,
SharedArrayBuffer, Worker Loader, dynamic import/eval, network/subrequest,
arbitrary callback, retained native `SharedRef`, leaked locker or native output
view into Wasm. Close all `Global`/local/view scopes and request work before
disposal; the sole output is a separately admitted owned native byte buffer.
Demonstrate that no backing-store handle survives Worker/isolate destruction.
Use deterministic retained-handle negative tests and native destruction observers;
RSS reduction alone is not the oracle. If the proof needs a new native disposal
observer, pin and review that observer rather than inferring completion from
`stop_cell`, a GC request or a dropped JavaScript reference.

### Concrete ownership and transport API

These are proposed private types/methods to freeze before implementation, not
new public caller-created readiness constructors:

| Native owner | Construction and terminal event |
| --- | --- |
| `GuardedCapacity` | One node-owned value shared across scripts, generations and retiring work; `try_admit` acquires one of two execution credits and one 64-MiB reservation before isolate load. No wait queue or per-JS-realm counter. |
| `GuardedInvocation` | Receives the actual first-byte frontend request owner, original deadline/cancellation, raw call and verified TLS facts; retains the same operation ID, sticky SQL counter and allocation ledger through every phase. It conveys no SQL authorization before admission. |
| `OwnedGuardedRuntime` | Owns the dedicated Worker, actual cell scope/epoch/slot, Wasm-memory reservation and execution reference. No raw Worker/storage/isolate getter. Its consuming disposal completes only after all turns, detached work and native handles are gone and native isolate destruction returns. |
| `GuardedFrame` | Created only by the closed post-Fresh path; a preadmitted native buffer plus the original request/execution reference, exact sequence, target binding and celld activity guard. It exposes only consuming transport, not a bare reusable byte result. |
| `RuntimeDestroyed` / `OutputDisposed` | Private completion edges from actual disposal or physical output. They cannot be supplied by JS, an HTTP header, a timer or a diagnostic JSON report. |

Keep memory and execution lifetimes distinct. The Wasm reservation releases at
proven backing destruction. The execution credit releases only after **both**
runtime/work destruction and frontend output completion/disposal. A shared
native lease retained by those two physical owners can express this without
early-credit booleans exposed to callers. Order fields so buffers, handles and
the Worker drop before their reservations. Destruction failure quarantines the
credit; a timeout never claims successful destruction.

Preserve the existing native request boundary precisely: its deadline begins at
the first FOKS application-frame byte after TLS, not after frame decode. Record
TCP/TLS setup separately and include it when reporting total client latency;
the application interval alone does not qualify connection-establishment time.

The frontend retains its original request memory through `write_all` and `flush`
under the original absolute deadline, or through dropping the frame and closing
the failed transport. This means completion at the local writer/kernel boundary,
not receipt or acknowledgment by the remote user. A partial frame already written
before cancellation cannot be recalled; close that connection and never publish
the remainder or reuse a stale completion. Do not use `poll_frame`, channel send,
JS stream close or producer completion as this terminal edge.

The integration must retain celld's `LocalRequestCompletion.activity` through
the same owned output rather than dropping it when the handler returns. Preserve
the existing output gate even though root sync has already completed. Every
additional platform wait still uses the original deadline and keeps physical
owners. The initial profile uses no `Response` clone/HTTP body serialization:
one fixed native capture copies the complete checked frame out of the private
Wasm view only after reserving the destination and accounting for the overlap.

After output is disposed, stop the exact scope/epoch through the celld coordinator
and retire its dedicated slot immediately. Keep a native Busy/retiring record
until that slot is physically destroyed, so a new call cannot adopt the old
runtime or create uncharged replacement work. Extend the existing
`AppHandle::evict`/`Effect::StopRuntime`/`RuntimeManager::stop_cell` flow with an
epoch-bound private completion if necessary; do not merely remove a runtime
registry entry while the coordinator still believes it is published. Failed or
cancelled calls enter the same retirement path. This first profile pays actual
isolate creation/activation/destruction per call and must measure those costs.

### Maintainable build seam and exact source groups

Celld has a library (`crates/celld/lib.rs`) as well as a binary, and exports
`RuntimeManager`, `AppHandle`, `Actor`, and `LocalRequestCompletion`. Its node
startup and service wiring still largely live in `main.rs`; copying that program
into a test fixture would create an unmaintainable second lifecycle implementation.
Extract a narrow qualification runner using the same startup, bucket gate,
coordinator and shutdown path. Ordinary builds do not install the guarded route.

There is a concrete linking obstacle: celld pins rusqlite 0.31/libsqlite3-sys 0.28,
while native FOKS pins rusqlite 0.40.2/libsqlite3-sys 0.38.2. Do not put both full
servers into one binary or silently upgrade celld's SQLite to resolve it. Use a
separate locked qualification workspace with celld and SQLite-free FOKS protocol,
crypto/TLS and transport ownership dependencies. The existing native R3 source
validator/exporter remains a separate maintenance executable; its successful
exit/JSON is not target readiness, and B still proves the actual copied target.

| Reviewed source group | Proposed paths and responsibility |
| --- | --- |
| A0: shared transport extraction | New `crates/foks-hosted-transport/` plus narrow call-site changes in `foks-server/src/{deadline.rs,net/memory.rs,net/session.rs}`. Move reusable first-byte deadline, request allocation owner and bounded framing/write support into SQLite-free code; preserve server APIs by reexports/limit/error conversions and reuse source/tests without parallel accounting implementations. Preserve native F3 regression coverage. Exact extraction API requires its own review. |
| A1: reproducible runtime patch/build | New `tools/foks-celld-guarded/{Cargo.toml,Cargo.lock,runtime.lock.json,prepare-runtime.py,patches/}` as a separate workspace. Pin upstream archive/tree, patch series, Rust toolchain/target, dependencies and output binary. Prepare a disposable verified source tree; never rely on manual edits in `/private/tmp`. |
| A2: native capacity and disposal | Celld patch to `crates/celld/{pool.rs,runtime.rs,engine_api.rs,js.rs}` and new private `guarded_execution.rs`. Add node-wide admission, dedicated one-cell/one-call placement, original control ownership, fixed artifact policy and consuming destruction. No changes to ordinary pool policy by implication. |
| A3: coordinator and owned output | Celld `actor.rs`, `actor/local_request.rs`, `lib.rs` and a narrow runner extraction from `main.rs`; integration `tools/foks-celld-guarded/src/{main.rs,frontend.rs}`. Carry activity/resource ownership through raw FOKS output, then coordinator-aware exact-epoch retirement. No generic HTTP or forwarded transport qualification. |
| A4: native clock and SQL cancellation | Celld `storage.rs`, `js/storage_ops.rs`, guarded host/ABI code. Install original-deadline progress/interrupt controls for the guarded connection, distinguish abandonment from actual SQL completion, and make native checkpoints read the original monotonic clock rather than frozen JS time. |
| B/C continuation | Existing genuine R3 source package/export path, `tools/foks-metadata-host/guarded-list.mjs`, private target importer and guarded Wasm module. Freeze the exact importer/ABI/artifact paths before edits; A is not a replacement for those gates. |

The inspected source archive is
`/private/tmp/foks-celld-lab/source-v0.6.2.tgz`, SHA-256
`9f0c391ee9b592e1c384dc696a8a3bb307b06f5d5dc57950cdff7573fa18b206`.
The recorded upstream tree inventory is
`/private/tmp/foks-celld-lab/source-tree.json`, SHA-256
`8ea2a3ea73908bd1780b90e6701d636e29cf340e9b8a0b09e53bcc99a6cb28b0`.
Resolve and record the immutable upstream commit/tree and verify source bytes
before producing the patch series; the directory name is not a source pin.
Keep the external qualification workspace out of the ordinary FOKS dependency
resolution so the two native SQLite engines do not collide.

Freeze the integration dependency/feature graph explicitly: `foks-metadata-core`
must remain native-disabled, and neither full `foks-server`, `foks-server-db` nor
testkit native dependencies may enter that binary. Prefer celld's existing public
library seams; extract only startup integration that cannot already be reused.
The qualification must identify its selected SQLite/V8 versions at runtime and
bind them into its profile rather than inheriting native F3's engine identity.

Keep A0 narrow: `foks-hosted-transport` owns the moved `Request` and depends on
`foks-metadata-core` with `default-features=false, features=["owned-list"]`, plus
the existing protocol/Tokio dependencies required by transport. It implements
the existing `ListRetainedOwner` trait on its own local `Request` type, satisfying
the orphan rule, and preserves `admitted_request`'s metadata-core error type.
Core never depends on transport: its Wasm build acquires no Tokio/runtime edge.
Keep server-specific `SessionLimits`/error conversion at server call sites and
reexport the shared implementation. Do not introduce a second accounting engine
or move an ownership trait merely to avoid this straightforward layering.

The qualification dependency graph and the Wasm graph are separate checks. The
native FOKS workspace may activate metadata-core/native for its own existing
tests; that does not authorize it in the separately locked celld executable.
Freeze the exact feature tree and inspect the actual linked SQLite/V8 identities
before attributing results to either target.

Before builds, inventory free disk, toolchain availability, V8 build/download
artifacts and target-directory budgets. The current machine has roughly 7.4 GiB
free and an existing shared FOKS target; that is not an established celld build
budget. Keep the separately locked runtime build artifacts distinct, reserve a
single coordinated Cargo slot with limited jobs, and retain native validation
evidence. Do not delete unrelated targets or start a second large build to hide
an unmet disk prerequisite. Source/plan reviews precede dependency resolution
and build execution.

A1 must provide an executable preparation/build recipe. `prepare-runtime.py`
verifies the pinned archive/tree and patch inputs, creates only its ignored
`tools/foks-celld-guarded/runtime/source` tree, applies the reviewed patches with
no fuzz and writes a post-patch hash manifest. The nested Cargo manifest points
to that path and has its own reviewed lockfile. Record the Rusty V8 native archive
and compiler/target; the previously downloaded celld executable supplies no Rust
library artifacts. After reviews and disk admission, the recipe builds with
`--locked -j 2`, a dedicated `CARGO_TARGET_DIR`, `CARGO_INCREMENTAL=0`, and dev/test
debug information disabled. Do not trigger a full local V8 source build by
silently falling back from a missing pinned native archive. A later supported
upstream hook may replace the patch series after its own qualification.

### Added freeze and verification requirements

The 2,048 sticky attempt budget counts admitted application SQL, fixed C4 checks,
application transaction controls and the floor-write `changes()` companion.
Celld-internal sync/position/replication/transaction plumbing is a separate
platform-work ledger; do not claim the application counter bounds every hidden
SQLite operation, I/O or CPU step. Requalify the exact target engine/policy and
measure those platform terms. Root sync uncertainty leaves the target unservable
until actual durable binding recovery; it never mints a ready receipt.

Every new connection/reactivation applies the fixed settings during construction
outside a transaction, then reads them back before request admission. In
particular, the inspected celld runtime starts with `recursive_triggers=0`;
the guarded profile must set it to 1. Request C4 reads settings and refuses drift;
it does not repair them inside Main or Fresh.

Add deterministic barriers at before-instantiation, active SQL, pending sync,
pending native capture, blocked TLS write/flush, outstanding driver task,
remaining native backing handle and just before/after isolate destruction.
For each barrier, cancel or shut down and verify which of the two owners still
holds execution and which holds memory. Refuse a third call until both required
credits can actually be reused. Exercise normal success, early failure, panic/
trap, delayed child cleanup, generation replacement and repeated cold calls.
Reject altered artifact memory declarations and import/export inventories before
instantiation. Keep exact frame/status oracles from native F3.

A native SQLite interrupt is not a proof that blocked filesystem/VFS work has
ended. The frontend can stop publication at its deadline while physical cleanup
continues to consume capacity. Record such a run as responsiveness/availability
failure where appropriate; do not turn termination uncertainty into a 15-second
success claim. Qualification must report creation, activation, Main, sync, Fresh,
native copy, local output and actual destruction separately, including cancelled
and retiring overlap.

Appended proposal author review: PASS after checking the real library/build
boundaries, distinct SQLite link dependencies, existing activity/output path,
isolate disposal ownership and the SQLite-free trait/feature graph. This remains
a plan/API proposal; the detailed signatures and platform patch source still
require their own reviews. Root and independent review: pending.
No implementation, dependency resolution, Cargo command or runtime test has run
for this proposal.

## A0 exact extraction proposal

The concrete phase-A appendix through its previous ending has now passed author,
root and independent review at SHA-256
`23701ce0cc58d9d1183d6c5c8c52cb26322ae8a9fd28016b82aa94389741c106`.
The corresponding root and independent receipts are
`/private/tmp/foks-celld-guarded-list-plan-phase-a-root-review.json` and
`/private/tmp/foks-celld-guarded-list-plan-phase-a-list-review.json`.
Those reviews require deterministic activity completion before coordinator epoch
stop and dedicated-slot destruction. Overdue physical cleanup continues to hold
its credits; expiration never grants a fresh publication budget. These are later
runtime requirements, not properties supplied by A0.

A0 starts only after the F3 implementation commit and this exact extraction plan
have passed root and independent review. Rebase the implementation onto that
commit, inventory its final source hashes, and freeze the edit manifest before
editing. This is a source move and a dependency boundary, not a new transport,
new request profile, or a celld execution qualification. Markdown remains
unstaged and outside the implementation commit.

### Shared API and ownership

Add `foks-hosted-transport`, with `default=[]`, `qualification=[]`, and
`test-support=["qualification"]`. Its production dependencies are
`foks-metadata-core` with `default-features=false, features=["owned-list"]`,
`foks-rpc`, `foks-snowpack`, `thiserror`, and Tokio with only `io-util`, `sync`,
and `time`; enable `rt`, `rt-multi-thread`, and `macros` for its tests. Declare
Tokio directly with the existing version requirement and `default-features=false`
rather than inheriting the workspace's `full` feature set. Use an explicit path/default-features declaration for the core dependency so
workspace feature defaults cannot weaken that boundary. Existing lockfile
versions remain selected; this step does not upgrade dependencies.

Move the following implementation once, keeping its data layout, field drop
order, admission arithmetic, fixed permit slots, sticky failure, thread-local
scope restoration and trait behavior. Server modules reexport the moved types
and scope helpers. There is one `Request` type and one current-owner TLS slot
across the ordinary server, native F3 and the future celld frontend.

| Surface | Exact proposed contract |
| --- | --- |
| `memory::PoolLimits` | Private fields; `new(response_bytes, working_bytes, control_bytes, maximum_in_flight_requests) -> Result<Self>`. Checked construction precedes every semaphore allocation. |
| `memory::Pools` | `new(PoolLimits) -> Arc<Self>` and existing `request(self: &Arc<Self>, Option<OwnedSemaphorePermit>) -> Arc<Request>`. All pool fields remain private. |
| Qualified construction | `Pools::admitted_request(self: &Arc<Self>, Option<OwnedSemaphorePermit>) -> foks_metadata_core::Result<Arc<Request>>`, compiled only with `cfg(any(test, feature="qualification"))`. Preserve the exact Arc-layout precharge before allocation and its existing `Bounds` failure. |
| `memory::Request` | Existing `failed`, `classify_input`, `sync`, `run`, and allocation/response/`ListRetainedOwner` implementations. No new layout field or independently reconstructed owner. |
| Ordinary work charge | `admit_working(input_bytes, Workload) -> bool`, where `Workload` is `Ordinary` or `ChunkDownload { maximum_encoded_bytes: usize }`. This replaces the server-module constant dependency; the accounting algorithm remains in the moved implementation. |
| Detached ownership | Existing `current`, `retain_work`, `RetainedWork::run`, `RetainedValue::into_inner`, and `retain_future`; keep work/result/future destruction before retained-owner release. |
| `deadline` | The same `Deadline(Instant)`, `after`, `instant`, `remaining`, `current`, `check`, `expired`, and synchronous `scope`. No new clock, deadline capture, deserialization or async thread-local scope. |
| `framing` | Existing `read_frame_ingress`, canonical length decoder, `phase_deadline`, and `write_response_until`. The first received FOKS frame byte starts the budget. `write_response_until` borrows bytes and completes local `write_all` plus `flush`; it does not construct an owned reply or prove peer receipt. |

For a chunk download, the server passes its existing
`chunks::MAXIMUM_ENCODED_CHUNK_BYTES`; the shared implementation still charges
`input_bytes.saturating_mul(2) + maximum_encoded_bytes.saturating_mul(8) + 4096`,
using the existing saturating additions. The ordinary branch has zero download
allowance. Preserve the `input<=4096 && ordinary` small-request classification,
bulk admission before working admission, and retained bulk credit on a later
working refusal. `net/session.rs` maps its existing route boolean to this enum;
`net/session/kex.rs` passes `Ordinary`. Do not move or duplicate the chunk-size
constant, change its meaning, or add a transfer field to every `Request`.

`PoolLimits::new` requires nonzero response/control capacities, total working
capacity at least `SMALL_HEADROOM` (16 MiB), all byte capacities no larger than
`min(u32::MAX, Semaphore::MAX_PERMITS)`, and in-flight capacity in
`2..=Semaphore::MAX_PERMITS`. It preserves the split into working minus 16 MiB,
small headroom 16 MiB, and bulk `min(in_flight.saturating_sub(1), 4)`. The new crate
has no default tenant policy. The stricter existing `SessionLimits::validate`
remains unchanged, including its 512 MiB working minimum and response/control
minimums. A small test fixture cannot silently become a valid server profile.

In `foks-server/src/net/memory.rs`, retain a thin
`pools(SessionLimits) -> crate::Result<Arc<Pools>>` adapter. It first calls the
existing `SessionLimits::validate`, then constructs `PoolLimits`, maps a rejected
memory limit to `Error::Config("invalid session limit")`, and calls the shared
constructor. Mechanically replace internal `Pools::new(SessionLimits)` calls
with this adapter; fallible production contexts propagate the existing server
error, while restricted qualification maps refusal to its existing qualification
configuration error. Known default test construction uses `unwrap`. Public
`Config`, `SessionLimits`, ordinary listener behavior and F3 qualification
signatures stay unchanged. There is no unchecked constructor or new panic path
for invalid external configuration.

### Errors, framing and source scope

The transport crate owns `Error::{Config(&'static str), Io(std::io::Error)}` and
`Result<T>`. Preserve the old error display strings and sources, including
`invalid server configuration: ...` and `server I/O failed: ...`, because the
frame-ingress overflow path currently embeds the deadline error into
`foks_rpc::Error::Io`. Add a server-local `From<transport::Error> for Error` that
maps directly to existing `Config`/`Io` variants, without a wrapper variant.
`Deadline::after` still reports `request deadline overflows monotonic clock`;
expiration remains `TimedOut` with `request deadline expired`. Framing and output
retain `frame deadline expired` and `response deadline expired`; socket I/O
errors preserve their kind and source.

The server deadline module reexports `Deadline`, `current` and `scope`, with thin
`check`/`expired` conversions preserving their existing server return types.
The session module reexports `read_frame_ingress` and wraps the shared
`phase_deadline`/`write_response_until` only to convert errors. Its ordinary
`read_call_body_async`, input semaphore waiting, TLS handling, route dispatch,
connection admission and timeout cleanup remain in the server. Do not move a
partial native F3 authority constructor into this generally reusable crate.

The decoder's byte/u16/u32 helpers move with framing. Its old test-only
`read_frame_length_async` need not become a public API: the two server input
admission tests can call the real `read_frame_ingress` and retain only its length.
Their body-memory oracle stays unchanged. Move the slow-socket memory test to
use the shared writer with an explicitly created deadline; remove the now-unused
server test-only `write_response` wrapper. Existing session tests continue to
exercise body, execution and output against the same original deadline.

### Real test seam and feature isolation

Move the private memory invariant tests into the new crate. They may still
inspect its private permit slots and inject poison/wrong-pool/overflow conditions;
do not expose those operations publicly. Move the two deadline scope tests and
add focused boundary/adapter tests needed to show the extraction preserves
existing errors, accepted limits and charges. Keep the frame decoder differential
oracle against `foks_rpc` and deterministic write/flush barriers for physical
retention; do not add timing sleeps to establish ownership order.

Keep `foks-server/src/net/memory/owned_list_tests.rs` in the server. It uses real
`NativeSql`, rusqlite and server-db fixtures and must not become a dependency of
the transport crate or the standalone celld executable. Its current direct
private-pool access is replaced with this narrow explicit test seam:

- `#[cfg(any(test, feature="test-support"))] memory::test_support` exposes
  `Capacities { working, small, output, control, bulk }` and
  `pools(Capacities) -> Result<Arc<Pools>>`. All numeric capacities are checked
  against the same semaphore/u32 bound before construction; zero is allowed only
  in this test seam. It returns the real implementation, not a substitute owner.
- `snapshot(&Pools) -> Snapshot` returns the five available permit counts by
  value. It returns no semaphore or reference to mutable bookkeeping.
- `try_hold(&Arc<Pools>, PressurePool::{Working, Output}, usize) ->
  Result<Pressure>` performs immediate checked acquisition. `Pressure` is opaque
  and releases only on drop; no split, merge, forget, release or raw-permit API.
  Over-range input is a configuration error; exhausted/closed acquisition maps
  to `Io(WouldBlock, "test memory pool unavailable")`.
- The private native test helper receives its already-known `Arc<Pools>` along
  with `Arc<Request>` instead of extracting `owner.pools`. The exact page-local
  credit, sticky refusal, output pressure and drop-order assertions are retained.

Dependency crates do not receive `cfg(test)` merely because the server is being
tested. Therefore `foks-server` enables transport `test-support` in its
**dev-dependencies**, and forwards transport `qualification` from its existing
`celld-proof` feature. The new crate's default public build exposes neither
`admitted_request` nor diagnostic constructors; the future harness enables only
`qualification`. The server's existing `celld-proof -> metadata-core/native`
edge remains for native F3 and must never be inherited by the celld harness.
Use external-consumer compile checks, including paired positive/negative doctest
snippets under the relevant features, to verify this behavior without relying on
the implementation crate's own test cfg.

### Exact proposed edit manifest

The following is the initial closed A0 edit manifest; add a path only after
reporting why the reviewed API cannot be implemented within it. File moves count
as edits to both old and new locations. Existing `config.rs`, core ownership
traits, SQL catalog and Wasm sources are deliberately outside this group.

```text
Cargo.toml
Cargo.lock
crates/foks-hosted-transport/Cargo.toml
crates/foks-hosted-transport/src/lib.rs
crates/foks-hosted-transport/src/error.rs
crates/foks-hosted-transport/src/deadline.rs
crates/foks-hosted-transport/src/framing.rs
crates/foks-hosted-transport/src/memory.rs
crates/foks-hosted-transport/src/memory/tests.rs
crates/foks-hosted-transport/src/memory/test_support.rs
crates/foks-server/Cargo.toml
crates/foks-server/src/error.rs
crates/foks-server/src/deadline.rs
crates/foks-server/src/net/memory.rs
crates/foks-server/src/net/memory/owned_list_tests.rs
crates/foks-server/src/net/session.rs
crates/foks-server/src/net/session/kex.rs
crates/foks-server/src/restricted_list/mod.rs
crates/foks-server/src/restricted_list/output.rs
crates/foks-server/src/restricted_list/operation.rs
crates/foks-server/src/restricted_list/operation/tests.rs
crates/foks-server/src/restricted_list/bootstrap.rs
crates/foks-server/src/restricted_list/profile/tests.rs
crates/foks-server/src/restricted_list/certificate/tests.rs
crates/foks-server/src/restricted_storage/import/tests.rs
tools/foks-hosted-transport-check/Cargo.toml
tools/foks-hosted-transport-check/Cargo.lock
tools/foks-hosted-transport-check/src/main.rs
tools/foks-hosted-transport-check/check_graph.py
tools/foks-hosted-transport-check/test_check_graph.py
```

The standalone check fixture is a small nested `[workspace]`, not celld itself.
Its native-target dependency is transport with `default-features=false`; it
selects default, qualification and test-support in separate checks. It also has
an explicit native-disabled core/owned-list dependency. Its Wasm target excludes
transport and exercises only that core edge. `main.rs` uses public APIs to prove
trait/error type compatibility; it uses no server fixture or full server crate.
`check_graph.py` reads saved `cargo metadata` JSON, follows only selected-target
normal/build dependency edges from this fixture, and checks active features. Require a resolved fixture root and complete reachable
package/edge data; missing or malformed evidence fails closed. Its small Python
test suite checks an allowed graph, each forbidden production edge/feature,
dev-only exclusion, selected-target filtering and missing-root/data refusal.
It rejects `foks-server`, `foks-server-db`, `foks-server-testkit`, rusqlite,
libsqlite3-sys and
core/native in the native transport closure; it additionally rejects transport
and Tokio in the Wasm closure. Record `cargo tree --edges normal,build,features`
with the same target/profile as readable corroboration. Do not infer absence
from package names alone or ignore target/feature resolution. Its own checked-in
lockfile keeps this proof independent of native FOKS workspace feature unification.

### A0 review and validation gate

After implementation, perform an author review of the entire move and a second
review using the independent agent before any test execution. Review source
identity, owner layout/drop order, limits/error conversion, dependency cfg and
test oracles; root reviews the same frozen hashes. Resolve findings and repeat
both affected source reviews before rerunning tests. Preserve failures and
source hashes in a qualification ledger.

Review the manifest/source proposal before resolving dependencies. With the
single coordinated Cargo slot and after disk preflight, update only the root
and standalone-fixture lockfiles, retaining existing selected versions. Review
those exact lockfile changes and freeze them with the final source manifest
before testing; resolution is recorded separately from compilation or tests.
Then run the new crate's default, qualification and test-support
unit/public-API/doctest checks and strict all-target lint. Run the server's default
and `celld-proof` library suites and strict all-target lint, including existing
native F3, real native owned-list, input/body/deadline, detached work, and blocked
output regressions. Run the graph checker's Python tests before accepting its
closure report. No expected ignored deployment tests become passing evidence.
Run the standalone native closure checks in each profile, the core-only Wasm
closure check, and the existing Wasm build/policy checks using the previously
qualified feature set. Inspect lockfile changes for unintended upgrades.

A0 acceptance requires unchanged normal server API/errors and accounting
behavior, retained native F3 success/refusal oracles, no native SQLite edge in
the standalone transport closure, and no transport/Tokio edge in the Wasm core
closure. Commit A0 separately with implementation and repeatable check fixtures
only after those checks pass; leave this plan and other Markdown unstaged.
A passing A0 does not demonstrate celld compilation, Wasm backing destruction,
physical runtime capacity, S3 sync, a 15-second recovery bound or production
metadata readiness. A1 still needs its own exact patch/build source freeze and
reviews before dependency resolution or runtime work.

A0 proposal author review: PASS after checking all current constructor and
private-pool test callers, the local-trait implementation rule, the chunk charge
seam, deadline/error propagation and the separate native-disabled feature graph.
The review corrected the diagnostic test boundary, added fail-closed graph
checker tests and kept dependency resolution behind an explicit source review.
Root and independent review: pending.
No A0 implementation, dependency resolution, Cargo command or runtime test has run.

### A0 reviewed-plan status

Author, root and independent A0 plan reviews passed at full-document SHA-256
`131ded5230949ccfb0d015374ec174a8b7c33b80c42be7c59cee6ac734571741`.
Their receipts are `/private/tmp/foks-celld-guarded-list-plan-a0-author-review.json`,
`/private/tmp/foks-celld-guarded-list-plan-a0-root-review.json`, and
`/private/tmp/foks-celld-guarded-list-plan-a0-list-review.json`. The reviewed body
above remains byte-for-byte unchanged.

Carry the reviews' explicit gate into source and evidence review: graph evidence
must bind the selected target and feature profile to the actual
`cargo metadata --filter-platform <target>` invocation and its output hash.
Missing or ambiguous target evidence fails closed; the checker cannot obtain a
pass by skipping an unclassified edge. Verify external-consumer feature absence
as well as positive qualification/test-support access. The path-only proposal is
recorded at `/private/tmp/foks-hosted-transport-a0-proposed-paths.txt`; it is not a
source/build freeze. A0 still waits for the F3 implementation commit and a frozen
source-edit boundary. No A0 source or dependency-resolution work has started.


## A1 exact reproducible celld library build proposal

This is a finite build, link and provenance slice after the reviewed A0 transport
extraction. It does not change A0. Its initial celld patch series is empty: the
existing public library suffices for a real engine identity probe. Dedicated
isolate placement/disposal remains A2; shared node-runner extraction, coordinator
activity and physical output remain A3; original-clock and SQL controls remain
A4. A passing A1 is not a serving, recovery, durability or capacity result.

Implementation starts from the completed F3/A0 commits, in a worktree under
`QMUX_WORKTREE_ROOT`, after this plan receives root and independent review.
Keep Markdown unstaged. Root retains the sole Cargo slot. This draft was prepared
by reading existing local source, archives and caches; no dependency resolution,
Cargo/build/test command or network download was performed for it.

### Immutable upstream inputs

Use celld 0.6.2 at commit
`90b43017241f81189453d326d05948f388b34652`, Git root tree
`8f52d32fd372bbc5d396fd73b47fc653ec78d5ee`.

| Input | Verified local identity |
| --- | --- |
| `/private/tmp/foks-celld-lab/source-v0.6.2.tgz` | 1,641,375 bytes; SHA-256 `9f0c391ee9b592e1c384dc696a8a3bb307b06f5d5dc57950cdff7573fa18b206` |
| `/private/tmp/foks-celld-lab/source-tree.json` | 93,898 bytes; SHA-256 `8ea2a3ea73908bd1780b90e6701d636e29cf340e9b8a0b09e53bcc99a6cb28b0` |
| Upstream `Cargo.lock` | SHA-256 `d6a17422788aa910ba76bed9cf59ac15767f93fe107ebdb336954d0e9efe052b`; 419 packages, including 416 registry packages and three local packages; no Git dependency |
| Official commit response, retained outside source | `/private/tmp/foks-celld-v0.6.2-commit.json`, SHA-256 `121bbce8b116323a2158d08238ffe62d13a8d832b3bb16842c803cf3c5795dbb`; explicitly binds that commit to that root tree |

The archive has 279 regular files and 49 non-root directories under
`denoland-celld-90b4301`, with 6,326,405 regular-file payload bytes. Every file's
Git blob hash and every recorded subtree hash were reconstructed and matched.
The independently reconstructed root tree matches the official commit response.
The existing extracted source has the same roster, contents and executable-bit
modes. The inventory JSON's top-level `sha` is the commit label, not the root
tree hash; do not confuse them in the preparer.

`runtime.lock.json` is a versioned, closed input record. It pins those archive,
inventory, commit/tree and upstream lock identities; the target and exact
compiler; selected V8 crate/native assets; ordered patch-series hash; and the
expected engine/package/feature policy. Commit the exact upstream inventory as
`upstream-tree.json`. Output paths, post-patch manifests, dependency-resolution
receipts and binary hashes belong in separate generated evidence, avoiding an
input/output self-hash cycle. Qualification records also bind the actual A0/FOKS
source commit and dirty-file hashes; a Git commit label alone is insufficient
when sources are modified.

### Source preparation and patch seam

`prepare-runtime.py verify` accepts an explicit local archive and checks exact
size and SHA-256 before parsing. Validate a closed archive roster, root prefix,
regular-file/directory types, UTF-8 relative names, safe path components, unique
paths, permitted modes, bounded entry counts and payload sizes. Reject links,
special files, traversal, absolute paths, extra/missing entries and mode changes.
Do not call unrestricted `extractall`. Stream extraction into a new private
staging directory beneath the ignored runtime directory and verify every blob,
subtree and root hash against the committed inventory.

`prepare` publishes only the verified staging tree as `runtime/source`. Reuse an
existing tree only after its entire content/mode roster matches the recorded
post-patch manifest. Refuse an altered tree rather than silently overwriting
manual edits. Failure removes only that invocation's newly created staging
directory; it never deletes an existing source, target or unrelated cache.

`patches/series.json` starts as an explicitly versioned empty ordered list.
Reject non-empty or unknown patch instructions in A1. This creates the stable
patch-input seam without shipping an unused fuzzy patch engine. A2's reviewed
extension will add exact patch bytes, order, preimage/postimage hashes and
application with neither fuzz nor offsets; manual `/private/tmp` changes will
never become build inputs. A1's post-patch manifest therefore equals the verified
upstream tree and records the empty-series hash.

### Separate workspace and SQLite boundary

`tools/foks-celld-guarded` is a nested Cargo workspace with resolver 2 and only
its probe package as a member. Exclude `runtime/source` from this outer
workspace. The celld path dependency points to
`runtime/source/crates/celld`; its three upstream packages continue to inherit
their dependencies from their own verified upstream workspace. Do not move
them into the outer workspace or modify their workspace manifests.

The harness depends on the real celld library, transport with
`default-features=false, features=["qualification"]`, and metadata-core with
`default-features=false, features=["owned-list"]`. Add only the direct engine
identity/serialization dependencies needed by the small probe: exact locked
rusqlite 0.31.0, V8 152.1.0 and tikv-jemallocator 0.6.1, plus the already selected
serde_json and SHA-256 implementation. No FOKS server, server-db, testkit,
metadata-core/native or transport/test-support edge is permitted. This phase
does not add TLS, a listener, a Tokio runtime, a cell or an S3 client invocation.

The whole executable must have exactly celld's rusqlite 0.31.0 /
libsqlite3-sys 0.28.0 pair. The FOKS transport/core normal-and-build subclosure
must have no SQLite package or forbidden native edge at all. Those are different
checks: applying A0's blanket SQLite denial to the entire celld executable would
be wrong. Keep the ordinary FOKS workspace and its lockfile unchanged.

Seed the new nested lockfile with the verified upstream lock, then perform a
recorded controlled resolution after the manifest/preparer/probe source has
passed author and independent review. Resolve only in this nested workspace;
prefer offline resolution after the exact missing registry metadata/packages
have been fetched. Inspect the resulting lock diff before compilation. Preserve
the identity and checksum of every retained upstream registry package. Record
legitimate pruning of unreachable upstream workspace/dev-only lock entries; a
nested library consumer need not retain unrelated packages. Any changed retained
package/version, replacement or upgrade, especially SQLite/V8, is a new review
decision. Added FOKS/transitive entries must be explained by the actual graph.
Freeze and review the completed lockfile and effective features before any build
or test.

`check_graph.py` reads saved `cargo metadata --filter-platform
aarch64-apple-darwin --format-version 1` evidence and verifies its command,
working directory, selected package/features, Cargo version, manifest/lock
hashes and output hash. Follow complete selected-target normal/build edges;
missing/ambiguous edge or target evidence fails closed. Check the whole engine
closure and the independently rooted FOKS subclosure. Reject alternate SQLite
providers, engine versions, V8 pointer-compression/sandbox/check features and
FOKS native/test-only features. Record a matching `cargo tree --edges
normal,build,features` as readable corroboration. Do not infer absence from
package names or silently skip an unclassified edge. This checker does not
modify or weaken A0's separate native/Wasm checks.

### Compiler and native V8 selection

Select the already installed `1.95.0-aarch64-apple-darwin` toolchain, rustc
`1.95.0 (59807616e 2026-04-14)`, full commit
`59807616e1fa2540724bfbac14d7976d7e4a3860`, LLVM 22.1.2, target
`aarch64-apple-darwin`. Record exact `rustc -Vv`, Cargo version and executable
hashes before build. Celld declares Rust 1.94.1; its release CI's 1.98.1 is
upstream provenance, not a requirement for this separately qualified artifact.
The local sparse-index audit matched exact checksums for 377/416 registry
versions; the highest declared MSRV among them was 1.89. Thirty-nine exact
versions lack cached metadata, including V8 and temporal dependencies. Their
manifests/source, and all newly selected dependencies, remain a pre-build MSRV
gate. If one actually needs a newer compiler, report that package/source reason
and revise the compiler pin before fetching/building; do not silently select
nightly or 1.98.1. An undeclared MSRV is not a compatibility proof.

Rusty V8 is the registry crate 152.1.0, checksum
`eb60870e5cff45e02719ab782f1c67507d33c4ef184755656bcc11a9795ccf83`.
The inspected upstream source is commit
`8149304f6924411df09a1917cd6de0fc90fa7fec`. Its `build.rs` SHA-256 is
`daebca36604ff3e03b6a123bf9b78fad7233c6b03c0fbc1bcda3983aada91c4d`;
its source `Cargo.toml` SHA-256 is
`cf9f467f738dfda05e33395d4eb9ad614e39ad72dddfb2cbeee3e9f540c97200`.
Verify the fetched crate's checksum and actual build source before executing it.
Account for Cargo's normalized packaged manifest; compare source
`Cargo.toml.orig` where present and verify effective dependencies/features rather
than pretending normalized Cargo.toml has the source-file hash.

The upstream manifest requests V8's default `use_custom_libcxx`, with no pointer
compression, sandbox or V8-check feature; verify this in the resolved harness
graph before build. Its build script chooses native
**release** V8 even for Cargo dev/test unless `V8_FORCE_DEBUG=true`. Therefore
the selected upstream assets are:

| Asset under `https://github.com/denoland/rusty_v8/releases/download/v152.1.0/` | Size and official SHA-256 |
| --- | --- |
| `librusty_v8_release_aarch64-apple-darwin.a.gz` | 40,641,117 bytes; `7317c9c45d9ee3166f1ad1abd5704a2617882dc0e56d3e6f452ad409e86ce9b4` |
| `src_binding_release_aarch64-apple-darwin.rs` | 40,056 bytes; `b9d5652ebca515eb2bf4c2ee4a6551b2567117b5aa720f430d2032616dd7e3b5` |

These asset hashes were read from official release metadata, retained at
`/private/tmp/foks-rusty-v8-v152.1.0-release.json` (SHA-256
`be78cb599b332b80ee4d9dfb2c47b2edc9b334bc0d29378971dc42937a32a9d7`).
The native asset and binding bytes have **not** been downloaded or locally
verified. Fetch them only after source review; verify exact compressed length,
hash and binding length/hash before use. Measure the archive's expanded length
and hash with bounded streaming decompression; do not assume its compressed
size is its disk cost. An initial 512-MiB expanded-file admission ceiling may
refuse the build and require a reviewed adjustment; it is not an asserted
upstream size. Record the actual expanded value before admitting compilation.

Set `RUSTY_V8_ARCHIVE` and `RUSTY_V8_SRC_BINDING_PATH` to verified absolute local
files. The reviewed script copies a local archive and uses the local binding;
it need not download either while Cargo runs. Reject inherited `V8_FROM_SOURCE`,
`V8_FORCE_DEBUG`, `RUSTY_V8_MIRROR`, `DOCS_RS`, `DENO_TRYBUILD`, RLS mode,
unreviewed GN/clang/wrapper overrides and other build-affecting inputs. Use an
explicit controlled environment and record all allowed variables/config files.
Missing prebuilt assets fail closed; never fall back to compiling V8 from source.
Cargo offline mode alone does not sandbox arbitrary build scripts: audit the
selected native build scripts and run the build with network unavailable once
their reviewed prerequisites are local.

Use dev/test `debug=0`, `incremental=false`, and an explicit V8 package
`opt-level=1` override. V8's own dev profile warns that opt-level 0 may miscompile,
and a dependency's profile does not control this outer workspace. Preserve the
upstream dev package overrides for sha2/rsa at opt-level 3. Pin these settings in
the outer manifest and receipt. This is a debug-asserting dev artifact, not the
upstream release CLI or a production performance baseline.

### Real library probe and later runner seam

The probe's sole command is `identity`. It calls the real
`celld::runtime::init_v8()` before creating any thread that might enter V8, then
records `v8::V8::get_version()`. It opens an in-memory connection through the
same pinned rusqlite, reads `sqlite_version()`, `sqlite_source_id()` and sorted
compile options, then closes it. Use upstream's global Jemalloc choice and
features; record allocator identity without claiming an RSS bound. Exercise one
real qualification `Pools::admitted_request` and the real owned-list trait
boundary, then drop that request. This proves the selected libraries/API can
coexist and link; it does not create a ready tenant or synthetic execution lease.

Emit bounded versioned JSON containing only diagnostic engine identities,
compiler/target/profile and the embedded input-record hash. The external receipt
binds the compiled artifact to source/lock/feature/config/native-archive hashes
and the actual binary hash; caller-supplied JSON is never readiness. The probe
does not instantiate Wasm, run the celld coordinator, serve requests or sync S3.

Celld already exports its library and initialization entry point. Node service
wiring, bucket gates, background tasks and shutdown still live largely in its
246,684-byte `main.rs`. Do not copy that program or construct a bare AppHandle and
call it equivalent to a node. A3 will extract one shared startup/shutdown path
used by both upstream main and the qualification runner, preserving
`LocalRequestCompletion.activity`; A1 neither implements nor claims that seam.

### Resource prerequisites and finite build recipe

Read-only inventory on 2026-10-16 found about 7.0 GiB free on the current volume,
the FOKS target about 35 GiB, the celld lab about 462 MiB and registry about
1.5 GiB. Of upstream's 416 registry archives, 354 were present and checksum
verified; 62 were missing. This is a whole-lock count, not the selected Darwin
build dependency count. Missing inputs include V8, celld's SQLite pair,
sqlite-vec and native allocator/temporal components. No reusable V8 native
archive or V8 source crate was found in the inspected local caches. The existing
57,051,008-byte celld CLI executable supplies no reusable Rust library artifacts.

After review, fetch only needed exact lock versions/native assets, verify them,
and inventory their extracted and cached sizes. Compiler installation is not an
initial prerequisite because the selected exact compiler exists. No unrelated
target/cache deletion is part of A1. Any later cleanup is a separate concrete
review of task-owned files, preserving the hashed F3 qualification artifacts.

Use `runtime/target` as the dedicated Cargo target, with `CARGO_INCREMENTAL=0`,
`--locked --offline -j 2 --target aarch64-apple-darwin --bin
foks-celld-guarded`. After prerequisite extraction, require at least 6 GiB free:
an initial 4-GiB target working allowance plus a 2-GiB free-space floor. This is
an operational admission budget, **not** a measured upper bound. Monitor free
space and target size while the single build runs; terminate that build and
retain its failure/logs if it crosses either threshold. Monitoring cannot
promise a filesystem hard quota or prevent concurrent external disk use. If it
needs more space, review the measured requirement before increasing the budget
or moving the task-owned target; do not retry indefinitely or delete other work.

Start Cargo in one new owned process group/session, without an intervening
shell, and retain the sole Cargo slot through its children's cleanup. Set a
30-minute operational build timeout; this is not a service responsiveness bound.
Resource refusal, timeout or cancellation sends termination to the entire owned
process group, followed by group-wide forced termination after a bounded grace
period if necessary. Reap the parent and establish that the owned group has no
remaining children before releasing the slot. Cargo's parent exit alone is not
cleanup evidence: compiler, linker and native-build children may still run.
If group cleanup cannot be established, or a child is observed escaping the
group, report uncertain cleanup and retain/quarantine the build slot; do not
start another build. Record the group identity, termination attempts and cleanup
observation in the failure receipt. The same rule applies if the Python wrapper
is interrupted; root remains the owner of the outstanding build until cleanup
is established.

`prepare-runtime.py build` first re-verifies source, assets, lock, feature report,
toolchain and current disk admission, then invokes the pinned Cargo with that
controlled environment and captures command/status/log/disk evidence. Record
inherited `.cargo/config.toml` too: the current parent config adds the macOS SDK
library search path and forces TMPDIR. Separate workspaces do not isolate Cargo
configuration. Record actual SDK/clang/linker identity; the current SDK is 26.5.
The wrapper must reject unexpected effective config/env overrides, not merely
print them. It records cancellation/resource refusal distinctly from compilation
failure and never turns partial artifacts into a successful receipt.

Run the resulting binary directly with `identity` under a finite diagnostic
timeout after checking its hash. Record wall time and output hash. Reproducible
here means pinned inputs and a repeatable reviewed recipe; do not assert
bit-for-bit determinism until separately demonstrated. No second cold build is
required merely to manufacture that claim on this constrained machine.

### Closed proposed A1 edit manifest

```text
tools/foks-celld-guarded/.gitignore
tools/foks-celld-guarded/Cargo.toml
tools/foks-celld-guarded/Cargo.lock
tools/foks-celld-guarded/rust-toolchain.toml
tools/foks-celld-guarded/runtime.lock.json
tools/foks-celld-guarded/upstream-tree.json
tools/foks-celld-guarded/prepare-runtime.py
tools/foks-celld-guarded/test_prepare_runtime.py
tools/foks-celld-guarded/check_graph.py
tools/foks-celld-guarded/test_check_graph.py
tools/foks-celld-guarded/patches/series.json
tools/foks-celld-guarded/src/main.rs
```

Ignore only the task's generated `runtime/` source/artifact/target/evidence
subtree. No root manifest/lock changes, transport extraction edits or celld
runtime patches belong to this group. Report any needed path addition and its
reason before editing it.

### Review, validation and acceptance gates

1. Root and an independent agent review this exact plan and source manifest.
   Implement the preparer, checker, probe and input pins, then author-review and
   independently review their frozen source before running checks or resolving
   dependencies. Preserve the source and review hashes.
2. Perform only the reviewed input acquisition/resolution step. Verify crate and
   native archive bytes, source/build-script/MSRV assumptions and precise graph.
   Review the completed nested lockfile and any source changes twice before
   compiling or testing. A changed input that violates the approved engine or
   feature policy is a plan change, not an automatic upgrade.
3. Run meaningful Python tests for archive traversal/links/duplicates, wrong
   content/modes/hash/size, bounded decompression and failed preparation cleanup;
   altered-tree reuse refusal; non-empty patch rejection; unexpected environment
   and graph evidence; wrong engine/feature, missing target/root/edge and the
   SQLite-free FOKS subclosure. Test resource refusal/cancellation against a small
   controlled process tree, including a parent that exits before its child and a
   child that needs forced termination. Assert group cleanup precedes Cargo-slot
   reuse and uncertain cleanup refuses reuse, without launching a real Cargo/V8
   build from tests.
4. Prepare twice from verified inputs and compare manifests, confirming exact
   idempotent reuse. Validate actual locked native metadata/feature evidence with
   the tested checker, then run the single admitted library build and strict
   harness lint. Execute the actual identity probe and retain source, input,
   graph, compiler/SDK, expanded native library, binary, output and failure hashes.
   Native compiler acceptance and the actual linked engine/source IDs are gates,
   not facts inferred from manifest versions. No upstream internal-test suite is
   claimed by this small probe.
5. Author-review the result and obtain independent review of the same source and
   evidence before the A1-only commit. Include implementation and its exact nested
   lock/pins; leave Markdown unstaged. Preserve failures. Later A2-A4 must rebind
   the altered runtime/profile and repeat relevant build/engine qualification;
   A1 does not grant physical disposal, SQL cancellation, S3 durability, 15-second
   recovery or native F3 allocator/layout proofs to a different executable.

Author plan review: PASS for this finite slice, with prerequisites and unsupported
claims explicit. Root and independent plan review are pending. No A1 source,
dependency-resolution, build, test or download step has run.

Plan review status: author, root and independent reviewer passed the exact A1
plan at SHA-256 `6371d0846cd811494657b8605d7259803631185590ded5951db62f26a588c2c3`. The closing paragraph above records the draft status; these reviews are now complete. Implementation waits for A0 acceptance and its separate commit.


## A0 completed qualification

Implementation-only commit `cc7777864c1eb1752e28425d9f6f5dcf2d15b352` extracts the same retained Request,
monotonic Deadline and bounded framing into `foks-hosted-transport`. Root and
independent source reviews passed before testing, including both lockfiles and
the narrow lint correction. All 30 committed paths match the frozen source.

The three transport profiles each passed 21 unit tests, two positive/negative
public API doctests and strict lint. Actual standalone native consumers ran in
all three profiles: each selected normal/build closure has 43 packages and no
SQLite or server edge. The core-only Wasm consumer built with 36 packages and no
transport/Tokio edge. The existing core default and owned-list Wasm builds retain
the unshared 64 MiB maximum; Python graph/memory-limit tests passed 11/3 and Node
host-policy tests passed nine.

Both server configurations passed 216 tests with four existing deployment tests
ignored; five allocation tests and strict all-target lints passed. Fourteen
original tests moved to the new crate, and two adapter tests were added,
explaining the server count change from 228. No registry versions changed.

Evidence: `/private/tmp/foks-hosted-transport-a0-qualification.json`, SHA-256
`b618e26aa27f7c53251735bfdea926f1ccc9b9584f6159d82efad4492726de02`; author final acceptance and the
commit/source binding are recorded alongside it. The original lint failure is
retained. These results establish extraction equivalence and dependency
isolation. A1 library build, A2 physical disposal, A3 runtime/output ownership,
A4 deadlines, target import and actual celld recovery/capacity remain open.
All Markdown remains unstaged.

## A1 amendment: exact existing FOKS dependency additions

The reviewed A0 transport/core closure already uses SHA-2 0.11, while celld uses 0.10.9. Keeping both versions does not upgrade the celld engine or change cryptographic/wire behavior. This amendment permits only the following exact additional same-name registry identities from the committed A0 external-consumer lock. It does not authorize dependency resolution or compilation before the existing source/lock reviews.

Baseline commit: `cc7777864c1eb1752e28425d9f6f5dcf2d15b352`. Baseline file: `tools/foks-hosted-transport-check/Cargo.lock`; SHA-256 `b012709ac33093d0d0a5116af4693207e794c44ca67bd02662ccefea5a6d1013`. Upstream lock SHA-256 remains `d6a17422788aa910ba76bed9cf59ac15767f93fe107ebdb336954d0e9efe052b`. The finite data record is `/private/tmp/foks-celld-a1-additional-versions.json`, SHA-256 `e66fb9768f6ba9a46de343bb3d621a9f147c066ba42524df4165cd9afd51170a`. All rows use `registry+https://github.com/rust-lang/crates.io-index`.

| Package | Additional version | Exact checksum |
| --- | --- | --- |
| block-buffer | 0.12.1 | `d2f6c7dbe95a6ed67ad9f18e57daf93a2f034c524b99fd2b76d18fdfeb6660aa` |
| const-oid | 0.10.2 | `a6ef517f0926dd24a1582492c791b6a4818a4d94e789a334894aa15b0d12f55c` |
| crypto-common | 0.2.2 | `ce6e4c961d6cd6c9a86db418387425e8bdeaf05b3c8bc1411e6dca4c252f1453` |
| digest | 0.11.3 | `f1dd6dbb5841937940781866fa1281a1ff7bd3bf827091440879f9994983d5c2` |
| sha2 | 0.11.0 | `446ba717509524cb3f22f17ecc096f10f4822d76ab5c0b9822c5f9c284e825f4` |
| syn | 3.0.4 | `e6275cddf4610d1775e6d1fe9469b2e77d0f39fd98fb7450901b821e0c53649f` |
| thiserror | 2.0.20 | `ec86235f5fcc2a73650310756d2ac5b138a5780bbbdfae3eeccec992c435ba4f` |
| thiserror-impl | 2.0.20 | `bc04cd3e1236dd4a98afca4569f2deb3f120e5422a4023be2cb683f8486292af` |

Embed this exact eight-row allowlist and baseline lock/hash in the existing `runtime.lock.json`; no proposed path is added. The checker verifies the committed baseline lock bytes and that each row exists there, then permits an actual newly added same-name identity only if its source, name, version and checksum match this list. Added packages with names absent upstream still require the ordinary source/actual graph and lock-diff review. The allowlist is an upper bound, not a command to add all eight versions.

Keep every selected upstream registry identity/checksum unchanged. Report all removed upstream entries as unreachable candidates and review their actual graph reason; do not label a selected celld dependency replaced by a newer compatible version as an innocent removal. An entry on this list is not approval to remove or upgrade a retained upstream package. Replacement, changed retained version/checksum, an extra same-name version outside the list, or any SQLite/V8 engine change fails the current policy and needs explicit review. The actual graph/lock review remains mandatory before build.

Qualify the outer optimization override as `[profile.dev.package."sha2:0.10.9"]` with `opt-level=3`. This preserves the upstream celld override without an ambiguous package selector when the FOKS closure adds SHA-2 0.11. No profile override is newly applied to 0.11. Keep the existing V8 opt-level1 and RSA override.

Add focused checker tests for an exact approved addition, wrong version/checksum/source, missing or altered baseline lock, an unknown same-name addition and an engine addition/replacement. Preserve retained-checksum and removal reporting tests. Source and lock changes still receive author and independent review before running any test or resolution.

Author amendment review: PASS after comparing the exact two existing lockfiles and the native-disabled metadata-core SHA-2 declarations. No Cargo, test, input acquisition or build ran; no main Markdown changed. Root and independent plan review are pending.

Amendment review status: author, root and independent review PASS for the exact eight-row policy above. Source implementation and the actual selected lock/graph remain separately gated.

A1 resource preparation: a separately reviewed closed inventory removed 1,346 stale `foks_server` codegen object files (2,801,183,200 bytes). Every file was a regular Mach-O object from a non-current-F3 inventory group; all 20 retained F3/A0 artifact hashes were rechecked and preserved. No linked binary, library, metadata, fingerprint, source or qualification evidence was removed. An elevated process check found no Cargo/compiler/linker running. Free space after cleanup was 7,310,172,160 bytes; admission must still be remeasured after actual prerequisites. The exact plan and result are retained in `/private/tmp/foks-celld-a1-object-cleanup-{plan,result}.json`, with root and independent review receipts. This does not relax the 6-GiB build admission or 2-GiB running floor.

A2 now has a separately reviewed detailed [runtime lifecycle plan](HOSTED_CELLD_RUNTIME_LIFECYCLE_PLAN.md). Author, root and independent plan reviews passed; accepted A1 implementation/engine provenance remains its prerequisite.

A1 prerequisite results: all 279 celld source files prepared from the pinned Git tree. Both official V8 asset hashes matched; the static archive expands to 157,808,960 bytes, SHA-256 `1c55d1cc66e97b4c4f2fd4473dbddba4858a2a9cd35543a4b3851d58769e9b8a`, below its admission ceiling. Initial offline Cargo metadata attempts failed before resolution on workspace inheritance; their logs and complete process cleanup receipts are preserved. Reading the pinned Cargo source identified that `members = ["."]` prefix-matches all descendants and overrides exclusions. The narrow correction removes that redundant explicit member while retaining concrete upstream exclusions and checking that the resolved workspace contains only the harness. Upstream source and engine pins remain unchanged. No build or test result is claimed at this stage.

## A1 workspace-boundary amendment

Real metadata exposed a second inheritance boundary: the cached FOKS ancestor workspace can intercept celld packages before Cargo discovers their nearer workspace. Add only `exclude = ["tools/foks-celld-guarded/runtime/source"]` to the root workspace. This adds root `Cargo.toml` to the A1 implementation manifest (13 final paths). It preserves every existing member, dependency and the exact root lock; no upstream source is patched. Root and independent plan reviews passed the exact amendment at `/private/tmp/foks-celld-a1-root-exclusion-plan.md`, SHA-256 `2cc5ae76440e4c7932322db41c452158e13c0a79aa261672d57fe8f887606569`. The real pre-edit baseline records 29 members and seven default members. After the two source reviews, compare real metadata membership before/after and retry nested resolution. Earlier failures remain evidence, not passing qualification.

A1 resolved-input review: actual locked metadata contains 370 selected packages, including a 58-package SQLite-free FOKS subclosure. All 416 upstream registry lock identities/checksums remain unchanged; 1,032 selected upstream dependency edges match their original targets. The six added registry identities are five approved FOKS crypto-family versions and new-name hybrid-array 0.4.15. Initial online resolution selected SHA-2 0.11.1; a targeted offline pin restored the reviewed 0.11.0 baseline before freezing source-f. All 360 selected registry archives and extracted file bytes matched; the largest declared MSRV is 1.89 under Rust 1.95. Source-f is `/private/tmp/foks-celld-a1-source-f.json`, SHA-256 `5270b5c42de559a27d8078311e3fe623dd70a1db285e6c03c01f1567cffaa4a7`. Root source/lock/native review passed; independent review and tests/build remain pending.

The initial six-GiB build free-space gate is currently unmet, so no build has started. The exact selected registry payload totals about 357 MB extracted plus 73 MB of archives, including preexisting inputs. The observed larger fall in volume free space is not attributed solely to fetching these dependencies; other volume activity and allocation overhead remain possible. Only separately reviewed stale task-owned build artifacts are eligible for cleanup.

A1 harness validation: the first 32 tests exposed four cancellation failures. A retained minimal trace proved that Darwin can return EPERM from `killpg(group, 0)` while the terminated parent still appears in the process table. The corrected source-g treats permission denial as possible presence, records denied signals, and retains the existing bounded cleanup/quarantine behavior. Root and independent reviews passed before rerunning; all 35 tests now pass, including deterministic transient/persistent-denial regressions. Both source/native preparation passes matched exactly. Actual locked/offline graph collection passed at `/private/tmp/foks-celld-a1-graph-g`; evidence SHA-256 `af11432ff0d99c1bb7d043bbd366bc8ffef762a1d7ad19e9944628542f52fcd8`. Original failures remain retained.

Separately reviewed artifact cleanup removed exactly 1,308 stale hosted-task outputs (2,237,380,320 logical bytes), preserving all 20 final F3/A0 artifact hashes and 624 provenance inputs. Actual free space after removal was 5,639,917,568 bytes, below the unchanged 6-GiB build-admission gate. The current A1 target remains empty. Result: `/private/tmp/foks-celld-a1-server-artifact-cleanup-result.json`; only further proven task-owned candidates are under consideration.

The additional reviewed cleanup removed exactly 774 stale task-owned files (1,026,494,728 logical bytes), rechecking 707 protected artifacts and 377 provenance inputs. Actual free space afterward was 6,657,126,400 bytes, satisfying the unchanged 6-GiB admission gate. The combined inventory supersedes the unexecuted metadata-only proposal. Result: `/private/tmp/foks-celld-a1-additional-artifact-cleanup-result.json`. The source-g actual build, strict lint and identity sequence is now running under the pinned offline process-group/resource wrapper; no runtime success is claimed before its receipt completes.

The first actual A1 build stopped after 15.66 seconds when the disk monitor enumerated a Cargo temporary object that was removed before stat. This was a `FileNotFoundError` in resource observation, not a compiler diagnostic. The owned process group was terminated and reaped, with no observed escaped children; source/assets and engine pins remain unchanged. Original failure evidence is retained at `tools/foks-celld-guarded/runtime/evidence/build-5irm95qc` in the validation worktree. The narrow correction will tolerate only vanished live-target entries while retaining symlink/error refusal and all resource ceilings. It requires root and independent source review before retesting.

Source-h correction passed author, root and independent review before execution. All 39 harness tests passed, with deterministic vanished-file/directory and error-propagation regressions. Actual locked graph recollection passed at `/private/tmp/foks-celld-a1-graph-h`, evidence SHA-256 `119012bb1f55ec8b603e01b8809ea827149b4351e485ef28c9e867e57830eeee`. The failed attempt’s 891 generated target files were removed under an exact reviewed inventory; source, assets and failure evidence were preserved. Actual free space after cleanup was 6,649,384,960 bytes. The unchanged resource gate admitted the new build.

The actual source-h A1 build, strict harness lint and identity probe passed under the owned offline wrapper (100.80 s, 30.12 s and 0.76 s respectively). The executable reports V8 `15.2.124.1-rusty` and SQLite `3.45.0` with the source ID matching the selected native inputs, and exercises the actual A0 Request owner. Final target size was 1,694,884,393 logical bytes and free space 5,058,252,800 bytes. Build receipt SHA-256: `911fd5c5f376c1517c5f77f9cc1dcd7861a48fc3ebce2e979b000466d1e6baa7`. Root evidence review passed; independent review remains pending before commit. This is local Darwin library/tooling identity evidence only; physical Wasm retirement, actual node ownership, S3 serving, recovery and capacity remain open.

A1 accepted and committed as `8e31944080e03651caaa3512169fe468f8375342` (`Pin and qualify the celld runtime build`). All 13 committed files match source-h; no Markdown is in the commit, and the root lock remains unchanged. Independent final review passed at `/private/tmp/foks-celld-a1-final-import-review.json`, SHA-256 `e2520d85e8214484960978c0e496d3ff1aae6d26f1cf17b6174ee059f2d6ce95`. Acceptance and commit bindings are `/private/tmp/foks-celld-a1-acceptance.json` and `/private/tmp/foks-celld-a1-commit.json`. The exact 39-test invocation is preserved separately; it used Python from the inherited shell PATH, with no claim of a separately pinned Python interpreter. A2 is now being rebound to this accepted base before source edits.
