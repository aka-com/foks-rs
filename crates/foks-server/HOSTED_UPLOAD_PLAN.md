# Durable client upload recovery plan

Status: proposed, not implemented. Prepared against implementation commit
`3c60f78e` for the hosted rollout review. This is a client durability change;
wire encodings, file IDs, cryptography, padding, authorization and Go-compatible
RPCs remain unchanged. It does not establish server replication or a 15-second
host/AZ recovery guarantee. Keep this plan unstaged for review.

## Problem and existing boundaries

`KvWriteSession::put_file_with_size_bound` in
[write.rs](../foks-client/src/kv/write.rs) creates a random file ID and seed on
each call. It sends `UploadInit` and subsequent chunks before
`prepare_namespace_mutation` journals the final directory publication. A caller
retrying `put_file` after a lost response starts a different upload. The
external-chunk test initially demonstrated two charged uploads when it did this;
its corrected byte-identical authenticated request replay demonstrates the
server already provides the necessary chunk idempotency.

Existing recovery in `resume_namespace_mutation` authenticates and reconciles an
ambiguous directory mutation; it is not a chunk upload journal. Preserve that
more conservative behavior. Do not globally make `SubmissionUnknown` replayable
in [MutationCoordinator](../foks-client/src/mutation.rs).

[EncryptedFileMutationStore](../foks-client/src/protected_store.rs) already
provides authenticated encryption, durable no-replace installation and separate
master-key custody. Its 128 MiB per-record bound is not a total spool budget.
[ProtectedRecordInventory](../foks-client/src/protected_inventory.rs) defines
which records are live, removable and exportable; upload records must participate
before cleanup or state export can safely understand them.

## First implementation: immutable encrypted preparation

Prepare the complete encrypted upload locally before sending its first chunk.
This intentionally favors reliable restart over overlap between reading an
arbitrary input stream and networking. A later streaming-source mode can be a
separate design with explicit resumability limits.

1. Under profile operation exclusion, allocate an opaque upload operation ID,
   one file ID, one file seed and fixed metadata/nonces. Bind the operation to
   HostID, local user, target party/team, destination directory, intended name,
   overwrite policy and selected dirent identity/version. The name and any local
   source path belong only in protected material.
2. Durably install the protected preparation manifest, then record its owner in
   hard-state SQLite before any network submission. Read the input once using
   the existing chunk size/padding/encryption code. Store each exact canonical
   chunk as an individually protected immutable record; fsync it before inserting
   its descriptor, hash, offset and ciphertext length in the journal.
3. Enforce the existing 1 GiB plaintext ceiling, declared-length checks and
   chunk-order/final-size invariants while preparing. Unknown-length streams keep
   the existing metadata behavior: do not invent a declared size or amend
   metadata after `UploadInit`. Validate EOF before marking preparation complete.
4. Atomically mark the complete ordered manifest `Ready` only after every record
   is durable and the manifest totals agree. A process crash during preparation
   causes no remote writes. First release may mark such a partial preparation
   `NeedsSource` and require a new preparation with a fresh file ID/seed; it must
   not silently reopen a path and continue encryption with changed input.
5. Upload using the persisted metadata and exact chunk encodings. Persist
   `Submitting` before each attempt and an acknowledgment afterward. A restart
   or ambiguous outcome retains the original operation and bytes.

Never encrypt different plaintext with a previously used file seed/ID/offset/
finality tuple. File length, mtime and inode are not sufficient proof of unchanged
source content. Complete encrypted preparation removes that unsafe inference.
Once preparation is complete, deleting or changing the source cannot change the
upload. If secret material is missing or corrupt, fail closed; do not generate
replacement material under the existing identity.

## Journal, limits and secret handling

Add a dedicated typed upload journal in `foks-client-db`, rather than representing
hundreds of chunks as independent generic namespace mutations. Suggested states:
`Preparing`, `Ready`, `Uploading`, `ContentComplete`, `Publishing`,
`RemoteVerified`, `Finalized`, `NeedsSource`, `Blocked`, `Cancelled`.

