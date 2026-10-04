# Client libraries

Area key `client-libs`. 12 verified findings. Part of the [foks-rs review](../README.md).

## Current state

The client libraries are large and mostly carefully engineered. foks-client (about 43.8k lines) implements the protocol workflows. foks-client-app (about 49k lines) handles profiles, vaults, checkpoints and orchestration. foks-client-db (about 15k lines) holds STRICT SQLite hard and soft state with trigger-enforced invariants. Three small crates cover the keystore, Yubi and OIDC. Several things work well: the journal-first mutation discipline, the file-handling hygiene (O_NOFOLLOW, fsync, inode checks), the rollback checkpoint, and inspection that compares the schema against a fresh reference. The main problems are structural and all local, so none of them touch Go v0.1.9 wire compatibility. (1) There is no operation-scoped context. The hard-state database is reopened at 176 production call sites, and `open` creates the file if it is missing. Session material and reentrancy live in thread-locals. The raw master key and the vault are threaded through 108 and 249 function signatures. (2) The error taxonomy is coarse and stringly typed. `InvalidAccount(&'static str)` covers 324 sites, so the agent classifies errors by walking downcast chains. (3) Software and Yubi entry points are duplicated (43 public `_yubi` methods), and the copies have drifted. (4) The durable state of a team rekey or member edit is split across the vault, the hard-state journal and the protected store. The reconciliation for it sits in client-app `runtime.rs` functions of up to 733 lines. (5) The hard-state migration is a set of ad-hoc version branches, one of which has no test. (6) Vault records holding raw seeds derive `Debug`. (7) The checked-session wrapper discards a committed result when checkpoint publication fails. (8) Crash-orphaned plaintext large-file stages have no production reclaim path. Lower-priority items: report types duplicated with foks-agent-proto, boilerplate in the KV API, integer state literals repeated in SQL, and OIDC discovery that ignores the client deadline. All async boundaries sit in the agent; the client crates are synchronous by design, and the only async code is the reqwest DNS resolver inside foks-oidc, which runs from blocking workers.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [Replace catch-all string errors with an exhaustive error class so the agent stops classifying by downcast](#client-libs-error-classification) | code-quality | high | M |
| [Give CheckedProfileSession an operation-scoped context instead of per-call DB opens, thread-local session material and raw master-key parameters](#client-libs-operation-scoped-context) | maintainability | high | L |
| [Collapse the _yubi / _with_credential / _as_local_team method matrix behind one acting-credential type](#client-libs-acting-credential) | code-quality | medium | L |
| [Checked-session wrapper reports a committed operation as failed when checkpoint publication fails](#client-libs-checked-session-result-loss) | correctness-risk | medium | S |
| [Replace ad-hoc hard-state version branches with a stepwise migration ladder and frozen historical fixtures](#client-libs-hard-state-migration-ladder) | maintainability | medium | M |
| [Define journal-state SQL encodings and terminal sets once instead of repeating integer literals](#client-libs-journal-state-encoding) | correctness-risk | medium | S |
| [Vault records with raw seeds derive Debug and rely on hand-written Drop zeroization](#client-libs-secret-record-redaction) | security | medium | S |
| [Wire orphaned large-file stage reclamation into exclusive session entry](#client-libs-soft-stage-reclaim) | correctness-risk | medium | S |
| [Move team rekey/member-edit/expulsion intent state machines out of client-app runtime into foks-client](#client-libs-team-intent-layering) | maintainability | medium | XL |
| [Stop duplicating foks-agent-proto DTOs as Serialize-only report types in client-app](#client-libs-agent-dto-duplication) | maintainability | low | M |
| [Unify personal/team KV methods behind a KvScope and a shared write-session prologue](#client-libs-kv-scope-api) | code-quality | low | M |
| [OIDC provider discovery ignores the FoksClient deadline and cancellation token](#client-libs-oidc-deadline) | correctness-risk | low | S |

### client-libs-error-classification

**Replace catch-all string errors with an exhaustive error class so the agent stops classifying by downcast**

- Type: code-quality
- Priority: high
- Effort: M
- Layers: client-lib, agent, desktop-ui
- Verification: adjusted

`foks_client_app::Error::InvalidAccount(&'static str)` has 324 production uses. It covers corrupted vault records, user-input validation (team-name rules) and workflow preconditions ("team has a pending membership mutation; resume it first") alike. foks-client has similar `&'static str` buckets, flattens ProtectedStoreError into a String, and reports clock failures as Transport. The agent's dispatch_error_response (about 280 lines) walks error source chains with downcasts across six crates. client_error_response then classifies some cases by matching literal message text (for example `InvalidAccount("host does not require SSO")` and `reason.starts_with("group names use")`). Everything else falls back to OperationFailed with the leaf message. As a result, the desktop cannot tell input errors, resumable pending work and integrity failures apart, a reworded message silently changes an IPC code, and new client errors are never forced through classification.

**Evidence**

- [`crates/foks-client-app/src/lib.rs:113`](../../../crates/foks-client-app/src/lib.rs#L113): `InvalidAccount(&'static str)` is the dominant variant (324 production uses in foks-client-app).
- [`crates/foks-client-app/src/team.rs:553`](../../../crates/foks-client-app/src/team.rs#L553): A precondition the user can act on (resume pending mutation) is raised as InvalidAccount; it is repeated at line 1044.
- [`crates/foks-client-app/src/team.rs:39`](../../../crates/foks-client-app/src/team.rs#L39): Team-name input validation is also InvalidAccount.
- [`crates/foks-client/src/error.rs:106`](../../../crates/foks-client/src/error.rs#L106): `ProtectedStore(String)` discards the typed ProtectedStoreError (filled via to_string() at mutation.rs:286).
- [`crates/foks-client/src/lib.rs:132`](../../../crates/foks-client/src/lib.rs#L132): System clock errors are reported as Error::Transport.
- [`crates/foks-agent/src/main.rs:1858`](../../../crates/foks-agent/src/main.rs#L1858): About 280 lines of downcast-based classification with wildcard arms; unmatched errors become OperationFailed with the leaf message.
- [`crates/foks-agent/src/main.rs:2224`](../../../crates/foks-agent/src/main.rs#L2224): client_error_response matches literal InvalidAccount messages ("host does not require SSO", "request is no longer in the pending inbox; refresh it", starts_with("group names use")) and AccountRequest/TeamRequest literals (2200-2215) to choose IPC codes.
- [`apps/desktop/src/operation-outcome.ts:28`](../../../apps/desktop/src/operation-outcome.ts#L28): failureOutcome maps any code it does not recognize to 'rejected', so classification quality directly affects how the UI reports outcomes.

**Recommendation**

Add `pub fn class(&self) -> ErrorClass` to foks_client_db::Error, foks_client::Error and foks_client_app::Error. Write each as an exhaustive match with no `_` arm, so a new variant fails to compile until it is classified; the From wrappers delegate to the inner error. Suggested classes: InvalidInput, PendingWork(ClientPendingKind), Conflict, NotFound, CapabilityDenied, TrustViolation, LocalIntegrity, Unavailable, Cancelled, DeadlineExceeded, CredentialsRequired, Internal. ClientPendingKind is defined in foks-client-app, not taken from foks-agent-proto's PendingOperationKind, and the agent maps between the two. Split InvalidAccount into InvalidInput(&'static str), PendingOperation { kind, alias } and CorruptRecord(&'static str), starting with the user-reachable sites in team.rs, kv.rs and account.rs. Replace the message-literal matches in client_error_response with typed variants. Map clock errors to a Clock variant and keep ProtectedStoreError typed. The agent then maps class() to ErrorCode in one match and keeps its existing special cases (chat, adapter retention, web admin) as overrides. Add any new ErrorCode values to the desktop's failureOutcome and error-presentation handling, which currently treats unknown codes as 'rejected'. This changes only local IPC codes, not the FOKS wire protocol. Add a unit test that classifies a representative value of each variant.

<details><summary>Verifier note</summary>

Confirmed. InvalidAccount(&'static str) is at lib.rs:113 and has 324 production uses (325 matches including the definition, excluding test modules). team.rs:40, 553 and 1044 raise input validation and a resumable precondition through it. foks-client's ProtectedStore(String) is at error.rs:106 and is filled via `error.to_string()` (mutation.rs:286). Clock failures map to Error::Transport (lib.rs:132-135). dispatch_error_response runs from main.rs:1858 to about 2140, downcasting foks_client_app, foks_client, foks_client_db, foks_rpc, foks_keystore and foks_verify errors, with `_ => {}`/`_ => None` arms and an OperationFailed fallback. No ErrorClass or class() exists in the client crates. The finding misses the strongest evidence. client_error_response (main.rs:2165-2255) classifies by matching literal message strings: `InvalidAccount("host does not require SSO")`, `InvalidAccount("request is no longer in the pending inbox; refresh it")`, `reason.starts_with("group names use")`, and the AccountRequest/TeamRequest literals. Rewording a message silently changes the IPC error code. One correction to the recommendation: PendingOperationKind lives in foks-agent-proto, which the client libraries should not depend on, so the class needs its own client-side kind enum that the agent maps. The desktop's failureOutcome (operation-outcome.ts) maps any unrecognized code to 'rejected', so new codes need explicit handling there. Not tracked in ISSUES.md.

</details>

### client-libs-operation-scoped-context

**Give CheckedProfileSession an operation-scoped context instead of per-call DB opens, thread-local session material and raw master-key parameters**

- Type: maintainability
- Priority: high
- Effort: L
- Layers: client-lib, agent
- Verification: adjusted

Every hard-state access reopens SQLite by path. HardStateStore::open creates the file if it is missing, canonicalizes the path, opens a connection, sets five pragmas and re-verifies the schema. There are 177 production call sites in foks-client and foks-client-app (272 including tests). Because open silently initializes a fresh database with a new database_id when the file is absent, read paths outside the profile lock create state as a side effect. Examples are the agent's ListProfileOverview and SSO handler, which call requires_import_verification. After a reset has removed the database and its keyring records, that recreates the database outside the lock and defeats the pristine-enrollment check at the next exclusive admission. Call sites guard against this inconsistently: merkle_maintenance uses try_exists and the reset preview uses exists(). Meanwhile the checked-session handle is only `&ProfileSession`. The master key and reentrancy bookkeeping ride in thread-locals. 108 functions take `master_key: &[u8; 32]` and 249 take `vault: &mut AccountVault`, so callers re-derive vault and mutation-store keys by hand.

**Evidence**

- [`crates/foks-client-db/src/repositories/metadata.rs:12`](../../../crates/foks-client-db/src/repositories/metadata.rs#L12): `open` does create_new, a symlink check, canonicalize, SQLite open, five pragmas and initialize_or_verify on every call.
- [`crates/foks-client-db/src/lib.rs:780`](../../../crates/foks-client-db/src/lib.rs#L780): A zero application_id/user_version database is initialized with a fresh random database_id and write_token.
- [`crates/foks-client/src/mutation.rs:151`](../../../crates/foks-client/src/mutation.rs#L151): MutationCoordinator holds `hard_database: &'a Path` (line 68); each journal transition (151-265) opens a new HardStateStore.
- [`crates/foks-client-app/src/checkpoint.rs:695`](../../../crates/foks-client-app/src/checkpoint.rs#L695): requires_import_verification calls HardStateStore::open, which creates and initializes the database if it is absent.
- [`crates/foks-agent/src/main.rs:3676`](../../../crates/foks-agent/src/main.rs#L3676): ListProfileOverview calls requires_import_verification on a plain ProfileSession outside any checked session or profile lock; crates/foks-agent/src/sso.rs:19 does the same.
- [`crates/foks-client-app/src/checkpoint.rs:1109`](../../../crates/foks-client-app/src/checkpoint.rs#L1109): Admission relies on create-on-open: it computes `pristine` and then rollback_checkpoint() opens and creates the database. A database created earlier by a read path makes the profile non-pristine with no keyring record (reconcile at 1455-1462).
- [`crates/foks-client-app/src/merkle_maintenance.rs:18`](../../../crates/foks-client-app/src/merkle_maintenance.rs#L18): Guards with try_exists() before open; the guard is applied per call site.
- [`crates/foks-client-app/src/registry.rs:2035`](../../../crates/foks-client-app/src/registry.rs#L2035): `CheckedProfileSession { session: &ProfileSession }` carries no operation resources; 171 pub methods are implemented on it across 23 files.
- [`crates/foks-client-app/src/checkpoint.rs:141`](../../../crates/foks-client-app/src/checkpoint.rs#L141): The HELD_CHECKED_PROFILES thread-local provides reentrancy for nested federation visits.
- [`crates/foks-client-app/src/checkpoint.rs:214`](../../../crates/foks-client-app/src/checkpoint.rs#L214): The SESSION_MATERIAL thread-local caches the Zeroizing master key for the session lifetime.
- [`crates/foks-client-app/src/kv.rs:824`](../../../crates/foks-client-app/src/kv.rs#L824): put_kv_file takes `vault: &mut AccountVault<'_>, master_key: &[u8; 32]` and calls derive_mutation_key(master_key) in the body.

**Recommendation**

Work in three steps. (1) Split HardStateStore::open into two constructors. `create_or_open` is used only where creation is intended: exclusive checked-session admission of a pristine profile (lock_and_verify_checkpoint/lock_and_verify_checkpoints, which call rollback_checkpoint after the pristine test), explicit test setup, and import staging. `open_existing` returns a typed StateMissing error and never creates a file. Switch the other roughly 177 production sites to open_existing. Start with read paths reachable outside a checked session (requires_import_verification, the agent's overview and SSO handlers). Add a test showing that a read after reset does not recreate hard.sqlite3. Treat SoftStateStore the same way only where creation is not intended. (2) Change MutationCoordinator::new to borrow a `&mut HardStateStore` instead of a path. Each transition still commits in its own IMMEDIATE transaction, so durability ordering is unchanged. (3) Introduce an OperationContext owned by CheckedProfileSession and created in with_session_policy. It holds a lazily opened HardStateStore and the Zeroizing master key, replacing SESSION_MATERIAL, with accessors vault(), protected_store() and soft_state(). Nested sessions come from an explicit `checked.nest(&other)` instead of HELD_CHECKED_PROFILES reentrancy, so the lock-holding relation is visible in the federation recursion. Remove the master_key and vault parameters from public methods as they migrate. Make the context !Send because the profile and database locks and the nesting relation are bound to the executing worker, not because of SQLite's threading mode. No repository transaction may stay open across network I/O; the repository API already keeps transactions internal.

<details><summary>Verifier note</summary>

The core claims hold. HardStateStore::open (metadata.rs:12) runs create_new, the symlink check, canonicalize, SQLite open, five pragmas and initialize_or_verify, and initialize_or_verify inserts a fresh database_id when application_id and user_version are both 0 (lib.rs:780-807). MutationCoordinator stores `hard_database: &'a Path` (mutation.rs:68) and reopens the database at 151, 191, 197, 208, 221, 238, 247 and 265. requires_import_verification opens with create (checkpoint.rs:695). merkle_maintenance guards with try_exists (line 18). HELD_CHECKED_PROFILES and SESSION_MATERIAL are thread-locals (checkpoint.rs:141, ~214). The parameter counts match: 108 `master_key: &[u8; 32]` and 249 `vault: &mut AccountVault`. Some numbers need correcting. Excluding inline test modules there are 177 production HardStateStore::open sites in foks-client/src and foks-client-app/src (272 including tests). I count 171 pub methods on CheckedProfileSession across 23 files. The hazard is also more concrete than stated. The agent calls requires_import_verification outside any checked session (ListProfileOverview, main.rs:3676; sso.rs:19). A reset deletes the hard database and its keyring records (checkpoint.rs:1759-1780). After that, this read path recreates the database with a new id outside the profile lock, and the next exclusive admission is no longer 'pristine'. Two parts of the recommendation need correcting. First, the creation set omits the deliberate create-on-open in checked-session admission: lock_and_verify_checkpoint computes `pristine` and then calls rollback_checkpoint (registry.rs:2052), which is how a newly added profile is enrolled. Second, the !Send rationale is wrong: SQLITE_OPEN_FULL_MUTEX is serialized mode and does not require single-thread use. Not tracked in ISSUES.md.

</details>

### client-libs-acting-credential

**Collapse the _yubi / _with_credential / _as_local_team method matrix behind one acting-credential type**

- Type: code-quality
- Priority: medium
- Effort: L
- Layers: client-lib
- Verification: adjusted

foks-client has 43 public `_yubi` methods, and 42 of them duplicate a software method of the same name. `FederationCredential` is already credential-agnostic, but the `_with_credential` wrappers only use it to dispatch to the duplicated twins. The copies have drifted. The software team load returns a reusable `TeamViewGrant`, which the read caches use. The Yubi copy has no grant variant and has its own inline enrollment check, which skips `require_enrolled`'s UID/host binding. The software load has no explicit enrollment check at all. The client-app read caches (`AuthCacheKey`, team-view and KV-node caches) accept only `DeviceCredential`, and invitations.rs:621 confirms that hardware credentials bypass them. PUK rotation and revocation are spread over three bodies. Software rotation is 235 lines and software revocation 257. The Yubi copy is 274 lines and combines rotation with bot revocation. Software and Yubi rotation differ in about 120 lines, ignoring whitespace. The Yubi copy also checks recipient key types per entity type, which software rotation does not.

**Evidence**

- [`crates/foks-client/src/auth.rs:64`](../../../crates/foks-client/src/auth.rs#L64): `FederationCredential { Software, Yubi }` already provides transport(), device_id() and require_enrolled(). It is used 96 times outside foks-client.
- [`crates/foks-client/src/team.rs:2319`](../../../crates/foks-client/src/team.rs#L2319): `load_and_pin_team_yubi` has no grant path. Its inline enrollment check (2326-2334) checks the device id, HEPK and subkey, but not the UID/host binding that `require_enrolled` adds. The software `load_and_pin_team_recording_view` (2238) returns a TeamViewGrant and has no explicit enrollment check.
- [`crates/foks-client/src/team.rs:2418`](../../../crates/foks-client/src/team.rs#L2418): `load_and_pin_team_with_credential` and `_as_local_team_with_credential` (2439) only match and forward to the twins.
- [`crates/foks-client/src/team/rotation/mod.rs:801`](../../../crates/foks-client/src/team/rotation/mod.rs#L801): refresh, resume and replay each have four variants: user, user_yubi, as_local_team and as_local_team_yubi (801-942, 1395-1494, 1697-1783).
- [`crates/foks-client/src/device.rs:1218`](../../../crates/foks-client/src/device.rs#L1218): `rotate_yubi_puks_inner` (274 lines) combines PUK rotation with bot revocation (`revoke_target`) and checks recipient key types. The software counterparts are separate: `rotate_software_puks_inner` (915, 235 lines) and `revoke_user_credential_with_software_device_inner` (617, 257 lines).
- [`crates/foks-client-app/src/auth_cache.rs:43`](../../../crates/foks-client-app/src/auth_cache.rs#L43): `AuthCacheKey::new` takes `&DeviceCredential`, and `load_team_for_read` (309) is software-only. invitations.rs:621 says only the software arm has read caches.

**Recommendation**

Rename `FederationCredential` to `ActingCredential`, since it is used well beyond federation, and make it the only credential parameter on public team, KV, rotation and device methods. Add `TeamActor<'a> { User(&AuthenticatedUserOutcome), Team(&AuthenticatedTeamOutcome) }` to fold the `_as_local_team` axis. Implement one body per operation over a small `PukSigner` trait (software seed vs `&dyn YubiDevice`) for signing and decapsulation, and keep the mTLS material from `ActingCredential::transport()`. Order the work so each step is reviewable: team load and grants (team.rs:2222-2470) and `AuthCacheKey`, which gives Yubi accounts view-grant reuse; then rotation/mod.rs; then device.rs PUK rotation. Delete each `_yubi` and `_with_credential` twin once client-app callers in yubi.rs, federation.rs and runtime.rs are migrated. The wire format is unaffected.

<details><summary>Verifier note</summary>

The core claims hold. foks-client has 43 `pub fn *_yubi` methods, and 42 of them have a same-named software twin. The 43rd, `revoke_bot_credential_with_yubi`, pairs with `revoke_user_credential_with_software_device`. `FederationCredential` (auth.rs:64) already provides `transport()`, `device_id()` and `require_enrolled()`. It is used 96 times outside foks-client, which supports the rename. The software `load_and_pin_team_recording_view` returns a `TeamViewGrant`. `load_and_pin_team_yubi` (team.rs:2319) has no grant path and repeats an inline enrollment check (2326-2334). That inline check omits the UID/host binding that `require_enrolled` performs. The software `load_and_pin_team` has no explicit enrollment check at all, so the copies differ in both directions. The `_with_credential` wrappers (team.rs:2418/2439) only dispatch. The rotation/mod.rs four-way variants are where the finding says: 801/847/905/942, 1395/1427/1462/1494 and 1697/1725/1751/1783. `AuthCacheKey::new` takes `&DeviceCredential`. invitations.rs:621 says 'Only the software arm has read caches; a hardware credential authenticates on the card every time.' The line counts check out: 235 + 274 = 509 lines, and a whitespace-insensitive diff shows 123 changed lines. The evidence needs one correction. The Yubi PUK copy also carries bot revocation (`revoke_target`, DeviceRevoke) and checks recipient key types per entity type, which the software rotation does not do. Software revocation is a separate third copy, `revoke_user_credential_with_software_device_inner` (device.rs:617-873, 257 lines). Merging the bodies therefore means reconciling behaviour, not only transport. ISSUES.md does not track this. The recommendation does not change the wire format.

</details>

### client-libs-checked-session-result-loss

**Checked-session wrapper reports a committed operation as failed when checkpoint publication fails**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: client-lib, agent, desktop-ui
- Verification: adjusted

with_session_policy runs the operation, calls advance_checkpoint, and returns the checkpoint error ahead of the operation's result. If a mutation commits (a server chain advance plus local journal and vault) and the keyring write then fails, for example because the keychain is locked or the request is unattended and needs CredentialsRequired, the caller receives an error and the report is discarded. The agent forwards it as an ordinary error code, and the desktop's failureOutcome classifies codes such as credentials-required and operation-failed as 'rejected'. The UI therefore reports a committed change as not applied. Reconciliation already accepts a higher revision with a new write token as AdvanceExternal, so the next admission repairs a missed publication. The same pattern appears in all four session wrappers, and no test covers it.

**Evidence**

- [`crates/foks-client-app/src/checkpoint.rs:752`](../../../crates/foks-client-app/src/checkpoint.rs#L752): `checkpoint.map_err(E::from)?;` precedes `result`, so Ok(T) is replaced by Err(checkpoint error).
- [`crates/foks-client-app/src/checkpoint.rs:901`](../../../crates/foks-client-app/src/checkpoint.rs#L901): The two-profile variant does the same for both checkpoints (901-902); it is repeated in try_with_checked_session (951) and try_with_shared_checked_session (1008).
- [`crates/foks-client-app/src/checkpoint.rs:1508`](../../../crates/foks-client-app/src/checkpoint.rs#L1508): advance_checkpoint reads and writes the native manifest; it can fail with keystore errors (CredentialsRequired, Native) and also with RollbackDetected or CheckpointResetRequired from reconciliation.
- [`crates/foks-client-app/src/checkpoint.rs:2749`](../../../crates/foks-client-app/src/checkpoint.rs#L2749): Reconciliation treats a higher revision with a different token as AdvanceExternal (also 2784-2788), so a later admission republishes.
- [`apps/desktop/src/operation-outcome.ts:28`](../../../apps/desktop/src/operation-outcome.ts#L28): failureOutcome returns 'rejected' for typed codes other than the admission, busy and transport codes, so a committed operation whose checkpoint publication failed is shown as rejected.

**Recommendation**

Distinguish transient publication failures from integrity failures. When the operation returned Ok and advance_checkpoint failed only with a keystore availability error (CredentialsRequired, a native or locked-keychain error), return the result together with a typed deferred-publication signal. Because the wrappers are generic over T, carry it beside the value: either a `Committed<T> { value, checkpoint_deferred: Option<Error> }` return from a new wrapper used by the agent, or a per-request outcome the agent attaches to the success response as a warning field (for example reason "checkpoint-deferred"). The desktop then shows success plus a keychain notice instead of a failure. No extra per-profile flag is needed, because the next admission already publishes via AdvanceExternal. Keep returning the error when the operation itself failed, and when the post-operation check reports RollbackDetected or CheckpointResetRequired, since those must stay visible. Add tests that fail the post-operation manifest access after a successful local mutation. They should check that the value is surfaced with the deferral signal, that reconciliation failures still surface as errors, and that the next session reconciles as AdvanceExternal.

<details><summary>Verifier note</summary>

Confirmed mechanics. with_session_policy calls advance_checkpoint after the operation and evaluates `checkpoint.map_err(E::from)?` (checkpoint.rs:752) before returning `result`. The same pattern is at 901-902, 951 and 1008. advance_checkpoint (1508) performs a native-manifest read and write. Reconciliation returns AdvanceExternal for a higher revision with a changed token (2749-2788), so the next admission republishes. No test injects a post-operation publication failure. Two parts need correcting. First, the agent's contract does not say every other error means 'did not run': its comment says non-admission errors may have committed. The concrete misreport is in the desktop, whose failureOutcome (operation-outcome.ts:28-50) maps OperationFailed, CredentialsRequired, Conflict and similar codes to 'rejected'. Second, Option B as written is unsafe. advance_checkpoint can also fail with RollbackDetected or CheckpointResetRequired from post-operation reconciliation, and logging and swallowing those would hide an integrity failure. The proposed per-profile flag is also redundant because admission already publishes AdvanceExternal. Option A's 'Error variant carrying a boxed result' does not fit wrappers that are generic over T. Not tracked in ISSUES.md.

</details>

### client-libs-hard-state-migration-ladder

**Replace ad-hoc hard-state version branches with a stepwise migration ladder and frozen historical fixtures**

- Type: maintainability
- Priority: medium
- Effort: M
- Layers: client-lib, tooling
- Verification: adjusted

initialize_or_verify handles schema 38 and 39 in separate branches that each jump straight to SCHEMA_VERSION (40). The admission_floor -> validated_time_floor rename is duplicated in both. The v38 step rebuilds merkle_heads by string-splitting the current INITIAL schema and then inserts rows in the v40 column shape, so a future change to that table would silently change, or break, what the v38 migration produces. The only migration test builds its 'v38' database by text-replacing one CHECK clause in the current schema, which already contains validated_time_floor. No test creates a v39 database or an admission_floor column, so the v39 branch and both rename paths are untested. The v38 step bumps revision and write token while the v39 step does not, and neither difference is documented. The server applies ordered version guards in one IMMEDIATE transaction, and as of commit 9fb5e16 it tests every supported predecessor (43-48) by rebuilding each one with reverse DDL.

**Evidence**

- [`crates/foks-client-db/src/lib.rs:816`](../../../crates/foks-client-db/src/lib.rs#L816): `if version == 38 { merkle_checkpoint::migrate(..) }` and `if version == 39 { ... RENAME COLUMN ... }` (820-842) both set user_version to SCHEMA_VERSION directly.
- [`crates/foks-client-db/src/merkle_checkpoint.rs:153`](../../../crates/foks-client-db/src/merkle_checkpoint.rs#L153): The v38 step extracts the merkle_heads definition from the current SCHEMA with split("CREATE TABLE merkle_heads ("), then inserts 6-column v40-shaped rows.
- [`crates/foks-client-db/src/merkle_checkpoint.rs:166`](../../../crates/foks-client-db/src/merkle_checkpoint.rs#L166): The admission_floor rename is duplicated from lib.rs:825-838; the v38 path bumps revision and write_token (line 181), while the v39 path does not.
- [`crates/foks-client-db/src/merkle_checkpoint.rs:282`](../../../crates/foks-client-db/src/merkle_checkpoint.rs#L282): The test fixture synthesizes v38 with `SCHEMA.replace("IN (1, 2, 3, 4)", "IN (1, 2, 3)")` and already contains validated_time_floor; no test anywhere builds a v39 database or an admission_floor column.
- [`crates/foks-server-db/src/schema.rs:44`](../../../crates/foks-server-db/src/schema.rs#L44): Server pattern: one IMMEDIATE transaction applying ordered version guards (43..=47), rechecked under the write lock.
- [`crates/foks-server/src/operations/backup/compatibility_tests.rs:57`](../../../crates/foks-server/src/operations/backup/compatibility_tests.rs#L57): Server tests from 9fb5e16 rebuild each predecessor schema 43-48 by reverse DDL on the current schema, including column renames; the client has no equivalent for v39.
- [`book/19-agent.qmd:160`](../../../book/19-agent.qmd#L160): The book documents 'migrate supported versions 38/39 to 40; refuse other versions'.

**Recommendation**

Introduce `const MIGRATIONS: &[(u32, fn(&Transaction) -> Result<()>)]` with steps 38->39 and 39->40, run in order inside one IMMEDIATE transaction after rechecking user_version under the lock. Before setting user_version, compare sqlite_schema with an in-memory INITIAL database, reusing inspection::schema, so every migrated database is proven equal to a fresh one on the normal open path and not only in inspect_existing. Freeze the historical DDL each step needs, such as the v40 merkle_heads definition, as constants instead of deriving it from INITIAL. Document whether each step bumps revision and write_token. For tests, at minimum add predecessor builders for v38 and v39 the way the server's compatibility_tests.rs does, including a v39 database with admission_floor and a v38 database with admission_floor. Preferably also check in SQL dumps captured from the tagged builds (tests/fixtures/hard-v38.sql, hard-v39.sql). Assert database_id preservation, revision and write-token behavior, and schema equality for each. Under the pre-v1 policy, state in a comment which versions the ladder keeps and when old steps may be dropped.

**Already tracked:** book/19-agent.qmd (hard state table) documents the 38/39 to 40 migration behavior; it does not cover the structure or the test gap.

<details><summary>Verifier note</summary>

Confirmed. initialize_or_verify branches on version 38 (lib.rs:816) and 39 (lib.rs:820), and each jumps straight to SCHEMA_VERSION (40). The admission_floor rename appears both at lib.rs:825-838 and at merkle_checkpoint.rs:166-179. The v38 step derives the merkle_heads DDL with `SCHEMA.split("CREATE TABLE merkle_heads (")` (merkle_checkpoint.rs:153) and then inserts rows assuming the v40 column shape. The only migration test synthesizes v38 with `SCHEMA.replace("IN (1, 2, 3, 4)", "IN (1, 2, 3)")` (line 282) on a schema that already has validated_time_floor. Nothing in the repository creates a user_version=39 database or an admission_floor column, so the v39 branch and both rename paths are untested. Both branches already recheck user_version under an IMMEDIATE transaction. Three evidence corrections. The book row is at book/19-agent.qmd:160, not 180. The server tests added in 9fb5e16 are not captured fixtures: compatibility_tests.rs:57-104 rebuilds each predecessor schema (43-48) by applying reverse DDL to the current schema, as server-db's migration tests do. The server pattern is still the right comparison because it covers each supported version, including renames. Also, the v38 step bumps revision and write_token but the v39 step does not; the ladder should state per step whether it does. Not tracked beyond the book's one-line behavior description.

</details>

### client-libs-journal-state-encoding

**Define journal-state SQL encodings and terminal sets once instead of repeating integer literals**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: client-lib
- Verification: adjusted

Each journal state enum has its own hand-written integer mapping. Mutation, team, ad-hoc team, signup and federation saga states start at 1, and chat and SSO states start at 0. Parsing is done by `from_sql` (lib.rs), `parse` (sso.rs) or an inline match with a catch-all `_ => Cancelled` (chat.rs:224-229). Only Mutation, FederationSaga and Chat have `is_terminal`. The nine non-terminal predicates are written as raw SQL literals in two places, `portability_blockers` (inspection.rs:12-49) and `compact_merkle_roots` (merkle_gc.rs:33-41), and the two copies have already diverged. merkle_gc.rs:38 uses `federation_saga_operations WHERE state!=3`, but FederationSagaState's terminal set is {5 Completed, 6 Rejected}, which inspection.rs:35 uses correctly. Saga rows are never deleted. Once any federated member add finishes, Merkle root compaction is therefore disabled for good, and a saga in LocalPrepared (3) does not block collection. No test covers either path.

**Evidence**

- [`crates/foks-client-db/src/merkle_gc.rs:38`](../../../crates/foks-client-db/src/merkle_gc.rs#L38): `SELECT EXISTS(SELECT 1 FROM federation_saga_operations WHERE state!=3)`: wrong terminal set for FederationSagaState (terminal = 5,6). A completed or rejected saga blocks GC permanently, and LocalPrepared (3) does not block it.
- [`crates/foks-client-db/src/inspection.rs:35`](../../../crates/foks-client-db/src/inspection.rs#L35): The same table uses the correct `state NOT IN (5,6)`. The nine-predicate list at lines 12-49 duplicates merkle_gc.rs:33-41.
- [`crates/foks-client-db/src/lib.rs:594`](../../../crates/foks-client-db/src/lib.rs#L594): `FederationSagaState::is_terminal` is `Completed | Rejected`, which contradicts merkle_gc.rs:38.
- [`crates/foks-client-db/src/lib.rs:487`](../../../crates/foks-client-db/src/lib.rs#L487): TeamMutationState 1..7 with `from_sql` and no `is_terminal`. Terminality is implied only by `can_transition_to`.
- [`crates/foks-client-db/src/repositories/chat.rs:224`](../../../crates/foks-client-db/src/repositories/chat.rs#L224): Chat state is parsed inline with a catch-all `_ => ChatOperationState::Cancelled`, with no named from_sql and no unknown-value error.
- [`crates/foks-client-db/src/repositories/sso.rs:19`](../../../crates/foks-client-db/src/repositories/sso.rs#L19): `SsoFlowState::parse` returns `rusqlite::Error::InvalidQuery` and has no `is_terminal`. Line 114 repeats `state IN (0,1,2,3,7)` a third time.
- [`crates/foks-client-app/src/merkle_maintenance.rs:79`](../../../crates/foks-client-app/src/merkle_maintenance.rs#L79): Production caller of `compact_merkle_roots`. foks-client-app/src/federation.rs:740 records sagas through add_remote_team_member.

**Recommendation**

First, fix merkle_gc.rs:38 to `state NOT IN (5,6)`. Add tests showing that a Completed or Rejected saga lets compaction proceed and that a LocalPrepared saga defers it. Then define the nine (workflow, unfinished-predicate) pairs once as a crate-private const shared by `portability_blockers` and `compact_merkle_roots`. Add a small internal trait (`to_sql`, `from_sql` with a shared unknown-value error, `is_terminal`, `ALL`) implemented by each journal state enum, including Signup, AdHoc, TeamMutation and Sso, which lack `is_terminal` today. Replace the chat.rs inline catch-all with a strict `from_sql`. Add a test that derives each table's non-terminal set from `ALL.filter(!is_terminal)` and compares it with the shared predicate, plus a round-trip test of `ALL` through to_sql and from_sql per enum. Keep separate, named predicates for non-terminal sets with other meanings: the team reserved-position set IN (1..5) and the SSO active-with-expiry predicate. No schema change is needed.

<details><summary>Verifier note</summary>

The core claim holds and the cited lines are accurate. inspection.rs:10-58 hard-codes the nine predicates. TeamMutationState (lib.rs:487) uses 1..7 with from_sql and has no is_terminal. ChatOperationState (chat.rs:38) starts at 0 and has is_terminal. SsoFlowState (sso.rs:6) starts at 0 and uses parse returning rusqlite::Error::InvalidQuery. Signup, AdHoc, TeamMutation and Sso have no is_terminal. ISSUES.md does not track this. The statement that the literals are correct today is false. The same nine-table blocker list is copied into crates/foks-client-db/src/merkle_gc.rs:33-41, and its federation predicate is `state!=3`, a copy of the signup/ad-hoc form. FederationSagaState 3 is LocalPrepared, and the terminal set is {5,6}: is_terminal at lib.rs:594 and the `NOT IN (5,6)` in inspection.rs:35 both say so. No code deletes federation_saga_operations rows. Because of this, compact_merkle_roots returns 0 forever after the first saga reaches Completed or Rejected, and a saga in LocalPrepared does not defer collection, although the comment requires every owning workflow to be terminal. Production code reaches this through foks-client-app/src/federation.rs:740 (add_remote_team_member) and merkle_maintenance.rs:79. The lib.rs tests cover only import_readiness as a blocker. This is the drift the finding predicts, already present, so priority goes up to medium. Two more points: chat_operation (chat.rs:224-229) parses state inline with a catch-all `_ => Cancelled` instead of a named from_sql, so an unknown value is not reported as an error (schema CHECK state IN (0..4) mitigates this). Some queries also need predicates other than terminal, such as the team reserved-position set IN (1..5) at schema.rs:379 and journals.rs:700/883, and the SSO active-with-expiry check at sso.rs:114. One is_terminal method therefore cannot replace every literal.

</details>

### client-libs-secret-record-redaction

**Vault records with raw seeds derive Debug and rely on hand-written Drop zeroization**

- Type: security
- Priority: medium
- Effort: S
- Layers: client-lib
- Verification: adjusted

Several serde records persisted in the credential vault store raw secrets as plain arrays and derive Debug. The most sensitive is StoredTeam, which holds the team's member_min, member, admin and owner seeds plus a removal key. Others are StoredAccount, PendingSignup, PendingDevice and PendingRecovery (device, PUK and self-token secrets), StoredLocalMembership and StoredFederatedMembership (removal keys), and StoredTeamPtkRotation (PTK seeds). Any `{:?}`, `dbg!` or `unwrap_err` on them would print the bytes, which contradicts the book's rule that secret types redact Debug. No current call site formats them, so the risk is latent. Zeroization relies on manual Drop impls that must be kept in sync with the fields. Nested membership records have no Drop of their own and are zeroized only through StoredTeam's Drop, so a value moved out of the parent is never zeroized. 33 vault writes use `Zeroizing::new(serde_json::to_vec(..))`, whose growing buffer can leave unzeroed plaintext copies in freed allocations.

**Evidence**

- [`crates/foks-client-app/src/team.rs:1781`](../../../crates/foks-client-app/src/team.rs#L1781): StoredTeam derives Debug with member_min, member, admin and owner team seeds plus removal_key; zeroized only by the manual Drop at 2165.
- [`crates/foks-client-app/src/account.rs:1221`](../../../crates/foks-client-app/src/account.rs#L1221): StoredAccount derives Debug and holds `device_seed: [u8; 32]`; a manual Drop at 1231 zeroizes it.
- [`crates/foks-client-app/src/account.rs:1237`](../../../crates/foks-client-app/src/account.rs#L1237): PendingSignup derives Debug with device_seed, puk_seed and self_token; PendingDevice (1255) and PendingRecovery (1331) are the same.
- [`crates/foks-client-app/src/team.rs:1808`](../../../crates/foks-client-app/src/team.rs#L1808): StoredLocalMembership derives Debug with `removal_key: [u8; 32]` and has no Drop of its own; StoredFederatedMembership (1879) is the same. Both are zeroized only through StoredTeam's Drop loop.
- [`crates/foks-client-app/src/team.rs:1954`](../../../crates/foks-client-app/src/team.rs#L1954): StoredTeamPtkRotation derives Debug with a PTK `seed: [u8; 32]`; zeroization is done by the parents' Drop impls (1839, 1870, 1975).
- [`crates/foks-client-app/src/account.rs:1671`](../../../crates/foks-client-app/src/account.rs#L1671): `Zeroizing::new(serde_json::to_vec(..))` wraps the buffer only after serialization; there are 33 such sites in the crate.
- [`book/08-local-keystore.qmd:176`](../../../book/08-local-keystore.qmd#L176): The book states that every secret type implements Debug as [REDACTED] and that encoders precompute sizes to avoid growing buffers.

**Recommendation**

Add a `SecretArray<const N: usize>` newtype, either in foks-client-app or in foks-crypto next to SecretSeed. It wraps `Zeroizing<[u8; N]>`, implements Debug as "[REDACTED]" and ZeroizeOnDrop, and implements Serialize/Deserialize by hand with serialize_tuple(N) and a tuple visitor, because serde's built-in array impls only cover N up to 32. That emits the same JSON number array as today, so existing vault files stay byte-compatible and no migration is needed. Replace the secret fields in StoredTeam (member_min, member, admin, owner, removal_key), StoredAccount, PendingSignup, PendingDevice, PendingRecovery, StoredLocalMembership, StoredFederatedMembership and StoredTeamPtkRotation, including the [u8; 17] self tokens. Then delete the manual Drop impls. Add `encode_private_json<T: Serialize>(&T) -> Result<Zeroizing<Vec<u8>>>`, which serializes first into a byte-counting writer and then into a pre-sized Zeroizing buffer, and use it at the 33 vault-write sites. Add a test that builds each record with secret bytes 0xAB and asserts that its Debug output contains neither "171" nor "ab".

<details><summary>Verifier note</summary>

Confirmed. StoredAccount (account.rs:1221), PendingSignup (1237), PendingDevice (1255) and PendingRecovery (1331) derive Debug and hold raw [u8; 32] seeds and [u8; 17] self tokens, with hand-written Drop impls. StoredLocalMembership (team.rs:1808), StoredFederatedMembership (1879) and StoredTeamPtkRotation (1954) derive Debug and hold raw removal keys or PTK seeds. account.rs:1671 is `Zeroizing::new(serde_json::to_vec(offer)?)`, and there are exactly 33 such sites. book/08-local-keystore.qmd:175-185 states the redaction and pre-sized-encoder rules. The finding omits the most sensitive record. StoredTeam (team.rs:1781) derives Debug and holds the team's member_min, member, admin and owner seeds plus removal_key, zeroized only by a manual Drop at team.rs:2165. StoredLocalMembership and StoredFederatedMembership have no Drop of their own and are zeroized only through StoredTeam's Drop, so a value moved out of those vectors is never zeroized. I found no current `{:?}` of these records, so the leak is latent. Feasibility note: serde's built-in array impls stop at N=32, so a const-generic SecretArray<N> must use serialize_tuple(N) and a tuple visitor; the resulting JSON array matches the current form. Not tracked in ISSUES.md.

</details>

### client-libs-soft-stage-reclaim

**Wire orphaned large-file stage reclamation into exclusive session entry**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: client-lib, docs
- Verification: adjusted

Large KV downloads stream decrypted chunks, up to 1 GiB per file, into `kv_large_files`/`kv_large_file_chunks` in soft state. `Drop` cleans up a store's own stages, but a process crash leaves orphaned plaintext stages. `reclaim_orphaned_large_files` exists and the book presents it as part of the design, but its only caller is a unit test. Orphans are swept only as a side effect of `invalidate_directories` after a local write to the same party, so a read-only party, such as a team vault where the user is a reader, keeps them indefinitely. That sweep also deletes every unreferenced stage of the party without the `owned_stages` guard. Its safety depends on the exclusive profile operation lock, which is not documented on either function.

**Evidence**

- [`crates/foks-client-db/src/soft.rs:905`](../../../crates/foks-client-db/src/soft.rs#L905): `reclaim_orphaned_large_files` refuses only when this store owns stages; the only caller is the test at soft.rs:2791.
- [`crates/foks-client-db/src/soft.rs:1439`](../../../crates/foks-client-db/src/soft.rs#L1439): `invalidate_directories` deletes all unreferenced stages for the party, with no ownership guard.
- [`crates/foks-client-db/src/soft.rs:1629`](../../../crates/foks-client-db/src/soft.rs#L1629): Drop removes only this connection's stages; a crash bypasses it.
- [`crates/foks-client-db/src/soft_schema.rs:132`](../../../crates/foks-client-db/src/soft_schema.rs#L132): `kv_large_file_chunks.content` holds plaintext chunks.
- [`crates/foks-client-app/src/merkle_maintenance.rs:11`](../../../crates/foks-client-app/src/merkle_maintenance.rs#L11): Existing rate-limited maintenance hook that runs at outermost exclusive session entry (called at checkpoint.rs:742).
- [`book/19-agent.qmd:214`](../../../book/19-agent.qmd#L214): The 'Staging and reclamation' section describes reclamation as an active mechanism.

**Recommendation**

Add `compact_soft_large_file_stages` next to `compact_session_merkle_roots` at both outermost exclusive entry points, checkpoint.rs:742 (`with_session_policy`) and checkpoint.rs:941 (the try_ variant). Alternatively, add it to the agent's `maintain_adapter_state`, which already skips busy profiles. Rate-limit it per database the way LAST_SCAN does. It should open the soft store and call `reclaim_orphaned_large_files`. On both `reclaim_orphaned_large_files` and `invalidate_directories`, document that they are safe only under `ProfileLock::operation`, which excludes shared readers that may own in-flight stages. An `owned_stages` guard in `invalidate_directories` would do nothing, because its only caller (kv/write.rs:1108) opens a fresh store. To make reclamation age-aware, add `created_at` with the in-place additive pattern used for `kv_large_file_measurements`, not with a VERSION bump. A version bump makes `UnsupportedSoftSchema` require manual cache recreation. Add a test that leaks a store with `std::mem::forget`, opens an exclusive checked session, and checks that the orphan is gone. Update book/19-agent.qmd to say when reclamation runs.

**Already tracked:** book/19-agent.qmd 'Staging and reclamation' describes the function but not that it has no production caller.

<details><summary>Verifier note</summary>

The core claims hold. The only caller of `reclaim_orphaned_large_files` (soft.rs:905) is the test at soft.rs:2791. `invalidate_directories` (1409; the SQL is at 1439) deletes every unreferenced stage of the party, and its only production caller is the KV write path (foks-client kv/write.rs:1108). Drop (1628) removes only the stages this store owns. `kv_large_file_chunks.content` holds plaintext, and `MAX_KV_FILE_BYTES` is 1 GiB. Incomplete orphans are never adopted again. The book's 'Staging and reclamation' section (19-agent.qmd:213) describes reclamation as active. The recommendation needs three corrections. (1) Exclusive entry calls `compact_session_merkle_roots` from two places, checkpoint.rs:742 (`with_session_policy`) and 941 (the try_ variant), so the hook belongs in both, or in the agent's `maintain_adapter_state`, which already skips busy profiles. (2) A soft schema version bump does not rebuild the cache automatically. `initialize` returns `UnsupportedSoftSchema`, and the agent reports it to the user as 'cache must be recreated'. A `created_at` column should therefore be added with the additive pattern already used for `kv_large_file_measurements` (soft.rs:1715-1725). (3) Adding an `owned_stages` guard to `invalidate_directories` would have no effect, because its only caller opens a fresh store with no owned stages. Documenting the exclusive-lock precondition is the actual fix. The existing doc on `reclaim_orphaned_large_files` gives the precondition only in general terms, not as the profile lock.

</details>

### client-libs-team-intent-layering

**Move team rekey/member-edit/expulsion intent state machines out of client-app runtime into foks-client**

- Type: maintainability
- Priority: medium
- Effort: XL
- Layers: client-lib
- Verification: confirmed

The durable state of a named-team member edit or rekey lives in three stores. client-app generates the PTK seeds and writes a JSON vault record (`team-member-edit.*`, `team-rekey.*`, `federation-expulsion.*`). foks-client then journals the operation in hard state and writes the request to the protected mutation store. Reconciling these, by matching the vault intent to the journal row and choosing resume, replay, reject or discard, is implemented in client-app `runtime.rs` inside very long functions. That makes the protocol state machine depend on product-layer code and hard to unit-test without a network. foks-client already owns the journal transitions and replay primitives that this logic must agree with.

**Evidence**

- [`crates/foks-client-app/src/runtime.rs:1890`](../../../crates/foks-client-app/src/runtime.rs#L1890): `refresh_local_team_member_keys` is 733 lines, mixing intent/journal binding checks, credential fallback and resume/replay/reject decisions.
- [`crates/foks-client-app/src/runtime.rs:562`](../../../crates/foks-client-app/src/runtime.rs#L562): `refresh_authenticated_team_graph` is about 412 lines; `execute_stored_team_rekey` (2914) is 245 lines.
- [`crates/foks-client-app/src/runtime.rs:3237`](../../../crates/foks-client-app/src/runtime.rs#L3237): `team_mutation_matches_pending` and `stored_team_rekey_matches_current_plan` (3162) compare vault intents with hard-state journal rows in the app layer.
- [`crates/foks-client-app/src/team.rs:1076`](../../../crates/foks-client-app/src/team.rs#L1076): The app generates rotation seeds, writes the StoredTeamMemberEdit to the vault (1140), and then foks-client journals and stores the request (1145).
- [`crates/foks-client/src/team/rotation/mod.rs:1395`](../../../crates/foks-client/src/team/rotation/mod.rs#L1395): foks-client already implements resume/replay/reject/discard for 'caller-durable' plans, so the state machine is split across crates.

**Recommendation**

First step (M): extract runtime.rs lines 1890-3250 into `runtime/team_rekey.rs`. Factor the decision logic into a pure function `resolve_stored_rekey(pending, journal_row, verified_team, actor) -> RekeyResolution { Complete, Resume, Replay, Reject, Discard, TryStrongerCredential }` and unit-test it with fixtures, with no network. Then (L) define in foks-client a `DurableIntentStore` trait (put_if_absent, get, remove and list by operation_id). Client-app implements it over the vault, which keeps foks-client vault-agnostic. Move the intent record types and the resolution function into `foks_client::team::rotation::intent`, so that seed generation, intent persistence, journaling and reconciliation sit next to each other. client-app keeps only scheduling, alias binding and capability policy. Consider storing intents in the protected mutation store keyed by operation_id. It is already derived from the master key, supports owner enumeration (`ProtectedRecordOwner`) and is covered by portability inventory, which would remove one of the three durable stores.

<details><summary>Verifier note</summary>

The cited functions and their lengths are correct. In runtime.rs, `refresh_local_team_member_keys` runs 1890-2623 (734 lines), `refresh_authenticated_team_graph` 562-974 (413 lines) and `execute_stored_team_rekey` 2914-3159 (246 lines). `stored_team_rekey_matches_current_plan` (3162) and `team_mutation_matches_pending` (3237) compare vault intents with hard-state journal rows in the app layer, and no test calls either of them directly. In client-app team.rs, the app generates seeds (1074), writes `StoredTeamMemberEdit` to the vault as JSON (1140), and then calls foks-client, which journals and stores the request (1145). The vault families `team-rekey.`, `team-member-edit.` and `federation-expulsion.` exist (lib.rs:324-332, account.rs:1562-1567). foks-client documents 'caller-durable' resume/replay/reject/discard in rotation/mod.rs (1393ff). `ProtectedRecordOwner` and the protected-mutation portability inventory both exist. ISSUES.md does not track this, and AGENTS.md sets no conflicting layering rule. The first step, a pure resolution function, is feasible and touches neither the wire format nor the server. The idea of moving intents into the protected store is offered only as a consideration, which is appropriate. An intent is written before its journal owner exists, so that move would need a new protected record family and owner.

</details>

### client-libs-agent-dto-duplication

**Stop duplicating foks-agent-proto DTOs as Serialize-only report types in client-app**

- Type: maintainability
- Priority: low
- Effort: M
- Layers: client-lib, agent
- Verification: adjusted

foks-client-app depends on foks-agent-proto only to borrow its base64 serde adapters. It then defines its own Serialize-only report types that mirror agent-proto Deserialize types field for field, kept in sync by doc comments. The agent turns these reports into `serde_json::Value`, so a renamed or added field fails only at runtime in a frontend. The crate doc also says it has no dependency outside the FOKS workspace, although it depends on rustix, fs2, toml, serde_json, base64 and sha2.

**Evidence**

- [`crates/foks-client-app/src/kv.rs:1381`](../../../crates/foks-client-app/src/kv.rs#L1381): `KvReadReport` mirrors `foks_agent_proto::data::DataEntry`, and the comment at 1390 says the two shapes must move together. `KvChunkReport` (1400) mirrors `DataChunk`.
- [`crates/foks-client-app/src/kv/data_stat.rs:4`](../../../crates/foks-client-app/src/kv/data_stat.rs#L4): `DataStatReport` mirrors `foks_agent_proto::data::DataStat` (agent-proto data.rs:189). `DataStat` has no `deny_unknown_fields`.
- [`crates/foks-client-app/src/kv.rs:1355`](../../../crates/foks-client-app/src/kv.rs#L1355): `KvRoleSummary` duplicates `foks_agent_proto::KvRole` (message.rs:315). The agent maps between the two by hand in `catalog_role` (agent main.rs:2870), `app_role_to_wire` (3012) and `wire_role_to_app`.
- [`crates/foks-agent/src/data.rs:55`](../../../crates/foks-agent/src/data.rs#L55): The agent forwards reports as `serde_json::to_value(...)`. foks-agent has 128 such calls, and foks-mcp (agent.rs:94, :343) deserializes the result into the agent-proto types at runtime.
- [`crates/foks-client-app/src/lib.rs:5`](../../../crates/foks-client-app/src/lib.rs#L5): The crate doc says the crate has no dependency outside the FOKS workspace. Cargo.toml lists rustix, fs2, libc, rustls, toml, url, serde_json, sha2 and base64.

**Recommendation**

Either return the agent-proto types (`DataEntry`, `DataChunk`, `DataStat`, `KvRole`) directly from the data and KV methods, or keep the client-app reports and add one contract test per report. The test should serialize a fully populated value and deserialize it into the agent-proto type. Add `#[serde(deny_unknown_fields)]` to `DataStat` first, or the test cannot catch an added field. If client-app returns `DataEntry`, it gives up Clone and Debug and gets zeroizing Drop. Delete the hand-written `KvRoleSummary`<->`KvRole` mappers in foks-agent main.rs once one role type is shared. Correct the crate doc in lib.rs.

<details><summary>Verifier note</summary>

The core claims hold. foks-client-app uses foks-agent-proto only for the `base64_bytes` serde adapters, in seven places in kv.rs and kv/data_stat.rs. `KvReadReport` (kv.rs:1381), `KvChunkReport` and `DataStatReport` mirror `DataEntry`, `DataChunk` and `DataStat`, and the doc comments say the shapes must move together. `KvRoleSummary` duplicates `KvRole` (message.rs:315). The agent converts between the two by hand in three places: `catalog_role` (main.rs:2870), `app_role_to_wire` (3012) and `wire_role_to_app`. foks-agent has 128 `serde_json::to_value` calls. foks-mcp deserializes `DataEntry` and `DataStat` at runtime. The lib.rs crate doc says the crate has no dependency outside the FOKS workspace, which is false: it uses rustix, fs2, libc, rustls, toml, url, serde_json and others. The recommendation needs one correction. `DataEntry` and `DataChunk` have `deny_unknown_fields`, but `DataStat` (agent-proto data.rs:189) does not, so the proposed contract test would not catch an added `DataStatReport` field unless `DataStat` gets the attribute. Also, `DataEntry` implements zeroizing Drop and has no Debug or Clone, while `KvReadReport` derives Clone, Debug and Eq. Returning `DataEntry` directly changes what callers can do with the value, which is acceptable but should be stated.

</details>

### client-libs-kv-scope-api

**Unify personal/team KV methods behind a KvScope and a shared write-session prologue**

- Type: code-quality
- Priority: low
- Effort: M
- Layers: client-lib, cli
- Verification: adjusted

client-app's KV surface pairs most personal operations with a team twin: checked put, put_with_size, symlink, mkdir and remove/remove_bound. The checked twins already share one prologue helper per side, `put_kv_node_checked` and `with_team_kv_write_session`. The unchecked user functions `put_kv_file`, `mkdir_kv` and `remove_kv` each repeat that prologue in full: capability check, split_parent, vault.account, pinned_host, authenticate_and_pin, opening the protected store, and opening the write session. About 20 public functions take team or party IDs as hex `&str` and parse them inside. Only the CLI's direct `kv` mode reaches the unchecked variants, and they allow overwrite with no version precondition. The agent exposes only Create, ExactVersion and ExactEntry, so the CLI and the agent-backed frontends give the same command different overwrite semantics.

**Evidence**

- [`crates/foks-client-app/src/kv.rs:824`](../../../crates/foks-client-app/src/kv.rs#L824): `put_kv_file`, `mkdir_kv` (879) and `remove_kv` (995) each repeat the full authentication and write-session prologue. The checked variants share `put_kv_node_checked` and `with_team_kv_write_session`.
- [`crates/foks-client-app/src/kv.rs:383`](../../../crates/foks-client-app/src/kv.rs#L383): `put_kv_file_checked` and `put_team_kv_file_checked` (446) and the `_with_size` twins (410/477) are separate public methods, as are the symlink (517/549), mkdir (932/962) and remove (1042/1135, 1063/1160) pairs.
- [`crates/foks-client-app/src/kv.rs:1332`](../../../crates/foks-client-app/src/kv.rs#L1332): `KvMutationPrecondition` has only Create, ExactVersion and ExactEntry. There is no unconditional variant, and the agent's `wire_precondition` (agent main.rs:3020) maps the same three.
- [`crates/foks-client-app/src/lib.rs:172`](../../../crates/foks-client-app/src/lib.rs#L172): `entity_id_from_hex` parses IDs inside client-app. About 20 public functions in kv.rs, kv/checked_move.rs, team.rs and federation.rs take `team_id_hex`, `party_id_hex` or `team_id` as `&str`.
- [`crates/foks-cli/src/main.rs:1117`](../../../crates/foks-cli/src/main.rs#L1117): The CLI's direct `kv` path calls the unchecked `put_kv_file` with `overwrite`, and `mkdir_kv` (1137) and `remove_kv` (1149).

**Recommendation**

Introduce `KvScope<'a> { User { account }, Team { account, team_alias, team_id: &EntityId } }`, following the single-method `team_selector` shape that `prepare_data_write` already uses. Merge `put_kv_node_checked` and `with_team_kv_write_session` into one `with_kv_write_session(scope, ..)`. Collapse each user/team pair into one method, and fold `_with_size` into an `Option<u64>` parameter. Parse hex IDs in the agent and CLI argument layers and pass `EntityId`. Remove the unchecked `put_kv_file`, `mkdir_kv` and `remove_kv`. Move the CLI onto the checked API by resolving the current entry and passing `ExactEntry` when overwriting, `Create` otherwise, and `remove_kv_bound` for removal. Do not add a `KvMutationPrecondition::Any` variant, because that would reintroduce unconditional overwrite. Update the examples and tests that call the unchecked functions (webview_fixture.rs, portability import tests, vault_identity.rs).

<details><summary>Verifier note</summary>

The core claims hold. KV operations come in user/team pairs: put_*_checked (383/446), the _with_size variants (410/477), symlink (517/549), mkdir_checked (932/962) and remove_checked/bound (1042/1135, 1063/1160). The unchecked `put_kv_file` (824), `mkdir_kv` (879) and `remove_kv` (995) each repeat the full prologue. In production only the CLI's direct `kv` command reaches them (main.rs:1117/1137/1149), while the agent always calls the checked variants. Three statements need correcting. (1) The checked variants do not each repeat the prologue: the user side shares `put_kv_node_checked` and the team side shares `with_team_kv_write_session` (kv.rs about 585-750). (2) About 20 public functions take a hex party/team ID string. The figure of 28 counts parameter occurrences, including private helpers. (3) `KvMutationPrecondition::Any` does not exist; the variants are Create, ExactVersion and ExactEntry. The agent wire `KvPrecondition` deliberately has no unconditional variant. The CLI should therefore move to checked semantics by resolving the current entry and passing ExactEntry, or Create, not by gaining a new `Any` variant that brings unconditional overwrite back. `prepare_data_write`'s `team_selector: Option<String>` is an existing single-method scope that the new design can follow.

</details>

### client-libs-oidc-deadline

**OIDC provider discovery ignores the FoksClient deadline and cancellation token**

- Type: correctness-risk
- Priority: low
- Effort: S
- Layers: client-lib, agent
- Verification: adjusted

`Provider::discover` makes two sequential blocking reqwest calls (configuration, then JWKS), each with a fixed 10 s total timeout, before SSO initialization or result validation continues. It does not observe the FoksClient's CancellationToken or per-RPC timeout. The agent's SSO handler builds ProviderHttp with a default policy, although the request's timeout and cancellation token are in scope. Discovery can therefore run about 20 s against the agent's default 15 s request budget. After the agent cancels, waits its 1 s grace and closes the connection as ambiguous, the spawn_blocking task keeps its blocking-pool permit until discovery returns.

**Evidence**

- [`crates/foks-oidc/src/provider.rs:159`](../../../crates/foks-oidc/src/provider.rs#L159): `Provider::discover` calls `http.get(config_uri)` and then `http.get(metadata.jwks_uri())` at line 179, with no deadline or cancellation parameter.
- [`crates/foks-oidc/src/provider.rs:108`](../../../crates/foks-oidc/src/provider.rs#L108): `ProviderHttp::new` fixes `.timeout` and `.connect_timeout` at PROVIDER_DEADLINE_MS (10 s, lib.rs:17) per request.
- [`crates/foks-agent/src/sso.rs:18`](../../../crates/foks-agent/src/sso.rs#L18): The agent constructs `ProviderHttp::new(NetworkPolicy::default())` although `timeout` and `cancellation` are handler parameters in scope.
- [`crates/foks-client/src/sso.rs:85`](../../../crates/foks-client/src/sso.rs#L85): begin_sso calls `Provider::discover` before journaling the flow. validate_sso_result does the same at line 362.
- [`crates/foks-client/src/transport.rs:251`](../../../crates/foks-client/src/transport.rs#L251): OperationControl's deadline is `Instant::now() + self.timeout` per RPC (also at 513-516). There is no operation-wide remaining budget.
- [`crates/foks-agent/src/main.rs:1720`](../../../crates/foks-agent/src/main.rs#L1720): On timeout, supervise_blocking cancels, waits CANCELLATION_GRACE and returns DeadlineExceeded with close_connection, while the spawn_blocking closure still holds the blocking permit.

**Recommendation**

Give ProviderHttp an optional control, for example `ProviderHttp::with_control(policy, deadline: Instant, cancellation: CancellationToken or Arc<AtomicBool>)`, and keep `new` unchanged for the server. In `get`, check cancellation before each request and set `RequestBuilder::timeout(min(deadline - now, PROVIDER_DEADLINE))`. Return new foks_oidc::Error variants (Cancelled, DeadlineExceeded), and map them in foks-client to Error::Cancelled and Error::DeadlineExceeded instead of the generic Error::Oidc(ProviderUnavailable). Construct the controlled client in the agent's sso::handle from its `timeout` and `cancellation`. Alternatively, have begin_sso and validate_sso_result build the control from the FoksClient's own per-RPC timeout and token; do not add a nonexistent 'remaining operation budget'. Apply the same control to the SyncHttpClient::call path used for token exchange. Add a loopback test (NetworkPolicy::loopback_test) whose JWKS response is delayed past a short deadline, and assert that discover returns the deadline error within the budget and that a pre-cancelled token returns Cancelled without a request being sent.

<details><summary>Verifier note</summary>

The core claim holds. ProviderHttp::new (provider.rs:108-117) builds a blocking reqwest client with a fixed 10 s total and connect timeout. Provider::discover (provider.rs:159, not 153, which is the struct) runs http.get(config_uri) and then http.get(jwks_uri) at line 179, with no deadline or cancellation input. begin_sso (sso.rs:85) and validate_sso_result (sso.rs:362) call it. The agent's sso::handle (foks-agent/src/sso.rs:18) creates ProviderHttp with NetworkPolicy::default(), even though `timeout` and `cancellation` are in scope there. SSO is dispatched with the at-most-15 s request budget (main.rs:1255-1258, default request_timeout_seconds=15). supervise_blocking (main.rs:1655-1740) cancels the token, waits CANCELLATION_GRACE (1 s), returns DeadlineExceeded with the connection closed as ambiguous, and the blocking permit stays held inside spawn_blocking until discovery returns, which can take up to about 20 s. ISSUES.md and the book do not track this. The recommendation is inaccurate in two ways. FoksClient has no per-operation remaining deadline: transport.rs:251-254 and 513-516 set `deadline = Instant::now() + self.timeout` for each RPC, so `FoksClient::operation_budget()` returning a remaining deadline has nothing to return. Also, foks_oidc::Error has no deadline or cancellation variant and converts to foks_client::Error::Oidc (error.rs:12), so the proposed test asserting Error::DeadlineExceeded would fail unless a mapping is added. Note also that in begin_sso, discovery runs before the flow is journaled, so a timeout there is not actually an ambiguous mutation; the agent only reports it as one. The server also uses ProviderHttp::new (foks-server/src/sso/mod.rs:57, config.rs:99), so its behaviour must not change.

</details>
