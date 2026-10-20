# A2: process admission and physical celld runtime retirement

Status: implemented and locally qualified in `dca38eecb08263a3594f70025df7a309ed3682a5`.
Plan and source reviews, including independent subagent reviews, preceded testing;
direct and independent result reviews passed before the implementation-only commit.
This Markdown remains unstaged. The final production metadata adapter/backend
remains deferred. The detailed plan and amendments below preserve their original
scope; the final evidence and limitations are recorded at the end.

## Result this phase will establish

Add one closed, qualification-only native lifecycle to pinned celld 0.6.2. In
one process it admits at most two guarded invocations and reserves exactly
64 MiB of non-shared Wasm memory capacity for each before constructing its
isolate. Building, running, retiring and quarantined invocations all consume
capacity. Admission refuses immediately; it never waits for a permit. A tenant
has no idle Wasm instance after successful retirement.

Return the Wasm reservation only after physical destruction is established.
Return an execution slot only after both runtime/work destruction and output
release. A2 proves the latter with a bounded owned diagnostic result and
controlled disposal barriers; A3 must connect that same owner to actual FOKS
socket write/flush/disposal and celld activity. A2 is not a FOKS serving,
coordinator, S3, failover, 15-second recovery or end-to-end deadline result.

The 128-MiB total is a **Wasm backing-capacity reservation**, not a whole-process
memory or RSS limit. JS/V8 heaps, compiled code, allocator metadata, native
buffers, SQL caches, stacks and runtime task overhead are separate. In
particular, celld's near-heap-limit path can enlarge its nominal V8 heap limit;
it must not be counted as a hard process-memory ceiling. Record these other
terms and per-invocation creation/destruction costs without assigning a tenant
capacity from the Wasm number alone.

## Source-backed decision

Use a narrow celld-only patch, with no initial Rusty V8 wrapper/native fork.
The proof is a conjunction of closed ownership, joined work, scoped lockers,
no escaped backing references, and normal complete Worker destruction. Unique
Slot ownership by itself does not prove V8 destruction.

| Existing boundary | Consequence for A2 |
| --- | --- |
| `pool.rs:600`, `Pool::grow` constructs a Worker before locking/rechecking its cap, and counts only non-retiring slots. | Do not repurpose that cap. Admit before construction and keep retirement charged in a separate process ledger. Ordinary pool placement remains unchanged. |
| `runtime.rs:1397`, `stop_cell` gives storage back and drops Residency; shared Worker/isolate ownership can remain. The give-back result is discarded. | Cell stop, an empty registry and logical retirement are not physical completion. The guarded close must preserve errors and proceed to consuming Worker teardown. |
| `pool.rs:164`, Slot privately owns `Mutex<Option<Worker>>`; turn closures execute synchronously. | Add a sealed dedicated Slot path, retain its complete task/affiliation/residency ownership, and consume it after those owners drain. Do not return the raw Slot publicly. |
| `js.rs:2194`, WorkerIsolate owns a non-Clone SharedIsolate; its Drop at 2895 only performs preliminary cleanup. | A completion event at Drop entry is wrong. Emit completion only after explicit full `drop(worker)` returns, including all WorkerIsolate fields. |
| `runtime.rs:1608`, fetch dispatch spawns its driver separately from its reply receiver. | Receiving a reply or dropping the receiver does not join the driver. The guarded supervisor owns the actual JoinHandle and its payload. |
| `js.rs:2635–2745`, the watchdog service retains IsolateHandle/runtime state and removes watches under the same mutex used while handling a due watch. | Scope the watchdog through a turn and complete its removal barrier before destruction. No surviving watcher is inferred merely from a JS result. |
| `js/storage_ops.rs:897`, structured clone can retain native backing stores. Worker Loader and other host work have additional task/handle paths. | The first program excludes those capabilities. A generic WorkerConfig or arbitrary native callback is not accepted by the guarded constructor. |

