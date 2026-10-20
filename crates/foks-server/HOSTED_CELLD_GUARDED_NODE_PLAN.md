# A3b: guarded invocation ownership in the actual celld process

B2 amendment for continuation: [the lifetimes plan](HOSTED_CELLD_NODE_LIFETIMES_PLAN.md)
records the measured and reviewed payload policy that supersedes this broad
plan's initial numeric proposal for B2/B3: 48 KiB typed Rule-R storage plus
8 KiB owned/channel, 64 KiB per cleanup task and 520 KiB total cleanup reserve.
Inline/native bounds and the 8-MiB invocation cap are unchanged. Historical B1
receipts still require their original 200-KiB reserve. Use the accepted B2
source/profile and exact patch roster when drafting B3.

Status: source-backed plan reviewed by the author, root, and an independent
subagent on 2026-10-16. A3a baseline acceptance is complete in
`fee8db31ae8df590e32bc02f8c8de815d9f7ecea`. B1's exact input rebind to that
commit is being reviewed before implementation.
The frozen plan is `/private/tmp/foks-celld-a3b-execution-plan.md`, SHA256
`9d9f9d166142435dc43a75edde559c2399a2d130dc8e2ef524fcde2fb2579ff0`.
Root review: `/private/tmp/foks-celld-a3b-execution-plan-root-review.json`, SHA256
`adabd338c5264ca80c8cdfd73179d9bf781a09a3405416ca54c1563ffe2d3be4`.
Independent review: `/private/tmp/foks-celld-a3b-execution-plan-list-review.json`,
SHA256 `9a698f1af429dce9379207059be23c39bcecd85de2765fb0bfc8ef08a3c3b0d8`.
Before B1 edits, its layout freeze must classify allocation pools and count the
process-lifetime Request against `maximum_in_flight`; excess connections must
refuse without consuming mandatory control capacity. The stated connection
ceiling does not promise that every connection can simultaneously obtain a
Request. Keep this Markdown unstaged.
The accepted A2 base is `dca38eecb08263a3594f70025df7a309ed3682a5`.
A3a's actual-binary baseline must be accepted and committed, and this plan must
pass root and independent review, before source work starts. Rebind the source
inventory to that accepted base; preserve the prior A3b design and binary
assessment unchanged. Root owns repository Markdown, which remains unstaged.

The result sought here is a closed, default-off qualification mode of the real
celld binary. It must use its canonical bucket deployment, ownership actor,
restore/VFS, durability gates and shutdown service loops, while a real A0
Request in that same process owns each guarded invocation through physical
output and retirement. It is not yet a production FOKS endpoint or metadata
adapter. No FOKS wire, cryptographic, database-schema or Go compatibility change
is part of this group.

## Source findings that determine the design

The companion inputs receipt binds the actual accepted A2 source bytes. These
locations are navigation aids, not substitutes for the hashes.

| Actual source | Consequence |
| --- | --- |
| `celld/main.rs:3442–3631`, `3632–4383` | Keep V8-before-Tokio and the existing lexical Run startup, actual listeners, bucket, recovery-before-DurabilityOwner installation, actor and host channels. No second Run implementation or fabricated AppHandle. |
| `celld/generation.rs:463–627` | Both boot and reload warm every script's stateless pool, then create cell pools. The primary stateless field is mandatory. Merely skipping a guarded primary leaves an invalid generation. |
| `celld/cell_runtime.rs:92–107` | Existing generation lookup falls back to current for zero or missing IDs. Guarded execution needs an exact nonzero lookup and a retained original generation. |
| `logic/lib.rs:3603–3621` | Ordinary GateRequestAt falls back to ordinary routing/activation. A precheck in the asynchronous shell cannot make that resident-only. |
| `celld/js/storage_ops.rs:250–344`, `host_channels.rs`, `cell_dispatch.rs:48–67` | Sync samples the real connection in the isolate, but its enqueued gate lacks original invocation ownership and replies before ActivityGuard drops. |
| `celld/actor.rs:935–1010`, `2889–2907`; `logic/lib.rs:6324–6346` | Guard Drop sends ActivityFinished without acknowledgment. Processing the event can leave an outstanding gate pin; processed and inactive are different facts. |
| `celld/actor.rs:3632–3718`, `3770–3895`, `3914–4014`; `actor/effects.rs`, `actor/production.rs` | Restore, start, durability/ownership proof and stop cross spawned tasks. Dropping actor-side join wrappers can detach them. Original ownership must survive independently of the reply and actor. |
| `celld/runtime.rs:1243–1350`, `1397–1500` | Start obtains the real activation VFS. Stop removes runtime handles before later persistence disposition, which can fail. Worker destruction is not activation retirement. |
| `celld/main.rs:3248–3325` | Hyper owns ordinary HTTP socket transmission; returning a response body is not a socket/flush ownership proof. |
| `celld/main.rs:4745`, `5053`, `5184`, `5244`, `5469`, `5590` | Guarded futures must be polled in serving, pre-drain and semantic drain; both Handoff and Preserve completion require them. Final connection flush no longer pumps host services. Keep intentional hard exit. |
| `celld/fleet.rs:282`, `bucket.rs:1631–1746` | Ordinary startup may continue after repeated probe transport errors. Guarded readiness must separately require Conformant from the same real Bucket. Ordinary policy remains unchanged. |
| A0 `memory.rs:97`, `deadline.rs`, `framing.rs:95` | Use actual preadmitted Request and retained-owner methods. Capture one original Instant; use the real write_all-plus-flush helper. A timeout or dropped waiter does not establish destruction of detached work. |

## Explicit refinements to the prior conditional design

1. Add one internal pure-logic event for exact resident-only auxiliary gates.
   This now explicitly includes `logic/lib.rs` and `logic/types.rs` in scope.
   Keep ordinary GateRequestAt behavior unchanged. Event has no serialization
   derive today; this is an internal model/IPC change, not a network protocol.
2. Add a processed-finish acknowledgment and a separate inactive observation.
   Neither alone is an activation, effect-join or memory-disposal certificate.
3. In the closed qualification Action only, use the existing public listener
   for a fixed diagnostic TCP frontend. It owns the accepted TcpStream and
   actual A0 framing future. The canonical internal HTTP listener, actor and
   host-service pumps remain real. Ordinary Run retains Hyper unchanged. This
   qualifies actual TCP output ownership, not production HTTP, FOKS RPC or TLS.
