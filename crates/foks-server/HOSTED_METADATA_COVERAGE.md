# Hosted metadata coverage and next slice

Read-only inventory, 2026-10-15, server baseline `2e9a7745` (schema55); U/UC and M0 remain in review/qualification. This is a proposed implementation boundary, not a claim that hosted authorization, fencing, failover or complete route coverage exists. No FOKS wire or cryptographic changes are proposed.

## Coverage boundary

The generated catalog has 128 `RouteId` entries. The executable family dispatcher is `src/net/session/handlers/mod.rs:24`; `RealTimeRtGetChannel` explicitly returns Unsupported. The three asynchronous poll paths are additionally dispatched by `src/net/session.rs`, so counting synchronous match arms alone misses behavior. Build a checked-in classification keyed by every `RouteId`, with tests asserting exact agreement with the generated catalog. Add HTTP/background entry points to a separate required list. Classifications must distinguish hosted implementation, dependency, and explicitly disabled; unknown routes must never fall through to a local database.

| Family / actual entry point | Metadata and side effects | Milestone |
| --- | --- | --- |
| KV: Mkdir, Put, PutRoot, FileUploadInit, FileUploadChunk, PutSmallFileOrSymlink, GetRoot, Get, GetNode, GetEncryptedChunk, GetDir, CacheCheck, List, LockAcquire, LockRelease, Usage | Complete 16-route family; namespace/root/dirent/node/chunk state, preconditions, key/role authorization, logical quota and physical publication ledger | M1 complete KV |
| Probe; Beacon; Merkle Lookup/MLookup, historical/current roots/hash/signed root, CheckKeyExists; SelectVHost and host/config discovery | Host routing, exact signed bootstrap/proof data and Merkle node reads; identity must match authoritative key/schema generation | M1 dependencies, broader lifecycle M2 |
| Registration ResolveUsername, LoadUserChain, GetClientCertChain, login/UID/subkey challenges and response consumption | Client discovery/recovery and credential issuance; apparently read-like challenge endpoints persist one-shot state and receipts | Required usable-client subset M1; signup, passphrase/recovery and remaining registration M2 |
| User LoadUserChain, GetPukForRole, PPE parcels, host config, generic chains, team list | Current user/device/subkey roles, shared-key generations and protected key boxes; data needed by genuine clients | M1 dependencies; provisioning/revocation, passphrases, generic writes, rename, grants and Yubi management M2 |
| TeamLoader challenge/activate/check token, chains/membership/removal and remote tokens; TeamAdmin/TeamMember/TeamGuest | Team KV depends on live bearer-token bindings, effective membership/role and team key generations; token creation/renewal is a write | Team-KV dependencies M1; create/edit/invite/removal/cert/federation lifecycle M2 |
| Identity capabilities/challenge/prove; OAuth init/poll/login; WebAdmin URL/CheckURL | Challenge consumption, SSO protected access/session transitions, admin grant/session claims | Fail closed when disabled in early slice; full support M2 |
| Realtime channel/send/read-through/inbox/delta/poll; KEX send/receive | Realtime durable rows and read reconciliation; KEX bounded ephemeral relay ownership and cancellation | M2; retain protocol poll waits |
| LogSend init/file/block; waitlist/invites | Bounded upload sessions/blocks, reservations and expiry | M2 |
| Management HTTP, web-admin HTTP, OIDC callback, manual/periodic jobs, CLI operations | Backup, restore, bootstrap/migrations, key rotation, policy changes, expiry, inventory/conversion/collection and checkpoints | Fenced control-plane jobs, never native-authority fallback |

## Hidden writes and authority dependencies

