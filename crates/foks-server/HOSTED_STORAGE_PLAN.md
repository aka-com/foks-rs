# Hosted storage implementation plan

This work keeps the go-foks v0.1.9 RPCs, encrypted chunk encoding, padding,
authentication, and client rollback protections unchanged. The hosted target is
personal servers with files and organizations using credentials and small notes,
with the existing 64-member team cap. Successful recovery within 15 seconds is a
qualification target, not a guarantee supplied by these changes.

## Sequence and review gates

1. Materialize transactional quota and upload counters; reuse validated reader
   configuration in the bounded pool. Review directly and independently with a
   subagent before tests, then commit.
2. Add a controller-owned recovery ledger and real-binary crash/restart and stale
   backup experiments. Review directly and independently before tests, then
   commit. This establishes a local durable-restart baseline, not replication.
3. Review this storage design before implementation. Implement external chunks,
   review directly and independently, run correctness and compatibility gates,
   then commit. Finish with an independent system review and replication roadmap.

## External chunk representation

Make external storage opt-in; existing installations continue using SQLite.
Support mixed inline/external rows so upgrade never requires copying all files
at startup. A new schema migration adds a discriminated external reference with
SHA-256 of the canonical encoded upload chunk, encoded length, and ciphertext
length. An external row contains no ciphertext or duplicate encoded payload.
Store one immutable canonical upload-chunk object, addressed by its digest.
The digest covers offset, final-upload information and ciphertext, preserving
exact retry identity. This is an internal storage checksum, not a wire change.

Keep namespace and per-upload accounting correct for either representation.
Initially retain existing quota charging (ciphertext plus exact encoding) so
backend selection alone does not change quota admission or go-foks usage replies.
The representation may change where bytes reside without changing charged bytes.

Use a bounded synchronous store interface from existing blocking handlers, with
filesystem and S3/R2-compatible implementations. Use a maintained S3 client
library, not handwritten request signing. Configuration names an explicit bucket,
endpoint, region and prefix; secrets come from the credential provider/environment,
not committed config. Require HTTPS except explicit loopback test endpoints.
Bound remote operations and downloaded bytes; validate digest, length, decoded
offset and final flag before returning the existing encrypted-chunk response.
One deadline covers each whole store operation, including conditional PUT and
verification GET/retries. Share the runtime/client; do not create a runtime per
request. Unsupported conditional creation and ambiguous transport results must
fail or reconcile safely, never fall back to unconditional replacement.

## Publication, reads and failures

Validate authorization and obvious retry/order/quota errors before object I/O.
Write the immutable object outside the SQLite writer; then recheck current
authorization, upload state, ordering and quota in the committing writer job.
Only acknowledge after object publication and SQLite commit both succeed.
Do not delete an object following a timeout or ambiguous commit outcome.
Concurrent retries must converge on the same immutable bytes; conflicting retries
must retain existing protocol errors. An interrupted upload may leave an orphan.

Download lookup/authentication runs in a short read snapshot. Copy the reference
and end the snapshot before object-store I/O, rather than pinning the WAL across
a network request. Missing or corrupt objects fail closed; they never become an
empty or substituted successful response. Refuse startup with external references
but no configured store. Operator-selected store/prefix must remain stable across
restart and failover.

Filesystem publication writes a private temporary file, fsyncs it, atomically
publishes without replacing another object, and fsyncs the directory. Reject
symlink/nonregular object leaves and verify an existing object's contents.
S3 publication uses conditional creation and verifies an existing object.
Synchronous filesystem I/O cannot enforce a deadline on a hung mount. Use the
filesystem backend for local storage qualification; the remote deadline applies
to the S3 backend. Neither backend alone supplies a 15-second recovery guarantee.

## Backup, migration and reclamation

Preserve the self-contained backup contract: hydrate external rows into the
private backup SQLite copy before integrity checking and publishing the final
manifest. Hydration reads and rewrites one bounded chunk at a time. Existing
restore therefore needs neither original bucket nor object-store credentials.
Incomplete hydration must never publish a complete backup. Backup temporary disk
and configured database limits must accommodate the materialized inline data.
Backups still incur the old payload amplification; failover should use replicated
metadata and durable objects rather than full-backup hydration.
Hydration applies to library, scheduled and standalone CLI backups. The CLI must
receive explicit store configuration when backing up external references; old
backup validation and restore remain independent of the source backend.

Existing inline rows remain readable. Provide an offline, resumable conversion
command for existing installations; each converted row is published only
after its object is durable. Do not silently convert during startup.

Do not introduce online object deletion without coordinated publication/read/
backup protection. Initial external objects are retained even when SQLite removes
their references. Document orphan and removed-upload growth explicitly; bucket
lifecycle deletion is unsafe. A later collector must coordinate all live writers
and backups or operate under exclusive offline ownership. No file-history pruning
is required for disaggregation; keep historical reference semantics for this step.
Document this physical-capacity limitation in the operator README as well as here.

## Acceptance and review

- Counter migrations preserve accounting, retries, rollback and cleanup; production
  quota-query VM work stays bounded as retained history grows.
- Existing inline tests continue passing. External and mixed storage exercise
  multi-chunk upload/download, identical/conflicting retries, quota and final-size
  checks, authorization changes during I/O, and object publication failures.
- Corrupt/missing/oversized objects fail; object I/O does not hold a writer job or
  read snapshot. Backend deadlines and request admission remain bounded.
- A completed backup restores and downloads files after the original object store
  is removed. Failed hydration leaves no completed backup. Test old schemas too.
- Run actual pinned go-foks client upload/download/usage coverage with both storage
  choices; extend the live gate beyond small files. Protocol generated artifacts
  and wire/crypto fixtures must not change.
- Exercise S3/R2 through a local compatible test endpoint and offer an explicit
  live-service gate. Local/mock success is not a provider durability qualification.

## Subsequent HA work

Choose the actual replication platform, acknowledgment durability boundary and
cross-AZ placement. Fence stale owners, including queued writes, before publishing
client-visible state. Preserve keys and HostID, bound recovery and routing time,
and keep enough capacity for concurrent tenant activation. Extend the external
ledger harness to host/AZ loss, split brain, resumed old owners and delayed
replication. Never clear client trust anchors to make stale recovery pass.
Existing rollback checks do not detect every missing newly created KV entry;
the independent recovery ledger must also detect acknowledged writes disappearing.
