# Hosted capacity, memory and recovery-load plan

Implementation update (2026-10-15): `be58328d` implements response/working/input
allocation ownership through socket transmission and detached work, plus reader
release for external transfers and KV writer waits. The tested defaults include
512 MiB working, 128 MiB output and 1 MiB control allowances; small-request
headroom and bulk concurrency are separately constrained. These are per-server
admission ceilings, not idle allocations or a process RSS bound. Codec/container
and SQL-result allocations are charged conservatively; SQLite page caches,
allocator fragmentation, TLS/kernel buffers, runtimes and the experimental celld
JSON bridge remain outside this accounting. See HOSTED_EXECUTION_PLAN.md for tests.
Shared runtimes, idle reader eviction, process-wide/host-wide admission and
measured density limits remain open. The sections below preserve the original
plan and its baseline findings, not a claim that those defects remain unchanged.

Planning baseline: implementation commit `3c60f78e`. This document proposes
changes; it does not report them as implemented. The workload is personal servers
with files and organizations using credentials and notes, with a 64-member team
cap. The deployment target is thousands to tens of thousands of installations.
Successful recovery after the specified host/AZ failures should take at most
15 seconds and preserve client-visible authoritative state.

The existing local process-restart measurements and accounting-query VM tests
are useful baselines, but neither measures host/AZ recovery or fleet capacity.
Host/AZ qualification depends on an actual replication, ownership and deployment
implementation. This plan defines the resource and workload side of that gate;
replication/fencing must be qualified independently and together with this load.
No protocol, cryptographic format or Go client behavior change is proposed.

## Code baseline and immediate constraints

| Current behavior | Evidence | Consequence |
| --- | --- | --- |
| Incoming frames reserve twice their encoded length. | `src/net/session.rs::read_call_body_async` | A small download request can cause a large allocation that its input reservation does not cover. |
| `RequestOutcome` owns a raw response `Vec<u8>`; execution permits end when the blocking closure returns, before socket writing. | `src/net/session.rs::execute_bounded` and the response-writing loop | Slow receivers can retain large responses after execution capacity becomes available again. |
| Timeouts stop awaiting `spawn_blocking`; they do not stop the underlying work. | `src/net/session.rs::execute_bounded` | A disconnected client does not make request, object or response buffers disappear. |
| KV dispatch holds a `ReadLease` during object GET/PUT; only the download snapshot is dropped before GET. | `src/net/session/handlers/kv.rs::Operations::dispatch`, `src/services/kv.rs::dispatch` | Slow storage can exhaust the 32 default reader slots and obstruct unrelated credential reads. |
| Each native server creates four default I/O workers plus a runtime driver, writer, maintenance and management work; optional backups and blocking execution add work. | `src/config.rs::SessionLimits`, `src/net/listener.rs`, `src/writer.rs`, `src/maintenance.rs`, `src/operations/management.rs` | A separate live process per tenant needs measured thread, memory and scheduler capacity. |
| Writer open and initial pool validation scan retained tree/node data; pool growth reuses validated configuration but idle connections are retained. | `../foks-server-db/src/connection.rs`, `../foks-server-db/src/kv.rs::validate_kv_tree_capacity`, `src/read_pool.rs` | Cold activation and high-water reader memory remain history dependent. |

Default limits are 64 MiB of request-buffer reservations, 32 readers, 64 executing
requests and 256 active connections per installation. Those are current config
values, not safe density or latency recommendations. Full encrypted file chunks
are approximately 8 MiB, and object bytes, decoded ciphertext and encoded replies
can coexist. Measure the allocation path before choosing memory limits.

## Implementation sequence and review gates

1. Inventory allocations and add resource observability and fault hooks. Establish
   the single-tenant workload baseline before changing admission behavior.
2. Separate KV database preparation from external transfer and final commit.
   Add tests proving that neither reader leases nor writer jobs span object I/O.
3. Introduce byte-accounted ownership through decoding, execution, queued writes,
   response encoding and socket transmission. Start with chunk routes, then cover
   every route and special asynchronous response path before claiming a complete
   bound. Keep deployment file concurrency conservatively limited during rollout.
