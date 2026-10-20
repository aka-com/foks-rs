# B4: qualify the guarded celld process

Status: resumed after the P1 pause. P1 is committed as `66090566`, and P2 as
`2e226249`. P3 source and component tests are in progress; actual process
qualification remains incomplete and P3 is uncommitted. Markdown is deliberately
unstaged. B4 started at `9151df82364fa60177cb62f6645746ce1aff9b48` (B3).

## Outcome and boundaries

B4 must run the real `celld foks-guarded-node` Action, including canonical
startup, bucket restore, SQL, storage.sync, actor ownership/durability gates,
native driver, TCP output and physical retirement. A received response, empty
Observe result, zero aggregate counter or successful process exit is not a
replacement for those observations. The complete G matrix in
`HOSTED_CELLD_GUARDED_NODE_PLAN.md` remains the acceptance target.

This is local process qualification against the pinned Moto bucket service.
It does not establish AWS/AZ recovery timing, production zero-loss ACK/fencing,
tenant density, actual FOKS Wasm/SQL integration or termination of outstanding
object PUT attempts. The production adapter/backend decision remains open.
FOKS wire formats, crypto, database schemas and Go-client behavior are unchanged.
Any diagnostic protocol extension below is private to the guarded Action.

Preserve the accepted A1/A2/A3a inputs, all four existing patches byte for byte,
the exact primary/guarded fixture, engine/native dependency versions and the
ordinary default build. Do not duplicate main startup or manufacture Ready,
successful proof, completion, retirement or ownership tokens for a test.

## Starting evidence and execution discipline

Use the existing validation checkout at
`.claude/worktrees/metadata-guarded-list-validation`. The author source is
`/private/tmp/foks-celld-b4-source-author`; it is never executed. The accepted
B3 source/profile/series/asset receipt is preserved under the validation
tool's `runtime/evidence/accepted-b3`.

`/private/tmp/foks-celld-b4-start.json` binds all 288 predecessor source files,
the validation/main HEAD and all 42 pre-existing dirty Markdown files. Its
SHA-256 is `7bd0657c0a00e94455f74d9331c5a951b84faf723c9b02c2a1a80c261133912c`.
Preserve those Markdown bytes; this plan is a new unstaged file. The accepted
B3 durable evidence inventory is
`7b5d70d3b3b39c38598086cda01239a6c35057ec7aead3c1712870b386ac983d`.

Before each implementation group: freeze the exact source/API/test roster,
perform root source review, then independent subagent source review. Resolve
findings before root executes candidate formatting, tests, builds or process
qualification. Reconstruct from the pinned upstream archive and strict patch
prefix; compare the full resulting tree to the reviewed author freeze. Test
only that reconstructed tree. Fixes repeat the relevant reviews before rerun.
Root alone owns candidate execution and Git operations. Agents may investigate,
author and review. Commit each passing implementation group separately with
explicit implementation paths; never stage Markdown. Review actual results
independently before committing, retaining failed attempts as evidence.

## Findings that determine the implementation

* Observe currently clones active records only. Held and retired requests
  disappear from view; the final report has no per-invocation history.
* Production `write_pending` is sticky and has no byte progress. Initial
  reactor Pending can occur before any bytes, then hide later backpressure.
* Forward roots reset, and proof results are consumed. Retrospective counters
  cannot reconstruct successful semantic proof or distinguish failure disposal.
* The public request guard currently drops after publishing `bank.retired`.
  Its destruction must be observed after its actual drop.
* The six scenario names do not install fault barriers. In particular, a
  nonreading socket can expire at 15 seconds before the 40-second shutdown cut;
  it cannot prove retained work at that cut.
* Guarded startup drops the reload receiver. The ordinary pointer watcher
  detaches a task and adoption creates a blocking build job. Simply enabling
  that watcher would introduce unowned qualification work.
* The old controller hard-codes A3a HTTP, a different program, CELLD_WATCH and
  its own report claims. Its reusable process/storage machinery is useful,
  but those oracles are not B4 oracles. Its merged stdout/stderr also cannot
  serve as strictly structured guarded stdout.

## Implementation groups and reproducible stages

P1 adds `0005-node-observations.patch` and exact stage `node-observations`.
P2 appends `0006-node-process.patch` and exact stage `node-process`.
P3 adds the process controller/profile and executes the complete matrix on
the P2 prefix. Each native stage has default, enabled and A2-lifecycle build
modes; the existing stage names keep their historical meanings. Current
`runtime-node.lock.json`/`patches/node-series.json` bind the accepted B3 inputs
and exact append-only prefix. P2 additionally binds the accepted P1 image.
There is no arbitrary patch count, stage string or caller-selected feature.

Preparer/graph tests reject reordered patches, changed predecessor bytes,
unknown paths, wrong preimages, cross-stage manifests and unbound controller
inputs. Existing strict parser restrictions apply: no fuzzy patching, symlink,
mode change, rename or binary patch. Retain existing per-patch/combined/tree
byte caps; review any actual overrun before increasing a cap. Each group must
publish its source manifest and graph/artifact bindings, not merely edit the
ignored generated runtime source.

### P1: physical observations and actual output progress

Add a Copy-only scalar observation to each invocation and transfer it to held
retirement before dropping the original state. Preserve terminal snapshots in
a fixed process-owned archive, with at most 64 entries of at most 512 native
bytes each, charged before construction. Reserve an archive index before
physical admission; exhaustion refuses further admission. No diagnostic Arc,
Weak, JobContext, Request, Worker, V8 resource or dynamic string is retained.
Temporary active observers must be dropped before serialization or socket I/O.

Bind every snapshot to tenant, invocation sequence, bank slot, actual A2
slot/generation, and the actual epoch/runtime generation and primary/auxiliary
request/operation identities once assigned. Do not assume the A2 slot equals
the bank slot. Explicit phases are NeverAdmitted, Active, Retiring,
PendingOwner, Quarantined and Free. Sequence zero never means Free.

Record semantic milestones at their existing real consumers: native driver
join/delivery; primary and auxiliary OutputAt tickets and actual proof verdicts;
sole C1 terminal-proof disposition (including explicitly not-required); ordered
socket and frame destruction; primary/auxiliary ActivityFinished processed,
Inactive and finish-consumer disposal; dispatcher disposal; actual Worker,
probe and backing-store destruction; C2/C6/native/replica disposition; complete
terminal conjunction; original Request destruction; A2 Free; public guard
destruction; then bank retirement and externally reusable execution credit.
A failed proof consumed or joined is not a
successful proof. Export existing bounded A2 events and correlate by token;
do not pretend separately locked snapshots are atomic.

One narrow ordering correction accompanies the observations: after successful
original-owner retirement and A2 Free, physically drop the public guard before
publishing bank.retired and clearing its tombstone. The bank tombstone prevents
readmission during this cut. Preserve the public guard if A2 publication fails;
that call is still fallible even after Request destruction. Distinguish internal
A2 Free from externally reusable bank Free and record the actual ordering.

The production writer counts actual Ready(Ok(n)) bytes with checked arithmetic
bounded by frame length. Record zero-progress Pending separately from the
first Pending where `0 < accepted < frame_len`, and retain final progress and
transport outcome. A TCP flush is not evidence of peer receipt. The same
observer is used by leaf tests and production sockets.

Keep version-1 Begin/Observe and their response behavior unchanged. Add a
16-byte version-2 Observe request (opcode 2, existing tenant/selector positions,
reserved bytes zero) to select the new snapshot; unsupported version/opcode
combinations still refuse. Begin retains its 32-byte/4-MiB response. Freeze the diagnostic
32-byte Observe v2 layout before implementation: magic/version, tenant,
observation selector, explicit phase, exact u64 invocation sequence, u32
positive-Pending byte count, u32 frame length, and a u32 milestone mask.
Full scalar records and existing A2 history use bounded structured stdout:
one header, at most 64 individual invocation records (each <=2 KiB), and at
most eight chunks of at most 64 A2 events (each chunk <=16 KiB), followed by
one end record binding the counts. Reuse the 65,536-byte serialization buffer;
do not build an unbounded combined JSON value. At most 64 invocation records,
512 A2 events and 128 KiB retained report storage; charge the fixed scalar
archive and any copied event buffer before constructing them and measure the
worst-case serialized forms. Missing/truncated/end-count mismatch is failure,
never partial success. Observe cannot authorize or
wait for a release. Failed-process final reporting may read scalar state, but
must not relax forward authority/admission checks to serve a failed process.
The complete emitted snapshot stream is separately bounded at 264,192 bytes
(two <=1-KiB framing records, 64 <=2-KiB invocation records, eight <=16-KiB A2
chunks). This is an output/log bound; retained storage remains <=128 KiB.

P1 source ceiling: `crates/celld/guarded_node.rs`,
`guarded_node/output.rs`, `guarded_node/control.rs` (only scalar access to
existing prepaid summaries if needed), `guarded_node/tests.rs`, `runtime.rs`,
`guarded_execution.rs`, `main/guarded.rs` and `actor.rs` (only a feature-gated
Copy semantic view of the private CompletedEffect.event). Observe actual
DurableReached/OwnershipVerified op and verdict separately through that narrow
accessor; consuming an opaque actor result is not proof that either succeeded.
No LTX/core/fixture edit is needed
merely to observe existing predicates. Outer changes are the fifth patch,
profile/series and preparer/graph code/tests. Review concrete schemas, update
sites and exact test inventory before source execution.

Tests cover initial zero-byte Pending followed by positive partial Pending,
active/held/free/quarantine identity, stale slot reuse, finite exhaustion,
checked overflow, worst-case report bounds, contention refusal, semantic
failure versus disposal, and no diagnostic alias blocking retirement. Run
the source-bound native components, both build modes, A0 output regressions
in its isolated target, and A2 lifecycle regressions before the P1 commit.

### P2: finite deterministic process faults and retained reload

Freeze a closed enum/table of scenario, checkpoint and reaction combinations
before implementation. The table must cover the matrix below; arbitrary phase
strings, free-form code, caller-selected success values and replacement
completion tokens are forbidden. Checkpoints sit at actual ownership
boundaries and retain the original task/result/owner while held. No lock is
held across await, no extra issuing authority is created and no deadline is
renewed. Fault records and their waiters are fixed/precharged process storage.

Use an explicit observed-entered barrier before the controller acts. Any new
diagnostic release/cancel command must name the observed invocation sequence
and fixed checkpoint, accept only the installed scenario's transition, reject
stale/replayed/mismatched controls and merely let the original operation run or
submit the canonical cancellation event. It cannot complete a proof or release
credit. This is an explicit B4 amendment to B3's unsupported Release command;
the exact byte layout and state machine require source review before use.

Control reply transmission is capped by the minimum of the original acceptance
deadline and reply-start plus one second. Header classification still uses the
original 15-second clock. Accepted P1 source admits its Control owner with a
deadline of acceptance plus one second; scalar_reply enforces that owner deadline.
P2 changes the actual Control admission helper to start the one-second reply
budget after header classification while preserving the original acceptance
stamp and its 15-second maximum. A slow valid header must not be rejected merely
because its first second elapsed. The previous description of scalar_reply as
using only 15 seconds was incorrect; the accepted source and raw evidence remain
the historical record.

Retain process holds needed for destructive tests independently of a canceled
connection. A held native/proof/cleanup result survives original invocation
expiry when its real work has not terminated. Shutdown-cut deliberately keeps
that actual resource charged until canonical non-unwinding process exit. A
private pending-flush adapter is labeled as such, never TCP-flush evidence.

Actual actor loss means the celld-core OS thread exits, not dropping an
AppHandle or a reply sender. A feature-only selector exit hook must reach the
existing all-loop thread-death self-fence. Emit a bounded retained-state report
before the canonical `exit_flushed(3)`; do not expect ordinary retirement after
the sole actor has died.

Reload uses the real canonical deployment graph/build/adopt path on the same
pinned deployment, with force=true yielding a new generation. Retain watcher,
adoption and blocking-build handles under a finite owner; stop admission at
shutdown and join or explicitly report retained work at the original process
cut. Do not detach a replacement watcher, abort away a spawn_blocking handle,
add a second startup path or adopt a new generation into an old invocation.
Actual GenerationChanged processing and original generation pins determine
the result. Ordinary watcher behavior remains unchanged.
The existing reload channel is unbounded. The qualification branch must admit
only one owned watcher/adoption operation and reject or coalesce excess nudges
before enqueue; retaining just the outer watcher handle is insufficient.

P2 source ceiling is P1's roster plus `main.rs`, `main/cli.rs`, `cell_dispatch.rs`,
`actor.rs`, `actor/effects.rs`, `actor/production.rs`, `actor/local_request.rs`,
`generation.rs`, `cell_runtime.rs`, `host_channels.rs`, `js.rs`,
`js/storage_ops.rs` and a new private `guarded_node/qualification.rs`. These
are narrow guarded hooks, not permission to redesign ordinary behavior.
`ltx_repl.rs`, database/bucket/replication code and fixture bytes remain read-only;
if an actual required boundary cannot be reached within this roster, review
the smallest explicit roster amendment before editing it. Freeze the exact
subset, hook API, memory charges, failure semantics and tests before coding.