Use a parent upload row plus ordered chunk descriptors. Public rows may contain
opaque operation/file IDs, host/party binding, versions, ciphertext fingerprints,
byte counts, attempt state, timestamps and child namespace operation IDs. Keep
file seeds, names, source paths, plaintext digests and any bearer material out of
public SQLite, errors and logs. Credentials are reloaded by the existing credential
provider; do not embed a reusable device private key or a team bearer token in
all chunk records.

Extend the protected key vocabulary with domain-separated upload-manifest and
upload-chunk keys bound to operation and sequence. Authenticate record scope and
manifest hashes on every reopen, including an offset/final marker and the expected
file ID. Reuse the current protected-store master-key protection. Process one
chunk at a time with zeroized temporary secret buffers; never combine a 1 GiB
upload into one protected record or raise the existing per-record bound to permit
that. Already-encrypted wire chunks still receive local protected-store encryption
to keep filename/material relationships and local export/cleanup rules uniform.

Bound preparation before consuming disk: per-operation plaintext and encoded-byte
ceilings, per-profile aggregate reserved bytes, number of pending uploads and
preparation workers. A maximum-size file needs roughly 2 GiB of canonical encrypted
spool, plus manifests, protected-record overhead and temporary-file headroom.
Specify the exact worst-case reservation from the existing encoder's limits and
reject insufficient capacity before submission. Unknown-length inputs reserve
incrementally under the same hard aggregate cap; do not rely on free-space checks
alone when concurrent preparations share a profile. Count temporary and
terminal-pending-cleanup files until they are actually removed.

Do not promise a universal default disk allowance before measuring typical
profiles. The first release must expose the configured limit and return a clear
local-capacity error without publishing a partial directory entry. Do not spill
plaintext to a normal temporary file, and do not upload remotely as an implicit
fallback when local preparation cannot be made durable.

## Resume and publication ordering

Dispatch by persisted upload and namespace-child state before attempting any
content transmission. If a child is `Submitting` or `SubmissionUnknown`, first
use existing namespace reconciliation; do not reupload chunks or prepare another
publication. A successful publication may already exist even if access was
subsequently revoked or the destination changed. Inability to reconcile must
retain that ambiguity rather than restarting content upload. If the child or
upload is `RemoteVerified`, finish the durable application receipt and terminal
transition; if `Finalized`, return the persisted receipt and finish eligible local
cleanup. Neither state requires another upload or current destination permission
to report its already verified result. A definitely rejected child may return to
an appropriate earlier state only under the explicit intent-preservation rules
below. Only operations without an unresolved or successful publication enter
content submission or replay.

Keep at most one in-flight chunk per upload. Chunk RPC application arguments are
identical on replay; connection sequence numbers and fresh authentication are
transport details. On an ambiguous chunk response, reconnect and replay that exact
request. Treat conflicts, revoked access and quota exhaustion as distinct blocked
conditions; retain recoverable material and never turn them into a new upload ID
automatically.

The existing protocol has no upload-progress query and the server can reclaim
unreferenced uploads after an abandonment interval. A local acknowledgment cursor
alone therefore cannot justify skipping chunks after restart. First implementation
replays from `UploadInit` through every prepared chunk when the state dispatch
above permits a resumed content submission,
including previously acknowledged chunks. This recovers safely if server cleanup
removed the old upload; it also preserves exact replay if the upload still exists.
A server currently reclaiming it can reject requests; retain state and surface a
retryable/blocked outcome rather than changing identity. Mid-replay reclamation or
an ambiguous response restarts the conservative replay pass. Never infer durable
absence from one failed read.

This can retransmit roughly 2 GiB for a maximum-size file. It is an explicit
wire-compatible correctness tradeoff, not a claim that full-file resumption
finishes within 15 seconds. The hosted recovery objective must be measured through
successful authenticated operations separately from completing an arbitrarily
large user transfer. Optimize replay only after proving a compatible server state
check that cannot mistake a reclaimed or incomplete upload for a complete one.

After final-chunk acknowledgment, record `ContentComplete`. Refresh authorization
and destination state before preparing a directory publication. Preserve selected
dirent ID/version and overwrite intent; a path deleted and recreated by another
writer must not silently become the new overwrite target. If membership, role or
key generation changed, suspend for reconciliation rather than silently modifying
already persisted metadata. Re-encrypting content is a new explicit operation.

Close the current gap between `ContentComplete` and directory publication:

