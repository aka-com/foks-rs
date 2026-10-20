# Celld turn scheduler cancellation repair

The accepted scheduler can lose its active token when a queued acquisition is
notified and then cancelled before its next poll. `release` sends a unit value,
but `acquire` creates the RAII permit only after receiving it. Cancellation in
that gap leaves `current` occupied forever. Root and independent source review
confirmed the defect. Its connection to the earlier case-21 stall is unproven.

Use a checked, monotonically increasing `u64` ticket under the existing mutex.
Current, queued and pending/returned permits carry the same ticket. Establish
the permit before the first await. Cancellation removes only its own queued
ticket or releases its own current ticket; an already-skipped ticket is a no-op.
Preserve round-robin lane order and unit notifications, avoiding destructor
reentrancy under the scheduler lock. Exhaustion must fail before registration;
poisoning remains fail-closed. No per-turn heap allocation is needed.

1. Add a private deterministic regression to the exact accepted source. Manually
   poll A to acquire, B to pending, drop A, drop notified B without repolling,
   then require C to acquire immediately. Review twice, then capture the exact
   expected baseline assertion failure with owned cleanup. No timing sleeps.
2. Implement the ticket fix and add cancellation, successor, normal handoff,
   fairness, repeated-cycle and exhaustion tests. Review twice before testing.
   Preserve the initially failing regression unchanged. Check actual layout
   costs against existing bounds.
3. Publish a separate native patch and required pins/qualification tooling.
   Capture a separate exact scheduler suite alongside existing 123 guarded,
   six transport, 28 lifecycle and one binary-layout checks, plus tooling tests.
   Preserve the 128-KiB retained and 12-KiB main-work caps. Independently review
   actual results before committing the native component.
4. Refresh process bindings and run the original full 101-case matrix against
   the repaired component. Review results twice before committing process
   implementation files. Preserve earlier failures and causal uncertainty.

The ten-trial baseline experiment stopped at launcher preflight before any
server ran; its failed scope remains evidence. The outer owner must be launched
directly with pinned Python `-I -S -B` if that experiment is revisited. Its
expected module inventory must not be weakened.

No protocol, cryptographic or wire-format change is planned. Deadlines and
resource limits remain unchanged. All Markdown stays unstaged. Local evidence
does not establish AWS/AZ recovery, zero-loss durable fencing or tenant density.

Detailed private plan: `/private/tmp/foks-celld-b4-p3-turn-scheduler-plan.json`.

## Stage A result — 2026-10-19

The unchanged accepted production source plus the deterministic regression
failed the intended assertion with libtest exit 101 (zero passed, one failed,
zero ignored). Metadata validation passed; both command cleanups completed.
Raw logs, the original executable, and the input closure are preserved under
`/private/tmp/foks-celld-b4-p3-turn-scheduler-baseline-build/`. The enclosing
receipt is SHA256
`2dfd0564c2c5beac457eb2d4e2eb4323c69ae467b1bf32405c253a80502c7717`.
This demonstrates the scheduler bug; it does not attribute the previous
case-21 failure or qualify the process matrix.

## Packaging constraint discovered during Stage B

The immutable ten-patch prefix totals 1,616,273 bytes; its existing revision-2
limit is 1,617,920 bytes. An additional scheduler implementation and regression
suite will exceed the remaining 1,647 bytes. A narrowly scoped plan amendment
is under review: keep the historical prefix and its original limit intact,
and admit exactly one separately bounded scheduler patch with reviewed
bytes/hash and the single `pool.rs` before/after roster. This concerns source
packaging only; runtime memory, request deadlines, and test limits stay fixed.
The actual new patch size must be recorded before publication/testing.

Both source reviews of the repair passed. The regression function remains
byte-identical; six additional tests cover cancellation, handoff, fairness,
repetition and ticket exhaustion. Focused execution is pending.

The packaging amendment passed root and independent review. Patch 0011 is
18,720 bytes, SHA256
`8520441e67ee1520f6ed19a500e096ad83f561530fab899e9016ca23e057e2e2`.
The successor's fixed aggregate source-patch allowance is **1,634,993 bytes**.
The historical ten-patch cap (1,617,920 bytes), per-patch cap (524,288 bytes),
and all runtime/deadline/build limits remain unchanged. Any change to the new
patch invalidates this exact binding and requires another review.