Root and independent reviewers must verify that injected failures test real
consumers and disposal rather than early, unrelated refusal. Run focused
barrier/state-machine/layout/native checks and required regression modes
before the P2 commit. Full process success remains unclaimed until P3.

### P3: closed controller and complete process qualification

Leave `run-node.py`, `test_run_node.py` and `node.lock.json` byte-identical.
Add `run-guarded-node.py`, `test_run_guarded_node.py` and
`node-guarded.lock.json`. This explicitly amends the earlier outer-path roster.
Reuse the frozen Clock/Children/Evidence/Storage/process-inspector machinery
by an exact-path, hash-checked stdlib import only after bootstrap validation;
validate the closed two-script import provenance. Do not reuse A3a Run.execute,
input validation, fixture program, report oracles or claims. A small node-spawn
adapter keeps the same process owner while separating bounded JSON stdout
from bounded tracing stderr. No new Python/Moto/native dependency or proxy.

Deploy the exact no_bundle fixture first and primary second through canonical
deploy. Read back and hash the exact seven module/manifest/pointer objects.
Versions remain fixture `1fdd2523a443ea95`, primary `b19584ba0ac8c37f`.
Each case gets a fresh process, scoped bucket/prefix and fresh absolute local
data directory. Only restore-next-process shares the bucket between two
sequential fresh node incarnations. Main initializes V8 before Tokio.

Use a finite profile-enumerated case list, not an uncontrolled Cartesian
product. Freeze every case and its oracle before execution. Each independent
controller case has one original <=600-second parent deadline, <=90 seconds
for its node scenario, <=12 total child handles, at most two live celld nodes,
and the exact five loopback port roles (Moto plus two public/internal pairs).
The complete suite is capped at 128 named cases and one original 14,400-second
aggregate deadline. This is a reviewed suite envelope, not permission to renew
case/request timeouts. The exact list may be smaller and remains immutable
within an accepted evidence run. All diagnostic operations per case are
bounded (64), request acceptance remains 15 seconds, and ordinary shutdown
retains its canonical 40-second cut.

The environment is closed: local dummy credentials, EC2 metadata disabled,
AWS config/shared credentials at /dev/null, explicit loopback HTTP endpoint,
scoped s3 bucket/prefix, fixed listeners and fresh data directory. Refuse other
credential sources, CELLD_WATCH, test-script substitution, dev store and
unreviewed env inputs. Network policy permits only the finite localhost roles;
no AWS access is needed. Keep pinned Python/Moto/module/native aliases and
validate them before candidate imports. Parent independently verifies complete
owned-process-group disappearance after cleanup, not just return codes.

Readiness requires structured ready report plus actual matching node lease/
internal state. A listening public port is insufficient and is not HTTP.
Send the exact header followed by write-half EOF. Check complete response
bytes, fixed payload SQL counter, fresh-Wasm flag and 64-MiB length. Correlate
all phase observations to the same sequence and A2 token. Real socket holds
require actual positive incomplete Pending bytes and measured finite endpoint
buffers before peer drain/disconnect. Record raw frames, separate stdout/
stderr, monotonic bounds and exact process/binary/profile/input identities.

| Case family | Required process observation |
| --- | --- |
| cold-sync-retire | Actual restore/activation/SQL/sync, primary+aux proof, F3/C1, exact frame, core processed+Inactive, full destruction and later reuse. |
| restore-next-process | Counter 1 ACK, new process/fresh local directory, counter 2 through bucket restore, fresh Wasm again. |
| capacity and tenant busy | Two actual occupied slots, third refusal without Worker construction, same-tenant refusal in Active/Retiring/Quarantined, reuse only after physical Free. |
| descriptor-closed | Boot/reload no guarded warmup; fixed generic-entry refusals before activation; invalid descriptor/binding cases; separate ordinary-mode regression. |
| binding matrix | Wrong owner/epoch/core request/invocation generation/descriptor and zero/missing/drained runtime generation at real owned boundaries; no fallback adoption. |
| sync-held | Before enqueue, after enqueue, resident dispatch, before proof, after proof, before processed/Inactive; owner remains live and all host pumps progress. |
| sync-abandon | Original expiry, reply loss, parent cancel, generation swap and actor loss at those boundaries; joined or quarantined work, no late sync success. |
| failed-turn-gated | Write-then-throw status 1 only after actual failed-turn gate; infrastructure/proof/owner failure cannot publish that status. |
| reload-held | Before construction, during sync, before output and during retirement; original generation remains pinned or canonical cancellation occurs. |
| bucket readiness | Actual unknown/transport/HTTP-contract failure never Ready; positive probe uses the same actual Bucket. |
| proof-held | Actual durability followed by ownership verification before publication; startup/local-commit proof cannot substitute. |
| TCP held/disconnect | Positive partial server Pending; original frame/Request/activity/slot held; exact drain or actual socket/frame disposal before credit. |
| cancel write/flush | Canonical CancelCellActivity, no late publication, exactly-once finish, original ownership; label private flush test separately. |
| deadline matrix | Original acceptance clock at header, route, restore/build, sync, F3, C1, write, flush and retirement; no renewed deadline or success from ambiguous disposal. |
| effect abandonment | Caller/reply/actor loss at restore, start, proof, verification and stop; actual handles/results charged until disposal or quarantine. |
| stop-after-memory | Real Worker/probe destruction then held/error stop; Wasm credit returns while execution remains occupied. |
| panic matrix | Construction/task/turn/payload-drop/teardown uncertainty retains A2 taint and original-owner quarantine; no generic Err-to-Free conversion. |
| both shutdown modes | Handoff and Preserve complete only after semantic+guarded drain; cut reports retained work before hard exit and parent proves group disappearance. |

Component tests can establish exhaustive input parsing and private-entry
refusal coverage, but cannot replace these full-process ownership/lifetime
oracles. The profile/report must label each evidence layer accurately. No
sleep-based ordering: use observed barriers. Real elapsed time is asserted
where original deadline, process cutoff or responsiveness is itself tested.

## Validation, resources and completion

Tool tests first exercise hostile profile/argv/environment/import/input paths,
port ambiguity, log/report overflow, truncated/stale observations, unowned
children, cleanup failure and misattributed evidence. Then inspect the exact
locked/offline default/A2/node graph, list the exact private native test names,
build and Clippy the real bin, and execute source-bound focused suites. Preserve
each binary before another mode overwrites it. Retain the isolated A0 root
workspace target to avoid the known unhashed metadata rlib collision.

Native build policy stays jobs=2, initial free>=6 GiB, ongoing free>=2 GiB,
target<=4 GiB, finite per-command deadline and owned process cleanup. At B4
start, root reclaimed only main target/debug/deps rlib/rmeta caches after a
process check; the deletion receipt is
`/private/tmp/foks-celld-b4-cache-reclamation.json`. It did not remove source,
executables or accepted/validation evidence. Process evidence is bounded at
256 MiB per case and 2 GiB for the suite; enforce the aggregate ceiling and
ongoing 2-GiB free-space floor before admitting each case and during writes.
No deletion of failed evidence to make a current run pass its budget.

Execute the smallest cold case first. If actual runtime reveals a missing join,
proof, pump or disposal, retain the failed evidence, diagnose and review the
smallest source correction, reconstruct and rerun. Do not weaken an oracle to
accept it. Then execute the finite remaining matrix, including ordinary/default
and A2 regressions. Bind final reports to reviewed source, graph/features,
binary, controller/profile, fixtures, argv/env/network policy and cleanup.

B4 completes only after the full enumerated acceptance matrix passes and an
independent reviewer checks the evidence and exclusions. Any missing physical
observation remains an explicit failed or incomplete qualification. Commit
implementation separately from unstaged plan/results Markdown, and leave a
handoff naming the still-open production adapter/fencing, deployment-scale
capacity/recovery and object reclamation work.

## Plan reviews

Root review, first pass: corrected four planning hazards before independent
review. (1) The original six labels are only a start; the full G matrix remains
required. (2) A1/A2/A3a and B3 receipts cannot be relabeled after a source edit;
each append-only stage has its own exact reconstruction and evidence. (3) A
single combined 64-record/512-event JSON object could exceed the existing
serialization allocation, so reporting uses finite individually bounded
records and an end-count check. (4) Per-case limits alone permit excessive
suite runtime/disk use, so the plan includes finite aggregate time/evidence
ceilings. P1's source schema/update-site/test freeze and P2's closed hook table
remain explicit source-review gates before execution, rather than permission for
arbitrary diagnostic controls. Root finds the P1 architectural plan suitable
for independent review; P2 cannot begin until its concrete hook table is also
reviewed.

Independent P1 architecture review passed. The source-bound receipt is
`/private/tmp/foks-celld-b4-p1-plan-list-review.json`, SHA-256
`a98dd1d4bb5d5598772c2549bbc0bae18987094e6fbbd291467eba00673a3337`.
It reviewed plan image `ef165cc6ebcfc9baef0de6c8047da492eb96e8eecd8f05eb7ff5817946f5e330`
and the eight exact predecessor Rust paths. Findings were closed by adding the
narrow actual actor verdict view and distinguishing A2 Free from bank reuse,
with public guard retention on A2 publication failure. Concrete scalar schema,
update sites, test names, actual numeric layouts and worst-case serialized sizes
still require both source reviews before candidate execution.

Root reviewed the separate P1 tooling adaptation
`/private/tmp/foks-celld-b4-p1-tooling-adaptation.json` and authorized source
authoring in the isolated author tree/validation checkout. The schema and test
roster are frozen with that authored source before execution; P2 holds/controls
are excluded. P2/P3 detailed controls and controller acceptance still require
their own plan/source review gates. No candidate execution or implementation
commit is implied by this architectural review.

P1 authoring contract:
`/private/tmp/foks-celld-b4-p1-observation-contract.json`, SHA-256
`2251a42f33aea5fc28ec02e561e7343332c673ff98b89db6fefcd2d077fabf1f`.
It fixes forty internal milestones, the actual eleven native-kind counters,
separate A2Free/PublicGuardDropped/BankFree observations, twelve added component
tests and the eight-path roster. Root reviewed it against the existing enum,
consumers and bounded reporting plan. Its exact Observe mask projection,
serialized field schema and numeric layout records are frozen with the source
and independently reviewed before execution. P2 control fields are absent.

P1 source review distinguishes concrete task/result/channel/abandon payload
disposal from non-issuing notification and context aliases. A notification can
still wake a cleanup waiter after the root becomes reusable; final original
Request destruction, followed by A2 Free and public-guard destruction, remains
the required proof that those aliases are gone. The root completion cut must
capture a re-registered waiter and notify it outside locks without losing its
wakeup. A record's `public_guard` scalar records admission provenance; the
separate PublicGuardDropped milestone records actual destruction.

Sequential storage.sync operations reuse an auxiliary lane with a new core
request ID. Its current-auxiliary identity, proof verdicts, positions and
milestones must reset together at that actual replacement. The overall ordinal
remains monotonic. Earlier auxiliary success cannot become evidence for the
replacement request. Source-bound tests cover this transition as well as
zero/positive socket Pending and the retirement ordering.

P2 review is still separate from P1 execution approval. The current source
analysis uses real bucket-probe failures after the preliminary list and wake
format checks. Canonical shutdown has a 40-second absolute budget and shorter
derived no-progress/token limits; a cut test records the actual cause and time,
not an assumed exactly-40-second residence. The concrete closed case table,
header storage expansion and retained reload state remain plan-review gates.

## P1 results and pause boundary

The enabled, ordinary default and A2 lifecycle configurations built and passed
their configured Clippy checks. All 74 preparation/graph tests, 86 guarded
component tests and six transport regressions passed. The lifecycle harness
passed one Rust test containing 28 scenarios. Enabled library diagnostics retain
the same three pre-existing dead-code warning groups; this is not a claim of
a warning-free whole-library Clippy run.

Actual compiled diagnostic storage is 106,832 bytes, below the 131,072-byte
retained limit. Each invocation observation is 376 bytes; the 64-entry archive
is 24,064 bytes. Strict reconstruction matched all 288 reviewed source files.
The first four patches are unchanged; P1 adds only the fifth patch and its exact
preparation, graph and test bindings.

Root results review: `/private/tmp/foks-celld-b4-p1-results-root-review.json`,
SHA-256 `fca7c2fc193002c85b86573481139ba0974081175de5e7b7e225428afc035426`.
The source, inputs, reviews, logs and preserved binaries will be retained under
`tools/foks-celld-guarded/runtime/evidence/accepted-b4-p1-handoff` in the main
checkout. The accepted P1 reconstruction inputs are under
`runtime/evidence/accepted-b4-p1` in the validation tool directory.

