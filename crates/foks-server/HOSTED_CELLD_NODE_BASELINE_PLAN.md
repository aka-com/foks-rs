# Actual celld node baseline

Status: committed as `fee8db31ae8df590e32bc02f8c8de815d9f7ecea`, after direct
and independent plan, source, and final evidence reviews. All 139 harness tests,
the actual build/Clippy checks, and the 25-step canonical node scenario passed.
Only the seven implementation files were committed. This Markdown remains
unstaged. The baseline qualifies the ordinary celld executable; integrating
guarded FOKS requests is the next phase.

## Purpose and scope

Build and run the actual pinned celld 0.6.2 `main.rs`, using the existing outer
locked workspace, existing prepared sources, and existing local Moto S3 fixture.
Exercise canonical deployment, acknowledged SQLite writes, handoff to a live
successor, restoration from a fresh local directory, Preserve restart, and
signal-triggered handoff. No celld source, engine, Cargo manifest, Cargo lock,
FOKS network protocol, or cryptography changes belong to this phase.

The result establishes observed core handoff, restore, and Preserve
preparation. It does not establish the 15-second host/AZ recovery target,
production S3 timings, guarded request deadlines, tenant density, or complete
internal task/destructor cleanup. Those remain later qualification work. The
production metadata backend choice remains deferred.

## Implementation group

Only these seven files under `tools/foks-celld-guarded/` change:

| File | Responsibility |
| --- | --- |
| `node.lock.json` | Closed input profile for the accepted baseline, fixed fixture, runtime policy, and finite limits. |
| `prepare-runtime.py` | Select, compile, lint, and hash the actual celld executable; run the controller under the existing owned process slot. |
| `test_prepare_runtime.py` | Command/artifact selection, unchanged earlier modes, and failure/cleanup checks. |
| `check_graph.py` | Validate the exact node mode, celld target/subclosure, and additional inputs. |
| `test_check_graph.py` | Reject changed or incomplete graph, target, feature, source, and receipt inputs. |
| `run-node.py` | Bounded local controller, fixed Worker/configuration, and actual node sequence. |
| `test_run_node.py` | Controller oracles and real small-child/network containment checks. |

Historical A1/A2 evidence keeps its original meaning. The node profile adds new
receipts instead of reinterpreting those results. Any necessary change outside
this roster gets a narrow plan review before implementation.

## Build and fixture gates

Select both `foks-celld-guarded` and dependency package `celld@0.6.2`, with
`--bin celld`, no default features, the existing outer manifest/lock, offline
resolution, the explicit `aarch64-apple-darwin` target, and two build jobs.
Metadata and tree checks precede compilation. Actual Cargo artifact output must
identify exactly the expected celld executable, package, `main.rs`, target, and
empty celld feature set. An unsupported package/target selection fails the gate;
there is no upstream-workspace fallback. Focused Clippy uses that same target.

Preserve existing compiler, SDK, V8, SQLite, source, registry, and dependency-edge
checks. The outer workspace includes FOKS transport/core dependencies; ordinary
celld's own dependency closure must contain no FOKS dependency. Record the
actual executable hash. Following the reviewed source-h amendment, SQLite
identity is source/build-graph-derived; the tenant SQL authorizer prohibits
the Worker version query. A2's separate binary identity is not substituted
for an ordinary-node runtime observation.

Bind the existing fixture interpreter, environment, installed sources, usable
bytecode, native libraries, startup hooks, and relevant path absences before
execution. Do not install another environment. Use isolated Python invocation,
disable bytecode writes, and explicitly constrain OpenSSL configuration and
provider paths for this HTTP-only fixture. Compact tree digests/counts belong
in the input profile; retain the complete inventory in evidence.