4. The fixed deployment has an inert ordinary primary and exactly one cohosted
   guarded script. Canonical deployment loading discovers the latter through
   the primary's fixed service binding. Only the guarded descriptor is excluded
   from every ordinary pool/Worker construction. The small ordinary primary is
   explicit process overhead outside the guarded two-slot/64-MiB accounting.
   This avoids a placeholder Worker or an optional-primary Generation redesign.

These are proposed plan decisions, not claims that the current source already
implements them. They need the same two plan reviews as the rest of this file.

## Closed profile and limits

Use `foks-guarded-node-qualification`, forwarding the existing A2 feature plus
the two optional existing local dependencies, exactly as the reviewed
`foks-celld-a3-binary-build-selection.md` specifies. The outer feature is
`node-qualification`. No registry package, engine version or native archive
change is proposed. Native metadata stays disabled; no foks-server/server-db
edge enters celld. Default celld and A2 lifecycle resolve their old subclosures.

The CLI is `celld foks-guarded-node <closed-scenario> <the reviewed node settings>`.
Parsing yields validated Settings into the existing Run body. Settings are the
accepted local bucket profile: exact dummy credential environment, explicit
loopback public/internal addresses, canonical deployment pointer, bucket
durability and fresh scoped data directories. Reject local-script/dev-store,
managed enrollment, no-runtime and log/fleet-durability alternatives. Use the
same A3a up-to-two-live-node/five-port policy; no extra public listener, proxy,
external service or AWS permission is introduced. A separate process starts
each destructive/quarantine scenario.

| Resource | Fixed limit / refusal |
| --- | --- |
| Guarded execution | A2's same two slots; one invocation per tenant, including Retiring and Quarantined. No second counter or overflow queue. |
| Wasm | A2's exact fixed nonshared memory32 bytes, initial=max=1024 pages. Reserve 64 MiB accessible capacity per slot before construction; virtual reservation/RSS remain separate. |
| Fixture tenants | Three fixed class-qualified names, at most two active; no arbitrary scope/path from ingress. Scope storage cap 256 UTF-8 bytes. |
| Original request deadline | 15 seconds from accepting the invocation socket, including reading its header. It is captured in this process before asynchronous parsing/admission and is never renewed. |
| Diagnostic frontend | At most four owned accepted connections (two invocation slots plus bounded control/refusal work), no permit wait queue. One request per connection; 16-byte fixed header. Unknown bits/opcodes/trailing request bytes refuse. |
| Native response | At most 4 MiB including framing, a fixed pattern only in backpressure cases. Ordinary fixture response is fixed 32 bytes. No arbitrary JS response stream. Charge before allocating; no full duplicate frame. |
| Native request accounting | Actual A0 PoolLimits: response 8 MiB, working 64 MiB, control 256 KiB, maximum in flight 4. A separate real process Request holds static diagnostics/ledger; actual invocation Requests hold their allocations. A2's Wasm-capacity ledger is distinct from these native allowances. |
| Guarded effect jobs | Fixed 16 entries per invocation, split into eight forward-work and eight reserved cleanup entries, two invocations. Four additional preallocated control records per invocation cover cancellation, primary finish, auxiliary finish and retirement disposition. One pending auxiliary sync per invocation. Forward work cannot borrow cleanup/control reservations. Actual observed high-water counts are reported separately. |
| New typed task payload | Charge the sum of simultaneously live typed capture/result/retained-completion/channel payloads before every actual task, <=16 KiB and alignment <=16, plus separately bounded owned allocations. A maximum rather than sum requires source/layout proof of a real shared union and no overlap. Tokio headers/retired-task allocation overhead are separate process overhead. |
| Native immutable/report storage | Descriptor source <=32 KiB and two exact script records; <=512 event records and <=64 operation summaries; <=128 KiB native report cap and <=65,536-byte JSON output record. Overflow is failure, never truncation. Actual layout/serialization overlap is charged before allocation. |
| Qualification run | Original parent 600-second mode deadline, existing 256-MiB evidence cap and owned process-group cleanup. Each scenario also has a fixed <=90-second child window; no new per-await budgets. Requests still close/refuse publication by their original 15-second boundary. |

The header is a qualification-local record, not a FOKS wire extension: eight
literal magic bytes `FOKSGN01`, protocol byte 1, closed command byte, tenant
ordinal byte, one phase ordinal byte, four zero reserved bytes. The selected
scenario determines the finite legal command sequence. Begin uses phase zero;
Release/Observe use only that scenario's named private barrier; no caller
Instant, budget, generation, epoch, arbitrary program or completion assertion
is accepted. The client half-closes its write side after the header; the server
requires EOF under that same original deadline before invoking the actor, so
trailing-byte rejection is observable and does not require an unbounded read.
Control/refusal connections have a one-second cap under their own
original acceptance deadline and cannot consume execution slots. Diagnostics
do not let callers mint a successful gate or retire a native resource.

The new node fixture has new exact JS/manifest/descriptor hashes. Reuse A2's
Wasm bytes unchanged, but do not reuse its JS identity. Its admitted fetch
instantiates once, touches the complete 64 MiB, uses one fixed SQL row, awaits
the real `ctx.storage.sync()`, and either returns 32 fixed bytes or throws after
the write. No imports, extra Wasm, egress, alarms, queues, loaders, facets,
WebSockets, container or scheduled handler is admitted. Compilation and
instantiation stay within measured native execute_us; instantiate_us remains
null unless an actual native timing seam is separately reviewed.

## Private interfaces and ownership

Keep the separate binary/library boundary small. The feature-only library
reexports closed `GuardedNodeScenario`, opaque `GuardedNode`, consuming
`GuardedConnection`, and scalar report/layout types. It does not export raw
Capacity, Lease, Worker, BackingStore, mutable descriptor, ready token or caller
disposal callback. Suggested concrete entry signatures are:

```text
GuardedNode::install(GuardedNodeScenario) -> Result<Arc<GuardedNode>>
GuardedNode::require_bucket(&Arc<Self>, &Bucket) -> Future<Result<()>>
GuardedNode::bind_running(&Arc<Self>, &AppHandle) -> Result<()>
GuardedNode::accept(&Arc<Self>, TcpStream) -> Result<GuardedConnection>
GuardedNode::stop_admission(&self)
GuardedNode::poll_retirement(&self, &mut Context) -> Poll<Result<()>>
GuardedNode::drained(&self) -> bool
GuardedNode::report(&self) -> borrowed bounded scalar/event view
```

