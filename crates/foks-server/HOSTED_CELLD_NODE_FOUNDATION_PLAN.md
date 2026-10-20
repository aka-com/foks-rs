# B1: closed descriptor and original-owner foundation

Current status: B1 source-g passed both source reviews and all three final
qualification modes. The final evidence reviews passed: 21 native component
tests, nine actual compiled layouts, 28 fresh-process A2 lifecycle scenarios,
and 15 focused Python tests. The earlier 161-test tooling run remains the
broader regression baseline; it was not repeated in full on source-g.
Acceptance is recorded in `/private/tmp/foks-celld-b1-acceptance.json`.
Implementation-only commit `23039e209d1b3b129a274dd65fe07f0abfdba8e0` is
in the main checkout; all Markdown remains unstaged. B1 has no guarded serving Action or readiness and does not qualify
host/AZ recovery, whole-node retirement, production density or S3 reclamation.
Frozen planning history and its reviewed amendments follow below.

Frozen revision-C status: proposed exact B1 subset of the reviewed A3b execution plan. This is a
plan and source-input freeze only. It does not authorize source edits, input
preparation, Cargo, tests, node execution, or a guarded Ready state. The accepted
source base remains A2 `dca38eecb08263a3594f70025df7a309ed3682a5`; rebind every
mutable outer input to the eventual accepted A3a commit before implementation.
Keep the accepted A3b plan and its reviews unchanged. Repository Markdown is
root-owned and remains unstaged.

## Result and boundaries

B1 supplies the default-off local dependency edges, a closed immutable
deployment classifier, refusal at ordinary construction/route boundaries,
original-owner accounting primitives, exact generation lookup, and one new
pure-core resident-only event plus its inactivity query. The selected B1
binary has no new Action, listener, route to guarded execution, positive
readiness, activation, SQL/sync implementation, effect supervision, or output
publication. No A0, FOKS wire/crypto/schema, upstream engine or native asset
change is included.

The B1 component tests do not initialize V8 or install/reset A2's global
physical-capacity ledger. They exercise the real A0 Request and real core
`on_event`, not a substitute owner or ready certificate. A model test may feed
canonical core input/result events as a model; it must be labeled model
evidence and cannot stand in for actual actor, ownership, storage, or native
completion. A2 lifecycle regression remains a separate fresh-process test.

The actual A2 execution ledger stays the sole two-slot/Wasm-capacity authority.
The native Request-owner and accepted-connection limits below are separate
resource bounds, not another execution semaphore. B1 does not acquire or
release an A2 execution lease. B2 must compose the native owner with that real
lease and its exact physical retirement obligations before a request can run.

## Source-backed amendment: four owners are not four connections

The actual A0 `memory.rs` has an important distinction:

* `Pools::new` uses `maximum_in_flight_requests` only to size the bulk
  semaphore. It does not count every `admitted_request` call.
* `admitted_request` precharges the real Request Arc allocation and stores its
  optional `OwnedSemaphorePermit` in `Request::_input`.
* A real process Request remains alive for the ledger/report lifetime. It
  counts against the selected maximum of four. No count is inferred from
  output/working semaphore capacity or A2's two execution records.

Use three fixed role semaphores, with capacities process=1, invocation=2, and
control=1. Their sum is the private four-owner admission geometry. The role
permit is passed as `Some(permit)` into the real
`Pools::admitted_request`; failure drops it without producing an owner. No
`Pools::request` or `admitted_request(None)` is allowed in the new node path.
Only the private factory can reach these role semaphores. Invocation and
refusal work cannot borrow the control lane. A single transient control Request
may coexist with the process Request and two invocation Requests, including
when both invocations are Retiring or Quarantined.

This bounds admitted strong Request owners. The role permit's return is not a
physical disposal certificate: `_input` is declared before later Request
fields, and a retained Weak can keep the Arc allocation after strong=0. New
resource aggregates retain their actual owner until resources are gone.
Concrete task/output/Wasm/activation completion remains B2/B3 work. Report
Weak storage and still-live Arc backing have their own bounded process-owned
allowance; neither weak-count zero nor allocator deallocation is inferred from
the role semaphore. Keep owner registry references through the explicit
retirement operation, rather than relying on handler Drop to release them.

There are separately four fixed accepted-socket/header cells, precharged on
the real process Request before their storage is allocated. At the future
single private `GuardedNode::accept` handoff, capture `accepted_at` immediately
after `listener.accept` without an intervening await. The cell owns the actual
socket, inline 16-byte header and one-byte EOF check, original Instant, finite
parser state and process-owner clone. No invocation Request exists yet. This
explicitly refines the earlier broad sentence that a Request exists before
asynchronous parsing: **the process Request owns this bounded interval; the
invocation-specific Request is created only after exact classification.**

The invocation retains that same `accepted_at + 15 seconds`; parsing or
transfer never resets it. A header/EOF must finish within one second of
acceptance, still inside the original deadline. Exact header/EOF I/O and the
frontend are B3, not B1. Control/refusal uses the same acceptance Instant with
the one-second cap. Extra connections close without a new Request, queue, or
heap-backed refusal. Invalid/Begin-overload traffic cannot reserve the control
Request. A control whose single lane is already occupied promptly refuses or
closes; it does not queue another owner. Four accepted cells are a ceiling,
not four promised simultaneous Requests or immunity to arbitrary connection
floods. Two slow unknown headers can temporarily occupy the other cells; their
fixed one-second lifetime is the bound, not a claim of guaranteed external
control delivery under adversarial scheduling.

Mandatory **internal** cancellation, primary/auxiliary finish, and retirement
never depend on another socket, parsing opportunity, Request, or forward slot.
Their existing planned per-invocation reservations remain available after
original expiry/failure. This is how the two occupied/quarantined invocations
cannot consume mandatory cleanup admission. The external transient control
lane is reserved against invocation use, but is not a substitute for that
internal cleanup guarantee.

## Native allocation buckets and headroom

Keep A3b PoolLimits response=8 MiB, working=64 MiB, control=256 KiB,
maximum_in_flight_requests=4. This private Pools instance is not shared with
ordinary celld runtime allocations. A0 divides working into 48 MiB primary
and 16 MiB small-request headroom. The following additional role debt caps are
closed qualification limits proposed by this amendment; they need review, not
an assertion that the existing code already enforces them:

| Original owner | Cumulative new retained payload charge, including actual Request Arc |
| --- | --- |
| Process, one for the process lifetime | at most 1 MiB |
| Invocation, at most two strong owners | at most 8 MiB each |
| Transient control, at most one strong owner | at most 64 KiB |

All new node-owned heap payload reservations use the actual Request's existing
`ListRetainedOwner::reserve_retained` (working allowance) through one private
checked role-debt method. This includes the fixed native response buffer; no
new direct foks-rpc dependency is needed. The output/control PoolLimits remain
configured but are not falsely reported as the pools that paid these private
diagnostic buffers. A0's own Request Arc charge is included in the role total;
do not reserve it again. Inspect and pin its actual header-plus-Request Layout
using the same two-AtomicUsize/repr(C) source convention, maximum 4 KiB and
alignment at most 16. Actual bytes, not 4 KiB, are recorded.

Maximum ordinary retained working debt of these strong owners is therefore
17 MiB + 64 KiB, well below the primary 48-MiB pool. Reserve/report temporary
destructor/Weak overlap separately, bounded by the fixed owner cells and
retirement discipline. The control Request's actual Arc and at most 64-KiB
payload must fit that protected headroom even when both invocation debts are
full. A role cap refusal is sticky on that actual Request and cannot spend
another role's capacity. No permissive small-pool fallback is used to claim
more per-role credit. Tests exhaust the invocation caps through this exact
factory and then admit the real control owner; they do not assume the argument
named `maximum_in_flight_requests` enforces this.

For repeated owners, the process registry retains its base Arc until explicit
retirement. Once all known resources/copies are retired, consume that Arc with
`Arc::try_unwrap`; an unexpected remaining strong alias leaves the owner cell
occupied (later runtime paths quarantine). Drop the returned Request
synchronously while the private admission/retirement guard is held, before
marking that cell reusable. This ensures a newly admitted generation does not
race an intentionally retained prior Request's permit reservations. There is
no await, native I/O, V8 destruction or resource teardown under this guard.
Future actual resource teardown happens before this point. Contended
admission uses try-lock and refuses; no wait queue is created. Fixed diagnostic
Weak witnesses can retain Arc backing after this step; at most 64 invocation
summaries plus one process/one current control witness are reserved in the
process budget. Exhaustion stops new admissions, not a growing Vec.

The role method checks checked-add of prior cumulative debt and requested
bytes, the fixed cap, original failure and original deadline where applicable,
then calls the actual retained-owner trait before constructing the allocation.
It never refunds cumulative allocation traffic. A fixed bounded slot may be
reused only after its prior payload dies; it cannot create a new allocation
credit. Avoid moving a caller's already allocated String/Vec into an apparently
preadmitted charge. All new owned strings/Arcs/channels/buffers are either
precharged at construction or already part of an enumerated fixed workspace.

Allocation classifications to freeze in B1 source/layout review:

* Process-owned: descriptor fixed records, admission registry/control lane,
  four socket/header cells, fixed Weak summaries, any new static Arc/layout,
  bounded scalar report plus simultaneously live serialization storage. The
  A2 Capacity allocation/events and any retained source/config copies must be
  included when B2 composes them; do not hide them outside the 1-MiB cap.
* Invocation-owned: actual Request Arc, original-control Arc plus cancellation
  object, fixed scope/binding, native frame, task payloads and enumerated owned
  buffers. B2's 200-KiB cleanup reserve is charged here at admission, not to the
  global 256-KiB control pool. Eight one-use 24-KiB cleanup credits plus four
  2-KiB control records retain the reviewed meaning. B1 reserves the bytes and
  closed credit counters only; it does not implement B2 jobs or certify their
  layout/maximum-creation sufficiency.
* Control-owned: only the real transient control Request and bounded native
  control result/payload. Its selected record cannot retain invocation plaintext
  or a new output frame. Mandatory internal controls use the already held
  invocation credit, even after expiry/failure.
