# Current continuation status — 2026-10-19

The October 17 handoff below is historical transfer guidance. Native follow-ups since that archive are committed through main `385eb80004b352b83fa43e86db918d234dcb0471` (parent cancellation propagation). Validation is detached at `942bcb45eef4410b23fe868829f030aca9e721ee`, with identical tree `908808819e5f18c615c6b566270b16a20990cf63`. Markdown remains unstaged; process profile/controller/tests P/G/H remain uncommitted in validation.

Latest full101 run `node-guarded-parent-cancel-all101-b18e7c` passed1–20 and failed21 (`sync-held-after-proof`);22–101 were unrun. Readiness succeeded, but shared original-owner failure appeared before the selected checkpoint9. The original90s node deadline expired; inner terminal cleanup proof failed and the node was killed. Outer physical cleanup succeeded; both failure reviews verified all4,063 inputs, complete657-file/170-directory scope and absence of80 recorded process IDs. Outer receipt SHA256 `ddf5a486fb670ae432e470049d96ed3f4d372c2654da56b6e82332d8101f5a0c`. Initiating fault remains unproved. A private atomic first-failure diagnostic passed source reviews. Its first build passed metadata but stopped before test compilation because Cargo rejected testing the non-workspace transport dependency with dev-dependencies. The failed build is preserved (`first-failure-build/evidence/receipt.json`, SHA256 `bd821cd643086949ff6e8e9565f301f98771342858f63de414b3aaa621d9786b`). Both actual-failure reviews passed. A twice-reviewed correction makes transport a private workspace member, permitting only that exact metadata membership and declared dev-Tokio edge change; fresh B authoring is underway. No diagnostic process execution, process commit or SDK execution. The earlier failed25 remains preserved and its native repair is committed.

Root and two independent source reviews identified a missing cancellation transition: ActorControl Cancel drives core Event::Cancel, which need not emit CancelCellActivity. Only the latter currently cancels the original invocation and releases the selected qualification barrier. A barrier-only acknowledgment would be insufficient: the invocation cancellation watch/runtime/control must be updated first. The reviewed plan is `/private/tmp/foks-celld-b4-p3-parent-cancel-plan.json` (SHA256 `09d81b5e46c003e3c9917075970db2d2600fc96740141e076174b61535269bcf`). Stage A reproduced the deterministic actual-control regression with the exact expected assertion/exit101, successful metadata validation and cleanup. Both actual-result reviews passed. Receipt `/private/tmp/foks-celld-b4-p3-parent-cancel-baseline-build/evidence/receipt.json` has SHA256 `784243d043fb933be0449d62d562aa6a93705680a746a210fb83b649fb98cfa1`; the executable is preserved mode0555. Stage B repair attempt A failed compilation on two test observation receiver calls; no repair tests ran. Its receipt (`73af877d3a39cb3f75cacc6d6090306db74bac7553993eb861e5ff7ba4656b88`) and inputs remain preserved. A reviewed two-call correction in fresh source `parent-cancel-repair-author-b/source` passed both source reviews; production remains unchanged. Fresh B validation passed metadata, twelve tests and both builds, with cleanup and input stability verified twice. Receipt SHA256 `72c55e2e0cd80ad208c57ffee0bd6ff1097d02e4b7072e0ccbc44f97b99a7a34`. Stage C is now installed, tested, twice reviewed and committed as the seven-file parent-cancel implementation. Tooling128, guarded129, scheduler7, transport6, lifecycle28, layout1, default/enabled builds and Clippy all passed. Both actual reviews verified all3,167 frozen inputs, source289 and physical absence of85 process groups. Published component receipt `runtime/evidence/accepted-b4-parent-cancel/receipt.json` has SHA256 `59854a5352afe1040cb0752a461102d1cb6ab9ec3897ab2e03ec798bdf5bac21`; both publication reviews rehashed aggregate572 and source/binaries. One mistaken relative-path publisher invocation was rejected before mutation and retained as provenance. Stage D planB (`/private/tmp/foks-celld-b4-p3-parent-cancel-stage-d-plan-b.json`, SHA256 `fec72e42929ded5fe87981bc397ac348f794cc2573283c23fcc12c2422cba916`) passed both reviews; private compatibility/ready/suite data work is underway. It preserves3981 prior logical freeze rows, budgets83 finite additions (F<=4064/D<=4065), then requires fresh42, full101, the original8 prerequisite tests and all4 SDK/HTTP executions. No full101 or host/AZ15s claim is established.