`accept` samples its original Instant immediately at the owned acceptance
handoff, before allocation or asynchronous work; the sole private main call
site invokes it directly after listener.accept without an intervening await.
The library derives the fixed 15-second deadline. No public
constructor accepts a prebuilt Request, caller token or readiness boolean.
`bind_running` checks the actual runtime/actor private installation identities,
the positive same-Bucket probe and actual initialized deployment/runtime state.
It cannot derive authority merely from AppHandle existence. Every operation
still obtains the live actor's exact route, epoch and generation; a startup
observation is not perpetual authority. No public mutable fields or test-minted
Ready object are permitted.

Place `guarded_node.rs` as a child of private `guarded_execution` (using a fixed
path attribute), so it can reuse A2's private ledger without broadening that
ledger's visibility. Its `control.rs` and `output.rs` children isolate ownership
and transmission. `main/guarded.rs` owns only settings validation, the finite
frontend set and diagnostic formatting; it never drives a second actor or
supplies success callbacks.

The consuming states are `Accepted -> Admitted -> Routed -> Executing ->
GatedFrame -> Retiring -> Retired`, with Quarantined terminal for uncertainty.
One owned aggregate carries socket/input/response, actual runtime and activation
handles, core ActivityGuard, generation, real Request/control and A2 lease.
Data/resource fields precede control and the execution lease in declaration
drop order. Construct the aggregate before spawn or fallible handoff; never
capture disjoint fields. Failure before admission closes the real socket while
the original Request remains alive through input destruction.

The original binding contains: actual Request Arc identity, original Instant,
same cancellation object, fixed tenant/scope, nonwrapping invocation generation,
canonical core request ID, real node process generation, actor-assigned epoch,
nonzero RuntimeStarted generation, exact retained Generation Arc and immutable
descriptor digest. Epoch comes from the actual RestoreSpec/StartRuntime effect,
not from the header. No replacement may change any already-bound field.
Checking a copied tuple is insufficient: private ownership identity is checked
at every enqueue/dispatch/result/publication/retirement transition.

Separate process ownership is required. The static A2 ledger plus descriptors,
bounded retained jobs/report and serialization overlap remain charged to a real
process A0 Request until diagnostic process exit. Normal invocation Weak<Request>
witnesses must become dead only after their own resource completion. Quarantined
invocations remain live and are reported separately; process exit is not a
physical-credit-return observation. Do not charge process-lifetime storage to a
normal invocation then declare its Weak witness dead.

This is a bound on the new owned payloads and retained buffers, not a complete
celld heap bound. Existing Pools/semaphore bootstrap allocations, canonical
deployment parsing, actor maps, SQLite caches, V8/JS engine heap, TLS/kernel
buffers and Tokio scheduler/header/retired-task storage remain explicit process
overhead. New guarded records/queues are finite and charged as specified;
using the process-overhead exclusion to hide an added unbounded guarded queue
is forbidden. Fixed control refusal checks use direct original Instant
comparisons and fixed status enums, avoiding an allocating discarded timeout
error before admission. Actual allocation/layout review must account for every
new owned string, Arc, channel, boxed future and simultaneous report copy.

## Descriptor, canonical activation and generation binding

Before Generation::build, install one immutable exact-byte descriptor policy.
Validate the fixed two-script graph, script/version/module identities and class
catalog without executing it. Both boot and every reload call the same policy.
The inert primary follows ordinary construction. The guarded cohosted config
is retained as data but excluded from StatelessRuntime::start and cell_pool.
No warmup evaluates the guarded script. The final ordinary services map must not
contain its pool and the ordinary cell-isolates map must not contain its script.

Add a private WorkerConfig marker whose constructor belongs to descriptor
validation; script name alone cannot confer it. Generic Worker::load_config
refuses that marker unless a private preadmitted construction scope is active.
That scope is created only from the same live A2 lease and exact original
binding. Refusal occurs before Isolate creation. The new node load helper takes
the real validated generation config; A2's hard-coded plain-storage fixture
constructor is not a substitute. Preserve A2's typed CleanBefore/CleanAfter
distinction; unexpected errors/panics quarantine rather than infer destruction.

Central actor ingress rejects every ordinary request for a guarded scope before
any core event that could activate it. Runtime entrypoints independently refuse
generic fetch/service/RPC/alarm/queue/loader/facet/WebSocket access, and generic
construction has the marker defense. Wake/resume/handoff and eager reload must
not instantiate a guarded descriptor without its admitted origin either.
These are separate defenses: a missing generic pool is not by itself proof that
no ownership/restore or hidden restart was initiated.

The private guarded route registers its original binding before submitting the
real actor request. Canonical owner lookup/CAS, takeover and RestoreSpec remain
unchanged. The actual Restore and StartRuntime paths find the registered owned
binding by exact scope/core-operation identity; a stale reply cannot bind a new
request. Use actual RuntimeManager::db_path and replication.ltx().activation_vfs
for its real epoch. No A2 temporary SQLite path, epoch 1 constant, serialized
whole state, fake RuntimeStarted/Published, or fallback to ordinary pool exists.

Start creates a dedicated Worker/Slot using the same A2 two-slot admission and
normal destruction proof. It adopts the canonical CellStorage with real path,
epoch, replicated-wake setting and activation VFS. Publish follows the real
actor transition. The guarded fetch uses the canonical cell driver and actual
LocalRequest success/failed-in-turn output-ticket semantics, not generic
fetch_cell's detached fire-and-forget wrapper. Register and join the actual
driver, including its post-response/background work, through its final abandon.

Add exact-generation lookup returning None for zero/unknown/drained IDs; normal
generation_by_id remains unchanged. Snapshot the real generation before guarded
construction and retain it through output/retirement. A reload may let an already
bound old invocation finish only while its original actual generation remains
valid and core cancellation has not revoked it. It must never rebind to current.
Use checked generation increment for the guarded profile and refuse wrap.

## G1: owned sync and processed auxiliary retirement

The running guarded Worker installs its private native invocation origin before
the first event. `start_cell_event` copies it into that event's IoContext, which
already follows promise reactions. Neither JS arguments nor a global lookup by
tenant name supplies the origin. Validate the actual scope, epoch, generation
and parent core request before sampling storage and again before enqueue.

The guarded `storage.sync()` branch retains the real sample, original owner and
control through a preadmitted single auxiliary GateReq. It uses the existing
real GATE_TX and main's real dispatch_gate; ordinary gates remain unchanged.
No unbounded queue is introduced: reserve the auxiliary slot and typed payload
before asyncrt enqueue/channel construction, and fail before sending if full.
Retain that reservation until processed auxiliary inactivity, even if the JS
promise or reply receiver disappears.

