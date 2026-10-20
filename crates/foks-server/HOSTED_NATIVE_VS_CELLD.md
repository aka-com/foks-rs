# Native hosted FOKS versus celld

Assessment: 2026-10-15. Native implementation includes `2bcf7ec0`; the celld
experiment used v0.6.2. This document changes no runtime behavior and stays
unstaged. See the [native recovery evidence](HOSTED_RECOVERY_VALIDATION.md) and
[celld experiment](HOSTED_CELLD_EXPERIMENT.md).

Update: items 1–5 are now implemented through `57790f38`. The newer
[real FOKS celld proof](HOSTED_CELLD_PROOF_RESULTS.md) exercises a bounded native
TCP/mTLS adapter against authoritative cell SQL, including process loss and Go
compatibility. It supersedes the earlier toy-worker-only evidence below, but is
not a production backend or a physical host/AZ qualification.

## Decision and scope

Native hosting has the lower application-migration cost. Celld has the stronger
existing platform implementation for ownership, replication and idle-cell
lifecycle. Neither currently constitutes a qualified hosted FOKS service.
The decisive question is how much native FOKS must change to use celld's
authoritative database and durability boundary while retaining Go compatibility.
A counter beside an ordinary FOKS SQLite file does not do that.

Compare equivalent completed designs, distinguishing their unfinished parts:

- **Native:** stable FOKS TCP/mTLS ingress, native server and SQLite, a qualified
  replication/storage layer, distributed ownership fencing, durable-output
  gating, and a host supervisor/router that activates tenants on demand.
- **Celld:** the same externally compatible FOKS ingress, an adapter executing
  authoritative transactions in cell storage, and celld's ownership, replication,
  routing and activation machinery. A supported native storage integration could
  be an alternative to a Worker port. The local proof now demonstrates that
  boundary for pre-provisioned personal small-file operations; production coverage
  and an efficient persistent adapter remain work.

The workload is many mostly idle personal installations, plus organizations
capped at 64 members using credentials and small notes. Personal installations
may store files. The target is successful recovery within 15 seconds. The user
allows up to five minutes of lost writes after host/AZ loss; the stronger
zero-loss gate remains our preferred design and existing test contract. A stale
restore is not automatically a compatible fallback: clients may reject rollback,
and lost revocations/key state need particular care. We have not demonstrated a
safe, transparent five-minute rewind with unchanged clients.

## One-to-one comparison

| Concern | Native hosted design | Celld-backed design | Assessment for FOKS |
| --- | --- | --- | --- |
| Protocol/authentication | Retains existing framed RPC, delegated TLS and client-certificate verification. Needs stable routing and reconnect handling. | The new native TCP/mTLS proof preserves those semantics for its bounded workload, including pinned Go clients. Production routing and broader mutation coverage remain work. | Both can retain compatibility; the celld production adapter is unfinished. |
| Database integration | Existing `rusqlite` transactions and writer closures stay usable if a physical replication/storage integration preserves their semantics. Backend selection remains open. | Cell SQL/transactions become authoritative. Native connections, thread closures and local paths do not become cell storage through configuration. | Celld integration is substantive, not a launcher change. |
| Ownership/durability | Must integrate stale-owner authoritative commit rejection, replication receipts, recovery sealing and output gating. The current process-local writer lock is insufficient. | Epoch ownership and durable output gating are supplied inside the platform boundary. FOKS adapters must not bypass it. | Celld removes substantial correctness work if genuinely integrated. |
| Activation/density | Current process-per-installation layout duplicates runtimes, worker threads, readers and caches. Demand activation, eviction, placement and shared resource budgets remain work. | Shared Worker/cell runtime supports activation and eviction. Native containers have their own resource cost. | Celld has an architectural density advantage for a true cell port; neither has a measured 10,000-tenant result. |
| Write throughput | One serialized writer per installation today, with read connections. Independent installations can use different cores/hosts. | One cell is also a serialization point; transaction/concurrency rules must preserve FOKS atomicity. Independent cells distribute work. | Neither makes one tenant's mutations arbitrarily parallel. Small organizational writes look plausible but require measurement. |
| Write latency | Today a local commit; zero-loss hosting adds remote durability and ownership checks before visible output. | Replication/gating already contribute to cell latency. Durability mode and storage/network RTT matter. | Comparing current local SQLite latency with durable cell latency would be unfair. |
| File payloads | Optional external encrypted chunks are implemented; metadata remains in SQLite. Inline installations remain supported. | External objects hold encrypted chunks; cell storage holds authoritative references. | Disaggregation helps both. FOKS publication, quota and GC rules remain necessary. |
| Keys/identity | Encrypted key directory is separate from SQLite and must survive coherently with metadata. | Needs an equivalent durable key provider and stable installation-to-cell mapping. | Required in both, including cold-start access to wrapping/root material. |
| Host/AZ recovery | Must fence/promote, activate and reroute. Current positive trials restore complete pre-positioned backups under controller direction. | Actual FOKS process-loss trials now exist, with both passes and simultaneous-recovery deadline failures. See the latest proof results; physical host/AZ behavior remains unqualified. | Neither has established the 15-second AZ target. |
| Operations | Own lifecycle, routing, storage integration, upgrades and observability; platform can remain narrower. | Operate celld nodes, backing storage, ingress, private networking, upgrades and capacity; platform handles cell lifecycle. | Both are self-hosted fleets. Celld shifts responsibilities, not all operations. |

