# Celld guarded node lifetimes (B2)

Status: B2 accepted and committed as
`d22ff3deedf0f0a87f635aec2317a87410e39110` after reviewed planning, two
source-review rounds and root/independent results review. All 26 preparation
tests, 53 native component tests, default binary checks and 28 fresh-process
A2 lifecycle scenarios pass. The commit contains seven implementation files;
all 41 Markdown changes remain unstaged. B3 implementation has not started.

Stopping point requested by the user: finish and commit B2, then stop with a
B3 handoff. Do not begin B3 implementation in this run.

The accepted starting point is B1 commit
`23039e209d1b3b129a274dd65fe07f0abfdba8e0`, described in
[the foundation plan](HOSTED_CELLD_NODE_FOUNDATION_PLAN.md). B2 owns the
actual work behind an admitted invocation. It does not enable a frontend,
mint Ready, or qualify production metadata, host/AZ recovery or tenant density.
The production metadata backend decision remains deferred. The first hosted
milestone remains a restricted profile; native-valid records outside that
profile may be rejected without changing FOKS/Go wire encoding.

## Purpose and invariants

A completed reply, cancelled await or timeout does not prove that the task,
native child, storage handle or final result has been destroyed. Every admitted
operation must retain its original owner and actual tasks through their joined
completion and result disposition. The existing accepted timestamp and
15-second deadline travel through the work; helpers cannot start a fresh
deadline or acquire a new Request to escape a failed budget.

Use a process-owned registry with two invocation slots. Each invocation has
eight forward roots, eight one-use cleanup roots, one native child per root,
and four prepaid controls. Root handles, native handles and completed but
unconsumed results remain in fixed slots. The maximum is 32 root/child handles
per invocation. Dropping a receiver or wait ticket leaves its actual task and
result retained. Unknown result types, stale sequences, panic, poisoned
disposition and lost completion ownership fail/hold the original record.
Weak back-references prevent successful draining from depending on a cycle.

The existing A2 two-slot capacity ledger is the sole physical capacity
authority. Each invocation owns one consumable A2 lease. Runtime and control
witnesses cannot release it. Actual canonical database path, VFS, epoch and
generation are bound before activation; no private temporary database or
ordinary pool fallback is accepted. The generation pin and reaper share a
single lock order. New-pin admission is nonblocking; explicit release of an
already-owned pin may wait on its short metadata critical section. A retiring
generation cannot receive a new pin.

The following work remains deliberately unsupported: unsealed node-log
recovery, guarded generation swap/reload, generic object readers that hide
detached helper threads, mutable bucket/shipper registration, and unqualified
alarm or WebSocket execution. A marked unsupported effect fails closed before
ordinary detached execution. Normal canonical no-log/absent/sealed ownership
paths remain available.

## Actor and activity boundaries

Classify each borrowed effect before cloning its data. Named factory types
make actual capture/future/result layouts measurable. The guarded registry
owns the real task and channel; guarded work never enters ordinary
StepOutput/FuturesUnordered delivery wrappers.

The production actor directly polls the registry alongside ordinary mailbox,
completion and timer inputs. Completion consumption applies exactly one
synchronous core transition and retains its effect batch. Dispose the old
result/channel and retire that exact root sequence before executing the
successor batch. This allows legitimate sequential activation/proof work
without prematurely reusing a slot or clearing a newer task.

An opaque GuardedRequest consumes an already-admitted envelope into its exact
registry before RequestAt. Only component fixtures can create it in B2.
Primary and auxiliary route/output results use fixed completion cells, not
ordinary pending maps or new unbounded mailbox nodes. Existing ordinary
ActivityGuard behavior remains unchanged.

The guarded local-request wrapper preserves answer-ticket checks for failed
handlers as well as successful ones. The full native driver directly polls
that wrapper, retains the answer/activity and joins background/final-abandon
work before its own completion. No timeout may drop that complete driver as
a substitute for joining it.

Cancel, primary finish, auxiliary finish and final retirement use four prepaid
controls. Finishing observes the actual alarm before ActivityFinished.
Processed is distinct from Inactive: only actual core request inactivity
satisfies the finish wait. Observe inactivity after every real transition,
including a later output Release. A lost reply schedules retained cleanup.
The auxiliary sync route validates the actual Worker/IoContext parent, storage
sample, scope, epoch and generation; it is single-flight. Output pin/forced
swap and stale-parent cases refuse without creating another authority.

Output proof ownership comes from the exact request and the actual core
transition's returned operation. Equal storage positions do not identify an
activity: two writes can create distinct proof operations, while an observed
read can join an existing barrier without creating another proof root.
Sequential auxiliary sync calls may reuse their fixed activity cell only
after the previous activity is Inactive and its result has been disposed.
Monotonic activity and control identities reject stale consumers without
replenishing cleanup credits.

## Root and cleanup roles