Current process authorities after twice-reviewed Stage D publication: P has361 frozen inputs; I42 has256 and the fresh42 passed. F/D have4,062/4,063 rows. F SHA256 `256af6bb1f56c31b48977260731e607a2169206b0e1b4f01a1114d0ea1985526`; D `83a0244932605fecda00bbbf274190f4064e326b74dac92a07a35cf2c02e90f5`. Fresh full101 `node-guarded-parent-cancel-all101-b18e7c` failed21 as recorded above. Original8/all4 SDK execution remains gated on successful101 and both actual-result reviews. The six authority preimages and complete failed25 scope inventory (748 files/206 directories) are preserved under `/private/tmp/foks-celld-b4-p3-parent-cancel-preservation/`. Existing 4,095/4,096-row and 2-MiB bounds remain in force.

The retained-owner oracle correction and four process-event wait corrections passed both source reviews and the unchanged 42-unit suite. Latest run exercised their affected cases 12 and 23 successfully. Earlier failed runs remain failed: scheduler full101 failed23, retained-owner full101 failed12, ordinary-ready full101 failed21. Detailed source, execution and result reviews are recorded in `HOSTED_CELLD_TURN_SCHEDULER_PLAN.md` and private JSON receipts. Accepted native scheduler component passed tooling122, guarded123, scheduler7, transport6, lifecycle28, layout1, default/enabled builds and Clippy. Its acceptance does not grant process101 or host/AZ qualification.

Use direct pinned Python `-I -S -B` for candidate owners. The earlier ten-trial baseline stopped before any server because an unapproved runpy bootstrap added origins; do not resume that batch. Unit/process owners need process-inspection access (`/bin/ps` is blocked in the default sandbox). Root owns canonical edits, execution and commits; independent agents author privately or review. Review source/launch and actual results twice; do not weaken deadlines, memory limits, failure or cleanup oracles. Private owners and compiled evidence are host-specific: a Git clone alone does not reproduce qualification. AWS/AZ15s, zero-loss replication, destructive S3 reclamation and capacity claims remain unqualified.

---

# B4/P3 continuation handoff — 2026-10-17

This handoff supersedes the last progress paragraph of
`HOSTED_CELLD_NODE_PROCESS_PLAN.md`: a fresh cold smoke test **ran and failed**.
The native memory-budget correction is committed; the controller readiness
correction is planned but **not implemented or fully reviewed**. No full
101-case process run has passed. Stop implementation here for transfer.

## 1. Start here

Read this file, `HOSTED_CELLD_NODE_PROCESS_PLAN.md`, and these transferred files:

1. `/private/tmp/foks-celld-b4-p3-cold1c-readiness-plan.json`
2. `/private/tmp/foks-celld-b4-p3-cold1c-source-investigation.json`
3. `/private/tmp/foks-celld-b4-p3-cold1c-results-independent-review.json`
4. `/private/tmp/foks-celld-b4-p3-main-work-cold1c-controller-findings.json`

The immediate task is a small controller-only readiness fix, deterministic
regression coverage, refreshed input provenance, and a new owned cold smoke.
Only after that succeeds should the complete 101-case matrix run. Preserve
strict shutdown, ownership, deadline, and durability checks throughout.