The user requested a pause after finishing and committing P1. No P2 or P3 code
has been authored or executed. P2 proposal revision d preserves the 101-case
matrix and the reviewed scope corrections, but is not implementation approval.
Resume from its paused review and the separate panic/binding-cut investigation;
close those exact hook, ownership, protocol and test-roster gates before source
authoring. The P2 tooling adaptation is a source-only proposal. Full-process
qualification, AWS/AZ timing, production fencing/durable acknowledgment, density
and object reclamation remain outside the completed P1 evidence.

P1 is committed as `66090566c0ffcccc12e2d1c2bc9917f2f84019c4` on main and
the validation checkout. Its seven files are implementation only; all Markdown
remains unstaged, and the 42 pre-existing dirty Markdown files are unchanged.
Independent aggregate results review passed:
`foks-celld-b4-p1-results-list-review.json`, SHA-256
`edd5a8d0aae9d844cbd4a22c58cfc8ddf1c3c2de8a491138d40ef613f2b19891`.
The durable handoff inventory is
`681d2612a28e53255d3cf8ca8bc6f04544a1fce3a12f92cca23a637bb2f18325`.
Its `paused-design` directory also retains the unapproved P2 panic/binding
investigation, including the proposed new local destructor catch and the
remaining real-generation refusal gates. Resume requires closing and reviewing
those plans; P1 completion does not authorize an unreviewed P2 candidate.

## Resume after P1

The user resumed B4 after the P1 pause. P2 starts at commit
`66090566c0ffcccc12e2d1c2bc9917f2f84019c4`; the data-only resume inventory
`/private/tmp/foks-celld-b4-p2-resume-start.json` binds all 288 accepted source
files and the 43 currently unstaged Markdown files. The accepted P1 author,
prepared and preserved trees are byte-identical. P2 plan closure is underway;
source execution still requires both reviews of the complete candidate.

P2's concrete revision-e plan passed root and independent review before source
authoring: `/private/tmp/foks-celld-b4-p2-final-plan.json`, SHA-256
`3bc4246642cbf22c46e0ed128abe1d6c5f84ef41a8767ddc1d1bd8530dfaf71f`.
The source candidate adds one private qualification module and changes thirteen
existing celld files, preserving all prior patch stages. It has not been built
or tested yet. The main adapter's source-c review passed independently, subject
to the remaining library, tooling, compiled-bound and result checks.

Source review closed several ownership and reporting gaps before execution:
the main adapter retains the real core-thread handle and blocking-build future
through inherited error paths; reload adoption checks actual process health and
admission; snapshot attempts consume credit before writing; final journal
reporting seals a fixed diagnostic frontier and requires exact emitted counts.
That frontier does not prove task termination. The specific early-exit fence
protects these guarded owners, without claiming every ordinary main-local
destructor is bounded. A blocked stdout still requires the controller's bounded
external ownership and cleanup.

Additional library review is checking exact checkpoint identity, lossless
wakeups, deadline selection, persistent TCP hold behavior, and measurement of
the actual consuming destructor-catch closure. P3's controller plan remains
under review, with no controller implementation or process qualification run.


## P2 component validation in progress

Both full source reviews passed before formatting or candidate execution. The
mechanical formatting delta was reviewed separately by root and an independent
agent. The final source contains 289 files and 7,496,179 payload bytes; only the
approved fourteen Rust paths differ from P1. The sixth patch is 232,205 bytes;
its 127 exact-position hunks reconstruct every reviewed postimage. The five
historical patches, dependency inputs, fixtures, and FOKS wire encoding remain
unchanged.

The 92 tooling tests passed without skips in 20.008 seconds. Their supervisor
verified unchanged frozen inputs, no remaining observed descendants, and no
occupied build slot. Native strict reconstruction also passed. Enabled,
ordinary-default and lifecycle qualification, compiled numerical limits and
results review remain required before the separate P2 implementation commit.

The amended P3 controller plan passed root and independent review. It preserves
all 101 actual process cases and 42 controller tests, original deadlines, and
resource limits. Its three-file implementation is being authored with an
explicit incomplete-profile gate. Final source/profile freezing and any P3
execution require the accepted P2 commit, source inventory, and preserved
binary images. Component qualification alone cannot establish the assembled
process, host/AZ, production metadata fencing, or durable acknowledgment claims.


## P2 validation findings and qualification gaps

The initial candidate above was superseded after actual validation. The first
run found three binary compiler errors; the next compiled the binary, passed
the configured Clippy check and CLI probes, then found two test-constructor
errors. Both narrow fixes received root and independent source review before
their reruns. The third run compiled and executed all 123 component tests:
122 passed, and the unchanged B2 task-handle test exposed a blocking ownership
lookup added to actor polling. All three failed attempts and their input
freezes, logs and source images are preserved. No P2 implementation commit has
been made. The 92 tooling tests passed on each of these three candidates.

The polling fix now has root and independent source approval, pending compiled
and runtime validation. It takes scalar identity snapshots under the existing
record guard, releases ownership guards before qualification checks, and keeps
the actual joined result owned when delivery encounters contention. A consumer
drop witness follows the actual consuming drop. Further reviewed work is
being planned to test that second-lock contention with a real joined metadata
result and to observe actual validator refusals and failed task joins. These
observations must preserve the complete original identity and cannot infer an
outcome from an injected fault or a generic failure flag.

P3 source review has tightened successor-node terminal validation, pre-output
response checks, historical TCP backpressure handling, sealed diagnostic
framing, and final evidence accounting. Review remains open for peer-close
causality and the new native outcome witnesses. The controller profile remains
incomplete and cannot launch qualification. Its ordinary HTTP failure matcher
and negative S3 SDK endpoints still require bounded, separately reviewed local
diagnostics against accepted P2 binaries.

External request-to-EOF timing will be measured independently of native expiry
records. Observing cleanup within the 90-second node phase cannot establish
the 15-second request target. No transport or controller-observation headroom
has yet been established for the native 15-second deadline; an over-target
measurement must remain a qualification failure. Markdown remains unstaged.

## Final P2 component results

The final candidate supersedes the in-progress inventories above: 289 source
files, 7,549,708 payload bytes, and a 291,482-byte sixth patch with 139 exact
hunks. Reviewed fixes preserve the historical test prefix and observe actual
validator comparison failures and panic joins. Ordinary route fixtures remain
valid when no process qualification target is selected.

All 92 tooling tests, strict reconstruction, enabled and default builds,
configured binary Clippy checks and CLI probes passed. All 123 native component
tests, six inherited transport tests and 28 lifecycle scenarios passed. The
compiled component retention measurement is 114,128 bytes against a 131,072-byte
limit. Its main-work contribution is zero in this component test; actual main
process work must still be measured in P3. All 773 native input images were
rechecked after execution. Exact enabled, default and lifecycle executables are
preserved in verified lossless compressed artifacts alongside raw evidence.

P2 passed final independent results review and is committed as
`2e226249358568bd136796775378dd7cd8100651`, containing only its seven
implementation files. The main checkout has been fast-forwarded to that commit;
Markdown remains unstaged. The accepted source, receipts and read-only enabled
and default executables are preserved under the validation checkout's ignored
`runtime/evidence/accepted-b4-p2` directory.

P3 remains incomplete. Its first controller unit run executed all 42 tests:
41 passed, and one test incorrectly reused a deliberately poisoned budget for
an independent evidence-cap check. The test-only correction preserves poison
persistence and uses a fresh budget for the separate boundary assertion; it
passed both source reviews and a fresh frozen run with all 42 tests passing.
Before/after input checks, child cleanup and process-group absence checks also
passed. The prerequisite HTTP/SDK diagnostics remain unqualified. Their
source and populated input data have passed root and independent reviews,
including the actual Python runtime and inherited launcher alias, original
bootstrap deadlines, guaranteed final checks, and the parent's consumption of
the endpoint's final-validation record.

The first eight-test harness launch stopped before tests when its strict import
audit found a missing standard-library image. Cleanup passed and the failed
freeze and source are preserved. A fresh freeze adds five exact library images
for temporary-file support; the audit and all time, memory and process limits
remain unchanged. The external owner plan for the full 101-case run has also
passed both reviews. None of these component results establishes the 15-second
request or host/AZ recovery targets.

The eight prerequisite harness tests passed with the corrected import freeze,
then passed again after accounting for Python argument-parser lazy imports.
Two diagnostic preflight attempts stopped without starting helpers or sockets:
one exposed the missing parser-phase closure, and one detected an extra
generated Python bytecode cache in the pinned standard library. The cache was
preserved in failed-run evidence; root and independent inventory reviews proved
that removing only that exact cache restored the original frozen fixture.
Neither fixture baseline nor source policy was relaxed.

The next diagnostic attempt started real celld and deployed the seven canonical
objects. Its ordinary HTTP case failed: the client half-closed its write side,
celld reported incomplete requests, and repeated readiness checks exhausted the
64-operation budget. All failures, exact requests and empty response wires are
preserved. Final fixture/input checks and actual external process-group cleanup
passed. SDK cases 14–16 did not run. A narrow HTTP client correction is being
planned and reviewed before another attempt; no ordinary HTTP matcher or SDK
sequence has been qualified yet.

The full-suite external supervisor passed both source reviews after one narrow
correction: it checks the inspector's original three-second deadline after
actual helper reaping and pipe closure, rejecting an observed overrun. Its
populated data, actual smoke run and fresh 101-case matrix remain outstanding.
P3 remains uncommitted and incomplete; Markdown remains unstaged.

Both reviews approved the HTTP write-side and mode-aware log-pointer fixes.
The refreshed frozen runs passed all 42 controller and eight diagnostic tests,
with unchanged inputs, reaped children and empty recorded process groups. The
next real capture obtained complete readiness, the expected 76-byte HTTP 500
body, and canonical Preserve shutdown. The body is specific to the frozen
primary/service fixture and is not a generic HTTP error matcher.

That attempt's S3 transport case emitted the exact native pre-Ready refusal
records and sealed end but exited with code 3. The harness incorrectly expected
code 1; accepted native `Run::drop` always exits with code 3 on this path. Root
and independent source review approved changing only the two exact exit
expectations. Refusal reasons, sealed observations, absence of Ready, original
deadlines and cleanup requirements remain mandatory. The failed attempt and
all original inputs are preserved; cases 15–16 remain unobserved. Refreshed
freezes and another real four-case capture are still required.

The corrected fresh four-case prerequisite capture completed and passed root
and independent source/wire review. Case 13 returned the exact 76-byte fixture
error; cases 14–16 produced native reasons Failed, Expired and Invalid,
respectively, with exit 3, sealed observations and no Ready event. The range
case completed the conditional-write checks before two held ranged reads and
reached its original native expiry at 30.038 seconds. All 188 external process
inspections, final group absence, endpoint disposal and final fixture/input
validation were reviewed. The 30-second startup path remains an explicit gap
against the requested 15-second recovery target.

The prerequisite qualification record and immutable original-profile snapshots
passed both reviews. The ready profile contains 510 exact input images and is
231,152 bytes, within the unchanged 262,144-byte limit. Its protocol, 101 cases,
42 test names, fixtures and budgets remain unchanged. Its data review passed,
and the final 42-test run against that ready profile passed both results
reviews. The pre-profile eight-test result remains bound to its saved original
inputs; that test source is not silently reinterpreted after profile promotion.

The full process freeze binds 1,663 images, including accepted native inputs,
durable source copies, all prerequisite capture files and the final ready-profile
unit evidence. Root and independent reviews approved that freeze and the external
supervisor binding. The first actual cold-case launch stopped before importing
the controller or starting a helper, socket or native celld process. Its saved
module roster lacks exactly `linecache`: the preliminary roster collector used
Python command mode, whereas the supervisor runs in file mode. CPython's command
execution path imports `linecache`; the file execution path does not do so.

The failed launch and exact original inputs are preserved. A separate truthful
file-mode collector passed root and independent source reviews and completed
successfully. Its separately observed phase rosters contain 74, 85 and 86
file-backed modules, with only `linecache` absent from each former roster.
Root and independent results reviews verified all 79 standard-library images
and exact agreement of the first phase with the actual failed-launch roster.
The supervisor's successor data binding contains those exact maps and 1,693
images, including the new reviews and preserved original attempt.

The historical closure, profile, final unit evidence and process freeze remain
immutable; no import allowlist, source policy, timeout or qualification oracle
is relaxed. Both binding and launch reviews passed. The fresh cold-case run
matched all three actual parent rosters and launched native celld. Its bucket
probe succeeded, but native startup then fenced with `Bounds` before Ready.
There was no `MainWorkCharged`, initial snapshot or sealed End. The downstream
parser and case cleanup consequently failed. Independent review verified all
inputs, physical pipe EOF, four native process inspections and disappearance of
the controller group and all four case children. External OS cleanup does not
substitute for the failed case protocol.

Read-only disassembly of the accepted binary identifies the precise cause.
The real reload future is 4,560 bytes, counted twice for overlapping construction.
The complete typed formula is 10,168 bytes plus twice the string lengths; this
fixture's lengths total 113, giving a calculated charge of 10,394 bytes. The
8,192-byte individual check rejects it before the aggregate check. Adding that
charge to the accepted 114,128-byte component ledger gives 124,522 bytes, below
the unchanged 131,072-byte total ceiling. These are static layout calculations,
not a successfully emitted native measurement.

