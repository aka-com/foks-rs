# Local celld process-failure experiment

One small celld fixture recovered eight synthetic JavaScript ledger cells, all
40 acknowledged prior writes, and eight new writes with cross-node rereads in
**10.067868 seconds** from SIGKILL. There were **296 HTTP 500 responses** during
recovery, through 9.637785 seconds. The result measures successful recovery with
client retries, not uninterrupted successful requests.

See the [native-versus-celld assessment](HOSTED_NATIVE_VS_CELLD.md) for equivalent
target architectures and a pinned-source explanation of the HTTP errors,
including the lease timeline and immediate routing retry behavior.

This is **not FOKS, host/AZ recovery, or production storage qualification**. All
three nodes ran on one Mac; the controller and an in-memory Moto object store
remained alive. Local files were not removed, and their isolation from survivors
was not verified. No native FOKS TCP/mTLS-to-worker adapter was exercised. The
roughly five seconds left below the 15-second target is an observation for this
fixture, not a bound for a larger deployment.

## Environment and setup

Run performed 2026-10-15 UTC on Darwin arm64, macOS 26.5.1 (25F80):

| Component | Version/setup |
|---|---|
| celld | 0.6.2, official `celld-aarch64-apple-darwin.gz` [release](https://github.com/denoland/celld/releases/tag/v0.6.2) |
| S3 emulator | Moto 5.1.17, independent loopback server, in-memory |
| Python | 3.14.3, isolated virtual environment |
| boto3 / botocore | 1.43.114 |
| esbuild | 0.27.4 |
| Topology | Three celld child processes, distinct public/internal loopback ports and `CELLD_WATCH` directories; shared physical machine, working directory and temporary directory |
| Credentials | Explicit dummy AWS credentials; metadata disabled; AWS configuration and credential files `/dev/null`; sanitized child environment without HOME |

Docker CLI was present, but its configured daemon socket
`/Users/raymond/.orbstack/run/docker.sock` did not exist. Official native MinIO
latest and pinned `RELEASE.2025-04-22T22-12-26Z` downloads returned HTTP 410.
No large source build was attempted. celld's embedded `dev` store is documented
as unavailable to fleet nodes, so the experiment used a separate Moto server.
[Deployment documentation](https://celld.dev/docs/)
The celld binary emitted a macOS allocator background-thread warning and ran.
Telemetry was disabled using the [documented setting](https://celld.dev/docs/telemetry/).
Artifacts and dependencies occupy approximately 451 MiB
under `/private/tmp/foks-celld-lab`.

Before deploying, the controller required successful conditional create,
create-conflict HTTP 412, matching-ETag update, stale-ETag HTTP 412, two contenders
for the same ETag with exactly one success, and an immediate exact ranged read.
These passed. They are basic API sanity checks, not proof of emulator
linearizability, crash durability or qualification for celld. Moto is not a
persistent S3 service. No real cloud bucket or credentials were used.

## Configuration and method

The default 10,000 ms lease and 15,000 ms operation deadline were retained.
`CELLD_DURABILITY=fleet`; two Tokio threads per node; resident-cell cap 32;
512 MiB memory threshold; per-isolate heap 64 MiB; local cache 64 MiB.
`CELLD_READY_FLEET_GATE_MS=0` and `CELLD_REBALANCE_INTERVAL_MS=0` made initial
placement deterministic. Normal fleet readiness, new-node capacity readiness,
rebalancing and autoscaling are excluded from the measurement.

A synthetic Durable Object stores operation ID/value pairs. Its handler rejects
a reused ID with different content and accepts identical retries. Values are
deterministic SHA256 strings. Eight cells were activated on node 0, then two
additional nodes started. Each cell accumulated five acknowledged writes. The
external controller fsynced each acknowledgment into its JSONL ledger. Node 0's
`/state` showed the eight cells immediately before the fault; the controller
asserted the count, logged the IDs and PID, then sent that PID SIGKILL.

Eight concurrent probes compared every pre-fault row, submitted an idempotent new
write per cell, and compared the complete resulting rows through another live
node. The common monotonic timer began immediately before SIGKILL and included
all retries. Only transport failures/timeouts and HTTP 5xx were retried; HTTP 4xx
or mismatched content failed. The controller admitted retries within a 30-second
observation window and applied per-request I/O timeouts; this is not a process
watchdog against arbitrary hangs. Completing
outside 15 seconds would fail the overall result and process exit status even
if integrity eventually passed. The controller was self-reviewed, independently
reviewed, corrected and rereviewed before this run.

## Results and failures

The first run deployed successfully and acknowledged all 40 writes. Both
survivors ran, and follower persistence logs were present. Immediate post-kill
requests returned HTTP 500 because routing still referenced the unreachable
owner. The initial controller incorrectly stopped at that point instead of
observing recovery through lease expiry. Consequently that run established an
immediate request failure, but measured neither recovery time nor data loss.
Its ledger and logs remain preserved.

The corrected controller retained the first-run failure, added bounded retries
without resetting the timer, and performed one fresh run:

| Observation | Result |
|---|---:|
| Cells / pre-fault acknowledged operations | 8 / 40 |
| Missing or mismatched pre-fault operations | 0 |
| Post-fault new writes and cross-node content rereads | 8 / 8 |
| All-cell completion from SIGKILL | 10.067868 s |
| Individual cell completion range | 10.044753–10.067441 s |
| HTTP 500 retries | 296 |
| First / last HTTP 500 observation | 0.028608 / 9.637785 s |
| Integrity / 15-second budget / overall result | Passed / passed / passed |
| Controller exit status | 0 |

Both survivors logged follower append persistence with fsync. No artificial
bucket lag or replica loss was injected: those logs do **not** prove which
acknowledgment proof won for each write, or exercise all unsaved-tail recovery
paths. `fleet` mode may acknowledge follower persistence or bucket upload.

The controller's `finally` block stopped and waited for all its child processes;
both runs recorded `all_children_exited: true`. No celld or Moto service from the
experiment was intentionally left running. Temporary artifacts were retained.

## Evidence and limits

- Executable controller: `/private/tmp/foks-celld-lab/run_smoke.py`.
- Corrected run: `/private/tmp/foks-celld-lab/run-1792046974540835000/`.
  Immutable-at-run-time script snapshot: `controller-script.py`; acknowledgment,
  retry, verification and cleanup evidence: `controller.jsonl`; logs:
  `node-0.log`, `node-1.log`, `node-2.log`, `moto.log`, `deploy.log`; pre-fault
  node state: `state-before-{0,1,2}.json`.
- First failed controller run:
  `/private/tmp/foks-celld-lab/run-1792046861588363000/`.
- Corrected script SHA256:
  `f67b543540deaca3f8d4a6e3229e82e723705daab2a13213988b6ede8b832aaa`.
- celld binary SHA256:
  `92ededd2bcfebeb1e78cc2d97219f00214a79eb6606f0eef48208fca3f98c15a`.

For newer real FOKS integration evidence, see
[the celld proof results](HOSTED_CELLD_PROOF_RESULTS.md). The limitations below
describe this earlier toy-worker experiment, not the later integration trials.

This experiment did not test stale-owner fencing after pause/resume, network
partition, lost local volumes, backing-store failure, correlated AZ replica loss,
10,000-tenant density, simultaneous large restores or any FOKS protocol/storage
integration. The CAS sanity check is not a stale-owner fencing test. Further
qualification needs the actual FOKS platform adapter or native replication
layer, qualified persistent object storage, independent fault domains, ownership
fencing, and an external acknowledged-write ledger under host/AZ injection.
Use the platform's [testing documentation](https://celld.dev/docs/testing/) when
choosing provider and failure qualification beyond this smoke test.