**The current execution harness is not portable as-is.** It is pinned to
macOS/Apple Silicon, Homebrew Python, absolute paths, native binary hashes,
module origins, SDK/V8 assets, and macOS process/network controls. A Git clone
alone cannot resume it. Sections 8–10 explain transfer and requalification.

## 2. User requirements that remain in force

- Detailed plan, root self-review, and independent subagent review. Review
  implementation twice (root and independent), then test and independently
  review results before each separate implementation commit. Review material
  corrections again before running candidates.
- Keep **all Markdown unstaged**. Do not accidentally include plans or this
  handoff in implementation commits.
- Preserve FOKS wire encoding, cryptographic compatibility, and Go-client
  compatibility. The current task needs no FOKS protocol change.
- Local qualification first; there is no AWS environment. S3 is assumed
  durable/reliable with strong consistency. Local Moto and fault fixtures are
  test infrastructure, not evidence of actual AWS AZ timing.
- Target: recovery completes within 15 seconds; worst-case request latency
  target also 15 seconds. Normal data loss should be zero or near zero; after
  host/AZ loss, up to five minutes of lost writes is acceptable. **Five minutes
  is the allowed rewind, not the recovery time.**
- Hosted workload: personal servers (files allowed), organizations capped at
  64 members (credentials/passwords/small text notes). Initial metadata profile
  is explicitly restricted to fit celld, rather than silently promising the
  full native FOKS range.
- S3 object reclamation remains report-only until outstanding PUT termination
  is proved. This task does not establish zero-loss replication/fencing,
  production metadata authority, RSS/tenant density, or broad production readiness.
- Root executes candidates and Git; subagents author/review source or evidence.
  Do not substitute a fabricated test runner, relax an oracle, or rewrite old
  receipts to make a failure look successful.
- Repository `AGENTS.md` applies. Commit with a short subject and paragraphs
  using multiple `-m` flags. Cargo jobs=2. Before a full workspace test, build
  the standalone `foks-agent`; desktop-specific prerequisites are in AGENTS.md.
- A newly created worktree must use the opaque `QMUX_WORKTREE_ROOT` as parent.
  On the original host, reuse the existing validation checkout. On a new host,
  recreate a normal checkout/worktree; do not copy its old `.git` pointer.

## 3. Git and working files

Both main and validation HEAD are:

`885dbeb2c8ef2371a53d2988fbe5cee383c84fb9`

Commit: **Bound real celld reload work within guarded memory budget**.
Its predecessor is `2e226249358568bd136796775378dd7cd8100651`
(Add celld process qualification checkpoints).
P1 is `66090566c0ffcccc12e2d1c2bc9917f2f84019c4`.
B3 is `9151df82364fa60177cb62f6645746ce1aff9b48`.

Original paths (historical, not suggested destinations on a different host):

```text
MAIN = /Users/raymond/Code/foks-rs
VALIDATION = MAIN/.claude/worktrees/metadata-guarded-list-validation
T = VALIDATION/tools/foks-celld-guarded
EVIDENCE = T/runtime/evidence
```

Main had 43 unstaged/untracked Markdown changes before this handoff; this file
adds one more. No implementation edits are pending in main. Validation is
detached at the same commit, with exactly these three untracked P3 files:

| Alias | Path under T | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| G | `run-guarded-node.py` | 137542 | `af9bb037a1c1c2faaaf90c77d87806a747e3d2f271a9d6c66beb9429cebb20f1` |
| H | `test_run_guarded_node.py` | 42139 | `80f470d158a21c36ca2cf13c5859737a0f09fde3f29680d109495b7833883833` |
| P | `node-guarded.lock.json` | 239672 | `bdf3de4e0714944f5ca3d4cd14108415839cf18ecfd5cfd1d2c148e2e292696b` |

These are essential: **they are not in Git**. Preserve their exact preimages
before editing. `run-node.py`, `test_run_node.py`, and `node.lock.json` are the
older controller and must remain byte-identical for this readiness fix.