A separately reviewed append-only correction is being planned: a 12 KiB
individual allowance, unchanged total ceiling and conservative accounting,
and a regression measuring the actual main-process types through the production
layout helper. No further actual case has passed. A fresh cold-case attempt and
fresh 101-case run remain required. P3 remains incomplete and uncommitted.
Markdown remains unstaged.

### P3 follow-up: measure and admit the real reload owner

Before implementation, review the exact source and test roster against the
failed cold-case archive and accepted-binary disassembly. The follow-up keeps
the 131,072-byte aggregate ledger, all per-type alignment checks, both copies
of the concrete future, and checked string arithmetic. Only the individual
main-work allowance changes from 8,192 to 12,288 bytes. Oversized generic
configurations still refuse; the local qualification fixture has 113 string
bytes and is not a claim of unrestricted configuration support.

1. Extract one production layout helper used by `configure_reload` and by a
   dedicated binary unit test. Measure the concrete future, context, owner,
   outcome, stop allocation and thread handle without constructing a runtime,
   V8 isolate, SDK operation or reload job. Test the actual fixture's string
   geometry and checked overflow. Expose bounded scalar context on refusal.
2. Update all native main-work checks consistently. Retain the exact 123
   library test names while extending their relevant boundary assertions.
   Add the separately selected binary layout regression to the existing
   supervised native-test workflow. Test individual and aggregate rejection.
3. Append a seventh patch and an explicit reproducible follow-up stage. Keep
   the first six patches, accepted P2 source and binaries immutable. Extend
   existing preparation/graph tests for the exact new prefix, feature modes,
   source hashes and test selection; add no dependency or FOKS protocol change.
4. Review source first in root and then independently, including corrections,
   before formatting, reconstruction, tests or builds. Reconstruct the new
   prefix, compare it to the reviewed source, and run relevant preparation,
   graph, native library, binary layout, default/enabled CLI, Clippy and
   lifecycle/transport checks under the existing resource supervisor. Review
   results independently and commit this native correction separately.
5. Update the uncommitted process parser and its existing 42-test roster for
   the consistent individual bound. Preserve exact old source/profile/input
   preimages and review the new binary provenance and profile binding. Use
   fresh freezes and re-establish the relevant HTTP/SDK prerequisites for the
   amended artifacts; historical proof is never silently relabelled.
6. Run a fresh cold case under the external owner, requiring an actual
   `MainWorkCharged` measurement, Ready, complete observations and final
   ownership cleanup. Review results before a separate fresh all-101 attempt.
   Keep every existing case oracle, original deadline and the strict external
   15-second request-to-EOF check. Any further failure needs its own reviewed
   correction; no late completion is accepted as target compliance.
7. Independently review the complete actual results before the P3 implementation
   commit. Keep Markdown unstaged. Production fencing/durable acknowledgment,
   actual AWS/AZ timing, remote PUT termination, and tenant density remain
   separate qualification work.

The native follow-up is committed as `885dbeb2` and fast-forwarded into the
shared checkout. Its seven implementation files passed root and independent
source and results reviews. Strict reconstruction matches all 289 reviewed
source files. Validation passed 100 tooling tests, 123 native component tests,
six transport regressions, 28 lifecycle scenarios, the real binary layout
regression, configured Clippy/CLI checks, four fresh HTTP/SDK prerequisites,
eight diagnostic tests and the final 42 controller tests. Git whitespace checks
pass outside the patch artifact; its two flagged spaces are required unified
diff context markers. Git hooks were disabled for the commit; the recorded
checks were run explicitly.

The new guarded binary is preserved separately from accepted P2. Fresh storage
failure evidence still shows the original roughly 30-second stalled startup
probe, leaving the 15-second recovery gap open. Review also caught an oversized
profile before execution. The repaired profile uses the original compact JSON
format and bounded proof-manifest references, stays within the unchanged
262,144-byte limit, and passed independent data review and all 42 tests. The
complete raw proof roster must remain in the external F/D freezes.

Fresh cold-case and all-101 process qualification remain pending. These native
and prerequisite results do not establish recovery timing or full P3 completion.
Markdown remains unstaged.


P3 continuation: readiness and qualifier contention
-------------------------------------------------

The guarded runtime emits structured readiness records rather than the ordinary
listener log. The controller now uses the already-required structured P1/Ready,
internal-state and lease checks for guarded nodes; ordinary-node readiness still
requires its listener log. The narrow change passed both source reviews and all
42 controller tests. The subsequent real cold1e run reached readiness and
returned its first native response in about 34 ms. After observing the original
invocation as Free, its reuse request received an empty response. This is a
failed case, not a recovery or reuse qualification. Native sealed End was absent;
the independent outer owner nevertheless verified process-group absence and
physical pipe EOF after cleanup. Exact inputs and failed evidence are preserved.

Source review found qualification bookkeeping conflates a busy mutex with a
poisoned mutex and can permanently fail the process on ordinary contention.
The capture does not identify the exact accessor responsible for its first
error or prove why reuse returned empty. The reviewed correction therefore
addresses the established ownership defect without retrying requests or
relaxing the case oracle.

Native plan v2 (`9e6149cca1537c6c00fa641c172aa95ed03b4e377ce954449e803240188eb67b`)
passed root and independent review. It distinguishes try-only new admission,
short synchronous bookkeeping for already-owned work, and polling that defers
on contention. Poison remains a failure. The blocking checkpoint evaluator must
re-evaluate its full predicate under the same mutex before a condition-variable
wait, preventing a missed notification after an unmatched or consumed target.
Reply consumers retain their actual payloads on Pending; connection-drop
witnesses follow physical disposition; original forward deadlines are rechecked
after bookkeeping waits. No persistent production field, wire change or memory
allowance is added. Existing native tests gain deterministic contention and
poison coverage using actual lock boundaries, with their 123 IDs preserved.

Tooling plan v3 (`7281874c5b6432fd1cdd208a2cf2bbf1744308098aff24149206d45438abd3a9`),
with correction 1 (`b4af291f0324c7d624c27d325b0ba2292aff26e849880369ef5814c3366337e7`),
also passed both reviews. Append patch 0008 as an explicit revision of the
existing main-work profile. Canonical validation requires exactly eight patches;
explicit archived predecessor validation retains the seven-patch history.
Preserve the original profile, series, assets, source and binaries. Fresh current
proof freezes must include every current raw proof image; superseded historical
rows require explicit preserved mappings and consumer classification. Existing
row, byte, resource and timeout limits remain unchanged.

Implementation and source reviews are in progress. Next: reviewed source,
reconstruction and tooling tests; enabled/default native validation, lifecycle
and actual binary layout tests; new HTTP/SDK prerequisites; fresh controller
validation; actual cold-start/reuse case, then all 101 cases. Each new failure
remains evidence to investigate. No full P3 or host/AZ 15-second claim follows
from the successful initial response. This Markdown remains unstaged.

The qualifier contention correction is committed separately as `9baecfa4`
(`9baecfa4b6fd487a65c3c56a1d858412f44a0d4d`) in both checkouts. Its seven
implementation files contain patch 0008 and the corresponding preparation,
graph, profile and test changes. All Markdown remains unstaged; the three P3
controller/profile files remain separate.

Both source reviews and independent actual-results reviews passed. Validation
passed 108 tooling tests, exact reconstruction of 289 source files, 123 native
component tests, six transport regressions, 28 lifecycle scenarios, the actual
binary layout regression, and configured default/enabled Clippy and CLI checks.
The first tooling run correctly caught an incorrect test-name namespace in the
new metadata (107/108 passed). The corrected metadata uses Rust's actual
`guarded_execution::node::tests` namespace; the assertion and all test bodies
remain unchanged. The failed run and its input images are preserved.

The new component is preserved in
`runtime/evidence/accepted-b4-qualifier-contention`, with receipt SHA-256
`639ed58021ec269131fb889c2a0dd713c417e4086d27dc61836ed1cc12a876a0`.
Its enabled binary is
`f751f6643f2221a4988b9b546875939b0ff30d136a6d3490d452c6817e33c240`;
the default binary remains byte-identical to the accepted predecessor. The
927-image native input freeze and all 567 published raw evidence images were
independently checked. Publication rechecks every raw evidence image immediately
before writing its acceptance receipt.

Fresh prerequisite, cold-start/reuse, and all-101 process qualification remain
in progress. The initial prerequisite data refresh refused before mutation
because its guard expected `ready` rather than the actual pinned profile status
`qualified-inputs`. Correcting that fixed-data guard requires no runtime or
protocol change. No process101 or host/AZ 15-second claim is made by this commit.

Fresh qualifier-contention prerequisites now pass with both result reviews:
the operative eight controller-contract tests and all four actual HTTP/SDK
cases. The deliberately closed ordinary cell returns its exact 76-byte HTTP500
body. The SDK cases show Failed/Expired/Invalid refusals with native exit3 and
sealed diagnostic frontiers. Start-to-exit times were 1.187610542 seconds,
30.045882917 seconds, and 0.046669500 seconds. The external owner observed
process-group disappearance after 205 inspections. The 30-second stalled
startup probe remains a gap against the 15-second recovery objective.

The new prerequisite qualification is
`/private/tmp/foks-celld-b4-p3-qualifier-contention-prerequisite-qualification.json`
(SHA-256 `11cceeb991ae2facda70050c166772ce262c08880bf55fbb9ab5a2fcbab1a3c1`).
It binds all 328 raw diagnostic and preserved-input files. Independent review
corrected one historical manifest mapping to its immutable archive. During
metadata generation, a concurrent two-literal recipe rebind meant source C ran
instead of the intended reviewed source B. The actual source identity was
reconciled and reviewed with the generated data before any further candidate
execution; the record explicitly does not claim prior review of C. No native
binary, test oracle, or completed-run evidence changed in that reconciliation.

The fresh 42-test controller run passed with both results reviews
(`b4-p3-unit-1123071879163916`, receipt
`e2266cd4339ab4d0ab7178121249523dc985bede58a6e1864ad1043492d61fe2`).
Its 253 input descriptors remained unchanged and its child/process group was
reaped. The first new cold launch stopped before importing the controller or
starting native Celld: the external-owner input set omitted its required plan
and two hardcoded plan-review records. The source-derived membership audit
confirmed exactly those three omissions. The failed launch and its original
2,529/2,530 input sets are preserved; a metadata-only supplement restores those
roles without changing owner code, test expectations, or limits.

The corrected cold1g run reached readiness in 153,319,958 ns and passed the
native cold/reuse oracle. Both 32-byte responses succeeded, with EOF latency
32,865,541 ns and 31,224,875 ns; both A2 owners reached Free, the native stream
sealed its End record, and the native process exited0. External group626
absence, pipe EOF, and all2,553 final inputs/fixture/origins were verified.

The outer result remains incomplete: its report consumer requires
`cleanup.error == []`, whereas inherited `Children.cleanup()` emits `None` on
success. The failure is preserved at
`node-guarded-cold1g-1123961630486375/outer-result.json`
(SHA-256 `91124801631a43e23455d22a9d9bf301fb4338c7102bf363da01bc1a6e6b99ec`).
A minimal private-owner schema correction must require the error key explicitly
and require its value to be null. Regression verification must use the actual
successful report and reject missing/error-valued/incomplete cleanup variants.
This successful native case is not relabeled as a successful outer qualification
or full101/host-AZ15s proof.

The private owner cleanup schema correction now requires an explicit null
`cleanup.error`, with both completion booleans still exactly true. Its actual
report regression passed two tests covering 20 invalid variants. After preserving
the six prior authority images and the complete cold1g evidence, the fresh
42-test controller run passed at `b4-p3-unit-1125018548960625` (receipt
`99d12cf33e20ad98b20b221c958a579e1f388bf0f0a9055c6b83a952ff531c20`).
The successor process inputs retain all prior proof and bind 2,647 controller
images / 2,648 outer-owner images. Both source and actual-data reviews passed.

Fresh cold1h (`node-guarded-cold1h-1125155670355208`) exposed a separate native
failure. Its first 32-byte request succeeded, and ObserveV2 returned a complete
32-byte response while retirement was progressing. The invocation ultimately
reached Free, but the native process emitted a failed header and exited3 before
reuse. The external owner preserved that failure, observed group disappearance,
and validated its complete input set. Outer result SHA-256 is
`842a813a704294413be79824008299aeec5994a799cce3783e03a6834262a893`.

Root and independent source inspections found a supported failure path:
`scalar_reply_owned` closes the response socket, then calls
`retire_unstarted`; the latter still uses a try-lock for an already-admitted
control owner. Transient contention can therefore return a held owner and
trigger the caller's node-wide failure fence after sending a successful reply.
The exact failing accessor was not recorded, so this remains a source-supported
cause rather than uniquely observed attribution. A narrow correction and
barrier-based contention/poison regression are under plan review. HeaderLease
cleanup already uses the owned-lock path. No cold1h, full101, or host/AZ success
is claimed, and no test oracle or deadline is relaxed.