Add internal `ResidentGateRequestAt { request, parent, cell, epoch, generation,
now_mono_ms }`. The pure core atomically requires node authority, no drain or
forced-swap cancellation, exact Resident epoch and generation, and the exact
active non-gate parent belonging to that cell. It admits the auxiliary gate or
returns the existing refusal result. It never calls ordinary request/restore
as a fallback. Ordinary GateRequestAt and its historical behavior stay intact.

After admission call the existing gate_output with the real sampled ticket.
AwaitDurable and VerifyOwnership remain the actual canonical proof; do not
turn a passed startup probe, local commit, failed write, synthetic ticket or
expired caller into a durability acknowledgment.

Add a consuming guarded finish operation to ActivityGuard. It snapshots alarm
state under the same existing lock/order, disarms the ordinary Drop sender and
sends one owned finish message. The actor processes AlarmObserved then the real
ActivityFinished exactly once. Its first observation is Processed; it returns
Inactive only after the actual core has no active/gate-pinned/barrier ownership
for that request. Add a narrow pure state query for that predicate and a bounded
private finish waiter checked after transitions; do not repeatedly inject
ActivityFinished to poll it. Processed-but-pinned retains all ownership.

Only after the auxiliary Inactive disposition may dispatch_gate settle its
reply and release that auxiliary payload. Reply loss does not discard the
recorded disposition or ownership. Actor loss, stale identity or an unprocessed
finish quarantines; it cannot make the parent finish successfully. Cleanup
identity checks still run after expiry, but expiry never forbids attempting
destruction/finish or causes early credit return.
This wait concerns only the auxiliary request ID. The parent remains active
while awaiting sync; waiting for parent inactivity here would recreate a cycle.

## G6: actual effects, activation and final retirement

Introduce a private bounded guarded effect registry under the real process
owner. Before enqueue/spawn, register the owned task payload and original
invocation binding. The registry owns the actual JoinHandle, not a wrapper that
only awaits another forgotten handle. Its record remains even if actor reply
delivery or the actor's FuturesUnordered disappears. Main polls its joins in
all three host-service loops. A completion is recorded before attempting actor
delivery; undeliverable completion remains a typed observed result, not success
of the absent actor's transition.

Reserve the entire cleanup/control geometry as part of the original admission,
before allowing forward work. Its eight cleanup task slots and four control
records are inaccessible to ordinary admission and forward effects. The four
control records remain usable without allocating or acquiring a forward permit
for cancellation, primary finish, auxiliary finish and final retirement. Closed
duplicate cancellation uses the already-owned sticky signal; repeated finish
is refused rather than enqueueing additional records. Reserve bytes as well as
slot counts: each cleanup task has a one-use creation credit for up to 16 KiB
of simultaneously live typed payload plus 8 KiB of explicitly enumerated owned
allocations; each of the four control records has up to 2 KiB of separately
enumerated allocation credit. The complete 200-KiB cleanup reserve is charged
on the original actual Request before admission. Preallocate reusable fixed
control structures then; any later allocation consumes its already-held
private credit without calling a fresh Request reservation after failure.
These maxima are caps, not measurements or coverage of V8/SQLite/Tokio headers.
The B2 layout ledger must prove each real layout and owned buffer fits.

Cleanup credits remain usable after original expiry or sticky Request failure
only for cancellation/destruction/finish/retirement; they never clear failure
or authorize execution/publication. A spent creation credit is not refunded by
task completion. Retries within one supervised native task keep that same job
and payload ownership; another task needs a remaining reserved credit. A slot
is physically reusable only after its prior result/capture/channel payloads
are consumed, and reuse cannot manufacture another allocation credit.
Held/failed/uncertain work never donates its reservation to another invocation.

Before B2 source edits, freeze a complete actual effect/control inventory with
constructor, synchronous versus task behavior, forward/cleanup class, maximum
simultaneous count, original owner, exact result and actual join site. Prove
from the closed one-parent/one-auxiliary model that mandatory cancellation,
finish, proof, stop and disposition fit the reserved eight task creations/four
control records and byte geometry even
when all eight forward slots are occupied. If this does not fit, change the
reviewed geometry before implementation; do not discover the shortage by
dropping a mandatory cleanup task. Tests deliberately fill forward capacity
then execute every cleanup path. Reserved-capacity exhaustion or uncertain
cleanup remains charged/quarantined and cannot yield a successful release.

Cover the entire per-cell guarded effect chain, including ownership/read/CAS
and VerifyOwnership tasks preceding or following restore, as well as Restore,
StartRuntime, Publish handoff, AwaitDurable, EnsureDurable and StopRuntime.
Do not retain only the four conspicuous native tasks while allowing a late
ownership task to change the activation after the invocation was released.
Ordinary node lease/fleet background work remains process-owned and unchanged.
For each covered variant freeze an explicit constructor/result row in source;
an unmatched guarded effect refuses/quarantines before executing rather than
falling through to the ordinary detached spawning path.

The inventory must classify every existing per-cell variant, not just a name
pattern: ReadOwner, ReadNodeLease, RecoverNodeLog, ReadCapacityPeers, CasOwner,
ReconcileWakeEntry, ReleaseOwner, AdoptReleased, CancelCellActivity, Restore,
StartRuntime, Publish, EvictionAdmission, EvictionFinished, EnsureDurable,
AwaitDurable, VerifyOwnership, Release, StopRuntime, AbandonEvictionStop,
FireAlarm, Complete, CompleteWorker and CloseWebSocket. Include the separate
actor MaintainWake/schedule_wake_maintenance path: it currently detaches even
though an alarm-free cell can still have wake-entry maintenance. Task-producing
reachable cases receive actual owned supervision; synchronous deliveries use
the reserved control records; unsupported closed-profile cases refuse before
execution. No-alarm/no-WebSocket policy is not permission to omit refusal or
identity handling for those variants. Node-global lease/timer/Halt work remains
explicitly process-owned; it must still revoke guarded publication on fence.

The original deadline closes permission for further user execution, new sync
requests and output. It does not erase already-started I/O. Close the invocation
socket by its original deadline, signal actual cancellation, and keep polling
owned cleanup without installing a replacement request deadline. Existing
physical attempts and required exact-epoch cleanup remain charged. The parent
scenario/process deadline bounds how long the qualification waits; crossing it
is incomplete/quarantined, never a claim that the task or PUT terminated.