## Stage B focused result — 2026-10-19

All seven scheduler tests passed, including the byte-identical regression that
failed against the original scheduler. The three existing failed-turn, deadline
and layout tests passed, as did default and enabled builds. Each command
completed with owned cleanup. Receipt SHA256:
`19295f4a76aca7a65ea719e3867f1f42fbdf7eee9db20cb6bb660aa9a2f337a6`.
Both root and independent actual-result reviews passed.

On the pinned 64-bit build: scheduler state grows 104→120 bytes, full scheduler
120→136, queued row 8→16, permit 8→16, and acquisition future 96→104. `Slot`
is 920 bytes and its actual allocation size is already charged by the runtime.
The repair adds scalar ticket storage without another per-turn allocation.
The full native/tooling/lifecycle/binary-layout qualification and process101
remain outstanding. No component has been committed or accepted yet.

## Stage C tooling fixture correction — October 19

The first 122-test tooling run failed with 206 subtest errors, all patch-context mismatches. The new foundation fixture overwrote the existing three-line `pool.rs` preimage with one line. Native qualification had not started. The failed receipt and raw log remain at `runtime/evidence/b4-turn-scheduler-tooling-1187780811402666` in the validation worktree.

Removed only the redundant comment and `extra.setdefault` entry. Root and independent source reviews passed. Successor B launchers change only freeze filenames, preserve the previous closures except the corrected canonical test descriptor, and retain the same 122 test IDs, deadlines, bounds, and cleanup. The new freezes contain 2,547 native images and 314 tooling images. Original failed-run inputs remain immutable. The corrected test run is pending.

The corrected tooling run at `runtime/evidence/b4-turn-scheduler-tooling-1188057501680083` reached the original 60-second deadline (60.1906 seconds including cleanup). Its log records all 116 historical tests passing, then interruption during the first new scheduler test. Inputs remained identical and no observed descendants remained. This is a failed full run, not acceptance of the 122-test suite.

The proposed bounded grouping amendment separates the original 116 tests and the six new scheduler tooling tests into two closed groups, each with the original 60-second limit. Both must complete successfully with the same source and input checks. The aggregate scheduled tooling budget becomes at most 120 seconds; production request, node, case, and recovery-matrix deadlines do not change. No fixture cache or production validator semantics change on the basis of this timeout.

Both reviewed fixed tooling groups passed: historical116 in 51.2587 seconds and scheduler6 in 21.1171 seconds. The exact disjoint union is 122 tests, with zero errors, failures, or skips; both stayed within their own 60-second deadlines. All 328 frozen tooling inputs rehashed and both before/after maps matched; no observed descendants remained. The two earlier failed runs retain their original verdicts. Native preparation follows actual-result review.

Stage C native progress: prepared source matches all 289 reviewed entries, with only `pool.rs` differing from accepted background-retirement source. Default and enabled builds and Clippy checks passed. The enabled run passed exact 123 guarded tests, seven scheduler regressions, and six transport tests. Its server and library-test executables and the default server are preserved with exact hashes and mode0555. All 28 lifecycle scenarios passed; the separate binary-layout mode is running. Root and independent result review continue before publication and implementation-only commit.

## Stage C complete — implementation committed

All component checks and both actual-result reviews passed. Acceptance is `runtime/evidence/accepted-b4-turn-scheduler/receipt.json` (SHA256 `0013f5d8a1d1a4e507f902fd98c2663539c92bba2e3118f29cd594d5e5baa1ff`), with 572 aggregate images and 289 copied source entries verified. Component acceptance does not grant full101 or host/AZ15s qualification.

Main commit: `2fb1e261313580622ec2fc8062f1be89477571c6`. Validation commit: `47a505e9e56e7f32bfe2d9b92498f6fb3c0c634d`; both have identical tree `e567d596f278ae423a0597df714d54c409b3feab`. Exactly seven implementation files were committed. Markdown and P/G/H remain unstaged. All 2,547 native input hashes still match after committing.

Stage D C plan passed root and independent review. The ready profile/I42 producer passed both structural reviews. Review of the final F/D producer found two issues before execution: native wrapper elapsed times require the finite-float parser, and the producer binding hash must be supplied independently of producer source to avoid a hash cycle. Corrections are being reviewed before any data publication or process run.

