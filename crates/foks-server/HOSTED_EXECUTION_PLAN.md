# Hosted hardening execution plan

Requested 2026-10-15. This execution plan and all other Markdown remain unstaged.
Implementations are separate commits. Existing RPCs, encrypted formats and Go
compatibility remain unchanged. Organization membership remains capped at 64.

## Assumptions and qualification boundary

Treat correctly configured managed S3 as durable, highly available and strongly
consistent, including its documented conditional operations. Test our usage of
those operations, not the provider's internal replication implementation. Local
qualification is authorized; no AWS environment is available. Local emulator
results establish adapter/controller behavior, not remote S3 latency or AZ service
availability. Do not weaken fencing or ambiguous-operation handling based on the
storage assumption: an interrupted request can still have committed remotely.

## Sequence

1. Storage namespace binding: schema row and immutable marker, startup checking,
   conversion/backup enforcement, explicit legacy adoption and hydrated restore.
   Follow HOSTED_STORAGE_FOLLOWUP_PLAN.md section 1. Do not silently bind an
   existing externally referenced DB to a new location.
2. Resource ownership: separate DB preparation from external transfer; release
   reader leases; retain allocation charges through background work, writer queues
   and socket transmission. Follow HOSTED_CAPACITY_PLAN.md. Start from observable
   conservative bounds; include non-chunk and asynchronous response paths before
   claiming full coverage. Density-driven shared-runtime redesign is conditional
   on measurements, not a prerequisite to this correctness slice.
3. Physical object accounting: durable publication reservations, present/uncertain
   bytes, reference tracking and bounded attempts; report-only reconciliation.
   Keep client-visible logical quota unchanged. No deletion in this commit.
4. Durable client uploads: protected immutable preparation, bounded journal/spool,
   stable handle, exact replay, atomic association with directory publication,
   state-first ambiguity reconciliation and application integration. Follow
   HOSTED_UPLOAD_PLAN.md. Server protocol remains unchanged; local IPC may need
   its existing versioning mechanism. Preparation and retransmission costs must
   be explicit in API/status behavior.
5. Coordinated reclamation: non-reused object identities, pins/barriers and
   tombstones; offline exclusive collection first. Report-only wherever storage
   or ownership cannot establish safe deletion. Hosted online collection requires
   the qualified replicated ownership boundary. History pruning is optional and
   outside this commit; existing historical references continue protecting data.

Client work may be prepared in parallel in disjoint files, but commits follow the
sequence above. Resolve each component's review findings before its tests and
commit. Do not stage another component's files or any Markdown.

## Review and verification gates

- Before implementation: author checks existing detailed plans against code;
  independent subagent reviews interfaces, migration, crash states and acceptance.
- Round one after implementation: direct review of every changed code path,
  including resource ownership, cancellation, migration/backup and compatibility.
- Round two: independent subagent review with the diff and acceptance contract;
  fix findings and obtain targeted re-review where correctness changed.
- Then run focused fault/compatibility tests, formatting and relevant lints.
  Build the standalone agent before CLI integration tests after client-state/IPC
  changes. Broaden suites when touched integration boundaries require it.
- Commit only the reviewed implementation and tests, with summary and explanatory
  paragraphs. Record evidence and remaining qualification limits in unstaged docs.

## Celld integration and capacity work

Design/review an actual FOKS durable-store adapter proof alongside these changes.
The proof must execute real FOKS transactions against celld authority, preserve
TCP/mTLS/Go compatibility and keys, and route observations through the durability
and ownership boundary. A replicated toy counter or native local SQLite in an
ephemeral container does not qualify. Prefer bucket durability for the first
proof; record the exact release and local object-store implementation.

Use the external acknowledgment ledger for crash, lost response, stopped/resumed
old owner and recovery. The 15-second clock covers the entire affected request/
recovery path; retryable errors alone are not success. Run an initial disclosed
capacity matrix (idle/active tenants, history, writes, file transfers and recovery
headroom) at sizes the local machine supports. Publish failures and resource
limits as well as successful results; no extrapolated 10,000-tenant or real-AZ
claims. Candidate adapter design and detailed execution are in
HOSTED_CELLD_INTEGRATION_PLAN.md once reviewed.