| Forward root | Work |
| --- | --- |
| F0 | Hydration, including its joined open/step native child |
| F1 | Sync, retaining native capture after a caller times out |
| F2 | Compaction and its scoped lower-host native callbacks |
| F3 | Complete driver and owned storage-sync OpFuture |
| F4 | Primary durability and ownership proof |
| F5 | Sole auxiliary proof |
| F6 | Sequential ownership resolution, restore, build and publication |
| F7 | Exact wake maintenance |

| Cleanup root | Work |
| --- | --- |
| C0 | One terminal durability ticket |
| C1 | Continue only already-issued primary/auxiliary proofs |
| C2 | Stop, driver dependency, native Worker destruction and local retirement |
| C3 | Actual owner release after successful stop |
| C4 | Finite canonical peer-adoption attempts |
| C5 | One exact final wake state after earlier wake work joins |
| C6 | Stop/join storage background work, final sync/snapshot/close/VFS retirement |
| C7 | Cancel and join the original driver/background disposition |

Cleanup admission uses the already reserved 520 KiB: eight 64-KiB root
partitions plus four 2-KiB controls. Each cleanup root has 48 KiB for its
typed wrapper, 8 KiB for owned capacity/channel debt, and at most sixteen
512-byte native child wrappers. Those sixteen credits are shared across all
child kinds in that root and never replenish. Failure/expiry permits only
already-reserved cleanup; it grants no renewed publication deadline.

The dependency order is C1, activity Inactive, one final eviction, C0 and C2.
C2 joins C7 and C6; neither dependency can wait on C2. Release follows actual
stop, then adoption. Final wake has its own exact binding. Bounded eviction
abandon signals only the already-retained exact operation. Swap refuses
before mutation. All guarded stop errors, including an actual bounded
eviction abandonment, hold/quarantine the invocation in B2. Ordinary
RuntimeStopFailed/Cancelled restarts an epoch; B2 has no qualified restart
after dedicated Worker destruction, closed background admission and spent
C2/C6 credits. Only actual stop success reports RuntimeStopped. The ordinary
celld path remains unchanged.

## Replication and wake details

Attach EpochJobs before any restore or activation work. Validate the actual
immutable AmazonS3/static-credential construction and registration.current
being None; an expired registration Weak is insufficient. Restore returns the
real path, restored flag, VFS and joined alarm read. Start binds the actual
current generation and retained successful ownership-CAS freshness decision.

Hydration, sync and compaction own their actual lower work. Compaction receives
its own F2-scoped host rather than reusing an expired activation context.
Native payloads move only into same-owner fixed resource slots after actual
join. A normal joined read_file NotFound retains the canonical remote-cache
fallback; admission failure, panic, unknown disposal or terminal lower-host
error fails/holds. Ordinary process close cannot snapshot/unregister guarded
cells outside their reserved retirement path.

Wake maintenance reads the dedicated runtime's actual snapshot and uses the
canonical wake operation through a context-bound AlarmHost. Even an alarm-free
snapshot can require retiring a source revision, so absence of an armed alarm
does not skip its I/O. F7/C5 can use only an already-issued proof whose stored
canonical position covers that source and whose exact ticket is covered by
actual synced_seq. This lookup creates no request sequence, task or child.
Missing proof holds the origin. The terminal EnsureDurable API currently has
no canonical position sample, so its coverage is zero; an LTX TXID must not be
substituted. Nonzero wake sources require a prior primary/auxiliary proof.

Local join never proves that remote PUT attempts have terminated. FOKS S3
object reclamation remains report-only. S3 is treated as durable and reliable
under the selected strong-consistency model; AWS timing and AZ behavior still
need a later real environment.

## Retirement and accounting evidence

Final retirement requires all conjuncts: response disposition, full driver and
background/final-abandon join, primary/auxiliary Inactive, actual Worker/Probe
destruction, all root/child/result disposal, exact local storage and ownership
disposition, and no remaining generation/resource alias or taint. B3 will own
the actual output-disposition boundary. Dispose the final A2 control witness
while capacity is Retiring, retire the original B1 owner, then mark A2 Free.
Any failure retains Retiring/quarantine; it cannot roll back to available.

Measure 16 fixed layouts, 36 actual named constructors and four control types.
Actual closed root/native result enums and Option<enum> channel payloads must
be measured; a smaller per-family result would undercount. Existing B1 layout
schema remains unchanged, though actual sizes can grow.

One FOKS_B2_LAYOUT record reports type measurements and separately labeled
source-derived bounds. Root capture+future+result must fit 48 KiB; the separate
8-KiB owned/channel partition includes its lifetime allocation debt. Native
capture+result+channel must fit 512 bytes; directly visible large resources
must have a separately admitted same-owner slot. Inline observations are
nonadditive. Tokio channel bounds use the pinned source/layout convention,
not a claim about private allocator size or RSS. V8/SQLite, lower opaque
client buffers and scheduler internals remain named runtime overhead; these
wrapper limits do not establish a whole-process memory bound.