* Stack-only/nonadditive observations: fixed enum/binding/header fields inside
  the charged containing object; report size/alignment and live phase, without
  charging the same bytes twice.
* Explicit unchanged process overhead: Pools and role-semaphore bootstrap
  allocations before a Request can exist, canonical deployment parsing,
  ordinary primary Worker and pools, existing actor maps, V8/JS and SQLite
  caches, allocator overhead, kernel socket buffers, Tokio task/header/retired
  allocation storage. No complete RSS/heap bound is claimed. Any new guarded
  record or queue must fit the charged closed geometry rather than be moved
  into this exclusion.

Only B1's actual types/layouts are measured in B1; no invented B2/B3 task size.
For any new task eventually added, charge the **sum** of simultaneous typed
capture/result/completion/channel payloads. A `max` is permitted only after
source/layout proof of a shared union/nonoverlap. B1 starts no such task.

## Exact B1 private contracts

Names below are the source/API freeze. Narrow type/method spelling changes
found necessary at compilation require a declared reviewed delta. They must
not introduce a public mutable capability or readiness constructor.

`guarded_execution.rs` declares the feature-only child:
`#[path = "guarded_node.rs"] pub(crate) mod node;`.
`node` contains `control` and test-only `tests` children. `js.rs` declares the
feature-only `js/guarded_node.rs`. The only eventual binary-facing public type
introduced in B1 is opaque `GuardedNode` with `install()`/`stop_admission()` and
bounded foundation-layout/snapshot observations. B1's install takes no caller
owner, pool, deadline, descriptor, success callback, or Ready argument. There
is no scenario-dependent native runtime work yet. Its static installation is
once-only and remains charged until exit. Public binary acceptance/running/
bucket/retirement methods are introduced with their real B3 callers, not stub
implementations in B1. No marker reports B1 as runnable.

The internal admission factory is reusable source, invoked for the process
owner by install and for the other closed roles only by the B1 component tests
until B2/B3 supply their genuine callers. It must not export a raw Request,
permit, role selector, or caller-controlled stamp. Tests are descendants of
the actual private module, not a public `for_test` readiness API. Source lint
review must keep any not-yet-needed B2/B3 method absent, instead of adding
placeholder uses or broad dead-code suppressions.

```text
control::OwnerRole = Process | Invocation | Control             // private
control::AcceptedStamp { accepted_at, deadline }                 // fields private
control::AcceptedStamp::capture() -> Result<Self, Refusal>       // original now, checked +15s
control::AcceptedStamp::control_deadline(&self) -> Instant       // min(original,+1s)
control::OwnerFactory::new() -> Result<Self, Refusal>             // fixed pools/role semaphores
control::OwnerFactory::try_owner(role, Option<AcceptedStamp>)
    -> Result<OriginalOwner, Refusal>                            // no waiting
control::OriginalOwner::charge(bytes) -> Result<(), Refusal>     // actual Request, sticky debt
control::OriginalOwner::check() -> Result<(), Refusal>           // original Instant/failure/cancel
control::OriginalOwner::same_owner(&self, &Self) -> bool          // actual Arc::ptr_eq
control::OriginalOwner::cancel(&self)                            // sticky, allocation-free
control::OriginalOwner::retire_unstarted(self)
    -> Result<(), HeldOwner>                                   // pure pre-execution ownership only
control::CleanupCredits { remaining_task_creations:8,
                          control_bits:4, prepaid_bytes:200KiB } // private one-use bookkeeping
control::CleanupCredits::consume_closed(kind) -> Result<Credit, Refusal>
```

`capture` has no production caller until actual socket admission B3; its
component test-only exact-Instant constructor cannot escape the test module.
`try_owner` requires no stamp for Process and requires the already captured
stamp for Invocation/Control; it does not call `Instant::now` to renew one.
Process owner creation is only once. Refusal has a finite enum: Capacity,
Busy, Contended, Expired, Cancelled, Failed, Bounds, Invalid, Exhausted,
NotReady. No String or allocating timeout error is needed at this boundary.
Generation/id counters use checked increment, never wrap/saturate into reuse.
An attempted cleanup credit reuse latches failure and cannot publish; consuming
a valid already-held cleanup credit remains possible after original failure.
That credit is an accounting token, not permission to run JS, create a Worker,
fabricate a core event result, or return any physical lease.

`OriginalOwner` uses one aggregate declaration order: concrete owned payloads
and state, then original control, then registry ownership. Actual Request
clones given to native scopes are always held by a containing owned aggregate
and do not release the role on waiter cancellation. B1's cleanup/retire method
is only for objects that have never started native work; B2 must not reuse it
as the post-activation disposition path.

## Exact immutable descriptor and construction classification

`js/guarded_node.rs` owns the fixed two script records and derived source
identity helpers, not a general JS/Wasm parser. It reuses A2's exact WASM byte
slice unchanged. B1 introduces its own node JS bytes; their final lengths and
SHA256 values are source-frozen before execution and carried in the B1 profile.
Do not claim an actual hash before those bytes exist. No fixture execution is
needed to derive them. Node source must remain within the approved semantics:
one fixed memory, complete 64-MiB touch, one bounded SQL marker, actual
`storage.sync`, fixed response/controlled throw, no other native capability.
Behavior belongs to B2/B4 qualification; B1 accepts only exact source bytes.

Use fixed script names `foks-guarded-node-primary` and
`foks-guarded-node-fixture`; class `FoksGuardedNodeFixture`; primary service
binding `GUARDED` names the fixture with no entrypoint. The primary has no DO
class, data/queue/workflow/asset/loader/cron/container binding or vars. The
fixture has exactly the one class and DO binding `OBJECTS` naming that class;
it has no
outgoing service, extra module, imports, other bindings, crons or containers.
The Wasm bytes are embedded in the exact JS literal, so `options.modules` is
empty. Primary and fixture compatibility flags are six explicitly frozen
booleans matching the reviewed canonical fixture metadata; default is not
assumed equivalent to deployed compatibility settings. Three fixed lowercase
64-hex tenant IDs are `11` repeated 32, `22` repeated 32, `33` repeated 32; their
only legal scope is the exact class, colon, and that ID (maximum 256 bytes).

The descriptor records both source hashes/full bytes, exact deployment versions
and prefixes, class/binding graph, compatibility booleans, original node name
binding and descriptor digest. Canonical `deployment_version` currently returns
16 hex characters, not a full SHA256. Freeze its actual metadata bytes and
computed version at source authoring; validate exact version and
`deploy/<script>/<version>` prefix. Also independently compare exact source and
all admitted derived fields. The short canonical version alone is not the
descriptor security identity. `LoadedDeployment` has already discarded raw
manifest JSON, so do not pretend B1 revalidates raw manifest bytes at
Generation::build. It validates the exact loaded executable/configuration
projection; canonical loader module verification and later controller manifest
identity are distinct observations. `crons` are not included in upstream's
version digest, making the explicit empty-crons check mandatory.

```text
node::DescriptorPolicy::validate_graph(&DeploymentGraph, expected_node: &str)
    -> Result<ValidatedGraph, Refusal>
node::ValidatedGraph::classify(index, &LoadedDeployment)
    -> Result<ScriptClass, Refusal>                 // OrdinaryPrimary | Guarded
node::DescriptorPolicy::is_guarded_scope(&str) -> bool
node::DescriptorPolicy::is_guarded_script(&str) -> bool
js::guarded_node::GuardedConfigMarker                // private constructor/fields, no Clone token API
js::guarded_node::mark_validated(WorkerConfig, &ValidatedGraph)
    -> Result<WorkerConfig, Refusal>
js::guarded_node::ordinary_load_allowed(&WorkerConfig) -> Result<(), Refusal>
```

The expected_node argument is the actual GenerationOptions.node supplied by
the canonical build caller, bounded and compared with both loaded options;
it is not an ingress string or readiness assertion. The installed immutable
policy is consulted by both boot and reload before
`runtime::init_engine`, queue catalog construction, WorkerConfig construction,
or pool warming. Disabled/uninstalled policy preserves ordinary behavior.
Move validated graph/config values forward; do not create a second owned source
copy just to classify. Guarded configs remain in the class registry as data,
but cannot enter service_pools or cell_isolates. The ordinary primary still
supplies Generation's mandatory stateless field. The same classification
controls both pool loops; do not filter only one of them. ValidatedGraph is
neither admission nor permission to build a guarded Worker.

WorkerConfig carries a private feature-only guarded marker constructed only
from validation. Ordinary `Worker::load_config` rejects it **before**
`v8::CreateParams`/`Isolate::new`. B1 has no privileged construction scope and no
load exception, including tests. B2 introduces the owner/lease-bound exception
as one reviewed activation group. `StatelessRuntime::start` and generic
`cell_pool` entry also refuse or have no valid guarded selection before
constructing an ordinary pool. A2's existing config/source/constructor stays
unmarked and unchanged. No caller script name or new public flag can mint the
private marker.

## Ordinary route refusals and exact generations

B1's policy hooks reject guarded targets before ordinary side effects:

| Existing path | Earliest B1 refusal |
| --- | --- |
| Actor Message::Request, including forward/capacity/handoff/WebSocket/gate modes | Before pending/handoff/route maps, begin_route_if_cold, or any core Request/GateRequestAt event. Reply using existing refusal result; do not hold a fake ActivityGuard. |
| Actor WakeHint/MaintainWake | Before WakeHintAt/schedule_wake_maintenance; no new wake/restore job. |
| Actor local-cells resume input | Before apply_core_event, a CompletedEffect carrying LocalCellsRead with a guarded entry is a closed-profile startup refusal. Never replace it with an empty successful scan. No successful resume/activation is reported. |
| Eager reload and other activation-producing decisions | Keep GenerationChanged cancellation semantics. Refuse guarded Effect::Restore/StartRuntime/Publish before route timing, no-host success branches, tasks or native work. Report the existing exact-op failure, never a fabricated successful RuntimeStarted/Published. |
| Generic restore_cell/start_cell/publish_cell and resident Worker fetch | Before path/VFS/registry mutation, pool placement, or spawning work. |
| Generic cell fetch/alarm/RPC/WebSocket/facet methods | Central guard at actual affiliation/admission helper plus entry guard before channel/job/request side effects where needed. |
| Service/RPC/queue dispatch and dynamically loaded configs | Guard script classification before pool lookup/dispatch; marker guard before any Worker construction. Guarded descriptor disallows loaders/facets/queue/WS rather than adding implementations. |