After output success or physical abandonment, first join the actual driver,
including background work and final turn_abandon_input_gates/entry.abandon,
then finish the primary ActivityGuard through Processed and Inactive. The live
driver retains Affiliation/Slot ownership; output reply is not that join.
Request the existing canonical eviction for the
exact owned epoch under the guard registry's no-new-admission exclusion. Treat
AlreadyAbsent as insufficient by itself: the registry must also know all prior
exact-epoch work, Worker/probe and activation disposition completed. A normal
eviction is acknowledged only through its real actor result and actual stop
task join. No direct synthetic RuntimeStopped is allowed.
If actor-initiated stop/cancel races a still-live driver, destructive Slot/Worker
retirement must retain and await that actual driver ownership or quarantine.
A StopRuntime result alone cannot establish the uniqueness required by A2's
guarded_consume/destruction path. Do not probe uniqueness while known driver
ownership is intentionally live and then reinterpret its failure as disposal.

Preserve real StopCause/StopMode mappings. Normal Worker/SharedIsolate destruction
and final scoped BackingStore probe drop release Wasm capacity only after A2's
actual ownership checks. They do not release the execution slot while output,
finish acknowledgments, native task payloads or activation cleanup remain.
Native stop persistence can fail after memory destruction; that state stays
Retiring/Quarantined. Real stop failure/cancellation is recorded as such, never
used as a convenience restart signal. Any canonical restart attempt for a
guarded descriptor still needs the live original admitted binding and cannot
create an ordinary Worker or fresh replacement token.

Final execution release requires all of: output disposition, complete driver
join, normal Worker/probe destruction, primary and auxiliary Inactive, all
guarded effect joins/results consumed, exact-epoch activation cleanup, and no
taint/uncertain slot/pending callback. Same-tenant Busy persists until this
conjunction, even after the Wasm capacity edge. Quarantine retains the concrete
objects/handles when possible; no Boolean claiming an unknown resource is gone.

## G2/G5: actual output ownership and cancellation

`GatedFrame` owns the bounded complete frame, actual Request, activity, original
control, exact generation/epoch and execution/activation retirement owner.
Its consuming transmit method calls the existing A0 write_response_until on
the actual owned TcpStream, selects against original deadline and the actual
ActivityGuard cancellation receiver, and checks both before the first write.
It must observe a cancellation receiver already true, not only a later changed
notification. Gate failure can publish only the closed bounded refusal format
when the original control still allows publication; expiry/cancellation closes.

On cancellation/error/timeout, physically drop the transmit future and close
the socket before marking output abandoned. On success, actual local flush
completes before output disposition. Retain the original Request through all
write/flush pending states and frame destruction. Do not release it when the
handler replies or when an awaiting caller times out. Bytes already accepted
by the socket cannot be recalled; this is no peer receipt or TLS guarantee.

For a real write-pressure test set finite small send/receive buffers on the
owned sockets, send the closed 4-MiB native pattern and wait for an instrumented
actual poll_write Pending observation. The peer deliberately does not drain
until the next closed control action. Absence of real Pending fails that oracle;
elapsed sleep is not a substitute. Exercise disconnect and cancellation there.

Tokio TcpStream flush is immediate on this selected engine. A separate private
generic AsyncWrite seam tests a deterministic pending flush on the SAME frame
and A0 helper, with barriers, successful release, actual canonical activity
cancellation and original-deadline expiration. Label it test-writer evidence;
do not claim the OS socket actually had a pending flush. Paused Tokio time must
not be described as advancing std::Instant. Deadline tests either use the real
captured timeout_at clock with its scoped claim or a real finite deadline.

For canonical cancellation, the closed test drives an actual generation reload
and its real GenerationChanged/forced-swap path, or the actual ReleaseAll drain
input. It observes the resulting real Effect::CancelCellActivity and its watch
notification. A diagnostic Boolean alone does not satisfy G2. Do not invent a
RuntimeStopped or call the shell effect handler with a fabricated effect.

## G4 and canonical main-loop readiness/drain

Call require_bucket on the same Bucket object used by Run before guarded
readiness. It invokes actual probe_startup_storage_steps and accepts only
Conformant. Error/unknown/contract violation refuses the guarded mode even if
the ordinary fleet helper would continue. Keep fleet.rs and bucket.rs unchanged.
Readiness also waits for real canonical recovery, DurabilityOwner installation,
actor authority and host channels. No request is admitted while startup proof
or recovery is unresolved.

Proof tests hold the actual AwaitDurable/VerifyOwnership work before launch or
after its real result, with original-owned closed barriers. They never replace
the result with an injected success. Real startup transport error and a fixed
contract-violating HTTP fixture can test refusal through the actual Bucket
client. Such a small fixture is test-only in the existing new Rust test module,
uses an owned loopback port and finite scripted requests, and is not a runtime
S3 proxy. No A3b claim is made about terminating an already transmitted S3 PUT
or safely reclaiming its object. A network proxy for that later proof would
require a separate plan/path/policy review.
The actual Bucket refusal fixture runs under a recorded loopback-only test
policy, using the existing S3-role port with no live node or Moto occupying it.
Its command, finite listener/task cleanup and policy are a B3 test-command
freeze item. Do not run it under the deny-all build policy or substitute an
unrecorded unsandboxed launch.

After the real actor and host channels exist, main/guarded owns the fixed
frontend set. Poll it and the guarded effect/retirement registry in the same
serving, admitted-request pre-drain and semantic-drain loops, alongside existing
gate_rx/do/service work. No await of an entire invocation blocks those pumps.
Guarded public admission stops at shutdown; actual PublicRequestGuard remains
owned until full retirement, with separate explicit guarded-drained checks.
Require guarded-drained in BOTH Handoff and Preserve predicates before final
host-service cessation. If the original process cut arrives first, record
incomplete/charged/quarantined and preserve the intentional hard exit.

Qualification-only stdout uses bounded structured records. Route both explicit
startup Output::line announcements as well as tracing correctly; no mixed text
is accepted as JSON. Emit observations before exit but do not call zero exit,
empty core occupancy or local transport completion full-node disposal. Keep
A3a's distinction between observed core drain and unobserved final upstream
transport/local-task teardown.

## Staged, inseparable implementation groups