## Execution gates and commit boundary

1. Preserve accepted B1 profile, series, source manifest/tree and native asset
   receipt. Use a separate ordinary author copy; never edit prepared source.
2. Implement the reviewed 21 Rust paths and seven outer tooling/profile/patch
   paths. Append 0003 after byte-identical accepted 0001/0002. No Cargo/lock,
   dependency, program identity or public CLI change.
3. Complete direct source review and independent cross-author source review.
   Resolve findings and freeze exact hashes before candidate imports or tests.
4. Reconstruct strictly from the pinned upstream archive and patch series.
   Verify the final inventory, unchanged native assets, exact accepted lock
   and unchanged dependency/engine graphs for all three modes.
5. Run the 21 existing B1 plus 32 B2 tests serially on one retained test runtime.
   Cover effect inventory, original origin, saturation/cleanup, actual handle
   and native-child ownership, result disposal, panic/unknown quarantine,
   stale epoch/generation, actual activation binding, parent/sample binding,
   finish Processed/Inactive, lost delivery, failed-handler tickets, registration
   immutability, hydration/sync/compaction joins, bounded background queue,
   snapshot/close failure, retirement conjunction, original deadline and no
   readiness/frontend surface. Do not manufacture native success or Ready.
6. Validate both bounded raw layout records and their complete numeric
   inventories. Run strict Clippy and native binary/CLI probes for default/node
   modes, plus the unchanged 28 fresh-process A2 lifecycle scenarios.
7. Review execution evidence, preserve failed runs and any narrow corrections,
   then commit implementation only as B2. Root owns all candidate execution,
   formatting, builds and Git mutations; source authors do not run candidates.

Remaining work after B2: B3 actual main-pump/task ownership, absolute request
deadlines, bounded socket transmission and consuming output retirement; B4
full guarded-node failure/recovery qualification. The earlier local canonical
celld/Moto handoff result is not proof of abrupt host/AZ recovery within 15
seconds, simultaneous recovery, broad tenant density or production fencing.

## Review record

The frozen accepted-input rebind, constructor ledger and revision-b API
handoff passed direct and independent source-only review before authoring.
The authoring-go receipt is
`96fdb353c24bc81e525f308a5721fd6e8350f542c31dbed68d5c6dd6f479e8a1`.
Implementation clarification C records the exact abandon/wake/layout seams;
its SHA256 is
`3a009b77eee1e0b1bdab2164df897d067400fe11cbb8cce2159db9f4f6259dee`.
Independent review of C passed with receipt SHA256
`835a40a0328c92c6777298818e396d3b5031876e0aea9220b04ca63c401bffbd`.
Clarification D records held stop errors, actual channel/wrapper measurements,
nonissuing inline delivery and callback-panic disposition. Its SHA256 is
`082c7d74b3ab8c058a1524066b2bb7eb9a33bc70be41908e963b15de86d0661f`;
independent plan review passed with receipt
`853f4b89629278fe2042b83964f53cf6299d57e159e3820e69304781dd90b6b4`.
The four-file actor component passed author and independent source reviews
at source-a, manifest SHA256
`36ad88140d7a5079b595952ac936301103305d67f398c29e5ad7d23d1a7f807c`.
Independent receipt:
`383e961f89dea72600a8ec421dee4544e25242b32da5e9c22f512fa43ea3a0d5`.
That review corrected the actual selector fixture's request-consumption
witness and ensures blocked reads are released and joined before propagating
fixture assertions. Its cold invalid-Publish, nonresident-gate and missing-
runtime assertions are early-refusal checks; the outer bank tests must still
cover the complete binding and retirement conditions.

The four-file Python tooling component passed its author and root source
reviews, manifest SHA256
`698425cbe70b940c66188e85291070f68aa66ce574881fe22a9a84b3573986de`.
Root review receipt:
`c282c97dadbc6241932a5671f8b5248c3c280339064afb58eab8228073594b3b`.
Its cross-author review and complete Rust integration reviews remain pending.
No B2 candidate has been imported, built or tested at this checkpoint.

Offline native component tests cannot qualify successful S3 activation:
canonical fresh activation writes a real remote marker. Local attachment,
managed database, native join and VFS tests must distinguish those component
states from successful restore, Ready or remote durability. A background
error or expired operation can conservatively retain capacity even after its
native child has joined; that is quarantine pending process recovery, not
successful reclamation. Both full implementation reviews remain required
before testing and committing B2.

Clarification E binds output proof operations to the exact activity through a
synchronous core transition, before executing its returned effects. Plan
SHA256: `ce98944ae888bbe2669b515b524eceb72238cb6e4197250a1fd58a683fe9b230`.
Independent plan review passed with receipt
`c5ab5871bec22f0908115e048e82effb3ea3f3713989dd95192629e5bfc88a4c`.
The actor hook and corresponding bank changes require renewed source review.