The admitted-control retirement correction plan passed root and independent
review (plan `90425cab83d5256dc7d2529913e97641ce6709afc56aee8d4812c5dd7f7e8c93`).
It changes registered-owner cleanup to the existing owned-lock path while
retaining true admission try-locks, process-unique cleanup, alias/generation
checks and poison failure. Two existing B1 tests cover real lock contention,
permit retention, HeaderLease destruction and poison. The native roster stays
at123; tooling retains108 selected historical tests and adds4 focused checks.
Append-only patch0009 preserves the accepted eight-patch prefix and all caps.
Implementation and actual component/process verification remain pending.

The patch0009 source, tooling and package passed root and independent reviews.
The actual112 tooling run `b4-control-retirement-tooling-1126999257524208`
passed all112 tests with no failures, errors or skips and203 unchanged inputs
(receipt `fcaf1319b012a00081626727bb44898e53602bf361270364149b8d466507ad17`).
It completed in59.900329917s under the original60s deadline; this leaves little
headroom, and no timeout was increased. Native input freeze
`8db3f6a5d89f5b68983b18a97677b594c325b50b217211e83555b3b5401835d0`
binds1036 immutable images and the complete289-file target tree. Historical
raw evidence is preserved; native modes and process qualification are pending.

Patch0009 was committed as `3c71c49a062e74f96cdddc6c85df17eaed28ae33`
(seven implementation files only; Markdown remains unstaged). The actual
default/guarded/lifecycle/bin-layout modes passed, including123 native tests,
six transport tests,28 lifecycle scenarios and one binary layout test. Root
and independent source, raw-result and publication reviews passed. Accepted
component receipt `3d86e15db9d58a68e4442e5da21b977082328d90e08e894b12a2b41e4aa394f4`
binds the new enabled binary `d89bac72ab79482a0b30105a879111ec7f71cd89404e5135fbad4cc0e524f077`
and565 aggregate evidence images. No full101 or host/AZ15s success is claimed.

The next process-input refresh plan passed both reviews after explicitly
resolving one historical mutable source-manifest reference to its exact
accepted9baec immutable copy. It preserves physical history, original caps,
G/H/owner and all101 oracles. Fresh8/actualSDK4/Q/42 precede the next cold run.

The refreshed eight prerequisite unit tests passed at
`b4-p3-prereq-unit-1128346476214416`, with all208 inputs unchanged.
The actual four diagnostic cases passed both raw-result/source reviews at
`guarded-prereq-control-retirement-1128241550692166`. The ordinary HTTP500
retains its exact76-byte fixture error; SDK startup refusals retain Failed6,
Expired4 and Invalid8 with exclusive native checkpoint streams. The external
supervisor observed all child cleanup and process-group disappearance. The
timeout case took30.040460416s, which remains an explicit15-second target gap.
These results qualify prerequisites only; cold/full101 qualification is pending.

The new prerequisite qualification binds327 evidence images
(`4279da10477b30117d861e6358e0d924a0939e8c935931946e507c2aef392676`).
Both source and actual-data reviews passed for Q and the ready profile. The
profile retains all260 prior entries and now contains270; exactly five of256
controller-unit input roles changed. Fresh42 passed at
`b4-p3-unit-1129357737558000`, receipt
`e5879be47e96890c2988a7665927c4182223bf42d93cd35c8e7eaaf43607020b`.
The original test identities, native oracles and timing caps are unchanged.

The final process inputs passed source and actual-data reviews: F has2822
images (`4990ade8976fbb95b1815e1d663ee24576d14e668b3109af2c2330f3df8405ea`),
D has2823 (`52c9d4a9e1cce93dc9ab77216519e53325ad7f6c2d3fe70e694b6f00c1813f35`).
Two metadata-generation attempts stopped before publication: an ordering-only
comparison of44 unchanged history files, then omission of two unchanged
original profile references while retaining their archive copies. Both were
corrected narrowly and reviewed; no native test or oracle changed.

Cold1i (`node-guarded-cold1i-1130205181761375`) passed native cold/reuse and
external ownership checks. Readiness150.636ms, complete32-byte EOF responses
40.095417ms/30.943916ms, bothinvocationsFree, full32A2events, sealedEnd7,
nativeexit0 and externalgroup28925 absence. The expected outerexit2 records
a successful single-case diagnostic, not full101. Both actual-result reviews
passed. The full101 suite is now running in
`node-guarded-all101-1130331986925500` with the same reviewed inputs.

The first full101 attempt stopped atcase2 aftercase1 passed. Outer receipt
`9b95ab0844a14e9788e673be2eb50bfc46b3886591ff329ed37cd7ab5f1bc29e`
records incomplete qualification with successful external cleanup and final2823
input validation. Donor counter1 response/Free/End/exit0 succeeded. Fresh
successorB reached readiness and checkpoint2 but returned an empty frame;
the counter2 restore oracle correctly refused it. The storage trace shows
B reading the existing owner and still-live donor lease, with no successor
ownership PUT or LTX restoration before failure. Later storage transport
errors occur during harness cleanup and are not the initial failure cause.
Source analysis and any correction remain pending; no full101 success claimed.

The case2 diagnosis identified a harness precondition mismatch: Preserve retains
the old node lease, while the harness immediately launched a differently named
successor. The reviewed correction observes exact donor lease expiry before one
successor Begin, keeping a single15s budget from observed donor exit through
complete counter2 response EOF. It also corrects class-prefixed cell identity
at the three existing storage/local-fault sites. Native code and101 oracles
remain unchanged. A second review caught binary hashing between the deadline
check and spawn; final sourceC checks the same deadline immediately before
spawn and adds a deterministic regression. Both source reviews passed.

The six pre-change authorities and both complete raw scopes were preserved
(receipt35e57c2107213f599dfceb5d92cb7d34b56d147f242acb4c59d82291217f895d).
The old successful50-file cold1i and failed81-file full101 scopes retain their
original verdicts. New controller validation and full101 remain pending.

A further source review found that cases7/83 used a Discard-style source-path
fault against an actual Cleanup→Rebase→Preserve close. Moving a directory to
an absent db.evicted could succeed. Reviewed addendum2 instead creates an
exclusive nonempty db.evicted destination while preserving db.sqlite. Final
sourceD passed author, root and independent reviews, including deterministic
OS rename refusal and existing-destination preservation regressions within
the same42 test IDs. Native101 oracles and binaries remain unchanged.

FinalD was adopted after both installer/source and actual-data reviews. Profile
now has271 entries (only G/H replacement plus one explicit historical-reuse
manifest); I42 keeps256 entries and42 test IDs with only G/H/P replacements.
The first controller run completed42 tests but correctly failed qualification
when the sandbox denied /bin/ps cleanup inspection. Its raw evidence remains
under b4-p3-unit-1132749027047416. The same reviewed launch with process
inspection enabled passed at b4-p3-unit-1132772767179666, receipt
`e7c23aaa2340f07d314b64fabf48074d816911a1932b93921e84c502d3c328f6`.
All42 passed in0.4674725s; childexit0, emptyprocessgroup, inspectorreaped,
all256 source hashes/modes and rawlogs verified. Full101 remains pending.

The next full101 attempt (`node-guarded-all101-18798625`) stopped atcase2
aftercase1 passed. Outerreceipt
`1f808a1a0dac5fe10f2638036ab389499278de5ff1bbd124f95541bf44dc20bb`
records incomplete qualification and completed external cleanup. Case2 donor
returned counter1/32-byte successfulEOF in46.794167ms. Its first Observe was
alreadyQuarantined (mask0x83ff1fff): WorkerDestroyed present, C6Succeeded absent.
All63 observations remained identical; the64-operation budget was a secondary
exhaustion, not the original failure. No successor or new lease-wait code ran.
The source database/WAL/SHM remained, with no preserved db.evicted. The existing
error flattening discards the original anyhow cause, so no exact native leaf
can honestly be inferred. All90 raw files are preserved.

The bounded guarded-retirement diagnostic planV2 passed root and independent
reviews (`12c481c739d3b522a3576086a73dcab28f80b6eb18bd19722ccb60d01534b5b2`).
It adds failure-only stderr diagnostics at actual C6 stages before error
flattening, preserving original error/ownership semantics and unchanged G/H.
Formatting uses two256-byte stack destinations and at most16 source hops;
overflow returns fmt::Error with honest truncation. The existing123 native
test roster is preserved by extending one test ID. No functional race fix or
full101/HA15s success is claimed. Append0010 implementation/review is pending.

Two reviewed inventories of old main-target compiler objects were removed
after fresh process census and exact identity/hash rechecks:674 files/537MB
and1931 files/1.074GB. No source, binary, evidence, validation-target or pinned
input was removed. Free space rose to7.72GB, restoring headroom over the
unchanged6GiB native build admission floor.

The diagnostic Rust source passed author, root and independent source review.
Root formatted only the four approved Rust files with the pinned formatter;
formatted sourceB retains289 files and123 test IDs, with a28,005-byte patch
inside the unchanged44,611-byte remaining allowance. Root formatting review
found only wrapping, whitespace, trailing commas and import ordering. The
strict ten-patch loader/graph changes preserve the archived nine-patch path;
all203 historical tooling test method ASTs are unchanged, with four additions.
The selected116 tests must still meet the original60s bound. New component
execution and actual process diagnosis remain pending. Markdown is unstaged.

Formatted nativeB and both fixed execution supervisors passed independent
source review. Root executed the reviewed data-only packager; generated
patch0010 is28,005 bytes and combined patch payload1,556,258 bytes, within
the unchanged1,572,864-byte allowance. Root independently applied all hunks
to the accepted predecessor and obtained the exact four reviewed afterimages.
Both actual-data reviews verified all289 rows, nine-patch prefix, inherited
policies, T0 preservation and all90 failed-run raw files. No new native
build or101 process result is claimed yet.

The new tooling116 run failed the unchanged60s deadline at
`b4-guarded-retirement-diagnostics-tooling-1135580308626666`, receipt
`fcab6ce2dc6203d2b4cbf783b8bfdf3a848003fa223d664213e120010b9c86f9`.
The log records all112 historical tests successful, then interruption during
the first new diagnostic test. No returned116 summary or traceback supports
attributing an assertion failure. Both failure reviews verified all255 inputs
unchanged and no observed descendants. The failed run remains preserved.
A source-only review of redundant synthetic fixture ancestry reconstruction
is underway; no timeout relaxation or unchanged retry is authorized. Native
builds have not started for diagnostic patch0010.

The fixture-assembly optimization plan and sourceA passed root and independent
reviews. Only two test files change: eighteen synthetic prepare/assets setup
calls and the graph fixture setup call use a single-operation successful
ancestor cache. Original validators are restored before fixture yield and
test-body assertions. All207 test names and206 method ASTs remain unchanged;
one existing new positive method gains deterministic cache-contract checks.
All203 historical method ASTs and the original116/60s acceptance remain.
No speedup is claimed until actual execution. Source/input correction remains
separate; review caught a predecessor test-metadata schema mismatch before
execution, which must be fixed in the bound successor.

The optimized116 attempt also failed: run
`b4-guarded-retirement-diagnostics-tooling-1136654189024666`, receipt
`918bf0936ab6ff2ced7352314afc993b75294295d971acd659900f173da1b4e7`.
It logged107 successful tests, an earlier main-work graph test error, then
the original60s abort during the ninth-patch test; no full traceback or116
result summary was returned. All256 bound inputs remained unchanged and
no descendants were observed. Root and independent source diagnosis found
a regression missed during source review: an existing test creates root/graph
before fixture setup, but the new generic top-level inventory requires it
to be a regular file. The proposed follow-up uses an explicit positive read
dependency inventory for accepted-ancestor validation and avoids scanning
unconsumed archived source trees. Timing benefit is still unproven. Both
failed runs remain preserved; no native build or process101 success claimed.

The positive-metadata inventory addendum and fixture sourceB passed root and
independent review. SourceB (`140293c1da22ade4183855c5c54690cf6c1360b0fe88bd7c5fa48e2755f14a18`)
changes only preparation-test helpers; all207 immediate predecessor test method
ASTs and116 selected IDs remain unchanged. It permits the existing graph/output
directory while checking the explicit files read by accepted-ancestor validators,
including parent identity and consumed-file mutation. Correction2 preserves the
previous head/history and replaces exactly three source rows plus the active
correction-head binding, retaining252 of256 rows. Its plan passed both reviews;
actual data adoption and the unchanged60s tooling gate are still pending.

Correction2 was adopted after source and actual-data reviews, preserving all
prior evidence with256 current bindings. The third tooling run passed all116
tests in51.840416041s under the original60s limit: receipt
`5b50b8f79d6dc8f9343d667923285d747482ba0b665ec91c9d80c01c1a4824cb`,
run `b4-guarded-retirement-diagnostics-tooling-1137693483179708`. Zero failures,
errors or skips; all256 before/after/current hashes match and no descendants
were observed. Native diagnostic build and process101 qualification remain
pending. Native freezerF now models both immutable correction heads explicitly
and passed both structural reviews; final actual-evidence bindings come next.