Each group gets a concrete source/layout freeze, author review and a second
review by a subagent before root runs its tests, and a separate commit only
after those checks pass. Fixes re-enter the relevant source review before rerun.
No stage exposes a runnable guarded Action until its dependencies and refusal
paths are complete. Source may compile behind the feature while readiness is
unreachable; no temporary constructor is allowed to mint readiness for tests.
Each implementation commit includes its exact patch, selected stage profile,
strict reconstruction support and locked graph evidence. Editing generated
runtime/source alone is not a reviewable or reproducible implementation commit.

**B1 — Descriptor, original owner and core admission.** Install the default-off
dependency/feature edges and private process/control state; implement exact
descriptor classification, generic refusal and exact generation. Add the pure
resident-only event/inactive query with model tests. Keep the new frontend
unavailable. Test actual Request preadmission/failed-state/lifetimes, no warmup,
all generic entry categories, old/default behavior and real core transitions.
Publish and reconstruct the B1 prefix and run its focused tests before commit.

**B2 — Canonical activation, supervised effects and sync retirement.** This is
one inseparable source group: the dedicated A2-owned runtime must not be wired
to activation while effects or sync can escape its owner. Implement real VFS
handoff, complete effect roster/joins, event-local sync origin, processed/inactive
acknowledgments, native driver join and exact stop disposition. Include failed
start-after-restore, late effect/reply/actor loss and stop-after-memory-release
tests. Still no external guarded listener or Ready marker.
Publish and reconstruct the B2 prefix and run its focused tests before commit.

**B3 — Consuming socket output and Run integration.** Wire the closed Action,
fixed public diagnostic frontend, actual ActivityGuard cancellation, owned A0
frame, all three loop pumps, positive Bucket proof and both drain predicates.
Test live TCP plus deterministic flush, original deadlines, cancellation/reload,
shutdown cuts and strict bounded diagnostics. Enable guarded readiness only
when the completed B1/B2 invariants are present.
Publish and reconstruct the B3 prefix and run its focused tests before commit.

**B4 — Final controller and actual process qualification.** Finish the closed
scenario controller/profile and full-process acceptance evidence on the already
reproducible B3 implementation; preserve A1/A2/A3a identities.
Root runs source-bound unit/model tests and both native build modes, then the
closed actual-node scenarios below under owned group/network/resource controls.
No production availability label, density extrapolation or backend decision
is inferred from this finite qualification. B1/B2 source/model/component checks
do not satisfy any full same-process G1–G6 runtime oracle; B3's leaf transport
and startup refusal checks likewise do not substitute for B4's actual node.
They must not construct fake Ready, successful canonical proof or completion
tokens to bypass the absent frontend. Actual driver/V8/quarantine and complete
GatedFrame scenarios run through the B4 closed Action in fresh processes, using
the same process isolation as A2. Leaf tests can exercise actual A0 owners and
transport primitives without claiming canonical admission or publication.

### Reproducible stage topology

Keep the accepted `runtime.lock.json`, `patches/series.json`,
`patches/0001-guarded-physical-lifetimes.patch` and `node.lock.json` byte-identical.
Add separate `runtime-node.lock.json` and `patches/node-series.json`. They bind
the accepted base profile/series by exact path, length and hash, then apply only
the following closed append-only node prefix:

| Exact stage selector | Applied node patches after unchanged A2 patch | Capability |
| --- | --- | --- |
| `node-foundation` | `0002-node-admission.patch` | B1 source/model tests and actual feature-enabled bin build; guarded Action absent, no runnable readiness. |
| `node-lifetimes` | `0002-node-admission.patch`, `0003-node-lifetimes.patch` | B2 component/lifetime tests and bin build; guarded Action still absent. |
| `node-qualification` | all previous plus `0004-node-frontend.patch` | B3 actual guarded Action; full external scenario controller is accepted in B4. |

The outer feature is the same fixed `node-qualification` in all three stages;
the immutable prepared source prefix determines which code exists. Stage names
are build/input modes, not additional Cargo features. `runtime-node.lock.json`
has one current exact stage enum, exact node-series hash and copied immutable
native/compiler/engine pins checked equal to the accepted base. Each stage
commit freezes that current file and the exact required prefix. There is no
free-form patch count, arbitrary input path, mutable stage environment variable
or caller-selected capability bit. Tool mode must match the profile's enum.

The node series format is closed and names the accepted A2/A3a commits, exact
base-series image, exact stage, and the exact ordered patch prefix for that
stage. Every patch records its own byte hash/length and every changed path's
mode/preimage/postimage, including additions. A later patch's preimage must
equal the preceding reconstructed postimage. The union is the reviewed 25-path
ceiling; unchanged files retain the upstream/A2 inventory identity. Use the
existing strict patch parser: no fuzzy apply, shell-generated patch, rename,
symlink, mode change, binary patch or extra path. Proposed finite caps are
512 KiB per node patch, 1.5 MiB combined node patches and 8 MiB reconstructed
source payload; a larger actual implementation requires review before raising
these. Publish a full-tree manifest bound to selected profile and exact prefix.

Before **each** B1/B2/B3 test/build/commit, reconstruct that prefix from the
verified upstream archive plus unchanged A2 patch and that stage's node patches.
Compare the reconstructed complete tree with the author freeze. Test only this
prepared tree, never the unrecorded author copy. A runtime/source or manifest
belonging to another phase refuses; root first preserves its exact tree,
manifest and evidence in a named evidence location, then prepares the new one.
No automatic deletion or in-place relabeling is permitted. Native asset bytes
may be verified and reused, but their input receipts stay phase-bound.

Existing identity/lifecycle/node-baseline receipts retain their accepted
source/input meanings. They do not qualify the new source by name. Historical
reproduction uses the accepted commit/profile; current-stage default/A2
regressions get new stage-bound receipts. In particular, an unchanged A3a
node.lock must refuse a differently prepared node-stage source rather than
silently refreshing its pins. The changed outer manifest/lock and actual
default/A2/node dependency subclosures are reviewed in each stage as needed.

## Exact proposed source roster

Paths below are relative to the pinned celld source root unless prefixed outer.
This is a ceiling, not permission to edit unrelated code in these large files.
New private tests are kept together to avoid changing upstream test topology.
Before each source group, freeze its precise subset and public signatures;
any additional path or capability requires review before editing.