For the remaining source authoring, root owns `runtime.rs` in addition to the
four actor files; the integration author owns its bank bridge, JS sync path,
tests and packaging. This is a disjoint ownership transfer within the same
reviewed roster. The source-only review found a lower compactor panic
being converted into an ordinary retry result; guarded compaction must fail
and retain the original owner on that result, including a concurrent stop.

Clarification F adds `asyncrt.rs` as the twentieth existing Rust path. Its
private guarded enqueue accepts the already charged, measured OpFuture, so
the ordinary helper cannot hide a second boxed conversion future. Ordinary
enqueue, handler lifetime and IoContext attribution remain unchanged.
Plan SHA256:
`c0c0d86f2c71ec35e2aff69c4f9256107adb3c728fd3e610502a8f6b7051ed6c`;
root review `8b02ce533612aae6eed96457a52582c76527fa6fa853e66ce9b8987e579ebd4f`;
independent review
`27125023e8fd5b1757c92d10038dbf3274a87407b8ca919325ecd3713e45a068`.
Existing TLS operation queues and driver scheduling remain named native
runtime overhead; the constructor measurements do not claim a total heap bound.

The driver-disposition seam passed independent plan review with receipt
`68b101c6313175b7466da44e5be3114c9bee8c65ed2dc992b8843a21c34df619`.
After its actual handle joins, F3 retains its answer and activity while sealed
async disposal continues already-issued proofs through one-use C1, discards
the answer, and waits for ActivityFinished and Inactive. C7 can reclaim an
abandoned consumer and uses the same disposition. Only then is F3 reusable.
B2's component consumer discards output; B3 must supply socket disposition.

The four-file tooling source-b completed author, root, and independent source
reviews. Its focused 18-test Python run passed (11 preparation/layout cases,
7 graph cases; 5.732 seconds of tests). The direct supervisor receipt is
`c5a3c0228bfc0eacd9357705ca93f8db4273889b983011fdfd20905a9a68c765`;
all four frozen sources and ten bound inputs were unchanged, and the final
process observation found no descendants. This is tooling validation only,
not a Rust layout measurement, native lifetime proof, or recovery result.

Runtime source review now covers actual F3 join before response disposition,
in-isolate original-origin rechecks for Op/Poll turns, native Worker/SQLite
shutdown, and same-origin final wake observations. A supplemental lexical
path test cannot establish adoption: the component proof must retain a real
local managed database and VFS through actual Worker adoption/destruction and
C6 retirement. Its local fixture must not imply successful remote restoration.

The local adoption bridge and its runtime counterpart now have both source
reviews. Runtime source-e binds the real worker's installed SQLite context
for alarm and storage observations; the prior final alarm read had used only
the Slot lock, which does not install that context. The callback uses actual
native WorkerBuild, adoption, an overflow-page read, and WorkerDestroy before
C6. The LTX bridge observes real pager reads and exercises both successful
retirement and a file-disposition failure that must retain the original
ownership. Its deterministic failure branch stops the actual background
roots before changing the path, then joins them through C6.

Runtime source-e independent review:
`671757badd86fcb9995c7e453292077c520543112502eb5a58df434a84d2eba7`.
LTX source-b independent review:
`bf343b29aad05242a5ee0a5de0475c6717e8f22c076cb321a643d71129e29e15`.
These are source reviews only. The combined bank/JS/test changes, compiled
constructor measurements and Rust component suite remain unvalidated.

The additional 24 shared foundation preparation/graph tests passed against
the same frozen tooling source-b (1.157 seconds of tests). Receipt:
`27ebe605eac04849a57e95d3220f4412ecee4eeef8e980822a8e0b3a146a954e`;
independent result review:
`1a9394344b93bbc4d84bd9b31e35fc7619164e7cf0551069807fcfd1f7042b65`.
Together with the 18 new tooling tests, this validates the shared Python
preparation and graph paths. Rust source review found an additional task
publication race: native work could start while its publisher still held the
invocation-state mutex and fail its own nonblocking identity check. Its fix
and a deterministic regression remain prerequisites for native validation.

Clarification G preserves the distinction between component tests and complete
driver/socket execution. The F3 test exercises actual retained task handles and
result disposition; full NodeDriverWork execution remains B3/B4. Sync tests use
the production reservation predicate without creating native publication. The
failed-handler test uses the real guarded local-request wrapper with an already
admitted activity. Plan SHA256:
`a14e66a1a27c73579c2b65c99a6d34957c6e37fee6d20df1c96d6c587554a739`.
Its root and independent reviews passed. Runtime/actor source-g independent
review: `a0527b7074303f3e3d1f4276d29b35b7c4852edf8ab029655d406eeda2f7f6be`.
The intermediate source-f review was withdrawn after cross-file review found
that a removed forwarding method still served the gate dispatcher; source-g
restores it. No Rust candidate was executed with that defect.

