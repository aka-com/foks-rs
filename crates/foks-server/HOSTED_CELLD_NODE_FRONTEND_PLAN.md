# B3: consuming output and canonical Run integration

Status: complete in implementation-only commit
`9151df82364fa60177cb62f6645746ce1aff9b48`. Based on B2 commit
`d22ff3deedf0f0a87f635aec2317a87410e39110`. Markdown stays unstaged.
This is one implementation group and one implementation-only commit after
plan review, two source reviews, focused validation and results review.

Direct plan review is recorded in
`/private/tmp/foks-celld-b3-plan-root-review.json`. Independent plan review
passed at `ef5fc145b3f66a6a0379cf939fe3bf542cc5faa74e0c187b5caf5bc05266832d`
in `/private/tmp/foks-celld-b3-plan-list-review.json` (receipt SHA256
`4f2a3f21e91e270bfa9a78193fedfbcffeb3538a5520124649e5352c2e3dac5a`).
The narrow real DurabilityOwner observation amendment passed independently
in `/private/tmp/foks-celld-b3-node-log-plan-list-review.json` (receipt SHA256
`63bbad3522f4d888909f3b890e37f1d545eedadb7d2e0b4d3125b9463d0ad7b5`).

## Scope and retained limits

B3 makes the guarded diagnostic Action reachable through the existing Run
body. It connects the real driver, output gate, socket, canonical cancellation
and retirement owners. It is not the B4 full-process controller or production
FOKS metadata adapter. No FOKS wire or cryptographic compatibility changes.
S3 remains the assumed durable storage authority; reclamation remains
report-only without proof that outstanding PUT attempts terminated.

Retain B2's two execution slots, original 15-second acceptance deadline,
four accepted frontend connections, 16-byte closed header, three fixed tenant
names, eight forward/eight cleanup roots, four controls, 8-MiB invocation
allowance, 48-KiB typed plus 8-KiB owned/channel Rule-R ceiling, and 520-KiB
prepaid cleanup reserve. Process versus invocation ownership remains distinct.
The ordinary diagnostic frame is 32 bytes, derived from the existing exact
12-byte fixture result; the backpressure frame is at most 4 MiB including
framing. Neither requires changing the already pinned fixture program.

Use the existing validation checkout and its verified B2 postimage. A separate
author tree is copied only after checking all 286 files against the accepted
manifest. It is a source authoring aid, never the execution input. Publish
`0004-node-frontend.patch`, preserve the exact first three patches, and strictly
reconstruct the full stage from the pinned archive before tests or builds.
The initial source and all 41 existing unstaged Markdown images are inventoried
in `/private/tmp/foks-celld-b3-start.json`.

## Implementation and ownership

1. Add the feature-only closed `foks-guarded-node` Action and scenario parser.
   Validate only the reviewed local bucket settings; reject unsupported storage,
   local-script, enrollment, no-runtime and nonloopback surfaces. Reuse Run,
   the public listener, deployment loader, actor and host channels. Install the
   descriptor policy before any generation construction. Default and A2 modes
   keep their existing behavior. The enabled CLI gets its own exact oracle.

2. Establish readiness from the same actual Bucket's Conformant startup probe,
   canonical recovery, installed DurabilityOwner, actor and host channels, and
   exact deployment/runtime state. Errors and unknown probe outcomes refuse.
   AppHandle existence or a caller Boolean never constitutes readiness. Bind
   to actual installation identities and continue checking live ownership on
   every route, execution and output transition. Do not alter bucket/fleet
   semantics to make the guarded probe pass.
   Check actual actor ready-to-serve and ownership-confirmed state, successful
   local scan, installed process latch, and matching runtime/Bucket/channel
   identities. In particular generic Health alone is insufficient, and host
   channel setters silently ignoring a failed OnceLock installation do not
   prove the newly created receiver is installed.
   A narrow feature-gated crate-private `DurabilityOwner::guarded_matches`
   in `node_log.rs` inspects the actual owned stack, manager/LTX Arc identity,
   bucket posture, task owner and started/non-stopped background ownership.
   The borrowed actual owner is checked against actor/runtime observations;
   do not substitute a caller Boolean or a construction-order assertion.