The complete1240-file native input freeze passed both actual-data reviews
(`c7093ad769cd55ebd09ba34971cea8fc8a9cdd40eb2d3c93f61131aa4382f722`),
including both289-file source trees and explicit two-head history. Preparation
reconstructed the reviewed source exactly. Default mode passed in81.807s and
guarded main-work mode passed in114.848s; lifecycle and binary-layout modes
remain pending. The default binary remains byte-identical to accepted control
(`9956c095…`): because the shared target had already been replaced by the
enabled build, an explicit preservation receipt records copying identical
immutable accepted bytes, without relabeling the new default run result.
The enabled binary was copied from its actual completed output before the
next mode, SHA `b1ed0f60eb5bf198a7ab61ab0cbbef887c9a55bdc9eb312931faef07f7be4f86`.

All four diagnostic native modes now passed: default, guarded main-work,
lifecycle (28 cases), and binary-layout; the fixed native suites retain123
native tests and6 transport tests. Both actual-result reviews passed. Published
component `accepted-b4-retirement-diagnostics/receipt.json` has SHA
`d5792a0216c2676a4a0a124fab9496b1846b867a5310429f647f0fc0321818cb`;
its566-image aggregate and289 copied source files were independently rehashed.
This is component qualification only. The prerequisite input refresh preserves
the prior epoch and binds the diagnostic binaries; SDK4 and process101 remain
pending. No AWS/AZ, zero-loss, remote-PUT-termination, or GC claim is made.

The current prerequisite8 run passed with227 unchanged inputs (receipt
`d19c861e3650c135952ba16854aded2f1aec1e16096b67f53968827b48d9af33`).
Both result reviews passed. The subsequent all4 attempt at
`guarded-prereq-retirement-diagnostics-16532875` failed in case15: cases13/14
captured expected observations; case15 returned native refusal4/second1,
exit3 after30.030987792s, but its endpoint was killed before writing final
validation. Case16 did not run. The outer owner proved group disappearance;
this does not qualify the SDK sequence. All269 raw files are preserved.
Source review found conflicting cleanup budgets: endpoint final validation
allows30s, while inherited child cleanup advances from TERM to KILL after3s.
The exact validator operation interrupted by KILL is not recorded. A later
latched-evidence accounting error also prevented the aggregate report and
masked the endpoint error in controller stderr. A narrow diagnostic-harness
correction is being planned; native request deadlines and process101
qualification remain unchanged and unproven. Markdown remains unstaged.

The endpoint cleanup correction plan passed root and independent reviews
(plan SHA `0207aeae2a70badf4e35e8741e915de7e50ce529679a00a611d30a42503a56ef`).
It changes only the private prerequisite diagnostic harness and tests, with
one absolute30s cleanup deadline, a cooperative phase and reserved forced
cleanup time. Deterministic tests must cover late validation, stalled cleanup,
uncertain inspection, storage joins, cancellation and primary-error retention.
The old R1 receipt remains immutable; a distinct correction receipt and exact
source/input preimages will support fresh8/all4 results before Q/ready42/101.
Native source, G/H and all protocol/functional deadline limits are unchanged.

The native diagnostic component is committed separately as
`2755f875cdf74c3f59dd77d50b0274a5e95696d3` after both precommit reviews.
Only the seven implementation/patch/tooling files are included; G/H/P and
all Markdown are excluded. Main and validation worktree now share that HEAD.
The staged patch whitespace checker flags two required space-only unified-diff
context lines; source reconstruction/formatting and native tests already
validated their exact bytes. The six nonpatch files pass the whitespace check.
P3 remains active: the diagnostic SDK cleanup correction is under source
revision before rerunning8/all4 and continuing42/full101. No full recovery
qualification is implied by this component commit.

The endpoint cleanup source correction passed both source reviews and the
fresh fixed eight-test suite (`b4-p3-prereq-unit-1142135979333041`): all eight
passed, all 244 inputs matched before/after, and the owned group was absent.
The first data adoption failed its final check because DATA still referred
to the previous profile. That failed attempt and its partial five files were
preserved; a separately planned and twice-reviewed two-reference rollforward
updated DATA index 0 and I8 index 212. Both full data reviews passed. The
original R1 receipt remains unchanged, and the distinct correction receipt
records the complete chain and failed intermediate attempt honestly.

Fresh all4 capture `guarded-prereq-retirement-diagnostics-cleanup-92db071b`
then completed with both actual source/wire/ownership reviews passing. All
313 raw files and 269 diagnostic input images were verified. Case13 returned
the exact 76-byte fixture HTTP500; cases14/15/16 returned native refusal
6/4/8 with exit3, final validated endpoint records and cooperative endpoint
exit1. Case15 took30.083335916s; its endpoint completed3.385317209s after
native exit, exceeding the old3s grace but within the unchanged30s cleanup
budget. Outer exit0 followed200 inspections and observed group76711 absent.
This closes the prerequisite harness cleanup failure, not the15s recovery
objective. Final Q, ready profile, fresh42 and full101 remain pending.
All Markdown remains unstaged.

The refreshed prerequisite qualification and ready profile passed both source
and actual-data reviews. Fresh42 `b4-p3-unit-1143145094577958` passed all42
unchanged test identities with zero failures/errors/skips; all256 inputs and
modes matched before/after/current. Receipt SHA
`f7160b05c54ff03b0f5bd5eb0821f3e516821b45d723f9ad70d226eaaf3f7c41`.
Both result reviews passed; the owned group was absent and inspector reaped.
The final full101 input closure is being rebound to these actual results,
with the existing4096-row/2MiB caps and all current profile inputs retained.
This is still prerequisite/unit qualification; full101 and15s HA remain open.

Final input publication completed after both source reviews. F has3812 rows
(1,112,318 bytes, SHA `a96ca5fab662c56c2919f4006b0147ef70238fa8d760efd02ba2d3289f90c402`);
D has3813 rows (1,158,713 bytes, SHA
`48614f10ff9b845df6a3ebb6bdf1a6ea9100af0342191543c7fe52c30c705cf7`).
Both fit the unchanged4096-row/2MiB caps. Nine mandatory archived inputs were
promoted from the557 eligible historical flat rows, leaving548 omitted from
per-launch hashing; their full historical inventories and physical bytes were
checked during publication and retained. All314 current profile rows remain
unprojected. Root actual-data review passed; independent actual-data review
and launch admission remain before the fresh complete101 attempt.

Fresh full101 attempt `node-guarded-all101-827ab654` completed unsuccessfully
with both result reviews agreeing. Cases1/2 passed, case3(two-slots) failed,
and cases4–101 did not run. All3813 inputs remained valid. Case3 logged an
actual Forward(Compaction) root failure (slot0/invocation1/lane2/sequence1),
followed by join_compaction and C6 failures; the later64-operation exhaustion
is secondary. The generic Failed value does not distinguish a retirement
admission race from taint or a lower compaction failure. Native cleanup was
incomplete and required SIGKILL; outer cleanup independently proved group83155
absent. The145 raw files/39 directories are preserved (inventory SHA
`bc4c09122887ee292026cac21f08e6b78a97fe776939edbb93071e76c2596566`).
Outer receipt SHA `7a1398a5f73ca2466c9eb0a7814931dafe8c303e27e412f174a9ae21b24a7bae`.
A combined causal-diagnostic/functional-fix plan is being prepared. No full101
or HA qualification, no automatic retry, and no increased limit is authorized
by this result. The passing case2 does not establish its earlier intermittent
failure was fixed by diagnostic logging.

The combined background-retirement correction plan passed root and independent
review (plan SHA `54090cf6b08e3e980161a974c4cb6275e8c27d38d727e5a834610f8f0c0644d6`).
Its first step is private failure-only attribution at the actual forward-check
sites and lower compaction failure, with unchanged returned errors. Scalar
state is copied under the guard, and logging happens after guard release.
Two existing test identities gain deterministic original-owner checks; these
are explicitly not a real Worker/compactor reproduction. Actual private case3
attribution must precede a concrete functional-change plan and its reviews.
The diagnostic candidate will not become a separately accepted component.
Final combined source still requires complete native and process qualification
before an implementation commit. All Markdown remains unstaged.

The private attribution source passed author, root and independent review after
a test correction: the complete state matrix now shares one absolute deadline
rather than renewing five seconds per case. Root formatted the three files
with pinned rustfmt1.95.0; final formatted source is being bound to the private
build. The isolated build review also found missing inherited Cargo config.
The private build must copy the actual effective SDK linker/TMPDIR config and
reject unexpected source files. Historical native receipts recorded both
effective config files, but native1240 and F3812 did not directly include them.
The final combined qualification must explicitly bind those config files and
absence/precedence checks in its new input epoch; historical records remain
unchanged. No diagnostic execution or functional-fix qualification follows
from these source reviews. All Markdown remains unstaged.

Private attribution build D completed successfully (receipt SHA
`3560c29c3d989b42556be83c49e1639eea39356b503f98bc62e83664b7437f70`).
The full metadata graph matched after exact relocation normalization; both
original focused test identities passed once (0.10s and1.01s), with122 other
native tests filtered each time. The enabled private binary is159,846,448 bytes,
SHA `a65bfffdcc0aae6ac4f38c43aa8079e79d6971c174834f509bd3bc6182463890`.
All four Cargo groups completed with cleanup established;1560 input images
and297 generated inputs remained unchanged. Root results review passed;
independent results review and actual case3 runtime launch gates remain.
This is diagnostic-only, not full native123/process101 or a functional fix.

Two earlier private runner failures are preserved: B stopped before Cargo
because it selected the base rather than MAIN_WORK profile; C ran metadata
successfully but correctly refused the graph because the isolated copy omitted
tests/lifecycle.rs and Cargo included the relocated fixture name in its ID.
D copies that already-bound real test source and normalizes only that exact
fixture identity; graph equality remains complete. Both fixes received root
and independent source/data reviews before execution. Original B/C records
remain unchanged. The one-case process owner is under review for cancellation
admission, common cleanup deadline and final module-binding checks.
All Markdown remains unstaged.

The private two-slot diagnostic `node-guarded-diagcase3-4bc07f2e` completed
successfully: all three responses were exact32-byte success frames with EOF,
both original tenants retired, tenant0 reused at epoch2, and the native node
exited0. The outer owner established process-group absence and both pipe EOFs;
all1449 runtime inputs rehashed unchanged. There was no forward or retirement
refusal event. This is nonreproduction, not evidence that the original race
is fixed. The original failed full101 attempt remains authoritative and open.
Raw49 files/15 directories are preserved (inventory SHA
`e1b09b6013e9f64bb47261b26aa19cb9924c62823360facb961d825e61a9e9d1`);
outer receipt SHA `b4e108e5ead1bac892aa80dbb746f2e9d5c0213656a0e8b8529d0bc5e7cfc987`.
Root results review passed; independent results review is pending. The next
experiment must have a reviewed bounded reproduction plan rather than an
until-pass retry or a speculative functional correction. All Markdown remains
unstaged.

Independent results review also passed for that nonreproducing diagnostic
(SHA `eeb23ff0cce84df1c15deb7083f299baa097c8ca10e3c9b4a67a447a405f2206`).
It independently checked the actual92/93/639 module-origin sets, final644 bound
modules, all1449 inputs, raw49 files, native retirement observations and the
single cleanup deadline. The clean-reload inventory warning remains preserved.
A controlled real-retirement-cut experiment is being designed; no automatic
repeat or functional source change has been performed.

The controlled real-retirement experiment plan V2 passed root and independent
review (SHA `17d00e1af2d15fcf6edf5fc5945a52ae4b525de9fb246d8a31b10d4eecf9055b`).
A private scalar ticket binds the original case3/slot0/invocation1 and exact
compaction root/epoch/generation. The actual C5 stop retains its returned
NodeRuntime after publishing retiring, before WorkerDestroy/C6, until that
exact real compactor reports the actual Failed(false,true) state branch.
A single deadline is clamped to the original owner deadline and five seconds.
Waker registration and observation are atomic; callbacks/destruction happen
outside state guards. Failed/expired/cancelled paths retain the original
resources and never manufacture a witness. The experiment establishes a
reachable ordering bug if observed; it cannot uniquely attribute the earlier
uncontrolled failure. All private ticket/wait hooks must be removed from the
final separately reviewed functional correction. Fresh private source is being
authored; no additional native execution or production acceptance yet.

Pre-execution source review found that a compaction-only witness could lose to
a real Sync/Hydration retirement failure that first taints shared control.
The reviewed amendment (SHA
`83af2f832003b99aafc935d889bcafddff2f6e5f820795f4e92aee59eba5a5ae`)
therefore captures the exact three possible admitted background root tokens
and records one actual matching retiring-only witness and its root identity.
It records the first ticket winner, not a globally earliest error. A Running
root may still be entering jobs.bind; the witness proves admitted-background
ordering, not that lower compaction/callback work had already begun. Both
plan reviews passed before the unexecuted draft was amended. Functional
regressions must separately cover healthy lower callbacks during quiesce,
failed sync capture during stop, exact consumed-root identities, no duplicate
C6 consumption, and WorkerDestroy only after physical background disposal.