Clarification H now addresses both task publication and later contention in
already-admitted bookkeeping. New-work admission stays nonblocking. Short
internal observations, completion, owned cleanup and proven retirement must
not fail merely because another admitted task holds a bookkeeping mutex.
Callbacks, notifications and concrete payload destruction move outside these
locks. The actual joined task's result uses non-cooperative `try_recv`; Tokio's
ordinary oneshot poll can return Pending solely because of its cooperative
budget. A private raw join seam moves panic payload destruction outside the
invocation lock, after the original failure is latched.

H revision-i plan SHA256:
`cdf271e462fcd89ed1549de986cd4bf5c31ebbaeb65c9daa1c99b9f21de14021`;
root review `c9ae534b49e01b5c9bcb8a05712c1228c683eba774016e771464a513bdb0cd45`;
independent review
`743b6ebb015fff2d2513d0e45a89879e4d50bf7df70ff4ab99c5e4fb966bbbc2`.
Implementation and source reviews remain pending. Deterministic contention
tests observe the actual WouldBlock boundary before releasing a held lock;
they must release barriers and join helpers before propagating assertions.
No runtime budget, wire format, source-path roster or test count is expanded.

Runtime/actor source-i passed both reviews. Its local request builder is
test-only until B3 supplies a frontend, while the actual completion type and
dispatcher methods retain their production feature gates. Independent review:
`605a5fb7828392af1a5cf45bb3665129c4119a6b134315096c56617740e9e530`.
The six-file JS/dispatch bridge also passed both source reviews; these reviews
do not establish full frontend, socket, or recovery behavior.

LTX source-c implements H's admitted synchronization and post-lock deadline
checks. New registration still refuses contention immediately. Its tests hold
the actual registration and epoch-state mutexes and observe the production
WouldBlock boundary before releasing them. Root source review:
`4533093c6a600f7845b76924b626d2c643bb5013a2bfee66ff6ab2d19237da9a`.
Independent source review also passed:
`aca5763862923b674ade31190550a48f30964f18ec80d09a7329b99565f9ecce`.
Combined native validation remains pending.

H's final-owner refinement was reviewed before implementation. A temporary
JobContext alias can outlive physical resource cleanup. The existing two held
cells distinguish disposal, pending original-owner retirement, and failure.
Payload disposal, A2 detachment and generation-pin release occur once; only
the final owner-uniqueness observation is retried. The slot stays occupied and
A2 stays Retiring until the original owner is actually consumed, before Free
is published. Persistent aliases can cause repeated actor observations; B2
does not qualify their CPU cost or worst-case recovery time. B3/B4 must cover
that scheduling behavior in full-node tests.

The final bank review found two further H gaps before execution. A forward
check must recheck its original Control after an admitted state-lock wait.
Actor input scans must defer on bank/state contention instead of acquiring
those locks synchronously. Once terminal selection has extracted a concrete
owner under its occupied Disposing marker, synchronous physical disposition
and the short final metadata reinstall remain necessary to retain that owner.
This exception does not establish a nonblocking actor or a latency guarantee.

## Native validation checkpoint

The first native attempt refused before compilation because free disk space
fell below the unchanged admission floor. Root reclaimed only regenerable
Cargo test/library artifacts from the main workspace; all evidence and the
guarded target remained. The next attempt reached Rust and reported four
E0275 Send-trait recursion-limit errors. Its owned process cleanup completed.

A twice-reviewed amendment adds a feature-scoped crate recursion limit of256
in existing celld/lib.rs. This raises compiler proof depth only; it adds no
unsafe Send/Sync claim and changes no runtime allowance. The exact B2 patch
roster now contains21 existing Rust paths in the same286-file tree. The first
20 Rust postimages remain unchanged. A review caught and corrected the new
lib.rs patch-count assertion to3 (A2,B1,B2) before testing.

Source-b seven-file implementation manifest:
`73b08ad7cb3810753b4f3583b543bf2cc5a64f9f7a8eb3c1efbe0e94394d4055`.
Independent source/package review:
`52244e7f33d9c04cf33c5325a25ab7722e24b18e1b9447e859aa33fa81ebadb3`.
The26 preparation tests pass with source-d tooling; receipt:
`634abba8f79f55d29b8ceac48835c26f263e2850c8172340c5ebc11356a18f99`.
The failed source-a tree, source manifest and stage asset receipt were preserved
before restoring the exact accepted B1 predecessor. Fresh source-b preparation
then passed; receipt:
`c455bc6eb4af9b19e001866164ce9ac2df6f181392cca140ecfbd6cf3fc887e3`.
Native build, strict Clippy, component/layout tests and A2 lifecycle results
remain pending at this checkpoint.

The source-b build then caught an E0505 borrow error in poisoned-state
retirement. Source-c moves the process-failure transition after the borrowed
match temporary and bank guard scopes, retains the occupied slot, and adds a
deterministic real-state poison regression. A review corrected its expected
terminal result to Cancelled, matching the existing stopped-process behavior.
The correction passed both source reviews before reconstruction and execution.
Source-c's binary build, configured binary Clippy and three CLI probes passed.
Test-list compilation then caught three references to the wrong futures crate.