Cargo retains its deny-all-network sandbox. The runtime has a separate tested
policy permitting traffic only on five selected loopback ports: S3,
public/internal A-or-C, and public/internal B. Actual listeners and connections
use numeric IPv4 loopback. Seatbelt requires its special `localhost` selector
in policy filters; this is a platform loopback selector, not a DNS lookup or
an IPv4-only filter. No IPv6 runtime qualification is claimed. Loopback bind remains allowed;
the negative policy test concerns traffic, not an unlisted bind. Use controlled
local helpers for positive and negative tests. Never fall back to unrestricted
networking. Environment variables contain fixed dummy AWS credentials, isolated
configuration/data/temp paths, numeric local endpoints, and no inherited
credentials, proxies, or cloud configuration.

## Real node sequence

Use one Moto process, at most two live celld nodes, and exactly four node
incarnations: A, B, restarted B, and C. A and C have different identities and
fresh directories; only B intentionally reuses its own directory. Every child
remains in the controller's owned process group. Reuse a node's ports only after
its process and attributed descendants are confirmed retired.

1. Check bounded CLI version/help/invalid-flag behavior. Start Moto and prove
   create-if-absent, stale conditional rejection, a concurrent same-ETag race
   with one winner, and byte-range reads against that exact endpoint.
2. Deploy a fixed vanilla-JavaScript Worker through canonical `celld deploy`,
   using `no_bundle: true` in a fresh project. It exposes one Durable Object,
   one SQL row, bounded fixed operations, and `storage.sync()` before successful
   write responses. It does not accept arbitrary SQL or executable input.
3. Start A alone. Write/sync/read the first marker. Record its exact cell,
   resident state, cell-owner epoch, node lease, and process generation.
4. Start fresh B without sending it Worker requests. Confirm healthy,
   zero-owned/zero-resident state and a live eligible canonical node lease.
   Trigger A's internal Handoff shutdown. Require positive acceptance by B at
   a newer epoch, processed handoff, A's retirement, and B's dormant ownership
   before its first Worker read. Read the acknowledged marker from B only after
   A is gone, proving actual restoration into the fresh directory.
5. Write/sync a second marker on B. Trigger Preserve shutdown and require both
   `clean_reload_prepared` and the real local certificate bound to B's exact
   process generation. After retirement, restart B in the same directory and
   read the marker. Report certificate consumption separately from any evidence
   that the exact-generation resume optimization was used.
6. Confirm A remains retired and its old lease is ineligible. Accept a retained
   same-process expired record, including a sealed folded-log tombstone. A
   missing no-log record needs retained expired identity, a genuine exact-key
   404, and the live fixture owner B at the accepted newer epoch. Never treat
   permission, parsing, or transport errors as absence. Never alter the lease.
   Start fresh C on A's retired ports and verify eligibility without activating
   its Worker.
7. SIGTERM restarted B through its actual process handle. Require the same
   positive handoff evidence naming C, a newer epoch, B's retirement, and C's
   dormant ownership. Read the second marker from fresh C after B is gone.
8. Preserve C with the same event/certificate witnesses. Retire C, Moto, and
   controller work; verify final process-group disappearance and all input and
   executable hashes. Retain complete bounded evidence, including failures.

The lease observation in step 6 neither grants ownership nor proves that GC ran
or outstanding PUT attempts terminated. Remote reclamation remains report-only.

## Acceptance and observation limits

Celld may exit zero after an incomplete shutdown. Require positive
`drain_token_acquired`/`drain_token_released`, exact
`cell_handoff_accepted`, and subsequent same-cell `cell_handoff_timing` witnesses
for each handoff. Receiver state must show the one owned cell dormant and no
resident/activating/restoring/releasing work before its first Worker request.
Bind the state to the live receiver and its actual node-lease generation;
deployment generation is a different identity.

Reject observed forced drains, shutdown cuts/stalls, abandoned Preserve,
ambiguous/lost drain authority, fence/actor failures, wrong epochs/successors,
missing required evidence, truncated logs, and cleanup uncertainty. An expired
sealed lease must not be confused with an open folded log. Missing folded-log
records or changed process identity are incomplete evidence.