Stage D ready-data publication passed both reviews: P now contains 358 frozen inputs (all original349 plus exact six component and three compatibility entries); I42 retains256 entries, changing only indices0/4. Four original authorities were preserved. The fresh unchanged42-test controller run (`b4-p3-unit-1189260317067500`) passed with both result reviews, no errors/skips, identical before/after input maps and complete process cleanup. Final F/D data preparation remains pending; no full101 run has started for the scheduler repair.

Stage D final input publication passed root and independent actual-result review. F has 3,917 rows (SHA256 `12494c48b5114667b283814d345d8cc3604a5a9c8793c84cecd941b3c1034926`); D has 3,918 (SHA256 `e7e76a2691d0e439e44ecc05ae3be45278b1a55d34ec36b890ee29c24c6ae33a`). All 3,783 historical input tuples remain preserved through exact mappings. All 52 evidence scopes were checked, along with the native provenance and 572 aggregate rows. The unchanged 4,096-row/2-MiB limits hold. Full101 launch review is in progress for fresh scope `node-guarded-scheduler-all101-c74d19`; no process acceptance is claimed.

The reviewed full101 run `node-guarded-scheduler-all101-c74d19` stopped with exit1: cases1–22 passed, case23 failed, and24–101 were not run. Outer result SHA256 `0ea0967a18efb4ee20a6e4a1b80e40568e341b1c906ee23c9275853edb9745e8`. Inner/outer cleanup completed and all input rechecks passed. Case21 now passed, without establishing the cause of its prior stall. Case23 expired at its original deadline and closed the response socket, then failed the retained-owner assertion because `held=0`. Native source counts active `bank.records` separately from retirement `bank.held`; this case has one record, phase4/refusal6, and public_in_flight1. A source-bound oracle correction is under review. The failed run remains immutable; no retry, full101 acceptance, or process commit has occurred.

### Case23 oracle correction plan

1. Preserve six current authorities and the complete failed-run inventory. Keep the native component, P, all101 case IDs, all42 unit-test IDs, imports and deadlines unchanged.
2. Add one retained-owner validator shared by cancellation/expiry, panic/binding, and peer-close checks. Require the exact original record in the final snapshot, an admitted non-Free phase, admission public-guard provenance, no original PublicGuardDropped/BankFree, and positive records-or-held plus public counts. Aggregate counts corroborate ownership; they do not select the owner. Preserve each branch's stricter phase/refusal rules and the panic branch's four release-event exclusions. Native archive ordering is the source basis; do not claim atomic snapshots.
3. Extend existing deterministic tests with the actual case23-shaped retained record, held-retirement and mixed-tenant cases, and negative missing/stale/substituted/released-original cases. Review source twice before installation/execution, run the unchanged42 suite, and review actual results twice.
4. Refresh only I42's G/H descriptors and the bounded F/D input closure. Preserve every old F row through exact changed-path mappings; freeze a complete failed-run inventory without flattening it. Reuse existing owners, stay below4096 rows/2MiB, and keep post-publication results outside their own input closure.
5. After reviewed publication and launch, run a fresh unchanged101 matrix; review actual results twice before the implementation-only commit. The previous failed run remains failed.

Private correction plan B is `/private/tmp/foks-celld-b4-p3-retained-owner-plan-b.json`; closure plan is `/private/tmp/foks-celld-b4-p3-retained-owner-closure-plan.json`. Root review passed; independent plan review is completing before any source change.

Correction source C passed root/independent source and launch reviews and was installed as G/H only; P stayed identical, I42 changed only rows2/11. The first42 run (`b4-p3-unit-1190585089668791`) passed every child test but failed outer cleanup verification because the sandbox denied `/bin/ps`. That failure is preserved. The identical reviewed command with process-inspection access passed in `b4-p3-unit-1190606253003333` (receipt SHA256 `d030ed5cb4a3ba86d8291e40aee1c16f5dd247290f4324ce9f7c27f5b45113b1`). Both actual-result reviews passed, including all256 input hashes/modes and actual empty process group. Final bounded F/D publication is pending.

The first fixed F/D publisher stopped before mutation: it compared qualified I42 test IDs with unqualified P names. Producer B changes only that comparison to an exact fixed prefix and fresh source/binding paths; both reviews passed. Publication then succeeded, with F3,949 rows (`d846187b6321ab047bf5b32c256b83aa830976be77c97acae6e544911c641b0b`) and D3,950 (`e38ab5c4be02c47422698c875fb354b738a24aa4f56daeb59e5971aadf35b27a`). Both actual reviews rehashed all3,950 inputs and preserved all3,917 prior rows plus the complete failed101 scope. Fresh full101 launch review is finishing for `node-guarded-retained-all101-9b2ac4`; no matrix acceptance yet.