Source-d changes only those test helpers to the already-declared
futures_util::task::noop_waker_ref, with rustfmt reflow. Both reviews passed;
independent source/package/native binding review:
`0d2e2d985c3a1e047fe57d625f4f1315c31a683e3953b889840ffabe005f8c3a`.
The source-c tree, manifest and stage assets were preserved before restoring
the exact accepted B1 predecessor. Source-d preparation passed with receipt
`7373c0112d9db1473e90f6bea0d46e79836dc60134b87e0ba1c3c29b5827ac7d`.
Its first complete native run passed46 of53 tests. The layout test reveals
that the first constructor, root/owner_read, exceeds the8192-byte typed
payload ceiling. The same production admission refusal explains five actor
fixture failures before their memory-adapter barrier or restore path can run.
Measure all36 layouts before deciding the correction; do not change those
fixtures to conceal an admission failure.

Source diagnosis also identifies two fixture expectations that need precise
correction: a real RestoreFailed core result is delivered externally as
NodeFenced once the original control has failed; a local file-deletion error
injected after successful Db closure and VFS unregistration must observe the
pager absent while the epoch and original capacity remain held. Both need
reviewed source changes and fresh validation. Failure receipt:
`316c7c82600ce46933417974d08769cdbe808dd55fe7e33167b6d6329e8bc31a`.
No failed run was discarded or silently relabeled successful, and no runtime
allowance has been increased.

Source-e keeps production behavior and limits unchanged. Its twice-reviewed
test changes emit all36 constructor measurements on rejection, distinguish
stored/core errors from fenced delivery, retain unknown/unsubmitted work
during failed fixture cleanup, and assert the actual local-disposition phases.
The close-failure test now passes. All remaining six failures are consistent
with the original owner-read admission refusal. Its exact stored route error
is ResolveFailed, before the intended restore path is reached.

The rejected layout record measures14 oversized constructors. The maximum
typed sum is37,288 bytes for restore; the maximum cleanup typed sum is20,072
bytes for wake. All native child totals remain at or below328 bytes. The
record is explicitly failure evidence, not an accepted layout ledger.
Diagnostic artifact SHA256:
`4a02e9b5d9c2c20644270a3686d02fcc0c427ce120aded1f21b1763490e31712`.
Failure/evidence receipt:
`15e259e1e2558c0b410b65a7128cab29c7aaf3efade3d03af83f0e8e66c1d1a0`.
Source-e and its diagnostic binary remain preserved for review.

The reviewed budget amendment keeps the two invocation slots, original deadlines,
8-MiB invocation debt cap, all task counts, native-child credits and 8-KiB
owned/channel partition. It raises Rule R's typed allowance to48 KiB and its
combined ceiling to56 KiB. This explicitly includes the separately inventoried
single-flight storage-sync future. Each cleanup root then prepays64 KiB,
including its sixteen512-byte native children; eight roots plus four2-KiB
controls total520 KiB. Rule I stays at8 KiB and native children at512 bytes.
The change requires coordinated admission/credit constants, measured-policy
checks, and a stage-aware foundation-record validator; the historical B1
record still requires200 KiB, while B2 requires520 KiB. Do not accept
either value indiscriminately. Root and independent plan review passed for
`/private/tmp/foks-celld-b2-layout-budget-proposal.md`
(`28bccfde6b8d880afca922988747b4a1fc21e3b4aad4b52fcc909b0ed115549c`).
Two source reviews precede candidate execution. These are bounded debt allowances, not
new physical allocations or an increase to the invocation's total debt cap.

## Accepted B2 implementation and qualification

The committed implementation is source F, based on accepted B1. Its 286-file source
inventory contains 7,101,033 bytes; inventory SHA256 is
`b6f56b07244b18685a4977e0da68e53a37bfc0053bd4b9412933c46e050998db`.
The third patch changes 21 existing Rust paths and has 635,255 bytes:
`b00759fbb721bde82971488e2f13f4d35d37a21a0b7309810b773caf977aa832`.
The series hash is
`f7e5d4dc073cabbc7dcdec17e798163881f04da2a60952c8dae53f960577622e`;
the profile hash is
`b005ea3af86f1a09243184f359321d51ba1f81caf4aff34d22e377de32d359d3`.
The accepted 0001/0002 prefix, Cargo lock, pinned source/V8 inputs, program
identities, 53-test inventory and original 8-MiB invocation cap are unchanged.