3. Implement fixed frontend ownership. Capture acceptance before reading any
   header; retain that stamp through header/EOF validation and conversion into
   the original invocation Request. Unknown magic/version/opcode/tenant/phase,
   nonzero reserved bytes and trailing data refuse. No queued admission or
   arbitrary caller scope, deadline, generation or completion callback. Keep
   sockets, header cells and refusal work bounded and owned. Control/refusal
   work has a one-second cap bounded by its original acceptance deadline.
   Reserve fixed control/refusal bytes from A0's existing separate control
   pool using a qualification-gated `Request::reserve_control` wrapper. Two
   full-size frames may exhaust output capacity; Observe must still work.
   Promote the sealed B2 registry/driver constructors from test-only use only
   behind this admitted frontend. Register canonical origin before actor work.

4. Replace the discard-only driver completion with an ownership-preserving
   consuming seam. F3 must actually join before its result is consumed. Retain
   answer, activity, alarm, replication/epoch attachment, original control,
   witness and F3 owner while terminal proof is checked and the frame is sent.
   Successful and failed-in-turn replies retain existing output-ticket checks.
   Transfer result into the frame before marking output disposed. Allocate
   the bounded frame only after charging its original owner; do not duplicate
   the complete frame. Disposal remains available for cancellation/cleanup.
   Output uses a separate one-shot allowance charged to the same 8-MiB
   invocation debt and actual A0 response pool; it must not enlarge the
   Rule-R 8-KiB owned/channel allowance. A qualification-gated A0
   `Request::reserve_output` delegates to its existing response allocator and
   preserves sticky failure, avoiding a new dependency edge.
   That allocator retains its existing Small fallback semantics generally;
   these guarded owners already classify as non-small, so their output uses
   the primary response pool. Charge aggregate debt once without also calling
   the ordinary working-memory charge for the same frame bytes.

5. The same consuming transport implementation calls A0 `write_response_until`
   with the exact original absolute deadline. Add a narrow `Deadline::at`
   constructor to A0 if needed; it preserves even an already expired Instant
   and never samples a replacement time. Select against canonical activity
   cancellation, checking its current value before writing. Wire a bounded
   notification from the actual core `CancelCellActivity` handling; a diagnostic
   flag or fabricated effect is not cancellation evidence. On success observe
   local flush. On error/expiry/cancel physically drop the transmit future and
   close the socket before recording output disposition. Then dispose frame,
   finish/inactivate activity, retire F3 and request canonical retirement.
   Keep the same Request alive throughout. Flush is not peer receipt.
   Drop every frame/driver JobContext, NodeControl and Request alias before
   waiting for final retirement. Observe it with scalar slot/generation/sequence
   and the process-owned registry. Do not retain even a Weak<InvocationRecord>:
   it would keep the invocation-charged Arc allocation after Request retirement.
   Holding a strong alias while awaiting Free
   would deadlock B2's last-strong-owner predicate.

6. Poll the fixed frontend set, GateReq dispatch and effect/retirement work in
   serving, admitted-request pre-drain and semantic drain. Do not await an
   entire invocation in a way that blocks its host-channel pump. Retain the
   actual dispatcher task/AppHandle/results until join and disposal. Keep the
   separate PublicRequestGuard through output and guarded retirement.
   Its owner follows the retained retirement tombstone through PendingOwner,
   rather than dropping when the frame or invocation record is dismantled.
   Stop admission at shutdown, including public sockets accepted by shutdown
   acceptors; do not route these diagnostic sockets into Hyper. Both Handoff
   and Preserve require guarded-drained. Respect the process failure latch
   and original shutdown cut; incomplete/retained work is reported as such.
   Separate ordinary stop-admission from the B2 process-failure latch: a
   graceful shutdown must allow already admitted work to finish until its
   original deadline or a real canonical cancellation. Three tenant names
   map onto two available execution records; tenant ordinal is not a slot
   index, and a third tenant may use a physically retired slot later.
   Reload remains refused unless actual generation events and cancellation
   satisfy these same obligations. No fabricated RuntimeStopped or release.