4. Add idle-reader eviction and configurable shared runtime/scheduling interfaces
   only after the baseline identifies their value. Preserve one active metadata
   writer per tenant and the current trust boundary during the refactor.
5. Qualify density and correlated recovery using the matrix below, first for the
   native baseline and then for the selected hosted architecture. Choose capacity
   and admission settings from measured results, including reserved recovery room.

For each code component: direct self-review, independent second review, then
focused correctness/conformance tests and a commit. A benchmark is not a substitute
for ownership or quota tests.

## Database phases and reader ownership

Replace the KV handler's unconditional long-lived checkout with preparation and
execution APIs. Proposed internal types and names are illustrative, not public
protocol additions:

```rust
enum PreparedKv {
    Ready(BudgetedResponse),
    Download(PreparedDownload),
    Upload(PreparedUpload),
    // Other operations can retain short synchronous handling initially.
}

fn prepare_kv(/* owned request, principal, short read access */) -> Result<PreparedKv>;
fn finish_download(/* owned PreparedDownload, store, budget */) -> Result<BudgetedResponse>;
fn finish_upload(/* owned PreparedUpload, store, writer, budget */) -> Result<BudgetedResponse>;
```

`PreparedDownload` owns the authorized immutable chunk key, offset/final flag,
external digest/lengths or inline lookup identity, a request deadline, and (once
object deletion is enabled) an owned read pin. It must
not contain `ReadLease`, `ReadSnapshot`, a statement, a borrowed connection, or a
closure capturing any of them. If authorization is needed to choose a size,
perform a metadata-only query first: do not fetch the inline BLOB merely to find
its length. A public internal lookup in server-db can return a discriminated
inline locator/length or external reference without materializing the payload.

For downloads:

1. While GC remains disabled, acquire a reader, authenticate/authorize and copy
   the metadata-only descriptor in one short snapshot, then drop snapshot/lease.
   Once GC is enabled, perform authorized descriptor lookup and object-incarnation
   pin acquisition together in one short writer/coordinator transaction, with no
   prior reader lease or snapshot retained while waiting for that transaction.
   Return the owned pin after the transaction completes. Use the pin protocol in
   the [storage reclamation plan](HOSTED_STORAGE_FOLLOWUP_PLAN.md); a plain
   post-snapshot pin has a lookup/deletion race. A future separate read coordinator
   may reduce writer traffic, but must establish collector exclusion before its
   reader lookup and transfer that exclusion into the owned pin without a gap.
2. Admit the transfer and its complete allocation envelope without holding any
   database resource. For inline data, reacquire a short reader and revalidate
   authorization plus the selected immutable row before materializing it. If it
   disappeared, return the existing retryable/not-found outcome; do not substitute
   another version. For external data, fetch and verify the immutable object.
3. Retain the pin while awaiting admission and object GET, until verified object
   bytes are fully captured. Release it on rejected admission or failed transfer,
   but a cancelled blocking transfer must retain its pin until that work actually
   finishes. Release the pin once captured bytes no longer depend on object-store
   presence; the response-memory reservation separately survives socket writing.
   Encode the existing response and return owned bytes with their reservation.
   No database connection is needed to send those bytes. GC must remain disabled
   until pin acquisition/release is integrated with this path.

The external read's authorization point remains its initial database snapshot,
matching current behavior. Revocation during an already authorized transfer can
therefore finish that bounded read; document this instead of implying that
revocation retracts bytes already handed to a socket. If the product needs a
stricter policy, add a second short authorization check before response encoding;
that still cannot retract bytes already transmitted. Recheck errors must never
be converted into an empty successful chunk.

For uploads, preparation owns the canonical encoded upload, ciphertext and
metadata and releases all reader resources. Run the existing writer preflight,
release that writer job, publish the object, then submit the existing committing
writer job. Both jobs retain authorization checks. The final job rechecks quota,
ordering and exact replay; preflight reserves no logical quota. A race can leave
an orphan, which belongs to the physical-accounting/GC design. No storage operation
or permit wait may occur inside a writer closure.