The pinned Rusty V8 source is commit
`8149304f6924411df09a1917cd6de0fc90fa7fec`, crate 152.1.0. Its
[locker implementation](https://raw.githubusercontent.com/denoland/rusty_v8/8149304f6924411df09a1917cd6de0fc90fa7fec/src/locker.rs)
returns a lifetime-bound Locker from non-Clone SharedIsolate. A Locker internally
retains an Arc, releasing it last in normal Drop. A forgotten guard or a panic
before that final release can retain the isolate. SharedIsolate's inner Drop
performs native isolate disposal. Accordingly, only an audited normal teardown
can release capacity; catching a panic and continuing with a fresh permit is
unsafe.

The
[isolate implementation](https://raw.githubusercontent.com/denoland/rusty_v8/8149304f6924411df09a1917cd6de0fc90fa7fec/src/isolate.rs)
invalidates handles during preparation before native disposal; an unusable
IsolateHandle is not a post-disposal acknowledgment. Its failed conversion to a
SharedIsolate also has a leak-preserving error destructor. The
[Global implementation](https://raw.githubusercontent.com/denoland/rusty_v8/8149304f6924411df09a1917cd6de0fc90fa7fec/src/handle.rs)
uses isolate liveness/deferred resets, rather than owning the SharedIsolate Arc.
Worker-owned Realm/runtime-state Globals may finish their drop after the isolate
field; full Worker drop must still finish before completion. All external
request/task Globals must already be gone.

The
[backing-store API](https://raw.githubusercontent.com/denoland/rusty_v8/8149304f6924411df09a1917cd6de0fc90fa7fec/src/array_buffer.rs)
can produce independent shared ownership. The
[SharedRef implementation](https://raw.githubusercontent.com/denoland/rusty_v8/8149304f6924411df09a1917cd6de0fc90fa7fec/src/support.rs)
provides a diagnostic use-count assertion that may wait before panicking. Do
not use it as a production polling/release protocol. In qualification only, an
owned backing probe may prove it is the final native reference after isolate
teardown; its own final reset must precede reservation release. A raw probe
cannot escape or become an uncharged ownership exception.

These public source pages were read directly; no local V8 crate or native asset
was fetched for this investigation. A1 must acquire and verify the selected
crate/build inputs. Before A2 implementation freeze, compare these exact source
contracts with the actual selected crate and record their file hashes. Any
material difference reopens this plan; the tag/name alone is insufficient.

## Fixed process capacity and state transitions

Create one feature-gated `GuardedCapacity` per process through a single native
installation boundary. The first co-located node owns it; later script or
generation construction can only clone that same owner. A second independent
installation fails. A2's diagnostic binary also installs exactly one instance;
panic/quarantine scenarios run in separate diagnostic processes rather than
resetting a quarantined live ledger.

Use two preallocated records, not an unbounded tenant map. Each record stores a
bounded tenant key, a checked non-wrapping invocation generation, state,
reservation flags, sticky taint and a retained clone of the original opaque
native owner. The tenant key is the fixed guarded class
plus its canonical fixed-size Durable Object ID. It is placement identity, not
FOKS authorization. The corresponding scope/epoch is retained exactly; no
string supplied later can redirect the record.

`try_admit` uses a non-waiting critical section and atomically reserves one
execution record and 64 MiB before script preparation, Worker load or Wasm
instantiation. It rejects exhausted capacity, an already occupied tenant,
contention, invalid identity or poisoned ledger. Do not acquire one resource
and queue for the other. Generation exhaustion closes admission instead of
wrapping. Diagnostics are scalar snapshots; they cannot mint a permit or clear
a state.

The intended states are `Free -> Building -> Running -> Retiring -> Free`,
with `Quarantined` reachable from every occupied state. Keep the memory and
execution completion edges distinct:

* While Building/Running/Retiring or uncertain, 64 MiB remains reserved.
* Normal full runtime/backing destruction clears the memory reservation.
* The record and tenant Busy state remain occupied while diagnostic output or
  other admitted native work remains owned.
* Only the conjunction of normal runtime/work completion, complete output
  disposal, complete owned task-payload disposal, and an untainted record can
  return the execution slot. The same generation is checked at every update.
* Unknown completion, lost cleanup ownership, any caught unwind, poisoned
  control, unexpected native alias, or inconsistent counters quarantines the
  record. No automatic retry, timer, report or RAII permit Drop resets it.

Admission flags live in the process-owned ledger. Dropping a lease does not
return them or discard the record's original owner. A lease dropped without its
private terminal operation marks uncertainty where possible and leaves the
record occupied, including its retained native owner. Thus ordinary
unwinding cannot accidentally grant replacement capacity. Node shutdown waits
for its owned jobs; overdue cleanup remains charged. A process exit destroys
the process, rather than proving a successful in-process release.

## Closed API and exact lifecycle

All serving primitives stay `pub(crate)` or narrower inside celld. The only
cross-crate API added by A2 is a feature-gated diagnostic scenario runner with
closed scenario names and bounded reports. It cannot accept script text, Wasm
bytes, WorkerConfig, native closures, arbitrary SQL or caller-created disposal
proofs. It returns no Worker, Slot, Residency, Affiliation, Locker, Global,
IsolateHandle, BackingStore, pointer or ready tenant.

Proposed private types and methods are concrete implementation boundaries:

| API | Required behavior |
| --- | --- |
| `GuardedCapacity::try_admit(GuardedKey, InvocationControl)` | Installs one record before allocation; returns `GuardedAdmission` or an immediate typed refusal. |
| `InvocationControl` | Owns the original absolute `Instant`, cancellation state and an opaque retained native owner. The diagnostic harness supplies the actual A0 Request Arc through this owner, not a replacement request or JSON token. No callback methods on the opaque owner are used. |
| `GuardedAdmission::start(ActivationStorage)` | Moves the admission and original owner into the complete build-task payload before spawn. It constructs only the embedded closed program and one dedicated Slot. |
| `OwnedGuardedRuntime` | Privately owns the Slot, Residency, exact storage identity, same admission/control and complete driver ownership. It exposes only the fixed diagnostic operation in A2. |
| `runtime::drive_guarded_cell` | A narrow wrapper around the existing real `drive_cell_inner` path that returns its result and remains owned/joined by the supervisor. It does not call detached `fetch_cell` and pretend the reply receiver is completion. |
| `OwnedGuardedRuntime::retire(self)` | Stops new turns, joins all owned work, closes the adopted cell with errors preserved, consumes the Slot, destroys its Worker and then completes the private physical edge. |
| `GuardedDiagnosticOutput` | Owns only a fixed bounded native diagnostic buffer and the same execution/reference owner. Its consuming disposal drops the buffer before recording output completion. A3 replaces this fixture edge with actual owned framing/write integration. |

The diagnostic-only public entry is proposed as
`qualify_lifecycle(Scenario, [QualificationInput; 3]) -> Result<Report>` (async).
`Scenario` is a closed enum. Each input holds an opaque `Arc<dyn Any + Send +
Sync>`, the already-created absolute `Instant` and shared cancellation state;
its constructor creates no runtime or readiness. Three inputs suffice for the
two admitted contenders and the refused third. They originate from real A0
Request owners in the outer harness. Serial measurement loops within this
single diagnostic call may retain its same diagnostic owner; this is explicit
fixture work, not proof of per-user frontend admission. A3 must pass one actual
frontend request owner for each real invocation. The runner creates its own
disposable local directory and returns bounded scalar observations, never a
caller-selected database or an arbitrary executable callback.

The opaque retained owner is a lifetime seam, not proof of request admission.
The A2 binary is a closed qualification caller. A3 must place its construction
behind the actual typed native frontend admission and coordinator route; there
will be no remotely supplied deadline, owner token or arbitrary callback.
Charge/measure the fixed control/task layouts before creating those owners in
the harness. A2 does not claim to bound every allocation inside celld/V8.

The supervisor retains a bounded set of task handles for the two records.
Create ordered payload aggregates before spawning, including before the first
poll. Worker-related payloads precede their lifetime owner in field drop order.
A reply receiver, cancelled waiter, completed-but-undropped JoinHandle or
`spawn_blocking` abort cannot release the record. The supervisor may outlive an
expired request solely to finish cleanup; it cannot gain a new publication
budget. If supervision itself is lost, the ledger stays occupied.

For successful retirement:

1. Mark the exact record Retiring so no new guarded admission/turn enters it.
   Observe cancellation without replacing its original absolute deadline.
2. Join the actual build/drive tasks and drop their completed handles and
   results. Dispose InFlight, pending operations, Globals, stream claims and
   watchdog scopes. The closed program has no background, loader or child
   operations. A future extension must enumerate and join any new owner.
3. Give back the exact cell through `Worker::own_cell(scope, None)` on its real
   Slot; preserve failure. Drop Residency and all Affiliation/temporary Slot
   references. Do not claim success from RuntimeManager's current ignored
   give-back error path.
4. Consume the dedicated `Arc<Slot>` using `Arc::try_unwrap`, with no retry loop
   on reference counts. Check turns/requests/cells/adopted cells are zero and
   the worker mutex is no longer borrowed. If the structurally expected unique
   owner is absent, keep/quarantine its reservation; do not infer V8 liveness
   from counters or force-drop another owner's reference.
5. Move the Worker out of the consumed Slot's mutex, dispose the rest of the
   Slot payload, and explicitly `drop(worker)` within the guarded teardown
   boundary. Mint the private completion only after normal return. Its scope
   includes WorkerIsolate's Drop body and every field destructor, not an
   observer at Drop entry or at SharedIsolate handle invalidation.
6. In diagnostic native-probe scenarios, the same reservation also owns the
   probe. Prove the final-reference condition using the pinned diagnostic API,
   reset/drop the final probe normally, and only then clear Wasm reservation.
   The ordinary closed path has no such native backing-store owner.
7. Dispose every remaining runtime/task payload before releasing the execution
   record; an outstanding diagnostic output can keep execution occupied after
   memory has returned. After both edges and all payloads complete, take/drop
   the ledger's retained native owner outside the bookkeeping lock while the
   record remains Retiring; only a normal return may make the record Free. A
   panic during this final release leaves it quarantined. The tenant Busy
   record remains until that conjunction.

This procedure requires the complete no-escape audit in addition to unique
Slot extraction. It does not assume that SharedIsolate lacks internal Arc
owners simply because Slot is unique.

### Partial construction and panic handling

Place a sticky guard before calling Worker construction, entering a guarded
turn, creating its native future, and starting teardown. Preserve taint before
an upstream JoinError is converted to anyhow or a task result is discarded.
Use a guarded turn scope that also restores `CURRENT_SLOT` on unwind; the
current normal-return-only restore must not strand another guarded call's
thread-local identity. Ordinary non-guarded placement behavior is unchanged.

After allocation may have begun, an ambiguous failure quarantines. An ordinary
constructor `Err` may return capacity only through a separately audited
normal-return branch proving that `load_config` has returned after all partial
OwnedIsolate and local handle destructors completed and nothing escaped. An
unwind from setup, into-shared conversion, partial-owner Drop or payload Drop
never uses that branch. Test that exact branch; do not make generic `Err =>
release` or `JoinError => release` rules.

The inspected `IoContext` cleanup catch resumes the first panic unless it is
already unwinding; it can forget that panic payload during an existing unwind.
The guarded outer scope must already be tainted in that case. No other caught
panic may become an apparently clean completion. Source review must revisit
this catch inventory after the patch and actual crate acquisition. A panic
that aborts at an FFI boundary terminates the diagnostic process; it is not
converted into successful capacity reuse.

## Program and backing-store contract

The constructor selects one embedded, hash-bound fixture consisting of a tiny
Wasm binary and fixed JavaScript Durable Object class. There is no configurable
user script. The Wasm bytes are represented as reviewed Rust byte constants,
with an annotated opcode/section inventory, avoiding a new compiler/download
prerequisite. Pin the decoded Wasm and JS hashes in the runtime input record.

The admitted module has one defined, non-shared 32-bit memory with initial and
maximum 1,024 pages, no memory import, no second memory, no memory64, and a fixed
import/export allowlist. The fixture imports nothing and exports exactly
`memory`, `write_page`, `read_page`, `grow_zero` and `grow_one`; page-touch
functions range-check their index against 1,024 before accessing memory. The
fixed JS creates exactly one instance per invocation. Exporting memory to that private JS host is allowed; no JS object,
view or backing handle is exported outside the isolate. Test rejection occurs
before Worker construction, so a changed artifact cannot consume unreserved
memory while being inspected.

Do not pass arbitrary configuration through and merely call it guarded.
`js/guarded.rs`, as a private child of `js`, constructs the known WorkerConfig:
one class, denied egress, no loader, bindings, subrequests, services, containers,
queues, workflow, WebSockets, alarms, timers/background work, dynamic import,
eval, structured clone, SharedArrayBuffer or caller callback. The fixed source
only instantiates the approved module, touches its memory, and uses the real
local storage SQL counter. Ordinary celld's broad installed APIs are not a
sandbox proof for arbitrary code; the proof depends on this immutable closed
program and its audited reachable host operations. Later guarded FOKS host/ABI
bytes need their own replacement profile and reachability review.

The fixture takes the existing local storage path and epoch through the same
`CellStorage`/`own_cell` open path. It creates one counter row, increments/reads
it and returns a fixed-size scalar diagnostic. No serialized database image,
whole-state scan, fake S3 acknowledgment or reconstructed tenant database is
part of this fixture.

A native diagnostic probe captures the actual instantiated Wasm memory backing
store inside a scoped Locker, retains the same reservation, exposes only scalar
length/pointer-equality observations, and cannot leak a raw SharedRef. Exercise
zero-page growth and failing positive growth; require no second backing store
or greater live backing capacity. If zero growth, memory instantiation or an
engine behavior retains an additional 64-MiB store, stop and revise the budget
or artifact policy before acceptance. RSS is corroborating measurement only.

## Stable SQL and the later canonical node integration

Dropping an isolate must not unlink or recreate its tenant database. The A2
local fixture reopens the same real SQLite file through celld's storage API
with the same explicit fixture identity, verifies its counter and schema, and
shows fresh Wasm state after each invocation. This is an application-connection
lifecycle proof; it does not perform fleet ownership changes.

A3 must use the actual node runner and coordinator, rather than promote the
fixture into a server. Its exact integration obligations are:

* Install the process capacity once at shared node startup, outside Generation
  and script pools. The canonical guarded-class StartRuntime branch receives
  an existing admitted invocation and calls the same dedicated constructor.
  Other ingress, alarms, warmups, generation replacements and maintenance
  cannot instantiate that class without a finite admitted native owner.
* Use RuntimeManager's actual scope/epoch, `CellRuntime::db_path` and
  `replication.ltx().activation_vfs` result. A sparse paged file cannot be
  reopened as ordinary SQLite. Use the canonical restore/stop disposition and
  preserve failures; do not synthesize RuntimeStopped or manually erase the
  registry while the actor still considers the epoch published.
* Retain `LocalRequestCompletion.activity` and the original Request through
  actual response capture, local write/flush or disposal. Output completion
  means that local boundary, not peer receipt. Then stop the exact epoch
  through the coordinator and retire its dedicated Slot. Busy/retiring
  prevents overlapping reactivation until physical teardown is known.
* Keep `storage.sync()` before Fresh and the original fixed SQL catalog/owner
  checks; hidden platform SQL has a separate accounting ledger. Reapply the
  fixed connection settings outside transactions after reopen, including
  `recursive_triggers=1`; verify them before request admission.
* Carry the same original monotonic control into the A4 SQL progress/interrupt
  and wait paths. A2's native admission checks and lifetime tests do not prove
  that existing celld handler/SQL timers meet the final absolute deadline.

The native FOKS importer remains a separate process because of SQLite linkage.
No celld SQLite/V8 upgrade, new backend decision, whole-state reconstruction,
wire-format change or production ownership/fencing claim is included in A2.

## Closed implementation manifest and patch boundary

A2 starts from the accepted A1 commit; record that commit and hashes before
editing. A1 is still being authored, so these are proposed path operations,
not preclaimed A1 postimage hashes. Any extra path requires a reviewed amendment.
The original upstream tree/inventory remains immutable.

Outer FOKS repository paths, all under `tools/foks-celld-guarded/`:

```text
Cargo.toml
Cargo.lock
runtime.lock.json
prepare-runtime.py
test_prepare_runtime.py
check_graph.py
test_check_graph.py
patches/series.json
patches/0001-guarded-physical-lifetimes.patch
src/main.rs
src/lifecycle.rs
tests/lifecycle.rs
```

The patch has exactly these celld paths:

```text
crates/celld/Cargo.toml
crates/celld/lib.rs
crates/celld/pool.rs
crates/celld/runtime.rs
crates/celld/js.rs
crates/celld/js/guarded.rs
crates/celld/guarded_execution.rs
crates/celld/guarded_execution/qualification.rs
```

`Cargo.toml` adds only `foks-guarded-qualification`; `lib.rs` keeps the serving
module private and reexports only diagnostic functions/types under that
feature. `pool.rs` owns the dedicated construction/unique consumption and turn
scope seam. `runtime.rs` adds the owned existing-driver wrapper, not a copied
runtime loop. `js.rs` declares its private child; `js/guarded.rs` owns closed
configuration, normal teardown and diagnostic backing probes. The private
execution module owns the ledger, control, supervisor and retained lifetimes;
its qualification child embeds the fixed program, bounded barriers/scenarios
and diagnostic observations.

The earlier outline listed `engine_api.rs` as a possible A2 path. This concrete
slice does not need a new public engine contract there; defer that path to A3's
actual request/output integration. Do not edit actor, storage, generation,
main-node startup or the ordinary pool policy in A2.

The outer manifest keeps default features empty and adds one `lifecycle`
feature forwarding to `celld/foks-guarded-qualification`. The diagnostic runtime
uses only a newly direct dependency on already-selected exact Tokio 1.53.1;
its identity-only path does not start that runtime. Any other
new dependency is a plan change. No V8/rusqlite/sys replacement or feature
upgrade is allowed. Preserve A1's isolated workspace, native-disabled core,
transport qualification-only features, compiler/target, native archive,
allocator and constrained build recipe. Review the exact nested lock and
selected-target graph before builds. Root workspace manifests/lock stay out of
scope.

Extend A1's currently empty-series preparer using a closed, bounded strict
unified-diff format: pinned series/patch hashes, exact allowed paths, complete
pre/postimage hashes and sizes/modes, exact hunk line positions/context, no
fuzz/search/offset adjustment, rename, copy, mode change, binary patch, symlink,
delete, duplicate path or path traversal. New Rust files are explicit additions.
Apply to a temporary verified tree, verify the complete resulting file roster
and hashes, then publish atomically. A malformed/partial patch never replaces
an accepted source tree. Preserve A1 identity-only mode as a historical
profile; the A2 patched build gets a distinct input/profile identity and may
not reuse an A1 receipt as if its bytes were unchanged.

`check_graph.py` distinguishes the exact identity feature set from the exact
`lifecycle` feature set, accepts only the latter's reviewed celld qualification
feature, retains all A1 selected-target and engine/FOKS-subclosure exclusions,
and binds the altered post-patch tree. `src/lifecycle.rs` owns the real A0 Request and one
finite Tokio runtime after V8 initialization. Only named `lifecycle` scenarios
are accepted. Its bounded report is diagnostic evidence, never readiness.
Outer tests launch this actual binary for scenarios needing fresh static or
quarantine state. No global `celld_internal_tests` simulated-runtime cfg is
allowed in the qualified binary.

## Deterministic validation matrix

Use bounded native barriers and explicit handshakes, not sleep races. Build
and source-hash gates precede execution. Diagnostic process timeouts contain
failed tests; they never mean runtime disposal succeeded.

| Scenario | Required oracle |
| --- | --- |
| Two admitted invocations paused before construction | A third is refused before Worker load or memory creation; ledger shows exactly two execution and 128 MiB reserved. |
| One or both invocations retiring, including delayed driver payload or blocked teardown | They still count. Repeated admissions do not allocate extra Workers or backings. |
| Same tenant during Building/Running/Retiring/output hold | Immediate Busy; a new invocation cannot reuse the old scope/epoch or overwrite its generation token. |
| Contended/poisoned ledger, invalid key, generation exhaustion | Prompt refusal; no queue and no partial reservation/instance. |
| Waiter cancelled before first poll, during construction and after reply | The complete pre-spawn payload stays owned by the supervisor; no early credit. Running blocking tasks are joined; completed handles/payloads drop before release. |
| Original deadline expires at each lifecycle barrier | No new admitted work/output; original Instant/Request identity unchanged. Cleanup can continue while charged. Report any overdue cleanup rather than giving it a fresh deadline. |
| Worker load ordinary error before/after isolate creation | Only audited normal partial-owner destruction can complete. Exact post-allocation uncertainty/panic cases quarantine. No generic error-based release. |
| Rust panic in build, guarded turn, native payload Drop and teardown boundary; caught task panic | Sticky taint remains after catch/JoinError mapping. Every occupied record is either normally complete or still reserved/quarantined; never reuse on unwind. Run destructive cases in separate diagnostic processes. |
| Delayed real driver or native backing probe | Unique Slot/normal teardown cannot be reported early. Probe keeps its memory reservation; final reference assertion/reset occurs only after actual isolate destruction. |
| Full normal teardown | Ordered events prove drivers/payloads/watchdogs gone, unique Slot consumption, Worker full-drop return, final probe reset if any, then memory release. A Drop-entry event is insufficient. |
| Runtime destroyed while output held, and output disposed while runtime held | First case frees only memory; second frees neither runtime memory nor execution prematurely. Execution returns only after both actual owners finish. |
| Altered module/hash/memory/import/export contract | Rejected before construction. Cover shared/imported/memory64/multiple/missing-max/oversized/extra-export cases. |
| Actual fixed64MiB module and zero/positive growth | Observe actual memory/backing identity. Zero growth introduces no extra live store; positive growth fails and does not change content/capacity. |
| Sequential fresh invocations of one and many fixture tenants | Wasm scratch starts fresh each time; real same-file SQL counter persists. No tenant retains an idle isolate. Fixed ledger/supervisor cardinality stays at two. |
| Repeated saturation/drain cycles and shutdown with pending work | No leaked successful credits/tasks/handles; uncertain paths remain visibly quarantined. Shutdown joins its jobs before claiming completion. |

The backing-probe test is intentionally stronger than observing RSS. After
normal Worker destruction the retained probe must be the sole native owner,
then its final reset must return normally before the same reservation is
released. The production path uses closed ownership and complete Drop, not
this potentially waiting diagnostic assertion.

Add patch/preparer negative tests for every excluded patch operation, wrong
pre/postimage and altered already-prepared tree. Extend graph tests for a
missing selected target, forbidden native FOKS edge, engine drift, alternate
V8 features and accidental simulated runtime. Confirm the A1 identity command
still reports the pinned real engines under the new profile.

Measure fixed-count cold-isolate cycles (initially 32 successful invocations,
plus the finite failure scenarios) in the already-initialized process. Record
create/adopt/instantiate/execute/close/drop durations and peak observed live
Workers, Wasm stores, reservations and native/RSS overhead. These are local
construction/retirement observations, not tenant-density or concurrent
host/AZ-recovery qualification. Do not multiply an average into a capacity
promise or claim a strict latency percentile from 32 samples.

## Review and acceptance order

1. Author-review this exact plan, then root and an independent agent review it.
   Record their hashes. Wait for A1 acceptance/commit and rebind this scope to
   that exact base; do not silently alter A1 while it is under review.
2. Before code, confirm actual selected V8 crate source hashes/contracts from
   A1 and freeze the API, patch roster and native owner drop order. Freeze the
   exact resulting closed fixture bytes with the implementation source for
   review, before any execution. Unresolved cleanup/escape findings block the
   relevant seam; neither a synthetic acknowledgment nor a refcount-poll workaround is an
   accepted substitute.
3. Implement the finite group and author-review all source, then obtain root
   and independent source review before Cargo, resolution or tests. Review
   the exact completed patch/preparer, source pre/postimages, reachable host
   operation inventory, original-owner capture and all panic/normal-Err exits.
4. Perform only the reviewed nested lock/graph update, review its exact result,
   and reverify A1's build/native prerequisites. Root retains the single
   coordinated Cargo slot; use A1's offline pinned assets, separate target,
   disk floor, limited jobs and owned process-group cleanup.
5. Run meaningful preparer/graph tests, source formatting, strict harness lint,
   real identity and finite lifecycle scenarios. Build/check the default
   identity binary (celld qualification feature off) and the `--features
   lifecycle` binary; the latter runs the finite scenarios. The same A1
   process-group/resource wrapper must own build, check/lint and test commands,
   each with explicit target, features and captured receipt. Preserve original
   failures and hash exact commands/logs/artifacts. Any fix receives author,
   root and independent source review before rerunning affected checks.
6. Review the complete source/evidence twice and commit only this implementation
   group. Keep Markdown unstaged. Report exactly which normal disposal paths
   qualified, which cases deliberately quarantine, and the measurements and
   limitations above. A3/A4 and actual hosted List integration remain open.

## Fork or stop conditions

If the closed reachable program retains a native backing store, Locker or
worker/driver owner that cannot be consumed with this API, do not release its
reservation. First remove that unsupported capability from the qualification
profile or keep the owner charged. If the required FOKS host later needs that
capability, propose a separately reviewed celld/Rust-only lifetime hook or a
process boundary, with exact pinned inputs. A Rusty V8 fork is not assumed by
this plan and cannot be added as an incidental fix.

If cleanup is safe but too slow for the requested recovery/latency targets,
report the actual timings. Keeping the reservation during overdue cleanup is
required even if it reduces available capacity to zero. Isolate reuse, multiple
tenants per isolate, relaxed failure behavior or an enlarged reservation is a
new design/qualification decision, not an automatic retry.

## Completed plan review

The author, root and independent reviews passed on the plan above (source SHA-256 `c17a25469caad41ff90fff5c4a747c036257021aa89a1584b2d2baeab925347e`). This supersedes its initial pending-review status. A1 acceptance and commit still gate implementation.

Implementation must preserve cleanup turns for already-owned drivers while Retiring refuses new jobs, install celld's actual async runtime handle once, and confirm the selected V8 shared allocator contract. The fixed 64-MiB fixture does not qualify the existing FOKS Wasm artifact's growth from 18 to 1,024 pages. The later real FOKS profile must either use a separately qualified fixed-memory artifact or prove that growth and old-backing overlap remain charged.

Receipts: `/private/tmp/foks-celld-a2-plan-author-review.json`, `/private/tmp/foks-celld-a2-plan-root-review.json`, and `/private/tmp/foks-celld-a2-plan-list-review.json`. No A2 source, build or test has run.


# A2 addendum: selected V8 backing-store lifetime and capacity

Status: author source review PASS; root and independent addendum review pending.
This clarifies the approved A2 plan at SHA-256
`c17a25469caad41ff90fff5c4a747c036257021aa89a1584b2d2baeab925347e`.
It changes no engine, allocator, feature, patch path or admission limit. A2
implementation still waits for A1 acceptance and commit.

The review used the actual selected registry crate `v8-152.1.0`, including its
vendored native sources, and the prepared celld 0.6.2 tree. The accompanying
`foks-celld-a2-selected-v8-review.json` records exact file hashes and source
locations. The crate's VCS metadata names commit
`8149304f6924411df09a1917cd6de0fc90fa7fec` and reports `dirty: true`; the verified
registry archive and file hashes, rather than an assertion of a pristine Git
checkout, bind this review. No candidate code, tests or build was executed.

## Accessible capacity and virtual address space

Each admitted fixture reserves 67,108,864 bytes of accessible Wasm backing
capacity: 1,024 initial pages and the same maximum, with 65,536 bytes per page.
Two occupied records reserve 134,217,728 accessible bytes. This accounting does
not bound virtual address reservations or process RSS.

The selected native implementation computes backing capacity from the module
maximum. When Wasm trap-handler guard regions are enabled on this 64-bit
target, a memory32 backing can reserve **8 GiB of virtual address space** even
though only 64 MiB is accessible. Without those guard regions, this fixed
memory's backing reservation is 64 MiB. These are conditional source contracts,
not measurements of the future A2 executable. The inspected celld initializer
does not enable the Wasm trap handler, and V8's initial state is disabled; A2
preserves that initialization. Any future initialization change reopens this
source assumption and the virtual-address capacity assessment. Do not claim
that this profile currently reserves 8 GiB per invocation merely because V8
supports that configuration.

Keep the report's accessible-capacity ledger separate from its documented
guard-region policy and any measured virtual-size/RSS observations. If virtual
mapping size is not measured, label it source-derived or unmeasured. No new V8
binding or flags are required. In particular, V8's native
`GetWasmMemoryReservationSizeInBytes` computes a guarded size when supported;
it is not a query of the current backing's actual mapping.

## Zero growth and probe identity

The fixed Wasm `memory.grow 0` instruction must return 1,024 pages. It uses
`GrowWasmMemoryInPlace`, whose zero-page branch does not allocate another store.
For a normal non-shared, non-resizable memory, V8 nevertheless detaches the old
JavaScript ArrayBuffer and clears its cached buffer. The test must reacquire
`memory.buffer` after growth. Reusing the old ArrayBuffer, its typed view, or
its post-detachment backing-store accessor is not a valid identity check.

Retain the original native `SharedRef<BackingStore>` as the charged probe.
Under the scoped Locker, obtain the current Wasm memory buffer after each
growth operation and compare its backing object/data identity with that
original probe. Require a non-shared, non-resizable store of exactly 64 MiB,
unchanged contents, a zero-length detached old ArrayBuffer after zero growth,
and no replacement backing. Drop each temporary comparison SharedRef before
the final sole-reference assertion. The fixed positive-grow instruction must
return `-1` at the declared maximum and preserve the same backing and contents.
This is the Wasm instruction result, not the JavaScript `Memory.grow` API's
exception convention. Only bounded scalar results leave the private probe.

## Final disposal and allocator overlap

The selected sources support the existing normal-teardown conjunction:
complete owned driver/Locker disposal, normal full `drop(worker)` return, and
then the charged diagnostic probe's final normal reset. `SharedIsolateInner`
performs native isolate disposal; V8 cancels and waits for isolate tasks,
releases managed backing owners, and joins/releases the ArrayBuffer sweeper.
The probe's `assert_use_count_eq(1)` remains diagnostic only, after those joins.
Its relaxed-load retry loop is not a production completion or synchronization
mechanism. Failure or unwind keeps the record quarantined as already planned.

There is no dangling per-isolate allocator dependency in this closed path.
Celld uses default CreateParams; Rusty V8 installs the default allocator through
the shared-allocator field, and ordinary ArrayBuffer backings retain it when
needed. Wasm backings instead use V8's process platform page allocator. Keep
the initialized V8 platform alive through every runtime and final probe drop;
do not add process-platform disposal to the per-invocation retirement path.

The final Wasm backing destructor unregisters any guard range and calls the
page allocator's `FreePages`. On the selected ordinary POSIX platform this
reaches checked `munmap`; failed native freeing is fatal rather than a successful
capacity release. Normal return establishes completion of that native free
contract. It does not establish a particular immediate RSS decrease, which
must remain an independent observation.

The global backing registry can temporarily retain registered stores while
another isolate is purged. Its sole registration call in these sources is
serialization of **shared** Wasm memory; non-shared Wasm serialization is
rejected first. The immutable non-shared/no-structured-clone A2 program never
registers its backing there. Keep that exclusion in the no-escape audit. A
future shared-memory or serialization capability requires renewed alias and
disposal review; a transient reference-count observation would not resolve it.

The existing lifecycle matrix remains required. Strengthen its growth oracle
with the buffer-reacquisition checks above; retain the final-probe hold barrier
and the requirement that memory credit returns only after its final drop.
Neither this source review nor eventual RSS observations replace those actual
qualified lifecycle executions.

Addendum review status: author, root and independent review PASS for the exact addendum SHA-256 `ce33b1922a98cccc7972216c62b81a3feda0cff4ee446055d211731de0bc7c38`. The selected registry archive and all 29 cited source hashes were checked independently. A2 implementation still requires accepted A1 qualification and commit. This Markdown remains unstaged.


# A2 build-budget reuse amendment

Status: direct root and independent review PASS. No A1 source,
input, result or threshold changes. A2 still starts only after accepted A1 commit.

The reviewed build policy reserves a 4-GiB logical target allowance plus a 2-GiB
free-space reserve. A1 requires all 6 GiB free on every initial build, even when
its dedicated target already holds verified compiler outputs. Repeated A2
identity/lifecycle build and lint runs should reuse those outputs instead of
requiring their deletion and recompilation.

For A2 only, replace that initial threshold with the remaining working allowance:

```
required_free = free_floor + max(0, maximum_target - observed_target_bytes)
```

The constants remain free_floor=2 GiB and maximum_target=4 GiB. An empty target
therefore still requires 6 GiB free. Reject an already oversized target before
using the formula. During commands retain the existing absolute 2-GiB free floor
and 4-GiB target ceiling. Check all values from the one resource sample; never
count source/assets, another target, proposed cleanup, or evidence as reusable
build capacity. Do not delete any outputs automatically.

This is an operational admission check, not a hard filesystem quota or an atomic
snapshot. It does not reserve space against other processes, and logical target
size is distinct from allocated blocks. Actual free-space monitoring still wins
if external disk use or allocation overhead consumes the reserve. Neither an
unknown scan nor an observation error supplies reusable bytes. Existing strict
symlink/error handling and sole-Cargo/process-group ownership remain unchanged.

Keep A1's historical recipe intact in its commit and evidence. A2 must use a new
profile/input identity and encode this admission policy explicitly in its input
record; an A1 receipt cannot qualify it. The patch is limited to the already
approved A2 runtime.lock.json, prepare-runtime.py and test_prepare_runtime.py
paths. No runtime or production capacity budget is changed.

Deterministic source-reviewed tests cover empty/partial/full targets, one-byte
threshold boundaries, oversized target refusal even with ample free space,
unknown/unreadable observation, and unchanged running floor/ceiling. Actual
commands remain owned, locked/offline, jobs=2, time-bounded, and input-bound.

Direct root review: PASS. The equation preserves the same total working allowance
and reserve; existing target bytes reduce only remaining growth allowance. No
reclamation or future disk availability is assumed. The live-monitor limitations
are explicit. A1 behavior is unchanged. Independent plan review is required
before implementing this A2-only policy.

Independent plan review: `/private/tmp/foks-celld-a2-build-budget-amendment-list-review.json`, SHA-256 `fa96f37b35f08c7eaa5821febc89e94d896df30c90aa912761d07dc943ad8d5b`. The original reviewed amendment hash is `ed4f950981e9c53e3fefcbd7ab011c2557b6e85801f84ab45c68aac75ce20c6a`; this repository copy only updates review status.

A1 prerequisite accepted: implementation commit `8e31944080e03651caaa3512169fe468f8375342`. Actual locked build, strict harness lint and real native identity probe passed after both source-review rounds; independent evidence review passed. A2 implementation will use this exact base. The source/API and strict patch/tooling freeze is being completed before edits; no A2 qualification is claimed.

## Accepted implementation contract (A2)

The following contract passed direct and independent plan review before source
edits. The final API review closed the process/report accounting finding with
a fourth distinct actual A0 Request retained until diagnostic process exit.
Invocation owners remain independently observable. Implementation is in progress;
no A2 runtime result is claimed yet. This document remains unstaged.

### Tooling and patch format

Frozen source SHA-256: `3ac2be0b55ddb35e5ed8bcf00dfd4bf8351d3768914a4dd354da368d387b269b`.

# A2 tooling and patch-format freeze

Base: accepted A1 commit `8e31944080e03651caaa3512169fe468f8375342`.
This refines the approved A2 plan and build-budget amendment; implementation and
execution wait for direct and independent review of this freeze and the API
freeze. No additional implementation path or dependency is authorized.

## Input and graph profiles

Retain historical A1 format/profile support and its empty format-1 patch series.
The new input profile is `a2-physical-lifetimes`, format 2. It retains every A1
engine/compiler/archive/SDK/environment/allocator pin and adds one `qualification`
object: accepted A1 base commit, fixed program JS/Wasm SHA-256/length, and the
reviewed two-slot/64-MiB-per-slot constants. The build record explicitly names
`remaining-target-allowance` admission; the approved formula applies only to A2.
The ordinary A1 thresholds and behavior remain unchanged.

Graph collection/verification accepts an explicit mode, `identity` or `lifecycle`
(default identity). Identity uses no default features and requires no harness or
celld qualification features. Lifecycle additionally selects exactly the outer
`lifecycle` feature and `celld/foks-guarded-qualification`; it is forbidden under
A1. Keep all previous exact engine, selected-target, sole-member and native-free
FOKS closure checks. Bind mode in evidence and report; evidence for one mode must
not qualify the other. The only newly direct registry dependency is optional
Tokio `=1.53.1`, already selected by celld. Do not resolve or upgrade dependencies
before source review. The exact completed lock and both real graphs require
review before builds.

Add the approved outer lifecycle source/test and patch files to local source
inventory. Bind the full prepared post-patch tree. The immutable upstream Git
inventory and original Cargo.lock remain unchanged; selected upstream dependency
edges are still checked against their original lock targets.

## Exact patch envelope

Use `patches/series.json` format 2 with exactly these keys:
`format`, `profile`, `base_commit`, `patches`. Profile and base must match the new
input record. Exactly one patch is admitted, with keys `path`, `bytes`, `sha256`,
`files`; its path is `patches/0001-guarded-physical-lifetimes.patch`. Maximum patch
size is 1 MiB. Each file entry has exact keys `path`, `mode`, `before`, `after`;
mode is `100644`. Before is null for an explicit addition, otherwise an exact
object `{bytes, sha256, git_sha1}`. After has that same exact object shape and is
at most 512 KiB. Total postimage bytes of changed files are at most 2 MiB.

Require exactly the eight approved paths, with no duplicates:

```
crates/celld/Cargo.toml
crates/celld/lib.rs
crates/celld/pool.rs
crates/celld/runtime.rs
crates/celld/js.rs
crates/celld/js/guarded.rs
crates/celld/guarded_execution.rs
crates/celld/guarded_execution/qualification.rs
```

Only the last three are additions. Existing files must retain their original
mode and match both Git SHA-1 and complete SHA-256/size before modification.
New parent directories are derived only from those added paths. All unmodified
upstream entries remain exact. No broader path prefix grants patch authority.

Patch bytes are UTF-8 with LF endings and terminal newline, ordinary unified
diffs only: `--- a/<path>` / `+++ b/<path>`, or `--- /dev/null` for additions,
followed by explicit `@@ -start,count +start,count @@` hunks and space/plus/minus
lines. No timestamps, Git headers, extra hunk annotations, binary/mode/rename/copy/
delete operations, CR, missing-newline markers, duplicate files, path traversal
or unused trailing content. A file diff follows the series file order. Exact
old/new hunk offsets and line counts must hold, context/removals must match, and
hunks cannot overlap or move backward. Never search for context, fuzz, shift or
invoke external patch tools. Preserve unchanged gaps/tails and verify the exact
postimage byte count and both hashes before writing it to private staging.

Validate the official archive in private staging first. Apply the complete patch
there, derive and verify the full postimage roster/hash/mode inventory, then
publish the new source tree and its bound manifest with the existing fresh-tree
protocol. Any malformed or partial patch must leave an existing accepted source
tree unchanged; mismatched existing source fails rather than being auto-upgraded.
Missing or partial publication remains unusable. Source verification checks the
full pinned postimage, not merely a caller-written success marker.

Before the first A2 preparation, root preserves the accepted A1 executable and
receipts, then retires only its verified generated source/asset preparation
copies under sole build ownership. The original archive/native asset inputs and
all evidence remain. A2 prepares fresh from the same local archives and reviewed
patch; neither a changed manifest nor manual edits relabel A1 as A2. The existing
full-input asset receipt binding stays in force. Input changes require explicit
fresh preparation and evidence, with no automatic receipt migration.

## Commands and result binding

Extend the existing owned build operation with explicit graph mode. Both modes
use locked/offline jobs=2, pinned native inputs, network denial, resource/timeout
checks and full child cleanup. Build and strict harness lint use that same mode.
Run the real identity probe for each compiled mode; under the new input profile
it reports `a2-library-identity-only`, never reuses A1's claim, and still does not
create an isolate. The lifecycle mode additionally runs the actual Rust integration
test target `lifecycle` serially with captured scenario reports (`--nocapture`).
Those tests launch closed child scenarios; they may not detach from the owned
process group. A scenario timeout or abnormal exit is a failure, not disposal.
No duplicate direct execution of the same scenario matrix is needed.

The outer lifecycle CLI and reports will follow the separately reviewed API
freeze. The report binds the actual program hashes and native original Request,
with bounded scalar/event output and the explicit physical-memory/RSS limits.
All original failures remain evidence. Do not overwrite a previous build/test
receipt, and do not claim A3 ownership, S3 durability, full FOKS serving or 15-second
recovery from A2.

## Focused validation and review

Add deterministic negative patch tests for excluded operations, wrong modes/path/
roster/hash/length, inaccurate/overlapping hunk positions/counts/context, malformed
UTF-8/newlines, additions and partial failure/publication. Verify unchanged source
reuse and refusal of altered prepared trees. Add mode/profile/feature mismatch
and actual-input binding tests plus the reviewed resource boundary/error matrix.
Preserve prior 39 tests and their A1 semantics. Source reviews precede tests and
Cargo; exact lock/graph changes are reviewed separately. Default identity and
lifecycle builds both must pass, followed by direct and independent result review.

Direct root plan review: PASS. The format is intentionally closed and finite,
with complete pre/postimage verification and no generic patch authority. Existing
A1 artifacts and claims remain historical; source, mode and input identity bind
every A2 result. API freeze and independent review remain required before edits.

### Exact input spelling

Frozen source SHA-256: `ffcf8094c83ef2d9ab5e0bf8d82c6f4f0c92f56d9ee506e7c74cbd46d6301e91`.

# A2 tooling spelling addendum

This fixes the exact field spelling in the accepted tooling freeze at SHA-256
`3ac2be0b55ddb35e5ed8bcf00dfd4bf8351d3768914a4dd354da368d387b269b`.
No semantics, path, engine or dependency is added.

The new `qualification` object has exactly these keys and values:

```json
{
  "base_commit": "8e31944080e03651caaa3512169fe468f8375342",
  "program": {
    "js": {"bytes": "positive integer, at most 524288", "sha256": "64 lowercase hex characters"},
    "wasm": {"bytes": "positive integer, at most 524288", "sha256": "64 lowercase hex characters"}
  },
  "execution_slots": 2,
  "wasm_bytes_per_slot": 67108864
}
```

The quoted descriptions for program bytes/hash are validation requirements,
not literal values. Actual reviewed fixture hashes/lengths replace them before
source freeze and execution. Reject bools/non-integers for byte counts.

The A2 build record uses `admission: "remaining-target-allowance"`. Historical
A1 has no such field and retains its original policy. The outer lifecycle
feature uses `dep:tokio` explicitly, so its complete active feature set is exactly
`["lifecycle"]`; Tokio is optional `=1.53.1`, with existing selected runtime
features only.

`verify <archive>` continues to validate only the original upstream archive.
`prepare <archive>` validates that archive, applies the pinned patch in private
staging and publishes the postimage. The internal prepared-source verifier
checks the full patched tree and bound receipt. Diagnostics and CLI help must
make this distinction clear; archive validation alone is never readiness or
patched-source qualification.

Root direct review: PASS. This resolves names and archive/postimage terminology
inside the already reviewed design; independent confirmation precedes edits.

### API and ownership

Frozen source SHA-256: `d5f06998a53e57e5be26defe272e8fd4af3b25303eecbac35accc9525dec30bb`.

# A2 closed lifecycle API and ownership freeze

Base: accepted A1 `8e31944080e03651caaa3512169fe468f8375342`.
This freezes implementation interfaces for the already approved A2 plan
`c17a25469caad41ff90fff5c4a747c036257021aa89a1584b2d2baeab925347e`,
selected-V8 addendum `ce33b1922a98cccc7972216c62b81a3feda0cff4ee446055d211731de0bc7c38`,
and A2 build-budget amendment. It adds no engine, dependency, patch path or
serving capability. Root and independent approval of this text and the tooling
freeze precede source edits. The source/fixture freeze and two source reviews
still precede execution.

## Public diagnostic seam

Under celld feature `foks-guarded-qualification` only, `lib.rs` reexports the
following names from the private qualification module:

```rust
pub enum LifecycleScenario { /* closed variants below */ }
impl LifecycleScenario {
    pub const ALL: &'static [Self];
    pub fn from_name(name: &str) -> Option<Self>;
    pub const fn name(self) -> &'static str;
}

pub struct LifecycleProcessOwner { /* private opaque Arc; not Clone */ }
impl LifecycleProcessOwner {
    pub fn new(owner: std::sync::Arc<dyn std::any::Any + Send + Sync>) -> Self;
}

pub struct LifecycleInput { /* private fields; not Clone */ }
impl LifecycleInput {
    pub fn new(
        owner: std::sync::Arc<dyn std::any::Any + Send + Sync>,
        deadline: std::time::Instant,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self;
}

pub struct LifecycleLayouts {
    pub process_control_bytes: usize,
    pub report_retained_bytes: usize,
    pub serialization_bytes: usize,
    pub invocation_control_bytes: usize,
    pub largest_task_payload_bytes: usize,
    pub diagnostic_output_bytes: usize,
    pub maximum_alignment: usize,
}
pub fn lifecycle_layouts() -> LifecycleLayouts;

pub struct LifecycleProgram {
    pub js_sha256: [u8; 32],
    pub js_bytes: usize,
    pub wasm_sha256: [u8; 32],
    pub wasm_bytes: usize,
}
// Copy/Clone; fixed diagnostic values, never executable inputs.
pub struct LifecycleReport { /* private, bounded scalar/event fields */ }
impl LifecycleReport {
    pub fn program(&self) -> LifecycleProgram;
}
// Implements serde::Serialize. No runtime or lifetime-owner getter.
pub enum LifecycleFailure {
    Setup, Admission, Fixture, Driver, Retirement, Observation, ReportBound,
    OwnerBound,
}
// Implements Debug, Display (fixed messages) and std::error::Error.
pub async fn qualify_lifecycle(
    scenario: LifecycleScenario,
    process_owner: LifecycleProcessOwner,
    inputs: [LifecycleInput; 3],
) -> Result<LifecycleReport, LifecycleFailure>;
```

`LifecycleScenario` is Copy/Clone/Debug/Eq and serializes to the exact CLI name.
`LifecycleProcessOwner` and `LifecycleInput` construction allocate no new runtime,
grant no readiness and accept no callback. The input's cancellation Arc is the
same original shared state, not an ID or JSON token. The input owns its original
absolute deadline unchanged;
serial fixture cycles may explicitly reuse an input, as allowed by A2. The
ledger retains the same opaque owner allocation and validates Arc identity at
its transitions; it never calls methods through `Any` or creates a replacement
FOKS request. `LifecycleProcessOwner` is a separate fourth, real A0 Request
allocation, not a clone of any of the three invocation Requests. It has no
invocation deadline/cancellation semantics and cannot admit an invocation.
Layouts are observations/admission terms, never disposal proofs.

The layout function uses the actual compiled private types: fixed ledger and
its shared allocation, invocation control/cancellation shared allocations,
ordered build/driver/retirement payloads and bounded diagnostic output. Report
both typed payload sizes and the conservative retained charge separately in
source assertions. Shared allocation terms include their two Arc counters and
padding; layout arithmetic is checked. Tokio task headers, V8 allocations,
allocator overhead, stacks, SQL caches and process telemetry are explicitly
outside this typed-control charge. `process_control_bytes` covers the fixed
installed ledger/shared control allocation. `report_retained_bytes` covers the
native bounded report/event/cycle storage at its maximum simultaneous live
extent; `serialization_bytes` is exactly 65,536 bytes for the fixed-capacity
JSON-plus-LF buffer. The native report remains live while that buffer is built,
so these are additive process terms, not alternative peaks. The outer envelope
uses borrowed fields and a borrowed LifecycleReport, avoiding a second owned
JSON tree or copy of the native events. Source assertions bind each reported
term to the actual representation and capacity; no post hoc charge is allowed.

Before constructing the installed ledger or report, the outer harness reserves
`process_control_bytes + report_retained_bytes + serialization_bytes` using
checked arithmetic on the separate real process Request through the existing
retained-owner trait. It reserves each invocation's reported fixed control/task/
output terms on that invocation's Request. The reported largest task payload is
an observation already included in the appropriate conservative retained term,
not an accidental second charge. A refusal ends the diagnostic before Worker
construction. The Request Arc's own allocation is already preadmitted by A0.
This is not a proof of every celld allocation or production frontend admission.

Successful installation stores the consumed opaque process owner in the
process-static guarded inventory, separately from its two invocation records.
It remains explicitly charged until diagnostic process exit, including after
qualify_lifecycle returns, while the returned report is serialized, and after
normal invocation retirement. No helper removes or resets this installation or
returns its process charge. Second installation still refuses. The static
owner covers both the ledger's physical lifetime and report/serialization
overlap; it is deliberately conservative when those buffers are later freed.
An installation refusal cannot report successful process ownership. The
unchanged upstream asyncrt PROCESS_DOMAIN, Tokio runtime metadata, immutable
input parsing and V8 platform singleton are explicitly excluded process
overhead, not allocations silently attributed to a dead invocation Request.

No public type exposes Worker, Slot, Residency, Affiliation, Locker, Global,
IsolateHandle, BackingStore, raw pointer, arbitrary program/config/SQL, tenant
path, lifecycle hook, ready tenant, or caller-minted completion. No arbitrary
scenario parameters or scenario callback are accepted.

## Exact scenarios and CLI

The outer command is exactly `foks-celld-guarded lifecycle <name>`, enabled only
by the outer `lifecycle` feature. Extra arguments and unknown names fail before
V8/runtime initialization. The existing `identity` command remains separate.
Each integration test launches the actual binary in a fresh process; children
inherit the already owned build/test process group and never create a session
or detach. The closed variant/name inventory is:

| Variant | CLI name | Finite oracle |
| --- | --- | --- |
| Normal | `normal` | Real load/adopt/driver/output/close/full drop and zero final occupancy. |
| Admission | `admission` | Two paused pre-build admissions reserve 128 MiB; third refuses before Worker construction; second installation refuses. |
| AdmissionRefusals | `admission-refusals` | Held lock contention, invalid fixed identity, generation exhaustion, and final poisoned-ledger refusal; no partial admission. |
| TenantBusy | `tenant-busy` | Same tenant refuses during Building, Running, Retiring and output-only occupancy. |
| CancelBeforePoll | `cancel-before-poll` | Supervisor retains complete pre-spawn payload after waiter loss before first poll. |
| CancelDuringBuild | `cancel-during-build` | Cancellation during a barrier-held real blocking build does not return credit; join and payload disposal precede completion. |
| CancelAfterReply | `cancel-after-reply` | Reply receipt/waiter loss does not imply real driver completion. |
| DeadlinePreAdmit | `deadline-pre-admit` | Already-expired original Instant refuses before construction. |
| DeadlineBuild | `deadline-build` | Original deadline expires at an acknowledged construction barrier; cleanup remains charged. |
| DeadlineRun | `deadline-run` | Original deadline expires at an acknowledged driver barrier; no later publication. |
| DeadlineRetire | `deadline-retire` | Expiry during teardown cannot grant credit or replace the deadline. |
| DeadlineOutput | `deadline-output` | Expiry while output is retained prevents publication; actual disposal is still required. |
| LoadErrorBeforeIsolate | `load-error-before-isolate` | The separately audited normal partial-construction branch drops all constructed payload before release. |
| LoadErrorAfterIsolate | `load-error-after-isolate` | Actual post-isolate normal constructor error exercises full partial-owner destruction; no generic Err-based release. |
| PanicBuild | `panic-build` | Build unwind leaves the exact generation reserved/quarantined. |
| PanicTurn | `panic-turn` | Turn unwind restores CURRENT_SLOT and remains tainted. |
| PanicPayloadDrop | `panic-payload-drop` | Native payload destruction panic cannot grant completion. |
| PanicTeardown | `panic-teardown` | Teardown unwind remains quarantined even if caught. |
| PanicTask | `panic-task` | Caught task panic/JoinError preserves sticky taint before result conversion. |
| DelayedDriver | `delayed-driver` | Real driver payload/JoinHandle barrier prevents unique Slot completion and credit. |
| HeldBacking | `held-backing` | Full Worker drop precedes final charged probe; memory remains reserved until sole-reference assertion and normal final reset. |
| RuntimeBeforeOutput | `runtime-before-output` | Runtime/backing destruction releases memory only; output still holds execution/tenant Busy. |
| OutputBeforeRuntime | `output-before-runtime` | Output disposal alone releases neither runtime memory nor execution. |
| FixtureRejections | `fixture-rejections` | Internal finite malformed/hash/import/shared/memory64/multiple/missing-max/oversized/export cases refuse before construction. |
| MemoryGrowth | `memory-growth` | Actual 64-MiB nonshared store; zero growth detaches old buffer, reacquired buffer has original backing; positive growth fails unchanged. |
| ColdCycles | `cold-cycles` | Exactly 32 normal invocations, first 16 one tenant and next 16 distinct fixture tenants; persistent same-file SQL, fresh Wasm, fixed two-record ledger. |
| SaturationDrain | `saturation-drain` | Four fixed two-admit/third-refuse/drain rounds, no successful-path leaks. |
| Shutdown | `shutdown` | Pending owned build/driver work is joined/disposed before normal shutdown completion. |

All barriers are native bounded handshakes, not sleep races. Original-deadline
scenarios wait on the supplied Instant only after acknowledging the target
barrier; they never substitute a fresh deadline. Failure to reach that barrier
before expiry fails that scenario. Ordinary scenarios use one finite outer
absolute deadline; the integration child timeout is a separate containment
limit, never evidence of disposal. Expected panic/quarantine scenarios return a
successful diagnostic report only after observing their closed expected state;
an unrelated invariant error fails the child. Poisoned/quarantined capacity is
never reset in-process or reused by a subsequent scenario.

## Closed report and outer caller

The report schema is format 1 with claim `a2-physical-lifetimes-local-fixture`,
the exact scenario, fixture JS/Wasm SHA-256 and byte lengths, two-slot/64-MiB
constants, layout terms, original-owner/deadline identity observations,
refusal/completion counts, two final record snapshots, peak live Worker/store
and reservation counts, ordered lifecycle events and at most 32 cycle timings.
Enums and fixed numeric fields replace unbounded native error strings. Events
are capped at 512; each event contains only sequence, record index, invocation
generation, closed event kind and scalar occupancy/reservation observations.
No report field contains a runtime pointer, owner, arbitrary path or SQL text.
Overflow of any report/event counter is an observation failure, not truncation.

Cycle timings name create, adopt, instantiate, execute, close and full-drop
stages. Native RSS samples use existing `crate::memory::resident_bytes()` and
report a zero/unavailable result as unavailable, never as zero usage. The peak
is explicitly a peak of finite samples, not an OS continuous high-water mark.
Accessible Wasm capacity, sampled RSS and unmeasured virtual mappings remain
separate fields/claims. Record the source-derived unchanged trap-handler policy;
do not report an actual 8-GiB mapping or promise an immediate RSS reduction.

Outer `src/lifecycle.rs` exposes exactly
`pub(crate) fn run(scenario: celld::LifecycleScenario, input: &str) -> Result<(), Box<dyn std::error::Error>>`.
Main validates the closed command and calls init_v8 once before this function;
run owns the finite Tokio runtime, native host setup, real Requests, diagnostic
call and serialization. It supplies four distinct real A0
`Pools::admitted_request` allocations: one process Request and three invocation
Requests. It precharges their respective terms through `ListRetainedOwner`,
verifies actual Request scope identity, and moves those same Arcs into the
separate process input and three invocation inputs. Keep only Weak<Request>
witnesses outside the call so final release checks are meaningful. Assert all
four owner identities are distinct before handoff. The static process owner
must remain alive on both normal and quarantine success; report this separately
as `process_owner_retained_until_exit`, without claiming that charge was freed.
The three invocation Weak checks never count that fourth owner. The third
refused input and temporary input-owner references must also be physically
dropped by return. The outer report adds input-profile hash,
actual shared-request evidence and expected Weak liveness after the call. It
compares the returned actual fixture hashes/lengths to the exact
`qualification.program.{js,wasm}` entries in the pinned runtime input using the
fixed Copy `program()` observation, without allocating a second native report
or serializing/parsing it into an owned JSON tree. Any mismatch fails before
emitting a successful report. Weak liveness expectations:
normal invocation records leave no retained invocation Request; quarantined
records retain the corresponding Request. Source review must bind the exact scenario-to-owner
expectation, not merely assert all weak references have the same result.

Serialize through a precharged fixed-capacity bounded writer with a
65,535-byte JSON limit plus one LF, so the entire stdout report is at most
64 KiB. It cannot grow beyond its 65,536-byte reservation. Serialization
failure produces no successful report and does not release the installed
process owner. Drop the native report and serialized buffer normally after
write completion/error; the process ledger/report accounting owner still
remains intentionally retained until exit. Native report disposal is distinct
from the per-invocation diagnostic-output completion already tested inside
the runner. The parent integration tests drain stdout/stderr while
owning the child, bound retained diagnostic output, and terminate/reap the child
on test containment failure. The parent remains in the owning process group;
root's process-group cleanup still contains any uncertain descendants.

## Host initialization and local SQL fixture

Initialization order is fixed: validate CLI; `celld::runtime::init_v8()` on the
main thread before creating worker threads; build one finite Tokio runtime
(two async worker threads and at most two blocking workers); then call existing
`celld::asyncrt::set_host_handle(runtime.handle().clone())` exactly once, before
Worker/native operations. The process domain retains a Handle, not ownership
proof that jobs have drained. Install the single guarded ledger once after
fixed native-owner admission, consuming the separate precharged process owner.
The process domain's retained Handle is excluded fixed upstream overhead; it
is not falsely charged to an invocation that has finished. The identity
command starts no Tokio runtime.

The runner creates one private disposable fixture directory, derives exact
paths itself, and retains directory ownership until all normal work closes.
The class is fixed `FoksGuardedFixture`; tenant IDs are canonical 64 lowercase
hexadecimal characters derived from the closed scenario's small integer
roster. Scope is exactly `FoksGuardedFixture:<id>`, epoch is 1, VFS is None and
replicated_wake is false. Repeated invocations for an ID reopen the same file;
distinct fixture IDs have distinct files. No user path or ownership epoch is
accepted. On uncertain disposal, keep the directory owner in the quarantined
process inventory rather than unlink a possibly live SQLite file. It may be
reclaimed only as diagnostic process cleanup, not reported normal retirement.

`js::guarded` constructs the private known WorkerConfig with exactly this class,
fixed script/node names, empty bindings/modules/services/loaders/queues/workflows/
containers/crons, default Compat (including sqlite_vec=false), denied egress,
no loader environment, no tail reporting and no arbitrary resource options.
The hash-bound JS contains the approved Wasm bytes and only synchronous local
SQL plus fixed Wasm operations and a bounded nonstreaming byte response.
Normal backing/cold-cycle measurements touch the entire existing 64-MiB buffer
with one private Uint8Array fill; touching only one byte per 64-KiB Wasm page
is not a measurement of fully faulted accessible memory. The view allocates no
second backing and remains private. Reacquire memory.buffer after grow_zero as
required by the selected-V8 addendum. One native scoped inspection reaches that
exact private memory for diagnostics; no Global or native reference escapes
except the explicitly charged probe.

Use actual `CellStorage` through `Residency::adopt`/`Worker::own_cell`, not
`storage::install_for_test`. Fixed SQL creates a one-row counter, increments
and reads it; response observations carry the actual write/observed positions.
The real `drive_cell_inner` Fetch response path (`js.rs` Answer::Fetch) samples
those positions and returns a bounded response. It does not itself synthesize
a replication gate acknowledgment. The closed nonstreaming/no-egress/no-alarm
fixture does not install `GATE_TX` or another fake host service. Successful local
reopen/counter read is the persistence oracle; no S3/durable-output assertion is
made. A3 must connect actual canonical output gating and storage sync.

After normal scenario completion, all owned jobs/payloads and outputs are gone,
local storage is closed, and the fixture directory may be removed; only then
return the report and drop the finite Tokio runtime normally. Keep process V8
platform/default allocator alive throughout every Worker and final probe drop;
there is no per-invocation or scenario V8::dispose/shutdown_platform. A
quarantine report describes retained records and owners; it is returned only
after the scenario has actually joined its jobs and disposed their completed
handles, even when a runtime/payload destructor did not prove physical release.
Remaining uncertain runtime/directory/native owners stay quarantined and the
finite Tokio runtime may then drop after those job joins. If joining cannot be
established, no successful quarantine report is produced: the containing child
times out/fails. Process exit contains retained uncertain objects but does not
claim their in-process disposal, capacity return, or safe whole-node shutdown.

## Private implementation boundaries and physical order

The eight approved upstream paths remain the complete patch. `js.rs` declares
its private guarded child. `pool.rs` adds only the sealed dedicated Slot path,
checked one-cell residency reservation/unique consumption, and feature-gated
unwind-safe CURRENT_SLOT restoration. Ordinary pool admission is unchanged.
`runtime::drive_guarded_cell` is a private wrapper calling actual
`drive_cell_inner` with no alarm reporter/trace parent and the finite remaining
budget sampled from the original deadline. It returns the real driver result;
A2 does not claim the existing budget is a complete absolute SQL deadline.
The guarded runtime owns and joins this future, never detached fetch dispatch.

`GuardedCapacity`, `GuardedKey`, `InvocationControl`, `GuardedAdmission`,
`OwnedGuardedRuntime`, `GuardedDiagnosticOutput` and completion markers remain
private. Exact fixed key, scope, epoch and nonwrapping generation are retained
from admission. A state transition validates that same record/generation.
`try_admit` uses try_lock, installs both reservations atomically and refuses
rather than queues. Poison, contention and uncertainty never grant capacity.

Every spawned job is one ordered payload aggregate before spawn/first poll:
its Worker/Slot/cell/job/native buffers come first; retained original control
and execution identity come last. The supervisor owns actual task handles and
completed results until their physical drop. A lost waiter cannot own or
cancel the supervision obligation. Taint is established before potentially
unwinding build/turn/future/teardown operations; only an audited normal return
can clear a temporary in-progress guard. A sticky failure cannot be cleared.
Lease Drop does not grant either completion edge.

Normal retirement retains the reviewed sequence: mark Retiring; join build and
real driver, drop completed handles/results and InFlight/ops/Globals/watchdog
scopes; close exact storage with own_cell(None), preserving errors; drop
Residency/Affiliation and all temporary Slot references; consume the dedicated
Arc once with try_unwrap and validate zero counters; move Worker out and drop
other Slot payload; explicitly drop Worker and wait for full normal return;
then assert/drop the charged final backing probe if present; only then release
memory. Output buffer disposal and all remaining payloads must also complete.
Take and drop the final record-retained original invocation owner outside the
ledger lock while still Retiring; only normal return and the complete
conjunction permit Free. The distinct process accounting owner is neither
removed nor credited by this transition.
No reference-count retry is used for Slot consumption. V8 probe count assertion
is diagnostic only after joined teardown, followed by normal final probe drop.

A native panic payload used by the closed diagnostic is separately armed so an
already-unwinding path does not cause an accidental double-panic test abort.
It does not turn caught panics into completion. If a scenario actually aborts,
the integration test records failure rather than a successful quarantine
observation. No runtime callback or fault hook is exposed publicly.

## Ownership split and review gate

The integration author owns only the eight upstream paths in a separate copied
author tree, plus outer `src/lifecycle.rs` and `tests/lifecycle.rs`. Root owns
strict patch serialization/preparation, series/pins, Cargo/lock, Python tests,
main dispatch and input graph. Do not edit accepted runtime/source or A1 input/
evidence in place. Source review freezes actual JS/Wasm bytes and hashes, exact
layout formulas, fault inventory and this scenario matrix before execution.
The only new direct outer dependency remains optional exact Tokio 1.53.1.
The reviewed API clarification adds one separate opaque process-owner input
and two scalar layout terms to close the ledger/report accounting lifetime;
it does not add an implementation path, dependency or serving capability.
The process owner is intentionally retained until exit. Normal invocation
Weak witnesses remain independent. Host-domain setup, local response gate
distinction and quarantined-directory retention remain details of the approved
closed local fixture, not new serving or durability claims.

Author plan review: PASS, including the separate process-owner amendment.
The previously frozen e0fa3aed revision and its receipt are preserved as
api-freeze-b.md and api-freeze-b-author-review.json. Read against actual celld
worker, Slot, driver, asyncrt, local storage/response and telemetry sources plus the selected-V8
review. Public API is closed; native ownership remains joined and no synthetic
replication/disposal acknowledgment is added. Source correctness and actual
lifecycle results remain unqualified until the required later reviews/tests.

### Final plan review receipts

- `/private/tmp/foks-celld-a2-tooling-freeze-root-review.json`: `23bfdf818e1926ddcba8157c3c798d6de376bcc1c70a88df6bc0fd881fabcba0`.
- `/private/tmp/foks-celld-a2-tooling-freeze-list-review.json`: `8025bde8ea30d9fe5a82c7c96e02ab97ed1445a59261787e693d51063dc5eb5f`.
- `/private/tmp/foks-celld-a2-tooling-spelling-root-review.json`: `cfd8a46e3f71e79fa8115f67d6a9f415bd62fee429142b5c4fa5d577780a13ac`.
- `/private/tmp/foks-celld-a2-tooling-spelling-list-review.json`: `e6abfde6e42ca3a6bd606d9e04a349a0467f16c81359729774149aae035ca161`.
- `/private/tmp/foks-celld-a2-api-freeze-root-review.json`: `e8d29e308775ccb75415246f493f735004542eee0fe990083f45a59fe8dc1f8f`.
- `/private/tmp/foks-celld-a2-api-freeze-list-review.json`: `b0cbfcfc53495c0a4350c3139a4b9cd0dc1cdd1b4a5f28ddf632020429216f28`.

## Accepted cold-cycle trace refinement

# A2 bounded cold-cycle trace refinement

The approved A2 API keeps the 512-event and 65,535-byte JSON limits unchanged.
A normal retirement uses enough lifecycle events that retaining a detailed
trace for all 32 cold cycles would exceed the event bound. For the closed
`cold-cycles` scenario only, retain the complete ordered trace of the first
cycle, then retain exact checked per-kind event totals across all 32 cycles
and all 32 per-cycle stage timings, normal completion, SQL persistence and
fresh-Wasm observations. Report `trace_mode=first-cycle-plus-exact-aggregates`
and `detailed_trace_cycles=1`. This is an explicit observation format, not
silent truncation or a claim of a full 32-cycle event transcript.

Every invocation still executes the same admission, original owner/deadline,
state/generation, joined task, normal Worker/probe drop, output disposal and
final owner-release checks. Each event updates its checked total even when its
full detail is not stored. Counter overflow or a full trace exceeding 512
fails the diagnostic. Other scenarios continue to retain all detailed events
and report `trace_mode=complete` with the actual detailed cycle count.

No runtime policy, capacity, public serving capability, source path, dependency,
fixture bytes or physical completion claim changes. The report explicitly
names its coverage. The source freeze and both source reviews still precede
execution. Author review: PASS; this preserves the finite observation budget
without omitting lifecycle validation or mislabeling aggregate evidence.

Direct review: `/private/tmp/foks-celld-a2-trace-addendum-root-review.json`, SHA-256 `1946f3b3542148c6f0f0443499e57166bca4fed944043d11149acbee84119e4b`. Independent review: `/private/tmp/foks-celld-a2-trace-addendum-list-review.json`, SHA-256 `8c1ee873e11e1624516454545243fbcd1a6bad20defd1caed7561a78eb5fc58b`. Both passed before candidate execution.

## A2 tooling source review and synthetic regression results

The four-file tooling source passed root and independent source review before
execution. The first 63-test run had 62 successes and one fixture error: its
synthetic upstream archive omitted the workspace `Cargo.toml` that the unchanged
graph collector requires. The real pinned archive contains that manifest.
Only the synthetic contents were corrected before inventory/archive generation;
production checks and test assertions were unchanged. Both narrow source
reviews passed before rerunning. The corrected run passed all 63 tests, with
zero failures, errors or skips. These results do not qualify the runtime patch.

- Frozen corrected source: `/private/tmp/foks-celld-a2-tooling-source-b.json`, SHA-256 `f3f03b2e4b0ef9262ff03c2bfca29e53c76edf33da413e4bea7ade5e46563b4e`.
- Full independent review: `/private/tmp/foks-celld-a2-tooling-list-review.json`, SHA-256 `ef16f40fd424c20ad3e28d800ff873c22efc60aaaef866c5fa9a8a406a0a3c50`.
- Narrow root review: `/private/tmp/foks-celld-a2-tooling-fixture-root-review.json`, SHA-256 `70a9745f65ea69cb199c90e3f492db060f90f4b0ef22efcc3be10324db40eea5`.
- Narrow independent review: `/private/tmp/foks-celld-a2-tooling-fixture-list-review.json`, SHA-256 `c744f54d7a2a36abd9982869b0af96ab22880071b54d4b2637a60b672a629807`.
- Preserved failed-run receipt: `/private/tmp/foks-celld-a2-tests-a/receipt.json`, SHA-256 `bb9201d3e7a8016d99bba1c2c2fef46fa9bce5037ff713d90266fca6703a7f97`.
- Passing rerun receipt: `/private/tmp/foks-celld-a2-tests-b/receipt.json`, SHA-256 `510f14481512da1db40fccb4dcbe4cb86639aef9aff47015354ae741ac7f2b29`.

## Accepted A2 stage-timing clarification

The pinned celld harness exposes the last I/O timestamp as performance.now()
and Date.now(). It does not advance during synchronous JavaScript execution
(harness.js lines 10748–10790). Therefore a JS timer around Wasm compilation
and instantiation cannot measure that interval. Do not report its zero delta
as a measurement.

Keep the cycle's instantiate_us field, set it to null, and explicitly state
that compilation/instantiation is included in the native execute_us interval;
no separate native instantiation interval is measured. The native Instant
intervals for create, adopt, execute, close and full drop remain measured.
This narrows only the stage observation. It does not alter the original
deadline, physical completion, capacities, report bounds or source roster.
The fixture removes the misleading JS timing instructions and their native
extraction; final program hashes must bind those reviewed bytes before tests.

The integration assertions must check the null field and explicit timing
policy. Later deployment performance work may add a separately reviewed native
measurement hook if an isolated instantiation measurement becomes necessary.
A2 is not a latency percentile, density or 15-second recovery qualification.

Direct review: `/private/tmp/foks-celld-a2-timing-root-review.json`, SHA-256 `e97faad1d0413ac1cee0baa44c4b80e6efea7a27b2c642fa77e583a8663b1fad`. Independent review: `/private/tmp/foks-celld-a2-timing-list-review.json`, SHA-256 `25befb0582cdb7ee3a7c2f2564c5d7bfdaff69d249c034f39be7510ed94d7bde`. Both passed before runtime execution.

## Accepted A2 implementation and local results

Commit `dca38eecb08263a3594f70025df7a309ed3682a5` contains exactly 12
implementation files under `tools/foks-celld-guarded`; no Markdown. The default
identity configuration and the lifecycle configuration both built and passed
strict lint on the selected outer harness targets. The upstream runtime was
compiled; this does not claim primary-package Clippy coverage of celld itself.
The 63 tooling regressions passed. The actual-binary integration test passed all
28 fresh-process scenarios, including 32 cold cycles, four saturation rounds,
and five expected panic quarantines. All owned command groups were cleaned up.

The raw event traces show normal Worker destruction, final backing-probe drop,
and memory release in that order. Execution credit follows output, native
payload, and record-owner disposal. The opposite output/runtime completion
orders both passed. Uncertain panic cleanup retained the original request owner
and its capacity. Saturation reached two occupied slots and 128 MiB of reserved
accessible Wasm memory, without granting early reuse. Across cold cycles, one
tenant retained SQL counters 1–16 while each Wasm instance was fresh; 16 further
tenants each began at counter 1. The largest diagnostic report was 10,868 bytes.

Two compile failures were preserved and corrected before runtime execution.
The deliberate lock-poison diagnostic now captures only the actual ledger
mutex, avoiding an unnecessary unwind-safety requirement on its opaque owner.
The outer caller converts admission errors using the existing identity-caller
pattern. Both narrow corrections received direct and independent source review
before rerunning. Neither changed admission, ownership, fixture assertions,
public protocol, dependencies, or the intended cleanup behavior.

Final identity build/lint/probe took 18.278/9.762/0.683 seconds. Final lifecycle
build/lint/probe/matrix took 2.362/10.931/0.999/36.940 seconds. These are local
development-build observations, not service latency or tenant-density results.
The 64-MiB reservation excludes V8/native/RSS overhead. Instantiation remains
unmeasured separately and is included in the native execution interval.

- Frozen final source: `/private/tmp/foks-celld-a2-source-c.json`, SHA-256 `a17a5b4ff5f32a793109ec0ae62fafa93a32d2c6f28c2df31d4ab1298f5807e9`.
- Qualification: `/private/tmp/foks-celld-a2-qualification.json`, SHA-256 `e7f698c19a8417ff825bc9d0875c2ffe6accf42b91f9026cc7168424e93ca8d2`.
- Direct final review: `/private/tmp/foks-celld-a2-qualification-root-review.json`, SHA-256 `f9e142b753048f74ef7bd57feecc553296f36fd112025a980b4ea139a8198a2d`.
- Independent final review: `/private/tmp/foks-celld-a2-final-import-review.json`, SHA-256 `9b83137912a13b6679a07c2a5ad97e2006221d67ffe8fbbdb3843173bc1c84f6`.
- Acceptance: `/private/tmp/foks-celld-a2-acceptance.json`, SHA-256 `e46995b4be5f4567e8b615572e54c82d057e6ef85fe4673ac8c5f9dc919fc780`.

The final build receipts and preserved binaries are in the validation worktree's
`tools/foks-celld-guarded/runtime/evidence/build-ahqojxqr` (identity) and
`build-lzm03inc` (lifecycle). Earlier source, graph, and failed-build evidence
remains intact. This qualifies only the closed local physical-lifetime fixture.
Canonical node integration, actual FOKS serving and socket output, durable S3
acknowledgment/fencing, absolute SQL interruption, whole-node cleanup, density,
simultaneous recovery, and the 15-second host/AZ target remain later gates.