* Durably install the exact namespace material first, then atomically insert its
  existing `MutationOperation` row and its association to the upload in one
  hard-state transaction. Both owners must refer to the same file ID and target
  binding. Avoid an after-the-fact association that can be lost after a committed
  directory mutation.
* Preserve existing `KvAdapter` parent/child ownership: model upload-to-namespace
  association separately or extend ownership deliberately, rather than replacing
  an adapter's existing parent pointer. A unique operation linkage must survive
  crash before either workflow sees the other's success.
* Submit and recover the directory operation through existing authenticated
  namespace reconciliation. A `Submitting`/`SubmissionUnknown` namespace operation
  must not cause a second publication, a fresh dirent ID or a new blind overwrite.
  Definite precondition rejection may prepare a new namespace child only after
  preserving the original user intent and durably terminalizing the rejected one.
* After remote verification, durably commit the application/adapter receipt and
  upload terminal state before removing protected chunks. Cleanup is incremental
  and restartable. A crash during cleanup must not lose the success receipt or
  cause a new remote mutation.

Cancellation stops new submissions and preserves any ambiguous publication for
reconciliation. It does not promise deletion of server objects, secure erasure of
storage media, or undo of an acknowledged directory mutation. Material needed to
resolve ambiguity cannot be discarded merely because the UI stopped waiting.
An explicit local-abandon action must clearly describe the loss of local resume
ability and possible remote orphan data.

## API and rollout slices

Suggested Rust API boundary: `prepare_file_upload(...) -> UploadHandle`,
`resume_file_upload(handle)`, `upload_status(handle)`, and
`cancel_file_upload(handle)`. Handles are opaque and bound to the current profile,
host and party. Status reports progress and actionable blocked conditions without
paths, keys or raw protected identifiers. Keep `put_file` convenience wrappers,
but return/expose a handle before network submission and make application-level
retry use that handle. A new call without a handle remains a new operation.

1. Add versioned hard-state tables, protected record families, inventory ownership,
   resumable cleanup and bounded reservation APIs. New binaries migrate existing
   profiles with zero new uploads. Old binaries must reject the newer hard-state
   schema; downgrading against pending uploads must not silently discard them.
   Existing already-started uploads cannot be reconstructed from server ciphertext
   alone without their original private material.
2. Refactor `put_file_with_size_bound` into pure preparation and a submission engine
   while preserving existing crypto helpers. Add deterministic preparation/reopen
   tests before networking. Keep small-file behavior unchanged initially; durable
   small-file content preparation can be a separately scoped follow-up.
3. Implement exact chunk replay and the atomic namespace association. Extend
   `MutationCoordinator` with a narrowly scoped association transaction rather than
   weakening the generic ambiguous-mutation replay rule.
4. Integrate [client-app KV operations](../foks-client-app/src/kv.rs), agent/CLI and
   desktop flows with persistent handles, per-profile locking, cancellation,
   progress and capacity errors. If agent DTOs change, version that local IPC
   boundary and build the standalone agent before CLI integration tests; this is
   separate from the FOKS network protocol.
5. Extend protected-state inventory, export/import, reset and relocation workflows.
   Initially reject export/import of live uploads consistently with current
   nonterminal mutation policy; do not export only the journal without its spool.
   Test profile relocation and keychain unavailability explicitly.

Each implementation slice receives direct review and independent subagent review
before its tests and commit. Use separate commits for journal/storage, replay and
publication, then application integration.

## Validation and acceptance criteria

* Inject process crashes between protected record fsync, journal insertion,
  readiness marking, every chunk send/commit/response/ack persistence, namespace
  preparation/association/send, application receipt and cleanup. Reopen all client
  state and retain original server/client pins. The completed operation must keep
  one file identity and produce at most one intended directory publication.
* Lose `UploadInit`, middle and final responses. Compare exact replayed arguments,
  stored file/chunk counts and logical usage against an uninterrupted upload.
  Verify decrypted full content, finality, offsets and encrypted-size totals.
* Exercise empty/small-file dispatch, 2040/2041-byte boundary, exact 4 MiB chunks,
  chunk-plus-one and the 1 GiB limit using generated streaming input. Cover
  declared-length mismatch, unknown length, short reads, source failure and disk
  exhaustion without allocating the entire input in memory.