Source review of controlled diagnostic A rejected an impossible Driver `Spent`
precondition: actual Forward-root disposal publishes `Empty`. Fresh B corrected
the production arm predicate and added a real submitted/joined/disposed F3 task
regression, with never-started and incomplete-disposal negatives. Root and
independent source reviews passed before root formatted a separate C tree.
A and B are preserved. The diagnostic still has no functional-fix or process
qualification claim; the next gate is the focused private build and actual
controlled two-slot case. All Markdown remains unstaged.

Formatted controlled candidate C passed both source reviews. Its private build F
completed successfully: full metadata equality, both focused tests (1 passed,
122 filtered each, 0.10s/1.00s), and enabled binary build. All four process
groups cleaned up; 1592 inputs and 297 generated files stayed unchanged.
Receipt SHA `6bc1463bb0142029b29eca8e9350b8c6b223eb3f9cc884e079a659917b7cf567`;
binary SHA `4cfc8207ad8460faca9a7d71d09147dcdbbb7ffddcde5120d6c0253ab0b14836`.
Root results review passed; independent results review and the controlled
real-process witness remain gates. No functional source is implemented.

The conditional functional draft proposes closing background root admission
and joining/disposal in C5 before retiring/WorkerDestroy, then C6 validation
of that exact proof without duplicate consumption. It explicitly handles
Forward roots returning to Empty (which can otherwise be reused), continued
callbacks for already-admitted lower work, and sync-capture errors before any
stopped shortcut. Concrete representation and entry-path review remain open
until the actual witness is evaluated. All Markdown remains unstaged.

The controlled run `node-guarded-diagcase3-72fa6c40` established a limited
reachable-bug witness: real F2 slot0/invocation1/sequence1, epoch1/generation1
returned State Failed with tainted=false and retiring=true while C5 retained
the actual taken runtime before WorkerDestroy. The ticket recorded/released
that exact failure within about20.5ms; Sync and C6 then failed through the
original propagation path. Both outcome reviewers accept this limited causal
evidence for concrete fix planning. It does not uniquely explain the earlier
uncontrolled failure or prove lower callbacks had already begun.

The attempt itself FAILED and remains incomplete: owner exit1, case budget64,
native SIGKILL-9/no End, worker cleanup false with stream parser failure.
Two original32-byte success frames reached EOF, but tenant reuse did not
complete. Outer physical cleanup separately succeeded (group41233 gone, both
pipes EOF, no inspection error). All1498 inputs and final module bindings
validated;66 raw files/17 directories are preserved. Outer receipt SHA
`76077ea67e214fe8148abd3a0acefe0480e866477986cfb7e30f7dd4c304aeff`;
independent split-verdict SHA
`9bad4a595b09c4a8229e2331303120a251a18ca8405c66239bbabb562dd7711a`.
No rerun or oracle weakening is needed merely to label this attempt successful.
Concrete quiesce-before-retiring plan review is next; no functional edit yet.
All Markdown remains unstaged.

The functional quiesce-before-retiring plan and integration plan now have root
and independent reviews. Private draft A preserves all accepted diagnostics
and moves actual F0/F1/F2 joins ahead of runtime take/WorkerDestroy, with an
inline original-attachment/token proof, atomic admission closure, and C6 proof
validation. Root and independent preliminary source reviews found no additional
production blocker after the original-attachment correction. Fresh draft B is
adding actual pending-future drop, original-deadline expiry and mutex poison
coverage; it is not yet tested or accepted.

The recomposed patch exceeds the inherited 1.5 MiB aggregate source-diff
allowance. This is build-time packaging, separate from the runtime 128 KiB
retained / 12 KiB main-work budgets. A reviewed amendment is being prepared to
preserve diagnostics and measure a revision-2-only source-diff allowance after
formatting, rounded to 4 KiB with a 1664 KiB review ceiling. Historical loaders,
source-tree/per-selected-patch limits, runtime limits, and test oracles remain
unchanged. The proposed diagnostic deletion was not applied. All earlier
failed and diagnostic-only attempts remain preserved; no new qualification or
commit is claimed.

Private native B passed both semantic source reviews; root then formatted a
fresh C copy using pinned rustfmt. C passed root and independent formatting
reviews, with all289 source rows, original123 native test IDs and accepted
assertions retained. C's exact n=3 recomposed patch is88,020 bytes (SHA
`02a810b1faf01af037ec9a42b69afdd9dacb4e166a37d7a130cb5c844ed5a26d`).
The independently reviewed artifact-budget amendment fixes revision2's
aggregate limit at1,617,920 bytes (1580 KiB), from measured1,616,273 bytes
rounded to4 KiB; historical1536 KiB loaders remain unchanged. This does not
increase runtime memory or deadline limits.

Eight current canonical tooling/profile/series/patch/assets preimages have
been copied and independently verified before installation. Tooling B passed
both source reviews, including the actual archived revision1 profile+series
refusal before tool admission. No candidate tests/builds or new native/component
qualification have run yet. Markdown remains unstaged.

Installation of the reviewed revision2 package completed in validation only.
All seven installed source/package roles and their preimages were rehashed;
source/root and independent installation reviews passed. The fixed tooling
suite then passed all116 tests in50.806 seconds under its original60-second
budget, with zero failures/errors/skips, unchanged before/after input hashes,
and no remaining observed descendants. Actual receipt:
`b4-background-retirement-tooling-1179822731835083/receipt.json`, SHA
`e50c5e282cc3ed5b3056044b89af62f5f01ffd382faa798385134084dc058002`.
Native freeze/prepare/four-mode qualification remain next; no new component or
full101 result is claimed. All Markdown remains unstaged.

The final native input freeze passed root and independent data reviews with
1,913 immutable images and two explicitly verified live tooling fixtures.
Freeze SHA `6f2c7f002e5bb5d35a7372cfba6b01027c3bc33db2d677bd8599789c3e34ccb5`.
The historical-input branch was corrected before execution to verify those
same two fixtures without incorrectly treating their live paths as immutable;
all earlier source revisions and reviews remain preserved.

Actual preparation passed in6.591 seconds and reconstructed all289 reviewed
source files exactly. The default native mode passed (wrapper SHA
`4acea995b874f3a23ec5b83f83d6fd8ee136c87f401335f49f107d6695622913`),
with its exact binary copied and rehashed before the enabled build. Enabled,
lifecycle and binary-layout modes, then prerequisite and full101 qualification,
remain open. No new component acceptance or implementation commit is claimed.

All four functional native modes passed with the same reviewed input freeze:
default77.014s, enabled109.842s, lifecycle106.503s, binary-layout45.415s.
Actual enabled tests:123 native plus6 transport; lifecycle28 scenarios;
binary-layout1. Retained process state measured114,128 bytes under128KiB.
Enabled binary SHA `38774c7c385c77e6d98c2496bc6043e8b49bce1993c52540a9a23f2a73c9ba20`
(159,875,600 bytes), copied/hash-verified before later modes. Component
publication and refreshed prerequisite/full101 validation remain gates.
The original SDK30-second startup gap and all deployment/replication/GC/density
limitations are unchanged. All Markdown remains unstaged.

The native component is accepted locally at
`accepted-b4-background-retirement/receipt.json`, SHA
`db600b3610201c3d517047d67df6400c1e871497fa537c91f99960be987b4f83`.
Both result/publication reviews verified565 aggregate images and1913 native
inputs, copied289source files, and both exact executable binaries.
The fresh prerequisite transition preserved eight preceding authorities,
produced339 incomplete-profile/295 diagnostic/256 unit inputs, and passed both
data reviews. Its operative eight tests passed in1.396s with exact input maps
and owned process cleanup. Actual HTTP/SDK cases13–16 are now running under
the unchanged owner and timing limits. No full101 or deployment claim yet.

Actual prerequisite cases13–16 passed both source/wire/result reviews in
`guarded-prereq-background-retirement-ab61734e`. Root preserved330 files and13
directories; independent review rehashed all295 current diagnostic inputs and
parsed the endpoint bytes. Case13 produced the exact HTTP500/76-byte body;
cases14/15/16 exited3 with native refusal6/4/8 and sealed diagnostic End.
Case15 had two actual held GET attempts9/10, both with peer EOF, and took
30.026300542s. External process-group disappearance and final validation
completed. This confirms the existing SDK30-second gap rather than15-second
HA compliance. Q/ready42/final101 publication remains pending.

Fresh Q and ready-profile publication passed both reviews. Q contains340
exact evidence images and nine preserved preimages; readyP contains349 input
rows, and I42 retains256 rows with exactly the five intended replacements.
The unchanged42-test owner passed all42 tests with no failures/errors/skips,
matched before/after input maps/modes, and complete process cleanup. Receipt
`b4-p3-unit-1181358035447791/receipt.json`, SHA
`439b2e5b33b9eae30da22c34dfe9eda0ec69da331ef72440cb52a0eea5f37f3e`.
Both result reviews passed. FinalF/D publication and actual101 remain next.

The final-proof producer plan keeps all1913 current native input tuples at
their original paths and promotes every current consumer. Its reviewed finite
historical policy covers1499 indexed files in29 preserved scopes, retaining32
historical anchors and considering1467 rows for inventory-only historical
checking. No physical evidence is deleted; exact final promotions/counts must
be reviewed against the unchanged4096-row/2MiB caps before actual101 launch.

Final input publication completed after correcting an inventory comparison
that treated historical traversal order as significant. The first producer
attempt stopped before writing outputs; all 29 inventories had identical
paths, hashes, sizes and modes. The reviewed correction sorts the complete
lists without dropping duplicates, and retains every pre/post evidence check.

The resulting controller freeze contains 3,745 inputs (1,064,365 bytes, SHA
`f7df4f86cbf5c19d60ceb8a51f8652b46a7d358dcfae78b1bfe9af496a0e2995`);
the owner freeze contains 3,746 inputs (1,110,760 bytes, SHA
`048b53f220daf5824f5401a614d65d408a36aa7a0b4b9a7f42b4f90d4379852f`).
All 1,913 current native and 349 profile inputs remain directly bound. Four
historical entries are promoted for current consumers; 1,463 are indexed
history rather than flat launcher inputs. No historical payload was deleted.
Root rehashed the actual outputs; independent data and bound-launch reviews
remain required before execution. Markdown remains unstaged.

The fresh complete matrix attempt `node-guarded-background-all101-04e721`
passed cases 1–12, including the former two-slot retirement failure. It stopped
at case 13 with `Refused: actual internal state status`; cases 14–101 were not
run. The outer result is SHA
`9eeaf41bf77dbc9f08b46c7326d003cdd4e2cdb2d7ebf0391be6e155cde2fac2`.
Root and independent reviews verified 383 raw files and 116 directories,
unchanged inputs, physical process-group disappearance and both pipe EOFs.
Case 13 needed SIGKILL during cleanup, so its ordinary shutdown did not pass.

The controller did not retain the failed readiness response bytes. Source
inspection identifies an early recovery-only internal listener that responds
503 before the normal-listener log is emitted; this explains a possible race
but does not retrospectively establish the exact failed status/body. The next
controller correction gates ordinary /state polling on that existing log and
retains unexpected response bytes, preserving strict 200/state/lease/generation
checks and the original deadline. Full process qualification remains open.

The six-file native background-retirement component is committed separately as
`3a7beb84c04f89551c3cb6ebea183873502447b2` and main was fast-forwarded.
Its component and result reviews passed; the case-13 controller readiness
failure does not qualify or invalidate unrun process cases. P/G/H remain
uncommitted. The narrow readiness plan passed root and independent review,
and six old launch authorities were preserved before controller changes.
All Markdown remains unstaged.

The ordinary readiness correction passed root and independent source reviews.
It waits for the existing owned normal-listener marker before issuing /state,
and retains unexpected HTTP response bytes before refusing them. Strict HTTP
200, state, lease, generation, child-liveness and original deadline checks
remain unchanged. The profile and native sources did not change.

The existing 42-test controller suite passed with deterministic coverage for
missing/wrong listener markers, effect ordering and exact 503/404 refusal-wire
retention. Only two entries in the 256-input test manifest changed (G/H).
Receipt `b4-p3-unit-1182152862320000/receipt.json`, SHA
`c1e2023503875b40ad80a340e9ec47072f15addda104843635a83ee2c38b0ba3`.
Root verified the result and cleanup; final result review and refreshed full
matrix input binding remain pending.

The refreshed input publication passed both reviews: controller F has 3,783
rows (SHA `e40afa689f4d267106f5eb47f440afeb308ce8d0ebd4f9667f7d4d0e25dad3e6`),
owner D has 3,784 (SHA
`5638a1858d82ff45a5c8fe5f83536f23667f7fa9883e520be1548d7e0e38ce2b`).
Every prior flat input survives directly or through an exact preserved tuple
mapping. P, all 1,913 native inputs and the 349 profile inputs are unchanged.
The fresh full matrix is running in
`node-guarded-ordinary-ready-all101-65bca4` under the original owner and limits.
No pass is claimed before its actual results and cleanup are reviewed.