Other KV write paths currently use `reader` again to form conflict responses;
prepare those responses through a newly acquired short lease only when needed.
Do not move an existing snapshot across a writer wait as part of this refactor.
Keep team capability, SSO and credential checks in their existing authoritative
transaction boundaries. Tests must cover revocation, membership changes, expired
capabilities, completion races and maintenance deleting an upload between phases.

## Byte ownership through cancellation and transmission

### Budgets and interfaces

Introduce explicit internal allocation ownership, for example:

```rust
struct AdmissionBudget { /* tenant + process/node limits, queue classes */ }
struct Reservation { /* RAII charge; private, no uncharged duplication */ }
struct Budgeted<T> { value: T, reservation: Reservation }
struct BudgetedResponse { bytes: Vec<u8>, reservation: Reservation }

fn try_admit(/* tenant, class, checked byte estimate */) -> Result<Reservation>;
fn try_grow(/* existing charge, checked additional capacity */) -> Result<()>;
```

Define input, transient working-set, output and control-response classes. Derive
a checked upper bound for each route's concurrent buffers, including canonical
re-encoding, `Value`/protocol decode copies, ciphertext, framing and container
capacity rather than only logical length. For chunk routes use the validated
encoded/ciphertext lengths plus audited serialization overhead. Refactor encoders
to write into reserved output buffers or eliminate copies where practical; do not
encode a large response and then reserve its already allocated size.

The first implementation should favor an auditable conservative envelope over a
tight estimate. Cover both inline and external files, large directory/list or
identity responses, status replies, and the KEX/realtime/OIDC special asynchronous
branches. Realtime/KEX long polls must not retain a worst-case bulk response budget
for their entire wait: retain their bounded pending-state charge and acquire an
output envelope when data is ready, without holding database resources.

Per-tenant and process/node budgets must be enforced together. In a shared
process, a single admission controller can atomically debit both under a short
mutex and return an owned reservation. In a process-per-tenant baseline, the
supervisor must distribute aggregate allowances and enforce a real node memory
limit; independent per-process counters do not constitute a fleet-wide bound.
Account separately for SQLite caches, TLS/kernel buffers, runtime stacks and
allocator overhead. The byte ledger bounds covered application allocations, not
all RSS. Keep an explicit measured margin and an external process/node limit.

### Ownership and timeout rules

- Input reservations follow the decoded request into `spawn_blocking`, as the
  existing code already does. Transient and output reservations follow the bytes
  they cover, never only the awaiting async request task.
- `PreparedUpload`/shared payload handles must retain a reservation until the last
  writer-queue or transfer reference drops. A budgeted `Arc` shares the same charge
  for the same allocation; copying its bytes requires another charge first.
- On successful completion, `RequestOutcome` carries `BudgetedResponse` through
  `write_response`, including slow sockets, partial writes and injected faults.
  Release it only after the owned buffer is dropped. Transport buffering beyond
  that point belongs to the separately bounded TLS/kernel allowance.
- On timeout, disconnect or shutdown, a detached blocking operation retains its
  reservation. Do not decrement gauges or return permits simply because its
  `JoinHandle` is no longer awaited. A completed result abandoned by its receiver
  drops its own response and reservation together. Queued closures and background
  retries must obey the same rule even if their original caller disappears.
- Cancellation may prevent work not yet admitted, but cannot make a committed or
  queued mutation uncommitted. Retain ambiguous-outcome semantics and independent
  fencing. A storage/thread operation that cannot be interrupted continues to
  consume its charge until completion; do not start replacement work against
  fictitiously freed capacity. Bound concurrent hung jobs and use supervision
  for an unresponsive process instead of unsafe forced permit release.

### Deadlock and starvation prevention

Prefer nonblocking, all-or-nothing admission for secondary memory and transfer
capacity. A request already holding input memory must not wait indefinitely for
output memory while other requests wait for the input budget. Never wait for
permits while holding a read snapshot, reader lease, writer job or queue mutex.