7. Emit bounded structured diagnostics in guarded mode; suppress ordinary
   startup text on its diagnostic stdout without changing ordinary output.
   Measure new owned layouts and document scheduler/engine/kernel overhead
   separately. Avoid self-waking scans for persistent aliases; use actual
   notification or a bounded host tick and test retained-owner behavior.

## Proposed source boundaries

Upstream delta: `crates/celld/main.rs`, `main/cli.rs`, new `main/guarded.rs`,
`lib.rs`, `node_log.rs` (readiness observation only), `guarded_node.rs`, `guarded_node/control.rs`, new
`guarded_node/output.rs`, `guarded_node/tests.rs`, `runtime.rs`, and narrowly
the existing actor/production/local_request, host-channel or generation
installation seams where real identities require them. Additional paths need
a concrete finding and plan amendment before editing. Existing asyncrt/LTX
ownership bridges remain unchanged unless a specific integration defect is
reviewed. No dependency/engine/fixture version changes are planned.
`guarded_execution.rs` also promotes the existing sealed Capacity::node_control
from test-only construction for admitted production requests; no replacement
capacity ledger or public constructor is introduced.

Outer delta: `prepare-runtime.py`, `test_prepare_runtime.py`, `check_graph.py`,
`test_check_graph.py`, `runtime-node.lock.json`, `patches/node-series.json`, new
`patches/0004-node-frontend.patch`; plus A0 `src/deadline.rs`, `src/memory.rs`
and `src/memory/tests.rs` for the absolute-deadline constructor and reuse of
the existing response-pool reservation. Full B4 `run-node.py` controller
and `node-guarded.lock.json` are deferred. Freeze exact paths, patch sizes,
source tree, layouts and test names after authoring and before execution.
The accepted B2 union is 24 upstream paths; four frontend/output paths and
the narrow node_log readiness seam make the reviewed union 29, superseding
the old broad plan's 25-path ceiling.
The two added files make a 288-file source tree absent a reviewed amendment.

Tooling adds the closed `node-qualification` stage and its default/lifecycle
regression modes. Preserve accepted B2 profile/series/source/assets as inputs;
require exact append-only preimages. Historical modes retain their identities.
Update the two misleading B1/B2 source-absence tests explicitly for B3, rather
than allowing an old command spelling to evade their assertions. CLI/help
oracles distinguish enabled B3 from default/A2 source.

## Closed frontend and allocation contract

The exact CLI scenario spellings are `cold-sync-retire`, `failed-turn-gated`,
`tcp-write-held`, `tcp-disconnect`, `shutdown-complete`, `shutdown-cut`.
Each process selects exactly one. Header bytes 0..7 are `FOKSGN01`, byte 8 is
version 1, byte 9 is command, byte 10 is tenant ordinal 0..2, byte 11 is phase,
bytes 12..15 are zero. Require peer write-half EOF after exactly 16 bytes.
Begin command 1 requires phase 0 and chooses the throwing fixture only in
`failed-turn-gated`. Its frame is 32 bytes, or 4 MiB for the two TCP scenarios
and `shutdown-cut`. Observe command 2 is read-only: phase 0 returns bounded
scalar state; phase 1 observes actual socket-write Pending only in those three
large-output scenarios. Command 3 (Release) and every other value refuse.
TCP release is the actual peer draining/closing, not an injected completion.
Shutdown uses the actual existing internal shutdown endpoint or process
signal. B4 may add reviewed finite scenarios/barriers; B3 does not pretend
those implementations or full-process oracles already exist.