This table is not permission to modify adjacent service semantics. B1 has no
legitimate guarded route: every ordinary route refuses. B2 adds a separate
private exact-owned route, leaving these defenses intact. If an actual input
cannot be stopped inside the 14-path roster before activation, report the exact
path and propose the smallest amendment before editing. Do not claim a class
pool's absence prevented ownership/restore jobs that ran earlier.

The concrete actor hook inventory is Message::Request (all five ordinary
request modes), WakeHint, MaintainWake; ActorInput::Completed containing
LocalCellsRead; and the existing Effect::Restore, StartRuntime and Publish
arms in actor.rs. Policy-refused actual effects settle with their existing
RestoreCompleted/RuntimeStarted/Published result Err(Failure::Definite), exact
op retained, before any native attempt. RuntimeStarted's refused result has
no isolate and generation 0, never success. These are known pre-attempt policy
errors, not invented uncertain teardown outcomes. Preserve the normal
GenerationChanged event so real cancellation is not disabled. Its attempted
unowned restart meets the same no-native-work refusal. Any lost/unhandled
startup refusal is incomplete, not successful baseline startup.
In particular, upstream LocalCellsRead(Err) only clears resume/confirmation
flags; it does not itself fence or make fleet readiness false. Refusing a
guarded scan therefore also latches a private process foundation failure,
which every later guarded install/bind/admission check must reject. B3 must
never interpret the core's cleared flags as a successful guarded startup.
Ordinary malformed/unavailable request refusal alone does not poison the
entire process; the sticky startup refusal is a separate closed disposition.

Scope classification for refusal covers the entire guarded class prefix,
including an unrecognized tenant suffix or facet-style extension; matching
only the three admitted IDs would allow an ordinary route to activate the
same guarded class under another ID. The closed policy rejects other cell
classes too, since its ordinary primary has no cell class. Admission separately
requires one of the three exact canonical scopes. Ordinary script/class
behavior is unchanged when this policy is absent. Additional actor input
variants must be individually classified during source review; no catch-all
"not guarded" fallthrough is evidence of coverage.

Add `CellRuntime::guarded_generation_by_id(id)` returning a Result<Option<Arc<Generation>>>
from one read lock: zero, absent or actually drained generation yields None;
poison of the outer `CellRuntime.generations` lock yields fixed refusal, with
no fallback to current. This does not make every nested observation fallible:
the existing `Generation::is_drained` calls `Pool::is_drained`, which can
expect-panic on a poisoned pool slots lock. Preserve that limitation explicitly;
do not claim a universal fixed-error path or treat this pool observation as
physical A2 Worker/BackingStore retirement. Add
`CellRuntime::next_guarded_generation_id()` returning Result<u64>, using
checked increment across current and draining IDs. Existing ordinary methods
are unchanged. The returned Arc retains the exact generation; it does not
prove authority, prevent cancellation, or itself supply a guarded activation
pin. B2 must integrate its real dedicated Worker/activation into retirement
before relying on this lookup. Factor the bounded ID-selection/checked-next
calculation so B1 tests exercise that exact production component using
non-V8 index records. The actual Arc/Generation/drained-pool integration is
source-reviewed in B1 and tested through the real generation in B4. Do not
construct an apparently live synthetic Generation/Worker merely to claim a
full runtime lookup test.

## New pure-core resident event and inactive observation

Add exactly the internal Event variant already reviewed:

```text
ResidentGateRequestAt {
    request: RequestId, parent: RequestId, cell: CellId,
    epoch: Epoch, generation: u64, now_mono_ms: u64,
}
State::request_inactive(request: RequestId) -> bool
```

No Serialize/Deserialize/wire encoding is added. The normal timed event clock
update uses the existing max(observed monotonic time) rule. The new handler
admits only if node_authoritative, not draining, exact Phase::Resident epoch,
nonzero exact cell generation, no forced-swap cancellation, and the exact
parent is currently active on that cell, not an admitted gate, and not in
`gate_pinned`. The last condition is explicit: processing ActivityFinished
while an output barrier remains leaves the request in active_requests and
adds it to gate_pinned. That output-only lifetime pin is not permission to
originate another auxiliary gate. A firing alarm or another active request
cannot substitute for this parent. Soft
quiescing/swap may still admit this already-owned parent's auxiliary work;
drain/forced cancellation cannot.

For a fresh request ID, successful admission uses the actual existing local
completion/index helpers, adds admitted_gates/admitted_gate_cells exactly once,
and emits the usual Complete(Local). A failed fresh precondition emits exactly
the existing Complete(NodeUnavailable), or NodeFenced if already fenced; it
does not call ordinary request, read owner, restore, acquire activation, or
queue. An already pending/active/gate-pinned request ID (including request==parent)
is a duplicate: leave request indexes and effects unchanged, as a duplicate
must not complete or corrupt the existing parent's request. The normal timed
event prelude still advances the core's monotonic clock by
max(previous, now_mono_ms); duplicate refusal does not undo or bypass that
clock observation. The shell's B2
owned auxiliary registry will prohibit duplicate submissions; no new replay
capability is exposed by this internal model event.

`request_inactive` is false while any request_cells, active_requests,
admitted_gates, gate_pinned or output-barrier owner/follower retains that ID.
It reads current core indexes; no side effects or mutation. An unknown ID can
be inactive, so this Boolean is explicitly not a processed-finish receipt.
B2 still requires its exact previously admitted ID, exactly-once processed
ActivityFinished, and the later actual inactive observation. The query never
answers activation/effect/Worker/slot disposal. Ordinary GateRequestAt remains
byte-for-byte semantically unchanged, including its existing fallback behavior.
No broader generic verifier or historical event acceptance is tightened.

## Exact B1 paths and staging

Celld source: 14 paths, four new (relative prepared source root):

1. crates/celld/Cargo.toml — only optional local edges and feature.
2. crates/celld/lib.rs — opaque foundation exports; no Ready/actor substitute.
3. crates/celld/guarded_execution.rs — fixed node child declaration only; A2 semantics unchanged.
4. crates/celld/guarded_node.rs (new) — descriptor/process foundation and closed observations.
5. crates/celld/guarded_node/control.rs (new) — real-owner/role/debt/credit primitives.
6. crates/celld/guarded_node/tests.rs (new) — named pure/native foundation tests.
7. crates/celld/generation.rs — shared preconstruction classification/filtering.
8. crates/celld/cell_runtime.rs — exact nonfallback generation helpers.
9. crates/celld/actor.rs — ordinary guarded-input refusal only.
10. crates/celld/runtime.rs — generic construction/dispatch refusal only.
11. crates/celld/js.rs — private marker/module and pre-Isolate ordinary refusal.
12. crates/celld/js/guarded_node.rs (new) — fixed program/config identity and marker policy.
13. crates/logic/types.rs — exact internal event.
14. crates/logic/lib.rs — new resident-only branch and inactive query.

No main.rs/main/cli.rs/main/guarded.rs, output.rs, actor/local_request.rs,
actor/effects.rs, actor/production.rs, host_channels.rs, cell_dispatch.rs,
js/storage_ops.rs or existing js/guarded.rs edits in B1. Their B2/B3 obligations
are not partially implemented here. The prior 25-path ceiling remains the
overall limit, not permission to use it prematurely.

Outer: 9 paths (under tools/foks-celld-guarded): Cargo.toml, Cargo.lock,
runtime-node.lock.json (new), patches/node-series.json (new),
patches/0002-node-admission.patch (new), prepare-runtime.py,
test_prepare_runtime.py, check_graph.py, test_check_graph.py. No run-node,
test_run_node, node-guarded.lock, existing node.lock, existing A2 series/patch
or accepted runtime.lock changes. Actual A3a corrections now in progress
belong to root/list_review; B1 does not edit their source until acceptance.

Add the exact outer `node-qualification` forwarding feature and two optional
celld-to-existing-local-FOKS edges from the already reviewed build-selection
contract. Transport selects `qualification`; metadata-core selects only
`owned-list`, default/native disabled. No registry identity/checksum/native
asset/toolchain change. Preserve A2 feature and all default subclosure edges.
The one-member outer workspace/exclusions and selected celld@0.6.2 binary are
unchanged. Actual lock update/graph review occurs only under root's later
explicit execution gate, after source reviews.

B1's only accepted profile stage is `node-foundation`, with the exact unchanged
A2 patch followed by exactly 0002. The source root, stage, base profile/series,
accepted A2/A3a commits, path roster, pre/postimages, fixture identities, graph
and test inventory are bound together. Existing modes refuse this differently
prepared root instead of relabeling their accepted meanings. A prior prepared
tree/evidence is preserved before fresh reconstruction. Apply strict patch
parser limits 512 KiB/patch, 1.5 MiB node series, 8 MiB source payload and no fuzzy
application/extra paths. Compare the entire reconstructed tree to the source
freeze before any build/test. B1 emits no node-qualified receipt and has no
controller capable of invoking a guarded Action.

## Named focused B1 test inventory

All new Rust tests use prefix `guarded_execution::node::tests::b1_`, are listed
before execution, and run with test-threads=1. They do not reset process-global
A2 state. Private owner component fixtures construct fresh local
OwnerFactory/descriptor values using the production bounded constructors;
their extra fixture instances are not reported as a single qualified node's
global four-owner inventory. Tests needing installed refusal policy share one
actual once-only GuardedNode::install in that test process, retaining its real
process Request and never minting Ready or installing the A2 Capacity. Do not
reset that static state. Local component owners use separate fixture pools;
ordinary no-policy behavior is additionally checked by the default binary's
separate process/regression. No V8 or ordinary node startup is claimed here.