Atomically reserve tenant and process dimensions, or use a fixed acquisition
order with immediate rollback on failure. Do not hold a tenant reservation while
awaiting a global one. Acquire a full per-operation working/output envelope before
starting a memory-producing stage; a failed growth request aborts that stage
before allocating more bytes. Requests larger than an allowed envelope fail
promptly with a bounded protocol-compatible error rather than waiting forever.
Configuration validation must ensure supported maximum-size operations can fit
at least one admissible envelope when that workload is enabled.

Reserve a small bounded control/error capacity tied to active-connection limits,
so saturation can return a retryable status without consuming the same bulk pool
it is reporting as exhausted. Do not create an unbounded emergency allocation
path. Separate bulk transfer capacity from credential/metadata work and bound
per-tenant wait queues; do not rely on one global FIFO to provide fairness. Input parsing precedes route
classification, so reserve bounded small-frame/control ingress separately from
large incoming frames and reclassify only after authentication and routing. An
untrusted method name must not grant unlimited priority. Test input floods as well
as output saturation; reserved output bytes alone cannot protect metadata traffic
if every connection or execution slot is already occupied by bulk work.
Admission failures are availability failures in the 15-second qualification,
not successful recovery just because an error returned quickly.

## Focused acceptance tests

1. Use deterministic barriers in a fake store and instrumented socket. While a
   GET or PUT is blocked, a second request must acquire the sole configured reader
   and complete an authenticated credential read; unrelated small writes must
   complete through the writer. Assert no snapshot pins a WAL across that wait.
   Once GC is enabled, pause a download after descriptor/pin acquisition but
   before admission and during GET; a collector must not delete that incarnation.
   Rejected admission releases the pin, while timed-out live blocking work retains
   it. Race deletion against descriptor selection to prove lookup/pin atomicity.
2. Revoke the uploader or change team authorization after preflight but before
   releasing PUT. Commit must reject, metadata/accounting must remain unchanged,
   and the possibly published orphan must never be treated as a committed file.
3. Hold a socket unreadable after a chunk response is built. Its response-byte
   reservation must stay occupied even after the execution permit is available.
   An additional bulk request should be rejected or bounded without exhausting
   reserved credential/control capacity. Release the socket and verify charges
   return exactly once.
4. Time out or disconnect during object GET, encoding, writer queueing and partial
   socket writes. Keep fake blocking work alive after timeout and inspect the byte
   ledger before and after its actual termination. Cover an abandoned result and
   a shared upload still retained by a queued writer closure; neither can free its
   charge early. Cover panic unwinding and stop signals without leaking charges.
5. Exhaust tenant/process budgets in opposite scheduling orders and force growth
   failures. Assert prompt, protocol-compatible outcomes, no lock/permit cycle,
   bounded queued requests, and progress for another tenant. Reject a request whose
   single envelope exceeds its configured limit without waiting.
6. Audit memory-producing routes with allocation instrumentation and peak live
   capacity measurements, including maximal list/tree responses and special poll
   branches. Assert all covered allocations are charged before allocation; compare
   measured RSS separately to account for allocator/caches/TLS overhead. Avoid a
   test that merely reads back the same estimated size formula.
7. Run Rust wire fixtures and the actual pinned Go client gate for both storage
   modes. Successful content, exact retries, conflict statuses and usage results
   must remain identical. Add slow transfer and connection-reset cases without
   introducing new wire fields or treating timeout as proof of cancellation.

## Density and simultaneous-recovery qualification

### Reproducible fixture and workload matrix

Record commit, release profile, allocator, host CPU/RAM, OS limits, filesystem,
SQLite settings, storage class, object endpoint/region, topology, replica policy,
network shaping and all admission settings. Publish commands, deterministic seeds,
raw controller observations and server/resource metrics. Never use real secrets.