Root and independent reviews cover the final three-file Rust budget delta,
two-file Python validator/test delta, generated package and all 153 immutable
execution inputs. Strict preparation reconstructed the complete source from
the pinned upstream archive and reviewed patch series. The new native run
passes 53/53 tests with no ignored or filtered cases, and emits one B1 and one
B2 record bound to the same complete stderr. These measure nine B1 layouts,
16 fixed layouts, 36 actual constructors and four controls. The largest typed
R row is restore at 37,288 bytes, below the 49,152-byte allowance. Cleanup
precharge is 532,480 bytes; native and inline limits remain unchanged.

Evidence is under the validation worktree's
`tools/foks-celld-guarded/runtime/evidence/`:

* Preparation: `b2-direct-prepare-1046792589992416/receipt.json`,
  SHA256 `95266f664d3949d2d3b8a974f5873752d318a773d8ed8853cfc4e2d3dbac33fd`.
* Preparation tests: `b2-tooling-compile-source-f/receipt.json`,
  SHA256 `aab01bbe3ffe5550b0914cf5ccdfa6f0871d6971923b72197cc2ca5ce20cc646`.
* Native component qualification: `b2-direct-node-lifetimes-1046800856526958/receipt.json`,
  SHA256 `b521dc60a10657e3b44523fe62bbc7e220aac8bcfdb5edd3e098588ebc3cecdd`.
  Detailed build, graph, probe, test and layout evidence is `lifetimes-kfrlelpn/`.
* Default regression: `b2-direct-node-lifetimes-default-1046907265854166/receipt.json`,
  SHA256 `b9cd808755eaf6088ec12e29b8028cbb1963e3de9c00db301188a95221eb3d77`.
  Detailed evidence is `lifetimes-0hxicd3j/`.
* Lifecycle regression: `b2-direct-node-lifetimes-lifecycle-1046971523138125/receipt.json`,
  SHA256 `7bb4df586785c258235163547f0b2cb4b98bdf18d84a11b3d4282d7af115ed94`.
  Detailed evidence is `lifetimes-pytyxcj9/`. One parent integration test
  verifies 28 distinct scenarios, each in a fresh native process.

The final root results review is
`/private/tmp/foks-celld-b2-source-f-results-root-review.json`
(SHA256 `29bf736701b8fef45e0ea845c6c954365060ec0d09a6253b8ac76deab94e104f`).
The independent results review is
`/private/tmp/foks-celld-b2-source-f-results-list-review.json`
(SHA256 `765e5681dd5eee0d8977cdc304d0848d328334641e8068053a17ae1a3c5c9950`).
Both passed before the implementation commit. The commit's seven paths and
bytes exactly match reviewed source F; the main-workspace fast-forward
preserved every previously unstaged Markdown image.

The final native and default images were compressed and verified before any
later mode could overwrite them. Historical source A–E failures and their
receipts are retained; they are not relabeled as successful qualification.
The 16 unchanged graph-tool tests passed earlier; final native modes also
validate their own actual feature graphs. No full FOKS workspace or desktop
suite was run for this celld tooling/patch-only change.

## B3 handoff — implementation intentionally not started

Use [the guarded node plan](HOSTED_CELLD_GUARDED_NODE_PLAN.md), especially
G2/G5, G4 and the B3/B4 stage boundaries. Start from B2 commit
`d22ff3deedf0f0a87f635aec2317a87410e39110` and the full source inventory,
profile, patch series, layouts and test receipts recorded above. Append the
reviewed `0004-node-frontend.patch` to
the immutable accepted prefix and reconstruct from the pinned archive before
testing. Do not make an implementation by editing prepared source alone.
Reconcile the older broad plan's path, patch-size and payload ceilings with
B2's reviewed amendments: B2 includes the actual asyncrt and LTX ownership
bridge. Its governing Rule-R limit is 48 KiB typed plus 8 KiB owned/channel;
each cleanup task prepays 64 KiB and the total cleanup reserve is 520 KiB.
The broad plan's earlier 16-KiB typed/200-KiB cleanup proposal is superseded
for B2/B3. Historical B1 records still require exactly 200 KiB. Inline/native
ceilings and the 8-MiB invocation cap have not increased.
Freeze the exact B3 delta against the accepted series rather than assuming
the older proposed 25-path roster is still the complete input inventory.

The next implementation group has these concrete obligations:

1. Add the closed Action and fixed diagnostic frontend. Capture the original
   acceptance time before header work, use its existing 15-second deadline,
   and retain the same Request through route, execution, output and disposal.
   B2's test-only admission and driver builders do not supply a production
   frontend. No helper may replace an expired owner or renew its deadline.
2. Make readiness depend on the actual Run Bucket passing the positive
   Conformant startup check, canonical recovery, DurabilityOwner, actor and
   host-channel installation. Error or unknown storage behavior must refuse.
   B2 has no Ready producer, and its component tests must not supply one.
3. Own and poll the actual frontend, GateReq dispatcher and retirement work
   through serving, admitted-request pre-drain and semantic drain. Include
   guarded-drained in both Handoff and Preserve. Observe B2's failed-process
   latch. Keep the actual dispatcher task, AppHandle, origin and results
   owned until join and disposal; a local sync-future test is insufficient.
   Retain the separate PublicRequestGuard through physical output and guarded
   retirement in all three loops, so ordinary request-count drain cannot
   finish while guarded resources are still owned.