* Every authenticated request can enter `authorize_principal` (`src/net/session.rs:2128`). Its failed SSO read invokes `SsoService::ensure_access`; that method claims refresh state, performs provider I/O and persists a result (`src/sso/access.rs:30`). Disabling hosted SSO must reject enabled-policy tenants, not skip this guard.
* `Principal::authenticate` checks certificate bytes/public key against an active stored credential (`src/auth/principal.rs:23`). TLS trust alone is insufficient. KV then derives current user or team authority and role-key generations (`src/services/kv.rs:1071`), and rechecks the actual credential/token/generations before mutations (`:1216`). Extract these checks into the same authoritative transaction as the effect; do not accept the frontend's preflight role as authority.
* `kv_write_is_current` calls `ensure_kv_namespace`, which can create metadata. The existing native API performs this outside some inner mutation transactions (`server-db/src/kv.rs:128,291`). M0 exposes the difference: the extracted command rolls new-namespace creation back on later conflict. Production adoption must explicitly decide/bootstrap namespace creation and test both paths rather than silently assume equivalence.
* Realtime GetChangedThreads and PollInbox can run writer reconciliation (`src/services/realtime.rs:93,179`, `net/session/handlers/realtime.rs:150`). They cannot be classified as read-only based on RPC names.
* Startup opens the key provider, bootstraps/validates identity, initializes storage binding, and may block/disable SSO policy (`src/standalone.rs:820`). Writer startup also recovers publication attempts. A warm activation must not rerun unfenced initialization or regenerate a missing key.
* Maintenance deletes expired names, challenges, request receipts and tokens, then reclaims unreferenced upload chunks through persistent cursors (`server-db/src/maintenance.rs:94`, `maintenance/uploads.rs:25`). These are authoritative mutations, including reference/capacity triggers. KV locks expire according to the next acquirer's supplied timeout; do not introduce an unrelated absolute expiry sweep.
* Object publication is reserve → external I/O → settle/presence → metadata commit with current authorization (`src/services/kv.rs:493–590`). Timed-out physical effects still require settlement. Remote deletion remains report-only. Conversion, backup hydration and collection have separate DB/storage lease and identity rules; none may bypass the hosted ownership envelope.

An installed tenant requires coherent HostID, canonical services, signed probe/hostchain/Merkle material, account/device/subkey and team projections, certificate bindings, SSO policy state, storage namespace identity and key-generation manifest. Seven persisted key purposes exist (`src/keys/manifest.rs:6`): Host, Metadata, Merkle, ClientCa, DelegatedTls, Recovery, Capability. Keep signing/decryption in the native trusted adapter initially; bind all derived artifacts to authoritative generation/revision and intent, then revalidate before commit. Do not send private keys in the SQL/Wasm envelope. Cell ownership and native key-owning adapter epochs are separate fences.

## Smallest safe next vertical slice

1. Define a bounded private envelope and durable authoritative installation/tenant/namespace/schema/key-generation + adapter-owner epoch/revision record. Each read validates it atomically; each mutation reserves an intent sequence and canonical request/auth/version binding, performs current authorization and domain checks, updates revision and records the exact recoverable outcome in one transaction. Allocation high-water survives payload retirement. Old/missing allocated identities cannot execute again. Commit plus `storage.sync` gates publication; paused old adapters cannot commit or publish fresh key-derived results after takeover.
2. First qualify one existing, explicitly provisioned non-SSO personal tenant: native TCP/mTLS, discovery/current credential checks, small-node creation, PutDirents and authenticated Get/List through the shared kernel. Seed/bootstrap is a separately fenced import job, never ordinary per-request snapshot reconstruction. Use a real Rust and pinned Go client with a durable existing credential. This is an integration checkpoint, **not complete M1**; reject team/SSO/unimplemented routes.
3. Expand to all 16 KV routes and required client dependencies above, including root/directory initialization, full version-vector/cache checks, locks, large-file replay, external object reservation/settlement and team-token/key access. Team KV cannot rely on an unfenced local token issuer/renewal path; federated KV also requires its remote grant/token dependencies, or must remain explicitly unsupported. Only then label the stated M1 scope complete. Exercise stale auth between prepare and commit, lost reply plus adapter loss, successor outcome recovery, retired intent replay, and current/historical key compatibility.
4. Port M2 families and every control-plane writer under the same envelope. Enforce coverage mechanically. A preprovisioned KV demo cannot establish signup, key rotation, federation, backup/restore or production service coverage.

## Reusable extraction and bounded SQL work