1. `b1_role_geometry_counts_process_owner` — actual process+2 invocation+1 control Requests, no fifth owner; verify distinct real Arc identities.
2. `b1_last_strong_and_registry_retirement_hold_role` — retained real Arc prevents retirement/reuse; expected role return after explicit unstarted retirement; Weak still observed/accounted without claiming backing deallocation.
3. `b1_control_headroom_survives_full_invocation_debt` — exact cap charges on both invocations; third Begin/refusal cannot take control; actual control Request and bounded payload still admitted.
4. `b1_control_lane_busy_does_not_consume_cleanup` — occupied transient lane promptly refuses another; both original cleanup credits still consumable after cancellation/sticky failure.
5. `b1_header_cells_are_process_owned_and_bounded` — four inline record slots; fifth refused; original stamp preserved in transfer; no invocation Request for malformed/unclassified/refused work. No actual socket-I/O claim.
6. `b1_original_stamp_never_renews` — exact deadline/one-second cap math, zero/overflow and already-expired input, clone/same-owner/cancel checks; no sleeps.
7. `b1_debt_failure_sticky_and_cleanup_one_use` — hidden role/actual-owner refusal followed by benign error never clears failure; 8+4 precharged credits usable only once; overflow/duplicate cannot publish or release a physical lease.
8. `b1_owner_construction_and_payload_drop_order` — allocation refusal, never-started owner and unwind dispose concrete test payloads before last strong owner; no actual task/Worker is minted.
9. `b1_layout_and_role_ledger` — every actual B1 size/alignment, Request Arc source convention, fixed collections/Weak cap, nonadditive nested fields and cumulative caps; maxima are assertions, actual values are recorded.
10. `b1_descriptor_accepts_exact_projection` — exact two loaded script records, versions/prefixes, flags and source bytes; actual accepted projection only.
11. `b1_descriptor_rejects_each_extra_capability` — independent one-field mutations for scripts/class/bindings/modules/versions/source/flags/assets/loader/queue/workflow/vars/cron/container/fence; no partial installation or config marking.
12. `b1_boot_reload_share_construction_selection` — same production pure preflight/filter data used by both paths; guarded excluded from both pool inventories, mandatory ordinary primary retained. Label selection evidence, not actual V8 warmup evidence.
13. `b1_marked_worker_refuses_before_engine_use` — real Worker::load_config early marked-config rejection without V8 init. With the shared actual foundation policy installed, real malformed Generation::build refuses before engine initialization. A2 config remains unmarked by inspection/identity assertion, not by constructing V8 in this suite.
14. `b1_generic_refusal_inventory` — every frozen ordinary actor/runtime/config category reaches its actual shared refusal predicate before forbidden effects; actor message component exercises real step with no host/tasks run and asserts no extra route/activation effects. Pure predicates alone are not claimed as full frontend coverage.
15. `b1_exact_generation_never_falls_back` — actual pure index/helper behavior for zero/missing/current/prior IDs and checked exhaustion. Actual Arc identity, drained-pool and poisoned runtime-lock behavior are source-reviewed here and exercised with real generations in B4; no fake Generation. Ordinary helper/source behavior remains unchanged.
16. `b1_resident_gate_accepts_exact_parent` — real core transition after model canonical setup; exactly one Local completion, correct indexes, State::validate remains valid.
17. `b1_resident_gate_refuses_binding_matrix` — unknown/wrong-cell/gate/gate-pinned parent, wrong epoch/generation/zero, nonresident, no authority, drain, forced swap; no owner/restore/start/queue effect or index mutation. Construct the gate-pinned-parent case through real core Output/OutputAt followed by ActivityFinished while its output barrier remains held: the parent is still in active_requests but must not originate a fresh auxiliary gate. Preserve that parent's barrier and pin, refuse the new auxiliary with the existing error, and assert no activation or gate admission.
18. `b1_resident_gate_duplicate_is_inert` — active/pending/pinned/parent ID duplicates leave request indexes unchanged and emit no duplicate completion or other effect, while retaining the normal max(previous, now_mono_ms) monotonic clock advancement. Assert both the inert request state and updated clock.
19. `b1_soft_swap_gate_and_ordinary_gate_regression` — original parent may gate through soft quiesce; forced swap refuses; ordinary GateRequestAt retains its existing fallback and ordinary request behavior.
20. `b1_inactive_observation_is_not_processed_finish` — real output barrier keeps ID active after ActivityFinished; actual model proof completion eventually unpins; unknown ID is inactive but no processed receipt is produced.
21. `b1_foundation_has_no_ready_or_action` — exported closed observations and source/profile contracts name foundation only; no actor/runtime integration object or executable Action is manufactured for testing.

Supplementary Python cases are added to the two existing tooling test files,
not a new helper suite: exact B1 prefix roundtrip; missing/wrong/extra stage and
patch; wrong A3a/base identity; reconstructed source/profile mismatch; unchanged
historical mode refusal; exact two node-only edges/default absence; real
Request-feature/native-SQLite exclusion; wrong package/bin/features/artifact;
focused test inventory empty/missing/extra; no guarded Action acceptance in
foundation. Source inspection/bytes assertions are identified as such and do
not substitute for compiler/test runtime evidence.

At B1 source freeze verify this exact ordinary actor input/effect inventory
against the edited source and freeze immutable program/metadata byte hashes. If an item cannot
be covered by the selected production seam, revise that concrete contract
before tests; never silently replace a missing integration assertion with a
pure helper test. Layout values and fixture hashes are authoring outputs,
not invented in this plan. B2's actual effect roster/task layout proof remains
a separate pre-edit gate and is not satisfied by test7's credit bookkeeping.

## Review, execution, and commit gate

1. Root and independent review this B1 amendment and exact source inputs.
   Wait for accepted A3a; rebind the mutable outer baseline/lock/profile to its
   commit and freeze any necessary narrow rebind correction.
2. Create a separate author source tree from accepted prepared inputs; edit
   only the 14 source paths and 9 outer paths after explicit authorization.
   Freeze preimages/postimages, actual layouts/program bytes, exact tests and
   strict 0002 reconstruction. Author self-review then independent subagent
   review before executing any candidate tool, dependency resolution or test.
3. Root alone reconstructs the selected prefix, reviews actual locked default,
   A2 and node feature graphs, then lists/runs only the named B1 Rust filter:

   ```text
   cargo test --locked --offline --manifest-path <outer>/Cargo.toml --no-default-features --package foks-celld-guarded --package celld@0.6.2 --features foks-celld-guarded/node-qualification --lib -j 2 --target aarch64-apple-darwin guarded_execution::node::tests::b1_ -- --list
   cargo test --locked --offline --manifest-path <outer>/Cargo.toml --no-default-features --package foks-celld-guarded --package celld@0.6.2 --features foks-celld-guarded/node-qualification --lib -j 2 --target aarch64-apple-darwin guarded_execution::node::tests::b1_ -- --test-threads=1
   ```

   Use accepted absolute Cargo/env and recorded network-denying/resource/group
   wrapper. Unexpected package selection, omitted test, zero listing, test
   escape or cleanup uncertainty stops. No celld_internal_tests/full upstream
   suite expansion. If a fixture genuinely needs loopback, freeze its exact
   existing-policy command first; none of the pure B1 tests depends on real
   network/SQL/V8.
4. Root builds and lints the exact actual celld bin in default and node feature
   modes, checks compiler artifacts and absence of Action in B1 help/invalid
   command probes under the owned wrapper. Applicable accepted A2 fresh-child
   cases run with new stage-bound receipts (not historical receipt relabels).
   Build target <=4 GiB/free >=2 GiB and retained process-group ownership remain.
5. Root and independent review final evidence. Commit B1 source/patch/pins/tool
   implementation separately, with all Markdown unstaged. Only then plan/freeze
   B2's real supervised ownership/activation group; B1 is not G1–G6 node proof.

## Final plan review status

Root and independent source reviews passed revision C. The frozen plan body
above has SHA256 `3fe4b670c7c5b789309cb9f04f30a5de8881b652d820744ccc717613069db884`.
Source inputs: `/private/tmp/foks-celld-b1-foundation-plan-inputs.json`,
SHA256 `544427a857e72a533385896a7a7b00fa45b3ec80022da0377db9901763ecaab4`.
Root review: `/private/tmp/foks-celld-b1-foundation-plan-root-review.json`,
SHA256 `13fa62728d216fbfa3bd60721760aa2c99ef741a222eb9ae391226ba9603bed0`.
Independent review: `/private/tmp/foks-celld-b1-foundation-plan-import-review.json`,
SHA256 `860f8385b0292ad7e330cc9802060a93be76f706f2620dd7d7ece2cab7da9644`.

Review corrections explicitly reject a finished parent retained only by an
output barrier, preserve ordinary monotonic clock updates for duplicate
events, and scope the poisoned-lock refusal to the outer generation lock.
Implementation still requires accepted A3a baseline inputs and their rebind.
This Markdown file remains unstaged.

## Reviewed staging amendment

This amendment supersedes the corresponding staging details above and passed
root and independent review. Its frozen source is
`/private/tmp/foks-celld-b1-staging-addendum.md`, SHA256
`24c61977c9cd696eeaf352d67f9a908523a795c6003da1f357186ecdb7c51d94`.
Root review: `/private/tmp/foks-celld-b1-staging-root-review.json`, SHA256
`7de4597d8afed4d39ef3ba14f360449aa2edb33e97aabdf2fde76d2410f40e5b`.
Independent review: `/private/tmp/foks-celld-b1-staging-import-review.json`,
SHA256 `467910bd0ab52f4035061a072e94e097c7003a482182636fd7e6576d8e8438e7`.

This narrow amendment refines B1 revision C, SHA256
`3fe4b670c7c5b789309cb9f04f30a5de8881b652d820744ccc717613069db884`.
It is planning only. Preserve revision C and its reviews. Implementation still
requires accepted A3a, its exact input rebind, and root/independent review.

B1 must pass strict feature-bin Clippy without dummy calls, wider capability
exports, or dead-code allowances. Its production descriptor/construction and
ordinary actor/runtime refusal guards remain unchanged. The exported opaque
`GuardedNode::install`, `stop_admission`, and closed foundation observations
remain the only closed foundation API. Their export does not assert that the
B1 CLI invokes installation; no guarded Action or positive readiness is added.