* During resume, rotate keys, revoke the device, remove team membership, change
  roles, rename/delete/recreate the target and race another overwrite. Demonstrate
  no unauthorized publication or unintended target replacement.
* Lose the directory-publication reply after its commit, then revoke access or
  change the destination before resume. Reconcile the existing child before any
  chunk replay; either recover its verified receipt or retain explicit unresolved
  state if current authorization prevents proof. Never manufacture a second
  publication or let a redundant upload failure replace the original ambiguity.
  Reopen `RemoteVerified` and `Finalized` operations after the same changes and
  confirm receipt completion/return requires no new upload.
* Run server reclamation between initial transmission and resume, including a
  partial reclamation pass. Verify replay-from-init recovery where possible and
  retained blocked state where access or current reclamation prevents it. Do not
  conceal this by advancing timestamps without running actual maintenance.
* Tamper with, swap or delete protected records; replay an older manifest; reuse a
  handle across users/hosts; run two resume workers. Fail binding/integrity checks
  and enforce one profile owner without leaking secret material.
* Validate cancellation before submission, during an ambiguous chunk and during
  ambiguous namespace publication. Incremental cleanup must respect all owners,
  including adapter receipts and protected-state export snapshots.
* Repeat Rust and pinned Go v0.1.9 compatibility gates with unchanged upload/get
  encodings against inline and external chunk storage. For a Go server retry
  qualification, verify the actual server's exact-upload replay semantics rather
  than assuming Rust-server behavior proves them. No new server RPC is required.
* Record local peak memory, preparation latency, reserved/actual spool bytes,
  fsync count and retry network bytes. Show that a restored upload no longer
  requires its original source file and does not create a second charged upload.

Acceptance: completed preparation survives client process death; exact immutable
content survives ambiguous responses and server restart; namespace reconciliation
preserves user intent; missing credentials or material fail closed; cleanup and
resource consumption remain bounded. This is resumable transfer, not a substitute
for the hosted metadata durability and ownership qualification.

## Execution addendum (2026-10-15)

The implementation owns client-only surfaces: `foks-client-db` schema/repositories,
`foks-client` KV and protected-store coordination, `foks-client-app` KV/adapter and
portability integration, `foks-agent-proto`/`foks-agent-client`, agent dispatch,
direct CLI commands, and desktop transfer/pending-operation flows. Server files,
FOKS RPC definitions and cryptographic constructions are outside this item.

Hard-state schema 40 migrates to 41. Add upload parent rows and ordered protected
chunk descriptors, bounded reservation accounting and explicit terminal receipts.
Include every new table in hard-state revision tracking. Extend protected-owner
inventory and export/reset/relocation checks together with the schema, so no
existing maintenance pass can mistake live upload material for an orphan. Existing
profiles migrate with no upload rows; old readers/writers reject schema 41.

The application API uses an opaque caller-allocated operation ID before reading
or transmitting source content. Local IPC version 34 first registers an
`AwaitingSource` row with host/user/party binding and reserves a pending slot.
The streaming worker atomically consumes that identity before its first source
read, then sends an `upload_ready` acknowledgment; only then does the caller
read or send source bytes. Interrupted registration is discoverable by ID, and
interrupted consumption cannot restart preparation under the same ID. The IPC
also supports upload list/status/resume/cancel operations.
List/status permit rediscovery after client or agent restart. This local IPC
change is independent of unchanged FOKS network compatibility. Direct CLI `kv put`
uses the same journal and exposes a handle; explicit resume never requires the
input path. Desktop pending uploads use durable agent state rather than only the
current in-memory source-path tokens. A new user upload action allocates a new ID;
an automatic retry must retain the old ID.

Preparation records manifest ownership and its reservation in the durable journal
before installing the protected manifest, then installs individually protected
exact chunk encodings. A crash or installation failure leaves a charged, cancelable
preparation; inventory permits a missing manifest only in the source-dependent
preparation states.
Reserve before allocating/writing each chunk, including its 48-byte protected
record envelope and bounded temporary-file allowance; reservations survive crashes
and cleanup failures. Configure and expose aggregate spool bytes and pending-count
limits. Count temporary records until cleanup actually removes them. A maximum
4 MiB clear chunk pads to approximately 8 MiB before canonical/protected overhead;
compute checked bounds from encoder limits rather than assuming input size equals
spool size. No plaintext disk staging or oversized single protected record is
introduced. Interrupted preparation is source-dependent and cannot restart
under the same file ID/seed; completed preparation is immutable and source-free.