The ordinary-readiness full matrix stopped at case 21 (`sync-held-after-proof`)
with `Refused: original/phase deadline expired`; cases 1–20 passed and 22–101
were not run. Outer receipt SHA
`8410af43c6238ebb1911f09c60e36f51782350a70ed4f3a5bcef4e9cbfe95e98`.
Root and independent reviews checked 633 raw files, 170 directories and
unchanged launch inputs. Outer physical cleanup completed with no survivors
and both pipe EOFs. Inner cleanup did not complete: its stream parser refused
the incomplete native stream, which lacks a sealed End.

Case 21 acquired ownership and wrote SQLite WAL frames but made no LTX PUT.
Sync and Compaction roots expired at their original 15-second deadline; the
controller subsequently reached its 90-second phase deadline without checkpoint
9. Independent source reviews confirmed the checkpoint selector and found no
proven lock cycle. The next step is a narrowly reviewed, bounded proof-progress
diagnostic, with unchanged admission, scheduling, selectors and deadlines.
This failure remains evidence; it is not a reason to extend a deadline or
blindly rerun the matrix.

A private case-21 diagnostic plan has passed the root review and is awaiting
independent review. It uses finite stderr observations for driver/auxiliary
routing, proof admission/completion and the first background sync attempt.
Native capture selector fields are deliberately deferred; existing error
warnings and object-store logs remain the evidence for that interval. The
private build reuses the existing owned runner and three existing sync/timeout/
layout tests. It will not replace accepted binaries or count as qualification.

Build space was reclaimed from unused main-workspace Cargo `.rlib`/`.rmeta`
outputs only (8,711 files, 4,048,117,552 bytes). No compiler process was active,
and current process/native input manifests reference none of these paths.
Source, executables and evidence were preserved; the cache-reclamation record
is `/private/tmp/foks-celld-b4-p3-proof-progress-cache-reclamation.json`.

A forensic read of a private copy of case 21's database/WAL found no
`foks_guarded_node` table. The independent check rebuilt the WAL index, verified
all 21 frames and eight commits, and passed SQLite integrity checking; original
files are unchanged. This does not establish that JavaScript never ran. Source
inspection shows this fixture's explicit SQL runs synchronously without an
automatic transaction spanning the turn, so successful fixture CREATE would
normally already be committed before storage.sync.

The first diagnostic was therefore narrowed to runtime activation and the
request driver's first turn. `driver-early-plan.json` (SHA
`a33242f0bcca8edf84c70f657d7cb85277d7a32252b8d5c3182f7298a2ea2f4f`)
passed root and independent review. It permits three private source files,
eleven finite progress labels and bounded original errors. The downstream
proof-progress plan remains unimplemented. Private source and fixed owner
adaptations are being reviewed before the three existing failed-turn, original
deadline and layout tests, enabled diagnostic build, and one case-21 attempt.
All accepted artifacts, process qualification flags and limits remain unchanged.


The private early-driver build completed: metadata graph comparison and all
three exact tests passed, followed by the enabled binary build. Every owned
command reported successful cleanup; the build slot is absent. The private
binary is 159,927,088 bytes, SHA
`7d1499afc9d39c110a11ac1aa391b4890d914f087dc075d60bdf127024fce567`.
The root result review verified actual output, generated inputs and current
hashes. One temporary pinned source archive was missing on resume and restored
byte-for-byte from its pinned URL; that audit and restoration are retained.
Independent result review and the isolated diagnostic launch remain pending.
No accepted artifact or qualification flag changed.


The independent diagnostic-build results review passed. Preparing the isolated
run then discovered 1,295 missing files in the pinned temporary Python fixture,
including all 448 pinned bytecode files. Compared with the retained full
inventory, there are zero changed or extra files. No diagnostic process was
launched. Exact restoration from cached copies is being investigated; the
original fixture digest, dependency versions and test limits remain unchanged.
The damage inventory and restore-only plan are retained under
`/private/tmp/foks-celld-b4-p3-driver-early-fixture-*`.


Exact recovery is available for all 1,295 missing fixture files. The 847
non-bytecode files were privately staged from matching local cache entries,
exact-version wheel members, the transfer archive's original configuration
blob, or hash-verified literals. All 448 bytecode files were reconstructed
without importing or executing fixture modules: compile-only recovery retained
only outputs matching each original pinned hash (445 first compilations and
three second compilations). Root and the author independently verified their
sizes, modes and hashes. Full staged-set independent review and absent-only
restoration precede the original full-tree validation and diagnostic launch.


Absent-only fixture restoration completed after both reviews. The full original
venv digest is restored: 13,448 entries, 11,378 files, 156,151,877 bytes and
448 bytecode files, SHA
`5000c92a516d9ac57571c2a428316f36129ed20499b12042d7d48b797eb52c31`.
No version, fixture pin, canonical source or deadline changed. The case-21
run inputs now bind 3,360 images (SHA
`758c633d8f339cc53c49dddcb8fffcc76da8507fa28c4683365e6c1fd60aaa3f`);
root review passed and independent final data/launch review is pending.


The private instrumented case-21 run passed and both actual-result reviews
passed. Scope: `node-guarded-diagcase21-a817c249`. Checkpoint 9 advanced, both
32-byte responses completed (57.2 ms and 34.3 ms), reuse completed, the native
End was sealed, and inner plus outer cleanup succeeded. All 3,360 input images
remained unchanged. The parent bootstrap used different string quoting from
the planned argv; AST/value equivalence is verified and the literal deviation
is recorded. Worker argv was generated by the reviewed owner unchanged.

This pass does not resolve the original uninstrumented stall. A bounded plan
for ten standalone accepted-binary case-21 trials is under independent review.
It keeps the original controller/input freeze and oracles, changes only the
private outer owner's fixed paths and selected case, and stops at the first
failure. No full-matrix rerun or qualification follows automatically from ten
passes; ordering after original cases 1–20 remains a possible factor.


### 2026-10-19 continuation: parent bootstrap and private case-21 result

The first-failure diagnostic parent preflight mismatch is now isolated. A
reviewed capture-only direct-script launch observed 96 modules versus the
expected 97: only `linecache` was absent, with no added modules or changed
paths. A second capture using the existing worker's exact `-c`/`runpy`
bootstrap observed all 97 expected modules. Both captures stopped before
helper loading or worker/native execution. Expected inventories were preserved.
Raw observations and root/independent reviews are retained under
`/private/tmp/foks-celld-b4-p3-first-failure-origin-capture-*` and
`/private/tmp/foks-celld-b4-p3-first-failure-origin-runpy-capture-*`.

The fresh case21b attempt passed parent definitions/helper checks but its
worker's `sandbox-exec` exited 71 (`sandbox_apply: Operation not permitted`)
under the tool sandbox, before Python/native execution. Its evidence was
preserved; owner cleanup and independent process/port checks found no
survivors. The separately reviewed case21c launch retained the inner network
sandbox and used tool sandbox escalation plus the proven parent bootstrap.

The private case21c diagnostic completed successfully in 20.911 seconds at
`node-guarded-diagcase21-0c19ca04`. Checkpoint 9 entered, advanced and continued;
both 32-byte responses completed within their original deadlines (72.363 ms
and 30.130 ms), same-tenant reuse succeeded, and the native End was sealed
without failure or overflow. Inner storage tasks joined and all children
retired; outer cleanup completed without uncertainty or survivors. Both final
validations passed. Root rehashed all 1,541 inputs after execution unchanged.
The outer result is SHA-256
`62eb3225003098b79e861c4251bb5a878737040e88a4513d2d968ee5cfcc1df8`.
Result reviews are retained under
`/private/tmp/foks-celld-b4-p3-first-failure-case21c-runpy-*`.

This is one successful run of the private first-failure diagnostic binary,
not a reproduction or resolution of the original uninstrumented stall. It
does not qualify the full 101-case matrix or host/AZ recovery. Preserve the
original failure and diagnostic evidence; review a bounded next experiment
before further candidate execution. Markdown remains unstaged.


### 2026-10-19 investigation: accepted standalone and same-controller prefix

The original ten-trial baseline plan had executed zero native trials: its
first attempt stopped in module-origin preflight, and attempts 02–10 were
absent. The latest parent-cancel full101 failure also differs from the older
expiry signature: Sync and Compaction observed `Failed` about 126 ms after
startup, nearly simultaneously, before checkpoint 9. This is consistent with
a shared original Request failing; retained raw does not identify its first
failure setter or establish a scheduler deadlock.

A fresh accepted, uninstrumented standalone case 21 passed under the original
controller and input freeze at `node-guarded-investigate21-20261020a`.
Readiness took 150.889 ms; the two replies took 55.277 and 35.569 ms. The owner
returned the expected subset exit 2, with complete inner/outer cleanup and
final validation. Root rehashed all 4,067 inputs after execution unchanged.

A separately reviewed private selection driver then ran cases 1–21 in one
controller lifetime at `node-guarded-investigate-prefix21a`. Every case passed;
case 21 readiness took 148.872 ms and its replies took 52.282 and 32.592 ms.
The whole attempt took 154.548 seconds. All per-case code, accepted native
binaries and original budgets were preserved. Canonical G/P/F/D stayed
unchanged; private identity/profile/freeze roles were explicit. Endpoint cases
14–16 retained their original canonical runner/profile. The private driver
returned exit 2 and `incomplete-prefix-diagnostic`, never full completeness.
Root rehashed all 4,072 inputs after execution unchanged. Independent review
verified actual evidence, all 81 recorded process identities absent, and no
remaining test listeners.

Both runs reached checkpoint 9, completed both 32-byte replies and same-tenant
reuse, sealed their native End without failure/overflow, and completed owned
cleanup. Root and independent result reviews passed. Evidence and findings:
`/private/tmp/foks-celld-b4-p3-case21-investigation-*` and
`/private/tmp/foks-celld-b4-p3-prefix21-investigation-*`.

These are two uninstrumented non-reproductions. Neither diagnostic
instrumentation nor the preceding-case sequence is necessary for success;
intermittent native scheduling/ownership or host timing remains possible.
No unique cause or native repair is established, and full101/host-AZ
qualification remains pending. A future first-failure recorder that preserves
Request layout and avoids successful-path stage writes could reduce diagnostic
perturbation; it requires its own review and layout validation before use.


### 2026-10-19 diagnostic continuation: layout-preserving failure byte

A separately reviewed private recorder now replaces the qualification-only
Request failure flag with a single `AtomicU8` in the original declaration
position. The nonqualification flag remains `AtomicBool`. Closed nonzero codes
latch the first actual failed transition with one relaxed compare-exchange;
zero stays healthy. The code distinguishes lower-budget failures, native
failure boundaries, process propagation and actor kind/refusal. Existing
cancellation, registry stopping, quarantine and drop behavior is preserved.
Actor-control successful processing is byte-identical to the accepted closure;
there are no successful-path diagnostic stage assignments or caller tracking.
Emission extends only the existing outside-guard retirement error logger.
Exact caller/stage and the earliest precursor remain outside this recorder.

The locked graph gate, all nine focused tests and enabled private build passed,
with root and independent actual-result reviews. The target-local accepted
AtomicBool declaration and private Request measured identically: size 128,
alignment 8, field offsets `[0,8,24,120,121]`, charged Arc 144 bytes. Control
remains 96 bytes and NodeRuntime 352. The six diagnostic tests cover real lower
failure setters, monotonic/isolation/racing behavior, actual actor control
identity/refusal, parent propagation and capture/report guard boundaries;
three original failed-turn/deadline/layout tests also passed. The new actor
failure test covers PrimaryFinish/AuxFinish/Cancel, not real Retirement delivery.
Identical layout does not establish identical machine code or timing.

One owned case-21 run at `node-guarded-diagcase21-0c20ba01` passed. Both 32-byte
responses completed in 58.088 and 36.357 ms, checkpoint 9 advanced and reuse
succeeded, with sealed End and complete inner/outer cleanup. All 2,197 inputs
were rehashed unchanged after execution, and independent live process/port
checks found no survivors. The original early failure did not reproduce and
no retirement-refusal category was emitted. No repair or full101/host-AZ
qualification follows. The diagnostic End cutoff remains narrower than task
termination proof.

Sources, manifests, exact tests, immutable binaries, source/build/runtime
reviews and results are retained under
`/private/tmp/foks-celld-b4-p3-byte-failure-*`. The aggregate result is
`/private/tmp/foks-celld-b4-p3-byte-failure-results.json`. No canonical native
source/profile or accepted binary was changed; Markdown remains unstaged.

A separate read-only timing audit and independent review found that the extra
private readiness time appears predominantly before the `host_runtime` marker;
post-marker state/lease intervals are comparable to accepted runs. The marker
follows guarded installation, so this is not a boundary for all initialization.
This mixed-clock/file-mtime inference does not establish a loader, signing,
cache or initialization cause. Readiness totals alone do not demonstrate a
one-second Request-path slowdown. The timing audit and review are retained
alongside the aggregate result.