The production `OwnerFactory::new` and shared `try_owner` implementation still
create the actual process Request and enforce the same 1/2/1 role geometry,
original stamps, cumulative debt caps, precharged retained storage, and sticky
failure. The invocation/control branches are the same factory code exercised
by component tests. Actual role values may appear in the fixed geometry used
to construct semaphores and report limits; do not add pretend invocations to
make variants appear used. `OriginalOwner::charge` and `check` remain real
production paths; `cancel` serves actual `stop_admission`. The unstarted-owner
retirement implementation serves genuine failed-construction cleanup and the
component tests; it is not a native teardown or physical-capacity release.

Use these exact limited test gates until the corresponding real caller lands:

* `AcceptedStamp::capture` and its exact-Instant test constructor are
  `cfg(test)` in B1. B3 supplies production capture immediately after actual
  accept. `control_deadline` remains in the shared role factory, preserving
  the original acceptance stamp and one-second control cap.
* `OriginalOwner::same_owner` is `cfg(test)` in B1; it compares actual Request
  Arc identity. Production registry retirement validates its actual held
  Request directly, without a fabricated second owner.
* `CleanupCredits::consume_closed`, its closed consumption-kind enum and
  returned accounting `Credit` are `cfg(test)` in B1. Production retains the
  precharged 200-KiB reserve and fixed counters. B2 brings the real consuming
  APIs into production with its reviewed complete effect/control roster.
* Private header-cell acquisition/transfer conveniences used solely by the B1
  model fixture are `cfg(test)` until B3 owns actual sockets. Fixed process-
  charged header-cell storage and its bounded layout/occupancy observations
  remain production. No uncharged parser interval or socket-I/O claim follows.

Defer `CellRuntime::guarded_generation_by_id` and
`CellRuntime::next_guarded_generation_id` to the B2/B3 group introducing their
first real caller. Preserve their reviewed exact/no-fallback/checked-next
contract. Current `next_generation_id` is called only by `main.rs:857`, a path
excluded from B1; ordinary generation resolution is used by
`runtime.rs:1020,1044,1059`. Adding uncalled private Arc/lock wrappers now, or
exposing them publicly to evade linting, is unnecessary. Do not change those
ordinary methods or routes in this amendment.

B1 test 15 retains only the pure exact-index and checked-next calculation
matrix, in the existing new `guarded_node/control.rs` under `cfg(test)` and
called by the existing test module. Label it algorithm/component evidence,
not an already-integrated production lookup. B2 moves those same calculations
into production with its genuine private wrapper/caller; actual Arc identity,
outer-lock poison and nested drained-pool behavior remain runtime gates there
and in B4. No synthetic live Generation/Worker or function-pointer dummy use.

Consequently B1 does not edit `crates/celld/cell_runtime.rs`: its actual source
roster becomes 13 paths, still four new, within the prior 14-path ceiling.
The nine outer paths and 21 named B1 tests remain. Strict stage reconstruction
must bind this actual roster. No new path, dependency, backend, wire change,
A2 capacity operation, or downstream B2/B3 implementation is authorized.

The source freeze must identify these cfg gates and genuine production call
edges. Source review precedes candidate execution; strict actual build/lint
and focused test listing remain execution gates, not conclusions of this
read-only inspection. Test-only source is never presented as a serving proof.

## Accepted A3a input rebind

The accepted implementation base is now `fee8db31ae8df590e32bc02f8c8de815d9f7ecea`.
The input rebind passed root and independent review. Frozen plan:
`/private/tmp/foks-celld-b1-accepted-a3a-rebind.md`, SHA256
`dcef516eacec243e7c68240d2db90694c10e8d96a6d08e05bd720a7f61210b6b`.
Root review: `/private/tmp/foks-celld-b1-accepted-a3a-rebind-root-review.json`,
SHA256 `b40c400246fbe260c5b2c8d93e0b940e2d472891e91fbc6bb60b45c302b63d5b`.
Independent review: `/private/tmp/foks-celld-b1-accepted-a3a-rebind-import-review.json`,
SHA256 `0aa8d9494573a01111b442021564b054f44cf31d206c2c684fd57e9c3396cded`.
The exact tooling/materialization interface remains the final pre-edit gate.

This is the mandatory immutable input rebind for B1 revision C
(`3fe4b670c7c5b789309cb9f04f30a5de8881b652d820744ccc717613069db884`)
and its approved staging amendment
(`24c61977c9cd696eeaf352d67f9a908523a795c6003da1f357186ecdb7c51d94`).
It changes no behavior, scope, API, limits, or acceptance criteria. The original
plans, source snapshots, author reviews, root reviews and independent reviews
remain unchanged, with additional byte-identical copies under
`/private/tmp/foks-celld-b1-pre-rebind/`.

The implementation base is now accepted A3a commit
`fee8db31ae8df590e32bc02f8c8de815d9f7ecea`, replacing the provisional A2 HEAD
in the old B1 input observations. Its parent remains accepted A2
`dca38eecb08263a3594f70025df7a309ed3682a5`. The validation worktree is clean at
that A3a commit. The acceptance and commit receipts are bound in the companion
input inventory; accepted A3a evidence means only the ordinary local canonical
baseline described there, not guarded serving or abrupt host/AZ recovery.

The accepted `node.lock.json` is profile `a3a-canonical-node-baseline`, SHA256
`af0c4f23c08fd2c1d17126c0b7c2d68720ecd275840945de444eae7d46bcc866`.
Both controller/preparer profile constants and the actual literal JS/Wrangler
bytes match it. This incorporates the accepted fixture correction: SQL
identity is source/build-derived, with no forbidden `sqlite_version()` probe.
B1 does not edit this baseline profile, controller, or its tests.

Prepared celld remains the unchanged `a2-physical-lifetimes` source profile.
The existing `runtime.lock.json` hash is
`4e94be5e3a8f86007bcdc9c0814ce2fa6dfb043d612d006a20e47b490df67b49`;
existing series hash is
`bf15b77b32416914a1d96426aa58fc65d1b436e0645c73dcc11ad91f6e9c0ea6`;
prepared manifest hash is
`0b618d7be50b44d5a8eeecbbab851ef0e3c3c9b2de64c5e33afbc6f9259489c5`.
Every one of its 282 regular files, contents, Git blob identity and executable
mode matches the manifest; no extra files or symlinks were admitted. The
accepted upstream commit/tree/archive and native/toolchain/engine pins are
retained verbatim, not updated or requalified by this rebind.

Relative to the prior provisional B1 inputs, only `prepare-runtime.py`,
`test_prepare_runtime.py`, and `node.lock.json` changed. All recorded celld/A0
source inputs and other prior rows still match. The companion inventory binds
all accepted tracked outer files to Git, refreshes the affected preimages and
profile headers, and records the seven required new-file absences. Existing
read-only callers are rechecked: `main.rs:857` alone calls ordinary
`next_generation_id`; ordinary generation lookup also occurs at
`main.rs:2406` and `runtime.rs:1020,1044,1059`. This adds no B1 caller to the
deferred private generation wrappers.

The exact B1 source roster is 13 paths, four new, as amended: celld Cargo/lib,
guarded_execution, new guarded_node/control/tests, generation, actor, runtime,
js, new js/guarded_node, and logic types/lib. `cell_runtime.rs` is read-only in
B1. The outer roster remains exactly nine: Cargo.toml, Cargo.lock, new
runtime-node.lock.json, new patches/node-series.json, new
patches/0002-node-admission.patch, prepare-runtime.py/test_prepare_runtime.py,
and check_graph.py/test_check_graph.py. The companion JSON spells out each
path/preimage or absence. The 21 named component tests and their claim limits
are unchanged.

The future B1 profile must bind this accepted A3a commit/profile alongside the
unchanged A2 base/patch and exact node-foundation 0002 source/fixture/test
inventory. It must reconstruct its own stage prefix and refuse mismatched
prepared roots; historical A1/A2/A3a receipts must not be relabeled. The exact
new profile/tooling interface is an implementation coordination freeze after
rebind review, not an invented schema or implemented mode in this document.

Root and independent review of this rebind precede explicit implementation
authorization. Subsequent source review must still prove charged bootstrap
allocation and acyclic ownership, original-owner retirement, exact refusal
coverage, actual layouts and strict reconstruction. Root alone executes
candidate tools/Cargo/tests after their source gates. No candidate imports,
source edits, dependency resolution, tests, build, native run, or download
occurred during this rebind. Repository Markdown remains root-owned/unstaged.


## Tooling interface accepted after A3a

The exact authoring, reconstruction and qualification interface below passed
root review (`01c3c3e9`) and independent review (`81c273e5`) before source
authorization. Its frozen revision-b SHA256 is
`8cf1ce432da1189cb91b5e372382a4c164085cfe607ccc5e1396f4213ba2600c`.
Rust changes are authored in a separate ordinary source copy; the accepted
prepared source and all A3a evidence remain preserved. The four Python tooling
files have a separate author. Root alone materializes Cargo.lock and executes
reviewed candidate code, after both source reviews. No B1 validation or commit
has occurred yet. All repository Markdown remains unstaged.

# B1 stage and authoring interface

This is a coordination companion to the accepted-A3a input rebind
`dcef516eacec243e7c68240d2db90694c10e8d96a6d08e05bd720a7f61210b6b`,
B1 revision C and its staging amendment. It implements their existing
13-celld/9-outer-path scope without changing foundation capabilities. Root and
independent review precede source authorization. No source, prepared input,
lock, test, build or candidate tool has been changed or executed here.

## Separate authoring, reconstruction and lock materialization

After authorization, the Rust author copies the fully verified accepted
`runtime/source` into `/private/tmp/foks-celld-b1-source-author`. This is an
ordinary private copy, not a Git worktree or a replacement prepared tree.
Only the approved 13 paths may change there; four are additions. The accepted
prepared source, manifest, native asset receipts and every A3a evidence file
remain untouched. Outer authors edit only their agreed paths in the validation
worktree. The Rust author owns Cargo.toml and the new profile/series/0002 patch;
the tooling author owns the four existing Python files. Cargo.lock is reserved
for root, despite belonging to the final nine-path commit roster.