The native response begins with `FOKSGO01` (8 bytes), version 1, status 0
for success or 1 for an output-gated handler failure, two zero bytes, a
little-endian u32 total length, the actual 12-byte fixture payload (zeroed
for gated handler failure), and four zero bytes. Large frames repeat the
12-byte fixture payload after this 32-byte prefix. Driver or output-gate
failure never becomes publishable status 1. The consuming send returns an
outcome plus a scalar retirement observation; frontend observes retirement
even after a transport error, after dropping every owning output alias.

The following preallocation ceilings are subject to measured layout checks;
actual constructor sums, rather than wrapper sizes, go into stage evidence.

| Owner | Count and charged storage | Lifetime |
| --- | --- | --- |
| Frontend cell/connection future | Four process-owned slots, at most 64 KiB each, precharged once before storage/allocation; captures plus retained future/result overlap included | Accept through physical socket disposal and non-owning retirement observation |
| Header and stamp | Existing four process-owned 17-byte cells for the 16-byte header plus trailing-byte check; exactly one per accepted connection | Accept through validated conversion or refusal/close |
| Guarded GateReq dispatcher | Two process-owned fixed slots, at most 8 KiB each including capture/future/result and any pinned wrapper; original invocation also retains its existing actual future admission; one per invocation, no overflow queue | Before dispatch through actual future/AppHandle/response disposal; separate from reply delivery |
| Cancellation watch | One original-invocation-owned channel, 4-KiB conservative allocation ceiling plus inline sender/receiver layouts; derive the constituent bound below and precharge before creation | Record through actual canonical cancel notification and output disposal |
| Output owner/frame | One per invocation; inline owner/capture/future/result measured against 48 KiB, a separate one-shot frame at most 4 MiB charged to original debt and A0 output pool | F3 join through terminal proof, socket write/flush or physical abandonment and activity finish |
| Retirement observation | Fixed non-owning registry/sequence ticket; no original-control alias | All owning aliases dropped before awaiting Free; PublicRequestGuard remains in tombstone |
| Scalar diagnostics | Existing finite summaries, 128-KiB report and 65,536-byte encoded record ceilings; serialization overlap included | Owned until write/disposal; overflow fails |

The existing Rule-R forward/cleanup allowances do not fund these process
slots or the large frame. Process charges must still fit the existing 1-MiB
role cap. No cap is increased silently if actual layouts exceed a proposal.
Frontend slots are prepaid once and reused only after physical disposal;
do not add a new cumulative process-debt charge for each later connection.
Derive the watch allocation bound from selected Tokio's Shared<bool>,
RwLock/Notify/BigNotify fields and Arc header, not handle sizes alone. If the
derived bound exceeds 4 KiB, refuse before construction. The earlier 256-byte
proposal was rejected during source review: selected Tokio 1.53.1 stores eight
Notify objects in BigNotify plus one Notify in Shared. Sum the rounded actual
public Notify and std RwLock<bool> layouts, three state/refcount AtomicUsize
fields, a conservative optional BigNotify index, two Arc counters, and
per-field alignment padding. Emit constituent sizes/multiplicities and the
derived bound; do not describe handle sizes as the allocation. This narrowly
changes the proposed watch allowance, not process/role/root/frame limits.
Add a
separate closed B3 layout record while retaining B1/B2 schemas and their
stage-specific actual measurements; B3 exclusions no longer say Action is
absent, but retain full-process/recovery/production qualification limits.

The proposed new exact library test inventory uses the existing
`guarded_execution::node::tests::` prefix and these suffixes:
`b3_header_closed_matrix`, `b3_header_eof_and_original_deadline`,
`b3_tenants_share_two_slots`, `b3_stop_admission_preserves_existing_owner`,
`b3_output_reservation_is_one_shot`, `b3_output_owner_survives_pending_write`,
`b3_tcp_pending_then_complete`, `b3_tcp_disconnect_disposes_before_retirement`,
`b3_flush_pending_then_complete`, `b3_cancel_already_set_refuses_write`,
`b3_canonical_cancel_during_write`, `b3_canonical_cancel_during_flush`,
`b3_output_original_deadline`, `b3_bucket_probe_positive`,
`b3_bucket_probe_transport_refusal`, `b3_bucket_probe_contract_refusal`,
`b3_public_guard_survives_pending_owner`, `b3_gate_dispatch_disposal_fence`,
`b3_both_shutdown_modes_require_guarded_drain`,
`b3_persistent_alias_does_not_self_wake`, `b3_frontend_layout_ledger`.
Existing B1/B2 behavioral tests stay; the two source-absence tests receive
explicit stage-correct names/assertions. Changes to this proposed inventory
are documented and source-reviewed before execution.