### Process-event wait correction

The fresh run `node-guarded-retained-all101-9b2ac4` passed cases 1–11 and failed case 12; cases 13–101 were unrun. Both cleanup layers and input checks passed. The raw sequence shows reload construction followed by shutdown and reload failure, with no processed generation. Case23's retained-owner correction was not exercised in this run.

Root and independent audit found four callers returning `stream.process(...)` lists to `CaseRun.wait`, which accepts any result other than `None` or the singleton `False`. An empty list therefore completes the wait. The other seven callers use valid booleans or None/nonempty records.

The next correction wraps exactly four process-event predicates in `bool(...)`: reload generation processed, header entered, header expired, and old generation drained. Keep the general wait contract unchanged. Extend existing tests to drive actual execute/wait paths with deterministic event barriers and an unchanged fake deadline; verify no shutdown, response disposition, or Begin occurs before its event. No native change, new case/test ID, import, or deadline change.

Preserve all six authorities and the complete failed-run inventory. Reuse the successful fixed data publisher with exact context updates, retaining all 3,949 prior rows. The same 35-role bound permits at most F3,984/D3,985, within existing caps. Root plan reviews passed; independent reviews are pending before source authoring. Both source and actual-result reviews remain required before any implementation-only commit. Markdown stays unstaged.

Process-wait source A passed both source/launch reviews; installed exactly four bool wrappers and deterministic tests in three existing methods. All 42 tests passed in `b4-p3-unit-1191167371782000` (receipt SHA256 `c0b6d3aaebceeb2839e1938d6d59df1bdbf236f9e1a4c64c9b5dae1040b05969`) with child exit0 and verified empty process group. Root actual review passed; independent review is finishing. P is unchanged and I42 changes only its G/H rows. F/D publication and the next full101 remain pending.

Process-wait F/D publication passed both actual reviews: F3,981 (`c9e33773af0fee3604167feb4d7cfc11045401938bc2153e9c0a501fca2e694c`), D3,982 (`51994d3160133cbf69906f17afdb8f3050cdd689b929d02e25062d9cf9b4448e`). All3,949 prior rows and the complete failed case12 scope remain preserved. The fresh full101 `node-guarded-wait-all101-53ed7a` is now running after both launch reviews, with unchanged native component and original deadlines. No matrix acceptance or implementation commit yet.

### Parent cancellation completion correction

The reviewed process-wait run passed cases1–24 and failed25 after Advance acknowledgment at the original15s request deadline;26–101 remain unrun. Inputs remained unchanged. Inner completion evidence is missing; outer physical cleanup succeeded. Complete raw failure evidence and all six authority preimages are preserved.

Root and two independent source reviews identify a missing transition: ActorControl Cancel processes core Event::Cancel, which need not emit CancelCellActivity. Only the latter currently cancels the original invocation and releases its qualification barrier. The planned repair shares the existing actual invocation-cancellation completion between both validated paths, without changing core cancellation semantics, fabricating resource release, or weakening the process oracle.

Plan: reproduce with a deterministic real-control regression; implement the shared completion and identity/idempotency tests; qualify and commit the native component; then refresh bounded process inputs and run the unchanged101 matrix. Each stage requires root and independent source and result reviews. Details: `/private/tmp/foks-celld-b4-p3-parent-cancel-plan.json`. No implementation change or test execution yet. Markdown remains unstaged.

Parent-cancel Stage A reproduced the exact expected assertion in `guarded_execution::node::tests::b4_parent_cancel_completes_original_invocation`: one failed test, exit101, no timeout/compiler substitution. Metadata and both owned cleanup checks passed; all2,850 frozen inputs remained identical. Receipt `/private/tmp/foks-celld-b4-p3-parent-cancel-baseline-build/evidence/receipt.json`, SHA256 `784243d043fb933be0449d62d562aa6a93705680a746a210fb83b649fb98cfa1`. The exact executable is preserved mode0555. Root result review passed; independent review is completing.