## Progress

- Existing implementation baseline: 2bcf7ec0.
- Plan review started; namespace, client and celld reviews delegated independently.
- Item 1 committed as `83895e55`: namespace binding and explicit legacy adoption.
  Two review rounds, database/binding/backend/backup tests and pinned Go flows pass.
- Item 2 committed as `be58328d`: bounded request/response allocation ownership
  through socket and detached work; reader leases released before object transfer.
  Two review rounds completed. RPC 24, Snowpack 15 and DB 50 unit tests pass;
  server 94 tests passed initially and the one corrected test assertion then passed
  with all five memory tests. External-storage 6, transfer 2, and pinned Go
  inline/external 2 integration tests pass. Maximum 9 MiB upload fits the minimum
  512 MiB shared working allowance. This allowance is a cap, not eager allocation;
  SQLite caches, allocator, TLS/kernel buffers and runtime overhead still require
  measured RSS capacity limits.
- Item 3 committed as `76093923`: physical publication accounting and bounded
  report-only inventory. Author and independent reviews passed. Isolated schema-52
  validation passed: DB library 57 (one ignored), external DB 3, backup 7,
  external storage 8, transfer 2, and pinned Go 3. A test helper initially masked
  the production malformed-input error with quota preflight; the assertion now
  calls the production negative path directly, which preserves error precedence.
- Item 4 committed as `1e0b7410`: immutable resumable client uploads and application
  receipts. Initial intent, protected-file ordering, cancellation, and adapter
  reentry fixes passed independent re-review. 844 Rust tests pass (two ignored),
  including nine new recovery scenarios and real-agent IPC; standalone-agent
  build, TypeScript, ESLint and formatting pass. UI had 1,671 initial passes;
  the single CSS-token failure was corrected and all six affected tests pass.
  The permanent 1,024 upload receipt cap per profile remains a documented broad
  rollout blocker pending safe retirement, independent of spool capacity.
- Item 5 committed as `57790f38`: coordinated offline filesystem reclamation,
  durable deletion intents and non-reused object incarnations. Plan, author,
  root and independent subagent reviews passed, including backup/read-only
  serving locks, detached filesystem-work lifetimes, migration invariants and
  uncertain remote PUT exclusion. Isolated schema-53 validation passed 123 tests
  with two intentional skips: DB 61, schema 13, external DB 3, backend 13,
  storage leases 2, backup 6, external storage 11, transfer 2, pinned Go 3,
  client uploads 9. Formatting and whitespace checks passed. S3 deletion remains
  report-only; existing historical references continue protecting objects.
- Final review found that database-path locks did not exclude independent raw
  database copies sharing one filesystem namespace. Follow-up `c6040630` implements
  HOSTED_GC_AUTHORITY_PLAN.md: fresh v2 namespace authority pins and physical
  namespace leases; legacy/unproven namespaces remain report-only. Direct and
  independent plan/code reviews passed, followed by 122 passing tests and two
  intentional skips (including pinned Go compatibility and uncached clone reads).
  The pin does not detect same-inode external
  rollback or provide distributed fencing; official hydrated restore is supported.
- Celld proof/control harness passed independent review; local qualification uses
  bucket durability by default and a pinned local S3 emulator. Real host/AZ and
  managed S3 timing remain outside available qualification.
- The schema-53 capacity matrix is complete and retained, including deadline and
  preparation failures. Guarded three-second-lease trials passed with four active
  cells, including 64 idle cells, but sixteen active cells missed the target.
  Clock-discontinuous runs are separately invalidated; clock-guard unit tests pass.
  Final schema-54 proof validation passed: Rust 4, Node 6, clock guard 5,
  formatting and default/proof Clippy with warnings denied. Go compatibility,
  adapter fencing, paused-owner return and four simultaneous recoveries passed.
  The proof and harness are committed separately as `541bc156`; all Markdown
  remains unstaged. See HOSTED_CELLD_PROOF_RESULTS.md for timings and limits.