| Paths | Narrow responsibility |
| --- | --- |
| `crates/celld/Cargo.toml`, `crates/celld/lib.rs` | Two optional local edges/default-off feature; opaque bridge reexports only. |
| `crates/celld/main.rs`, `crates/celld/main/cli.rs`, **new** `crates/celld/main/guarded.rs` | One Action into Run, existing listener diagnostic branch, bounded frontend/retirement selection, both drain predicates, structured diagnostics. |
| `crates/celld/guarded_execution.rs` | Reuse A2 ledger/normal destruction with private node child and separate activation/effect completion obligation; preserve A2 lifecycle semantics. |
| **new** `crates/celld/guarded_node.rs`, `crates/celld/guarded_node/control.rs`, `crates/celld/guarded_node/output.rs`, `crates/celld/guarded_node/tests.rs` | Closed bridge, actual Request/control/owned effect registry, consuming frame, focused native/model/integration tests. |
| `crates/celld/generation.rs`, `crates/celld/cell_runtime.rs` | Preconstruction descriptor policy for boot/reload, no guarded ordinary pools, exact generation lookup. |
| `crates/celld/actor.rs`, `crates/celld/actor/local_request.rs`, `crates/celld/actor/effects.rs`, `crates/celld/actor/production.rs` | Registered origin before canonical route; guarded effect supervision; processed/inactive completion; ordinary behavior preserved. |
| `crates/celld/cell_dispatch.rs`, `crates/celld/host_channels.rs` | Owned guarded sync branch and pre-enqueue admission, real shared host channel, reply after inactive. |
| `crates/celld/runtime.rs` | Actual dedicated activation/driver/stop ports plus generic refusal; preserve storage modes. |
| `crates/celld/js.rs`, `crates/celld/js/storage_ops.rs`, `crates/celld/js/guarded.rs`, **new** `crates/celld/js/guarded_node.rs` | Private config/construction marker, native event origin, real sync sampling, exact new node fixture and scoped V8 evidence; A2 fixture bytes unchanged. |
| `crates/logic/lib.rs`, `crates/logic/types.rs` | Internal resident-only gate event and narrow inactive predicate; no serde/wire dependency or altered ordinary event. |

This is **25 celld-source paths**, including six new paths. `engine_api.rs`,
`cell_host.rs`, `asyncrt.rs`, replication/LTX, bucket/fleet and database schemas
remain read-only. The origin follows the private Worker/native IoContext; do
not add a caller-supplied field to the public RuntimeFetch wire-like DTO. If
implementation cannot preserve the generic refusal or actual join obligations
inside this roster, stop at a specific source finding and propose an amendment.

Outer paths are under `tools/foks-celld-guarded`:

* `Cargo.toml`, `Cargo.lock`: only reviewed forwarding feature and exact existing
  local edges. Lock changes require actual graph review before build.
* **new** `runtime-node.lock.json`, `patches/node-series.json`,
  `patches/0002-node-admission.patch`, `patches/0003-node-lifetimes.patch`,
  `patches/0004-node-frontend.patch`: exact append-only phase reconstruction,
  matching A2 and each preceding postimage, exact 25-path ceiling and caps.
* **new** `node-guarded.lock.json`: separate closed mode/scenario/program/claim
  identity; baseline `node.lock.json` stays unchanged.
* `prepare-runtime.py`, `test_prepare_runtime.py`, `check_graph.py`,
  `test_check_graph.py`: exact feature/target/input/patch/profile gates and
  graph/artifact tests; historical modes retain their meanings.
* `run-node.py`, `test_run_node.py`: separate closed guarded mode using the
  already verified fixture bootstrap and owned tasks/children; fixed TCP
  protocol, canonical two-script no_bundle deployment, phase/barrier/scenario
  oracles and bounded report validation. No new interpreter imports, fixture
  dependency or arbitrary controller program.

There are **14 outer paths**. This plan does not authorize a fixture proxy,
general transport library, new source-import mechanism or build helper path.
Actual new file/fixture byte hashes are frozen after source authoring, then
independently reviewed before executing anything derived from them.

## Focused acceptance matrix

Closed scenarios below may share source fixtures but have distinct receipts.
For matrices, the controller enumerates the finite cases in the reviewed
profile; arbitrary command-line phase names are not accepted.

| Scenario / matrix | Required positive and negative evidence |
| --- | --- |
| `cold-sync-retire` | Same-process actual owner, canonical ownership/restore/VFS, real SQL write + sync/output gate + VerifyOwnership, real local TCP frame, driver/finish/stop joins, both release edges and dead invocation Weak witness. |
| `restore-next-process` | New process/fresh data directory restores the previously acknowledged fixed row through canonical bucket path; unchanged fixture/descriptor, fresh zeroed Wasm before complete touch. This is local restoration evidence, not 15-second HA qualification. |
| `two-slots`, `same-tenant-busy`, `retiring-busy` | Third tenant refused without construction; same tenant refused in every non-Free state; Wasm release alone does not free execution; later admission works only after full disposal. |
| `descriptor-closed` | Boot/reload produce zero guarded pool/Worker constructions; ordinary internal/forward/DO/service/RPC/alarm/queue/loader/facet/WS/wake/resume paths refuse before guarded activation. Invalid bytes/version/class/bindings refuse. Ordinary primary still works in ordinary-mode regression. |
| `binding-matrix` | Wrong owner identity, epoch, core request, invocation generation, descriptor and zero/missing/drained runtime generation refuse at each owned boundary; no replacement adoption. |
| `sync-held-matrix` | Independently hold before enqueue, after enqueue, at resident dispatch, before actual proof, after actual proof, and before processed/inactive completion. Original owner/slot stays live; handler waits without deadlocking host loop. |
| `sync-abandon-matrix` | Original expiry, reply loss, parent cancellation, swap and actor loss at the above boundaries never reactivate or settle successful sync; physical work remains joined or quarantined. |
| `failed-turn-gated` | Fixture writes then throws; canonical failed-in-turn ticket still gates the refusal; no bypass through an early error response. |
| `reload-held-matrix` | Reload before construction, during sync, before output and during retirement. In-flight original generation stays bound or is canonically cancelled; no fallthrough to current. |
| `bucket-readiness-refusal` | Actual probe unknown/transport failure and fixed test HTTP contract violation never produce guarded readiness; positive probe uses the same real Bucket. |
| `proof-held` | Actual AwaitDurable then ownership verification must settle before publication; held/failed work cannot be replaced with startup proof or local SQLite commit. |
| `tcp-write-held`, `tcp-disconnect` | Actual socket poll_write Pending, frame/Request/activity/slot live; release completes exact bytes, disconnect physically closes before retirement. |
| `cancel-write`, `cancel-flush` | Actual core CancelCellActivity notification while actual socket write/private test-writer flush is pending; no later publication, exact once finish, no early credit. Report transport distinction. |
| `deadline-matrix` | Original 15-second deadline at header, route, restore/build, sync, driver, write, flush and retirement. No renewed user deadline. Late native work cannot publish and stays owned until actual disposal or reported quarantine. |
| `effect-abandon-matrix` | Drop caller/reply/actor at restore, start, proof, ownership verification and stop; all actual handles/payloads remain charged. A late successful native result is not a fabricated actor acknowledgment. |
| `stop-after-memory` | Normal Worker/probe destruction followed by actual stop error/held disposition; memory credit returns, execution stays Retiring/Quarantined, no implicit restart. |
| `panic-matrix` | Construction, task, turn, payload destruction and teardown uncertainty preserve A2 taint/quarantine, including original owner. No catch-all Err release. |
| `shutdown-complete`, `shutdown-cut` | Stop admission; all three host-service loops still progress; both semantic predicates require guarded retirement. Complete branch observes required joins; cut branch reports incomplete and retained state before hard exit. |