The exact Stage B design also passed root and independent review. It keeps the baseline regression unchanged and adds five tests for gate-pinned cancellation, idempotency, invalid delivery/identity, invocation isolation, and notification after lock release. Full foundation identity validation precedes record locking; locked identity capture precedes state changes; watch/qualifier/cleanup notifications follow guard release. These component tests do not claim a live Worker or native retirement proof. Source authoring follows the second baseline result review.

Both baseline result reviews passed. Stage B first repair attempt stopped at compilation with two E0599 errors: new assertions called `observation()` on NodeControl instead of the retained OriginalOwner. No repair tests ran; owned cleanup and all 2,857 input checks passed. Failed receipt SHA256 `73af877d3a39cb3f75cacc6d6090306db74bac7553993eb861e5ff7ba4656b88` remains immutable. Root and independent source reviews missed these test API mistakes. The reviewed correction changes only those two test calls in a fresh B source tree; production and the full original regression block remain identical. Both corrected-source reviews passed. Fresh B owner/data retain the failed attempt, preserve all twelve tests and fifteen commands, and are completing binding/launch reviews before execution. No canonical native change or new commit yet.

Corrected Stage B run passed: metadata, twelve exact tests, default and enabled builds (15 owned commands), all cleanup established and all2,867 inputs/297 generated files stable. Receipt `/private/tmp/foks-celld-b4-p3-parent-cancel-repair-build-b/evidence/receipt.json`, SHA256 `72c55e2e0cd80ad208c57ffee0bd6ff1097d02e4b7072e0ccbc44f97b99a7a34`. Actual shared twelve-test executable is preserved0555 separately; root actual review passed and independent review is underway. Stage C packaging plan passed both reviews: additive0012 over immutable first11; preserve historical123/current129 tests, seven implementation files, exact additive patch-byte cap, three fixed tooling groups116/6/6 at60s each, four native modes and unchanged component receipt schema. Source implementation waits for the second Stage B actual review.

Both Stage B actual-result reviews passed, including independent rehash of all2,867 frozen and297 generated inputs, all15 completed process groups absent, exact original regression retained, and actual test executable preserved. Stage C private authoring is authorized by the reviewed plan `/private/tmp/foks-celld-b4-p3-parent-cancel-stage-c-plan.json` (SHA256 `9d68d0e49c931de38e743dde4aef18c714d5e5ce580ab26e5de68cb3a6741f14`). Production packaging and two tooling test modules are authored separately; root adapts the existing native/tooling/publication owners. All canonical implementation files remain unchanged pending source reviews.

Stage C final source passed root and independent review. Patch0012 is27,740 bytes (SHA256 `b41c685c3574168fd04b7024ac0cfd4bec2bc908d4f6c23789e4f4ace62f470a`); exact aggregate packaging cap1,662,733 preserves predecessor1,634,993 and runtime limits. Seven implementation files and four immutable predecessor metadata copies were installed in validation; canonical-install receipt SHA256 `2b2d1a2110e2a24bfd7ddceaccd2ea5b313ecf514d1b11e925264b47414441bc`. Six previous canonical files are separately retained. Native source reconstruction has not run yet. Both indexes remain empty; Markdown remains unstaged. Final source freeze/launch review precedes128tooling and native qualification.

Stage C tooling128 passed in exact sequential groups116/6/6 within original60s each (49.086s/19.951s/12.709s); both actual-result reviews passed. Nativeprepare reconstructed exact289 files and preserved predecessor; default/enabled build+Clippy passed, including129guarded+7scheduler+6transport. Actual default/enabled/server/libtest artifacts retained0555. Lifecycle and layout modes remain in progress before native/component acceptance. Stage D detailed planB passed both reviews after adding an explicitly budgeted binding-file role:83prospective additions, F<=4064/D<=4065 under unchanged4095/4096 ceilings. OriginalplanA is retained. P361/I42twoindices, exact42/101 and later original8/all4SDKworkflow remain gated on actual accepted component.

The final parent-cancel Stage C layout mode passed. Root and independent actual-result reviews verified all four native modes, 129 guarded tests, 7 scheduler tests, 6 transport tests, 28 lifecycle scenarios, the separate layout test, all 3,167 frozen inputs, all 289 source files and absence of all 85 recorded process groups. Tooling128 already passed both result reviews. The fixed publisher rejected one mistaken relative-path invocation before mutation; its failure record is retained. The reviewed absolute invocation then published `runtime/evidence/accepted-b4-parent-cancel/receipt.json` (SHA256 `59854a5352afe1040cb0752a461102d1cb6ab9ec3897ab2e03ec798bdf5bac21`). Root publication audit verifies all 572 aggregate images, copied source identities/modes and preserved binaries. Independent publication audit and the seven-file implementation commit are next. Process101 and host/AZ15s remain unqualified.