The author freezes the exact 0002 unified patch, every preimage/postimage and
the entire resulting tree inventory from the private copy. The implementation
must retain the strict parser restrictions, 512-KiB per-node-patch, 1.5-MiB
node-patch-series and 8-MiB source-payload limits. Source review covers the
private Rust postimages as well as the serialized patch and tooling; a patch
file alone is not a substitute for reviewing its code.

After both source reviews, root preserves the accepted prepared tree at
`runtime/evidence/accepted-a3a/prepared-inputs/source` and its sibling manifest at
`runtime/evidence/accepted-a3a/prepared-inputs/source-manifest.json`, verifies
that preservation, and uses the reviewed preparer to reconstruct from the
original pinned archive plus
exact 0001 and 0002. It compares all files/modes/hashes with the author freeze
before any Cargo operation. A wrong existing tree or manifest refuses; there
is no implicit replacement, deletion, mutable-source relabeling or test of the
author copy. Verified native bytes may be reused only with a new stage-bound
receipt; their accepted receipt remains preserved.

Before source review the author may write an expected lock postimage only to
`/private/tmp`, derived as a data transformation of the accepted outer lock.
The only proposed lock change is adding `foks-hosted-transport` and
`foks-metadata-core` to celld 0.6.2's dependency list. Every package identity,
checksum and other dependency list stays identical. The accepted lock is
SHA256 `6d838801444d99b44acf0b8ef6872c80e24929660557ec4dc0a39a704c6dcb0d`.
The profile pins both this preimage and the exact proposed postimage before
source review; it does not use a placeholder or deferred trust field.

Root alone subsequently lets the reviewed, offline Cargo operation materialize
the actual outer lock. Source preparation may run while that lock is still
its pinned preimage, because it does not resolve/build/test anything. All
stage graph collection, graph verification and build/test commands require
the exact pinned postimage. Root compares actual bytes and the parsed two-edge
delta before collecting the three selected graphs. A mismatch stops for an
explicit narrow source/profile delta review; neither a successful Cargo exit
nor compatible registry versions authorize refreshing pins. Matching expected
bytes need no source mutation. Actual graph/evidence receipts add observed
hashes externally and do not modify reviewed code or profile.

The one planned lock-materialization command, expressed as exact argv, is:

```json
[
  "/usr/bin/sandbox-exec",
  "-p",
  "(version 1) (allow default) (deny network*)",
  "/Users/raymond/.rustup/toolchains/1.95.0-aarch64-apple-darwin/bin/cargo",
  "metadata",
  "--offline",
  "--manifest-path",
  "/Users/raymond/Code/foks-rs/.claude/worktrees/metadata-guarded-list-validation/tools/foks-celld-guarded/Cargo.toml",
  "--no-default-features",
  "--features",
  "foks-celld-guarded/node-qualification",
  "--format-version",
  "1",
  "--filter-platform",
  "aarch64-apple-darwin"
]
```

Root runs it only after the source/reconstruction gates, from that exact outer
manifest directory, with the unchanged pinned controlled environment,
network-denial policy and owned process-group/Cargo slot. The existing finite
metadata bound is 120 seconds; normal log/resource/cleanup limits apply. The
absence of --locked is intentional for this one materialization operation;
there is no package update, online fallback or build/test. Its metadata is
unqualified materialization evidence, not any of the three accepted graphs.
Root checks the actual lock against the already-reviewed expected bytes and
exact two-edge delta immediately afterward, before any stage graph/build/test;
all later Cargo commands include --locked --offline. A timeout, missing offline
input, unexpected lock bytes or uncertain child cleanup stops this sequence.

## Closed profiles and patch chain

Use `runtime-node.lock.json` with exactly these top-level keys:
`format`, `profile`, `target`, `accepted_a2_commit`, `accepted_a3a_commit`,
`base`, `pins`, `node_series`, `cargo_lock`, `qualification`.
`format` is integer 1, `profile` is exactly `node-foundation`, target is the
accepted aarch64-apple-darwin target, and commits are the accepted A2/A3a
commits in the rebind. No caller-selectable source path or future stage is
accepted in B1. An image means the closed object `{path,bytes,sha256}` with
fixed relative path, strict positive integer byte count and lowercase digest;
booleans do not pass integer checks. Duplicate JSON keys refuse throughout.

`base` has exactly `runtime_lock`, `patch_series`, `node_profile`, and
`source_manifest`: images of the accepted runtime.lock.json, patches/series.json,
node.lock.json and preserved accepted prepared manifest. Their hashes are the
ones in the rebind. The source_manifest image's exact path is
`runtime/evidence/accepted-a3a/prepared-inputs/source-manifest.json` and its
contents are checked at that preserved path. It is never resolved to the new
`runtime/source-manifest.json`. The last item binds historical identity, not a
demand that the newly prepared manifest equal the old manifest. `pins` is a complete JSON
value copy of the unchanged accepted runtime.lock.json, checked equal to that
verified file. It supplies all existing compiler, native, engine, registry and
resource policies without a second independent allowlist. Its nested A2
profile remains historical; the selected enclosing profile is foundation.

`node_series` is the image of patches/node-series.json. `cargo_lock` has
exactly `before`, `expected`, `added_edges`. The first two are images of the
same fixed Cargo.lock path. `added_edges` is exactly the two closed
`{from,to}` records celld -> foks-hosted-transport and celld ->
foks-metadata-core, with local package versions and paths additionally
verified by metadata. No registry or other upstream-edge relaxation follows.

`qualification` has exactly `claim`, `source_roster`, `tree`, `program`,
`deployments`, `tests`, `modes`, `not_qualified`. Claim is
`b1-foundation-only`; source_roster is exactly the amended 13 sorted paths.
`tree` has `{files,payload_bytes,inventory_sha256}`; the hash is SHA256 of
UTF-8 compact sorted-key JSON of the full sorted file rows
`{path,mode,bytes,sha256,git_sha1}`, with no newline. This inventory has no
profile hash, preventing a self-reference. Actual values are frozen from the
reviewed author tree, not invented by this companion.