Build synthetic tenant images at increasing history sizes, for example 1,000,
10,000 and 100,000 retained objects/versions, plus selected near-configured-limit
cases. These are experiment sizes, not supported-density claims. Populate real
protocol-valid data using actual clients; an SQL-only microbenchmark can supplement
but cannot substitute for the RPC workload. Include histories with similar current
live sets but different accumulated versions, and a personal-file mix with full
chunks. Organizations use up to 64 team members and no bulk-file workload.

Run small secret writes, note overwrites, directory mutation bursts, file upload,
exact retries, slow downloads and idle chat/poll traffic separately, then mix
them. Drive offered load independently of response completion to avoid hiding
queueing delay; separately cap the load generator so its own saturation is visible.
Report offered/accepted/completed/error rates and timeouts together with latency.

Scale the process/tenant population progressively to the target 10,000 and, if
required, higher counts only when the test environment can instantiate them.
Use several disclosed active fractions and a concentrated hot subset; do not
extrapolate measured performance at a small population into a claimed fleet result.
Measure idle, post-burst high-water and evicted/restarted states, both warm and
cold database/object caches. Explicitly distinguish OS page-cache cold starts
from tenant-process restarts on a warm host.

### Instrumentation and architectural decisions

Collect per-tenant and node aggregate RSS/PSS where available, heap reservations
and actual live allocations, thread count, FDs, SQLite connections/cache use,
writer queue wait/execution distributions, WAL growth/checkpoints, DB open time,
CPU, scheduler delay, TLS handshakes, object request latency/retries and physical
object growth. Record p50/p95/p99 and worst observed end-to-end RPC latency as well
as sample count, test duration and errors; a percentile is not a worst-case proof.

Compare these measured architectures rather than selecting by assumed density:

- Native process per tenant, with explicitly tuned runtime/read/transfer limits.
- Shared runtime/scheduler with isolated tenant contexts, writer queues, budgets,
  storage identity, keys and authorization; test cross-tenant fault isolation.
- Platform-managed activation/cells with the same protocol and durability
  requirements, including actual ingress/mTLS and object access overhead.

Idle reader eviction must preserve at least the required validated access path,
recheck file identity on new opens, and avoid an eviction/recreation storm. Shared
maintenance and backups should use jitter, per-node I/O budgets and admission;
work due after downtime must not all run before user requests. Keep heavyweight
migration/integrity work outside the steady-state activation promise or explicitly
include it in the supported recovery envelope. Never skip validation silently to
improve a cold-start result.

### Host/AZ experiments and the 15-second criterion

Use a controller and evidence store outside the failed host/AZ. Freeze the affected
active-tenant set before the fault and select unaffected tenants as controls. Seed
immutable acknowledged values and durable client trust anchors for each tenant;
also capture newer state exposed by reads. The tests must retain those anchors.

Faults include killing a host with its local volume unavailable; partitioning a
live old owner from quorum while clients can still reach it; pausing and resuming
an old owner after promotion; and isolating a whole AZ with realistic correlated
loss of compute/storage/network paths. Specify which quorum and object-store
resources survive. A local process kill with its original volume is a separate
baseline, never relabeled as host/AZ recovery. Do not simulate an entire AZ merely
by restarting all its processes on their existing disks.

Start a common controller clock immediately before the fault injection. For each
affected active tenant, stop its availability clock only after the planned bounded
set of fresh authenticated reads succeeds and a new durably acknowledged mutation
is read back through normal routing. Include failure detection, old-owner fencing,
replica recovery/promotion, scheduling, database/key activation, routing, TLS and
client retries. The largest per-tenant elapsed time for this finite, declared
verification set must be <=15 seconds for that trial; report all failures,
including tenants never admitted or never recovered.
Do not reset a tenant's stopwatch when a scheduler finally starts it. Probe-only
success, retryable errors or success from a surviving stale owner do not pass.