The committed native follow-up changed exactly seven files under
`tools/foks-celld-guarded`: `prepare-runtime.py`, `check_graph.py`, their two
test files, `runtime-node.lock.json`, `patches/node-series.json`, and appended
`patches/0007-node-main-work.patch`. The six preceding patches are unchanged.
Do not recommit/reimplement this completed work.

## 4. What the native follow-up proved

The real reload owner exceeded the previous individual 8192-byte work limit.
The fix raises that individual limit to 12288 while retaining the aggregate
131072-byte limit. Production and its regression use one shared
`reload_work_layout` helper with conservative two-future/two-string accounting.

Layout investigation and actual macOS observations:

- Main future: 4560 bytes in the accepted-P2 static binary investigation;
  this is supporting layout evidence, not a separately emitted cold1c value.
- Qualified fixture reload charge: **10394 bytes**, alignment 8.
- Base charge: 114128; resulting aggregate: **124522**, below 131072.

Root and independent source/results reviews completed before commit. Recorded
validation passed: 100 tooling tests; 123 native component tests; six transport
regressions; 28 lifecycle scenarios; one real binary layout regression;
configured Clippy and CLI checks; four fresh HTTP/SDK prerequisites; eight
diagnostic tests; and 42 controller tests. These are separate suites/layers,
not an assertion that the 101 real process scenarios passed.

Accepted component: `EVIDENCE/accepted-b4-main-work/receipt.json`, 1772 bytes,
SHA `c8508c7bf9696b64121c8c1d4d795e9459ff9c01c5825e795bbacd67cbd6be46`.
It contains preserved binaries, the 289-file prepared source and manifest,
graph, and aggregate native evidence. Enabled binary SHA:
`d567656e48840d2f1d95623c590e607fb698d3da7b112878e48126d672ca6163`.
Default binary SHA:
`9956c095b692857330dfc3dbfbed4b8a5d8d3e6c981f30c4bdb2cdaa69181117`.
Both preserved executables have mode 0555. Their evidence is host-specific.

Original accepted P2 evidence remains separately immutable under
`EVIDENCE/accepted-b4-p2/`; do not replace it with the follow-up binaries.

Important remaining timing failure: the fresh stalled SDK startup probe took
**30,031,201,458 ns**, so the 15-second goal is not established even if the
next ordinary cold case succeeds.

## 5. Latest actual failure: cold1c

Directory:

`EVIDENCE/node-guarded-cold1c-1099315871446083`

Independent results review is complete and accepts this as **failed evidence**.
Review SHA:
`d1e1c560a62ce9f0d0f0ef8b545a5dcccac7df8053bbaf26e1ffe4a6636b620a`.

- External owner actually exited 1. Controller case failed with
  `Refused: actual owned internal listener log`.
- Native output recorded ProbeEntered, ProbeSucceeded,
  **MainWorkCharged 10394/8**, a complete initial P1 report, and
  ReadyInitialSnapshotComplete generation 1.
- No functional protocol actions/operations/frames completed. Source ordering
  shows the listener-log check follows the HTTP-state and lease checks, but
  successful raw responses were not saved on this failure. Do not elevate
  that source inference into a separately captured Ready proof.
- Native output later recorded ShutdownStarted, but no sealed End. Native
  required SIGKILL (return -9); case protocol cleanup failed.
- The external owner separately proved OS process-group/known-child absence
  and physical stdout/stderr EOF. This physical cleanup does **not** turn
  failed native/protocol cleanup into a pass.

The diagnosis is specific. Accepted native `crates/celld/main.rs`, around
4776–4793, emits structured P1 and ReadyInitialSnapshotComplete in guarded
mode. The human `celld internal listening on ...` line is emitted **only in
ordinary mode**. G's `CaseRun.readiness`, around 1426–1454, unconditionally
requires that human line. Guarded mode therefore necessarily fails that check.