Canonical logs are lossy, and the final transport-grace timeout is silently
discarded. Therefore a passing report must still state
`final_transport_grace_observed=false` and
`internal_task_drain_observed=false`. Absence of a negative log alone never
proves absence of an internal cut. Process retirement is measured separately
from complete in-process destruction.

Keep canonical shutdown at 40 seconds. Successful observed shutdown must take
strictly less than 35 seconds; the 50-second bound is containment only. Use
120-second startup, 15-second client operation, and 600-second original scenario
deadlines. Retries keep the earlier phase/scenario deadline. Reject backward or
non-finite clock observations and wall/monotonic drift greater than one second.
Keep 32-MiB per-command log and 256-MiB fixture/evidence limits, two Tokio workers
per node, four residents per node, and a 10-second lease TTL. Local-ready and
rebalance delays are zero as explicit fixture tuning.

These shutdown bounds are diagnostic baseline criteria. They do not weaken or
satisfy the separate production recovery target of 15 seconds.

## Reviews, tests, and separate commit

After fixture closure, implement the seven files. Review the complete source
and tests directly, then have an independent subagent review them before
execution. Fix findings and repeat narrow reviews before rerunning affected
checks. Test changed controller behavior with deterministic cases, including
each positive-witness failure, expired/tombstoned/absent leases, false 404,
generation mismatch, exact deadline boundaries, clock anomalies, and port reuse.
Use actual small children for timeout/signal/surviving-child cleanup, evidence
write failure, log/disk refusal, and unknown cleanup; mocks are insufficient for
physical process containment. Run the actual runtime network-policy test.

Then collect/review the locked graph, compile and lint the selected executable,
and run the canonical sequence. Review its evidence and limitations before
committing implementation files separately. Keep Markdown unstaged. Guarded
request/output integration into celld's serving and drain loops requires its
own reviewed plan after this baseline.

### Initial regression findings

Source-a passed direct and independent source reviews before execution. The
first 116-test run passed 113 tests, including all 29 controller tests. A
fixture expected 20 bytes but contained 21; two runtime network-policy tests
lost useful child diagnostics when their temporary directories were removed.
Source-b corrected the count and retained bounded diagnostics, after another
direct and independent review.

The second run exposed the precise policy syntax rejection: Seatbelt accepts
`localhost` or `*` as the host selector, and rejects a literal `127.0.0.1`.
Use `localhost` while retaining the five-port restriction and numeric IPv4
service addresses. Never substitute the unrestricted `*` host. This changes
the profile bytes and their pins, not the fixture/program inventories.

It also exposed an intermediate cleanup race: a process inspection begun at
the end of the TERM grace period can expire just before escalation. Treat
that intermediate deadline distinctly from inspection failure; a later fresh,
successful final observation must still establish actual process retirement.
Unknown, escaped, or unparseable work remains a failure. Source-c passed direct
and independent review; its 124-test run passed all 36 controller tests,
including the real surviving-child case and seven new boundary tests.

The two remaining policy tests reached another incorrect fixture assumption:
Seatbelt rejects an unlisted port at `listen()` with `EPERM`, before the tests'
later connect/accept checks. The outbound probe must use a live parent-owned
listener outside the child sandbox, with a successful local control exchange,
and require a specific permission error from the sandboxed connect. The inbound
probe must recognize the observed listen denial while still completing a real
parent-to-child allowed-port exchange. No network policy widening is needed.
All three failed runs are preserved. Source-d made only that test correction
and passed direct and independent review. Its rerun passed all 124 tests with
no skips, unchanged fixture/source inputs, and no remaining observed descendants.

The actual locked/offline graph collection also passed: 370 selected packages,
a 356-package ordinary celld subclosure with no FOKS dependency, and the existing
58-package FOKS transport/core subclosure. The selected target is the prepared
celld 0.6.2 `main.rs` binary with no qualification features. This proves Cargo
accepts the proposed selection at graph collection. The first actual ordinary
celld build and strict primary-bin Clippy check passed. The compiler-selected
executable is 153,951,264 bytes with SHA256
`9eb2aca63a9430e4811d1a4cc19bd165aec90592bb75b2d0a6c32dba86e32182`.

