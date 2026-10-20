# Rivet alternative assessment

Reviewed 2026-10-14 against current first-party documentation and GitHub; FOKS
baseline `3c60f78e`. Research only: no Rivet deployment, benchmark or failure test
was performed. Keep this document untracked and unstaged.

**Recommendation:** retain Rivet as a lower-priority prototype candidate. It is
more relevant than a generic VPC orchestrator, but is not a demonstrated drop-in
host for the existing FOKS binary. Resolve the released-version, native ingress
and SQLite integration questions before funding a cross-AZ deployment experiment.

## Current architecture and operational footprint

**Documented facts:** Rivet separates an actor control plane from application
workers. The control plane is one Rust binary; workers are application processes
using its SDK. Long-running workers can run on VMs, Docker or Kubernetes, while
serverless workers use an activation request. Actors sleep and wake independently.
A VPC or Kubernetes operator is not inherently required. This is a shared actor
service, not one control-plane instance per FOKS tenant.
[Architecture](https://rivet.dev/docs/architecture/)

A single-node deployment can use local RocksDB and one engine container. That
mode has no node fault tolerance. Current multi-node documentation specifies
PostgreSQL or FoundationDB plus NATS; PostgreSQL is described as suitable for
roughly 1,000 concurrent actors, with Rivet's FoundationDB integration requiring
an enterprise license for larger deployments. Concurrent actors are not the same
as total registered/idle tenants. The documented PostgreSQL path requires direct
connections rather than a connection pooler.
[Storage](https://rivet.dev/docs/deploy/self-host/control-plane/storage/)

For the documented HA deployment, budget multiple engine instances, HA database,
NATS, worker capacity and ingress. Shipped Kubernetes resource requests are
2 CPU/2 GB per engine instance, with 3 CPU/4 GB limits; these are deployment
settings, not measured minimums or FOKS capacity. Operators must configure database
failover and service replicas. This is considerably more machinery than the local
single-binary demo.
[Production checklist](https://rivet.dev/docs/deploy/self-host/control-plane/production-checklist/)

**Important version split:** the October 5 announcement says Rivet 3.0 is being
finished, with S3 as its only dependency and easier embedding among its goals.
It describes direct node-to-node request/reply and S3-backed broadcast logs,
replacing NATS and the previous database dependencies. Do not conflate that
announcement with the currently documented PostgreSQL/NATS deployment or assume
3.0 has passed production qualification. Pin a release and inspect its actual
configuration, storage implementation and upgrade path before selecting either.
[Rivet 3.0 direction](https://rivet.dev/blog/2026-10-04-replacing-our-message-broker-with-s3/)

## Maintenance evidence

The latest stable release observed on the release list was 2.3.21, published
September 24, 2026 at commit `64416b2`; its page showed 61 subsequent main-branch
commits when reviewed. The release includes namespace authentication and runtime
fixes. Recent releases also contain SQLite/runtime and engine work. This supports
an assessment of continuing maintenance, rather than inactivity. It does not
establish a support SLA or predict future priorities.
[Release](https://github.com/rivet-dev/rivet/releases/tag/v2.3.21),
[release history](https://github.com/rivet-dev/rivet/releases)

Current product documentation emphasizes agents and workflows but still exposes
actors, per-actor SQLite and Rust support. A concrete open issue, #5554, reports a
transient closed SQLite coordinator immediately after engine restart in 2.3.10;
it was reopened and assigned, and has subsequent reproduction discussion. Treat
it as a regression candidate to reproduce on the selected version, not proof that
all current deployments fail. Do not infer maintainer intent from product wording.
[Actor documentation](https://rivet.dev/actors/docs/),
[restart issue](https://github.com/rivet-dev/rivet/issues/5554)

## FOKS compatibility and durability questions

**Storage mechanism:** Rivet describes a native SQLite VFS with demand-fetched
pages, a hot storage tier, S3 cold tiering and a guard against simultaneous writers.
Its July article says using this storage independently of Actors would require
extracting internal Depot libraries and adding a transport. That makes it a
plausible storage/activation architecture, but not an already supported filesystem
mount for unmodified `rusqlite` applications.
[SQLite storage design](https://rivet.dev/blog/2026-07-31-how-we-built-the-first-zero-disk-s3-tiered-storage-engine-for-sqlite/)

**Native integration:** the public Rust SDK is beta; lower-level crates have no
stability promise. An alternative beta Actor Runtime Socket exposes actor SQLite
to colocated processes over a Unix socket, with explicit transaction lease keys.
This could preserve FOKS wire messages while changing its database adapter, but
would require substantial work around its synchronous connections, transactions,
reader pool and backup APIs. Running a child FOKS process with its ordinary local
SQLite file does not automatically put that file into Rivet's durable store.
[Rust SDK](https://rivet.dev/actors/docs/quickstart/rust/),
[runtime socket](https://rivet.dev/actors/docs/actor-runtime-socket/)

**Limits require reconciliation:** the reference lists 10 GiB total storage per
actor and 1,310,720 raw dirty SQLite bytes per commit. The July storage article
instead describes databases without a hard ceiling. Treat this as unresolved
version/documentation scope, and test the pinned implementation. External FOKS
chunks avoid multi-megabyte chunk INSERTs, but migrations, bulk maintenance and
large metadata transactions still need a dirty-page audit. Splitting a transaction
must preserve its original atomicity. Maximum-size FOKS RPCs also need transport
limit qualification.
[Limits](https://rivet.dev/actors/docs/limits/)

**Transport:** documented actor ingress is HTTP/WebSocket and SDK actions; the
runtime socket is local SQL access, not public TCP ingress. I did not establish a
supported raw TCP/mTLS pass-through path that preserves existing Go clients.
Inference: FOKS needs a separately qualified native ingress/adapter, maintaining
HostID, certificate validation and device identity. A new HTTP actor API requiring
client changes is outside the user's compatibility constraint.
[Architecture](https://rivet.dev/docs/architecture/),
[runtime socket](https://rivet.dev/actors/docs/actor-runtime-socket/)

**Zero loss:** generic actor state autosaves are throttled (default one second),
not tied to action completion; immediate saving is explicit. Therefore generic
actor-state persistence cannot substitute for FOKS's durable commit boundary.
[State persistence](https://rivet.dev/actors/docs/state/)

The VFS architecture is promising, but this review has not established its exact
stale-generation fencing and output visibility behavior for our integration,
especially reads/errors while replication is pending and old native sockets
surviving promotion. Nor does PostgreSQL primary/replica failover alone establish
zero RPO: replication mode, placement and promotion policy matter. Verify these
at the selected backend's commit boundary, including keys and maintenance paths.
The current multi-region guide uses shared storage and regional endpoints; it
is not evidence of a measured 15-second AZ recovery bound.
[Multi-region topology](https://rivet.dev/docs/deploy/self-host/control-plane/multi-region/)

## Brief comparison with celld

Celld's published contract is more explicit about epoch-prefixed storage, fencing
paused owners and holding writes, reads, errors, streams and TCP effects behind
durability proofs. It also documents a default 10-second node lease and potentially
minutes-long recovery of a large failed node, with surviving follower data needed
in some recovery cases. Those qualifications are directly relevant to our 15-second
target. Neither celld's contract nor Rivet's actor cold-start claims prove that the
native FOKS integration meets it.
[Celld guarantees](https://celld.dev/docs/guarantees/)

Inference: released Rivet offers an established actor ecosystem with a heavier
shared-service deployment; announced 3.0 aims at a simpler S3-dependent footprint
that deserves reevaluation once pinned and testable. Celld currently offers a
clearer published durability model to audit. Keep both separate from the existing
native server's measured local-restart result.

## Next gate before a deployment prototype

Time-box a source/API feasibility spike, with no production infrastructure:

1. Pin released Rivet and matching Rust SDK; separately track 3.0 readiness.
   Resolve storage limits, supported backend/license and the actual dependency set.
2. Demonstrate one unmodified Go client's TCP/mTLS session reaching a FOKS tenant;
   reject any route requiring new FOKS wire fields or weakening authentication.
3. Run the FOKS schema and representative atomic mutations through the proposed
   VFS/SQL adapter. Test commit ambiguity, dirty-page limits, migration, backup and
   key recovery. Obtain a concrete fencing/output-gating design before proceeding.
4. Only if those pass, provision a disposable multi-node, cross-AZ topology and
   run the controller-owned fault ledger: pause old owners, partition storage,
   kill a host/AZ, preserve client pins, and require successful authenticated reads
   plus a new durable write within 15 seconds. Audit all observed durable state
   separately; measure simultaneous tenant recovery and resource headroom.

Stop at the feasibility gate if stable native integration or client-compatible
routing is unavailable. No tenant-density, zero-loss or recovery-time claim is
established by this assessment.