`atomic_snapshot:false` is expected in this report format and explicitly
accepted by Stream. It is **not** the cause. A previous progress comment
suggested otherwise and was corrected after source inspection.

Fourteen `Contended` progress errors and later S3Connect errors appeared during
failure cleanup. Inherited cleanup signals the node and Moto without waiting
for native handoff before stopping Moto, then kills survivors after its bound.
This explains a possible secondary storage-loss path, but the exact contended
accessor/lock holder is **not proven**. Do not blanket-retry consumed actions,
change native locking, or relax End requirements on that evidence alone.

Key raw hashes:

| File within latest directory | SHA-256 |
| --- | --- |
| `outer-result.json` | `be16e73ca486ef3a2ee64336d6541c926d19fa55f582a9dcfdc7aef64395f8f2` |
| `run/case-001/case-report.json` | `11494d3e4260374f0ab2f798780141fc6220f6b0d37566d0693005806c7d17ed` |
| `run/case-001/node-a.stdout` | `624f518859318bd838e470807364e430034e47b8db192bb4723f2e10b7cbfb44` |
| `run/case-001/node-a.stderr` | `9e94dfcb1eb05063e1b51fd06cd0d92e66e9452f963dba56c5b804f60c719e9e` |

Older failed cold1 and cold1b directories remain history. The first found a
module-origin collection mistake; the second found the now-fixed 8192-byte
limit. None may be relabeled as successful.

## 6. Immediate implementation plan and acceptance

The author plan exists with an author review. **Formal root and independent
review of the complete new plan are still required.** No new G/H edit has run.

1. Preserve the exact latest failed-run epoch before mutations: G/H/P,
   F/D, unit freeze, Q, launch, and referenced inputs with original-path,
   size, hash, and archived-path mapping. The transfer snapshot preserves
   copies, but the qualification's own historical mapping still needs to be
   added/reviewed. Keep raw receipts byte-identical.
2. Review the readiness plan. Make a fresh G/H author copy. Guard **only** the
   human listener-log read/check with `if stream is None`. Retain ordinary-mode
   log checking. Retain guarded complete-P1/Ready gating, live-process checks,
   actual internal HTTP 200/state census, fresh scoped node lease, exact node,
   advertise address, generation, key, expiry, evidence, and original deadlines.
3. Extend the existing `test_readiness_requires_internal_state_and_lease`
   test body. Keep the same 42-test inventory. Invoke actual `CaseRun.readiness`
   with deterministic external-effect fixtures, not a copied predicate.
   Cover guarded success without human log; ordinary missing/present log;
   missing P1/Ready; invalid state/lease identity/expiry; exited child; and
   unchanged deadline propagation. No sleep-based race assertions.
4. Root and independent source review, including all corrections, before
   candidate imports/tests. A native rebuild is not needed solely for this fix
   on the original host. A different platform is a separate qualification.
5. Refresh P and the unit input freeze honestly. Preserve old evidence with
   original-path/hash mappings. Review whether the existing diagnostic four
   cases actually depend on the changed readiness method; they use a separate
   diagnostic flow. A source-use review may preserve their historical credit.
   Do not rewrite Q/raw old receipts or blindly rerun unrelated native suites.
6. Run the existing exact 42-test owner, review actual results independently.
   Refresh/review full F/D, including fresh test evidence and unchanged native
   evidence. Review the new launch before executing it.
7. Run a fresh case-1 owned smoke (new directory, e.g. cold1d). Require actual
   native readiness, cold request and durability/lifetime observations, sealed
   End and case cleanup, plus independent external process-group disappearance.
   An exit code alone is never sufficient. A successful single-case run still
   yields incomplete full-suite status; expected owner exit is 2 only when the
   actual single case passes. The latest failed run exited 1.
8. If Contended recurs on a normal path, preserve failure, isolate a deterministic
   ownership/action-retention regression, plan/review the smallest native fix,
   then rebuild/requalify. Do not hide it with generic retries.