Keep service wire decoding/status mapping and native TLS/key management at their current boundaries. Extract transaction-local domain functions, without nested `Connection` transactions, into modules for authority, namespace/root/directory/node, dirents/preconditions, chunks/ledger, locks/quota, and outcomes. The starting material is `server-db/src/kv.rs`, `kv_objects.rs`, `chunk_ledger.rs` and the matching `services/kv.rs` current-authority checks. Preserve trigger effects, signed bytes, replay-before-quota/error precedence, and expiry units. The `ReadSnapshot` APIs need equivalent same-revision query batches; a series of unrelated HTTP reads is not a snapshot.

The existing `kv_dirent_listing(uid,parent_id,name_mac,dirent_id,version)` indexes **history**, while current heads carry only identity/version (`server-db/src/schema/kv.sql:59–86`). NameConflict can therefore scale with unrelated heads or retained matching names. M0 now records separate history/head dimensions with the exact statement; results are not yet a production latency guarantee.

Propose a reviewed migration maintaining current-head projections for `name_mac`, live/tombstone state, and `creation_time`, either on heads or in a separate derived table. Populate and verify from the exact referenced historical row; update atomically on head insert/update/delete with bounded trigger work. Add nonunique live-head indexes `(uid,parent_id,name_mac,dirent_id)` and `(uid,parent_id,creation_time,dirent_id)`; query the projection then fetch exact historical bytes by its primary key. Do not introduce a new UNIQUE invariant without validating existing databases. Preserve MAC cursor `>` and time cursor `>=` behavior.

Historical `kv_dirent_at_name` is a different query (`server-db/src/kv.rs:1135`): consider `(uid,parent_id,name_mac,directory_version,version DESC)` after measuring plans. Do not replace it with current-head lookup. Keep all-version file-reference protection unless a separately reviewed retention policy deletes history. Tests must grow unrelated heads and same-name retained versions independently, check tombstones/renames/replay/cascades, and demonstrate targeted VM work is independent of unrelated data. Full version-vector traversal and whole-tree results still need bounded traversal/bytes/deadline admission; an index cannot make an inherently whole-tree response constant-cost.

## Next ownership split and gates

* Root: authoritative envelope, owner/epoch/revision/intent/outcome semantics and native request integration; decide namespace bootstrap transaction contract and key-provider fencing.
* Recovery agent: shared kernel/typed native+Wasm hosts, celld transactions/sync, crash/paused-owner/outcome-recovery harness and query-cost artifacts.
* Storage agent: authoritative KV extraction, current-head projection migration/query plans, chunk ledger and maintenance/backup participation; no remote GC enablement.
* Client/app agent: end-to-end Rust/Go usability/compatibility, deadline propagation from caller through native adapter, replay and recovery assertions.

Review the combined plan before code. Keep author plus independent review before every test/commit group. Gates include actual initialized/migrated schema parity, bounded parameters/results and SQL work, exact native/Wasm outputs/errors/counters, rollback on host/trap/COMMIT failure, old owner rejection, acknowledgement preservation and successful new authenticated work within the recovery budget. Local celld/Moto evidence remains distinct from real host/AZ/provider qualification.

## Qualification checkpoint through 235b8c95

The route inventory above is still a required implementation boundary. Upload
receipt retirement and native deadline slices are committed; current-head and
read-dependency projections, typed page/scalar catalogs, shared personal List,
strict context data and schema-enumeration qualifications are also committed.
These components do not yet constitute a hosted service route or complete M1/M2.

Connection activation now has an observed blocker: the tested celld binary permits
qualified TEMP creation and actual shadowing despite the source-level deny list.
See HOSTED_METADATA_CONNECTION_QUALIFICATION_PLAN.md. A same-connection TEMP and
attachment invariant, exclusive ownership and sealed SQL surface must be qualified
before readiness can be granted. Production owner/bootstrap choice, combined
memory/output lifetime, complete authoritative route coverage, authenticated
cross-host deadlines, successful <=15-second host/AZ recovery and tenant-density
qualification remain open. Remote S3 deletion remains report-only.