Native B3 leaf execution is separated from compilation. Compile the selected
two-package `--lib` target using locked/offline Cargo, jobs 2, the existing
target/feature flags and `--no-run --message-format=json` under deny-all
network. Require the exact celld library test artifact and hash. Run its
direct `--list` under deny-all, then its exact reviewed filter with
`--test-threads=1` under a recorded loopback policy. This policy denies all
network except bind/inbound/outbound for two recorded 127.0.0.1 TCP ports,
passed as `FOKS_B3_TCP_PORT` and `FOKS_B3_BUCKET_PORT`. Parent selects and
records those finite port inputs; tests refuse missing/invalid ports, own
listeners/tasks, execute serially and join/close fixtures. Use the existing
BuildSlot/run_owned deadline and group cleanup, without loosening historical
node-baseline or B1/B2 execution. No compiler/build script receives the
loopback test exception.
Artifact selection binds actual celld package ID, `lib.rs`, `profile.test`,
exact features and one regular executable under the pinned target/deps tree;
verify its hash before and after list/run. Port reservations are released
immediately before child binding; this is a recorded port choice, not an
exclusive lease. Collision fails rather than selecting an unrecorded port.
Seatbelt uses its special `localhost` address filter for those two ports;
fixtures bind/connect numeric IPv4 127.0.0.1. This is no IPv6 qualification.

## Review and validation sequence

First review this plan directly and independently against actual B2 APIs.
Resolve findings before implementation. Authors review their components;
root then reviews the integrated diff and agents independently review it
before any candidate test/build. Review corrections before rerunning.
Root alone executes tools/tests/builds and Git, using the existing owned
build slot, pinned toolchain, locked/offline graph and bounded process cleanup.

Focused tests cover fixed header/EOF/tenant/phase refusal and exact original
deadline; frame charging and owner lifetime; actual loopback TCP backpressure
with instrumented poll_write Pending, completion and disconnect; deterministic
test-writer pending flush using the same helper; already-set and later
cancellation, expiry and disposal ordering. Canonical cancellation tests drive
actual core transitions. Test no late publication, no early slot reuse and
retention of late results. Do not treat paused Tokio time as std::Instant.

Readiness tests exercise actual Bucket positive, transport-error and contract-
violation results with finite owned fixtures, under a recorded loopback-only
policy. Build execution remains network-denied. No AWS environment is needed.
Both shutdown predicates and all three pumps receive regression coverage;
tests must distinguish source/component checks from full fresh-process proof.
The actual main/driver/Worker/storage-sync path is wired here; destructive V8,
full core terminal conjunction and complete fresh-process success stay B4.

Run focused Python stage/graph tests, strict reconstruction, selected native
tests with an exact nonzero inventory, feature-enabled and default binary
build/CLI/configured binary Clippy checks, A0 deadline tests and the accepted
28-scenario lifecycle regression. Preserve complete logs and failed attempts.
The existing selected `--lib` command cannot run binary-private tests. Keep
library leaf tests under the existing private node prefix; if binary tests
are needed, freeze a separate `--bin celld` command and exact nonzero test
inventory before execution. CLI probes execute the actual selected binary.
Review results independently before the separate implementation-only commit.
Do not claim warning-free library lint if only the selected binary is checked.

## Completion and later work