Use a finite, documented correctness fixture small enough for the required probe
set to complete within the deadline under healthy operation. Separately reconcile
**every** controller-observed acknowledgment/read obligation and file digest across
the larger history after the fault; require zero observed loss, forks or resurrected
revocations. Report the exhaustive sweep duration separately from availability.
A fast sample is not evidence that all historical data survived. The current
`RecoveryLedger::recovered` requires all registered values and a new write/read;
retain that strict behavior for its small fixtures. A scale controller should
explicitly distinguish availability-probe completion from the full-ledger verdict,
not silently weaken or relabel the existing check.

Repeat failures while backup hydration, object-store delays, quota contention,
slow clients and cold tenant activation compete for capacity. Preserve resource
headroom for the entire affected active set; admission fairness must protect
unaffected tenants too. Measure eager recovery of all affected active tenants
separately from activation of idle tenants on their first request after failure.
Any claimed idle-activation bound includes its scheduling and cold-open delay.

Run multiple disclosed trials across placements/load levels and publish worst
observed times and the failure matrix. Do not claim an unconditional guarantee
from a finite benchmark. If an allowed failure loses the required quorum or
object durability, report that the architecture cannot meet the target; a faster
error or a five-minute rewind is not successful, zero-loss recovery.

## Deliverables before changing the hosted capacity promise

- Reviewed byte-ownership and reader-phase implementations with passing focused
  tests and unchanged Rust/Go conformance gates.
- A reproducible load controller, tenant fixture generator, exact deployment/fault
  commands and external observation ledger for the selected platform.
- Measured steady-state and mass-recovery results for the supported history,
  active-tenant mix, storage conditions and surviving failure topology.
- Explicit admission/resource settings, minimum recovery headroom and excluded
  failure cases, derived from those measurements. No tenant-density or requests-
  per-second capacity promise is established by this plan.

## Cleanup progress under contention: additional qualification gap

The D3b validation parallel client-app library run had nine cleanup-progress failures; the unchanged production code passed all245 active tests serially (two ignored). Preserve `/private/tmp/foks-d3b-client-app*` evidence referenced in the deadline ledger. Attribution to DB-open/fsync cost was not instrumented, so this is not a proven cause. The cooperative50ms maintenance turn can expire before its first erasure; qualification must measure first-progress latency, debt drain throughput and fairness under simultaneous profile traffic. Serial correctness success does not establish those load guarantees.

## Pinned celld admission audit: physical memory still needs qualification

Source inspection of the existing local celld 0.6.2 checkout distinguishes three
controls. `runtime.rs::pool_limits` applies `CELLD_MAX_REQUESTS` to stateless
request affiliations. Cell fetches instead use `CellRequestAdmission`, with a
per-cell default of 64. `CELLD_MAX_RESIDENT_CELLS` bounds logical cell residency.
None alone is a process-wide reservation for FOKS Wasm linear memory.

The cell request permit moves into the spawned event driver, so dropping the
waiting fetch future does not immediately release that permit. This is useful
physical-work ownership, but it does not establish that a returned response has
finished draining or that V8 has reclaimed an abandoned Wasm instance. An isolate
affiliation can also outlive cell residency during retirement. Multiplying the
resident-cell cap by a per-cell request cap therefore requires a separate proof
covering old generations, retirement and pending output. A per-realm JavaScript
counter must not be described as a process-wide semaphore.

The native guarded List implementation should first use actual process-owned
request/execution reservations. Later celld experiments may use an explicitly
bounded load driver and report observed RSS, linear-memory highwater, pending
drivers, response drainage and reclamation after cancellation or replacement.
Such experiments remain useful before a hard process-wide cap is implemented;
they establish measurements for the tested load, not a production admission
guarantee. Repeated large/small requests and old/new owner overlap are required
cases. Do not infer physical reclamation from removing JavaScript references.

This source audit introduces no platform patch, configuration change or new
capacity result. The inspected source is the pinned checkout already used by the
local qualification harness; deployment qualification must repeat the check for
its actual binary and configuration. Six inspected source hashes are recorded in
`/private/tmp/foks-celld-physical-capacity-audit.json`; the extracted archive has
no Git metadata, and this source audit alone does not establish binary provenance.
Direct review and independent source review both passed. This Markdown remains
unstaged.