Bind the protected manifest to host, local user, party/team, parent directory,
name, roles/key generation, file identity and exact selected dirent identity and
version. Capture selected identity even for legacy version-only overwrite calls.
Ancillary root/mkdir-p operations may precede preparation using their existing
journals; the no-remote-write-before-ready guarantee applies to the upload's file
content and final publication, not those explicitly requested ancestor changes.
Keep existing small-file wire dispatch unchanged. If a caller allocated an upload
ID before learning that the source is small, consume it into an explicit terminal
`inline-untracked` identity. That outcome releases the pending slot and claims
neither successful publication nor source-free recovery; callers must inspect the
destination after an ambiguous inline result.

Resume dispatches terminal and publication state before refreshing remote
permissions. Verified/finalized receipts can be returned from durable local state.
An unresolved namespace child is reconciled first and never causes renewed file
content submission. Otherwise replay init and every stored chunk from offset zero,
using current credentials and the original application arguments. Resume never
uses a local acknowledgment cursor as proof of remote retained content. Preserve
blocked state and identity on quota/auth/conflict failures. Revalidate immutable
selection and roles/generation before new publication. Install namespace material,
then atomically insert namespace row, existing adapter-child ownership and upload
association in one transaction. Application receipt and upload finalization precede
incremental spool erasure; cancellation cannot erase unresolved publication proof.
When publication is definitely absent/rejected, cancellation atomically records
the linked adapter rejection and any already committed ancillary changes before
cleanup, so the application cannot retain a permanently pending parent.

Implementation review gates cover the complete journal/storage, replay/publication
and application/IPC integration boundaries. Tests follow direct self-review and
independent review, then the item is committed separately from server changes.
Required checks include migration/maintenance ownership, partial preparation,
record tampering, per-profile capacity and cleanup debt, lost init/middle/final
responses, source deletion, overwritten/recreated destination, publication-response
loss, revoked access after verification, concurrent profile owners and restart
through agent/CLI/desktop. Rebuild the standalone agent before CLI integration;
run native desktop tests serially. Keep all planning Markdown unstaged.

Implementation bounds: default aggregate protected spool reservation is 256 MiB,
with 16 pending uploads, adjustable locally from 16 MiB through 1 TiB and 1–128
pending uploads. Each record is charged its exact canonical encoded length plus
48 bytes of protected wrapper before filesystem installation; one maximum-size
record per operation is additionally reserved for temporary-file cleanup debt.
Prepared files use at most 256 records. The existing protected inventory remains
bounded and fails closed when saturated; raising spool/pending limits does not
remove that independent export/maintenance bound.

**Rollout blocker: permanent receipt capacity.** The first implementation retains
at most 1,024 upload receipts/tombstones per
profile and does not automatically delete them or reuse their IDs. This includes
preparation intents, cancellations and explicitly identified small-file operations,
including the native desktop file-upload path. Ordinary small writes with no upload
identity do not consume this ledger. There is no supported forget/retire operation.
Reaching this
bound explicitly refuses new preparations; safe receipt aging requires a versioned,
expiring identity scheme and trusted retention clock in a follow-up. This is an
operator-visible limitation, not a claim of unlimited lifetime writes. Broad
desktop rollout requires bounded receipt retirement with identities that remain
unusable after retirement; increasing the spool-byte setting does not remove this
lifetime ceiling. Cleanup
removes chunk descriptor rows after their bytes and temporary files are durably
erased, while keeping the compact identity and success/cancellation receipt.

Desktop has an on-demand Uploads panel for each selected store. Its source-free
resume command uses a narrowly scoped reconciliation reservation: it may inspect
or finish a durable upload while an earlier response is ambiguous, but it preserves
in-flight exclusion and does not clear the ordinary catalog refresh requirement.
Account/team bindings are checked again by the agent/application layer. Local IPC
is version 34; no remote FOKS protocol or cryptographic encoding is changed.