9. Only after successful smoke run all **101** frozen scenarios under the
   existing external owner. Review complete source-bound evidence, failures,
   deadlines, and physical cleanup before claiming P3 complete or committing
   its three implementation files separately. If a separate prerequisite fix
   is split into its own commit, explicitly scope its claims and review it.

## 7. Proof inputs and resource limits

The three important JSON objects have different purposes:

| Alias | Original path | Current state |
| --- | --- | --- |
| F | `/private/tmp/foks-celld-b4-p3-suite-inputs.json` | 2958 images; 803571 bytes; SHA `fabc17db55ed706d571180a3e8714d1ffb363f87ef49f51abef69a2f2119869d` |
| D | `/private/tmp/foks-celld-b4-p3-suite-owner-inputs.json` | 2988 images; 857779 bytes; SHA `97b682ee978a74bbbdc8e6adac7698cfd66c245bda164cca7b4f0cbbdaa6dbaa` |
| Q | `/private/tmp/foks-celld-b4-p3-main-work-prerequisite-qualification.json` | 116751 bytes; SHA `4ae7b941c236acd6f231512dfb769fc1be8e6bd3ead414425ea8b73e9594b42e` |

Q binds eight diagnostic tests and four actual HTTP/SDK cases; its `complete`
field means prerequisite completion only. It is not a full-process verdict.
P has 538 compact descriptors; full native/prerequisite raw rosters remain in
F/D. Do not duplicate all rosters into P: a prior attempt exceeded its bound.

Other critical source/data:

| Original `/private/tmp/` filename | SHA-256 |
| --- | --- |
| `foks-celld-b4-p3-suite-owner.py` | `cafd85b2e7080b7b040d0388b3c99758f0beb1607698b4818dca6c4af97428a8` |
| `foks-celld-b4-p3-unit-direct.py` | `e4d9214718507b78694e905e8e495d9fdcdd4535b86ee8bf05a8dde8c7e139bc` |
| `foks-celld-b4-p3-unit-inputs.json` | `d9604563fc6b4f0014d09f05e7aae1064db5090c4eb2e3ce4d06854883efe134` |
| `foks-celld-b4-p3-main-work-native-inputs.json` | `5697ea2a7e2629d94f41729942f421088441cd4e2da1c7f95c515878e51539e4` |

Final old-source 42-test result:
`EVIDENCE/b4-p3-unit-1099117115749541/receipt.json`, SHA
`bbda236f763ba6c0c504d9ab6e55f3dc67d8fb391ac5703be56d4694ef329754`.
Fresh actual four-case evidence:
`EVIDENCE/guarded-prereq-mainwork-1098416947377125`.
Fresh eight-test evidence:
`EVIDENCE/b4-p3-prereq-unit-1098430553244541`.

Limits must remain unchanged unless separately justified/reviewed:

- P <=262144 bytes; F/D each <=2097152 bytes and <=4096 images.
- Suite 14400 seconds from one original deadline; each case <=600 seconds;
  native scenario <=90 seconds; request acceptance 15 seconds.
- Five fixed loopback roles; latest used 48080–48084. Never blindly reuse an
  occupied port. At most two live nodes and 12 child handles per case.
- Owner cleanup is one 30-second budget; helpers <=3 seconds; <=4096 process
  inspections including 128 reserved for cleanup; bounded stdout/stderr.
- Evidence <=256 MiB/case and <=2 GiB/suite. Existing historical archives are
  separate; do not delete failed evidence to squeeze a run under its budget.
- Native build: jobs=2, initial free >=6 GiB, ongoing free >=2 GiB,
  isolated target <=4 GiB. Keep separate targets to avoid the earlier metadata
  rlib collision. Do not indiscriminately delete executable/evidence inputs.
- Canonical ordinary shutdown retains its 40-second cut. Do not conflate it
  with external-owner emergency cleanup or a successful 15-second recovery.

