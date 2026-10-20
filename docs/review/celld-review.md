# Review of the ported celld code

A review of the celld code as it exists on branch `claude/focused-mccarthy-3amy72` at `bca9eee`, after the frozen `origin/celld` branch (78 commits from `d3ee213`) was squash-ported in `6581535` and `bca9eee` on top of `224a6c4`. Twelve reviewers covered nine areas (backup, chunks, deadlines, desktop, kernel, kv-accounting, session, storage-contract, uploads) and three cross-cutting concerns (security, Go FOKS v0.1.9 compatibility, test quality), skipping problems already recorded in [second-review.md](second-review.md), [STATUS.md](STATUS.md) and `findings/*.md`. They produced 51 candidates. Two independent verifiers (a skeptic and a reproducer) checked each candidate against HEAD and, where it settled a question, by experiment; a third verifier decided three split votes. Four candidates were refuted. The remaining 47 merge into 46 findings: 7 high, 10 medium and 29 low priority (35 S, 11 M, 0 L); by origin, 40 introduced by celld, 4 pre-existing and unrecorded, 2 introduced by the port's resolution.

## Summary

Review of the celld code as ported onto branch claude/focused-mccarthy-3amy72 at bca9eee (commits 6581535 and bca9eee on 224a6c4). Twelve reviewers produced 51 candidates; two independent verifiers checked each one against HEAD, a third decided three splits, and four candidates were refuted. The remaining 47 merge into 46 findings: 7 high, 10 medium and 29 low (35 S, 11 M). The serious problems sit in the new subsystems celld introduced. The external chunk store and its physical ledger treat every failed or interrupted publication as a permanent charge against lifetime budgets of 128 attempts or 256 MiB with no resolution path and no online counter, pin a digest to an absent object after a single failed PUT so the Rust resumable client can never complete that upload, close chunk admission whenever an inventory cannot finish or a fresh installation's chunk directory is a mount root, and derive a chunk-store authority from the database file's path, device and inode that makes serve, backup and every offline command fail after a restore from a filesystem copy, a move or a VACUUM INTO, with no rebind; a restored backup started against its previous store ends in a binding no configuration can satisfy. The new admission pools add a hard-coded four-permit bulk class, held through the response write, that one account can use to refuse every large request on the host, and keep the pre-existing (unrecorded) queued input reservation that two header-only connections exhaust; the KEX receive budget equals the per-source connection cap. Resumable uploads replay UploadInit and every chunk from zero, which a Go v0.1.9 server rejects as duplicate keys so resume against the reference server fails exactly when it is needed; a kvPut request that never reached the server leaves an upload permanently un-resumable and un-cancellable; and every file put, including small inline saves, claims a receipt-bearing identity that both consumes an 8192-row 30-day budget and makes saves depend on the receipt clock. The agent's deadline plumbing builds its DeadlineExceeded responses after the control they are written under has expired, so every deadline becomes a bare connection close that clients report as a lost agent (reproduced), and the desktop's new 15 s command budget covers a whole serial catalog walk. Backup hydration re-inlines external chunks into a copy capped by the live database limit and reports the overflow as disk_full, and a stale -wal at a restore destination is replayed silently (pre-existing, reproduced). Most findings originate in celld; four are pre-existing defects that celld's recovery and resume paths made load-bearing (the input reservation, the stale WAL, the read-selection binding, the MCP upload cap), and two were introduced by the port's resolution (a schema-49 collision for pre-port developer databases and the schema-59 pin in the SQLite policy tool, plus the README's stale 15-second statements). Verifier corrections lowered several candidates: the retention 50 ms budget, the cleanup back-off, the desktop app-lock and read-selection findings and the tool test suites are real but low. All of celld's qualification code (the celld-proof feature, the wasm32 kernel build, the recovery harness, the tool test suites) sits outside CI, and the guarded-runtime tests are bound to one lab machine.

## Do next

1. **Resume replays UploadInit and every chunk from sequence 0; a Go v0.1.9 server rejects any replayed row as a duplicate key, so an interrupted large upload to a Go host can never complete and Resume loops forever** ([`c3-uploads-go-resume-replay-incompat`](#c3-uploads-go-resume-replay-incompat); high priority, effort M). Deterministically breaks resumable uploads against a Go v0.1.9 server after any interruption, which the project's compatibility goal and the recovery feature's purpose both cover; the in-repo tests pass only because the Rust server is replay-tolerant. M, but the first step (resume from acknowledged_chunks) is small.
2. **Every failed, interrupted or ambiguous chunk publication permanently consumes admission capacity; after 128 attempts or 256 MiB the installation refuses all new chunk uploads, with no resolution path and no online visibility** ([`c3-chunks-failed-attempts-exhaust-admission`](#c3-chunks-failed-attempts-exhaust-admission); high priority, effort M). Faults consume lifetime budgets that eventually refuse every new chunk upload host-wide, with no resolution path and no metric or log; reproduced. The export of the counters and the definite non-publication outcome are the urgent parts.
3. **The persisted chunk-store authority hashes the database file's canonical path, device and inode; after a restore from a filesystem copy, a move or a VACUUM INTO, serve, backup and every offline chunk command fail with 'chunk authority identity mismatch' and there is no rebind or downgrade** ([`c3-chunks-authority-mismatch-no-rebind`](#c3-chunks-authority-mismatch-no-rebind); high priority, effort M). Routine recovery operations (restore from a filesystem copy, a move, VACUUM INTO) lock the installation out of serving, backing up and every offline chunk command with no supported remedy; reproduced. Fix together with c3-backup-restore-into-bound-store-dead-end (S), which covers the other recovery path that bricks a restored database.
4. **Four hard-coded bulk permits, held through the response write, gate every chunk transfer and every request with an argument over 4 KiB; one account slow-reading downloads refuses all other large requests with 1012 and a connection close** ([`c3-session-bulk-pool-four-permits`](#c3-session-bulk-pool-four-permits); high priority, effort S). One low-privilege account can refuse every large request and chunk transfer on the host, and five benign downloaders collide; the configurable limit and metric are S.
5. **Request input memory is reserved with a queued acquire after only the frame header arrives and held until the response is written, so a few header-only connections stall every request of the same size class on every listener** ([`c3-session-input-budget-held-before-body`](#c3-session-input-budget-held-before-body); high priority, effort M). Two header-only connections from one address stall every request over 4 KiB on every listener (pre-existing, unrecorded), and celld's new small pool adds a variant against every ordinary RPC; needs the admission redesign the verifiers describe.
6. **A chunk whose first PUT genuinely failed can never be uploaded again: the digest stays pinned to an absent state-2 object and every retry takes the Verify path, which cannot republish** ([`c3-chunks-failed-put-pins-digest`](#c3-chunks-failed-put-pins-digest); high priority, effort M). After a single store-side PUT failure the Rust resumable client can never complete that upload and is told to retry forever; reproduced with exact replays.
7. **A publication whose kvPut request never reached the server leaves the upload permanently un-resumable and un-cancellable, pinning one of 16 pending slots and its spool bytes** ([`c3-uploads-stuck-submission-unknown`](#c3-uploads-stuck-submission-unknown); high priority, effort M). A kvPut request lost before the server accepted it leaves an upload permanently un-resumable and un-cancellable, pinning pending slots and spool bytes shared by every upload in the profile.
8. **The agent's DeadlineExceeded responses are never delivered: write_controlled_response refuses to write once the request control has expired, so every operation that reaches its deadline ends as a bare connection close that clients report as 'agent disconnected'** ([`c3-deadlines-expired-response-never-written`](#c3-deadlines-expired-response-never-written); medium priority, effort S). Every agent deadline becomes a bare connection close reported as a lost agent, so the typed DeadlineExceeded and ambiguity notices are dead text and the desktop fires its connection-loss handling; reproduced, S.
9. **Every KV file put claims a resumable upload identity before the size is known, so each small inline write holds one of 8192 receipt rows for 30 days and every save depends on the receipt clock trusting the system clock** ([`c3-uploads-implicit-identity-before-size`](#c3-uploads-implicit-identity-before-size); medium priority, effort S). Bulk small writes exhaust an 8192-row receipt budget for 30 days and every save depends on the receipt clock trusting the system clock; one S change removes both consequences for inline writes.
10. **Starting the report-only inventory clears inventory_complete, three conditions make a run impossible to finish, and a fresh installation whose chunk directory is a mount root never confirms the empty inventory; in every case chunk uploads are rejected with 1012 and nothing names the cause** ([`c3-chunks-inventory-gates-admission`](#c3-chunks-inventory-gates-admission); medium priority, effort M). A fresh installation on a mount-root chunk directory, or any inventory that cannot finish, silently rejects every chunk upload with 1012; reproduced, and the lost+found skip plus a startup log are small.

## High-priority findings

7 findings.

### c3-uploads-go-resume-replay-incompat

**Resume replays UploadInit and every chunk from sequence 0; a Go v0.1.9 server rejects any replayed row as a duplicate key, so an interrupted large upload to a Go host can never complete and Resume loops forever**

Priority: high · Type: compat · Origin: celld · Effort: M · Area: uploads · Merged: `c-uploads-go-replay-incompat`, `c-compat-upload-resume-replays-init-against-go-server`

resume_file_upload always calls begin_upload_pass, which resets acknowledged_chunks to 0 and submitting_sequence to NULL for states 1-3, then iterates every chunk row, sending UploadInit for sequence 0 and UploadChunk for the rest, ignoring what was acknowledged. The only tolerance for this replay lives in the Rust server (foks-server-db accepts an identical UploadInit and returns Ok for a byte-identical chunk at an existing offset), which is why the in-repo tests pass. Go v0.1.9 has no idempotent path: KvFileUploadInit runs fileUploader.run with plain INSERTs into large_file (primary key includes file_id) and large_file_chunk (primary key includes chunk_offset); RetryTx retries only pgconn.SafeToRetry errors, so a 23505 unique violation is returned as-is and ErrorToStatus maps it to GENERIC_ERROR; after the final chunk the file flips to active and assertUploading refuses any later chunk. A RemoteStatus error from self.call propagates with `?`, the row stays in state 2 and the next Resume runs the identical pass, so against a Go host the loop never terminates and the report keeps saying 'uploading'. Resume works against Go in exactly one case, when nothing of the pass was committed (crash before UploadInit reached the server); every interruption after the first commit is permanently unresumable, including the lost-reply case the recovery feature is advertised for. The exit is CancelKvUpload and a complete re-upload (the orphaned Go large_file row has refcount 0 and is reclaimed by GC). Uninterrupted first passes are unaffected, so ordinary uploads to Go hosts work. Pre-port the upload streamed once under a fresh node id with no resume, so a failed put re-run as a new put worked against Go. The reviewer's statement that the client cannot switch to a fresh file_id is wrong: the manifest retains the seed and chunks are sealed with seal_kv_chunk(&manifest.seed, file, ...), so the client can open its own spooled ciphertext and re-seal under a new file id without the plaintext source.

Evidence:

- [`crates/foks-client/src/kv/uploads.rs:741`](../../crates/foks-client/src/kv/uploads.rs#L741): resume_file_upload: begin_upload_pass then a loop from op.chunks[0]; UploadInit at 766; `?` propagation at 770/775
- [`crates/foks-client-db/src/repositories/uploads.rs:293`](../../crates/foks-client-db/src/repositories/uploads.rs#L293): begin_upload_pass resets acknowledged_chunks=0 and submitting_sequence for states 1,2,3
- [`crates/foks-client/src/kv/uploads.rs:548`](../../crates/foks-client/src/kv/uploads.rs#L548): manifest keeps seed; chunks sealed with seal_kv_chunk(&manifest.seed, file, ...) at 615, so re-sealing under a new file id is possible
- [`crates/foks-server-db/src/kv.rs:553`](../../crates/foks-server-db/src/kv.rs#L553): Rust server accepts identical UploadInit and byte-identical chunk replay (553-597); services/kv.rs:517-525 returns Void
- `/root/go/pkg/mod/github.com/foks-proj/go-foks@v0.1.9/server/kv-store/file.go:281` (Go module source): plain INSERT into large_file; insChunk plain INSERT into large_file_chunk (334-371); assertUploading (192-207); finalize sets active (430-460)
- `/root/go/pkg/mod/github.com/foks-proj/go-foks@v0.1.9/server/sql/foks_kv_store.sql:107` (Go module source): large_file primary key (short_host_id, short_party_id, file_id); large_file_chunk primary key includes chunk_offset (137)
- `/root/go/pkg/mod/github.com/foks-proj/go-foks@v0.1.9/server/shared/db.go:160` (Go module source): RetryTx retries only SafeToRetry errors; lib/core/errors.go:1747 default GENERIC_ERROR
- [`crates/foks-client-app/src/kv/uploads/tests.rs:52`](../../crates/foks-client-app/src/kv/uploads/tests.rs#L52): only Rust-server faults are exercised

Recommendation: Resume from acknowledged_chunks instead of 0 (drop the reset in begin_upload_pass and keep submitting_sequence as the one ambiguous chunk). When that chunk or a replayed UploadInit is refused by a non-stale RemoteStatus and the host is not known to be exact-replay idempotent, re-spool under a fresh file id and seed by opening each spooled chunk with manifest.seed and re-sealing, then run the pass again; or, if a positive probe is preferred, read the file's metadata (Go returns KVUploadInProgressError while uploading and the file becomes readable once active) to decide between 'content complete' and 'retry from a fresh id'. Add a testkit server mode that mimics Go's plain INSERTs (rejects a repeated UploadInit or UploadChunk) and a lost-reply resume test under it; failing that, document that mid-stream resume is Rust-server-only and have Resume return a definite non-retryable error on a Go host instead of looping.

Verifier note: Both verifiers confirmed high, M, by reading the Go source (no Postgres is available here, so it was not executed against a Go server). Corrections applied: resume against Go works only when nothing was committed; a lost reply to the final chunk fails with 'file not in uploading state' rather than a unique violation, with the same stuck outcome; the re-seal option is feasible because the seed is retained, which the reviewers ruled out. The Go orphan row is temporary. Not covered by the open Go-compatibility findings.

### c3-chunks-failed-attempts-exhaust-admission

**Every failed, interrupted or ambiguous chunk publication permanently consumes admission capacity; after 128 attempts or 256 MiB the installation refuses all new chunk uploads, with no resolution path and no online visibility**

Priority: high · Type: bug · Origin: celld · Effort: M · Area: chunks · Merged: `c-chunks-ambiguous-attempts-exhaust-uploads`

The physical chunk ledger is designed so that no timeout, missing object or metadata deletion releases capacity (chunk_ledger.rs:1), but it provides no mechanism, online or offline, that ever resolves an attempt with outcome=2 or re-qualifies a state-2 object. The only statements touching chunk_object_attempts are the UPDATEs in settle_chunk_publication (outcome 1 to 2 or 3) and recover_chunk_publications (1 to 2) and the DELETE in clear_private_chunk_ledger (backup and restore only); confirm_chunk_presence frees uncertain bytes but leaves the attempt unresolved (pinned by a unit test); record_chunk_inventory_page neither promotes a listed state-2 object nor resolves attempts; and collection skips any object with an attempt in outcome 1 or 2. Each of the following therefore subtracts permanently from the admission budget: any put_object error settled as uncertain, including S3 4xx responses that prove the object was not stored, connect or DNS failures past object_store's retry budget, filesystem errors such as ENOSPC on the temporary file, and a request deadline that expired before the store call began (deadline::check at chunks.rs:415, 602, 770); a crash or SIGKILL while a reservation is in flight (recover_chunk_publications converts each one); and a lost PUT reply whose client never replays the exact bytes. The defaults are 128 unresolved attempts and 256 MiB of uncertain bytes (about 28 full chunks). Once either is reached, reserve rejects every new digest with ChunkAdmission, which map_upload_error turns into RateLimited, so clients back off and retry a permanent condition. The counters are printed only by the offline inventory-chunk-store command; the Prometheus export and `status` carry nothing from chunk_physical_usage and no log names the cause. A reproducer confirmed that after one injected put failure the ledger holds unresolved_attempts=1 and uncertain_bytes=8254 and that three exact replays (each answered 1014) leave the counters unchanged. The README presents the caps as capacity settings; in effect they are lifetime fault budgets. Raising [chunk_capacity] maximum_unresolved_attempts / maximum_uncertain_bytes and restarting restores admission without editing rows, but the counters stay monotone, so this only moves the wall.

Evidence:

- [`crates/foks-server-db/src/chunk_ledger.rs:286`](../../crates/foks-server-db/src/chunk_ledger.rs#L286): reserve rejects when unresolved_attempts >= cap or reserved + uncertain + new > cap; reject() counts admission_rejections (511-515)
- [`crates/foks-server-db/src/chunk_ledger.rs:316`](../../crates/foks-server-db/src/chunk_ledger.rs#L316): settle: the only writer of outcome 2|3, requires outcome=1; confirm_chunk_presence (337-348) sets state 3 only
- [`crates/foks-server-db/src/chunk_ledger.rs:205`](../../crates/foks-server-db/src/chunk_ledger.rs#L205): recover_chunk_publications converts every in-flight attempt and state-1 object to uncertain at each start
- [`crates/foks-server-db/src/chunk_ledger.rs:399`](../../crates/foks-server-db/src/chunk_ledger.rs#L399): record_chunk_inventory_page never touches attempts or promotes a state-2 row; test at 646 pins that presence does not free the attempt
- [`crates/foks-server-db/src/chunk_collection.rs:15`](../../crates/foks-server-db/src/chunk_collection.rs#L15): collection candidates require NOT EXISTS attempt with outcome IN (1, 2) (also 39, 112)
- [`crates/foks-server/src/services/kv.rs:490`](../../crates/foks-server/src/services/kv.rs#L490): any put_object Err -> settle(false); ChunkAdmission -> RateLimited at 1349 with no operational log
- [`crates/foks-server/src/chunks.rs:798`](../../crates/foks-server/src/chunks.rs#L798): S3 put: every non-AlreadyExists/Precondition error collapsed into one Error::Chunk; deadline::check before any I/O at 415, 602, 770
- [`crates/foks-server/src/operations/management.rs:328`](../../crates/foks-server/src/operations/management.rs#L328): Prometheus export has no chunk_physical_usage fields; bin/foks-server.rs:742 status prints none
- [`crates/foks-server-db/src/config.rs:101`](../../crates/foks-server-db/src/config.rs#L101): defaults 128 attempts / 256 MiB
- [`crates/foks-server/README.md:180`](../../crates/foks-server/README.md#L180): [chunk_capacity] documented as capacity settings

Recommendation: Split settlement into three outcomes: success, definite non-publication (release the reserved bytes and resolve the attempt; use it when the failure happened before any request could reach the store: deadline::check failures, local validation, filesystem errors before persist_noclobber, and S3 definitive 4xx other than 408/429, which needs a distinct error variant from the S3 put arm), and ambiguous (current behaviour). Add a reconciliation step to inventory-chunk-store that, under the exclusive storage lease and after a complete listing, resolves outcome-2 attempts whose object is listed with the recorded length and, for the filesystem backend only, releases state-2 objects that are absent. Export unresolved_attempts, uncertain_bytes, admission_rejections and inventory_complete through the Prometheus endpoint and `status`, and log a warning on each ChunkAdmission rejection. State in the README that these budgets are consumed by faults and how to recover.

Verifier note: Both verifiers confirmed high, M; one reproduced the counters with an injected put failure and exact replays. Corrections applied: 'no recovery path' is overstated because the caps are operator-configurable, but nothing tells the operator the cap is being hit; S3 transport errors are retried inside object_store, so a single transient 5xx is less likely to consume an attempt than the reviewer's scenario implies; a graceful restart does not usually consume attempts because the listener runtime's shutdown_timeout lets in-flight handlers finish before the writer stops, so only handlers exceeding the request timeout or a crash leave outcome-1 rows; with the filesystem backend an exact replay that finds the object does move uncertain bytes to present, so uncertain_bytes accumulates only for genuine non-publication while unresolved_attempts accumulates in every case.

### c3-chunks-authority-mismatch-no-rebind

**The persisted chunk-store authority hashes the database file's canonical path, device and inode; after a restore from a filesystem copy, a move or a VACUUM INTO, serve, backup and every offline chunk command fail with 'chunk authority identity mismatch' and there is no rebind or downgrade**

Priority: high · Type: bug · Origin: celld · Effort: M · Area: chunks · Merged: `c-chunks-restore-relocation-no-rebind`, `c-backup-authority-identity-brittle`

authority_identity = SHA-256(canonical path, st_dev, st_ino) of the database file is persisted in chunk_storage_binding and embedded in the version-2 namespace marker. prepare_chunk_storage_binding_with_authority and StorageLease::verify_authority treat a mismatch as a hard error, and verify_authority runs on every path: serve startup (initialize_online recomputes and verifies on every start), the listener's verify_reader, hydrate_backup (so scheduled and manual backups fail), externalize, inventory, adopt and collect. S3 bindings are unaffected (no namespace lease, authority None). The authority exists to keep a stopped clone from collecting objects, and tests pin that an inode replacement of the live database is rejected at startup; but the most common recovery operations (restoring the database file from a filesystem snapshot or rsync copy, moving the installation directory, repointing a symlinked parent, replacing the file with a VACUUM INTO copy) change the inode or canonical path, and a device-number change across reboot is plausible on btrfs subvolumes, overlayfs, device-mapper and when NVMe enumeration changes. A reproducer copied the database to a new inode and renamed it into place: inventory-chunk-store failed with Invalid('chunk authority identity mismatch') and start_server failed the same way on every attempt; by reading, the backup path fails identically. The only remedies are manual SQL plus a rewrite of the marker file (verify compares the stored marker to marker(binding)), or discarding the store, whose objects then can never be collected because no database will bind to that marker again. The schema-53 migration already shows the intended fallback for a binding without authority: serve with collection unqualified. The README documents the device/inode check only as the writer-lock recheck, and the only operator guidance is 'there is no automatic reset or rebind'. The separate restore branch (a restored database starting against its previous store) is tracked in c3-backup-restore-into-bound-store-dead-end.

Evidence:

- [`crates/foks-server-db/src/connection.rs:110`](../../crates/foks-server-db/src/connection.rs#L110): authority_identity = SHA-256(canonical path, st_dev, st_ino) (110-130)
- [`crates/foks-server-db/src/chunk_binding.rs:146`](../../crates/foks-server-db/src/chunk_binding.rs#L146): stored authority != recomputed -> Invalid('chunk authority identity mismatch'); no update path
- [`crates/foks-server/src/chunk_binding.rs:17`](../../crates/foks-server/src/chunk_binding.rs#L17): marker embeds the authority (version byte 2); verify() calls verify_authority (42); initialize_online recomputes and verifies on every start (110-122)
- [`crates/foks-server/src/storage_lease.rs:34`](../../crates/foks-server/src/storage_lease.rs#L34): authority only for stores with a namespace lease (filesystem); verify_authority error 'object namespace belongs to another database identity'
- [`crates/foks-server/src/chunk_storage.rs:115`](../../crates/foks-server/src/chunk_storage.rs#L115): hydrate_backup -> verify_reader before hydration, so backups fail too
- [`crates/foks-server/src/chunk_binding.rs:377`](../../crates/foks-server/src/chunk_binding.rs#L377): tests pin clone and inode-replacement rejection (377-439)
- [`crates/foks-server/src/bin/foks-server.rs:1012`](../../crates/foks-server/src/bin/foks-server.rs#L1012): 'there is no automatic reset or rebind'
- [`crates/foks-server/README.md:420`](../../crates/foks-server/README.md#L420): device/inode documented only as a startup recheck; 679-681 'restore without the original store' with no next step

Recommendation: Treat an authority mismatch as a downgrade rather than a fatal error, as the schema-53 migration treats pre-authority bindings: in prepare_chunk_storage_binding_with_authority and chunk_binding::verify/initialize_online, when the stored authority differs, clear it (binding row, accepting the old marker bytes), log loudly, and serve and back up with collection unqualified; keep the hard refusal only for collect-chunk-store. Alternatively or additionally add an audited rebind-chunk-store command that, under the exclusive writer and storage leases, verifies the namespace marker against the binding, re-derives the authority and rewrites the marker. Let verify_reader on the backup path skip the authority check (backups only read). Document in the README's backup section and chunk-store section how the authority is derived and what changes it.

Verifier note: Both verifiers confirmed high, M; one reproduced the inode-replacement lockout for inventory and serve. The behaviour is deliberate clone protection, so the defect is the absence of any supported recovery or downgrade and of documentation, not a coding slip. The st_dev-across-reboot claim of the merged backup candidate was not reproduced and is recorded as plausible only; the routine triggers (copy, move, symlink repoint) need no such argument. Merge decision: the reviewers' restore-relocation candidate covered two branches; its restore branch is the same scenario as the backup reviewer's restore dead end and is tracked there (c3-backup-restore-into-bound-store-dead-end), because that branch has its own cause (a PendingNew row committed before the marker check) and its own S-sized fix, while this finding keeps the authority-mismatch branch and the merged authority-brittleness candidate.

### c3-session-bulk-pool-four-permits

**Four hard-coded bulk permits, held through the response write, gate every chunk transfer and every request with an argument over 4 KiB; one account slow-reading downloads refuses all other large requests with 1012 and a connection close**

Priority: high · Type: security · Origin: celld · Effort: S · Area: session · Merged: `c-session-bulk-pool-four-permits`, `c-security-bulk-pool-single-account-dos`, `c-compat-bulk-permit-cap-four-transfers`

celld's memory pools add a fifth pool, 'bulk', sized min(maximum_in_flight_requests - 1, 4): four permits with the default 64 in-flight requests, with no SessionLimits field to raise it (crates/foks-hosted-transport/src/memory.rs:87; memory/tests.rs:464 asserts (64, 4), so the cap is deliberate). admit_working takes one permit by try_acquire, never queued, for every request whose actual argument exceeds 4096 bytes and for every KvStoreGetEncryptedChunk (classified ChunkDownload), and a KEX receive whose matched packet exceeds 4 KiB needs one too. The permit lives in the Request's PermitSlots, and the Request is held as RequestOutcome::_memory until write_response_until returns, so a slow reader keeps a permit for up to min(remaining request deadline, io_timeout) = 15 s per round, and detached object-store work keeps it through retain_future for up to the chunk-store timeout (default 10 s, configurable 1-60 s). All three listeners share one ServerData, so anonymous public-listener requests and authenticated transfers draw on the same four permits. Refusal is answered RateLimited (1012) at the call sequence and then the connection is closed; it is counted as AdmissionRejection::RequestMemory, indistinguishable from working-memory exhaustion. Neither client retries: the Rust agent maps 1012 to ErrorCode::RateLimited with a 'retry after a short delay' message and the Go v0.1.9 libkv/libclient have no RateLimit handling. The pre-port server had no such bound; large requests were limited by the 64 execution permits and the 64 MiB input budget. The README documents only request_memory and no serve-level test exercises the refusal. Adversarial case: one authenticated account opens four connections (its per-source allowance is 32), requests a 9 MiB chunk on each and reads at a few KiB/s; every other user's chunk upload, KV file put, chunk download, team or chain post over 4 KiB and large KEX receive is answered 1012 and disconnected for as long as it repeats. Benign case: five desktop users downloading from an object-store-backed host collide on the four permits and see intermittent 1012 failures. The ChunkDownload working charge (8 x 9 MiB against a 496 MiB pool) would by itself bound concurrent downloads at about six, so the bulk cap is the tighter limit and removing it does not remove the memory bound.

Evidence:

- [`crates/foks-hosted-transport/src/memory.rs:87`](../../crates/foks-hosted-transport/src/memory.rs#L87): bulk = min(maximum_in_flight_requests - 1, 4); no configuration input (net/memory.rs passes only the four existing limits); memory/tests.rs:464 asserts (64, 4)
- [`crates/foks-hosted-transport/src/memory.rs:296`](../../crates/foks-hosted-transport/src/memory.rs#L296): admit_working: any argument over 4096 bytes or a ChunkDownload workload try_acquires one bulk permit, retained in PermitSlots
- [`crates/foks-server/src/net/session.rs:410`](../../crates/foks-server/src/net/session.rs#L410): KvStoreGetEncryptedChunk classified ChunkDownload; refusal counted RequestMemory, RateLimited at the call sequence, `return Ok(())` closes the connection (417-431)
- [`crates/foks-server/src/net/session.rs:650`](../../crates/foks-server/src/net/session.rs#L650): outcome._memory held through write_response_until; framing.rs:86-108 bounds the write by min(remaining deadline, io_timeout) = 15 s
- [`crates/foks-server/src/chunks.rs:620`](../../crates/foks-server/src/chunks.rs#L620): retain_future keeps the Request with the spawned object-store transfer; store timeout default 10 s, range 1-60 s (54-56, 84)
- [`crates/foks-server/src/net/session/kex.rs:144`](../../crates/foks-server/src/net/session/kex.rs#L144): a KEX receive admits the queued packet size, so a packet over 4 KiB needs a bulk permit
- [`crates/foks-server/src/net/listener.rs:216`](../../crates/foks-server/src/net/listener.rs#L216): one ServerData for the probe, public and authenticated listeners
- [`crates/foks-server/src/metrics.rs:41`](../../crates/foks-server/src/metrics.rs#L41): no bulk-specific AdmissionRejection reason
- [`crates/foks-agent/src/main.rs:2724`](../../crates/foks-agent/src/main.rs#L2724): 1012 mapped to ErrorCode::RateLimited; no automatic retry (Go v0.1.9 client/libkv has no RateLimit references)
- [`crates/foks-server/README.md:283`](../../crates/foks-server/README.md#L283): only the request_memory reason ('shared request working-memory budget') is documented; no bulk row in book/18-server-storage.qmd:215-226

Recommendation: Add a SessionLimits field (for example maximum_bulk_requests) defaulting to a fraction of maximum_in_flight_requests (half, at least 4), pass it through PoolLimits, and apply the class only to chunk upload and download workloads rather than every argument above 4 KiB (or raise the threshold to the 16 KiB most routes use). Count the refusal under a new AdmissionRejection reason, document the limit in the README capacity section and book/18-server-storage.qmd, and add a serve-level test that a fifth concurrent large request is refused while four are held and succeeds with a raised limit. Releasing the bulk permit when the handler returns, keeping only the working and output byte charges through the socket write, is a further M-sized change.

Verifier note: Both verifiers confirmed and kept high; one rated the effort S, the other M because it included releasing the permit at handler return. The synthesis takes S for the configurable limit, class restriction, metric, documentation and test, and lists the permit-release change separately as M. Corrections applied: the per-round hold for a slow reader is at most 15 s, not the 30 s the merged security candidate stated; a stalled object store holds the permit for at most the configured chunk-store timeout bounded by the request deadline; the class is keyed on the actual argument length, so routes listed only by their request ceiling (RegSignup, KexSend, provisioning) are affected only when a given request really exceeds 4 KiB, whereas chunk uploads, KV file puts and every chunk download are affected unconditionally; the Go client's connectionMgr behaviour after the close was not traced beyond the pending call failing with EOF. One verifier ran the pool unit tests (19 pass) and confirmed the cap is intended. Not a duplicate: no open finding mentions the bulk pool and it did not exist at 224a6c4.

### c3-session-input-budget-held-before-body

**Request input memory is reserved with a queued acquire after only the frame header arrives and held until the response is written, so a few header-only connections stall every request of the same size class on every listener**

Priority: high · Type: security · Origin: pre-existing · Effort: M · Area: session · Merged: `c-session-input-budget-slowloris`, `c-security-small-frame-body-budget-starvation`

read_frame_ingress returns after the length header; serve() then calls read_call_body_async, which performs a queued acquire_many_owned(2 x length) on an input semaphore before read_exact, so a peer that announces a frame and sends nothing holds its reservation for the whole body phase (min(remaining request deadline, io_timeout) = 15 s). Tokio's Semaphore is FIFO, so a large waiter at the head blocks every smaller waiter behind it; a legitimate request that cannot get permits waits until its own body phase expires and serve() then breaks out of the loop, closing the connection with no response. Nothing checks the announced length against a route ceiling before reserving, because routing happens after the body is read. celld split the budget by frame length: frames of at most 4096 bytes draw from small_request_memory, sized by maximum_control_memory_bytes (1 MiB by default; the field is documented only as capacity for control responses), and larger frames from the 64 MiB request_memory. Both pools live in the single ServerData shared by the probe, public and authenticated listeners. Two connections from one unauthenticated address announcing 16 MiB frames (the maximum frame length) reserve the whole 64 MiB large pool and stall every request over 4 KiB (uploads, large puts, team and chain posts) on every listener, reconnecting every 15 s. 128 announced-but-unsent 4096-byte frames (8192 permits each) exhaust the small pool, which almost every FOKS RPC uses (36 routes cap at 4096 bytes, 19 at 1024); the per-source connection cap of 32 is per listener and the probe and public listeners share the pool, so two sources (32 header-only connections on each listener) or four sources on the public listener alone suffice. The large-pool vector is pre-port behaviour: 224a6c4 reserved on a single 64 MiB pool with the same queued acquire, and that vector then blocked every request including small ones. celld's split isolates small requests from the two-connection vector and introduces the costlier small-pool vector. Neither vector is recorded in docs/review.

Evidence:

- [`crates/foks-server/src/net/session.rs:968`](../../crates/foks-server/src/net/session.rs#L968): read_call_body_async: acquire_many_owned(2 x length).await before read_exact (951-983); the permit moves into the Request and is held until the response is written
- [`crates/foks-server/src/net/session.rs:356`](../../crates/foks-server/src/net/session.rs#L356): body phase under phase_deadline(deadline, io_timeout); `Err(_) => break` at 371-373 closes without a response; pool chosen by length <= 4096 at 362-366
- [`crates/foks-server/src/net/session.rs:177`](../../crates/foks-server/src/net/session.rs#L177): request_memory 64 MiB; small_request_memory sized by maximum_control_memory_bytes
- [`crates/foks-server/src/config.rs:55`](../../crates/foks-server/src/config.rs#L55): maximum_control_memory_bytes documented for control responses only; default 1 MiB (79); validate() accepts 16 KiB (116)
- [`crates/foks-hosted-transport/src/framing.rs:5`](../../crates/foks-hosted-transport/src/framing.rs#L5): frame length accepted up to maximum_frame_bytes before any routing; phase_deadline at 86-93
- [`crates/foks-rpc/src/lib.rs:41`](../../crates/foks-rpc/src/lib.rs#L41): DEFAULT_MAX_FRAME_LENGTH = 16 MiB
- [`crates/foks-server/src/rate_limit.rs:44`](../../crates/foks-server/src/rate_limit.rs#L44): maximum_connections_per_source = 32, per listener (29), loopback exempt; connection_burst 512, 256/s
- [`crates/foks-server/src/net/listener.rs:216`](../../crates/foks-server/src/net/listener.rs#L216): one ServerData for all listeners; the probe listener (379) uses the same session path
- [`book/18-server-storage.qmd:221`](../../book/18-server-storage.qmd#L221): documents the split ('frames wait for permits') without the consequence

Recommendation: Stop queueing on input memory while the body is outstanding: either try_acquire and answer or close immediately on the public and probe listeners, or read the body incrementally and reserve as bytes arrive with a minimum-progress timeout that closes a connection whose body makes no progress for a short grace period. Add a per-source cap on in-flight bodies, or a per-source share of each input pool, using the existing KeyedLimit<SourceKey> (the KEX relay's one-sixteenth model). Size the small pool by frame count independently of maximum_control_memory_bytes and document both uses in config.rs, the README and the book. Add a listener-level test with scripted header-only peers asserting that a concurrent small request on another connection completes within the request deadline.

Verifier note: Both verifiers confirmed high, effort M. Both corrected the origin: the queued acquire-before-read is pre-port (224a6c4 session.rs:718), so the two-connection large-pool vector existed before celld and the reviewer's statement that the pre-port pool could not be exhausted within the connection cap is wrong; celld's split did not make the exposure worse overall but added the small-pool variant. One verifier also corrected the source count: the per-source cap is per listener and the probe and public listeners share the pool, so two sources suffice for the small pool rather than four. Severity high rests on the pre-existing large-pool vector (one unauthenticated address, two connections, every large request stalled). Origin is recorded as pre-existing for the mechanism, with the small-pool variant attributed to celld. Not a duplicate of server-source-admission-fairness, which covers connection slots, buckets, long polls and the KEX relay, not body-memory reservations.

### c3-chunks-failed-put-pins-digest

**A chunk whose first PUT genuinely failed can never be uploaded again: the digest stays pinned to an absent state-2 object and every retry takes the Verify path, which cannot republish**

Priority: high · Type: bug · Origin: celld · Effort: M · Area: chunks · Merged: `c-chunks-failed-put-pins-digest`

After put_object fails, settle_chunk_publication(false) leaves the chunk_objects row in state 2 holding the digest; the partial unique index covers states 1..3, so a later upload of the same bytes is answered ChunkReservationAction::Verify with attempt_id None. The Verify arm only calls get_object; the object was never stored, so the filesystem backend returns Error::Io (NotFound) and read_remote collapses an S3 NotFound into Error::Chunk, neither of which status::shared_status knows, so both become TransactionRetry (1014). There is no Publish fallback, settle requires an attempt_id, and collection considers only states 3 and 4, so nothing can republish or retire the row. Because the chunk nonce is derived from (file_id, offset, final) and the Rust resumable client replays its spooled canonical chunk bytes, every resume of that upload sends the identical digest and is answered 1014 forever: a reproducer injected one put failure, then replayed the identical encoded request three times after the store recovered and each returned 1014 with the fault store's put counter still at 1 and kv_usage.large.chunks still 0, while a fresh upload of the same plaintext under a new file seed succeeded. The Go v0.1.9 client allocates a new file ID per PutFile attempt and so recovers by re-uploading; the Rust client's upload is stuck for as long as the server database lives, reported as a transient condition. The only failed-PUT test retries via a new upload, and the exact-replay test covers only the lost-reply case where the object exists.

Evidence:

- [`crates/foks-server-db/src/chunk_ledger.rs:254`](../../crates/foks-server-db/src/chunk_ledger.rs#L254): an existing row in state 1|2|3 never gets a new incarnation; state 1|2 -> Verify with attempt_id None (261-276); settle requires attempt_id (316-331); state=2 on failure (332)
- [`crates/foks-server-db/src/schema/chunk_incarnations.rs:47`](../../crates/foks-server-db/src/schema/chunk_incarnations.rs#L47): partial unique index on digest covers states 1..3
- [`crates/foks-server/src/services/kv.rs:497`](../../crates/foks-server/src/services/kv.rs#L497): Verify: get_object error returned through map_upload_error; no Publish fallback
- [`crates/foks-server/src/chunks.rs:462`](../../crates/foks-server/src/chunks.rs#L462): filesystem get_object: open error propagates as Error::Io; read_remote (657-681) collapses NotFound and transport errors into Error::Chunk
- [`crates/foks-server/src/status.rs:39`](../../crates/foks-server/src/status.rs#L39): Io and Chunk are not shared statuses, so both map to TransactionRetry via internal
- [`crates/foks-crypto/src/kv.rs:353`](../../crates/foks-crypto/src/kv.rs#L353): chunk nonce derived from (file_id, offset, final): same seed and plaintext give the same ciphertext and digest
- [`crates/foks-client/src/kv/uploads.rs:743`](../../crates/foks-client/src/kv/uploads.rs#L743): resume_file_upload replays the spooled canonical chunk bytes
- [`crates/foks-server-testkit/tests/external_chunks.rs:334`](../../crates/foks-server-testkit/tests/external_chunks.rs#L334): the only failed-PUT test retries via a new upload (334-380); exact replay is tested only for the lost-reply case (392-510)

Recommendation: Make FilesystemChunks::get_object and read_remote return a distinct 'object absent' error. In put_file_chunk's Verify arm, on definite absence create a new attempt row for the existing object_id/object_key (a ledger function such as reserve_chunk_republication inserting an attempt with outcome=1 and moving the object to state 1) and run the Publish arm; republishing identical bytes under the same key is idempotent for both backends (persist_noclobber then verify; PutMode::Create then verify). Report a definite absence that cannot be repaired as a non-retryable status. Add a testkit test that injects one put failure, then replays the exact encoded request as exact_upload_retry does and asserts success with two puts.

Verifier note: Both verifiers confirmed and agreed on M; one rated medium because data is not lost and Go clients and fresh Rust uploads succeed with a new digest, the other high because for the Rust resumable client the condition is permanent, triggered by any single store-side failure, and reported as transient. The synthesis adopts high: the resumable desktop client is the product's own upload path, and the reproducer showed that no number of retries can complete the upload while the user is told to retry. Correction applied: the dead end requires an exact replay after a genuine failure, not a lost reply.

### c3-uploads-stuck-submission-unknown

**A publication whose kvPut request never reached the server leaves the upload permanently un-resumable and un-cancellable, pinning one of 16 pending slots and its spool bytes**

Priority: high · Type: bug · Origin: celld · Effort: M · Area: uploads · Merged: `c-uploads-stuck-submission-unknown`

After ContentComplete the client prepares one KvNamespace child (the upload moves to state 4, Publishing, with namespace_child set) and sends kvPut. On a transport error mutate_namespace marks the child SubmissionUnknown, attempts one reconcile whose result is discarded, and returns. If the server never applied the request (socket reset before the server durably accepted it, or the server lost its queued job), nothing ever produces a definite outcome: resume_file_upload with namespace_child set routes unconditionally to resume_namespace_mutation; for SubmissionUnknown that only calls reconcile_namespace_mutation, which returns TransitionNotObserved when the dirent is absent and never calls rejected() (absence is correctly not treated as rejection, since the server might still run a queued job); cancel_upload refuses unless the child is Rejected ('upload publication must be reconciled before cancellation'). No other code path rejects a KvNamespace child (the other rejected() callers are bot_token, account_conveniences, device and invitations), upload maintenance only finalizes RemoteVerified children, CLEANUP_FIRST selects states 5, 6, 9 and 11 only, and upload_cleanup_debt counts the upload as incomplete but not actionable. admit() counts every non-terminal upload against max_pending (16) and sums reserved_bytes over all rows, so the slot and the spool bytes (up to 256 MiB by default) stay pinned; after a few such events, or one near the byte cap, every new large upload fails with UploadPendingFull or UploadSpoolFull and `retention maintain` reports incomplete with actionable 0. If the server applies the request later, a Resume observes the dirent and the child becomes RemoteVerified, so the trigger is a lost request, not merely a lost reply. Before celld a SubmissionUnknown put only left a journal row; celld adds the pending-slot and spool-byte pin. No testkit fault drops a request before the writer (the three faults are AfterDurableCommitBeforeResponse and BetweenLargeFileChunks), so the case is untested.

Evidence:

- [`crates/foks-client/src/kv/write.rs:774`](../../crates/foks-client/src/kv/write.rs#L774): mutate_namespace: transport error -> submission_unknown and return; reconcile result discarded (752-787)
- [`crates/foks-client/src/kv/write.rs:845`](../../crates/foks-client/src/kv/write.rs#L845): resume_namespace_mutation: SubmissionUnknown only reconciles (826-861); reconcile returns TransitionNotObserved on absence and never calls rejected() (1052-1076)
- [`crates/foks-client/src/kv/uploads.rs:710`](../../crates/foks-client/src/kv/uploads.rs#L710): resume_file_upload with namespace_child set always routes to resume_namespace_mutation
- [`crates/foks-client-db/src/repositories/uploads.rs:452`](../../crates/foks-client-db/src/repositories/uploads.rs#L452): cancel_upload refuses while namespace_child is set and the child is not state 5 (438-462); record_upload_namespace sets state 4 (491-573); retry_upload_namespace requires state 5 (574-577)
- [`crates/foks-client-db/src/repositories/uploads.rs:245`](../../crates/foks-client-db/src/repositories/uploads.rs#L245): admit counts state 4 as pending and sums reserved_bytes over all rows
- [`crates/foks-client-db/src/repositories/uploads/receipts.rs:25`](../../crates/foks-client-db/src/repositories/uploads/receipts.rs#L25): CLEANUP_FIRST selects states 5,6,9,11 only; actionable excludes state 4 (495-509)
- [`crates/foks-client-app/src/upload_maintenance.rs:143`](../../crates/foks-client-app/src/upload_maintenance.rs#L143): only RemoteVerified and terminal children are handled (143-156, 308-335)
- [`crates/foks-server-testkit/src/environment.rs:558`](../../crates/foks-server-testkit/src/environment.rs#L558): fault catalogue has no drop-before-writer fault

Recommendation: Give SubmissionUnknown a terminating proof in reconcile_namespace_mutation for upload children: the kvPut carries the path version vector, so when the refreshed authenticated projection shows a cited directory version strictly past the precondition and the prepared dirent is absent, the request can no longer apply; mark the child Rejected, after which resume_file_upload can call retry_upload_namespace (see c3-uploads-rejected-child-dead-publishing) and republish. Allow Cancel on a Publishing upload whose child is SubmissionUnknown by running that proof first and, if the directory has not moved, bumping it with a no-op precondition write before cancelling. Add a testkit fault point that drops the kvPut request before the server writer admits it, and a test for both paths.

Verifier note: Both verifiers confirmed and agreed on M; one rated medium because the trigger needs the request to be lost entirely rather than only its reply, the other high. The synthesis adopts high: when a connection resets during kvPut the server either received the request (Resume recovers) or did not (this dead state), so the trigger is an ordinary network event on the path the feature exists for, and the consequence is permanent, has no remedy at all (not even Cancel), and consumes capacity shared by every upload in the profile. Correction applied: the trigger is 'request never applied', not 'reply lost'.

## Medium-priority findings

10 findings.

### c3-deadlines-expired-response-never-written

**The agent's DeadlineExceeded responses are never delivered: write_controlled_response refuses to write once the request control has expired, so every operation that reaches its deadline ends as a bare connection close that clients report as 'agent disconnected'**

Priority: medium · Type: bug · Origin: celld · Effort: S · Area: deadlines · Merged: `c-deadlines-expired-response-never-written`

Every terminal deadline response in the agent is built only after tokio::time::timeout_at(control.deadline.instant()) or sleep_until(deadline) fired, and is then handed to write_controlled_response(&control), which calls control.check() before encoding and again before write_all. Deadline::check returns Expired when Instant::now() >= the same instant the timer fired on, so the first check fails deterministically, nothing is written, the Err propagates out of handle_connection and the socket is dropped. Affected: the supervise_blocking_with_control timeout arm ('connection closed because completion is ambiguous', written at main.rs:1463), the post-completion worker_control.check() that replaces a definite result with DeadlineExceeded (main.rs:1995-1998), the chat poll ('chat poll operation deadline exceeded'), and the streaming upload ('streaming upload outcome is ambiguous; refresh the store before another mutation'); ReconcileProfile and RefreshLease do not even build a response and return Err from handle_connection on timeout_at. Pre-port the agent wrote these responses with write_response(timeout) under the agent-wide write budget. The client's deadline is strictly later than the agent's (Deadline::receive caps at min(rest, cap) after transit), and read_frame_cancellable maps Ok(0) to Io(UnexpectedEof) 'agent disconnected' before any deadline check, so the client sees EOF first: a reproducer with the HEAD binary (--request-timeout-seconds 2, a never-answering server, a CheckAndAddProfile) printed 'local agent mutation outcome is ambiguous: local agent I/O failed: agent disconnected' after 2.01 s while the agent log recorded 'connection failed: FOKS operation deadline exceeded'. The desktop maps UnexpectedEof to connection_lost ('agent-lost', fatal) and fires its connection-loss notifier although the agent is healthy; a slow PutKv ends as Ambiguous(Io) rather than a typed DeadlineExceeded with the refresh advice. The existing test expired_rejection_does_not_start_a_fresh_response_write_budget pins the refusal for an admission-not-started rejection, where it is correct; no test covers a dispatched operation.

Evidence:

- [`crates/foks-agent/src/main.rs:2109`](../../crates/foks-agent/src/main.rs#L2109): write_controlled_response: control.check()? before encode and before write_all (2106-2125)
- [`crates/foks-agent/src/main.rs:2065`](../../crates/foks-agent/src/main.rs#L2065): supervise_blocking_with_control builds the DeadlineExceeded response only after timeout_at(control.deadline.instant()) fired (2061-2080); written with the same expired control at 1463
- [`crates/foks-agent/src/main.rs:1997`](../../crates/foks-agent/src/main.rs#L1997): worker_control.check() after completion replaces a definite result with a response that cannot be written either
- [`crates/foks-agent/src/chat_poll.rs:168`](../../crates/foks-agent/src/chat_poll.rs#L168): same pattern after sleep_until(deadline); write at 191
- [`crates/foks-agent/src/main.rs:1811`](../../crates/foks-agent/src/main.rs#L1811): streaming upload ambiguity response built after control.remaining() timed out, written at 1822 with the expired control
- [`crates/foks-agent/src/main.rs:1255`](../../crates/foks-agent/src/main.rs#L1255): ReconcileProfile (1255) and RefreshLease (1278) return Err(DeadlineExceeded) from handle_connection; 224a6c4 main.rs:1156-1166 wrote a response with write_response(timeout)
- [`crates/foks-local-deadline/src/clock.rs:233`](../../crates/foks-local-deadline/src/clock.rs#L233): check_with returns Expired when the clock instant >= the deadline instant
- [`crates/foks-agent-client/src/lib.rs:556`](../../crates/foks-agent-client/src/lib.rs#L556): read_frame_cancellable maps Ok(0) to Io(UnexpectedEof) 'agent disconnected'; is_connection_loss includes it (61-73)
- [`crates/foks-desktop/src/lib.rs:377`](../../crates/foks-desktop/src/lib.rs#L377): agent_client_error sets connection_lost for UnexpectedEof; apps/desktop transport maps it to 'agent-lost', fatal
- [`crates/foks-agent/src/main.rs:10801`](../../crates/foks-agent/src/main.rs#L10801): test asserts the refusal for an admission-not-started rejection only

Recommendation: Add a terminal write that does not consult the expired request control: reuse the existing write_response(stream, &response, timeout) with a fixed 1-2 s grace (or control.deadline.instant() + grace) for the supervise timeout arm, the post-completion replacement, the chat-poll close and the streaming-upload DeadlineExceeded arm, and for the ReconcileProfile/RefreshLease timeout branches build a DeadlineExceeded response and write it the same way instead of returning Err. Keep write_controlled_response for pre-expiry replies (the ADMISSION_REPLY_MARGIN case). Add a handle_connection test over a UnixStream pair that drives a sleeping operation past a short --request-timeout and asserts the client reads a DeadlineExceeded frame before EOF.

Verifier note: Both verifiers confirmed medium, S; one reproduced it end to end with the HEAD binary. Refinements: the agent does log the event as a connection failure, so only the client side is wrong; whether the client reports EOF or its own DeadlineExceeded is a race the client loses by the sampling and transit delta, so EOF-first is the common case rather than guaranteed, and either way the explanatory texts are never delivered.

### c3-uploads-implicit-identity-before-size

**Every KV file put claims a resumable upload identity before the size is known, so each small inline write holds one of 8192 receipt rows for 30 days and every save depends on the receipt clock trusting the system clock**

Priority: medium · Type: bug · Origin: celld · Effort: S · Area: uploads · Merged: `c-uploads-implicit-identity-receipt-budget`, `c-uploads-clock-gates-all-puts`

put_file_with_size_bound calls ensure_file_upload_claim before the first source read, regardless of size; for every session without an explicit identity it issues a fresh UploadHandle, samples the receipt clock, calls upload_issuance_context, and registers the intent, which runs receipt_admit and inserts a kv_upload_handles row with receipt_bytes=1024 whose AFTER INSERT trigger increments reserved_rows. Two consequences follow. (1) Receipt budget: a small file finishes as InlineUntracked (state 11) but keeps the handle row; terminal_at/expires_at = floor + 30 days are stamped later and PRUNE_CANDIDATES deletes only rows past expires_at (128 per maintenance turn), so each inline write holds one of max_rows = 8192 for 30 days. The budget is a per-profile singleton shared by every account and team, set_upload_retention_limits has no caller outside tests, and every desktop PutKv (item saves, imports), adapter write and MCP data write reaches this path. A bulk import of 9,000 items or an agent writing about 300 small files a day hits UploadRetentionFull on the 8,193rd write within 30 days, after which every file write in that profile, large or small, fails (desktop code 'upload-retention-full', not retryable) until the oldest receipts age out. (2) Clock dependency: upload_issuance_context fails with UploadClockUntrusted when the receipt clock's validation rejects the sample. Within one process the anchor is never re-taken, so a wall-clock step of more than 300 s in either direction while the agent runs (a user correcting a fast clock, a VM step) makes every file put fail for the rest of the process lifetime; after a restart a forward step under 24 h reanchors immediately, a backward step of S seconds is refused for about S - 300 s of real time, and larger steps need the manual `foks retention reanchor --domain upload` flow, for which the desktop has no action (the agent's error text names the CLI command). Reads, mkdir, symlink and remove are not gated, so the failure looks like a save-specific bug. Pre-port puts had neither an upload claim nor a clock dependency. Each inline write also leaves a four-phase cleanup ledger row that maintenance must walk, including a temporary-directory sweep.

Evidence:

- [`crates/foks-client/src/kv/write.rs:292`](../../crates/foks-client/src/kv/write.rs#L292): put_file_with_size_bound claims before the first read; inline branch (322-331) keeps the handle row
- [`crates/foks-client/src/kv/uploads.rs:454`](../../crates/foks-client/src/kv/uploads.rs#L454): ensure_file_upload_claim -> register_upload_intent for every session without an explicit identity; samples the clock and calls upload_issuance_context at 487-492
- [`crates/foks-client-db/src/repositories/uploads/receipts.rs:318`](../../crates/foks-client-db/src/repositories/uploads/receipts.rs#L318): register_upload_intent: receipt_admit (186-192) then INSERT handle with RECEIPT_BYTES; PRUNE_CANDIDATES requires expires_at <= floor (35); prune limited to 128 rows (723-733); set_upload_retention_limits has no production caller (375-384)
- [`crates/foks-client-db/src/repositories/uploads/schema42.sql:8`](../../crates/foks-client-db/src/repositories/uploads/schema42.sql#L8): max_rows defaults to 8192 (hard cap 65536); reserve/release triggers at 23-26
- [`crates/foks-client-db/src/repositories/uploads/receipts.rs:216`](../../crates/foks-client-db/src/repositories/uploads/receipts.rs#L216): upload_issuance_context fails with UploadClockUntrusted when validate rejects the sample
- [`crates/foks-client-db/src/repositories/receipt_clock.rs:54`](../../crates/foks-client-db/src/repositories/receipt_clock.rs#L54): check_elapsed with SKEW 300 s; restart rules at 80-87 (24 h forward gap, floor rule backward)
- [`crates/foks-client/src/receipt_clock.rs:30`](../../crates/foks-client/src/receipt_clock.rs#L30): one process id per process; CLOCK_BOOTTIME monotonic; anchor never re-taken in-process
- [`crates/foks-client-app/src/kv.rs:441`](../../crates/foks-client-app/src/kv.rs#L441): desktop and agent PutKv path binds the adapter clock sample and calls put_file_with_size_bound (427-461)
- [`crates/foks-agent/src/main.rs:2497`](../../crates/foks-agent/src/main.rs#L2497): UploadClockUntrusted error text points at the CLI reanchor command; no desktop reanchor action

Recommendation: Claim the upload identity only after read_kv_upload_chunk shows node_type == File (the first 4 MiB are buffered already and explicit-handle sessions have upload_identity set), or delete the kv_upload_handles, kv_uploads and kv_upload_cleanup rows in finish_inline_upload_identity when the identity was implicit, which releases the reservation through the DELETE trigger. That same change removes the clock check from inline writes. For large files, treat an untrusted sample on an implicit identity as 'no resumable identity' (single-pass upload) instead of failing the put, surface the condition once with the repair action, and re-anchor the in-process clock on a failed check_elapsed when the new sample is consistent with the stored floor so a corrected clock recovers without a restart. Add a test asserting reserved_rows is unchanged after a small put.

Verifier note: All four verdicts (two per merged candidate) confirmed medium, S. The synthesis merges the two candidates: both verifiers of the clock candidate stated that the dependency is a consequence of the implicit identity and that the same change removes it for inline writes; the residual for genuinely resumable uploads (degrade rather than refuse, in-process reanchor) is kept in the recommendation. Corrections applied: 'restarting does not help' is overstated (forward steps recover on restart, backward steps after S - 300 s); the agent does return an actionable message; the 9,000-item import rate is plausible but unmeasured, while the per-write accounting is exact. Not a duplicate of gap-mcp-adapter-clock-suspend, which concerns MCP data writes on the adapter clock; celld extended the same gating to every KV file put.

### c3-chunks-inventory-gates-admission

**Starting the report-only inventory clears inventory_complete, three conditions make a run impossible to finish, and a fresh installation whose chunk directory is a mount root never confirms the empty inventory; in every case chunk uploads are rejected with 1012 and nothing names the cause**

Priority: medium · Type: bug · Origin: celld · Effort: M · Area: chunks · Merged: `c-chunks-inventory-gates-admission`

inventory-chunk-store is presented as accounting that 'never deletes', but begin_chunk_inventory clears inventory_complete when it starts or restarts a run, and reserve rejects every new digest with 'chunk inventory is incomplete' until a run finishes. Three conditions make completion impossible without changing data by hand, and none can be skipped or acknowledged: a referenced object missing from the store (the final page fails with 'inventory is missing referenced objects', the page transaction including the cursor rolls back, so a resumed run refetches the same last page and fails identically, and inventory_running stays 1); a tracked key whose listed size differs from the ledger; and, for the filesystem backend, any non-regular entry in the directory. Separately, FilesystemChunks::empty() treats any entry other than the marker and lock as an object, so a brand-new installation whose chunk directory is a filesystem mount root (which contains lost+found) never confirms the empty inventory: this was reproduced on a fresh test environment, where startup succeeds silently, the binding is Active with authority None (the non-empty namespace also makes collection permanently unqualified for this installation, see c3-chunks-authority-mismatch-no-rebind), small inline writes succeed, an 8 KiB file upload fails with 1012, and inventory-chunk-store then fails with 'inventory encountered nonregular object'. Nothing is logged at startup when the ledger serves with admission closed, inventory_complete is not exported, and the README does not mention the inventory command or that an incomplete inventory blocks uploads.

Evidence:

- [`crates/foks-server-db/src/chunk_ledger.rs:392`](../../crates/foks-server-db/src/chunk_ledger.rs#L392): begin_chunk_inventory clears inventory_complete when starting or restarting a run (not when resuming)
- [`crates/foks-server-db/src/chunk_ledger.rs:251`](../../crates/foks-server-db/src/chunk_ledger.rs#L251): reserve rejects 'chunk inventory is incomplete' until a run finishes
- [`crates/foks-server-db/src/chunk_ledger.rs:452`](../../crates/foks-server-db/src/chunk_ledger.rs#L452): final page fails with 'inventory is missing referenced objects' and rolls back the cursor; length mismatch aborts at 432-434; no override
- [`crates/foks-server/src/chunks/inventory.rs:51`](../../crates/foks-server/src/chunks/inventory.rs#L51): any non-regular entry aborts the filesystem listing; empty() (19-29) returns false for any entry other than marker and lock
- [`crates/foks-server/src/chunk_ledger.rs:7`](../../crates/foks-server/src/chunk_ledger.rs#L7): initialize_online confirms the empty inventory only when the listing is empty; no log otherwise
- [`crates/foks-server/src/chunk_binding.rs:111`](../../crates/foks-server/src/chunk_binding.rs#L111): a non-empty namespace at first bind yields authority None
- [`crates/foks-server/src/bin/foks-server.rs:1038`](../../crates/foks-server/src/bin/foks-server.rs#L1038): command banner describes inventory as accounting that never deletes; README.md has no occurrence of 'inventory'

Recommendation: Track completion per generation and keep the previous inventory_complete value until the new generation finishes, or give inventory-chunk-store an explicit --acknowledge-missing-referenced flag that records missing_referenced_objects and completes anyway. In FilesystemChunks::empty() and inventory(), skip directory entries (the backend never creates one; keys contain no path separators) with a logged warning, keeping the error for symlinks and other non-regular leaves. Log at startup when the ledger serves with inventory_complete=0 and export inventory_complete and admission_rejections (shared with c3-chunks-failed-attempts-exhaust-admission). State in the command banner and README that an incomplete inventory blocks chunk uploads.

Verifier note: Both verifiers confirmed medium, M; one reproduced the lost+found case end to end. Corrections applied: begin_chunk_inventory clears the flag only on start or restart, not on resume, so the title's 'starting' applies to a first run or --restart on a serving installation; the same non-empty check also leaves the binding without an authority, so such an installation is also permanently collection-unqualified.

### c3-backup-hydration-exceeds-database-limit

**Backup hydration re-inlines every external chunk into a SQLite copy capped by the live maximum_database_bytes, so an installation whose external data exceeds about half its database limit can never complete a backup, and the failure is reported as disk_full**

Priority: medium · Type: bug · Origin: celld · Effort: M · Area: backup · Merged: `c-backup-hydration-database-limit`

External chunk storage exists so that SQLite holds metadata while encrypted chunks live elsewhere, up to maximum_chunk_physical_bytes. hydrate_backup undoes that: it copies every object into kv_file_chunks of the private backup copy as ciphertext plus exact_upload_chunk (the exact encoding contains the ciphertext, so each hydrated row is about twice the object size), and opens that copy with foks_server_db::Database::open(path, config) using the live database_config, whose configure() sets max_page_count from maximum_database_bytes. Nothing validates maximum_database_bytes against maximum_chunk_physical_bytes (both default to 64 GiB independently), so with defaults a backup fails permanently once metadata plus roughly twice the external ciphertext exceeds 64 GiB, about 30 GiB of external objects, well under the 64 GiB physical cap. The first UPDATE past the ceiling fails with SQLITE_FULL (verified with a page-count experiment), which database_reason maps to Reason::DiskFull, so the stderr record reads 'phase=backup_create, reason=disk_full' while the disk is fine; the scheduled backup then fails on every interval, foks_last_backup_success_unixtime stops advancing, and the only remedies are raising the live database limit or shrinking data. Restore has the matching bound: copy_backup_database_file rejects a backup larger than maximum_database_bytes. Logical quota accounting is unaffected (the triggers use coalesce(external_*, length(...))). The README's single sentence, 'Hydration requires temporary disk and database capacity', does not say backups become impossible past the limit.

Evidence:

- [`crates/foks-server/src/chunk_storage.rs:126`](../../crates/foks-server/src/chunk_storage.rs#L126): hydrate_backup opens the private copy with Database::open(path, config) using the live database_config (standalone.rs:587 passes database_config.clone())
- [`crates/foks-server-db/src/connection.rs:541`](../../crates/foks-server-db/src/connection.rs#L541): configure() sets max_page_count from maximum_database_bytes; opening a file already over the limit fails QuotaExceeded
- [`crates/foks-server-db/src/kv_objects.rs:263`](../../crates/foks-server-db/src/kv_objects.rs#L263): hydrate_kv_chunk stores ciphertext and exact_upload_chunk inline
- [`crates/foks-server-db/src/config.rs:96`](../../crates/foks-server-db/src/config.rs#L96): maximum_database_bytes 64 GiB and maximum_chunk_physical_bytes 64 GiB (99) set independently; installation.rs:272-290 validates non-zero only
- [`crates/foks-server/src/diagnostics/operational.rs:91`](../../crates/foks-server/src/diagnostics/operational.rs#L91): SQLITE_FULL -> Reason::DiskFull
- [`crates/foks-server/src/standalone.rs:413`](../../crates/foks-server/src/standalone.rs#L413): restore rejects a backup database or WAL larger than maximum_database_bytes
- [`crates/foks-server/README.md:682`](../../crates/foks-server/README.md#L682): 'Hydration requires temporary disk and database capacity' is the only mention

Recommendation: Open the hydration copy with a config whose maximum_database_bytes is raised to maximum_database_bytes + 2 x maximum_chunk_physical_bytes plus slack (or a dedicated open that skips max_page_count for the backup copy and relies on a free-space check against the backup directory), and give copy_backup_database_file on restore the same allowance so such backups restore. Map the private copy's SQLITE_FULL to a capacity reason or a dedicated phase rather than disk_full. Validate the two limits at config load or document the arithmetic in README:680-684. The longer-term alternative, keeping hydrated objects as a content-addressed sidecar directory inside the backup under the manifest-last publication and re-publishing them on restore, is L.

Verifier note: Both verifiers confirmed medium, M, by tracing the open path and the row layout; one verified the SQLITE_FULL mapping with a small SQLite experiment. The whole hydration path is new in celld and nothing in docs/review records it. Not executed at scale.

### c3-backup-restore-into-bound-store-dead-end

**Restoring a backup and starting against the installation's previous object store fails at the first start and leaves a PendingNew binding that no later configuration can satisfy**

Priority: medium · Type: bug · Origin: celld · Effort: S · Area: backup · Merged: `c-backup-restore-into-bound-store-dead-end`

The hosted recovery case celld targets is 'SQLite volume lost, object store survived'. restore_backup deliberately produces an inline-only database with no binding and no ledger. When the operator restarts with the unchanged server.toml (same bucket/prefix or directory), initialize_online sees a non-empty namespace, commits a PendingNew binding pinned to that locator with a fresh random namespace and authority None inside writer.call before any marker I/O, and then create_namespace_marker_if_absent finds the old installation's marker and fails with 'storage namespace marker conflict'. Because the row is committed first and prepare_chunk_storage_binding_with_authority refuses a different locator for any non-Legacy binding, the restored database is now pinned. A reproducer ran the sequence: start 1 against the old store fails with the marker conflict and leaves PendingNew (locator = old store, authority None); start 2 with a fresh empty directory fails Invalid('chunk storage binding mismatch'); start 3 back on the old store fails on the marker again; start 4 with no [chunk_store] at all fails 'bound external storage requires its configured backend' because a binding row exists. After one wrong first start the restored database cannot start under any configuration and must be deleted and restored again, and the operator has to guess that a new empty prefix or directory is required. The surviving objects are useless (the backup is self-contained) and become permanent orphans with no collector for S3. The testkit restore tests remove the chunk directory first or restore into a fresh environment, so the sequence is untested, and the README says only that backups 'restore without the original store'.

Evidence:

- [`crates/foks-server/src/standalone.rs:220`](../../crates/foks-server/src/standalone.rs#L220): the validated backup copy clears the binding and ledger
- [`crates/foks-server/src/chunk_binding.rs:111`](../../crates/foks-server/src/chunk_binding.rs#L111): needs_empty && !namespace_is_empty -> requested authority None; the PendingNew row is committed by prepare before the marker write at 133
- [`crates/foks-server/src/chunks.rs:401`](../../crates/foks-server/src/chunks.rs#L401): filesystem marker conflict; S3 equivalent at 752-765
- [`crates/foks-server-db/src/chunk_binding.rs:151`](../../crates/foks-server-db/src/chunk_binding.rs#L151): locator mismatch for any non-Legacy binding; no path discards a PendingNew row
- [`crates/foks-server/src/chunk_binding.rs:97`](../../crates/foks-server/src/chunk_binding.rs#L97): no store configured but a binding present -> 'bound external storage requires its configured backend'
- [`crates/foks-server/src/chunk_binding.rs:310`](../../crates/foks-server/src/chunk_binding.rs#L310): pending_binding_survives_lost_marker_reply pins that a PendingNew row is resumed
- [`crates/foks-server-testkit/tests/external_chunks.rs:318`](../../crates/foks-server-testkit/tests/external_chunks.rs#L318): restore tests remove the chunk directory first (320) or restore into a fresh environment
- [`crates/foks-server/README.md:679`](../../crates/foks-server/README.md#L679): 'restore without the original store' with no required next step

Recommendation: In chunk_binding::initialize_online and initialize_offline, when the database has no binding, read the store's namespace marker before calling prepare_chunk_storage_binding_with_authority: if a marker exists, fail with a message naming the remedy (configure a new empty directory or prefix) without writing the row. Additionally let prepare replace a PendingNew binding whose verified_objects is 0 and whose locator differs, or whose marker is absent or foreign, since nothing can have used that namespace. Have `restore` print that the restored installation must start against a new empty store namespace. Add a testkit test for restore, start against the previous store, then re-point. Document in the README restore section that the previous namespace's objects are permanent orphans.

Verifier note: Both verifiers confirmed medium, S; one reproduced all four starts and showed the dead end is worse than claimed (no configuration at all can start the database). Minor correction: with a store holding only the marker and no objects, empty() returns true and the row gets an authority, but the marker conflict is the same. Kept as a separate finding from c3-chunks-authority-mismatch-no-rebind (see its verifier note); together the two findings cover both recovery operations that leave a server unable to start with its existing [chunk_store] configuration.

### c3-backup-restore-stale-sidecars

**restore_backup checks only that the database leaf is absent; a stale -wal or -shm at the destination is replayed into the restored database on first open, silently**

Priority: medium · Type: bug · Origin: pre-existing · Effort: S · Area: backup · Merged: `c-backup-restore-stale-sidecars`

SQLite validates a -wal file's header checksum, page size and salts against the WAL itself, not against the main database it was written for, so a WAL with a valid header is replayed into whatever main file sits next to it. restore_backup refuses an existing foks-server.sqlite and a non-empty key directory but not an existing -wal, -shm, -journal, .storage-lock or writer lock at the destination; copy_new_file writes only the main file, whose header still declares WAL mode, so the first Database::open replays the stale frames. Both verifiers reproduced it: a -wal copied from database A placed next to an unrelated WAL-mode main file B was replayed on B's first open, B's own table disappeared, A's tables appeared, and PRAGMA integrity_check returned ok, so the corruption is silent. The scenario is the natural response to 'restore destination is not empty' after a failed integrity check: the operator stops the server, deletes only foks-server.sqlite and restores into the same path; the stale WAL is present precisely because the stop was unclean. The documented procedure (new explicit paths) and the recovery harness (remove_dir_all of the database directory) avoid it only if followed. The check is unchanged from 224a6c4, so this is pre-existing; it is reported because celld made restore the documented lost-volume recovery route and it is not recorded in docs/review.

Evidence:

- [`crates/foks-server/src/standalone.rs:118`](../../crates/foks-server/src/standalone.rs#L118): destination check is database_path.exists() || directory_has_entries(key_directory); sidecars not checked; same at 224a6c4:118
- [`crates/foks-server/src/standalone.rs:138`](../../crates/foks-server/src/standalone.rs#L138): copy_new_file copies only the main file; header still declares WAL mode
- [`crates/foks-server-testkit/tests/recovery_harness/volume.rs:100`](../../crates/foks-server-testkit/tests/recovery_harness/volume.rs#L100): the harness sidesteps the problem with remove_dir_all of the database directory
- [`crates/foks-server/README.md:755`](../../crates/foks-server/README.md#L755): documented procedure places the database and key directory at new explicit paths

Recommendation: In restore_backup, before the emptiness check, refuse when any of <database>-wal, <database>-shm, <database>-journal, <database>.storage-lock or the writer-guard lock file exists at the destination, naming the offending file in the error ('restore destination has SQLite sidecars; remove them or choose a new path'). Add a unit test that creates only foks-server.sqlite-wal at the destination and asserts the refusal with nothing written.

Verifier note: Both verifiers confirmed and reproduced the WAL replay; one kept low because the scenario requires both an unclean stop and a deviation from the documented procedure, the other raised to medium because the result is a healthy-looking restored database with the wrong pages and integrity_check passes. The synthesis adopts medium: silent corruption on the recovery path is the worst outcome a restore can have, the deviation is the natural reading of the error, the unclean stop is the usual reason to restore, and the fix is S. Origin pre-existing.

### c3-desktop-ordinary-cap-catalog-walk

**A single 15 s Ordinary budget now covers the whole serial catalog walk (list_catalog, list_catalog_progressive, list_profile_catalog) and the catalog repair inside Ordinary mutations, so a vault with many stores on a slow server never finishes loading**

Priority: medium · Type: perf · Origin: celld · Effort: S · Area: desktop · Merged: `c-desktop-ordinary-cap-whole-catalog-walk`

CommandState::admitted calls begin_operation, which captures Instant::now() + class.cap() once, and every transport the command uses is bound to it; list_catalog, list_catalog_progressive and list_profile_catalog are Ordinary (15 s). load_catalog runs ListProfiles, then per profile ListKnownStores/ListProfileOverview, then load_store_pages for each store in a plain for loop, each store issuing one or more ListKv pages; a fresh ListKv is not a cache read (the agent runs list_kv_metadata, which authenticates and syncs the namespace over the network), and the agent's retained first pages serve a non-fresh read only when the cache holds them, so the first boot after an agent start pays the full cost. Before the port the only bound was the agent's 60 s per request and the command had no total. Three profiles with 40 team stores at about 300 ms per round trip is 47 serial requests, about 14 s before any second page; list_catalog(fresh=true) then expires with deadline-exceeded (retryable, not ambiguous), note_fresh_catalog_read runs only on success so every later listing is fresh too, and the stores late in label order never load. A boot read that fails is not fatal: the shell mounts on an empty snapshot and the reconciliation scheduler runs a per-profile catalog job every 30 s under its own 15 s budget, so the real risk is a profile whose own stores cannot be walked in 15 s, which stays unloaded indefinitely, plus the user's Refresh and the boot walk. ensure_catalog_for_mutation spends an Ordinary mutation command's budget on the same per-profile walk when the profile's catalog is missing or retired. The agent's own cap for ListKv is per request, so widening the desktop command class does not lengthen any single agent request. No real latency was measured; the finding is a sizing defect.

Evidence:

- [`apps/desktop/src-tauri/src/commands/deadline.rs:112`](../../apps/desktop/src-tauri/src/commands/deadline.rs#L112): list_catalog, list_catalog_progressive (113) and list_profile_catalog (119) are DeadlineClass::Ordinary; deadline.rs did not exist at 224a6c4
- [`apps/desktop/src-tauri/src/commands/context/mod.rs:142`](../../apps/desktop/src-tauri/src/commands/context/mod.rs#L142): begin_operation captures now + class.cap() once; every transport is bound to it (174-180)
- [`apps/desktop/src-tauri/src/commands/vault.rs:1179`](../../apps/desktop/src-tauri/src/commands/vault.rs#L1179): load_catalog runs ListProfiles plus the whole multi-profile, multi-store walk under one deadline; note_fresh_catalog_read only on success (1283, 1367)
- [`crates/foks-desktop/src/lib.rs:1233`](../../crates/foks-desktop/src/lib.rs#L1233): serial `for store in stores` walk; page loop at 1708-1763
- [`crates/foks-agent/src/main.rs:5283`](../../crates/foks-agent/src/main.rs#L5283): a fresh ListKv runs list_kv_metadata, which syncs over the network (client-app kv.rs:54-71, foks-client kv/sync.rs:311-343)
- [`apps/desktop/src-tauri/src/commands/preparation.rs:29`](../../apps/desktop/src-tauri/src/commands/preparation.rs#L29): ensure_catalog_for_mutation loads a profile catalog under the mutation command's deadline
- [`apps/desktop/src/data/desktop-reconciliation.ts:79`](../../apps/desktop/src/data/desktop-reconciliation.ts#L79): 30 s per-profile catalog job via list_profile_catalog
- [`book/20-desktop.qmd:210`](../../book/20-desktop.qmd#L210): documents the class caps but not that a catalog walk of N stores shares one 15 s budget

Recommendation: Move list_catalog, list_catalog_progressive and list_profile_catalog to a longer command class (Bulk, or a Catalog class sized from the store count) in command_class, and widen the repair step in ensure_catalog_for_mutation the same way, while keeping each agent request Ordinary (bind the per-request transport to min(remaining, 15 s)). State the sizing in book/20-desktop.qmd. Add a foks-desktop test with a transport that delays each ListKv so the walk crosses the command deadline and assert partial publication plus the error class.

Verifier note: Both verifiers confirmed medium; one rated M for a per-store budget derivation, the other S for reclassifying the three commands. The synthesis takes S for the reclassification and notes the per-store sizing as the fuller M change. Corrections applied: the automatic retry after a failed boot is per profile under its own budget, not the identical whole-catalog walk; the repair inside mutations runs only when the profile's catalog is missing or retired.

### c3-session-kex-receive-budget-per-source

**The global KEX receive budget (32) equals the per-source connection cap, so one source can hold every pairing slot for an hour at a time**

Priority: medium · Type: security · Origin: celld · Effort: S · Area: session · Merged: `c-session-kex-budget-per-source`, `c-security-kex-receive-budget-single-source`

celld (9c19e75) added a non-queuing global budget for anonymous KEX receive long polls so that receives cannot occupy every public connection. kex_receive_capacity() = min(maximum_kex_receive_polls, maximum_active_connections / 2) = 32 by default; admit_kex_receive is a keyless try_acquire on one semaphore; the permit is held until the response is written; and kex::receive sleeps in 5 s slices until min(requested wait, MAX_BLOCKING_WAIT = 1 h). maximum_connections_per_source is also 32 per listener. One IPv4 address or IPv6 /64 can therefore open 32 public connections, send KexReceive for a random session id with poll_wait_milliseconds = 3_600_000 on each, and take the whole budget for an hour, renewing as the polls expire; every other device-pairing receive on the host is answered RateLimited (1012) at admission. Before the port the same 32 connections took 32 of 256 public slots and pairing for other sources continued, so celld changed the exposure from eight sources (all public connections) to one. The relay already has a per-source share (MAX_SOURCE_MESSAGES, kex.rs:16); the receive budget is the only KEX resource without one. The refusal is counted under its own reason (KexReceivePolls) and the README documents the global cap, but not that one source can take all of it.

Evidence:

- [`crates/foks-server/src/config.rs:95`](../../crates/foks-server/src/config.rs#L95): kex_receive_capacity = min(maximum_kex_receive_polls 32, maximum_active_connections / 2); asserted by the config test at 163-166
- [`crates/foks-server/src/net/session.rs:937`](../../crates/foks-server/src/net/session.rs#L937): admit_kex_receive: try_acquire_owned on one global semaphore (175), no source key
- [`crates/foks-server/src/net/session.rs:694`](../../crates/foks-server/src/net/session.rs#L694): KexReceive permit released only after the response is written
- [`crates/foks-server/src/net/session/kex.rs:19`](../../crates/foks-server/src/net/session/kex.rs#L19): MAX_BLOCKING_WAIT = 1 h; receive loops in 5 s slices (120-180); relay per-source share at 15-17, 98-108
- [`crates/foks-server/src/rate_limit.rs:44`](../../crates/foks-server/src/rate_limit.rs#L44): maximum_connections_per_source = 32 per listener; loopback exempt (179); KeyedLimit available (100-130)
- [`docs/review/STATUS.md:852`](../../docs/review/STATUS.md#L852): server-source-admission-fairness follow-up names a per-source cap on concurrent long polls, with no fix recorded
- [`crates/foks-server/README.md:352`](../../crates/foks-server/README.md#L352): documents the global cap only

Recommendation: Add a KeyedLimit<SourceKey> on outstanding KEX receives checked before the global semaphore in kex_long_poll (for example kex_receive_capacity / 8, at least 1, loopback exempt as the connection cap is), released with the KexReceive permit and counted under its own reason or the existing KexReceivePolls; alternatively validate that kex_receive_capacity exceeds maximum_connections_per_source. Add a unit test with two scripted sources showing the first cannot take the whole budget. Update README:352-357 and the server-source-admission-fairness entry in STATUS.md.

Verifier note: Both verifiers confirmed medium, effort S, and agreed this is not a plain duplicate of the open STATUS.md follow-up: the follow-up names the same fix, but celld lowered the requirement from eight sources to one and made the refusal explicit for everyone else. Minor corrections: a receive for a session nobody sends to ends with TransactionRetry at its deadline rather than an earlier error; a per-source budget should carry the loopback exemption; the practical impact today falls on Rust clients because Go-agent provisioning through a Rust host is already broken by r2-compat-long-poll-closes-on-concurrent-go-call.

### c3-storage-contract-import-statement-op-budget

**Import's whole-database checks (integrity_check, foreign_key_check, policy scans) run under the scanner's 1,000,000-opcode per-statement budget, so backups of a few tens of thousands of rows fail qualification after a successful copy**

Priority: medium · Type: bug · Origin: celld · Effort: S · Area: storage-contract · Merged: `c-storage-contract-import-statement-op-budget`

scan::Database::private_target installs the same Control progress hook (every 1000 opcodes, latch after 1000 callbacks) on the import target as on the scanner, and import::integrity and policy::verify only call statement() (which resets the counter) before each whole-database statement: PRAGMA main.foreign_key_check, PRAGMA main.integrity_check(1) (the argument bounds reported errors, not work) and policy::none's full-table EXISTS scans. The cap is sized for the scanner's keyset LIMIT 1 probes. Measured on the bundled SQLite 3.53.2 against the real schema-60 database (101 tables, 185 indexes): integrity_check costs about 24 opcodes per row for a two-btree table and 33 for a three-btree table, so the cap is crossed at roughly 25-40k rows in ordinary indexed tables (60k merkle rows: 1.4M opcodes, interrupted); foreign_key_check and the EXISTS scans bind at about 150-200k rows. Capture, scan, copy_all and COMMIT succeed; then the hook returns true, SQLite aborts the step with SQLITE_INTERRUPT, and the import returns ScanError::Sql { extended_code: Some(9) } (not the Unsupported('SQLite operation/deadline limit') the reviewer named, which appears only on later Control::check calls because the latch is permanent). The qualification harness therefore cannot run on any realistic dataset; only the tiny bootstrapped fixtures in import/tests.rs pass. The path is reachable only through the celld-proof feature and tests, so there is no production impact.

Evidence:

- [`crates/foks-server/src/restricted_storage/mod.rs:171`](../../crates/foks-server/src/restricted_storage/mod.rs#L171): Control::progress adds 1000 per callback and latches once the total reaches 999_000; statement() (165-169) is the only reset; check() returns Unsupported once latched (149-151); rusqlite error -> ScanError::Sql (52-60)
- [`crates/foks-server/src/restricted_storage/scan.rs:114`](../../crates/foks-server/src/restricted_storage/scan.rs#L114): progress_handler(1000) installed in configure, used by private_target (68-70)
- [`crates/foks-server/src/restricted_storage/import/mod.rs:184`](../../crates/foks-server/src/restricted_storage/import/mod.rs#L184): integrity: statement() then foreign_key_check and integrity_check(1) (181-203); called after COMMIT (330-334); celld-proof only (106)
- [`crates/foks-server/src/restricted_storage/import/policy.rs:65`](../../crates/foks-server/src/restricted_storage/import/policy.rs#L65): none(): one EXISTS full scan per table; counts LIMIT 4097 (95-105)

Recommendation: Give whole-database statements their own budget: add a Control method (statement_with_budget(ops), or an unbudgeted statement that keeps the cancellation and deadline checks in the hook but disables the opcode latch) and use it in import::integrity, policy::none and the key-generation counts, deriving the budget from the scanned summary (about 64 opcodes per row per table btree plus a floor); keep the 1,000,000 cap for the keyset probes. Add an import test that populates about 50k rows across merkle_nodes, merkle_leaves and tree_locations and asserts import_personal_list succeeds, and one asserting a deliberately tiny budget yields the chosen error class.

Verifier note: Both verifiers confirmed medium, S, and both measured opcode counts on the real schema (one with the system SQLite, one with the bundled 3.53.2 through a small Rust harness). Correction applied: the surfaced error is Sql{9}, not Unsupported.

### c3-tests-celld-proof-gated-never-run

**Everything behind the celld-proof and list-allocation-qualification features (hosted_personal, proof, the oracle and qualification integration tests) is never compiled, clippy-checked or run by any check script or workflow, and the features are undocumented**

Priority: medium · Type: test · Origin: celld · Effort: S · Area: tests · Merged: `c-tests-celld-proof-gated-tests-never-run`

No script, workflow or document passes --features celld-proof or list-allocation-qualification (grep over tools/, scripts/, .github/, *.md, *.qmd and AGENTS.md returns nothing outside tools/foks-celld-*); tools/foks-server/check.sh and scripts/test-rust-full.sh run default features only. The feature-only public surface (foks-server hosted_personal.rs, net/session/proof.rs, qualify_personal_list_import, start_native_personal_list_qualification, the celld-proof arms of services/kv.rs, listener.rs and session.rs, the testkit celld_proof module) and the five #![cfg(feature)] integration test files (native_list_oracle, the only test comparing hosted_personal::native_list output byte-for-byte with the authenticated List RPC; personal_authority; personal_list_domain; list_allocation; restricted_list_allocation) compile to empty binaries without the feature and so are outside every CI job. The restricted_storage import unit tests do run because that module is cfg(any(test, feature)). A regression in these modules, or a change that stops them compiling (for example to encode_kv_list_request_at or KvListResponse encoding), is noticed only when somebody runs the feature by hand on the lab machine. hosted_personal.rs describes itself as feature-only qualification helpers that install no listener route, so the exposure is silent bit-rot of the qualification evidence, not a runtime defect. A local run with the feature passed native_list_oracle, personal_authority (Go test ignored), personal_list_domain and the 102 restricted/hosted/owned_list unit tests.

Evidence:

- [`tools/foks-server/check.sh:29`](../../tools/foks-server/check.sh#L29): cargo test/clippy with default features only; scripts/test-rust-full.sh:7 likewise; foks-standalone.yml:125,161 only run check.sh
- [`crates/foks-server-testkit/Cargo.toml:10`](../../crates/foks-server-testkit/Cargo.toml#L10): celld-proof and list-allocation-qualification are opt-in features with no default
- [`crates/foks-server-testkit/tests/native_list_oracle.rs:1`](../../crates/foks-server-testkit/tests/native_list_oracle.rs#L1): #![cfg(feature = "celld-proof")]; same for personal_authority.rs, personal_list_domain.rs, list_allocation.rs, celld_proof.rs and foks-server/tests/restricted_list_allocation.rs:5
- [`crates/foks-server/src/lib.rs:78`](../../crates/foks-server/src/lib.rs#L78): hosted_personal and proof modules exist only under the feature (lib.rs:76-82); session.rs:494 and listener.rs:124-155 carry feature arms
- [`crates/foks-server/src/hosted_personal.rs:1`](../../crates/foks-server/src/hosted_personal.rs#L1): 'Feature-only A1 qualification helpers'; no listener route

Recommendation: Append to tools/foks-server/check.sh (or at least the nightly server-full job) `cargo clippy --offline --locked -p foks-server -p foks-server-testkit --features foks-server-testkit/celld-proof --all-targets -- -D warnings` and `cargo test --offline --locked -p foks-server -p foks-server-testkit --features foks-server-testkit/celld-proof` (the #[ignore]d Worker-backed lab tests stay ignored, so the run is self-contained). Add a short 'Qualification features' section to crates/foks-server/README.md naming both features and the ignored lab-only tests.

Verifier note: Both verifiers confirmed medium, S. Correction applied: 'production modules' overstates it; nothing shipped depends on the gated code compiling. All gated files are celld additions.

## Low-priority findings

29 findings.

### c3-backup-lease-failure-unreported

**The scheduled backup's storage-lease acquisition runs outside the phase wrapper, so its failures count as backup failures with no phase/reason record**

Priority: low · Type: maintainability · Origin: celld · Effort: S · Area: backup · Merged: `c-backup-lease-failure-unreported`

celld added StorageLease::with_store(...)? as the first statement of run_backup, outside the checked(Phase, ...) wrapper that BackupRead, BackupCreate, BackupPublish and BackupRetention use; report() is called only from checked() and the request-failure path, so a lease failure increments metrics.backup_failed and writes nothing to stderr, while README:828-836 tells operators to diagnose backups from the 'FOKS operation failed: phase=..., reason=...' record. The lease can fail on DatabasePathIdentity::existing (database path changed), the storage lock not being a regular file or unopenable (EACCES on the database directory), try_lock_shared ('storage is in use' while an offline command holds the exclusive lease), identity rechecks, and namespace_lease I/O on the chunk directory. Wrapping alone would report Error::Chunk variants as reason=internal because reason() has no arm for Error::Chunk. The CLI backup command is unaffected because it reports errors directly. Pre-port run_backup began with checked(Phase::BackupRead, ...) and had no lease.

Evidence:

- [`crates/foks-server/src/operations/backup.rs:115`](../../crates/foks-server/src/operations/backup.rs#L115): StorageLease::with_store(...)? outside checked(); BackupRead wrapper follows at 121-133; Err counted as backup_failed at 73-76
- [`crates/foks-server/src/diagnostics/operational.rs:117`](../../crates/foks-server/src/diagnostics/operational.rs#L117): only checked()/report() write the stderr record; reason() (73-82) has no Error::Chunk arm
- [`crates/foks-server/src/storage_lease.rs:60`](../../crates/foks-server/src/storage_lease.rs#L60): acquire: identity, .storage-lock open and metadata, try_lock_shared, namespace_lease (60-95)
- [`crates/foks-server/README.md:828`](../../crates/foks-server/README.md#L828): operator guidance relies on the phase/reason record

Recommendation: Wrap the lease: `let _storage_lease = checked(Phase::BackupRead, StorageLease::with_store(...))?;` or add Phase::BackupLease and list it in the README phase table; map Error::Chunk in reason() to a non-internal reason. Add a unit test that makes the namespace lock unopenable (or pre-holds the exclusive lease) and asserts the stderr record.

Verifier note: Both verifiers confirmed low, S, from the diff; one added the Error::Chunk reason mapping needed for the record to be actionable.

### c3-chunks-filesystem-inventory-quadratic

**Filesystem inventory re-reads and re-stats the whole directory for every 256-entry page, so a full run is quadratic in the object count and takes hours at the default 1,000,000-object cap while the server is stopped**

Priority: low · Type: perf · Origin: celld · Effort: M · Area: chunks · Merged: `c-chunks-filesystem-inventory-quadratic`

inventory() bounds memory to one page but not time: each page re-runs read_dir over the whole flat directory and calls symlink_metadata on every entry whose name sorts after the cursor, keeping a 256-entry BTreeMap, so a run costs about N^2/256 directory entries read and N^2/512 stats. Measurements on this host (warm cache) gave about 0.65 us per readdir entry and 4-6 us per lstat; extrapolated, 100k objects take about 2.5 minutes, 600k about 1.5 hours and 1M (the default maximum_chunk_objects) about 4 hours, more on a cold cache or a network filesystem. inventory_chunk_store takes DatabaseWriterGuard, so the server is down for the run, and begin_chunk_collection refuses until inventory_complete, so the cost is paid once per collection cycle that needs a re-inventory. The ledger-side work (one query per entry inside one transaction per page) is linear and not the bottleneck. The module comment records the memory bound as deliberate; the quadratic time is not called out anywhere.

Evidence:

- [`crates/foks-server/src/chunks/inventory.rs:39`](../../crates/foks-server/src/chunks/inventory.rs#L39): per-page read_dir over all entries; cursor filter at 47; symlink_metadata at 51 for every surviving entry; BTreeMap truncation at 56-59
- [`crates/foks-server/src/chunk_ledger.rs:43`](../../crates/foks-server/src/chunk_ledger.rs#L43): inventory_chunk_store takes DatabaseWriterGuard::acquire; pages of 256 (49)
- [`crates/foks-server/src/chunks.rs:424`](../../crates/foks-server/src/chunks.rs#L424): objects live in one flat directory (directory.join(object_key))
- [`crates/foks-server-db/src/chunk_collection.rs:30`](../../crates/foks-server-db/src/chunk_collection.rs#L30): collection requires a complete inventory
- [`crates/foks-server-db/src/config.rs:100`](../../crates/foks-server-db/src/config.rs#L100): maximum_chunk_objects defaults to 1,000,000

Recommendation: Minimal (S): select the page by name only, then symlink_metadata just the at most 256 selected names after the read_dir loop, which removes the stat term and leaves roughly 40 minutes at 1M objects. Full fix (M): on the first page read the directory once into a sorted on-disk or bounded in-memory name list under the exclusive storage lease and page through it, checkpointing the cursor as now; or shard objects into prefix subdirectories at publication, which changes validate_key, the marker and needs a migration.

Verifier note: Both verifiers confirmed low, M, and both measured: the reviewer's 'hours to days' is the right order of magnitude at the high end but 'many hours' at 600k assumes a cold cache. Both offered the stat-lazily change as an S partial fix.

### c3-chunks-readme-no-collector-and-15s-target

**The server README states there is no object garbage collector and a 15-second hosted target, contradicting the collect-chunk-store command and the 30 s request timeout the port restored; neither chunk-store command is documented anywhere**

Priority: low · Type: docs · Origin: celld · Effort: S · Area: chunks · Merged: `c-chunks-readme-stale-collector-and-timeout`

README.md:661 says 'There is no object garbage collector in this release' and that objects remain after upload cleanup or removal of their SQLite references, while the same port added collect_chunk_store (filesystem only, offline, exclusive lease, authority-verified) and the collect-chunk-store and inventory-chunk-store commands, which no README, ADMINISTRATION.md, foks-server-db README or book chapter mentions; the foks-server-db README:101 still says 'A coordinated collector is future work'. README.md:657 ('the 15-second hosted target') and 684 ('not a 15-second failover path') matched celld's 15 s request_timeout default; bca9eee restored 30 s and updated line 161 and the book but not these two lines, so the README contradicts itself. An operator relying on 'objects remain after removal of their SQLite references' who keeps an un-hydrated SQLite copy as a cold backup and runs collect-chunk-store on the live installation later finds the copy's external references pointing at deleted objects. The separate '15-second I/O timeout' at README:436 and book/18:225 is a different, still-correct limit.

Evidence:

- [`crates/foks-server/README.md:661`](../../crates/foks-server/README.md#L661): 'There is no object garbage collector in this release.'; 657 and 684 state a 15-second target; 161 and 437 say 30 s
- [`crates/foks-server/src/chunk_collection.rs:16`](../../crates/foks-server/src/chunk_collection.rs#L16): collect_chunk_store deletes unreferenced filesystem objects; commands at bin/foks-server.rs:80-105
- [`crates/foks-server-db/README.md:101`](../../crates/foks-server-db/README.md#L101): 'A coordinated collector is future work'

Recommendation: Rewrite the external-chunks section to describe the inventory and collection model (filesystem only, offline, writer guard plus storage lease, authority identity), what is never deleted (unknown, uncertain and remote objects), that a non-hydrated SQLite copy is not a backup once collection has run, and the two commands and their flags; change the two 15-second sentences to the request deadline (30 s default, request_timeout_seconds). Fix foks-server-db README:101 the same way.

Verifier note: Both verifiers confirmed low, S. Provenance correction: neither line existed at 224a6c4; both were written by celld and arrived with 6581535, so the collector statement was inconsistent within celld itself, and the 15-second statements went stale with the port's resolution in bca9eee. Origin is recorded as celld with that port component noted.

### c3-chunks-temp-files-charged-forever

**Temporary files left by a hard stop during a filesystem put are inventoried as unknown objects, charged to physical capacity permanently, never collectable, and block the empty-namespace fast path**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: chunks · Merged: `c-chunks-temp-files-charged-forever`, `c-security-filesystem-chunk-temporaries-leak`

FilesystemChunks::put_object and create_namespace_marker_if_absent create NamedTempFile::new_in(directory) (tempfile 3.27.0 default prefix '.tmp' plus six random characters) in the object directory and persist_noclobber it. NamedTempFile removes its file on Drop, so every error path (deadline check, write, sync, recheck, persist failure, a dying client) cleans up; a leftover needs the process to die (SIGKILL, OOM kill, power loss, or the binary's process::exit(1) fatal path) while a blocking put is between create and persist. Once left, the inventory treats any regular file other than the marker and lock as an object, record_chunk_inventory_page inserts it as state 3 with digest NULL, the insert trigger adds its bytes to present_bytes and one to objects, admission compares those totals against maximum_chunk_physical_bytes and maximum_chunk_objects, and collection candidates require digest IS NOT NULL, so the file (up to 9 MiB) is charged for the life of the installation and never deletable; delete_object requires a ledger key that validate_key accepts, which a '.tmp' name is not. empty() also counts it, so a stale temporary in the window of a first start (a hard stop during the marker's own temporary write, or after clear_private_chunk_ledger or an inventory restart) means every external upload is refused with 'chunk inventory is incomplete' until an offline inventory runs. The current test fixture treats 'old-temp' as an ordinary unknown key, so the behaviour is intended, but nothing distinguishes the server's own residue from foreign objects.

Evidence:

- [`crates/foks-server/src/chunks.rs:425`](../../crates/foks-server/src/chunks.rs#L425): NamedTempFile::new_in(&self.directory) then persist_noclobber; same for the marker at 397; error paths drop the temporary (429-446)
- [`crates/foks-server/src/chunks/inventory.rs:19`](../../crates/foks-server/src/chunks/inventory.rs#L19): empty() counts anything but marker and lock; unknown names accepted as keys at 45-55
- [`crates/foks-server-db/src/chunk_ledger.rs:437`](../../crates/foks-server-db/src/chunk_ledger.rs#L437): unknown key inserted as state 3, digest NULL
- [`crates/foks-server-db/src/schema/chunk_ledger.sql:39`](../../crates/foks-server-db/src/schema/chunk_ledger.sql#L39): insert trigger adds state-3 bytes to present_bytes and increments objects
- [`crates/foks-server-db/src/chunk_collection.rs:15`](../../crates/foks-server-db/src/chunk_collection.rs#L15): collection candidates require digest IS NOT NULL (also 36)
- [`crates/foks-server/src/chunks.rs:877`](../../crates/foks-server/src/chunks.rs#L877): test fixture treats 'old-temp' as an object
- [`crates/foks-server/src/bin/foks-server.rs:930`](../../crates/foks-server/src/bin/foks-server.rs#L930): process::exit(1) fatal path

Recommendation: Create temporaries with tempfile::Builder::new().prefix("_foks_tmp-") in put_object and create_namespace_marker_if_absent; skip that prefix in empty() and inventory() as NAMESPACE_MARKER and NAMESPACE_LOCK are skipped, and reject it in record_chunk_inventory_page alongside the marker name; unlink stale '_foks_tmp-*' entries at the start of inventory_chunk_store and collect_chunk_store, which already run under the writer guard with no live publisher. Add a test that a stale temporary neither counts as an object nor blocks confirm_empty_chunk_inventory.

Verifier note: Both verifiers confirmed low, S. Corrections applied: only a hard stop leaves a temporary, because NamedTempFile cleans up on every error path and a request deadline leaves the blocking handler running to completion; the merged security candidate's 'deadline-triggered abort' therefore still requires process death. The 'fresh namespace blocks uploads' sub-claim is narrower than stated because confirm_empty_chunk_inventory runs at startup before any put can create a temporary.

### c3-compat-memory-failure-closes-read-routes

**A working- or output-memory shortfall on a read-only route, or during request decoding, closes the connection without a response instead of answering 1012**

Priority: low · Type: compat · Origin: celld · Effort: S · Area: compat · Merged: `c-compat-memory-budget-failure-closes-read-routes-unanswered`, `c-session-decode-allocation-closes-connection`

serve() returns Ok(()) (close, no response) whenever outcome._memory.failed(), for every route, so that a possibly committed write stays ambiguous; AllocationBudget::reserve and ResponseAllocationBudget::reserve set failed on any shortfall, including budgeted_get during a read query. Read-only routes (KvStoreList, GetDir, GetEncryptedChunk) have nothing to keep ambiguous, yet under working-pool pressure a client listing a large directory gets EOF rather than the documented 1012: the handler maps the FromSqlConversionFailure to TransactionRetry (1014), that status is encoded, and then discarded because failed() is set. A second path has the same outcome: decode_message runs under memory.sync before admit_working, and an allocation shortfall during decoding becomes foks_snowpack ErrorKind::AllocationLimit wrapped as foks_rpc::Error::Snowpack, which serve() returns as Err, closing the connection and classifying it as a protocol (Rpc) error in diagnostics. The Go client surfaces a closed connection as io.EOF for the pending call and the Rust agent's RateLimited mapping is bypassed. book/18-server-storage.qmd:222 says the close happens 'once a handler may have written', which the code does not distinguish. Pre-port decode had no allocation charges and session.rs had no failed() check. Reachability is limited: with default limits the bulk cap bounds the heavy chargers and validate() refuses a working pool under 512 MiB, so exhaustion needs many concurrent small requests with large SQL results; raising the bulk cap (c3-session-bulk-pool-four-permits) makes this path more reachable.

Evidence:

- [`crates/foks-server/src/net/session.rs:640`](../../crates/foks-server/src/net/session.rs#L640): `if outcome.disconnect_before_response || outcome._memory.failed() { return Ok(()) }` for every route
- [`crates/foks-hosted-transport/src/memory.rs:340`](../../crates/foks-hosted-transport/src/memory.rs#L340): reserve sets failed on any working, output or retained-list shortfall (340-383); inputs of at most 4096 bytes fall back to the 16 MiB small headroom (274-294)
- [`crates/foks-server-db/src/allocation.rs:11`](../../crates/foks-server-db/src/allocation.rs#L11): budgeted_get turns a shortfall into FromSqlConversionFailure
- [`crates/foks-server/src/services/kv.rs:99`](../../crates/foks-server/src/services/kv.rs#L99): database errors mapped to TransactionRetry, not RateLimited
- [`crates/foks-server/src/net/session.rs:981`](../../crates/foks-server/src/net/session.rs#L981): decode under memory.sync before admit_working; decode error returned as Err at 385
- [`crates/foks-server/src/diagnostics.rs:40`](../../crates/foks-server/src/diagnostics.rs#L40): Error::Rpc classified SessionErrorClass::Rpc
- [`book/18-server-storage.qmd:222`](../../book/18-server-storage.qmd#L222): 'status 1012, or the connection closes once a handler may have written'

Recommendation: Give RouteSpec a read-only (or mutates) flag; in serve(), when _memory.failed() and the route is read-only, or the failure happened during decode before any handler ran, encode RateLimited at the decoded sequence from a fresh control-pool request (as the handler-cap path does) instead of closing; match ErrorKind::AllocationLimit in read_call_body_async so decode-time refusals are not classified as malformed frames. Add a session test with a forced budget failure asserting 1012 for KvStoreList and for an oversized decode, and a closed connection for a mutation.

Verifier note: Both verifiers confirmed low, S. Correction applied: even if the handler's status were written it would be 1014, not the documented 1012, so the fix must synthesize RateLimited rather than write the handler's status. One verifier noted the mitigating small-headroom fallback for inputs of at most 4096 bytes, so decode-time refusals affect only larger arguments.

### c3-deadlines-backend-pairing-cap-shrunk

**AgentClient::set_timeout is now a ceiling for every deadline class, and foks-desktop-backend also binds a process-wide deadline, so its pairing commands run under the 15 s default instead of 300 s**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: deadlines · Merged: `c-deadlines-backend-pairing-cap-shrunk`

Pre-port, call_cancellable applied self.timeout.max(DEVICE_PAIRING_TIMEOUT) and .max(CHAT_POLL_TIMEOUT), so a general-purpose client timeout never cut a pairing wait. deadline_for now uses self.timeout.unwrap_or(class.cap()).min(class.cap()), inverting that. foks-desktop-backend sets set_timeout(request_timeout_seconds) (default 15) and additionally binds a process-wide Deadline::capture(started + request_timeout_seconds) through with_local_deadline, which deadline_for's existing.shorten keeps, so fixing set_timeout alone would not restore the budget. FinishDevicePairing, AcceptDevicePairing and ResumeDevicePairingAcceptance are Pairing class (300 s cap) and are built by the backend; with a 15 s envelope the agent's pairing_relay_budget is 7.5 s because the cap is below PAIRING_COMPLETION_MARGIN (45 s). A `pair-accept` through the backend while the other device takes 30 s to respond now fails with a deadline error where it waited up to five minutes before. Impact is limited to the scripted transcript shell: the Tauri desktop derives its deadline from operation.deadline_class().cap() and the CLI budgets pairing commands at 300 s. Any other embedder that calls set_timeout as an Ordinary ceiling and then issues Pairing or ChatPoll requests inherits the same inversion.

Evidence:

- [`crates/foks-agent-client/src/lib.rs:159`](../../crates/foks-agent-client/src/lib.rs#L159): deadline_for: cap = self.timeout.unwrap_or(class.cap()).min(class.cap()); pre-port .max(DEVICE_PAIRING_TIMEOUT) at 224a6c4 lib.rs:175-177
- [`crates/foks-desktop/src/main.rs:26`](../../crates/foks-desktop/src/main.rs#L26): request_timeout_seconds default 15; set_timeout at 516 and a process-wide Deadline::capture at 505 (498-519)
- [`crates/foks-desktop/src/main.rs:767`](../../crates/foks-desktop/src/main.rs#L767): backend builds the three Pairing-class operations (767-790); the test at 1805 checks only the operation mapping
- [`crates/foks-agent/src/main.rs:1474`](../../crates/foks-agent/src/main.rs#L1474): pairing_relay_budget(15 s) = 7.5 s; PAIRING_COMPLETION_MARGIN 45 s at 82
- [`apps/desktop/src-tauri/src/agent/transport.rs:441`](../../apps/desktop/src-tauri/src/agent/transport.rs#L441): the Tauri desktop derives its deadline from the operation's class cap

Recommendation: In foks-desktop-backend derive the process deadline and client timeout from max(request_timeout_seconds, operation.deadline_class().cap()) for Pairing, Bulk and ChatPoll operations, treating --request-timeout-seconds as the Ordinary ceiling only (mirroring the Tauri transport), or document that the flag must be raised to 300 for pair-* commands and default it so. Add a backend test asserting a pair-accept command's envelope expires well beyond the Ordinary cap.

Verifier note: Both verifiers confirmed low, S. Both added that the process-wide deadline is a second, independent cap, so the fix must address the backend's run() and not only set_timeout.

### c3-deadlines-mcp-startup-window

**MCP startup shares one 5 s deadline across the running-agent probe, agent launch, readiness and account binding; pre-port each phase had its own wait**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: deadlines · Merged: `c-deadlines-mcp-startup-window`

mcp::run shortens the 15 s Mcp command budget to now + 5 s and passes that one deadline to ensure_agent_with_deadline, whose running-agent wait (2 s) and launch wait (5 s) are each min'd with it, and to connect_with_deadline -> bind_account, which shortens the same deadline by another 5 s (no effect) and retries ProfileBusy only until it expires. Pre-port the phases were sequential and independent (2 s probe, 5 s launch wait, 5 s bind, up to 12 s). The probe for an absent agent fails fast, so the material loss is that bind_account now gets only what agent startup left of the 5 s; the agent waits in admission up to admission_budget(remaining) before answering ProfileBusy, so a bind issued with 4 s left spends it waiting and retrying while another front end runs a 10 s profile sync, and startup fails with 'authenticated agent startup timed out' or 'selected profile remains busy' although nothing is wrong. startup_context treats Expired as Ok, so the loops end through their Instant checks rather than an error.

Evidence:

- [`crates/foks-cli/src/mcp.rs:35`](../../crates/foks-cli/src/mcp.rs#L35): startup = current().shorten(now + 5 s); ensure_agent_with_deadline and connect_with_deadline both run under it
- [`crates/foks-cli/src/mcp.rs:190`](../../crates/foks-cli/src/mcp.rs#L190): RUNNING_AGENT_STATUS_WAIT (2 s) and STARTUP_WAIT (5 s) each min'd with startup.instant() (190, 222); Expired treated as Ok at 279-286
- [`crates/foks-mcp/src/agent.rs:610`](../../crates/foks-mcp/src/agent.rs#L610): bind_account shortens the same deadline by 5 s and retries ProfileBusy until it expires (603-640)
- [`crates/foks-cli/src/main.rs:798`](../../crates/foks-cli/src/main.rs#L798): Command::Mcp budget is 15 s, so there is headroom
- [`crates/foks-agent/src/main.rs:1355`](../../crates/foks-agent/src/main.rs#L1355): admission waits up to admission_budget(remaining) before answering ProfileBusy

Recommendation: Keep the 5 s window for ensure_agent_with_deadline (probe then launch), then derive a fresh bind deadline for connect_with_deadline (for example current().shorten(now + 5 s)) inside the command's 15 s, or pass the command deadline and give each phase its own bound. Add a unit test with a fake agent that answers ProfileBusy for 3-4 s after a 1.5-3 s startup and asserts the bind still succeeds.

Verifier note: Both verifiers confirmed low, S; cold-start timing was not measured.

### c3-deadlines-mcp-upload-ordinary-cap

**MCP tools run every call, including 4 MiB streaming uploads, under a fixed Ordinary 15 s deadline captured before parsing and before worker admission**

Priority: low · Type: bug · Origin: pre-existing · Effort: S · Area: deadlines · Merged: `c-deadlines-mcp-upload-ordinary-cap`

call_tool captures Deadline::capture(now + DeadlineClass::Ordinary.cap()) for every tool before parsing and before waiting on the server's active permit; the worker receives that deadline, AgentBackend::invoke intersects it into the client via with_local_deadline, and deadline_for(Bulk) then shortens its 300 s original to the existing 15 s, so a Put write of up to MAX_FILE_BYTES (4 MiB) is bounded by 15 s minus queueing, parsing and binding. On a slow upstream link (about 17 s for 4 MiB at 2 Mbit/s) or a busy agent the upload is cancelled mid-stream and reported as 'upload interrupted; inspect submission_id', an ambiguity the caller must resolve with foks_status although a Bulk-class request would have completed. This is not a port regression: pre-port the MCP used AgentClient's default 15 s timeout for put_kv_stream and the agent bounded streaming uploads by its agent-wide timeout; the port introduced a Bulk class that the MCP server does not use, and the 15 s now also covers the permit wait, which was unbounded before.

Evidence:

- [`crates/foks-mcp/src/session.rs:99`](../../crates/foks-mcp/src/session.rs#L99): call_tool: Deadline::capture(now + Ordinary.cap()) before parsing and before the active permit (93-151)
- [`crates/foks-mcp/src/agent.rs:586`](../../crates/foks-mcp/src/agent.rs#L586): invoke intersects the deadline into the client via with_local_deadline (586-597)
- [`crates/foks-agent-client/src/lib.rs:163`](../../crates/foks-agent-client/src/lib.rs#L163): deadline_for(Bulk) shortened to the existing 15 s
- [`crates/foks-mcp/src/agent/writes.rs:75`](../../crates/foks-mcp/src/agent/writes.rs#L75): put_kv_stream_cancellable for Put writes; 'upload interrupted; inspect submission_id' at 76
- [`crates/foks-mcp/src/contract.rs:11`](../../crates/foks-mcp/src/contract.rs#L11): MAX_FILE_BYTES = 4 MiB

Recommendation: In call_tool capture the deadline after parse() from the invocation's class (Bulk for a Put write with a body, Ordinary otherwise), and start it after the active permit is acquired or bound the permit wait separately. Add a session test that a Put invocation reaches the backend with an envelope above the Ordinary cap.

Verifier note: Both verifiers confirmed low, S, and corrected the origin to pre-existing (the 15 s ceiling predates the port; 224a6c4 agent-client lib.rs:112, 218 and agent main.rs:1435-1446). Typical MCP bodies are small, which keeps it low.

### c3-deadlines-retention-50ms-covers-opens

**Retention maintenance runs admission, registry, credential and profile-session opens under the same 50 ms budget as the maintenance slice, and a pass that fails there is counted but never logged**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: deadlines · Merged: `c-deadlines-retention-50ms-session-open`

main.rs enters RequestControl::background(50 ms) around retention::run_one_profile and run_profile computes a second 50 ms Instant deadline; both start before profile_work admission. acquire_blocking checks request_control::current(), so the admission wait shrank from the pre-port 1 s to the remainder of the 50 ms, and ProfileSession::open binds to runtime::current_operation() and calls check_operation at entry, after building the client and inside the checked-session entry (checkpoint verification), so registry open, credential open, client construction and the checked session all consume and can be failed by the budget. An Err (deadline inside the opens) or an admission miss (Ok(None)) is counted as FAILURES with no log and Schedule::finish defers the profile 60 s; a pass whose opens merely use up the slice returns more_work=true and is rescheduled at 250 ms, so it does not back off. Measured on this 4-CPU host under load, the opens cost about 10-25 ms (about 2x headroom), so the budget is not exceeded here; a slow disk, a Native credential backend whose master_key() reaches the Keychain or Secret Service when no session key is cached, or any foreground request holding the profile for more than 50 ms (admission miss, then a 60 s back-off where pre-port waited 1 s) are the realistic triggers. An agent with a profile whose checked session fails showed attempts 2 / failures 2 / successes 0 with no retention line in the log. Pre-port cadence was one profile per 60 s, so the 60 s back-off is not itself a regression; the reduced admission and open budget and the silent failure are.

Evidence:

- [`crates/foks-agent/src/main.rs:377`](../../crates/foks-agent/src/main.rs#L377): RequestControl::background(50 ms) entered around run_one_profile
- [`crates/foks-agent/src/retention.rs:204`](../../crates/foks-agent/src/retention.rs#L204): deadline taken before acquire_blocking, ProfileRegistry::open, ClientCredentials::open, ProfileSession::open and the checked session (204-234); Err counted, not logged (137-157); finish: Err/None -> 60 s (56-71)
- [`crates/foks-agent/src/profile_work.rs:370`](../../crates/foks-agent/src/profile_work.rs#L370): acquire_blocking checks request_control::current(), bounding admission by the 50 ms
- [`crates/foks-client-app/src/registry.rs:1897`](../../crates/foks-client-app/src/registry.rs#L1897): ProfileSession::open binds to current_operation and calls check_operation at entry and after client build (1950-1990)
- [`crates/foks-client-app/src/checkpoint.rs:955`](../../crates/foks-client-app/src/checkpoint.rs#L955): try_with_checked_session: check_operation before and after verification (946-993); master_key may read the native keystore (594-610)
- [`crates/foks-client-app/src/adapter_maintenance.rs:56`](../../crates/foks-client-app/src/adapter_maintenance.rs#L56): maintain_adapter_state_until returns more_work=true when the deadline has already passed

Recommendation: Enter the RequestControl with a budget that covers admission and the opens (for example the pre-port 1 s), keep the admission timeout at that budget, and compute the 50 ms slice deadline immediately before maintain_adapter_state_until, after try_with_checked_session has entered. Log once per profile (rate-limited) when run_profile returns Err for a reason other than admission and expose a last_error in RetentionStatus; optionally treat an admission miss as 'retry at next tick' rather than 60 s.

Verifier note: Both verifiers confirmed and lowered the reviewer's medium to low, S: one measured the opens at 18-36 ms end to end on this loaded host, so the budget holds here with little headroom, and both corrected that a pass which only runs out of slice does not back off 60 s; only an Err or an admission miss does. Maintenance is delayed, not lost.

### c3-deadlines-time-namespace-required

**The Linux clock domain requires /proc/thread-self/ns/time, so kernels without CONFIG_TIME_NS (pre-5.6, the RHEL 8 family, some sandboxes) make every CLI command and every agent request fail with 'local deadline clock is unavailable'**

Priority: low · Type: compat · Origin: celld · Effort: S · Area: deadlines · Merged: `c-deadlines-time-namespace-hard-requirement`

linux_domain() maps any metadata error on /proc/thread-self/ns/time, including ENOENT, to Error::Unavailable with no fallback; platform_sample calls it twice per sample and every Deadline::capture, receive and check samples. Time-namespace proc entries exist only on kernels built with CONFIG_TIME_NS (Linux 5.6+, a build option), so on RHEL/Alma/Rocky 8 (4.18), older Android and WSL1 kernels, and sandboxes that hide /proc/thread-self/ns, the CLI fails in CommandScope::enter before any command runs ('local deadline clock is unavailable: No such file or directory (os error 2)', which names neither the file nor the requirement), the agent starts but rejects every request with DeadlineClockUnavailable, and the desktop and MCP server fail the same way. No document states a minimum kernel. The refusal is a defensible safety choice because the namespace inode is what detects a different CLOCK_MONOTONIC offset, but without CONFIG_TIME_NS there is exactly one monotonic clock per boot, so ENOENT is a safe case to encode rather than refuse.

Evidence:

- [`crates/foks-local-deadline/src/clock.rs:82`](../../crates/foks-local-deadline/src/clock.rs#L82): linux_domain: metadata('/proc/thread-self/ns/time') error -> Error::Unavailable, no ENOENT distinction (78-89); platform_sample calls it twice (91-98)
- [`crates/foks-local-deadline/src/lib.rs:15`](../../crates/foks-local-deadline/src/lib.rs#L15): Unavailable display text does not name the path or requirement; DomainValue::Linux {boot, device, inode} at 101-112
- [`crates/foks-cli/src/deadline.rs:26`](../../crates/foks-cli/src/deadline.rs#L26): CommandScope::enter captures a deadline before dispatch (foks-cli main.rs:809)
- [`crates/foks-agent/src/main.rs:1182`](../../crates/foks-agent/src/main.rs#L1182): RequestControl::receive fails every request; DeadlineClockUnavailable at 2259

Recommendation: In linux_domain treat ErrorKind::NotFound for /proc/thread-self/ns/time as 'kernel without time namespaces' and emit a distinct, deterministic DomainValue (device and inode 0, or a separate variant) so both processes still compare boot id and clock; keep refusing on every other error. Make the Unavailable message name the path, document the kernel requirement in the book, and add a unit test for the NotFound branch through an injectable metadata reader.

Verifier note: Both verifiers confirmed low, S, by reading; this host (6.18) has the file, so the failure was not executed. The list of affected deployment targets is correct in kind; the specific sandboxes were not verified.

### c3-desktop-read-selection-not-dirent-bound

**Read commands resolve the renderer's selection by path and version only, unlike mutations and copy_item_to_store, so an entry replaced under the same path and version by another dirent would be read in place of the listed one**

Priority: low · Type: bug · Origin: pre-existing · Effort: S · Area: desktop · Merged: `c-desktop-read-selection-not-dirent-bound`

celld's 4ed9899 bound the agent hop of a read to the catalog entry's dirent_id. The renderer hop is unchanged: read_item, copy_item_field, open_item_url and download_file resolve the selection through selected_item(store, path, version), the bridge sends only {storeId, path, version} although ItemDto.direntId exists, and selected_bound_item (written for reads) is used only by copy_item_to_store, while mutations use selected_bound_mutation_item and refuse a mismatch with 'conflict'. The reviewer's scenario cannot happen as written: a file removed and recreated at the same path is not version 1 again in either client (the Rust client's prepare_dirent reuses the existing dirent id, including a tombstone's, at version+1; Go v0.1.9's Dirent.edit does ret.Version++ keeping the id), so the existing version check already refuses it. A same-path, same-version, different-dirent entry requires the parent directory node to be replaced (a folder removed or moved and recreated with a same-named child), and the host's native snapshot is replaced only by renderer-invoked listings whose result the renderer receives at once and by the mutation repair in ensure_catalog_for_mutation; there is no background host loop, so the divergence window is a render tick or the interval between a repair and the renderer's own profile refresh, and what is then read is the current content at the path the user clicked. The gap is a consistency defect against the documented 'bound to entry identity' model with a trivial fix; it predates the port.

Evidence:

- [`apps/desktop/src-tauri/src/commands/context/lookup.rs:21`](../../apps/desktop/src-tauri/src/commands/context/lookup.rs#L21): selected_item(store, path, version): path then version (21-70); selected_bound_item at 593-606 used only by vault_copy.rs:361
- [`apps/desktop/src-tauri/src/commands/vault.rs:1402`](../../apps/desktop/src-tauri/src/commands/vault.rs#L1402): read_item; copy_item_field 1535, open_item_url 1596, download_file 1651 use selected_item; the agent hop is bound at 609-631
- [`apps/desktop/src/bridge/commands-vault.ts:107`](../../apps/desktop/src/bridge/commands-vault.ts#L107): readItem/copyItemField/openItemUrl/downloadFile send {storeId, path, version}; ItemDto.direntId (vault-catalog.ts:41) is not sent
- [`crates/foks-client/src/kv/write.rs:1152`](../../crates/foks-client/src/kv/write.rs#L1152): prepare_dirent reuses the existing id at version+1 for any existing entry including a tombstone (1152-1188, 1220-1236)
- `/root/go/pkg/mod/github.com/foks-proj/go-foks@v0.1.9/client/libkv/minder.go:1169` (Go module source): Dirent.edit keeps the id and increments the version

Recommendation: Add direntId to ItemRequest and send it from readItem, copyItemValue, copyItemField, openItemUrl and downloadFile; have the four host commands take dirent_id and resolve through selected_bound_item(.., 'reading'); extend the mock bridge; add a tests/vault.rs case where the snapshot is replaced by a same-path, same-version entry with another dirent and the read returns 'conflict'.

Verifier note: Both verifiers confirmed the code gap and refuted the failure scenario (recreation yields version+1 in both clients), lowering the reviewer's medium to low, S. Origin pre-existing: read_item took (store, path, version) before the port; celld's commit bound only the agent hop and its test substitutes the dirent in the agent reply, not in the snapshot.

### c3-desktop-upload-change-postwrite-lock-check

**resume_file_upload and cancel_file_upload re-check the app lock after the agent call, so a completed resume or cancel is reported as 'locked before the change was submitted' and classified as not applied**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: desktop · Merged: `c-desktop-upload-change-postwrite-app-locked`

change_file_upload is the only mutation whose check_mutation_access runs after the agent call (list_file_uploads does the same after its read). A lock taken during ResumeKvUpload or CancelKvUpload (a resume has a 300 s budget) discards a completed report and returns 'app-locked: The application locked before the change was submitted.', and failureOutcome maps app-locked to 'rejected' (never started) although the file was published or the spool cancelled. The user impact claimed by the reviewer does not reproduce: both lock paths tear the vault shell down before the reply can be shown (lockNow calls lockApp then setLoaded(null); the macOS menu path runs lock_now on the host and the renderer unmounts on the lock event), FileUploadsPanel keeps its error in component state that is lost with the shell and remounts closed, there is no idle auto-lock, and lock_now invalidates the whole catalog so every listing after unlock is a full reload; the panel prints only the normalized message and does not consult failureOutcome. What remains is a factually wrong message and code on a command that did apply its change, a redundant check (the report carries upload_id, state, counts, node_id and version, no plaintext), and a wrong classification for any future caller that classifies the error.

Evidence:

- [`apps/desktop/src-tauri/src/commands/vault.rs:2027`](../../apps/desktop/src-tauri/src/commands/vault.rs#L2027): post-dispatch check_mutation_access discards the report (1984-2029); pre-dispatch check at 1998 and invalidate_catalog_items at 1999; same trailing check on list_file_uploads at 1981
- [`apps/desktop/src-tauri/src/commands/preparation.rs:170`](../../apps/desktop/src-tauri/src/commands/preparation.rs#L170): fixed text 'The application locked before the change was submitted.'
- [`apps/desktop/src/data/operation-outcome.ts:80`](../../apps/desktop/src/data/operation-outcome.ts#L80): app-locked falls to 'rejected'; the comment at 36-47 lists only app-lock-unavailable for the post-write case
- [`apps/desktop/src/app/app-bootstrap.ts:476`](../../apps/desktop/src/app/app-bootstrap.ts#L476): both lock paths unmount the shell (476-510); unlock reboots (525-531)
- [`crates/foks-client-app/src/kv/uploads.rs:10`](../../crates/foks-client-app/src/kv/uploads.rs#L10): the report carries no plaintext

Recommendation: Drop the trailing check_mutation_access in change_file_upload (keep the pre-dispatch one), or return a distinct code such as 'app-locked-after-write' with ambiguous=true and a message saying the change may have been applied, set mutation_requires_refresh as the deadline branch at vault.rs:2022 does, and add that code to failureOutcome's 'unknown' list. Add a transcript test for a lock during the call.

Verifier note: Both verifiers confirmed and lowered the reviewer's medium to low, S: the shell teardown on lock and the root-scope catalog invalidation remove the visible consequences, leaving the wrong text and classification. The trailing check is celld's original.

### c3-desktop-upload-commands-untested

**No test exercises list_file_uploads, resume_file_upload or cancel_file_upload, or the Uploads panel**

Priority: low · Type: test · Origin: celld · Effort: S · Area: desktop · Merged: `c-desktop-upload-commands-untested`

The three new commands have distinct lock, deadline and reservation ordering (reservation before upload_id parse, invalidate before dispatch, lock check after dispatch) and the panel has a hand-written state-to-action table, but nothing under apps/desktop/src-tauri/src/commands/tests or apps/desktop/tests references list_file_uploads, resume_file_upload, cancel_file_upload, ListKvUploads, ResumeKvUpload, CancelKvUpload, FileUploadsPanel or the bridge methods. The only related native test, durable_upload_reconciliation_keeps_inflight_exclusion_and_does_not_clear_ambiguity, exercises begin_upload_reconciliation on AppState alone; tests/file-uploads.test.ts tests only the decoders, wire-contract-domains.test.ts records the FileUploads shape as covered by that decoder test, the deadline-table signature test checks only that the commands have a class, and the existing upload tests in tests/vault.rs cover upload_file for drop and replace. A change that moves the upload_id parse before the reservation, or renames an agent state string, passes the suite.

Evidence:

- [`apps/desktop/src-tauri/src/commands/vault.rs:1952`](../../apps/desktop/src-tauri/src/commands/vault.rs#L1952): list_file_uploads; change_file_upload at 1984; resume/cancel at 2031/2042
- [`apps/desktop/src-tauri/src/commands/tests/context.rs:1467`](../../apps/desktop/src-tauri/src/commands/tests/context.rs#L1467): the only related test covers the reservation semantics, not the commands
- [`apps/desktop/tests/file-uploads.test.ts:1`](../../apps/desktop/tests/file-uploads.test.ts#L1): decoder tests only; no render test imports file-uploads.tsx
- [`apps/desktop/src-tauri/src/commands/tests/vault.rs:1011`](../../apps/desktop/src-tauri/src/commands/tests/vault.rs#L1011): existing UploadTransport harness covers upload_file for drop/replace only (1011-1200)

Recommendation: Add transcript tests in commands/tests/vault.rs for the three commands (operation shape and `after` pass-through for ListKvUploads, reservation taken before parsing, invalidate_catalog_items before dispatch, ambiguous failure sets mutation_requires_refresh, lock during the call asserting the code chosen for c3-desktop-upload-change-postwrite-lock-check) using the existing command harness plus a transport stub for the three operations, and a render test of FileUploadsPanel asserting the per-state buttons and that completed rows show the receipt fields.

Verifier note: Both verifiers confirmed low; one rated S, the other M because the commands take AppHandle and Webview and need the command harness plus a transport stub. The synthesis takes S: tests/vault.rs already drives upload_file through that harness with an UploadTransport stub, so the new tests reuse it.

### c3-desktop-upload-handle-hint-without-record

**Every failed drop or replace upload tells the user to open Uploads to inspect or resume, including failures the agent never recorded**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: desktop · Merged: `c-desktop-upload-handle-hint-without-record`

upload_file issues the handle locally via ensure_upload_identity (KvUploadIssuance returns only database_id and issued_at; the id is random and nothing is recorded) before PutKvStream is sent, and appends 'Upload handle: {id}. Open Uploads to inspect or resume prepared content.' to every error from map_mutation_error once upload_id is Some. The agent records the handle in claim_upload -> prepare_kv_upload -> register_upload_intent, and several refusals precede that point: prepare_kv_upload's first statement is self.profile.require(Capability::Kv) (the capability-denied branch map_mutation_error rewrites), the agent's header size refusal, profile_work admission and worker-pool saturation in handle_streaming_upload, path validation inside put_kv_file_upload_with_size, and any transport loss before the header frame is consumed (the upload_ready handshake marks registration, and the desktop's error path does not record whether it completed). In each case ListKvUploads has no entry for the handle, the Uploads panel shows 'No recorded uploads', and the message promises a recovery that does not exist. The existing EarlyUploadLoss test asserts the code but not the message.

Evidence:

- [`apps/desktop/src-tauri/src/commands/vault.rs:1020`](../../apps/desktop/src-tauri/src/commands/vault.rs#L1020): handle suffix appended to every mapped error once an id was issued (982-1032); ensure_upload_identity at 988
- [`crates/foks-desktop/src/lib.rs:3214`](../../crates/foks-desktop/src/lib.rs#L3214): ensure_upload_identity: local issuance only, nothing recorded
- [`crates/foks-client-app/src/kv/uploads.rs:118`](../../crates/foks-client-app/src/kv/uploads.rs#L118): prepare_kv_upload: require(Capability::Kv) before register_upload_intent (110-153)
- [`crates/foks-agent/src/main.rs:1583`](../../crates/foks-agent/src/main.rs#L1583): size, profile admission and pool refusals before the worker starts (1577-1640); upload_ready handshake at 1526-1528 and 1643-1644
- [`apps/desktop/src-tauri/src/commands/execution.rs:68`](../../apps/desktop/src-tauri/src/commands/execution.rs#L68): map_mutation_error capability-denied/kv branch returns before any stream write
- [`apps/desktop/src-tauri/src/commands/tests/vault.rs:1115`](../../apps/desktop/src-tauri/src/commands/tests/vault.rs#L1115): EarlyUploadLoss asserts code agent-lost, not the message

Recommendation: Append the handle only for errors that can follow registration: ambiguous errors (deadline, cancelled), protocol or conflict errors from the stream, or when the transport reports that upload_ready was received; leave capability-denied, invalid-request, busy and pre-header transport losses without the hint. Alternatively have the agent return the handle in a structured error field once register_upload_intent has run. Add a test asserting the capability-denied message carries no handle.

Verifier note: Both verifiers confirmed low, S, and settled the reviewer's unverified point in the candidate's favour: the capability check precedes registration, so the hint is wrong for at least that branch, and for several agent-side refusals besides.

### c3-desktop-uploads-panel-receipt-discarded

**FileUploadsPanel's 'Show saved receipt' runs a Bulk mutation that invalidates the store's catalog, discards the report it asked for, renders nothing new, and every row action resets pagination**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: desktop · Merged: `c-desktop-uploads-panel-receipt-discarded`

act() awaits resumeFileUpload or cancelFileUpload, discards the returned report, and renders listFileUploads(storeId, after); the per-row buttons call act(upload.uploadId) with no cursor, so the list returns to page one. For a completed upload the button label is 'Show saved receipt', but the row renders uploadId, label, chunk counts and reserved bytes and never nodeId or version, although the listing already carries them for verified and completed entries. Host side, change_file_upload takes the exclusive upload reconciliation reservation and calls invalidate_catalog_items before dispatch unconditionally, which drops the store's items from the host catalog, retires the in-flight load and raises the mutation epoch; nothing in the panel or checkedMutation refreshes the catalog afterwards, so until the next 30 s reconciliation job host-side resolution of the store's listed items fails with item-not-found. Agent side, a resume of a finalized upload with a verified receipt skips the network and returns the report, so there is no server write; the cost is a local catalog invalidation and a fresh reload for a button that displays nothing.

Evidence:

- [`apps/desktop/src/features/files/file-uploads.tsx:33`](../../apps/desktop/src/features/files/file-uploads.tsx#L33): act(): result of resume/cancel discarded, then listFileUploads(storeId, after) (33-53); row buttons pass no cursor (106-115); 'Show saved receipt' for completed (111-113); nodeId/version never rendered (99-104)
- [`apps/desktop/src/bridge/uploads.ts:16`](../../apps/desktop/src/bridge/uploads.ts#L16): decoder carries nodeId and version (16-17, 69-70)
- [`apps/desktop/src-tauri/src/commands/vault.rs:1995`](../../apps/desktop/src-tauri/src/commands/vault.rs#L1995): change_file_upload: begin_upload_reconciliation and invalidate_catalog_items before dispatch (1995-1999)
- [`apps/desktop/src-tauri/src/commands/context/catalog.rs:432`](../../apps/desktop/src-tauri/src/commands/context/catalog.rs#L432): invalidate_catalog_items drops the store's items and raises the mutation epoch
- [`crates/foks-client-app/src/kv/uploads.rs:416`](../../crates/foks-client-app/src/kv/uploads.rs#L416): resume of a finalized upload returns the verified receipt without network work (416-456)

Recommendation: Render nodeId and version on verified and completed rows from the listing and remove the 'Show saved receipt' resume for completed entries (or display the returned report in a status line); thread the current cursor through the row handlers so re-listing keeps the page; after a successful resume or cancel trigger the store's profile refresh as the drop-upload workflow does. Add a render test for the per-state buttons and cursor retention.

Verifier note: Both verifiers confirmed low, S. The reviewer's line numbers (450, 514, 517) do not match the 150-line file but the content does; one verifier added that the invalidation makes the store's items temporarily unreadable from the renderer's list.

### c3-kernel-integer-text-error-codes-differ

**The native scalar, native page and Wasm scalar decoders map the same non-canonical INTEGER text to three different kernel error codes (integer_range, bounds, storage)**

Priority: low · Type: maintainability · Origin: celld · Effort: S · Area: kernel · Merged: `c-kernel-scalar-vs-page-integer-error-code`

Three decoders of the kernel's CAST(x AS TEXT) integer convention classify one storage condition (an INTEGER column that is not canonical decimal text) differently: native/scalar.rs returns Storage for bad UTF-8, IntegerRange for a parse failure and Storage for non-canonical text; native.rs returns Bounds for all three (and for every unexpected cell shape); the Wasm scalar codec's invalid() maps through parse() to Storage. The reviewer's operator-impact argument does not hold: the condition is unreachable without file-level corruption (kv_dirent_current.creation_time is INTEGER NOT NULL in a STRICT table and the page SQL emits CAST(... AS TEXT), which always yields canonical decimal for an i64; other integer columns use typeof() guards), the Bounds -> ScanError::Allocation mapping in restricted_list/bootstrap.rs applies only to pool admission and workspace construction in the bootstrap scan, which never calls query_page, and the List operation treats every kernel error uniformly (stats.rejected/failed, no code), so native and Wasm reports do not disagree in anything an operator sees. What remains is error-code hygiene between decoders; the Bounds and IntegerRange choices are celld's original code.

Evidence:

- [`crates/foks-metadata-core/src/native/scalar.rs:92`](../../crates/foks-metadata-core/src/native/scalar.rs#L92): Storage / IntegerRange / Storage for bad UTF-8, parse failure, non-canonical (89-107)
- [`crates/foks-metadata-core/src/native.rs:163`](../../crates/foks-metadata-core/src/native.rs#L163): Bounds for all three (160-185)
- [`crates/foks-metadata-core/src/scalar/codec/decode.rs:193`](../../crates/foks-metadata-core/src/scalar/codec/decode.rs#L193): invalid() -> parse() -> Error::Storage (189-201, 381-395)
- [`crates/foks-server/src/restricted_list/output.rs:374`](../../crates/foks-server/src/restricted_list/output.rs#L374): List handles every kernel error uniformly, no code (372-383)
- [`crates/foks-server-db/src/schema/kv_current.sql:3`](../../crates/foks-server-db/src/schema/kv_current.sql#L3): creation_time INTEGER NOT NULL in a STRICT table; page_catalog.rs:241 emits CAST(c.creation_time AS TEXT)

Recommendation: In native.rs:163-167 and native/scalar.rs:92-95 map bad UTF-8, parse failure and non-canonical text for an INTEGER column to Error::Storage (the stored value, not a caller parameter, is at fault), leaving Bounds for length and admission refusals; adjust any test asserting IntegerRange on the scalar path.

Verifier note: Both verifiers confirmed low, S, as a maintainability item, and refuted the failure scenario and the operator-visible misclassification.

### c3-kernel-wasm-input-alloc-infallible

**Wasm input buffers are allocated infallibly while every other kernel buffer uses try_reserve, so an allocation failure traps the instance instead of returning 0**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: kernel · Merged: `c-kernel-wasm-input-alloc-infallible`

input_alloc uses `*b = vec![0; length as usize]` for up to MAX_INPUT_BYTES (2 MiB) and page_input_alloc and small_box_input_alloc delegate to it, while the page, response and scalar buffers go through try_reserve_exact and the list_charge fixture entry shows the intended fallible pattern (return 0 on failure). No custom allocator or max-memory link flag exists, so an allocation failure goes through handle_alloc_error to abort and an `unreachable` trap; an exception out of an export poisons the instance in the spike host (poisonAndDiscard), whereas a 0 return raises a non-poisoning 'input allocation' error. The doc on small_box_input_alloc promises that a rejected allocation invalidates the request and returns 0, which the delegated path does not honour. Impact is narrow: the module declares no maximum, so growth fails only at the engine or host limit (wasm_limits.py requires at most 64 MiB); the hosts cap input at MAX_INPUT before calling; in a near-limit regime the same request also allocates infallibly in wire::run (serde_json::from_slice) and the output Vec, so making input_alloc fallible alone does not make such a request fail soft; the Wasm module is loaded only by the qualification tools under tools/, not by any server crate. The reviewer's 'charge the input length' suggestion would be a no-op: page::charge is reserve_allocation, which accepts whenever no thread-local budget is installed, and none is installed at the idle phase in which input_alloc runs (page_input_alloc's charge is equally inert in the shipped module).

Evidence:

- [`crates/foks-metadata-core/src/wasm.rs:126`](../../crates/foks-metadata-core/src/wasm.rs#L126): input_alloc: vec![0; length] (117-129); page_input_alloc (73-88) and small_box_input_alloc (149-162) delegate to it
- [`crates/foks-metadata-core/src/wasm/list_charge.rs:48`](../../crates/foks-metadata-core/src/wasm/list_charge.rs#L48): fixture entry does it the fallible way: try_reserve_exact, return 0 on failure
- [`crates/foks-metadata-core/src/scalar/response.rs:1`](../../crates/foks-metadata-core/src/scalar/response.rs#L1): a trap retires the instance; response/page buffers use try_reserve_exact (249-292)
- [`crates/foks-metadata-core/src/page.rs:37`](../../crates/foks-metadata-core/src/page.rs#L37): charge = foks_snowpack::reserve_allocation, which accepts with no budget installed (snowpack allocation.rs:22-32)
- [`tools/foks-metadata-spike/host.mjs:201`](../../tools/foks-metadata-spike/host.mjs#L201): host caps input at MAX_INPUT and errors on pointer 0; an exception poisons the instance (94-99)

Recommendation: In input_alloc: `let mut v = Vec::new(); if v.try_reserve_exact(length as usize).is_err() { return 0; } v.resize(length as usize, 0);` and store it, as list_charge_input_alloc does. Drop the 'charge the input' part of the reviewer's recommendation, or note that it has no effect without an installed budget.

Verifier note: Both verifiers confirmed low, S, and narrowed it to a consistency fix rather than a reachable per-request failure: the hosts already handle a 0 return, no production path loads the module, and the code is byte-identical to celld's original.

### c3-kv-accounting-preport-schema-49-collision

**A database stamped schema 49 by the pre-port branch is misread as celld's 49 and fails its upgrade with a raw 'no such trigger' error instead of a SchemaVersion refusal**

Priority: low · Type: maintainability · Origin: port · Effort: S · Area: kv-accounting · Merged: `c-kv-accounting-preport-49-collision`

Before the port this branch's SCHEMA_VERSION was 49 (454cc8b) and step 49 was the waitlist dedupe/unique/count step; after the port 49 means celld's kv_accounting step and the waitlist step moved to 60. initialize() accepts 43..=59 as upgradeable and cannot tell the two shapes apart: a pre-port file carries user_version 49, waitlist_count and the unique email index, and no accounting columns or triggers. It skips kv_accounting.sql (version <= 48 is false), applies kv_external.sql (version <= 49), whose ALTER TABLE and CREATE INDEX statements succeed and whose `DROP TRIGGER kv_file_chunks_account_insert` has no IF EXISTS and fails with 'no such trigger'. Reproduced with SQLite 3.45.1 from the 224a6c4 schema files. The IMMEDIATE transaction rolls back, so the file is intact, but Database::open returns Error::Sql rather than Error::SchemaVersion and nothing names the cause; even with IF EXISTS the chain would fail later because the recreated triggers reference missing accounted_* columns and migrate_waitlist would drop an index the pre-port step already dropped. tests/schema.rs predecessor() models only celld's 49, and the foks-server-db README documents 43..59 as upgradeable without the caveat. Only developer and CI data directories created between 454cc8b and 224a6c4 are affected (the port commit says no deployed databases exist).

Evidence:

- [`crates/foks-server-db/src/schema.rs:104`](../../crates/foks-server-db/src/schema.rs#L104): 43..=59 accepted as upgradeable; kv_accounting only for <= 48, kv_external for <= 49 (172-177); migrate_waitlist unconditional (205)
- [`crates/foks-server-db/src/schema/kv_external.sql:21`](../../crates/foks-server-db/src/schema/kv_external.sql#L21): DROP TRIGGER kv_file_chunks_account_insert without IF EXISTS
- [`crates/foks-server-db/tests/schema.rs:143`](../../crates/foks-server-db/tests/schema.rs#L143): predecessor() models only celld's 49 shape
- [`crates/foks-server-db/README.md:28`](../../crates/foks-server-db/README.md#L28): documents 43..59 as upgradeable without the caveat

Recommendation: In initialize(), when version == 49 probe `SELECT EXISTS(SELECT 1 FROM pragma_table_info('kv_namespaces') WHERE name='accounted_bytes')`; if false return Error::SchemaVersion { found: 49 } and say in the README that pre-port 49 stores must be recreated (or, if migration is wanted, run the <= 48 steps and skip migrate_waitlist for that shape). Add a tests/schema.rs fixture for the pre-port 49 shape asserting the refusal.

Verifier note: Both verifiers confirmed low, S, and both reproduced the failure from the 224a6c4 schema files. Origin: port (the renumbering introduced the collision).

### c3-kv-accounting-replace-unsafe-counters

**Accounting triggers are not REPLACE-safe except on kv_nodes and kv_file_uploads, and the current-projection test performs the exact drifting statement without asserting the counters**

Priority: low · Type: test · Origin: celld · Effort: S · Area: kv-accounting · Merged: `c-kv-accounting-replace-unsafe-counters`

With recursive_triggers off (the writer connection never sets it; only the restricted-list read connection turns it on), INSERT OR REPLACE deletes the conflicting row without firing DELETE triggers but fires the INSERT trigger, so the AFTER INSERT/AFTER DELETE accounting triggers on kv_dirent_heads, kv_directory_heads, kv_dirents, kv_directories, kv_roots, kv_locks and kv_file_chunks inflate their counters by one object and the old row's bytes per REPLACE; reproduced on HEAD's schema with SQLite 3.45.1 (dirent_count 2 against 1 head row; accounted_objects 4 against 3 dirent rows), exact with recursive_triggers on. BEFORE INSERT collision guards exist only for kv_nodes and kv_file_uploads. No production statement drifts today: the typed writers and the hosted catalog use plain INSERT or ON CONFLICT DO UPDATE. The gap is in the tests: kv_current_tests.rs:146-158 executes INSERT OR REPLACE INTO kv_dirent_heads onto an existing head and its invariant() checks only the projection and foreign keys, so dirent_count is left one too high unnoticed, and tests/kv_accounting.rs, whose assert_accounting would catch it, never uses REPLACE. The suite therefore neither proves REPLACE-safety nor documents its absence. A BEFORE INSERT guard on kv_dirent_heads is not an option because it would abort the production upserts (BEFORE INSERT runs before ON CONFLICT resolution, as the kv_usage_triggers.sql comment says for kv_nodes).

Evidence:

- [`crates/foks-server-db/src/schema/kv_accounting.sql:160`](../../crates/foks-server-db/src/schema/kv_accounting.sql#L160): kv_dirent_heads counters maintained by AFTER INSERT/AFTER DELETE only; same pattern 32-190
- [`crates/foks-server-db/src/schema/kv_usage_triggers.sql:7`](../../crates/foks-server-db/src/schema/kv_usage_triggers.sql#L7): collision guards only for kv_nodes and kv_file_uploads; comment on BEFORE INSERT and ON CONFLICT
- [`crates/foks-server-db/src/schema/kv_current.sql:25`](../../crates/foks-server-db/src/schema/kv_current.sql#L25): comment acknowledges REPLACE does not fire the implicit DELETE trigger with recursive_triggers off
- [`crates/foks-server-db/src/kv_current_tests.rs:152`](../../crates/foks-server-db/src/kv_current_tests.rs#L152): INSERT OR REPLACE INTO kv_dirent_heads with recursive_triggers off; invariant() (50-65) checks the projection only
- [`crates/foks-server-db/src/connection.rs:521`](../../crates/foks-server-db/src/connection.rs#L521): writer configure() never sets recursive_triggers; only restricted_list/bootstrap.rs:484 sets it on a read connection
- [`crates/foks-metadata-core/src/catalog.rs:255`](../../crates/foks-metadata-core/src/catalog.rs#L255): SetHead and kv.rs:1595 use ON CONFLICT DO UPDATE

Recommendation: Extend kv_current_tests.rs invariant() to assert kv_namespaces.dirent_count == count(*) FROM kv_dirent_heads (the existing REPLACE test then documents the drift; change it to UPDATE or ON CONFLICT DO UPDATE) and add a REPLACE case per accounted table to tests/kv_accounting.rs against assert_accounting. To remove the hazard outright, set PRAGMA recursive_triggers=ON on the writer connection (one line; the kv_current triggers are written to be correct in either mode); guards are only an option for tables with no upsert path.

Verifier note: Both verifiers confirmed low, S, and both reproduced the drift. Correction applied to the reviewer's recommendation: collision guards cannot be added to kv_dirent_heads without breaking SetHead.

### c3-session-response-write-remaining-deadline

**Ordinary response writes are bounded by the remaining request deadline, so a completed chunk download whose store fetch consumed most of the deadline cannot be delivered to a slow client and is repeated from scratch**

Priority: low · Type: perf · Origin: celld · Effort: S · Area: session · Merged: `c-session-response-write-remaining-deadline`

Before the port write_response always had the full io_timeout (15 s) for the response. celld's single request deadline charges the write against whatever remains after body read, handler and store transfer: ordinary routes write under phase_deadline(deadline, io_timeout) = min(remaining, 15 s) and only long polls get a fresh io_timeout. When the write times out the connection is closed after the expensive work completed, the output buffer already charged to the output pool is discarded, and a client retry repeats the fetch. With default configuration the scenario is narrower than the reviewer stated: a remote object read is bounded by min(request deadline, configured chunk-store timeout), the store timeout defaults to 10 s (range 1-60 s), so a GetEncryptedChunk fetch leaves at least about 20 s of the 30 s deadline and the write still gets its full 15 s. The regression appears when timeout_seconds is configured above 15 s, or when execution queueing plus handler time plus store time exceed 15 s. It also lengthens the bulk-permit hold (c3-session-bulk-pool-four-permits) up to the same 15 s bound.

Evidence:

- [`crates/foks-server/src/net/session.rs:645`](../../crates/foks-server/src/net/session.rs#L645): response_deadline is the request deadline unless the route is a long poll; write at 683-693
- [`crates/foks-hosted-transport/src/framing.rs:86`](../../crates/foks-hosted-transport/src/framing.rs#L86): phase_deadline = min(remaining request deadline, maximum); write_response_until fails with a timed-out I/O error
- [`crates/foks-server/src/chunks.rs:606`](../../crates/foks-server/src/chunks.rs#L606): remote object read bounded by min(request deadline, configured timeout); timeout default 10 s, range 1-60 s (54-56, 84)
- [`crates/foks-server/src/config.rs:88`](../../crates/foks-server/src/config.rs#L88): io_timeout 15 s, request_timeout 30 s; pre-port write_response(.., limits.io_timeout) at 224a6c4 session.rs:520

Recommendation: For a completed ordinary request give the response write max(remaining deadline, io_timeout), or a fresh io_timeout as long polls get, keeping the request deadline for body read, handler and queued writes. Add a framing or session unit test that a response write started with one second left still gets the io_timeout allowance.

Verifier note: Both verifiers confirmed low, S. Both narrowed the scenario: with the default 10 s store timeout the write allowance is not shorter than pre-port; the loss needs a store timeout configured above 15 s or time lost earlier in the request. Whether the client actually retries a chunk download after a closed connection was not traced.

### c3-storage-contract-capture-spool-in-tempdir

**Capture and import spool up to 64 GiB of private database copies into the system temp directory with no free-space check or location control, and hash the data about nine times**

Priority: low · Type: perf · Origin: celld · Effort: M · Area: storage-contract · Merged: `c-storage-contract-capture-spool-in-tempdir`

Package::capture spools its private copy into tempfile::Builder::tempdir_in(std::env::temp_dir()) and import_personal_list creates a second temp directory there for the target database, which copy_all fills row by row, so two full-size private copies exist at once. CaptureLimits carries only maximum_database_bytes and the default equals MAX_DATABASE = 64 GiB; there is no statvfs or free-space check anywhere in foks-server and no caller-supplied spool directory (std::env::temp_dir honours TMPDIR, so the environment is the only control). The source is read three times by capture (inventory before, during and after the copy, a deliberate change-detection check), and the private copy is re-hashed in full by Package::verify at capture end, in finish(), at import start, twice in copy_keys and in key_bundle_digest: about nine database-sized hash passes per import plus two copies. On a host whose /tmp is a 16 GiB tmpfs, a 20 GiB finalized backup is hashed, then the copy fails with ENOSPC after filling tmpfs and pressuring RAM. The failure is closed (I/O error) but arrives after tens of GiB of work. restricted_storage is compiled into every unix build, but RestrictedScanner is constructed only by restricted_list/output.rs (celld-proof) and tests, so this is a diagnostic-path gap.

Evidence:

- [`crates/foks-server/src/restricted_storage/capture.rs:283`](../../crates/foks-server/src/restricted_storage/capture.rs#L283): tempdir_in(std::env::temp_dir()); three inventory passes (290-298); verify at capture end (313), in copy_keys (432, 456), in key_bundle_digest (363); verify re-inventories the whole private directory (462-468)
- [`crates/foks-server/src/restricted_storage/import/mod.rs:222`](../../crates/foks-server/src/restricted_storage/import/mod.rs#L222): second temp directory for the import target; verify at 216; copy_all at 288
- [`crates/foks-server/src/restricted_storage/mod.rs:24`](../../crates/foks-server/src/restricted_storage/mod.rs#L24): MAX_DATABASE = 64 GiB; CaptureLimits has only maximum_database_bytes (84-93); finish() verifies again (257)
- [`crates/foks-server/src/lib.rs:41`](../../crates/foks-server/src/lib.rs#L41): pub mod restricted_storage in every unix build; only restricted_list/output.rs:191 constructs RestrictedScanner outside tests

Recommendation: Add an optional spool directory to CaptureLimits (default temp_dir, documented as TMPDIR-controlled) and pass it to both tempdir_in sites; before the copying inventory pass compare the summed entry lengths (doubled when an import will follow) against statvfs f_bavail * f_frsize of the spool and fail closed with Unsupported('spool free space'). Keep the three-pass change detection; drop the Package::verify calls that are not separated by a trust boundary. Optionally gate restricted_storage behind the celld-proof feature like import.

Verifier note: Both verifiers confirmed low, M. Corrections applied: the read count is understated (about nine passes, mostly over the private copy rather than the source); the reviewer's 'hash once while copying' would remove the deliberate before/copied/after equality check that tests.rs:556 relies on, so only the spool-directory and free-space parts of the fix are safe; TMPDIR is an existing mitigation.

### c3-storage-contract-interrupt-masks-cancellation

**A cancellation or deadline expiry observed inside the SQLite progress hook surfaces as Sql{9} and then as Unsupported('SQLite operation/deadline limit'), never as ScanError::Cancelled or Deadline**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: storage-contract · Merged: `c-storage-contract-interrupt-masks-cancellation-class`

Control::progress latches `interrupted` whenever check() fails, which includes cancellation and deadline expiry as well as the opcode budget, and returns true; SQLite then aborts the running step with SQLITE_INTERRUPT, which From<rusqlite::Error> turns into ScanError::Sql { extended_code: Some(9) }, the error advance() and import_personal_list return (advance() then latches `failed`, so later calls return Incomplete). Every later Control::check() or statement() returns Unsupported('SQLite operation/deadline limit') because check() tests `interrupted` before re-evaluating the inner ScanControl. So a cancellation inside a statement surfaces as Sql{9} or Unsupported, never Cancelled or Deadline, and a pure opcode-budget stop is indistinguishable from a cancelled job. The existing tests set the flag between statements, where intersect()/check() run first, so they see Cancelled and do not cover the in-hook path. For the scanner the window is one LIMIT 1 probe; for import it is the whole integrity_check or foreign_key_check. No caller in restricted_list branches on the Cancelled or Deadline variants, so the impact today is on diagnostics and reports.

Evidence:

- [`crates/foks-server/src/restricted_storage/mod.rs:171`](../../crates/foks-server/src/restricted_storage/mod.rs#L171): progress latches on budget or check().is_err() (170-177); check() returns Unsupported first (149-157); rusqlite error -> Sql{extended_code} (52-59); advance latches failed (226-245)
- [`crates/foks-server/src/restricted_storage/tests.rs:392`](../../crates/foks-server/src/restricted_storage/tests.rs#L392): cancellation tests set the flag between statements only (392-415; import/tests.rs:354-372, 492-520)

Recommendation: Track the stop reason in Control: keep `interrupted` for the opcode budget and add a second flag (or store the inner ScanError kind) set when the inner control check fails in progress(). In check(), when either flag is set, re-run the inner ScanControl check first and return its Cancelled/Deadline error, falling back to Unsupported only for the budget flag. Map rusqlite OperationInterrupted (extended code 9) through the Control at the From boundary or in a helper used by scan and import so the first surfaced error is also classified. Add a test that flips `cancelled` from a progress callback and asserts Cancelled.

Verifier note: Both verifiers confirmed low, S, and corrected the first error class (Sql{9}, reproduced in the opcode-budget experiment) rather than Unsupported.

### c3-storage-contract-qualify-tool-schema59-pins

**tools/foks-sqlite-policy still pins the schema-59 trusted source (foks-59.json) while the generator now emits foks-60.json, so the celld SQLite policy qualification cannot be run or regenerated for schema 60**

Priority: low · Type: maintainability · Origin: port · Effort: M · Area: storage-contract · Merged: `c-storage-contract-qualify-tool-schema59-pins`, `c-tests-sqlite-policy-schema59-pin`

origin/celld had SCHEMA_VERSION 59 and its generator wrote foks-59.json; the port bumped to 60, renamed the generator output to foks-60.json with FOKS_60_HASH, and updated scan.rs, generated.rs and objects.rs, but tools/foks-sqlite-policy/qualify.py still pins 'foks-59.json' (PINS, line 14) and indexes source['foks-59.json'] at three sites, and run-local.py keys the fixture provenance check on profile['foks-59.json']. Running the generator at HEAD (export_trusted_profile_sources) produces authority-v1.json, celld-0.6.2.json, scalar-catalog.json and page-catalog.json with hashes matching the pins, and foks-60.json (sha256 93cdfbb3b0c952c490c4b67b3c97a4e73301aa984846aec26c6689278d8db5fe), which the tool never looks for; artifacts() iterates PINS and pinned() raises the artifact gate for the missing file, so neither script can run against a HEAD export and the fixture provenance check can never be satisfied. test_qualify.py checks only the connection-policy pin, so nothing catches it. A hand-renamed schema-59 corpus would qualify the wrong schema: version 60 changed the waitlist objects and policy.rs TABLES already lists them. The tool is an offline diagnostic, not wired into CI or docs; the effect is that the recorded celld SQLite policy qualification covers schema 59 only.

Evidence:

- [`tools/foks-sqlite-policy/qualify.py:14`](../../tools/foks-sqlite-policy/qualify.py#L14): PINS['foks-59.json']; artifacts()/pinned() gate at 68-79; source['foks-59.json'] at 114, 128, 357
- [`tools/foks-sqlite-policy/run-local.py:38`](../../tools/foks-sqlite-policy/run-local.py#L38): fixture provenance check keyed on profile['foks-59.json']
- [`crates/foks-metadata-core/src/read_schema/tests/generator.rs:125`](../../crates/foks-metadata-core/src/read_schema/tests/generator.rs#L125): export writes foks-60.json; asserts SCHEMA_VERSION == 60 at 82
- [`tools/foks-sqlite-policy/test_qualify.py:68`](../../tools/foks-sqlite-policy/test_qualify.py#L68): only the connection-policy pin is checked

Recommendation: Rename the pin to foks-60.json with the exported sha256, update the three source[] sites and run-local.py, regenerate the fixture and report pins that embed PINS by rerunning the pinned celld SQLite qualification, and add a test_qualify.py case that reads SCHEMA_VERSION from crates/foks-server-db/src/schema.rs and asserts f'foks-{version}.json' in PINS (or derive the key from that constant in both places).

Verifier note: Both verifiers confirmed low (the merged tests candidate had medium; the tool fails loudly and nothing at runtime reads the pins); one ran the generator to confirm the file name and hash. Effort settled at M because the derived fixture and report hashes must be regenerated with the pinned celld SQLite build, not only the pin renamed. Origin: port.

### c3-tests-celld-guarded-lab-paths

**tools/foks-celld-guarded tests and fixtures are bound to one macOS lab machine's paths and devices, so a run anywhere else is red instead of skipped**

Priority: low · Type: maintainability · Origin: celld · Effort: S · Area: tests · Merged: `c-tests-celld-guarded-lab-machine-paths`

The guarded runtime tooling (five patch files of about 38,000 lines, prepare-runtime.py at 5,000 lines, run-node.py at 1,600, 6,500 lines of tests) can be exercised only on the lab host: node.lock.json pins /private/tmp/foks-celld-lab/venv/bin/python and /opt/homebrew dylib paths as the reviewed roster, prepare-runtime.py refuses unless /dev/null is the Darwin character device 3:2, run-node.py requires darwin and arm64, and the Cargo graph needs gitignored source. On Linux `python3 -m unittest test_run_node test_prepare_runtime test_check_graph` gives 239 tests, 13 failures, 21 errors, 6 skipped in 183 s (FileNotFoundError on the venv path, the Darwin device check, cascading ActualBoundedFixtures cleanup assertions); only six tests carry a platform skip. The lockfile carve-out in scripts/test_dependency_audit.py is deliberate and documented. Nothing portable detects bit-rot: a Homebrew Python patch upgrade on the lab machine changes every roster path and is discovered only by a failed qualification attempt, and toolchain or patch drift is invisible to every reviewer not on that machine.

Evidence:

- [`tools/foks-celld-guarded/node.lock.json:112`](../../tools/foks-celld-guarded/node.lock.json#L112): absolute /private/tmp/foks-celld-lab and /opt/homebrew paths as the reviewed roster (105-125)
- [`tools/foks-celld-guarded/prepare-runtime.py:4052`](../../tools/foks-celld-guarded/prepare-runtime.py#L4052): verify_platform_null requires Darwin character device 3:2
- [`tools/foks-celld-guarded/run-node.py:90`](../../tools/foks-celld-guarded/run-node.py#L90): requires sys.platform == 'darwin' and arm64
- [`tools/foks-celld-guarded/test_prepare_runtime.py:3679`](../../tools/foks-celld-guarded/test_prepare_runtime.py#L3679): the only platform skip guard
- [`scripts/test_dependency_audit.py:23`](../../scripts/test_dependency_audit.py#L23): lockfile excluded from cargo-deny and Dependabot, documented

Recommendation: Add a module-level guard in test_run_node.py and test_prepare_runtime.py (unittest.SkipTest or skipUnless on sys.platform == 'darwin' and the lab root existing, with the reason naming the lab root) so a Linux run reports skips, and a README line in tools/foks-celld-guarded stating the suite is lab-only. Parameterizing the roster root through an environment-provided manifest so the pure logic tests run anywhere is the M-sized alternative.

Verifier note: Both verifiers reproduced the Linux failures and confirmed low, S; the 239/13/21 counts cover all three test files (178/13/20 for the two fixture-bound files alone). The suite does not import with `python3 -I` because of sibling-module imports.

### c3-tests-recovery-harness-never-scheduled

**The recovery harness that proves acknowledged writes survive crashes and that stale backups are rejected is #[ignore]d everywhere, including the nightly server-full job that already builds the server binary**

Priority: low · Type: test · Origin: celld · Effort: S · Area: tests · Merged: `c-tests-recovery-harness-never-scheduled`

The four recovery_harness tests (acknowledged_writes_survive_process_crashes_with_bounded_authenticated_recovery, restoring_stale_kv_backup_is_rejected_by_existing_client_anchors, restoring_stale_identity_backup_is_rejected_by_existing_merkle_pin and one more) and the four volume-loss experiments are ignored with 'requires a freshly built FOKS_SERVER_BIN; run per RECOVERY.md', and no workflow or script sets FOKS_SERVER_BIN for them (only test-client-server.sh and test-small-team.sh use that variable, for binary_release). These are the behaviours celld's external chunk store, chunk ledger and backup finalization most directly affect, and none of the port's CI changes schedule them. Backup and restart behaviour is not wholly unverified: the nightly server-full job runs test-small-team.sh, which executes team_restart_recovery and backup_restore; what is missing is the stale-backup rejection and crash-recovery-budget experiments against the built binary. RECOVERY.md frames the harness as a local timing experiment with a 15 s budget to run serially, so it was ignored by design. All eight ignored tests run serially against target/debug/foks-server on this 4-CPU container in 71 s and pass, including the two 'expected counterexample' volume tests, which assert the counterexample and so pass as written.

Evidence:

- [`crates/foks-server-testkit/tests/recovery_harness.rs:34`](../../crates/foks-server-testkit/tests/recovery_harness.rs#L34): #[ignore = "requires a freshly built FOKS_SERVER_BIN; run per RECOVERY.md"] on all four tests
- [`crates/foks-server-testkit/tests/recovery_harness/volume.rs:147`](../../crates/foks-server-testkit/tests/recovery_harness/volume.rs#L147): four more ignored volume-loss experiments; counterexamples at 160 and 186 assert the counterexample
- [`crates/foks-server-testkit/RECOVERY.md:10`](../../crates/foks-server-testkit/RECOVERY.md#L10): documents the manual invocation; no workflow step runs it
- [`.github/workflows/foks-standalone.yml:170`](../../.github/workflows/foks-standalone.yml#L170): server-full builds a release server binary through tools/foks-server/test-client-server.sh (6-12) but never runs the harness
- [`tools/foks-server/test-small-team.sh:7`](../../tools/foks-server/test-small-team.sh#L7): nightly runs team_restart_recovery and backup_restore

Recommendation: Add one step to server-full after 'Test release client/server lifecycle': `FOKS_SERVER_BIN=$PWD/target/release/foks-server cargo test --offline --locked -p foks-server-testkit --test recovery_harness -- --ignored --test-threads=1` (optionally as tools/foks-server/test-recovery.sh mirroring test-client-server.sh), raising FOKS_RECOVERY_BUDGET_MS if the runner needs it; expect one to two minutes.

Verifier note: Both verifiers confirmed low, S; one ran all eight tests (71 s, all pass), so the reviewer's recommendation to exclude the volume experiments is unnecessary and the release binary the job already builds can be reused.

### c3-tests-tool-suites-not-in-ci

**The Python and Node test suites of the qualification tools are not run by any workflow, and most of the artifact-dependent files fail at import rather than skip**

Priority: low · Type: test · Origin: celld · Effort: S · Area: tests · Merged: `c-tests-tool-suites-not-in-ci`

celld added roughly 2,000 lines of Node tests and 7,000 lines of Python tests under tools/, but no workflow or npm script executes them: foks-standalone.yml's path filter lists only tools/foks-client, foks-protocol-sync, foks-server and foks-v019-oracle, foks-dependencies.yml touches only the foks-hosted-transport-check lockfile, and there is no package.json under tools/. Run from a clean checkout (node 22, python 3.11, no lab artifacts, no boto3): foks-sqlite-policy (10 node, 13 python with 4 skipped), foks-schema-cookie-probe node (25), foks-celld-proof python (5) and foks-hosted-transport-check python (11) pass; foks-metadata-spike fails 7 of 9 node files at import ('reviewed directory artifacts required') and one python class (KeyError on FOKS_SCHEMA_ENUMERATION_VECTORS in setUpClass); foks-celld-proof fails 2 of its 3 node files (FOKS_PROOF_VECTORS throw); foks-schema-cookie-probe's test_controller errors because exec_module(run-local.py) imports boto3 at module level. The failures are unconditional throws or os.environ[] at module scope, so a plain run is red regardless of code health, and the pure tests that do pass (host.mjs, durability.mjs, deadline-worker, worker.mjs, probe/configuration/connection-state, check_graph) guard code the qualification pipeline uses and would be cheap to run.

Evidence:

- [`.github/workflows/foks-standalone.yml:10`](../../.github/workflows/foks-standalone.yml#L10): path filter and steps never reference the tools directories (10-17, 121-125); foks-dependencies.yml:63-67 only cargo-deny on one lockfile
- [`tools/foks-metadata-spike/directory.test.mjs:12`](../../tools/foks-metadata-spike/directory.test.mjs#L12): throws 'reviewed directory artifacts required' at import; same pattern in page-codec, read-token-host, scalar-codec, schema-enumeration, directory-fixture, list-charge
- [`tools/foks-metadata-spike/test_schema_fixtures.py:6`](../../tools/foks-metadata-spike/test_schema_fixtures.py#L6): setUpClass reads os.environ[...] and raises KeyError instead of skipping
- [`tools/foks-celld-proof/authority.test.mjs:9`](../../tools/foks-celld-proof/authority.test.mjs#L9): throws without FOKS_PROOF_VECTORS; node-decoder.test.mjs:9 likewise
- [`tools/foks-schema-cookie-probe/test_controller.py:9`](../../tools/foks-schema-cookie-probe/test_controller.py#L9): exec_module(run-local.py) imports boto3 at module level (run-local.py:15)

Recommendation: Add a tools job (or a tools-tests.yml on tools/** changes) running the artifact-free files: `node --test` on durability, host, deadline-worker, the cookie-probe and sqlite-policy node tests, and `python3 -m unittest` on test_qualify, test_check_graph, clock_guard_test, wasm_limits_test and test_directory_fixtures. Convert the artifact-dependent files to `test.skip(...)` / `@unittest.skipUnless(os.environ.get(VAR), ...)` naming the variable, and move `import boto3` in tools/foks-schema-cookie-probe/run-local.py into the function that uses it.

Verifier note: Both verifiers reproduced the per-file results; one kept medium, the other lowered to low because this is qualification tooling, not shipped code, and the failing files fail deterministically with a clear message rather than masking a regression. The synthesis adopts low. Correction applied: foks-celld-proof has 3 node test files, not 7 (the reviewer counted Python files too).

### c3-tests-wasm-kernel-never-built

**The wasm32 build of foks-metadata-core, the artifact celld runs, is never compiled in CI**

Priority: low · Type: test · Origin: celld · Effort: S · Area: tests · Merged: `c-tests-wasm-kernel-never-built`

The native tests exercise the kernel through rusqlite, but the wasm32-unknown-unknown cdylib that tools/foks-metadata-spike, foks-celld-proof and the celld runtime load is built only on the lab machine: rust-toolchain.toml lists no wasm32 target and no script or workflow invokes cargo build --target wasm32-unknown-unknown (the only occurrences of the target string are string comparisons in tools/foks-hosted-transport-check/check_graph.py and its tests). .cargo/config.toml does anticipate the target (it sets the getrandom wasm_js cfg). Code under cfg(target_arch = "wasm32") (wasm.rs, list_charge.rs, read_token_wasm.rs, the wasm arm of allocation_trace.rs) can stop compiling, or acquire a dependency that does not build for wasm, without any CI signal. The build does succeed at HEAD: after rustup target add, `cargo build --locked --target wasm32-unknown-unknown -p foks-metadata-core --no-default-features` and the same with --features read-token-qualification,list-allocation-qualification both finish (about 45 s cold).

Evidence:

- [`rust-toolchain.toml:1`](../../rust-toolchain.toml#L1): no wasm32 target
- [`crates/foks-metadata-core/Cargo.toml:9`](../../crates/foks-metadata-core/Cargo.toml#L9): crate-type cdylib for the hosted kernel
- [`crates/foks-metadata-core/src/lib.rs:27`](../../crates/foks-metadata-core/src/lib.rs#L27): #[cfg(target_arch = "wasm32")] mod wasm; scalar module also wasm-gated
- [`.cargo/config.toml:17`](../../.cargo/config.toml#L17): sets --cfg getrandom_backend="wasm_js" for wasm32-unknown-unknown; no build step anywhere

Recommendation: Add `targets = ["wasm32-unknown-unknown"]` to rust-toolchain.toml (or `rustup target add` in the standalone workflow) and append `cargo build --offline --locked --target wasm32-unknown-unknown -p foks-metadata-core --no-default-features --features read-token-qualification,list-allocation-qualification` (and the base no-features build) to tools/foks-server/check.sh; both cost under a minute warm.

Verifier note: Both verifiers confirmed low, S; one installed the target and showed both builds succeed at HEAD, settling the reviewer's open question. They differ on whether to add the target to rust-toolchain.toml (forces it on every developer) or to the workflow only.

### c3-uploads-deadline-miss-backoff

**An upload-cleanup owner whose 50 ms turn expired before its first erasure is treated as blocked and backed off for 60 s; under load the cleanup of a cancelled spool stalls while its reserved bytes refuse new uploads**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: uploads · Merged: `c-uploads-deadline-miss-backoff`

In maintain_upload_state_until the only deadline check that exits without back-off is right after the receipt-time step (line 68). After that the turn opens the protected store, probes up to 131 candidates, reads the owner and loops over children, and cleanup_file_upload_records opens another HardStateStore before its first deadline check; if the deadline has passed there it returns Ok(()) with no write, so before == after, the metadata revision is unchanged and scanned is false, owner_progress is false, and line 269 inserts the owner into cursor.blocked for 60 s and clears cursor.active. For a single owner more_work is then false and the agent's schedule moves the profile to a 60 s cadence after two idle turns; `retention maintain` shares the cursor and the 50 ms turn, its early exit requires debt.actionable == 0, which a cancelled owner does not satisfy, so it spins with 10 ms sleeps until max turns and exits with 'Local cleanup is incomplete' although nothing is blocked. The agent takes its 50 ms deadline before profile_work admission, registry open, credentials open and the checked-session open, so the window is real; the repository's own test_support comment gives tests a 10 s turn because 50 ms can be missed under load. Magnitude is bounded: max_reserved_bytes defaults to 256 MiB, so the largest default spool is about 64 chunks (about 8 passes), not the 256-chunk spool of the reviewer's scenario; an hour-scale stall needs the deadline to land in the window on most turns, which was not measured.

Evidence:

- [`crates/foks-client-app/src/upload_maintenance.rs:269`](../../crates/foks-client-app/src/upload_maintenance.rs#L269): !owner_progress || report.blocked -> cursor.blocked for 60 s, cursor.active cleared (262-277); only back-off-free early exit at 66-71
- [`crates/foks-client/src/kv/uploads.rs:286`](../../crates/foks-client/src/kv/uploads.rs#L286): cleanup_file_upload_records opens a fresh HardStateStore then returns Ok(()) untouched when the deadline has passed (276-291)
- [`crates/foks-agent/src/retention.rs:204`](../../crates/foks-agent/src/retention.rs#L204): the 50 ms deadline is taken before admission and the opens (204-230); two idle turns then +60 s (64-70)
- [`crates/foks-cli/src/retention.rs:145`](../../crates/foks-cli/src/retention.rs#L145): maintain loop shares the cursor and turn; blocked exit requires actionable == 0; 'incomplete' error at 180-183
- [`crates/foks-client-db/src/repositories/uploads.rs:12`](../../crates/foks-client-db/src/repositories/uploads.rs#L12): defaults 256 MiB reserved / 16 pending
- [`crates/foks-client-app/src/test_support.rs:5`](../../crates/foks-client-app/src/test_support.rs#L5): tests use a 10 s turn because 50 ms can be missed under load

Recommendation: Have cleanup_file_upload_records (or its caller) report whether it returned because the deadline expired before any erasure attempt, and in that case keep cursor.active, skip the blocked insert and set more_work. Take the agent's 50 ms deadline after admission and session open (see c3-deadlines-retention-50ms-covers-opens), or guarantee one record removal per turn. Add a test that calls maintain_upload_state_until with a deadline that expires after selection and asserts the owner is not in cursor.blocked.

Verifier note: Both verifiers confirmed the misclassification by trace and agreed on S; one rated low after correcting the magnitude (the default spool cap bounds the stall to about 8 passes and the timing is unmeasured), the other medium. The synthesis adopts low: the structural defect is real but the effect is delayed cleanup, not loss, and the window and magnitude are smaller than claimed.

### c3-uploads-rejected-child-dead-publishing

**A publication rejected during crash recovery is never retried: the upload stays reported as 'publishing' and Resume refuses forever, leaving Cancel and a full re-upload as the only exit**

Priority: low · Type: bug · Origin: celld · Effort: S · Area: uploads · Merged: `c-uploads-rejected-child-after-crash-dead-publishing`

In-process, mutate_namespace retries a stale precondition up to MAX_NAMESPACE_ATTEMPTS with a refreshed tree after calling retry_upload_namespace. The recovery path does not: resume_namespace_mutation on a Prepared child sends the stored Put once, and on KvStaleCache calls rejected() and returns the error without retry_upload_namespace or a refreshed attempt. On the next Resume, resume_file_upload sees the Rejected child and returns 'upload publication was rejected; original intent retained' before reaching the content-complete path, although the intent (a Create, or an overwrite whose selected entry and version are unchanged) is still satisfiable from the manifest and the content is already on the server. retry_upload_namespace (state 4 to 3, child cleared, requires child state 5) exists but nothing calls it outside mutate_namespace. The same terminal state follows a non-stale RemoteStatus rejection inside mutate_namespace. KvUploadReport maps state 4 to 'publishing' without looking at the child, so List shows 'publishing' indefinitely and Resume returns an opaque OperationBinding error; Cancel is permitted and costs a full re-upload.

Evidence:

- [`crates/foks-client/src/kv/write.rs:833`](../../crates/foks-client/src/kv/write.rs#L833): resume_namespace_mutation Prepared arm: rejected() then return, no retry (826-845); mutate_namespace retries after retry_upload_namespace (752-767); RemoteStatus arm leaves a Rejected child (768-773)
- [`crates/foks-client/src/kv/uploads.rs:715`](../../crates/foks-client/src/kv/uploads.rs#L715): resume_file_upload refuses forever once the child is Rejected; the content-complete path at 722-798 would re-validate
- [`crates/foks-client-db/src/repositories/uploads.rs:574`](../../crates/foks-client-db/src/repositories/uploads.rs#L574): retry_upload_namespace requires child state 5; cancel_upload accepts a Rejected child (438-462)
- [`crates/foks-client-app/src/kv/uploads.rs:46`](../../crates/foks-client-app/src/kv/uploads.rs#L46): KvUploadReport maps state 4 to 'publishing' regardless of the child

Recommendation: In resume_file_upload, when the child is Rejected, call hard.retry_upload_namespace(&id) and fall through to the existing content-complete path, which re-syncs and re-validates the manifest's selected entry and expected_version before link_uploaded_node_bound; limit the automatic retry to the stale-cache case or an explicit Resume, since a server-side non-stale rejection would usually fail again. Report Publishing with a Rejected child as a distinct state ('publication-rejected') in KvUploadReport.

Verifier note: Both verifiers confirmed low, S, as described. Related to c3-uploads-stuck-submission-unknown: once that finding gives SubmissionUnknown a terminating proof, this retry path is what reopens the upload.

## Refuted during verification

These candidates were dropped by both verifiers or by the deciding verifier on a split vote:

- `c-deadlines-rundue-ordinary-class`: RunDueJobs is classed Ordinary (15 s) although the CLI budgets it at 300 s; the agent cuts it to 15 s and the CLI's budget is unreachable. Refuted by the deciding verifier after a split (one refuted, one confirmed in corrected form on the same facts). `foks-rs jobs run-due` never sends Operation::RunDueJobs: it opens the ProfileSession in-process and calls run_due_jobs_with_federation under its own 300 s CommandScope (foks-cli main.rs:1378-1392), exactly as before the port, so the described mismatch and 'agent disconnected' outcome cannot occur. The only sender is the scripted foks-desktop-backend run-jobs (foks-desktop main.rs:1178), for which RunDueJobs is Ordinary with claim_limit 16; that residual is pre-existing sizing of a transcript tool, and its connection-close symptom belongs to c3-deadlines-expired-response-never-written. If the backend path matters, use run_next_due_job_with_federation in a loop bounded by the remaining budget or class RunDueJobs as Bulk.
- `c-tests-kex-admission-wall-clock`: kex_admission tests assert wall-clock bounds of 1-2 s while running inside the parallel testkit suite. Refuted by the deciding verifier (one refuted, one uncertain). cargo runs integration test binaries one after another, so the three tests share the machine only with each other, each against an in-process server; the 1 s window starts after TcpStream::connect and the 2 s shutdown window covers an event-driven watch cancel. Three verifiers ran the binary under 2x CPU oversubscription (busy loops on the 4 CPUs) and all runs passed with no bound tripped, the first CI run containing the file succeeded, and the testkit already carries tighter pre-port bounds (listener_boundaries.rs:97 at 250 ms). Raising the bounds to about 10 s is harmless hardening if a flake is ever observed.
- `c-security-import-key-copy-in-system-tempdir`: The personal-list import qualification copies the host key bundle into a directory under the system temp directory. Refuted by the deciding verifier (one refuted, one confirmed as a location preference). Nothing copied is a usable private key: copy_keys copies only the package's keys/ entries, which are key-encryption.key (XChaCha20-Poly1305 ciphertext under the operator root key) and purpose keys (ciphertext under that wrapping key with purpose/generation AAD); the operator root key lives at <installation>/operator-root.key outside keys/, is not part of a backup package, and reaches the import only as an in-memory Zeroizing value. The directory is 0700 with 0600 create_new O_NOFOLLOW files, TMPDIR relocates it, and the code is compiled only under cfg(any(test, feature = "celld-proof")) with the sole entry point feature-gated. Residual: the qualification scratch directory (including a database copy) sits under the system temp directory, so a tmp cleaner could interrupt a run (which fails closed) and a hard abort leaves ciphertext behind; optional hardening is to derive the scratch parent from the destination installation root. c3-storage-contract-capture-spool-in-tempdir covers the spool location.
- `c-kv-accounting-budgeted-get-inert`: budgeted_get's allocation charge is a no-op on every native server path; its comments describe admission that does not happen. Refuted by both verifiers. Routed requests run inside Request::sync (net/session.rs:478-487, request_memory.run at 856), which installs the hosted Request as the foks_snowpack AllocationBudget, and read-lease reads are synchronous on that thread, so budgeted_get in kv.rs and kv_objects.rs charges the request's Working pool; writer jobs capture crate::net::memory::current() at submission and re-enter it with resources.sync on the writer thread (writer.rs:58-62, 392). The default working pool is 512 MiB (validated minimum) and a 9 MiB chunk read charging about 72 MiB fits with the bulk cap of four. Only maintenance, backup, the capacity report and schema migration run without a budget, so the AllocationLimit -> Error::Capacity branch in schema/kv_usage.rs is unreachable in production; that is a stale comment, not an admission gap. Optional: one doc line in allocation.rs and a test of kv_list at the 4096-row limit under a working-pool-sized budget.