The controller then refused before starting any nodes: Python/macOS added
`LC_CTYPE` and `__CF_USER_TEXT_ENCODING`, violating exact environment equality.
Reviewed startup probes established that explicit `LC_ALL=C` and this pinned
local platform's `__CF_USER_TEXT_ENCODING=0x1F5:0x0:0x0` preserve the exact key
set and isolation flags. Source-e keeps strict equality and adds a real
interpreter-startup regression, including 39 missing/changed/extra-key rejection
checks. After both source reviews, all 125 tests passed with unchanged inputs
and no remaining observed descendants. The repeated graph is byte-identical
to the first graph and binds the corrected launch profile.
The failed run and executable are retained under
`runtime/evidence/node-gk5fwkh7` in the validation worktree. Actual canonical
node qualification was still pending at that stage.

The source-e retry (`runtime/evidence/node-3omx1_w2`) passes environment startup
but cannot spawn `/bin/ps` from inside the runtime sandbox. The platform helper
is root-owned mode `04755`; execution returns `EPERM`. The controller reports
its inspections and cleanup as incomplete, while the outside owned-process
supervisor establishes whole-group disappearance. No canonical sequence step
or node incarnation completed. The binary is unchanged. A separate reviewed
probe of an exact-byte, non-setuid helper copy under the same network policy
also failed: the copied helper exited on signal 9 with empty output. Its outer
supervisor confirmed cleanup. This does not establish the unique cause of the
platform rejection, and no successful process-table observation is claimed.

The replacement diagnostic uses the already pinned Python `ctypes` module and
the system `libproc` API in a separate bounded helper process. After direct and
independent source review, it passed inside the unchanged sandbox: seven
inspections observed the real child alive, exited but unreaped (zombie), and
absent after reaping. Actual `Children` timeout, bounded output, kill/reap, and
cleanup code remained in use. The controller and outer supervisor both proved
cleanup; fixture, source, and SDK header inputs were unchanged. Runtime was
0.270 seconds. Receipt: `/private/tmp/foks-celld-a3a-libproc-probe/receipt.json`,
SHA256 `e72b362cea42ce8208f44369f907cd10d3a7c95cfd66bbdbcf5099dc795d4a23`.

The helper integration and expired-record correction passed both source
reviews. All 136 combined harness tests then passed, including the real
sandboxed helper lifetime regression, with unchanged inputs and no remaining
observed descendants. Receipt: `/private/tmp/foks-celld-a3a-tests-g/receipt.json`,
SHA256 `af38b10571b83f2f16a8973ee228fd7c028166f2b6640db66590d6bde3be54c1`.
The process inspection does not claim an atomic global snapshot or exclusion
of future forks. Actual canonical celld node qualification was still pending
at that stage.

The source-g actual run (`runtime/evidence/node-sbhi2wq7`) passed build, strict
Clippy, CLI checks, storage CAS/range preflight, deployment, and node A's live
health/lease checks. Its first Worker response failed because canonical celld
deliberately prohibits `sqlite_version()` in tenant SQL. The query ran during
response construction, after the fixture's write and awaited `storage.sync()`;
this failure does not establish rollback or absence of persisted writes. No
successful marker response, readback, or handoff was observed. The controller
and outer supervisor established cleanup, including forced termination of A.
This supplies no graceful-shutdown qualification. The fixture correction
removed the unsupported diagnostic and retained exact content/count assertions;
SQLite identity remains source/build-derived rather than a live Worker claim.

A separate source preflight found a canonical observation mismatch: the
no-log retirement path also CAS-writes an expiry-zero node record before
deleting its key. The harness must recognize this state separately from a
sealed folded-log tombstone. The reviewed correction retains exact identity,
prior expiry, old-process retirement and live-successor ownership checks, and
requires both the captured and current records to have no log. This observes
ineligibility only; it supplies no garbage-collection or PUT-termination proof.

## Successful local baseline