`program` has exactly `primary_js`, `fixture_js`, `wasm`, `primary_metadata`,
`fixture_metadata`, each `{bytes,sha256}` of actual fixed source/metadata bytes.
`deployments` has exactly `primary` and `fixture`, each `{version,prefix}` with
the actual canonical version/prefix bound to those metadata bytes and exact
loaded projection. This does not claim raw loader JSON survives in memory.
`tests` has `{prefix,names,threads}`: the approved prefix
`guarded_execution::node::tests::b1_`, all 21 exact full names from revision C
(with test 15's staging-amendment semantics), and integer threads=1.
`modes` is the exact table below encoded as an object whose three keys each
have `{root_features,celld_features,celld_foks_edges,direct_root_tokio}`.
`not_qualified` is exactly `["guarded-readiness","guarded-action",
"same-process-serving","host-az-recovery","whole-node-retirement"]`.

`patches/node-series.json` has exactly `format`, `profile`,
`accepted_a2_commit`, `accepted_a3a_commit`, `base_series`, `patches`.
Format=1 and profile=node-foundation; base_series is the accepted series image.
`patches` is exactly two entries: an exact JSON-value copy of accepted 0001's
entry, then the closed 0002 entry, using the existing entry structure
`{path,bytes,sha256,files}` and each file's
`{path,mode,before,after}` / existing image structure. Only 0002's 13-path roster
is new. Its preimages are compared against the reconstructed 0001 postimages;
unchanged paths remain original upstream. Reverse checking must walk the
chain in reverse, including multiply touched files, rather than compare 0002
against upstream. Four additions require absence at that point in the chain.
The base-series copy, original patch bytes and old profiles stay unchanged.

The selected prepared manifest uses format=3, profile=node-foundation and
kind=patched-source, with the existing full file rows plus exact
`input_sha256` (new profile), `base_input_sha256` (old runtime.lock),
`archive_sha256`, `tree`, and `patch_series_sha256` (node series). Its top-level
keys are exactly those eight identity fields plus `files`. Archive-only output
uses the same identity fields and kind=upstream-archive-only. Only the final
patched-source inventory is compared with qualification.tree. The
upstream-archive-only inventory is instead verified against the original
pinned upstream archive/tree roster; it cannot match or stand in for the
patched-source inventory. No manifest self-hash is stored.

## Modes, dependency declarations and actual selected targets

| Mode | root features | celld features | active celld FOKS edges | direct root Tokio |
| --- | --- | --- | --- | --- |
| node-foundation-default | [] | [] | [] | false |
| node-foundation | [node-qualification] | [foks-guarded-node-qualification, foks-guarded-qualification] | [foks-hosted-transport, foks-metadata-core] | false |
| node-foundation-lifecycle | [lifecycle] | [foks-guarded-qualification] | [] | true |

All three require the same reconstructed foundation source. Historical
identity/lifecycle/node-baseline modes keep their old profile/source meanings
and refuse this root. Do not infer stage from an environment variable or
silently select the newest profile. For source/archive/assets preparation and
verification, add only optional `--stage node-foundation`; omission preserves
historical behavior. The three new `--mode` values select this same stage for
graph/build and cannot select a different stage through another option.

The optional celld manifest declarations always exist in all three graphs:
transport has path ../../../../../../crates/foks-hosted-transport,
optional=true, default-features=false, features=[qualification]; core has the
corresponding foks-metadata-core path, optional=true,
default-features=false, features=[owned-list]. There are no target-specific,
build/dev, renamed or registry alternatives. The feature explicitly uses
`dep:` entries and forwards foks-guarded-qualification. Validate declarations
separately from selected normal/build edges: only node-foundation activates
the two new celld edges. Default/A2 celld closures remain FOKS-free even though
the already-existing outer harness closure contains FOKS. The entire selected
FOKS subclosure still forbids SQLite/native metadata. Preserve every other
upstream selected-edge identity and the sole outer workspace member.

For default/node builds select exactly outer package foks-celld-guarded and
celld@0.6.2, `--bin celld`; node mode adds only
`--features foks-celld-guarded/node-qualification`. Existing target/offline/
locked/no-default-features/-j2/resource/network wrappers remain. Strict Clippy
uses the same primary binary. Compiler JSON must bind the actual prepared
main.rs, celld package/bin, expected selected feature set and executable, and
exact boolean build-finished success. Capture bounded --version/--help and
unknown `--foks-guarded-node-qualification` refusal from that actual binary;
these cannot manufacture installation or Ready. The unchanged main/cli source
identity plus those probes establishes only absence of the new Action.

Default and node modes produce the same fixed target executable path. Finish
capturing each mode's exact compiler-artifact record, executable size/hash and
actual version/help/refusal probe outputs while that mode retains the sole
execution slot. Verify the executable hash before and after its probes and
final source recheck; finalize the mode-bound receipt before another mode can
build or overwrite that path. Never rebuild first and then probe the stale
mode's pathname. A later receipt cannot relabel the earlier artifact or use
its outputs as evidence for a different feature set.

Foundation mode also lists/runs exactly the approved private library filter
with the existing explicit two-package Cargo test commands from revision C,
serially. Listing must contain the 21 full names exactly once, no ignored or
extra test; execution must report all 21 passed and zero failed/ignored. Bind
both commands, raw stdout/stderr hashes, filter, names and threads in the
receipt. No celld_internal_tests, implicit full upstream suite or invented
runnable node is introduced. Test 15 stays component-only as amended.

Lifecycle mode reuses the unchanged outer lifecycle feature, fixed A2 program
and fresh-process lifecycle integration test, with the same finite commands.
The raw unchanged identity/lifecycle reports may still name their embedded A2
recipe; the encompassing receipt explicitly calls this a regression on the
foundation source and binds that actual tree, lock, graph and executable.
This is not a historical A2 source receipt or a guarded-node claim. No main.rs
or lifecycle adapter change is permitted merely to rename that raw diagnostic.

## Receipts and execution ownership

Graph evidence keeps format=1 but requires explicit profile=node-foundation
and one exact new mode; no omitted-mode compatibility applies. Its saved
source is the format3 manifest and its selected local inputs include the new
profile/series/0002 plus unchanged base images, current Cargo files, the four
tooling files, and existing selected source inputs. The derived graph report
adds closed `foundation` fields `{profile_sha256,series_sha256,
accepted_a3a_commit,cargo_lock_sha256,celld_foks_edges,declared_foks_edges}`.
Both declaration and actual-edge sets are verified independently.

The outer build receipt retains the existing owned-command structure and
adds the selected stage/mode/profile/series/source/graph/lock and executable
bindings. Closed claims are respectively `b1-default-bin-regression`,
`b1-foundation-component-only`, and `b1-a2-lifecycle-regression`.
Foundation's `focused_tests` has exactly `{prefix,names,threads,list,run}`;
list/run are the actual owned-command receipts. Default/node `probes` records
version/help/invalid-option actual owned commands and validated outcomes.
All three remain incomplete until the full command, source recheck, resource
and physical process-group cleanup conditions pass. Historical receipts are
not refreshed. Root alone runs candidate tools, resolution, tests/builds and
probes after reviews and retains the sole Cargo/process slot until cleanup is
established. No actual node service, port or HTTP fixture starts in B1.


## Reviewed actor test seam and scan-error amendment

Root plan review (`7f3cff8b`) and independent review (`93a716e5`) passed
before these edits. This amendment supersedes the earlier 13-path Rust roster:
14 Rust paths, still four additions, nine outer files and 21 focused tests.
The two additional test observations are absent from production builds.

# B1 actor test observations and scan failures

This narrow amendment follows reviewed foundation revision C, its staging
amendment, the accepted-A3a rebind, and tooling interface revision b
`8cf1ce432da1189cb91b5e372382a4c164085cfe607ccc5e1396f4213ba2600c`.
It preserves the 21 focused tests, nine outer implementation paths, default-off
feature and absence of guarded readiness/Action. Root and independent review
precede these seam edits and their inclusion in the source freeze.

## Test seam

The actual `actor/effects.rs` hides StepOutput's effect queue and places every
existing observation behind `celld_internal_tests`. B1 must not enable that
flag; draining the queue would execute work and cannot prove it stayed empty.
Add only `crates/celld/actor/effects.rs` to the Rust patch roster, bringing the
actual total to 14 paths, still four additions. On the existing read-only
`StepOutput::effects_is_empty` method, change only its cfg to
`any(celld_internal_tests, all(test, feature = "foks-guarded-node-qualification"))`.
Keep its implementation and existing visibility unchanged. No production
queue, scheduling, execution or drain behavior changes, and the other internal
observers remain internal-test-only.

Within already-approved actor.rs, add the exact test-only
`HandoffAcceptWaiter::for_guarded_refusal_test(reply: oneshot::Sender<AdoptedCell>)
-> Self`, `pub(crate)` under
`all(test, feature = "foks-guarded-node-qualification")`. It constructs the real
waiter with fixed released_epoch=1 and the supplied channel, making the fifth
ordinary request mode testable without exposing a production constructor.
Test14 uses actual Actor::step, verifies prompt refusal of all five modes,
empty effects/timers and no successful handoff acknowledgement. It does not
spawn/drain jobs, build a fake guarded Worker, mint readiness or change the
frozen test-prefix inventory.

## Failed resume scans

A real `LocalCellsRead(Err(_))` also means startup did not confirm its local
inventory. Under the installed closed policy it must call the same sticky
`node::fail_policy()` used when a successful scan contains a refused cell.
Preserve the original Err value; only policy-rejected Ok rows are replaced with
Err(Failure::Definite). The policy helper is allocation-free and is explicitly
a no-op when no guarded policy is installed, preserving ordinary behavior.
No empty successful scan substitutes for an error. New guarded admission and
future readiness must continue to check this original process failure.

The scan-error case belongs in the existing test14/component assertions and
source review. No additional named test, path, dependency, wire format, engine,
backend or downstream B2/B3 runtime integration is introduced. Preserve all
accepted evidence and leave Markdown unstaged. Both complete source reviews
still precede candidate execution.

## Actor assertion-helper source-review amendment

# B1 actor-local assertion helper

This source-review amendment extends the accepted test-only actor seams. The
existing 14-path Rust roster, nine outer paths and 21 named tests stay fixed.
There is no production behavior, visibility, event or runtime capability change.

CompletedEffect fields/constructors, Actor state/maps and execute are private
inside actor.rs. The existing handoff waiter and empty-effects observer alone
cannot exercise the planned LocalCellsRead and exact Restore/Start/Publish
refusals from guarded_node/tests.rs. Add one actor-local assertion helper under
cfg(all(test, feature = "foks-guarded-node-qualification")), called only by
b1_generic_refusal_inventory. It adds no independent #[test] and no public
production API. It may inspect actual private Actor fields and construct real
CompletedEffect/Effect values; it must not fabricate a runtime, successful
activation, Ready, ownership proof or physical-capacity release.

Use the canonical in-memory/no-host Actor constructor under a local current
thread runtime, with no bucket/network/listener. Start the Actor once but drop
its unpolled startup output before the refusal checks; do not drain or execute
queued native work. Assert all five ordinary request modes reject before maps,
core routing, effects and timers change; dropped handoff waiters report no
adoption. Check wake/hint and exact-op Restore/StartRuntime/Publish refusals,
including no-host branches, without scheduling their futures. Preserve normal
GenerationChanged semantics and verify the resulting source-order inventory.

Exercise empty local scan separately from rejected guarded rows, unknown-class
rows and actual scan errors. The installed foundation is once-only and cannot
be reset. Only this test may depend on it being healthy: perform all positive
global-policy observations before the first sticky scan refusal, then assert
failure remains latched. Other tests use local descriptor/factory fixtures and
must not depend on test order or reset/replace the process policy. The first
rejected scan demonstrates the latch transition; subsequent error cases prove
preservation/refusal behavior while already failed, not independent first-fault
causality. Source review separately verifies both actual error branches latch.

Root authors and directly reviews this helper; the independent source review
must cover its exact bytes before candidate execution. No additional test,
process, source path, pool, global-reset seam or production export is authorized.

Plan reviews passed before helper authoring: root `215d58b7649e3a0db2cbb592db5be2d1178c16d39e6238a26fddaf03fc9d7356`; independent `7ea16154f32e54d00da7b53ec7bd95e596bebab910fc39cc33456b8c09e5f95b`.

Tooling source review found and corrected the new native receipt Boolean/integer comparison. Frozen source-b is `918caee37b7f63cabf22fe1f4af2940f0f0529e3ea16fadb5fe96f227209db34`; direct review `7e3161500df848d9300b708b8fc30d118e5f0c8e5d11ccaef6b5a373c0c08bc7`; independent review `11a117b77c3e8a9a1aaa192ccef8fac4797536c023fa337224999873823501c4`. No tests/builds have run for B1 yet.

## First execution and packaging correction

The source-a tooling run passed all 161 tests (zero errors, failures, or skips; 11.719 seconds). Receipt: `/private/tmp/foks-celld-b1-tooling-tests-a/receipt.json`, SHA `63e89d32770da8e39f2b9f4ad4528056bff57c4eefd4ce719521d577ee14dbc5`.

The first actual foundation reconstruction then rejected 0002 because its Git diff headers were outside the preparer’s closed plain unified-diff grammar. No new source tree was published and no Cargo or Rust build ran. Failed staging was cleaned; the original accepted tree and manifest were restored from the explicit retirement location, and the separately preserved accepted copy remains unchanged. Failure receipt: `/private/tmp/foks-celld-b1-reconstruction-a-failure.json`, SHA `89b4f42610d3a50a9e05cade2065b80c69d68c9f66029f8c3dc0f5419aad947a`.

A narrow correction changes only patch serialization and dependent series/profile hashes. Both source reviews must approve it before rerunning the tooling suite and reconstruction. Rust postimages and Python implementation remain unchanged.

The source-b serialization correction passed direct review (`8839f4c70c8829cc70af2e1aa88073745f76662f4f7d6a405bad3f9627c12658`) and independent review (`d8f4d557f8af340ac3d5234607803afa470d5035dfba33abd5aaa092b378c7f9`). It removes exactly 32 Git metadata lines; all 53 hunks and 14 Rust postimages are unchanged. The repeated tooling suite passed all 161 tests, with no failures or skips (`/private/tmp/foks-celld-b1-tooling-tests-b/receipt.json`, SHA `552e3d9c87143a37a6dc8c01d2e04a598897a5d840f597f08c9e41bdd38d8363`).

Actual reconstruction now passes: all 286 files and 6,521,984 payload bytes match the reviewed full-tree inventory. Reconstruction receipt: `/private/tmp/foks-celld-b1-reconstruction-b.json`, SHA `95d8f89061969e8b9145e609464b62778fa65202d90178d9c7403af8c162c47f`. The old prepared tree remains preserved. Offline Cargo metadata produced exactly the expected two local dependency edges and no other lockfile changes (lock SHA `e05fe62986b04425769de96caae00224aeafa9513a9c38468a81410c28d65053`). The three mode-specific graph/build qualifications follow; these results do not yet establish Rust compilation or runtime correctness.

All three dependency graph qualifications passed. The first default binary build and strict Clippy also passed, but its version probe rejected upstream's expected macOS allocator diagnostics despite process exit 0. The raw output is retained in `runtime/evidence/foundation-xnr8l8h3`; that qualification remains failed. The correction is restricted to the Python output oracle and its regression test, allowing only the exact source-backed allocator diagnostics while preserving raw evidence and rejecting unexpected output. Both source reviews precede the targeted rerun and refreshed graph/build evidence. No Rust or runtime profile change is involved.

The narrow source-c oracle correction passed direct review (`9e9d04dbf8d7f5aea04fcb95a0540d2aeeed3d2e6e836d3222076b3d3dfe8602`) and independent review (`5e36299038bbe32ce475a37ca04cc8f40611ed6e142ede3456ebf5fd9cdea68b`). The targeted test passed its positive and negative output matrix (`/private/tmp/foks-celld-b1-probe-tests-c/receipt.json`, SHA `9e64bffc8cfebdc4d28464f7ce1514d8b0c2c1b26b64964d0bc05e8f108af04a`). The full 161-test source-b result remains the broader tooling regression result; only the changed oracle test was rerun. Source-c is `/private/tmp/foks-celld-b1-source-c.json`, SHA `8f87b4b9061d385ee90b4eb1deb9eb56a6f87b2909992b5f6b46e8efe19d303c`. All three graph/build modes are being refreshed against these exact tooling bytes.

Source-c default qualification passed (`foundation-iyy5lj9e`, receipt SHA `a82c464c0d3d8624271b4846473a40dab02ef6640e664424b9aa4b0be4ab1927`). The enabled foundation binary also built, passed strict Clippy and passed all three CLI probes, but Cargo rejected the library test command before compiling or running any of the 21 tests. Its dependency package still declares the unused upstream `lz4_flex` dev-dependency and is deliberately outside the harness workspace. [The pinned Cargo source rejects that combination](https://github.com/rust-lang/cargo/blob/f2d3ce0bd7f24a49f8f72d9000448f8838c4e850/src/cargo/ops/cargo_compile/mod.rs#L344-L355).

The node result in `foundation-rgsagysm` remains failed and the lifecycle mode has not run. A narrow plan amendment will remove only that unused development dependency from the closed qualification fixture manifest; the upstream internal-test suite is outside this qualification. The command, 21-test roster, ordinary dependencies and lockfiles remain unchanged. New source/patch/profile identities, two reviews and refreshed mode qualification are required before accepting this correction.

The manifest-only amendment revision b passed direct (`40f77ee39d30954310109abd7c5352e2840e500711bcf9c443c4d336dc2184a4`) and independent (`3ebf00575c6e01adabf15fd27bb6fbdeff97e087264f01d3e9150d293982f1c9`) plan reviews before editing. Its source-d correction passed direct (`98f07434673af95024d3658610eec87e21e752a5c279b7ebeb61195f41b6b1cb`) and independent (`76ce5c292e6ca933d43fe46569d01f634b618e1018f7d9bd3c80fe765b50e7cc`) source reviews before execution. All 13 Rust code files are byte-identical; only 45 manifest bytes and three derived packaging files changed.

Source-d reconstruction passed with 286 files and 6,521,939 bytes; manifest SHA `d1936bb6662781ffa027ccfdadfdb21606448ae8aeac6920892a297db129ddb7`. All 13 existing FoundationTests passed, with no failures/errors/skips (`/private/tmp/foks-celld-b1-foundation-tests-d/receipt.json`, SHA `5c5cebfc123e32e46c4f04d0c1d3e36c0911f0856c7ac8ecb4680457c471dda1`). Full tooling regression evidence remains the earlier 161-test run plus the source-c oracle matrix. Fresh mode qualification also compares complete Cargo metadata with prior runs, allowing only the removed celld development declaration; ordinary/build edges, versions, features, lower LTX codec dependency and both locks must remain unchanged.

The source-d test compile exposed an incorrect test-only include path: the
canonical CLI parser is `main/cli.rs`. Source-e corrected that path after
direct and independent review. All 21 native tests then ran; 20 passed and
the source-inventory test failed because searching for `fn rpc` matched the
earlier `fn rpc_service`. The real `rpc` entry already refused guarded calls
before allocating channels or jobs. Both failed runs and cleanup records are
retained (`foundation-o_l6ocli` and `foundation-rz4omkpc`).

Source-f uses exact method identifier boundaries, including generic methods,
and adds prefix-collision assertions inside the existing test. Production
code, distance checks, test count, commands, Python and lock remain unchanged.
Its freeze is `/private/tmp/foks-celld-b1-source-f.json`, SHA
`c69f9407c30bc15b88c8219f09ae49dd6fbd4dbaf2ec13f4e49678d4546c1fac`;
direct review `0e7d5dba5e28b77804b69c0e33b92e034ccb94f8eb6257f9a05eeacc854ea3d9`
and independent review `961825cde75746eea49cc64e332b080fc3fa96b3962f374687a86e34588a1a7f`
passed before execution. Reconstruction matches all 286 files and 6,522,456
bytes. Final qualification runs the enabled native tests first, then the
default binary and unchanged A2 lifecycle regression. No prior failed run is
reported as a pass.

Source-f completed all three modes: `foundation-j143zb15` (21 native tests,
receipt `9952c419983bcea4e51637b69a13b87b92495602b179fad1f7b6bd4f7099fe05`),
`foundation-zy6vzbol` (default binary,
`f1366e851d0d461485c58f4cb148afd25b94ce879598161775d98c10a30016f5`),
and `foundation-own01h3j` (all 28 fresh-process A2 scenarios,
`12cb74b54b71685f12ade8511b201dff1a5789c3887435fadb746b682c4242e1`).
Strict Clippy, exact CLI probes, dependency checks and child cleanup passed.

The final evidence review distinguished asserted layout ceilings from recorded
compiled sizes. The original plan requires both. The layout-evidence amendment
(`ea15f7fdae538e2b0b82eb997e97622f9b96d51ad0b8ec01fe46cddaea8e2081`)
passed direct plan review (`23335263b2cac0d8b10060dde139d050c9fc3bbd8e5702da65d2f0527dc2cf28`)
and independent plan review (`4eeb521bebbdc4db7f01d033aabf3f25d862f361db090d153ad76e5edf1946c0`).
Test9 will emit a bounded record of all nine actual native layouts after its
assertions and cleanup; the preparer will validate it against the whole stderr
hash and retain the numbers in the receipt. Commands, production behavior and
the 21-test roster remain unchanged. Two source reviews and refreshed final
qualification remain required before committing.

## Final source-g qualification

Source-g (`39bd812fe5a6179b727fa51996c9c4152be0a50b7c30b415987e0b63027347f8`)
passed direct and independent source review before execution. All 286 prepared
files match the reviewed inventory (6,524,494 bytes). The final runs are:

- `foundation-71zhotsd`: 21 native tests, actual compiled-layout record, strict
  Clippy and CLI probes; receipt `9aca2d14cd1cb072f2bcc2ca3f8603248fa7730041ded109a39b5f8df1ad7da2`.
- `foundation-e8bwypfe`: default binary regression; receipt
  `b7a2bcb95fff32ed65f1262e36c7f2df40bc25fde926ba872f074eedfff5139b`.
- `foundation-_jryw8lp`: all 28 fresh-process A2 scenarios; receipt
  `a325fe9895aaf964085a103cea08eaf20bc0274df46b5e9e06e6e3d319b165fa`.

The measured byte/alignment pairs are Request Arc 144/8, Control Arc 112/8,
Registry Arc 1,016/8, owner factory 64/8, original owner 40/8, header cell 96/8,
cleanup credit state 2/1, control observation 104/8 and Node Arc 320/8.
Initial process debt is 10,776 bytes and the prepaid cleanup reserve is
204,800 bytes. These are representation measurements under the stated Arc
convention; nested/stack observations are not additive allocations or RSS.

The first source-g Python run put its supervisor inside the native sandbox;
that prevented its process-observation helper from running. Fourteen tests
passed and one existing supervision test errored. The failed run is retained
in `b1-foundation-tests-g`. A twice-reviewed direct-supervisor correction
passed all 15 tests in `b1-foundation-tests-g2`, without skips, an expired
alarm or observed remaining descendants. Receipt:
`4a73b1ea496eb8d4cdb7933a59f008f1e493d50a9c401cf6adcbf08258902238`.
The final descendant observation is not a whole-group retirement proof.
A separate read-only inventory preflight ordering mismatch was corrected
before candidate import/reconstruction; all file bytes and modes matched.

Final direct review is `2fd16b9e7507c2244f162b284a77a359d36c5ccfff0ef6fc5cc3f1e8fec36110`;
independent review is `8658d4033abe984896f5b857a41a0a47d8e19f0ccf0d687e4000a20dbe3b20a1`.
They verified 58 command receipts and raw streams, 866 local inputs, exact
feature graphs and unchanged Cargo metadata, resource ceilings and cleanup.
No failed historical run was relabeled as passing. The final commit contains
only the nine reviewed implementation files. B2 remains the next group for
actual activation, effect/native-job ownership, storage.sync and local
retirement; B3/B4 still owe serving integration and recovery qualification.