Both publication result reviews passed. The seven implementation files are committed in validation `942bcb45eef4410b23fe868829f030aca9e721ee` and main `385eb80004b352b83fa43e86db918d234dcb0471` with identical tree `908808819e5f18c615c6b566270b16a20990cf63`. Markdown remains unstaged; P/G/H remain uncommitted pending Stage D. The new stage retains the failed25 evidence and uses the approved83-role finite projection, fresh42/full101, then required8/all4 SDK/HTTP follow-through.

Stage D compatibility passed root/independent reviews against the actual accepted three289-file source trees. Five prerequisite source files and scoped HTTP/startup routines remain identical; guarded_node.rs whole-file change is correctly retained. Ready source A had a review-schema mismatch caught before execution; B fixed exact actual publication schemas, C bound six actual descriptors and passed both source/launch reviews. C data publication passed both result reviews: P361 (SHA256 `f506520a7e9ebcbd04bec0767ca7dcc962e43e3a0f25d253360151e0aa703b1d`) and I42256 (`eb3f83807ffca5570b933f4b5a3fee10839bfe0c5f071c44f9cdb473fd92d302`), only six component replacements/three compatibility anchors and two I42 slots. Fresh original42 passed in `b4-p3-unit-1195868197938958`, receipt SHA256 `9d89905c1b64b6ba1efd234a1ce2acacbd9b7d0c59c9d99b95af6b4ee70fa0de`; both actual reviews verified complete raw logs, IDs, closure and cleanup. Suite data source A had an accepted-component inventory root error caught before execution; B corrects prepared-inputs to the full accepted scope, preserving A. Root B review passed; independent review and explicit data assembly are underway. Required later SDK/HTTP helper still polls /state before the normal-listener marker; an explicit correction/qualification plan is being prepared, without changing existing strict200/identity or deadlines.

Stage D final data publication passed both actual-result reviews. F/D now contain4,062/4,063 rows (SHA256 `256af6bb1f56c31b48977260731e607a2169206b0e1b4f01a1114d0ea1985526` / `83a0244932605fecda00bbbf274190f4064e326b74dac92a07a35cf2c02e90f5`), retaining all3,981 logical predecessor rows through exactly eight preserved projections. Both reviews rehashed all78 historical scopes (8,007 files,1,406 directories,3,304,675,749 bytes), current native/tooling closures and exact published objects. Fresh full101 launch `node-guarded-parent-cancel-all101-b18e7c` is prepared for independent review with unchanged owners, deadlines, caps and cases. SDK readiness source B passed two source reviews; its epoch preparation remains a private non-runnable draft gated on actual full101 success and result reviews.

Fresh parent-cancel full101 terminated unsuccessfully: `node-guarded-parent-cancel-all101-b18e7c` passed1–20 and failed21 (`sync-held-after-proof`);22–101 were unrun, including the repaired25. Outer receipt8,104 bytes SHA256 `ddf5a486fb670ae432e470049d96ed3f4d372c2654da56b6e82332d8101f5a0c`. Readiness succeeded in~103ms, then Begin produced no selected checkpoint9 or response. Sync/Compaction reported shared original-owner failure~126ms after host initialization; the initiating site is not captured. Case21 expired at the original90s node deadline, inner terminal cleanup proof failed and the node was killed; outer physical cleanup succeeded. Root and independent actual-failure reviews verified all4,063 frozen inputs, the full657-file/170-directory/11,997,192-byte scope, raw checkpoints and absence of all80 recorded process IDs. Inventory is `/private/tmp/foks-celld-b4-p3-parent-cancel-stage-d-failed21-inventory.json`. No retry, process commit or SDK execution. Read-only source analysis rejects an automatic AuxFinish context mismatch or mutex-contention explanation for the new identity check; cause remains unproved. Narrow first-failure diagnostics are being designed before any source change or further execution. SDK epoch and Q archival plans passed both structural reviews but remain gated on full101 success.