B3 acceptance requires reproducible committed source, passing scoped checks,
and a reviewed B4 handoff with honest pending runtime oracles. It does not
establish 15-second host/AZ recovery, tenant density, zero-loss durable ACK,
production metadata fencing, or safe S3 object reclamation. Keep Markdown
unstaged and preserve continuation evidence outside temporary author trees.

## B3 source-review corrections before execution

The complete connection also uses the captured acceptance deadline, including
route acquisition, driver join and output preparation. A timeout around only
the final writer would leave earlier stalls unbounded. The owned wrapper
physically closes the socket on expiry; uncertain activity/driver cleanup
stays held in the existing bank and cannot publish a free slot. This is the
original 15-second clock, not a replacement stage deadline.

The feature-only Action additionally checks the existing local dummy-credential
profile, explicit IPv4 loopback HTTP S3 endpoint and listener ports, an S3
bucket prefix, bucket durability and an absolute fresh data directory. Its
help/refusal oracles are separate from the preserved ordinary CLI oracle.
The binary emits finite readiness/final scalar records through one prepaid
65,536-byte serialization buffer; it does not emit an unbounded per-request
stdout log. Structured readiness remains a component implementation claim,
with fresh-process qualification deferred to B4.

The first native enabled build exposed three integration mistakes: the
GuardedLocalRequest type and NodeRuntime driver-started field/initializer still
had test-only gates, and a shared completion borrow across the socket await
required Sync from an intentionally Send-only response stream. Promote only
the needed qualification-feature items and borrow the sole retained completion
exclusively during transmission. Do not add unsafe Send/Sync implementations,
clone owners, weaken frame ownership, or widen the production feature surface.
Review those corrections twice and reconstruct again before rerunning. The
failed build and its clean process-group disposal remain part of the evidence.

The first TCP leaf runs refused during setup: the connected macOS receive
buffer measured 326,640 bytes despite the pre-connect setting, exceeding the
unchanged 131,072-byte fixture cap. Diagnostic-only reruns established this
before any transfer. Configure the actual connected endpoints, then measure
the same cap. Keep setup inside its original finite timeout. Await initial
socket writability and require a strictly positive, incomplete byte count at
the observed Pending point before the peer drains or closes, so initial
reactor readiness cannot masquerade as buffer backpressure. These are test
fixture corrections; production transport and all ownership/disposal
requirements remain unchanged.

After all 74 native component tests passed, Cargo refused the separate A0
test command because it tried to select a feature on an out-of-workspace
package. Select the actual `foks-celld-guarded` workspace member together with
`foks-hosted-transport`, using the existing member feature, just as the celld
test command already does. The pinned direct dependency already enables A0's
qualification feature. Keep the same manifest, lock, selected dependency
graph, deny-network policy and six-test inventory; update and rerun the
tooling command oracle before retrying the complete enabled qualification.

The corrected member feature selection then reached Cargo's separate rule
forbidding tests of out-of-workspace packages with dev-dependencies. Run the
six A0 tests through the actual FOKS workspace manifest, selecting only
`foks-hosted-transport` with `qualification`. Bind that workspace's lock and
every explicit member manifest before and after execution. Preserve the
existing offline/network/resource restrictions and label these results as
root-workspace A0 regressions: their dependency lock differs from celld's
nested workspace. The native celld tests continue to exercise the nested
feature graph. Neither Cargo failure ran an A0 test.

The first lifecycle retry after the successful root-workspace transport tests
exposed a build-cache collision: the metadata crate emits an un-hashed
`libfoks_metadata_core.rlib`, and the root workspace overwrote celld's copy
with its differently resolved dependency identities. Keep the A0 regression
target in `runtime/target/transport-regression`, inside the existing aggregate
resource ceiling. Preserve and invalidate the exact derived metadata output
and fingerprint files, then rebuild; no trait or Rust source workaround is
needed. The failed lifecycle compilation ran no lifecycle scenarios.

## Final B3 validation