File-mode module-origin inventory is 74 before G, 85 after definitions, and
86 after helpers. Do not collect it with `python -c`: that changed startup
imports (including linecache) and already invalidated one run. No forged
`__file__`, module origins, hashes, or runtime aliases.

## 8. Transfer package and safe first commands

A companion local package is prepared under:

`/private/tmp/foks-celld-handoff-20261017/`

It contains a Git bundle, this handoff, a content-addressed file snapshot,
an inventory, and a stdlib-only verification/materialization utility. The
snapshot preserves absolute original paths as **data** and stores files by
SHA-256. It is not an installer and does not make host-specific binaries
portable. See its `TRANSFER.md` and `capture-report.json` for exact coverage,
counts, exclusions, and checksums. The archive is local; nothing was uploaded.

On the receiving server, extract the outer archive in a new directory, then:

```sh
cd foks-celld-handoff-20261017
python3 transfer.py verify
git clone repository.bundle ../foks-rs-resume
git -C ../foks-rs-resume bundle verify "$(pwd)/repository.bundle"
git -C ../foks-rs-resume checkout --detach 885dbeb2c8ef2371a53d2988fbe5cee383c84fb9
python3 transfer.py materialize ../foks-celld-history
```

The materializer writes only below the new staging directory, reconstructing
historical paths under `filesystem/`. It does not overwrite `/Users`, `/opt`,
`/private/tmp`, a new checkout, or system tools. It records symlinks separately
and does not activate them. Review the snapshot before copying selected
uncommitted files into the new checkout. Bring the 43 prior Markdown files
and this handoff as unstaged files; copy G/H/P from the historical validation
path into the intended validation checkout. Do not copy old `.git` files.

Only explicitly frozen host dependency files are included; this is not a full
Python/Homebrew installation or Cargo dependency cache. Dependency versions,
asset descriptors, and acquisition code are retained in profiles and tooling.
Derive and review a target-host installation recipe from those records; do not
assume copying the captured critical-file closure creates a working venv.

The bundle carries the committed repository without relying on a push. The
original remote is `git@github.com:aka-com/foks-rs.git`; whether 885 is there has not
been established. The bundle is the transfer source of truth.
Cloning it sets origin to the local bundle, not that GitHub URL.

## 9. Continuing on another host

### Matching Apple Silicon Mac

Even another Mac is a new execution environment. Verify actual tool/runtime
identities, executable aliases, SDK/V8/fixture hashes, paths, permission model,
closed environment, and native process inspection. Recreate the Python venv
from reviewed dependencies; a copied venv is not assumed relocatable.

If paths differ, author and review an explicit relocation of execution inputs
and owners. Preserve original historical manifests/receipts and map them to
their transferred byte-identical images. Do not recursively string-replace
historical evidence. Exact-path/no-symlink checks mean ad-hoc symlink shortcuts
will fail and must not be used to bypass validation.

If actual runtime/binary bytes differ, use a new source/binary qualification
epoch and rerun affected prerequisites. Historical Mac evidence remains
historical; rebuild provenance must identify the new outputs. Reuse immutable
source proofs only after reviewing the actual dependency relationship.

### Linux or a different architecture

First create a portability plan and review it before candidate execution.
Committed `prepare-runtime.py` has an explicit `aarch64-apple-darwin` target;
profiles pin Darwin toolchain and V8 archives. External owners hard-code
Homebrew Python paths and validate platform/process/module assumptions.
Changing one path or target string is insufficient.

The portability plan must cover:

1. Target-specific Rust 1.95.0 toolchain, V8 assets/bindings, linker/SDK/native
   dependencies and locked graph, without unrelated upgrades.
2. Reproducible Python/Moto dependencies and real interpreter/module-origin
   capture in normal file execution. Review both logical executable and actual
   runtime executable identities where applicable.