The private first-failure diagnostic plan passed root and independent review: `/private/tmp/foks-celld-b4-p3-parent-cancel-stage-d-first-failure-diagnostic-plan.json`, SHA256 `1b08c38fc530c367e9acb5da2d2210508a6a788d6492cb50691fd3f390a54d7e`. It replaces the qualification-only Request failure flag with one atomic scalar recording the first actual failure transition, covers all four existing failure stores, preserves process/invocation causes separately and emits only outside guards. No wire or production behavioral repair is inferred. Private exact transport/native copies, six focused diagnostic tests plus three existing checks, exact Cargo graph relocation and one original-owner isolated21 run are required. All runtime limits remain unchanged. Current P/G/H/I42/F/D are additionally preserved under `/private/tmp/foks-celld-b4-p3-first-failure-preservation/` (receipt SHA256 `e25b51c45d28def729d0e2984d232363e178a46df71a657241eab3691e188244`). Private source authoring has begun; no diagnostic execution or implementation commit yet.

Private first-failure source A passed root and independent reviews: transport8 (two changed files) and native289 (three changed files; prior tests prefix unchanged). The nine-test build owner and fixed isolated21 owner also passed structural reviews. Final private build input assembly is being reviewed; no candidate execution or new implementation commit yet. Diagnostic capture preserves existing failure/cancellation semantics, runtime caps and deadlines; full101 remains failed21 and SDK qualification remains gated.

First-failure private build A stopped before compiling test1. Locked offline metadata matched the accepted graph, but Cargo rejected testing `foks-hosted-transport` because it has development dependencies and is outside the private workspace. Both source reviews missed that distinction. Receipt SHA256 `bd821cd643086949ff6e8e9565f301f98771342858f63de414b3aaa621d9786b`; all3,498 frozen inputs and305 generated files remain unchanged. Both owned command groups exited with cleanup established; no tests or enabled build ran. The failed scope is retained. A fresh private-workspace correction is being planned; no canonical code change or diagnostic process execution.

The fresh B build correction plan and source passed both reviews. It adds only private transport workspace membership, preserves the original root as default member, and permits only the existing declared dev-Tokio edge in normalized metadata; all versions, features and other graph fields remain exact. B uses unchanged diagnostic Rust sources and all original nine tests/eleven commands. Its input assembly retains all3,498 failed-A inputs plus thirteen finite additions (planned3,511), with complete failed313-file and prior657-file scopes checked before/after. The isolated21 owner and data recipe passed two structural reviews; a descriptor-mode mismatch in data recipe B was caught before execution and fixed in preserved successor C. No process diagnostic or implementation commit yet.

Private diagnostic build B validated the exact approved metadata delta and passed all six new Rust tests. Its owner stopped on test6 output parsing: the expected layout println occurs between the libtest name and terminal `ok`. This is a harness parser failure; tests7-9 and the enabled build were unrun. Actual measured layout is Request136, ArcRequest152, Control96, NodeRuntime352. Both result reviews verified3,511 inputs,305 generated files,970 prior preserved payloads and absence of all seven process groups. Receipt SHA256 `bc2dbc8b92a93abe1a77587b0ae0f988f4e5017e2790351d9a0e6620c52d154c`; B inventory328files/62dirs SHA256 `9ebbff9110d85851ac36217ed0e18b8f254b9e310eeccaaa37deb114e944c816`. Exact actual transport/native test executables are retained0555 outside the frozen B scope. A narrow C plan preserves Rust source and all tests/limits while validating only this exact test6 layout/ok structure. No isolated process case or new commit yet.

Private diagnostic build C passed all eleven commands: approved metadata check, all nine focused tests, and enabled server build. Both actual-result reviews rehashed3,527 frozen inputs,305 generated files,1,298 historical payloads/294directories, all raw commands/results, and confirmed all eleven process groups absent. Receipt SHA256 `5a65c640bdfb0ca356b246f82a3319b1fcc5fc509d10f60a10b0deff2aabff80`; immutable binary159,896,864bytes SHA256 `65efada1703461118f18d6510c797f7906cc65325a81bb64c9664b331169511e`. Exact transport/native test executables are retained0555. C corrects only the precise sixth-test output parser; earlier A/B failures remain preserved. The fixed isolated21 owner is being bound to these actual results; no process diagnostic executed yet and no new implementation commit.