Use barriers and exact owned events, not sleeps, for ordering. Finite real clock
deadlines are assertions only where time is the behavior under test. At every
pending boundary assert both concrete ownership (Weak upgrades/actual handles)
and accounting counters; at release assert physical postconditions first, then
credit. Test hidden admission failure followed by a benign/domain error cannot
clear sticky original Request failure. Test never-started tasks, completed
JoinHandles retained by the runtime, panic during payload drop, channel closure
and full bounded registries. Record actual payload layouts and operation
high-water counts; no sizeof-wrapper or counter-only claim of complete RSS.

## Build, review and acceptance gates

1. Rebind to accepted A3a source/lock/profile; inventory source and unchanged
   A2 native assets. Root and independent plan review this file plus inputs.
2. For each source group freeze exact changed paths, owned APIs, actual layouts,
   fixture hashes and tests. Perform author review followed by independent
   subagent source review before root's first execution. Preserve preimages.
3. Root runs meaningful pure/model/ownership tests under the reviewed native
   target and private feature. The new pure core event is tested through the
   celld private test module; upstream logic has lib test=false, which is not
   silently changed. Keep existing ordinary gate/swap/drain regressions and A2
   lifecycle qualification. A2 fixture/program identity must remain unchanged.
   The concrete command shape for the new private test module is:

   ```text
   cargo test --locked --offline --manifest-path <outer>/Cargo.toml --no-default-features --package foks-celld-guarded --package celld@0.6.2 --features foks-celld-guarded/node-qualification --lib -j 2 --target aarch64-apple-darwin guarded_execution::node::tests:: -- --list
   cargo test --locked --offline --manifest-path <outer>/Cargo.toml --no-default-features --package foks-celld-guarded --package celld@0.6.2 --features foks-celld-guarded/node-qualification --lib -j 2 --target aarch64-apple-darwin guarded_execution::node::tests:: -- --test-threads=1
   ```

   Use the accepted absolute Cargo path/environment and selected stage manifest.
   Listing must contain the exact reviewed test inventory, including the pure
   logic event cases, before executing the same filter. Only celld has a lib
   target among these selected packages; an unexpected target or zero tests is
   a stop gate, not a reason to widen to upstream --all-targets. A rejected
   package/target selection needs a narrow reviewed command correction. Keep
   `celld_internal_tests` unset for this new private module; existing relevant
   upstream regressions requiring that cfg get a separately frozen finite
   command/filter, not an implicit full-suite resource expansion. This module's
   B1/B2 listing contains core/model/native ownership components. B2's approved
   local managed-DB/VFS adoption bridge additionally invokes actual Engine::init,
   WorkerBuild and WorkerDestroy on one retained serial test runtime; that
   narrow exception does not exercise the canonical NodeDriverWork or reset
   the static tainted ledger.
   V8/global/quarantine cases run serially in fresh actual child processes in
   B4, with V8 initialized by main before Tokio; test-threads=1 alone does not
   reset a singleton or make repeated global installation valid. Earlier stages
   also rerun the applicable accepted A2 lifecycle child cases under new
   stage-bound input evidence, without calling that A3b node qualification.
4. Review actual locked/offline metadata/tree for default, A2 and node modes,
   allowing only the two exact celld-to-local-FOKS edges in node mode. Prove
   same one-member outer workspace, engines and checksums; no native metadata
   SQLite provider enters the FOKS subclosure. Review any legitimate lock prune
   or selected-edge delta before compiling.
5. Build and Clippy the actual selected celld bin using the accepted two-package
   outer command plus `--features foks-celld-guarded/node-qualification`;
   --locked --offline, aarch64-apple-darwin, jobs 2 and existing network-denied
   build wrapper. Require exact main.rs compiler artifact, selected features,
   executable path/hash and Boolean successful build-finished. Keep the accepted
   target<=4 GiB/free>=2 GiB envelope with reviewed reuse admission and whole
   process-group cleanup. Root alone holds the Cargo slot.
6. After source/tests/profile/network policy gates, execute closed real-node
   scenarios using actual compiled binary and same verified fixture. Preserve
   complete bounded inputs/argv/logs/phase results and normal versus quarantine
   observations. Outer cleanup requires complete group disappearance and all
   owned storage/helper tasks, not just child returncode or runner assertions.
7. Independent review binds the final source, executable, runtime reports and
   cleanup evidence before each commit. Markdown remains unstaged. Any missing
   physical observation is incomplete, not a reduced success criterion.

The remaining work after A3b includes actual FOKS Wasm growth/memory policy,
the real SQL/metadata adapter and its atomic ownership fencing/durable ACK,
full deadline propagation through its storage operations, retained receipt
retirement, simultaneous tenant recovery and measured density. A2/A3b fixture
success does not establish zero-loss production replication, AWS AZ timing,
whole-node safe unwind, or termination of every outstanding object PUT needed
for reclamation. These remain explicit later decisions and qualifications.

The exact first implementation group is now specified in
[the B1 foundation plan](HOSTED_CELLD_NODE_FOUNDATION_PLAN.md). Its direct and
independent plan reviews passed after clarifying role-based request ownership,
rejecting auxiliary work from an already finished output-pinned parent, and
preserving the precise generation-lock and event-clock semantics. B1 remains
gated on acceptance of the ordinary-node baseline and rebinding its inputs.