The source-h run `runtime/evidence/node-84hz_rop` completed all 25 steps and
four node incarnations. Its final receipt is SHA256
`a6ff49862c3eb740f9c46073a2b7756343303c27b3278311dce60cc1ba79c795`;
the controller report is SHA256
`63668c87213d3595c1d4849f7f667f45ac058e7b2afe86de23e70ee6835f4dbb`.
The final 139-test receipt is `/private/tmp/foks-celld-a3a-tests-h/receipt.json`,
SHA256 `64e7b0d542aae3b27c373ff2cc3058a03318700e0b8aedb40a3dc16c4f5eea1c`.

| Observation | Local measurement |
| --- | --- |
| Four node startups to healthy sampled lease | 106.9–117.5 ms |
| Handoff trigger to donor process exit, A and restarted B | 70.2 / 69.4 ms |
| Handoff trigger to successful restored-value response | 185.7 / 185.9 ms |
| Preserve trigger to process exit, B and C | 22.2 / 22.1 ms |
| Six successful Worker responses, including writes and restores | 1.6–47.9 ms |
| Whole controller scenario, including waiting for old A's lease expiry | 13.63 s |

The two synced writes were recovered with their exact marker and one-row count.
B and C initially held a dormant cell after handoff, then restored on demand
from fresh local directories. Handoff ownership epochs were 2 and 4;
subsequent activation published epochs 3 and 5. B restarted at epoch 3 and
consumed its Preserve certificate. Certificate consumption alone is not proof
of the exact-generation optimization; the report leaves that claim false.

All nine child processes retired with their expected exit status. The controller
joined its storage tasks and had no active responses; the outer supervisor
separately established process-group cleanup. Inputs were unchanged. Old A's
record was observed expired; no garbage-collection or outstanding-PUT completion
claim follows.

These are observations from one low-load local run using Moto. They do not
establish an abrupt host/AZ-loss recovery bound, a stable-front-door 15-second
RTO, production S3 timing, tenant density, guarded FOKS execution, or complete
internal node task/destructor cleanup. SQLite identity is source/build-derived.
Those qualification steps remain open.

## Review evidence

The detailed frozen artifacts live in `/private/tmp` for this local run:

- Original plan: `foks-celld-a3a-plan.md`, SHA256
  `d50d87adfc8852fc25d36ceff192faf412f42a30809d970c1986e944d94873e6`.
- Binding two-node amendment: `foks-celld-a3a-two-node-addendum.md`, SHA256
  `12c46c8d33ce5bc098bef1e6c06493005cc983eb9099fcbc7e09a268f3042300`.
- Root amendment review: `foks-celld-a3a-two-node-root-review.json`, SHA256
  `2df1ef636f529dc9f3c3eebe837190da7735499414f0e174ecd0f449b4093f8b`.
- Independent full plan review: `foks-celld-a3a-plan-integration-review.json`,
  SHA256 `2d907386765605c8697c3b068856cb6501c9c110c254b4393324b62b1b697f3f`.
- Independent lease-GC correction review:
  `foks-celld-a3a-gc-plan-integration-review.json`, SHA256
  `78999285bbe36e1685f109d688e1ccb91db1e5d02d89c5bdd1dc5fe94704b6ca`.
- Fixture launch contract: `foks-celld-a3a-fixture-launch-addendum.md`, SHA256
  `e3cd50729bc4ede18e7dc2b4eb0999b8fe453529ee674ca9e3b538bdd44f32c3`.
- Root fixture review: `foks-celld-a3a-fixture-launch-root-review.json`, SHA256
  `83da0e6750504d7425830225a676878e09e4d56e78bb76c4c8cbddd30c9b3b06`.
- Independent fixture review: `foks-celld-a3a-fixture-launch-import-review.json`,
  SHA256 `50bb16ac88bbcfd2dc52609a62f81ef670847f030a5b7d6609e7420f49064e55`.

The amendment supersedes the original one-live-node sequence and three-port
runtime policy. It is necessary because canonical handoff waits for a live
successor; a later successful restart would not prove successful handoff.