All final native modes used the same 480-image input freeze, SHA256
`ffb35f8383b64ef09c278ad3287188a8dcd8c8357facf8cd7b17ebe29aa1a857`,
and the same 288-file reconstructed source. These are local Apple Silicon
checks with the pinned Rust 1.95.0 toolchain; no AWS environment was used.

| Gate | Result | Evidence under validation runtime/evidence |
| --- | --- | --- |
| Tooling | 59 passed, no skips/errors | `b3-tooling-tests-source-d` |
| Enabled components | 74 celld tests and six separate root-workspace A0 tests passed | `frontend-qsy1jrv4` |
| Default compatibility | Build, exact CLI oracles and configured binary Clippy passed | `frontend-tcbdq9f0` |
| Lifecycle regression | One matrix test completed all 28 closed scenarios | `frontend-1f9r_he3` |

The enabled gate also passed its build, actual CLI oracles and configured
binary Clippy check. Three library dead-code warning groups remain; this is
not a warning-free library lint claim. All 66 recorded commands across the
three final native modes have matching raw output hashes, expected exits and
established cleanup. Artifact images were preserved before mode changes.

Both real TCP leaf tests observed Pending after accepting 4,096 bytes. The
completion case subsequently transmitted all 4,194,304 bytes and flushed;
the disconnect case disposed the socket/frame while retaining the original
owner until physical disposal. Both observed owner release and leaf
retirement. Their explicitly empty native retirement set is not B4 proof of
the full Worker, driver, storage-sync and core terminal conjunction.

Measured typed constructor totals are 18,385 bytes for frontend connection,
3,280 bytes for gate dispatch and 7,672 bytes for output send. The derived
watch allocation bound is 400 bytes within its 4,096-byte allowance. These
measurements fit the existing reserves; they exclude the documented engine,
scheduler, allocator, kernel and total-process RSS overheads.

Direct final results review is recorded in
`/private/tmp/foks-celld-b3-final-results-root-review.json`. Independent source,
input and tooling-result reviews cover every correction before its rerun.
The final aggregate independent review passed in
`/private/tmp/foks-celld-b3-final-results-list-review.json` (SHA256
`9c2268d6474431630c00850e65a525d8f211cdc9cf210d4de5d8a2615df01dd1`).
Failed attempts, diagnostic observations and the 17-file cache
quarantine remain evidence, not successful gates.

The continuation bundle destination is
`tools/foks-celld-guarded/runtime/evidence/accepted-b3-handoff` in the main
checkout. Its verified content-addressed inventory retains the inputs,
prepared source, reviews, raw results and native images, including failed
attempts. `handoff.json` binds the implementation commit and final receipts.
The bundle is ignored by Git. The plan and all other Markdown stay unstaged.

## B4 handoff (not started)

B4 supplies the closed process controller/profile around the actual compiled
`celld foks-guarded-node <scenario>` Action. Start from the existing
`run-node.py`/`test_run_node.py` ownership and fixture machinery. Review the
concrete `node-guarded.lock.json`, controller, tests and preparer/graph changes
before implementation; preserve the four-patch prefix, default mode, A2 fixture,
engine versions and native inputs. Do not duplicate main startup.

Each scenario needs a fresh process with main initializing V8 before Tokio.
Bind actual process handles, fixed deadlines, finite logs and complete group
cleanup. Review the Python/Moto inputs and the full-process port-role policy;
B3's two leaf-test ports do not authorize a node/Moto/shutdown configuration.
Use the closed dummy-credential environment, scoped local S3 prefix and fresh
absolute data directory. Readiness must come from the actual Bucket probe,
matching DurabilityOwner/NodeLogManager/LTX/runtime, installed host channel and
actor/core authority. A listening port or zero counters is insufficient.

The controller sends the existing 16-byte header followed by write-half EOF
and checks exact 32-byte or 4-MiB responses with the actual fixture payload.
Begin and Observe are the supported commands. Release remains unsupported;
progress comes from the peer draining/closing and the real shutdown mechanism.