4. Implement the consuming GatedFrame with the actual socket. Keep its frame,
   Request, activity, generation/epoch and execution owner through local
   `write_all` and `flush`, or through physical transmit-future disposal and
   socket closure. Only then report output disposition and retire F3/activity.
   Local flush does not prove that the peer received the reply. Preserve the
   failed-handler output-ticket checks as well as the successful path.
   Replace the consuming NodeDriverCompletion::dispose seam: B2 deliberately
   drops answer.result and marks output_disposed there. Transfer the actual
   answer and activity into GatedFrame before that disposition; a socket writer
   attached after the existing consumer has finished would be too late.
5. Exercise the actual NodeDriverWork, published Worker/IoContext and live
   JavaScript storage-sync path with the main pump. B2's retained TaskHandle,
   core transition and local Worker/VFS tests establish component ownership;
   they do not execute the complete driver or prove the full terminal
   conjunction. Keep the native/core/full-process positive cases in B4.
6. Specify canonical cancellation and shutdown cuts before enabling them.
   Guarded generation swap/reload is currently refused; any supported reload
   path must come from actual core events and cancellation notifications.
   Address or measure repeated pending-owner scheduling and persistent alias
   behavior before making an availability or CPU-capacity claim.

Review the concrete B3 plan, then review its source twice (second review by
subagents) before execution and its separate implementation-only commit.
The focused validation matrix should include actual TCP write backpressure,
disconnect and cancellation; a separately labeled deterministic pending-flush
writer using the same frame/helper; expired headers, route, restoration,
build, sync, driver and output; positive and refusing Bucket startup checks;
both shutdown modes; and real late results whose owners remain occupied.
Observe actual `poll_write` Pending rather than assuming it from a sleep.
Use a recorded loopback-only policy for the owned Bucket refusal fixture;
the native build's deny-network policy is not that fixture's test policy.

Carry forward the lint scope accurately. B2's configured `--bin celld`
Clippy command with `-D warnings` passes while its dependency library emits
nine dead-code warning groups. Six groups are consumers of the test-only
driver or reports that need actual B3 integration; the others concern retained
ownership and private reason/identity observations. This is not a warning-free
library lint result. Do not remove an owning field or add broad suppressions
to hide the missing production integration.

Continuation uses the accepted implementation commit recorded at the top of
this file, plus the ignored evidence bundle in the main workspace:
`tools/foks-celld-guarded/runtime/evidence/accepted-b2-handoff/`.
Its `inventory.json` maps original paths to content-addressed gzip blobs and
records uncompressed bytes, SHA256 and modes. `inventory.sha256` pins that
inventory; the separate `handoff.json` records the accepted commit and archive
identity. Check the inventory hash, each compressed object hash, then each
expanded size/hash before using restored data. Restore only the selected
structural files to their documented locations; never execute archived scripts
or binaries merely because they were present in the archive. Review and bind
any newly used launcher to its new workspace first.

The bundle carries the pinned upstream/V8 inputs, accepted A3a/B1 structural
inputs, final source/patch/profile, source reviews, final and failed-run evidence,
all four final native images and the unstaged plan snapshots. The current
validation checkout is
`/Users/raymond/Code/foks-rs/.claude/worktrees/metadata-guarded-list-validation`.
Its live prepared source is the validated B2 postimage. Regenerate an authoring
tree from verified inputs when needed; do not use modifications to prepared
source as the implementation. Older `/private/tmp` paths are provenance paths,
not durable continuation prerequisites once the corresponding archive objects
have been verified. A new workspace needs a newly bound execution environment;
restored historical receipts preserve observations but do not qualify changed
paths, tools, machines or capabilities.

The ignored runtime directory is a prerequisite, not checkout contents.
Preserve the accepted A3a/B1 manifests, B1 profile/series/asset receipt, exact
pinned source and V8 inputs, and B2's full source, graph/build/test/layout and
lifecycle evidence with a hash inventory. Restore exact structural inputs or
regenerate them with the historical accepted preparer and verify their pinned
hashes. Historical measured results and binaries cannot be recreated by
handcrafting manifests; if missing, rerun and review the qualification. Keep
failed attempts too. A new checkout alone, or the implementation commit without
these unstaged plans, is not a complete continuation environment.

Full fresh-process canonical success, V8 failures, native/core terminal
retirement and recovery remain B4 qualification. Neither B2 nor B3 alone
establishes 15-second host/AZ recovery, tenant density, production metadata
ownership fencing or zero-loss acknowledgment. The production metadata
backend decision remains open. Strong-consistent S3 remains the durability
assumption; safe deletion of orphan objects still requires evidence that
outstanding PUT attempts have terminated, so reclamation stays report-only.