3. Equivalent network restriction to the five authorized local roles, actual
   process/group inspection and descendant cleanup on that OS, timeout/signal
   and pipe/EOF behavior, and filesystem/path checks.
4. Reconstructing patched celld source from the committed series and verifying
   its full manifest, then building actual guarded/default binaries. The
   measured native layouts may change with target; remeasure, do not assume
   the Mac charge or arbitrarily raise aggregate memory limits. Re-review
   standard-library Mutex/Condvar backend allocation and eager initialization;
   existing accounting includes Apple pthread/OnceBox behavior, not a proof
   for every operating system's synchronization implementation.
5. Fresh native/component/transport/lifecycle/layout/tooling validation and
   HTTP/SDK prerequisite evidence, followed by controller tests and cold/full
   process tests. Reissue P/Q/F/D for this host; keep old records immutable.

No old Mac pass qualifies a Linux binary. If portability is extensive, make it
a separate reviewed implementation phase/commit before continuing P3. Keep
unchanged FOKS wire/crypto and local-only scope. Do not bypass fail-closed pins
just to get the suite running.

## 10. Launch and review mechanics

Use `/private/tmp/foks-celld-b4-p3-main-work-cold1c-launch.json` as a historical
shape/reference, **not a command to replay**: its directory already exists,
its inputs are historical, and its recorded expected exit was not achieved.
Make/review a fresh launch after the correction and host qualification.

The original actual process owner ran through the venv Python with `-I -S -B`
in normal file mode. It received `--owner-input-sha256`, `--input-freeze`,
`--input-sha256`, a fresh `--directory`, `--suite case-1`, and exactly five port
arguments. Full suite selection must come from the actual owner/controller
parser and frozen profile; do not invent an invocation or alter case IDs.

Environment is closed: dummy local AWS credentials; metadata disabled;
AWS config and shared-credential files `/dev/null`; explicit loopback storage;
fresh tmp/data/empty-native-config directories; fixed locale; no developer
watch/dev-store/test-script substitution. No real AWS access is needed.

Independent reviewers should inspect raw outputs, exact source and graph,
native binary identity, real ownership inspections, immutable deadline
propagation, final input/fixture checks, evidence bounds, and cleanup. Unit
test mocks establish controller contracts only. Full-process proofs require
actual native observations and frames. A listener open or HTTP error alone is
not proof of durable guarded execution.

The previous native commit used hooks disabled because checks were explicit.
This is history, not a requirement to bypass hooks in future. `git diff --check`
flags two required blank context markers in the unified-diff patch artifact;
do not strip them or silently alter the patch hash. All other changed files
passed whitespace checks.

## 11. What remains beyond this immediate handoff

B4/P3 completion requires all enumerated process ownership/lifetime cases,
including restore, held sync/proof/TCP, abandonment/cancellation, panic and
shutdown paths. See the main process plan for the exact family-level oracles
and P for the finite 101 cases.

Even a passing local matrix does not establish the 15-second host/AZ target.
The actual 30-second startup-storage-failure case still needs a separately
reviewed deadline/remedy and rerun. Production metadata atomic ownership
fencing and durable acknowledgment, replication/failover choices, safe upload
receipt retirement, S3 PUT termination before reclamation, and density/capacity
qualification remain subject to their existing plans. Avoid expanding this
readiness fix into those redesigns.

Suggested first message to a new coding agent:

> Read HOSTED_CELLD_B4_HANDOFF.md and the transferred cold1c plan/reviews. Resume
> from commit 885dbeb2 with the three uncommitted P3 files. Keep all Markdown
> unstaged. First review portability for this host; preserve historical evidence
> and establish a new runtime epoch if necessary. Then complete the narrow
> guarded-readiness fix with root and independent plan/source/results reviews,
> exact controller tests, owned cold smoke, and only then the full 101 matrix.
> Do not weaken oracles or claim 15-second recovery from component tests.