| Scenario | Required process evidence |
| --- | --- |
| `cold-sync-retire` | Actual restore, Worker adoption, SQL/storage.sync, durability and ownership proofs, F3 join, sole C1 disposition, frame transmission, processed ActivityFinished and Inactive, physical retirement and later slot reuse. Exercise two occupied slots, third-tenant admission and same-tenant refusal at the real boundaries. |
| `failed-turn-gated` | Actual write-then-throw result gets status 1 only after the failed-turn output ticket is proved. Driver, proof or ownership failure cannot publish that status. |
| `tcp-write-held` | Bind actual server write Pending to the same invocation's strictly positive, incomplete accepted-byte count before the peer drains. Retain original frame/activity/request/slot during backpressure; validate all 4 MiB and subsequent retirement. |
| `tcp-disconnect` | Establish the same positive partial-write Pending witness, disconnect the peer and establish socket/frame disposal before release. Distinguish successful disposal from deliberate quarantine. |
| `shutdown-complete` | Stop admission while work exists; continue all three pumps. Both Handoff and Preserve need the canonical drain predicates and guarded physical retirement before reporting success. |
| `shutdown-cut` | Hold output, trigger the finite shutdown cut, report incomplete/retained work and use canonical non-unwinding exit. The parent proves whole-process-group disappearance; exit code alone is insufficient. |

The complete trace must distinguish Wasm-memory release from execution-slot
release and include C2/C6/replica/native cleanup, ordinary Worker/backing-store
destruction, dispatcher future/result disposal and original Request retirement.
B3's transport leaves use an explicitly empty native set; their retirement
checks do not replace these process observations.

There is a concrete observability gap: Observe reads active records, not held
retirement records, and `write_pending` is sticky. A received frame or empty
Observe result does not prove Free. The final aggregate report also lacks
per-invocation F3/C1/destruction witnesses. If needed, design and review a small
bounded observation record tied to the actual invocation sequence and owned
transitions. It must not retain owning aliases while awaiting retirement or
manufacture completion authority.
The production Pending bit alone cannot distinguish initial reactor readiness
from a full send buffer and does not report byte progress. The full-process
held/disconnect oracle needs the same invocation's actual Pending plus a
strictly positive, incomplete accepted-byte count, or a separately reviewed
equivalent, before peer drain/close. B3's strengthened leaf witness does not
automatically provide that production observation.

Exercise the original 15-second acceptance clock at header, route/restore,
build, sync, F3, C1, output and retirement waits. Canonical CancelCellActivity
must drive cancellation. On timeout or ambiguous cleanup, require socket
closure and no late publication; retained capacity is not successful
reclamation. Keep PublicRequestGuard through pending-owner/failed retirement.
A deterministic pending-flush writer is not evidence of TCP flush backpressure.

The broader G matrix in `HOSTED_CELLD_GUARDED_NODE_PLAN.md` remains open:
next-process restore, held or abandoned sync/proof/effects, binding and panic
failures, and stop-after-memory cases need their own reviewed barriers or
scenario additions. B3 drops the reload receiver and skips the pointer watcher.
Reload/generation-adoption qualification therefore needs a separately reviewed
actual event and cancellation path before enabling it.

Runtime questions still to resolve include driver/resident-sync pump progress,
delivery of all terminal core events, exhaustion of original aliases before
slot reuse, and the distinction between error disposal and quarantine. Preserve
failed evidence and review the smallest correction before rerunning; do not
weaken the expected observations to make a scenario pass.

Acceptance binds source, graph/features, binary, fixture/profile, argv,
environment, network policy, raw frames/reports/logs, clocks, actual ownership
observations and cleanup. Preserve binaries before another mode overwrites
them, and independently review results. Local bucket/restore qualification
does not establish AWS/AZ recovery within 15 seconds, simultaneous recovery or
tenant density, production metadata fencing, zero-loss acknowledgment or safe
object reclamation. The production backend decision and actual FOKS Wasm/SQL
integration remain separate work.