Native anchors: [writer](src/writer.rs), [listeners/runtime](src/net/listener.rs),
[TLS/client authentication](src/pki/host_tls.rs), and
[key-provider startup/backup](src/standalone.rs).

Celld's [cell APIs](https://celld.dev/docs/services/durable-objects/) expose SQL,
transactions and durability synchronization; calls can interleave across awaits.
Name-derived IDs depend on script/class namespace, so deployment renames need
care to avoid selecting new empty storage. The documented
[ownership/durability mechanism](https://celld.dev/docs/guarantees/) uses epochs
and gates visible output. Fleet mode can use follower persistence; bucket mode
waits for object storage. These properties cover integrated cell operations,
not arbitrary neighboring processes.

Rust is supported through [WebAssembly/workers-rs](https://celld.dev/docs/wasm/).
That does not port native filesystem access, `rusqlite`, threads or listeners.
[Containers](https://celld.dev/docs/services/containers/) can run native programs
and expose ports to their controlling cell, but their disks are ephemeral and
the feature is experimental. Ordinary SQLite in a container does not acquire
cell replication. Port access also does not establish public FOKS TCP/mTLS ingress.

Celld [does not supply managed ingress or a multi-tenant scheduler](https://celld.dev/docs/limitations/).
Its [security model](https://celld.dev/docs/security/) trusts application code and
fleet nodes; v0.6.2 is beta and excludes mutually hostile tenant code. One
operator-owned FOKS application serving many customers is a different situation,
but hostile client inputs and resource isolation still need testing. Public TLS
and confidential internal networking remain operator responsibilities.

## What the timings compare

Native FOKS restored sixteen installations in **3.501 seconds aggregate** from
complete, quiescent backups copied before failure. The controller knew when the
fault occurred and directed restoration. That excludes continuous replication,
failure detection, distributed fencing and automatic promotion. It does exercise
real FOKS authentication and data operations.

Celld recovered eight **synthetic JavaScript cells** in **10.068 seconds** after
SIGKILL of their owner, retaining all 40 earlier acknowledgments and completing
eight new writes plus cross-node rereads. That includes lease waiting and
platform recovery, but excludes FOKS integration. All nodes shared one Mac;
the in-memory object emulator survived, and local files were not deleted or
verified inaccessible to survivors. Neither run exercises actual AZ loss.

Consequently, 3.501 versus 10.068 seconds is not a platform speed comparison.
Use the same real FOKS ledger, fault model, metadata history, tenant count and
remote storage before comparing performance or claiming either meets the target.

## Why the HTTP requests failed

These were owner-routing errors in the synthetic HTTP fixture, not FOKS RPC
errors, SQLite errors or evidence of lost writes. Source references pin celld
commit `90b43017241f81189453d326d05948f388b34652` (v0.6.2).

1. The owner was killed shortly after renewing its roughly ten-second lease.
   Ingress still resolved cells to it while that lease remained valid. Connection
   refusal alone cannot distinguish death from a partition where the owner
   continues serving other clients.
2. Celld classified the failed connection as `NeverConnected`, performed one
   immediate `Redispatch`, then stopped. This path does not wait out the lease.
   Its separate `WaitForOwner` policy concerns a reachable peer reporting
   `NotOwner`. The 15-second setting bounds applicable operation waits; it does
   not promise to keep failed requests pending that long or establish a universal
   end-to-end recovery bound.
   [Retry source](https://github.com/denoland/celld/blob/90b43017241f81189453d326d05948f388b34652/crates/celld/main.rs#L402-L495)
3. The bridge exposed `DurableObjectRoutingError`, code `owner_unreachable`,
   marked retryable. Our toy Worker did not catch it; the outer handler mapped
   the failed invocation to HTTP 500.
   [Exception construction](https://github.com/denoland/celld/blob/90b43017241f81189453d326d05948f388b34652/crates/celld/js/harness.js#L2027-L2050),
   [propagation](https://github.com/denoland/celld/blob/90b43017241f81189453d326d05948f388b34652/crates/celld/js/harness.js#L5176-L5183),
   [HTTP mapping](https://github.com/denoland/celld/blob/90b43017241f81189453d326d05948f388b34652/crates/celld/main.rs#L2373-L2399)
4. After expiry, celld recovered/sealed the old owner's log and restored the
   cells under new epochs. Subsequent controller attempts succeeded.
   [Lease/recovery interlock](https://github.com/denoland/celld/blob/90b43017241f81189453d326d05948f388b34652/crates/logic/lib.rs#L4420-L4506)

Corrected-run UTC timestamps:

| Event | Time / observation |
| --- | --- |
| Last owner renewal | 06:49:37.333052; 9,998 ms headroom |
| First / last refused peer connection | 06:49:37.528312 / 06:49:47.151637 |
| Approximate expiry inferred from renewal | 06:49:47.331 |
| Old log recovered and sealed | 06:49:47.443826; 17 entries; 47 ms reported recovery work |
| Eight replica restores logged | 06:49:47.488210–06:49:47.564052 |
| All content and new-write checks complete | 10.067868 seconds after fault |

The **296 HTTP errors were 37 failed attempts per cell**, across eight parallel
probe streams with 250 ms retry sleeps. They correspond to **592 refused peer
attempts**: one immediate redispatch per external failure. They are not 296
outages or lost writes. Controller records omit HTTP method/stage, but its
sequential GET → POST → cross-node GET flow and errors preceding restoration
identify initial GET failures by reconstruction. All acknowledged content
survived this trial.

Lease waiting dominated this tiny fixture. Ten seconds is not a fixed recovery
bound: renewal timing, log size, storage latency and simultaneous activations
change it. A real AZ network failure can blackhole connections and consume
timeouts that this immediate connection-refusal fixture avoided. The initial
controller stopped on its first 500 and measured no
recovery time. The corrected run retried under one common fault clock.

For HTTP, catch typed routing failures and retry reads/idempotent operations
within one end-to-end deadline, or return an intentional 503 with retry guidance.
Changing the status alone does not improve recovery. Native FOKS needs equivalent
internal handling with existing wire errors/reconnect semantics, rather than new
HTTP semantics. Neither a generic 500 nor the typed error's `retryable=true`
proves a mutation never executed: celld also wraps ambiguous peer-fetch errors
as `owner_unreachable`.
[Transport-error wrapping](https://github.com/denoland/celld/blob/90b43017241f81189453d326d05948f388b34652/crates/celld/main.rs#L1207-L1221)
Abandoned native blocking work can also complete. Use existing operation
reconciliation/exact-replay semantics for mutations. Celld distinguishes failure before
delivery from ambiguous execution; see its
[replay rationale](https://github.com/denoland/celld/blob/90b43017241f81189453d326d05948f388b34652/crates/logic/routing.rs#L3-L36).
This trial passed recovery-with-retries within fifteen seconds, not uninterrupted
first-attempt success.

## Recommended decision gate

Keep native FOKS as the compatibility reference. Before selecting a platform,
build a bounded celld integration proof: one real multi-statement FOKS mutation,
existing Go-client authentication/read/write, durable host keys, and the same
external recovery ledger. Include a paused former owner, active-write crash and
ambiguous-response test. Inspect the actual SQL/transaction adapter and output
boundary before broad porting. Do not begin by inventing a new consensus protocol
for the native option.

If a manageable adapter lets celld own authoritative transactions, its ownership
and lifecycle implementation is a substantial advantage for many idle servers.
If this requires a large database rewrite or only yields an ephemeral sidecar
around ordinary SQLite, prioritize native FOKS with a qualified replication/
storage substrate. Verify that substrate's fencing and durability contract;
asynchronous backup alone does not satisfy the zero-loss gate.

For either design, AZ qualification must specify surviving replicas, key access,
object-store availability and spare compute. Follower count alone does not prove
independent AZ placement. Evaluate bucket durability separately, measuring added
write latency and recovery behavior. Roughly five seconds of observed celld
margin does not establish large correlated recovery. Shorter leases are a tuning
experiment with failure-detection/storage-load tradeoffs, not the first fix.

Socket-held byte accounting, reader-lease release, namespace binding, orphan
accounting/reclamation and resumable uploads remain necessary in both designs.
Celld's log/epoch cleanup does not know FOKS file-version reachability. Density
qualification must measure idle and active tenants separately, then restore the
largest supported failed domain while surviving traffic continues. The member
cap and small values do not substitute for these measurements.
