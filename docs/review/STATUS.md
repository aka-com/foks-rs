# Implementation status

Status of the review findings on branch `claude/focused-mccarthy-3amy72` as of 18 October 2026. 75 of 165 findings are fixed and 12 are partly fixed. Each change was reviewed by two independent reviewers, corrected and re-reviewed. The explorer page (`index.html`) lists only the findings that are still open or partly open.

Every "what remains" and follow-up entry below was re-checked against commit `1876dff`. The first version of this file (`ab905f7`) was compiled from notes taken when each finding was implemented, and listed some work as open that later commits had completed; corrected entries name the commit that did the work.

Work was scoped as follows:

- Top recommendations: all except `desktop-native-auto-lock` (with `fe-product-auto-lock`) and `server-merkle-epoch-budget`.
- Phase 1 (chat UI on current IPC data): all medium findings and the low findings that were straightforward.
- Phase 2 (vault and desktop UX): all findings except text size (`gap-a11y-text-size`) and auto-lock, without new CLI or agent API surfaces and without breaking Go FOKS v0.1.9 compatibility.
- Phase 4 (client, protocol and server robustness): medium and high findings that are not high risk or high complexity.

## Partly fixed

Each entry lists what remains.

### `client-libs-operation-scoped-context`

client-libs-operation-scoped-context ([finding](findings/client-libs.md#client-libs-operation-scoped-context))

- Steps 1b-3 of the finding remain open: switching the remaining HardStateStore::open sites (about 175, mostly inside checked sessions) to open_existing or a renamed create_or_open, changing MutationCoordinator to borrow &mut HardStateStore instead of a path, and replacing the SESSION_MATERIAL / HELD_CHECKED_PROFILES thread-locals and raw master_key/vault parameters with an operation-scoped context owned by CheckedProfileSession. These are the L-sized parts the triage excluded from this pass.
- readiness::validate_for_proof and checked_profile_for_use still use HardStateStore::open; they run only inside an import or checked session after admission has created the database, so they were left unchanged.
- merkle_maintenance's try_exists guard and the reset preview's exists() guard are still per-site checks; they could move to open_existing in the later pass.
- SoftStateStore has no open_existing counterpart yet.

### `desktop-native-operation-builders-triplicated`

desktop-native-operation-builders-triplicated ([finding](findings/desktop-native.md#desktop-native-operation-builders-triplicated))

- The rest of Step 2: moving the other *_operation builders (execution.rs, yubikey.rs, portability.rs, vault.rs) and the servers.rs response validators into foks_desktop::ops. These are desktop-internal and allowed, but they were not done in this pass; the finding suggested starting with groups.
- The rest of Step 3: since 9b4a922 the backend's account creation and resume, owner backup and owner recovery commands call the shared foks_desktop::ops::accounts builders, and its passphrase confirmation uses the shared confirmed_passphrase. Its passphrase and YubiKey commands still build their operations separately, and the foks-desktop README and ISSUES.md say so. Some Tauri rules, such as promote/demote ordering, need input the backend command line does not take, and adding it would need new backend flags, which are not allowed.
- The federation builders (add_federated_team_member_operation and rerun_federated_team_member_add_operation) stay in Tauri. The add builder does no validation; the rerun builder only checks that the stored destination role is Member, and it takes a Tauri-specific DTO (FederationEntryDto).

### `desktop-native-startup-reset-safety`

desktop-native-startup-reset-safety ([finding](findings/desktop-native.md#desktop-native-startup-reset-safety))

- Recommendation part 2 (an agent-reported failure class through distinct exit codes, for example 65/75/73, or a startup-failure.json) is not implemented; the agent still exits with status 1 for every startup failure. The credential-store case is handled without it since 3791069: before offering a reset, startup runs the existing non-interactive check (require_auto_recovery_root under without_user_interaction), and when that reports agent-credentials-required the dialog offers Try Again only. Other exit causes are still not told apart from unreadable state, so the reset dialog still tells the user to choose Try Again if the agent output reports a locked or denied credential store.
- Show Log as a separate button: Tauri message dialogs allow at most three buttons, so the dialog shows the agent.log path and the agent output instead.
- Permanent deletion of a quarantined state directory as a separate in-app action: not added. The dialogs tell the user the moved directory still holds local keys and that they can delete it themselves.
- The automatic-recovery relaunch wait in maintenance.rs (auto_recover_with_reservation_timeout, also 50 x 100 ms) is unchanged. Changing its timeout error code would change the renderer's automatic-recovery behaviour, and the finding cites only the startup loop. Restoration after export, import or relocation is not affected: it starts the agent through the startup path (ensure_started_already_reserved) and waits up to 60 seconds.

### `fe-arch-css-structure`

fe-arch-css-structure ([finding](findings/fe-arch.md#fe-arch-css-structure))

- The teams rules moved out of app.css in 758f006 (now src/features/teams/teams.css, layer features.teams) and the settings and account page rules in 8195c5b (now src/features/settings/settings.css, layer features.settings); styles-geometry.test.ts fails if app.css selects a class that only those pages render. Still in app.css are rules that more than one feature renders: the account panel rules (.pcard/.panel-body .inset.form, .go-account-*, the account sheet .panel-body rules), the recovery-phrase checks (.recall-*, first-run and Devices), the exposure-review list (.exposure-rows, Teams and Devices sheets) and the PartyRow rules (.who2), which 758f006 kept as component rules. Moving any of them needs the same conflict check and Chromium comparison.
- Recommendation step 4 (feature prefixes for short classes when their feature moves). The first-run selectors mostly use shared primitive classes (.fr, .k, .v, .a, .t, .crit, .band), which cannot be renamed for one feature. The first-run-only short names (.res, .facts, .pb, .foot-slot, .alt-path, .choice-body, .bd, .pip, .err) were not renamed, because that touches TSX and tests. The suggested '.fr-' prefix would also collide with the existing .fr (field row) class.
- The two `.first-run-main .crit` rules are broad enough that two component cases needed restating in first-run.css. Scoping them to the first-run error elements (div.crit / p.crit outside cards) would remove the restatements, but it is a selector redesign rather than a move.

### `fe-arch-feature-folders`

fe-arch-feature-folders ([finding](findings/fe-arch.md#fe-arch-feature-folders))

- The feature folders are done (1e2a3bb..e5bc668): src/features/{chat,devices,files,first-run,settings,teams}/index.ts exist, the domain panels moved into features/teams/invitations/ and features/settings/account/ (including sso/), and layers.json, eslint.config.mjs and module-boundaries.test.ts require that a module outside a feature reaches it only through its index.ts. Not done: one README per feature folder; apps/desktop/README.md still describes the folders in its single layout table.
- Moving bridge/, diagnostics/ and scheduling/ physically into platform/ (and renaming components/ to ui/). layers.json puts diagnostics/ and scheduling/ in the same layer as platform/ ('data and platform'); bridge/ has a layer of its own. Their paths are unchanged, and platform/ holds only appearance.ts.
- navigation/ absorbing location.ts and navigation-guard.tsx.
- Moving chat-contract.ts and chat-limits.ts beside chat/ (the chat restructuring, to be coordinated with the chat owner).
- Breaking the chat runtime cycles that are allow-listed in module-boundaries.test.ts: inbox-provider imports channel-creation-provider and unread-badge, and both import inbox-provider's hooks. Moving the context and its hooks into their own module would remove them.
- The 39 files still at the src/ root (including main.tsx and lucide-icons.d.ts) are assigned to no layer in layers.json, so no layer's forbid list applies to them; module-boundaries.test.ts checks only that they reach a feature through its index.ts.

### `fe-arch-first-run-session`

fe-arch-first-run-session ([finding](findings/fe-arch.md#fe-arch-first-run-session))

- Recommendation 2, the hooks' parameters, is done in part (d6dd80c): useServerWorkflow, useTeamDiscovery, useAccountOperations, useAccountIdentity, useAccountActions, useProtectionActions and useSetupRestart take one SetupCore value (setup-core.ts) that useFirstRunSession builds per render, useSetupReconciliation takes it without fail, and each hook's other parameters are only what it adds (useAccountOperations now takes core, busy, onAgentReadinessFailure and onReplaceSession). The hooks still receive SetupCore as a parameter instead of reading it from a context, because they run inside the component that creates the SetupContext. Reading it from a context needs a second provider level (one core context for the checkpoint, message and fail, with a lower provider for the workflows).
- The four first-run copy buttons still call `void bridge.copyText(...).then(toast)` with no rejection handler. Behaviour was kept as is because that is the separate finding fe-arch-copy-action.

### `fe-arch-mock-bridge-contract`

fe-arch-mock-bridge-contract ([finding](findings/fe-arch.md#fe-arch-mock-bridge-contract))

- Convert the remaining mock domains to wire-level handlers. Since d8963e9, a093fcc and 141f43c the mock host answers the account listing and team and federation changes, every server command except list_servers, the enrollment commands and all sixteen runYubi commands with the card listings. Still answering decoded values: core/application with chat and desktop alerts (src/mock/core.ts), list_servers, listGroupDetails, listParties and listFederation (the fixture's records carry facts the native responses lack), openWebAdmin, setLocalAccountAlias, renameAccount and adapterClock (they reject with plain Errors and need the native CommandError first), and invitations (src/mock/invitation-mock.ts). Events do not go through mockHost.listen, which returns a no-op unsubscribe.
- Use the src-tauri/wire-contract/*.json golden records as seed responses for the wire-level mock.
- Recommendation step 4 (generate_handler! parity test) needs no new work: tests/command-contract.test.ts already checks every native call's command name and argument keys against src-tauri/command-signatures.json in both directions.
- Decide separately whether the invitation action 'inbox-count' should be in INVITATION_READ_ACTIONS; it was kept as a mutation to preserve behaviour.

### `fe-arch-render-test-harness`

fe-arch-render-test-harness ([finding](findings/fe-arch.md#fe-arch-render-test-harness))

- Step 4: moving tests into tests/unit/<feature>/ and tests/render/<feature>/, splitting bridge.test.ts (3,873 lines) by bridge/commands-*.ts domain, and changing the npm glob to a quoted 'apps/desktop/tests/**/*.test.{ts,tsx}'. Moving the 209 test files would conflict with work in flight on other lanes, so it should be done in its own pass.
- Static top-level imports in render tests. They still use typed `await import()` after installDom, which keeps the old evaluation order. Static imports need installDom to run before the imports are evaluated (for example a setup module imported first), plus a per-file check that react-dom and @testing-library/react do not capture globals before jsdom is installed.
- A typed tests/lib/vite-harness.ts load<T>(path) was not added: only app-root.render.test.tsx still needs Vite. It loads main.tsx for its side effects and casts one other module, app/deferred-screens.tsx, to call preloadVaultScreens.
- chat-text.test.tsx and chat-notification.test.ts could also move to static imports; they currently use typed dynamic imports.

### `fe-arch-screen-split-shared-primitives`

fe-arch-screen-split-shared-primitives ([finding](findings/fe-arch.md#fe-arch-screen-split-shared-primitives))

- Splitting the remaining large views into model, sheets and view. write-workflows.tsx (129 lines; 69c8d0b, 51c734e), settings-screen.tsx (217; 9223b34), servers-screen.tsx (566; 9223b34, 8ec9aa7, with AddServerSheet, RenameSheet and RemoveSheet in settings/servers/) and account-section.tsx (582; 340ae2a) are now under the limit. The max-lines warning (800 lines, eslint.config.mjs) still lists eight .tsx modules: items-screen.tsx (2,016 lines), chat-teams.tsx (1,663), details-panel.tsx (1,231), vault-shell.tsx (1,181), search-palette.tsx (1,099), devices-screen.tsx (1,038), teams-screen.tsx (914) and groups-screen.tsx (877, of which GroupSettingsScreen is about 760). items-screen, devices-screen, groups-screen and details-panel were split in part by 5c49ed4, ba5cc2f, 0ffdccf and 4bb97c3.
- Moving canTarget and demotionFor from groups/sheet-support.tsx into group-model.ts.
- Deciding whether the details panel's Sharing rows should use the member-row role wording ('Member · sees level N'). They currently keep 'Member · visibility N' to match the item access line above them; switching is a product decision.

### `fe-arch-shell-services-context`

fe-arch-shell-services-context ([finding](findings/fe-arch.md#fe-arch-shell-services-context))

- ItemsScreen (96d2291), DevicesScreen, SettingsScreen, TeamsScreen and GroupSettingsScreen (557c879) read the shell services and snapshot from context. AccountSection, ServersSection, ChatTab and FirstRunExperience, and their panels and sub-sections, still take bridge, snapshot and error handlers as props; the recommendation leaves screen props for last.
- DetailsPanel reads the snapshot, bridge, lease clock, refresh and both error handlers from the shell services since 5e7d84c. It still takes accessGeneration, accessSession and accessTicket as props, because they belong to the selected item's server and the current unlock; ShellServices has no accessTicket member, which the recommendation lists.
- Other sheets and panels still take bridge and error props, among them AbandonGroupSheet, FederationRemovalSheet, NewChatSheet, the server sheets and the invite, SSO, rename, bot and admin panels. RecoveryPhraseTestSheet reads the bridge from the shell services since 7609266 and keeps its onError prop like the other device sheets; TeamInfoPanel reads the services since 5e7d84c.
- The navigate and copy members the recommendation lists are not part of ShellServices yet. Copy depends on fe-arch-copy-action.
- One error policy for device and group sheets, so onError, onApplied and onMutationError could come from context, overlaps with fe-arch-mutation-action-hook.
- The migrated sheets still use their old prop names as local aliases (for example `refresh: onApplied`); renaming them is follow-up cleanup.

### `gap-security-key-pin-field`

gap-security-key-pin-field ([finding](findings/gap.md#gap-security-key-pin-field))

- Recommendation step 3, needs a forbidden surface: a new ErrorCode::PinRejected with ErrorFields.remaining and ErrorCode::PinBlocked in foks-agent-proto, mapped in the agent and in desktop transport.rs. Until then the renderer reads the attempt count from the operation-failed message text, and the shared JSON fixture checks that text against foks_yubi.
- Recommendation step 5 (optional), not done: calling yubi_pin_status before the first submission to show attempts up front. The team, invitation, bot and rename prompts hold no key alias, so the call would first need the enrolled alias resolved from the device cache.
- The blocked-PIN note names the path in text (open the server in Settings › Account, then Unblock PIN… under Security keys; the path was corrected in f0254b0) instead of linking to it; these panels have no navigation hook.

### `gap-storage-usage-display`

gap-storage-usage-display ([finding](findings/gap.md#gap-storage-usage-display))

- Showing the last-known usage next to a quota-exceeded refusal. This would need no forbidden surface, but quota refusals come out through many separate write paths: commandError toasts, write sheets and bulk/import/copy sheets that report inline with report:false, and uploads without an item. Most of them do not know which store the failed write targeted. Doing it properly means changing each workflow, so it was left out to keep this change bounded. The existing refusal text is unchanged.
- A cheaper server-side usage read (the recommendation relies on server-storage-usage-ledger). That is server-side work tracked elsewhere and outside this desktop-only scope.

## Left out of Phase 4

Triage excluded these medium or high findings as high risk or high complexity:

- [`client-libs-acting-credential`](findings/client-libs.md#client-libs-acting-credential): client-libs-acting-credential. Reason: high risk and complexity.
- [`client-libs-team-intent-layering`](findings/client-libs.md#client-libs-team-intent-layering): client-libs-team-intent-layering. Reason: high risk and complexity.
- [`protocol-hybrid-seal-duplication`](findings/protocol.md#protocol-hybrid-seal-duplication): protocol-hybrid-seal-duplication. Reason: high risk (crypto core).
- [`protocol-verify-shared-chain-step`](findings/protocol.md#protocol-verify-shared-chain-step): protocol-verify-shared-chain-step. Reason: high risk and complexity (verification core).
- [`server-storage-usage-ledger`](findings/server.md#server-storage-usage-ledger): server-storage-usage-ledger. Reason: high risk and complexity.
- [`chat-backend-stage1-redundant-envelope-and-counters`](findings/chat-backend.md#chat-backend-stage1-redundant-envelope-and-counters): chat-backend-stage1-redundant-envelope-and-counters. Reason: high risk and complexity (server write path and schema).
- [`server-merkle-incremental-prepare`](findings/server.md#server-merkle-incremental-prepare): server-merkle-incremental-prepare. Reason: high risk and complexity (Merkle prepare path).

## Fixed

Follow-ups recorded during implementation are listed under each finding.

### `agent-ipc-version-handshake`

agent-ipc-version-handshake ([finding](findings/agent.md#agent-ipc-version-handshake))

Follow-ups:

- Step 4 (accepting [N-1, N]) is deferred, as the recommendation itself advises without per-version response shaping and as the verifier note says is under-specified: many proto types reject unknown fields, so the agent would need to decode requests of each version and answer each in the request's version. SUPPORTED_PROTOCOL_VERSIONS stays at exactly PROTOCOL_VERSION, and hello.rs documents that rule for any later widening.
- The desktop does not pass minimum_protocol/maximum_protocol through to the renderer's error details; the error message already contains both versions, and passing the range would extend the TypeScript error contract for no current consumer.
- Desktop agent_status runs a Hello only when no reply is cached. A probe of a started, recovered or replaced agent refreshes the cache, but an agent replaced by an outside launcher at the same protocol version without a probe would keep the old capability set until the next probe.
- The desktop transport test agent::tests::transport::observed_cancellation_and_deadline_leave_the_next_call_healthy failed with ConnectionReset because of a bug in the test itself (the fake agent set the cancellation flag before the client read the length-prefix bytes it had written); 5c8c6a7 fixed the test, and it passes at HEAD. A future failure of this test is a regression.

### `build-crate-layer-boundaries`

build-crate-layer-boundaries ([finding](findings/build.md#build-crate-layer-boundaries))

Follow-ups:

- Removing the foks-desktop-app -> foks-client-app/keystore/yubi edge. It would need an agent operation for import verification, plus a lighter shared crate for the state leases, readiness and reset functions and the validators (normalize_profile_label, validate_local_alias, Capability, login_field). The finding calls this optional follow-up.
- Moving the CLI's in-process commands onto the agent, which would remove its exception.
- The doctor and Python-version prerequisite check belongs to a separate build finding.

### `build-deps-supply-chain-audit`

build-deps-supply-chain-audit ([finding](findings/build.md#build-deps-supply-chain-audit))

Follow-ups:

- Release jobs check advisories once before building (cargo-deny via workflow_call) and verify that the manifest is embedded. They do not run `cargo audit bin` on the built binaries: that would need a second ignore list beside deny.toml. Scanning shipped binaries later with `cargo audit bin` is documented in RELEASE_POLICY.md.
- No Go vulnerability scan (govulncheck) was added; the recommendation does not ask for one. Dependabot covers Go modules, but go-foks, go-snowpack-rpc and go-codec are ignored so the oracle stays on Go FOKS v0.1.9 (a compatibility constraint).
- cargo-deny, cargo-auditable and rust-audit-info are installed in CI with `cargo install --locked` at pinned versions, which matches how the repo already installs tauri-driver and cargo-fuzz. Switching to prebuilt binaries or caching belongs to finding build-ci-desktop-cost.
- Duplicate crate versions are only warned about; collapsing them and moving the local sha2 0.10 pins into the workspace belongs to protocol-crypto-dependency-hygiene. fs2 0.4.3 has no RustSec advisory, so cargo-deny does not flag it, and it is unchanged.
- The session hook (.claude/hooks/session-start.sh) still runs `npm ci --no-audit`. It only sets up containers; the enforced audit now runs in CI.
- The `npm audit signatures` registry signature check was not added: it failed with 'Failed to download' in this environment, so it could not be verified locally.

### `chat-backend-capacity-refusals-become-uncertain`

chat-backend-capacity-refusals-become-uncertain ([finding](findings/chat-backend.md#chat-backend-capacity-refusals-become-uncertain))

Follow-ups:

- STATUS_RATE_LIMIT_ERROR (1012) is not added to definite_rejection. The per-source request limiter refuses at the call's sequence number (it refused at sequence 0 before r2-compat-rate-limit-refusal-sequence-zero), and execute_bounded and the poll semaphore also return 1012. Treating 1012 as a definite rejection needs a check that no FOKS-RS or Go path returns it after a realtime commit.
- These remaining realtime mutation refusals still map to 12001: the channel-metadata size check in rt_create_channel (Capacity("realtime channel metadata")) and the stored-size checks in messages.rs ("realtime message", "realtime channel activity"). The verifier rated the stored-size checks effectively unreachable because proto validation already bounds them; they could become 1030 or 1060 later.
- Optional extras from the recommendation are not done: a client- or desktop-side warning before creating at the channel cap, a user-initiated 'retry exact' for Uncertain sends, and a conformance test for fanout above 1,024 members (it needs a non-default maximum_team_members; the fanout case is covered at the foks-server-db level).

### `chat-backend-single-message-poisons-channel`

chat-backend-single-message-poisons-channel ([finding](findings/chat-backend.md#chat-backend-single-message-poisons-channel))

Follow-ups:

- On Go v0.1.9 hosts, a member can still stop a channel. Unlike the Rust server, that host's send path checks neither predecessor pointers nor whether an ID is already used. A member there can submit an authenticated text message whose predecessor pointer contradicts the server's (sequence, ID) mapping, or reuse an existing message ID. The finding's recommendation explicitly keeps both page-fatal, because the client cannot tell them apart from a server substituting a row. The book chapter now documents this; removing it would need a Go server change or giving up substitution detection.
- A message encrypted under a key generation the reader has not loaded yet (the short window between loading the team and fetching history) now shows as unverifiable instead of returning a retryable error. The desktop history cache keeps that placeholder until the conversation reloads in full. The recommendation lists a missing key generation as unverifiable, and a Go host does not check the generation, so making it a retryable error would let a member stop the channel again. Refreshing the team and retrying the read automatically is left for later.
- Not addressed: two subagent review findings in `apps/desktop/src/chat/send-service.ts` and `outgoing.ts`. They are about how earlier commits handle preparation errors and stop states, not about this change.

### `chat-code-backoff-formulas`

chat-code-backoff-formulas ([finding](findings/chat-code.md#chat-code-backoff-formulas))

Follow-ups:

- The send scheduler's flat 2 s/5 s team gate waits (TEAM_INTERVAL and TEAM_RETRY in apps/desktop/src/chat/send/scheduler.ts) are not backoffs and were left as they are. The success re-check interval reads REQUEST_RETRY.base since 1c81547 instead of an inline literal.
- book/docs/research/research-desktop-overview.md still names the old FALLBACK_MS constant with line numbers; it is a dated research snapshot whose line references were already stale, so it was not edited.

### `chat-code-conversation-hook-api`

chat-code-conversation-hook-api ([finding](findings/chat-code.md#chat-code-conversation-hook-api))

Follow-ups:

- Nothing in this finding needs agent, IPC, client-library or protocol changes, so nothing is deferred to later phases.
- The three chat access guards (the send gateway, use-chat-conversation's history guard and channel-creation) raise the same local accessChanged() error since 18fe8e3 and e3f1991 (finding chat-code-local-access-errors), but each still performs its own check; no shared guard helper exists. This change only removed the dead ambiguity branch from the conversation guard.

### `chat-code-ipc-contract-parity`

chat-code-ipc-contract-parity ([finding](findings/chat-code.md#chat-code-ipc-contract-parity))

Follow-ups:

- Recommendation part (3): once the golden wire-contract inventory covers chat, move chat out of uncoveredPublicShapes in apps/desktop/src-tauri/wire-contract/inventory.json and update the assertion in wire-contract-domains.test.ts. The finding makes this conditional on golden-fixture coverage, which does not exist yet.
- No ChatAction::result_kind() method was added to foks-agent-proto. The Rust side of the result-kind mapping is checked through validate_chat_reply against reply cases covering every action. A dedicated proto helper would be an agent-crate API addition and can be added in a later phase if it is wanted.

### `chat-code-local-access-errors`

chat-code-local-access-errors ([finding](findings/chat-code.md#chat-code-local-access-errors))

Follow-ups:

- The shared chat/access-guard.ts module from the recommendation was not created; the send gateway, use-chat-conversation.ts's history guard and channel-creation.ts each keep their own check. The history guard raises accessChanged(..., 'read') since e3f1991, so a history read refused after it was sent stays unambiguous.
- The local error factories (cancelled, integrity, channelIntegrity, limit) were not moved to typed CommandError values in chat/errors.ts.

### `chat-code-mock-bridge-decoder`

chat-code-mock-bridge-decoder ([finding](findings/chat-code.md#chat-code-mock-bridge-decoder))

Follow-ups:

- The decodedChatBridge(fake, team) test helper exists (tests/lib/decoded-chat-bridge.ts, 2a714a9), but only chat-notification.test.ts uses it; the other ad-hoc chat fakes are not migrated. Many render and unit fakes build replies with non-hex IDs ('c', 'host'), or inject already-decoded rows (for example 'zero:4' in chat.render.test.tsx). Decoding is not idempotent for zero-ID rows, so wrapping every override would need those fakes rewritten. This is test-infrastructure follow-up work, not a later phase.
- Removing NotificationConsumer's duplicate 50-row and 256-character check (notification-consumer.ts around line 495). The finding says to remove it only once every fake goes through decoding, so it stays.
- The mock does not map history text over CHAT_TEXT_BYTES to 'oversized' as the agent does. The composer already blocks sends over that limit, so it is left out.

### `chat-code-notification-limit-constant`

chat-code-notification-limit-constant ([finding](findings/chat-code.md#chat-code-notification-limit-constant))

Follow-ups:

- book/docs/research/research-desktop-overview.md:201 still says 'max 256 chars per notification text'. It is a dated research snapshot (it cites HEAD 2ff0918), so it was left unchanged.
- Nothing tests the notification consumer's own bound check (notification-consumer.ts, 'Invalid notification history bound.') separately from the decoder it runs after. No harness for it was added.

### `chat-code-outgoing-state-machine`

chat-code-outgoing-state-machine ([finding](findings/chat-code.md#chat-code-outgoing-state-machine))

Follow-ups:

- The rejection-copy table (delivery.ts, 044142c), the unsent header chip (77d6b17) and service-owned action availability (outgoingActions(), operationActions(), deliverOperation() and cancelOperation(), 4f26035/653fdee) were built under chat-ux-delivery-states and chat-code-pending-row-actions. The per-row view model (one rows() with the row actions) is not built; it is a follow-up of chat-code-pending-row-actions. OutgoingRow and HistoryRow render through MessageRow; PendingRow does not.
- Problems the review subagents found that predate this change and that were not fixed, because they are outside this finding:
- (a) A saved message with no operation, revoked in its channel, loses its text and keeps its intent, which blocks that channel's queue.
- (b) restoreDraft returns silently when the intent clear fails, after which recovery sends the message.
- (c) 'Retry local cleanup' calls retry(), which does nothing for rejected or cancelled operations and starts a full send for a message with no operation.
- (d) A 'waiting' message that is only in memory survives revocation as a row with no text.
- (e) restoreDraft checks that the draft is empty before its awaits and writes it after them, so text typed meanwhile is overwritten.
- (f) The terminal-state test is still defined twice: the exported terminal() in chat/send/outgoing.ts and a local terminal in operations.ts. The copy in recovery-schedule.ts went with that module in 0488d01.
- (g) A non-attempt drive failure whose status request also fails gets no automatic re-check.

### `chat-code-pending-row-actions`

chat-code-pending-row-actions ([finding](findings/chat-code.md#chat-code-pending-row-actions))

Follow-ups:

- The recommendation's per-row view model (one rows(store, channel) with retry/check/cancel/restore actions from one operation and phase transition table) is not built. thread-rows.ts already merges the lists; row actions come from two service queries instead of one table: ChatSendService.outgoingActions() for outgoing messages (over phaseOf()/outgoingActions() in chat/send/outgoing.ts, the state machine from chat-code-outgoing-state-machine, 5eb8a2c) and operationActions() for operation rows.
- restoreDraft() and retry() keep their own guards and do not call outgoingActions(). retry() is unchanged; restoreDraft() also refuses a message whose tracked operation may have reached the server (653fdee), the case in which the row offers no Edit.

### `chat-code-render-purity`

chat-code-render-purity ([finding](findings/chat-code.md#chat-code-render-purity))

Follow-ups:

- 'react-hooks/refs' is only on for the chat files. Turning it on for all of apps/desktop/src would report 107 more sites in 27 files at HEAD.
- 'react-hooks/purity' is not enabled. In the chat files it reports one site, Date.now() during render in src/features/chat/chat-teams.tsx:545; changing that clock read is a separate task.
- The full ChatScreen render test the finding suggests (mount ChatScreen, check no Team record is created and the history cache order is unchanged) was not added. The Team-record half no longer applies because drafts() was already removed. The cache-order half is covered by the history-cache unit test, the lint rule and a render test that reads every retained window on each render of a probe component using useChatConversation (tests/chat.render.test.tsx:4409, f649ffd), not by mounting ChatScreen.

### `chat-code-scheduling-primitives`

chat-code-scheduling-primitives ([finding](findings/chat-code.md#chat-code-scheduling-primitives))

Follow-ups:

- Nothing in this finding needs agent/IPC, client-library or protocol changes, so nothing is deferred to a later phase.

### `chat-code-send-revision-rerender`

chat-code-send-revision-rerender ([finding](findings/chat-code.md#chat-code-send-revision-rerender))

Follow-ups:

- Per-channel useOutgoing(store, channel) snapshots and a React.memo message list with stable row inputs. These only limit re-renders caused by other publications (operations, recovery, errors), not keystrokes, which no longer re-render the thread. Phase 1 scope; left out to keep the change focused.
- Keeping the retained-message and observation byte totals by delta in canRetain and observeHistory (optional in the recommendation). Since 2b2422c each text is encoded once and its length kept (outgoingBytes, observation.bytes); the totals are still summed over the kept lengths on each call, which runs per submit or history page, not per keystroke.

### `chat-code-send-service-decomposition`

chat-code-send-service-decomposition ([finding](findings/chat-code.md#chat-code-send-service-decomposition))

Follow-ups:

- Nothing in this finding needs agent/IPC, client-library or protocol changes; it is entirely desktop-ui.
- Not done here because it belongs to other findings: canRetain() and observeHistory() still add up byte lengths on every call (chat-code-send-revision-rerender).

### `chat-code-vestigial-modules`

chat-code-vestigial-modules ([finding](findings/chat-code.md#chat-code-vestigial-modules))

Follow-ups:

- Optional, from the finding: tail fairness for new arrivals in the recovery job ordering (nextRecoveryJob in chat/send/recovery.ts, moved out of ChatSendService.recover in ab0282b): give unseen keys a served turn instead of due 0. Not implemented because it changes current scheduling behaviour; it needs a design decision. No phase-2+ (agent/IPC/protocol) parts apply to this finding.

### `chat-ux-channel-audience`

chat-ux-channel-audience ([finding](findings/chat-ux.md#chat-ux-channel-audience))

Follow-ups:

- Nothing in this finding needs new agent or IPC fields, client-library changes or protocol changes, so no later-phase work is left over.
- The mockup's 'See who can read' link and reader avatars in the empty conversation were not built. They are UI-only, but the finding's recommendation does not ask for them.

### `chat-ux-channel-list`

chat-ux-channel-list ([finding](findings/chat-ux.md#chat-ux-channel-list))

Follow-ups:

- Mute / Unmute channel in the context menu is built by chat-ux-local-mute (ec5e075), with the device-local muted set in chat_local. Muted rows are dimmed, captioned 'Muted' and show a grey count; the mockup's separate muted glyph on the row is not drawn.
- The unsent-draft glyph is built (pencil and 'Draft' mark, e24f344 in chat-ux-composer). The mockup's 'Draft:' preview line with the draft text is not built; this finding's recommendation steps do not include it.
- Copy link to channel: the verifier note says it is infeasible as written. The app registers no URL scheme, and in-app locations carry local store references. It stays out until a URL scheme exists whose links carry no local identifiers.

### `chat-ux-composer`

chat-ux-composer ([finding](findings/chat-ux.md#chat-ux-composer))

Follow-ups:

- Recommendation step (7): save-draft and load-draft agent chat actions that keep drafts in the agent's protected store, separate from SaveIntent and cleared on account removal and team replacement, so drafts survive lock and restart. This needs new agent/IPC actions. Until then drafts stay in memory, as the README now states.
- Mockup state B's toolbar buttons (bold, italic, code, code block, quote, list) are not in the finding's recommendation and were not built. Only the keys are implemented.
- Mentions as a protocol feature (notifications, counts) remain deferred per ISSUES.md. The completion inserts plain Basic text only.

### `chat-ux-delivery-states`

chat-ux-delivery-states ([finding](findings/chat-ux.md#chat-ux-delivery-states))

Follow-ups:

- Per-message integrity marks (recommendation 4): needs a new agent/IPC field `predecessor_unchecked` on each ChatMessage, computed in the agent from metadata.previous_sequence against missing_predecessors and validated in chat-contract.ts. After that, replace the page-level warning band with a 'History partly checked' header chip that scrolls to the first marked row. The page-level band is kept for now, as the finding says.
- An agent-side `rejection_reason` field beside rejection_code in ChatOperation was not added; the closed set is mapped in the desktop instead, which the finding allows as an alternative.
- Jump from the unsent chip opens the target channel but does not scroll to and highlight the specific row; the mockup suggests that.
- Agent chat error text is not written into the diagnostic timing log, because that log's contract forbids error messages. Only the code is logged, as before. The text stays on the error object and goes into 'Copy diagnostic details' for a stopped session.
- Related, not done here: the rows() view model from chat-code-pending-row-actions is still not built. Service-owned row actions (4f26035, 653fdee), attempt tokens in the outgoing state machine (5eb8a2c) and 1060 in the rejection table (80b0136) are done.

### `chat-ux-history-search`

chat-ux-history-search ([finding](findings/chat-ux.md#chat-ux-history-search))

Follow-ups:

- Step 2 (agent/IPC): a bounded, agent-side backward scan over verified pages across channels (chat-backend-history-search), with a new `search` ChatAction returning per-channel scanned_through and skipped-row counts; mockup states D, F and G.
- A 'Messages' scope in the ⌘K search palette (search-palette.tsx) and opening palette results, which depend on the agent scan (mockup states D and E from the palette).
- Any persisted message index: it must be sealed under the vault key, not plain FTS5, and the change to the no-durable-plaintext rule must first be recorded in book/16-chat.qmd.
- The topbar returns to the channel scope only on another channel or after the name filter is cleared, not on every reselection of the same channel as in the mockup; minor polish.

### `chat-ux-history-window-sliding`

chat-ux-history-window-sliding ([finding](findings/chat-ux.md#chat-ux-history-window-sliding))

Follow-ups:

- Nothing in this finding needs a new agent/IPC field, client-library or protocol change, so no later-phase work is deferred.
- Not done (outside this finding's scope): message rows construct new Intl.DateTimeFormat objects on every render (presentation.ts messageTime/messageClock), which is the main cost of re-rendering a window of up to 400 rows. Caching the formatters would also need the 'typing in the composer draws no message row again' test to detect re-renders another way.

### `chat-ux-keyboard-a11y`

chat-ux-keyboard-a11y ([finding](findings/chat-ux.md#chat-ux-keyboard-a11y))

Follow-ups:

- The mockup's visible key hint under the list while a row has focus was not built, because it would overlap the centered Jump to latest control. The README documents the keys instead.
- Text size, zoom and rem font tokens from the README overview row belong to gap-a11y-text-size and were not in this finding's recommendation.
- Per-message verification glyphs and Copy link need agent/IPC fields and a URI scheme (later phases); not part of this finding.

### `chat-ux-link-label-spoofing`

chat-ux-link-label-spoofing ([finding](findings/chat-ux.md#chat-ux-link-label-spoofing))

Follow-ups:

- A confirmation dialog shown by the desktop app itself (the Rust host) inside open_chat_link for every chat link. The recommendation offers it only 'if a host-enforced step is wanted', and the mockup marks it deferred. It would show a second prompt on every link click, including for mismatched links that already get the in-app dialog. It would only protect against a compromised app front end, which can already call other host commands. open_chat_link is also reused by sso.rs for sign-in URLs. It is compatible with Go FOKS v0.1.9 and could be added later if that protection is wanted.
- A host that appears only in the path of a label written without a scheme, such as `evil.example/@bank.example`, is not compared: without a scheme only dotted names before the first `/`, `\`, `?` or `#` are read as hosts, plus names after a `?@` or `#@` that ends that part (6b06773). The reviewer rated this low. The destination host is still shown after the link.
- The mockup's 'Posted by <sender> in #<channel> at <time>' line in the dialog. MessageText does not receive sender or channel information, and the finding's recommendation does not ask for it.

### `chat-ux-local-mute`

chat-ux-local-mute ([finding](findings/chat-ux.md#chat-ux-local-mute))

Follow-ups:

- Syncing mutes, team defaults and channel alert modes across devices as an encrypted record in the user's own KV store (agent/IPC step, plus the chat_channel_preferences client table that delta application would not overwrite).
- Protocol mentions with resolved identities (chat-backend-notification-prefs-mentions-names); Mentions only is a local text match only.
- The 'Channel alerts' list with Reset in Settings from mockup E. The app-icon unread badge and the Linux notification backend were added in e6cb693 (desktop-native-linux-notifications-and-badges).
- A bell-off glyph on muted channel rows (part of chat-ux-channel-list); muted rows keep today's 'Muted' caption and dimmed styling.

### `chat-ux-message-actions`

chat-ux-message-actions ([finding](findings/chat-ux.md#chat-ux-message-actions))

Follow-ups:

- Copy link to a message: needs a portable identifier (team id hex + channel id + sequence) resolved against the viewer's own stores (store refs are local JSON, src-tauri/src/commands/store_key.rs), MessageText recognising and routing it in-app instead of safeChatLink/openChatLink, an anchored window load with row highlight, and optionally an OS-registered URI scheme.
- Per-message verification result in Details (e.g. 'Follows the previous message'): needs the agent/IPC to return a per-row predecessor check; today only the page-level missing_predecessors flag exists.
- Edit, Delete, Reaction and structured Reply actions: the server accepts only RtMessageType::Basic sends (foks-server-db realtime/messages.rs) and the client renders other kinds as Unsupported; protocol/server and client-library work, out of scope.
- The '?' shortcut sheet belongs to chat-ux-keyboard-a11y and is still open. The roving tabindex across message rows (Up/Down/Home/End, Enter to open actions, Esc to composer) and the arrival announcer were added in 259c537.
- Hover/focus tooltip component for action buttons (the mockup's data-tip tooltips); the toolbar uses native title tooltips like the rest of the app.

### `chat-ux-message-rendering`

chat-ux-message-rendering ([finding](findings/chat-ux.md#chat-ux-message-rendering))

Follow-ups:

- Protocol-level mentions and mention counts, deferred in ISSUES.md; the @-highlight here is text styling of Basic messages only. The 'Mentions only' alert mode was added in ec5e075 (chat-ux-local-mute) as a match on message text on the device.
- Highlighting @names of departed members or senders missing from the roster needs per-sender usernames from the agent (chat-ux-sender-identity, Phase 2 IPC field).
- Hover tooltips on links and actions from the mockup; the toolbar uses native title tooltips. Merging consecutive '> ' lines into one blockquote and the per-message action bar were added in 77f5c6b (chat-ux-message-actions).

### `chat-ux-mock-fidelity`

chat-ux-mock-fidelity ([finding](findings/chat-ux.md#chat-ux-mock-fidelity))

Follow-ups:

- chat.render.test.tsx still rewrites mock replies inline (missing_predecessors, uncertain operations). Moving those tests onto the named scenarios would be a broader test refactor, so it is left out; the seeds and IDs are exported for that.
- Two review points left as is, since both match agent behaviour. (1) In the failures scenario the uncertain operation can never finish, because the mock's reconcile finds no message; that persistent unconfirmed state is the one the scenario is meant to show. (2) In the integrity scenario, #deploys reports before='3', so loading older messages returns an empty page; the decoder requires before to equal the lowest sequence above 1, so the agent would answer the same way.
- The scenarios cover send-message operations only. There is no seeded create-channel operation and no unverifiable or oversized content.

### `chat-ux-open-at-first-unread`

chat-ux-open-at-first-unread ([finding](findings/chat-ux.md#chat-ux-open-at-first-unread))

Follow-ups:

- No Go-compatibility-related deferral was needed; everything uses existing IPC data.
- Not built: the 'N of M new read' header meter and the 'Marked #channel as read' toast from the mockup. The Alt+arrow channel moves and the arrival live-region announcer from chat-ux-keyboard-a11y were added in 259c537.
- Possible agent-side follow-up: the agent (crates/foks-agent/src/chat.rs) narrows a range by keeping its end. Narrowing forward range reads toward their start instead would remove the client-side fill reads, which cost extra round trips only when a range hits the page limits.
- Known limitations, not fixed: when fill reads are merged, a missing-predecessor warning on either page applies to all rows of the merged page, which over-warns but never under-warns. While a window is detached, rows it already holds are not re-read until it reaches the head; the same is true of attached windows today.

### `chat-ux-thread-decomposition`

chat-ux-thread-decomposition ([finding](findings/chat-ux.md#chat-ux-thread-decomposition))

Follow-ups:

- Moving useChatViewport and useChatReadIntent into MessageList, as the recommendation suggests: their results also feed the alerts and the composer outside the list, so they stay in the ChatThread controller.
- An outgoing row's clock (local createdAt) can still differ from the history row that replaces it (server insert_time) when a message is queued or the clocks are skewed. Fixing that would need the server insert time before history arrives, which the current IPC data does not provide.

### `client-libs-hard-state-migration-ladder`

client-libs-hard-state-migration-ladder ([finding](findings/client-libs.md#client-libs-hard-state-migration-ladder))

Follow-ups:

- Replace the separate 38 and 39/40 branches with an ordered MIGRATIONS ladder in one IMMEDIATE transaction. The admission_floor rename and boot_id add, shared helpers in lib.rs since 045a254, would then each be one ladder step instead of being called from both merkle_checkpoint::migrate and the 39/40 branch of initialize_or_verify. Triage judged this high risk.
- On the normal open path, compare the migrated sqlite_schema with an in-memory INITIAL database before setting user_version. Today that check runs only in inspect_existing, and in the new tests.
- Check in SQL dumps captured from tagged builds (hard-v38.sql, hard-v39.sql). Not possible from this repository: history before 8cb0d1a is squashed, so the historical DDL is reconstructed by reverse DDL, as the server's compatibility tests do.
- Add a comment stating which predecessor versions the client keeps under the pre-v1 policy, and when old steps may be dropped. That is a policy decision for the maintainers.

### `client-libs-secret-record-redaction`

client-libs-secret-record-redaction ([finding](findings/client-libs.md#client-libs-secret-record-redaction))

Follow-ups:

- The other `Zeroizing::new(serde_json::to_vec(..))` sites are outside foks-client-app and were left unchanged: crates/foks-agent-proto/src/frame.rs:144 and three in crates/foks-agent/src/chat.rs. They would need encode_private_json moved to a shared crate.
- StoredYubiAccount and PendingYubiAccount in yubi.rs still use plain arrays with manual Drop impls (subkey_seed, puk_seed, self_token, management keys). They do not derive Debug and were not named in the finding; they could move to SecretArray in a follow-up.
- Moving a SecretArray still copies its inline bytes without zeroizing the old location. This matches SecretSeed and the previous arrays; boxing the storage would avoid it.

### `client-libs-soft-stage-reclaim`

client-libs-soft-stage-reclaim ([finding](findings/client-libs.md#client-libs-soft-stage-reclaim))

Follow-ups:

- Age-aware reclamation through a created_at column on kv_large_files (additive pattern like kv_large_file_measurements) was left out, as the triage note requires, to avoid any soft-schema change.
- The two-profile entry with_checked_sessions / with_checked_sessions_policy runs neither Merkle compaction nor stage reclamation. This is unchanged and outside the triage scope, which named only the two single-profile exclusive entry points.
- The research snapshot book/docs/research/research-client.md still cites old soft.rs line numbers for reclamation. It is a dated research note and was not updated.

### `desktop-native-app-acl-for-commands`

desktop-native-app-acl-for-commands ([finding](findings/desktop-native.md#desktop-native-app-acl-for-commands))

### `desktop-native-command-contract-check`

desktop-native-command-contract-check ([finding](findings/desktop-native.md#desktop-native-command-contract-check))

Follow-ups:

- The app ACL manifest named in the finding as an optional source for the command list was added in 4d6ecf9 (desktop-native-app-acl-for-commands): build.rs COMMANDS, checked against generate_handler! and the capability. The contract check (command_signatures.rs) still derives its command list from the generate_handler! source rather than from that manifest.
- Argument value types (DTO shapes) are not checked, only command names and keys and whether each key is required or optional. That matches the finding's scope; wire-contract/inventory.json still records DTO generation as deferred.

### `desktop-native-context-split`

desktop-native-context-split ([finding](findings/desktop-native.md#desktop-native-context-split))

Follow-ups:

- The HashMap<StoreKey, usize> index from recommendation (1) is not implemented. The catalog snapshot is changed in place in several context methods, and tests and a chat_local test write state.catalog directly, so a separate index could go stale; the catalog would need wrapping first. Comparing fields directly already removes the per-lookup serialization the finding objects to.
- Tauri commands still receive store ids as String and the lookups parse them. Taking StoreKey as a command argument would change every command signature and the generated command_signatures contract, which is out of scope here.
- The 65 PoisonError::into_inner sites outside the context module (agent/*, clipboard, close_guard, diagnostics, a few command modules) still use the old form. The finding marks these as optional, and leaving them avoids touching unrelated files that another lane may be editing.
- The fact caches (accounts, devices, rosters, federations, local_accounts) are still keyed by String ids. Since only the exact form is accepted, a parsed key and the string identify the same store.

### `desktop-native-linux-notifications-and-badges`

desktop-native-linux-notifications-and-badges ([finding](findings/desktop-native.md#desktop-native-linux-notifications-and-badges))

Follow-ups:

- A 'Group alerts by channel' toggle: grouping is always on, as the finding recommends. The mockup's toggle was not built; it needs no later-phase data.
- The live alert preview card from mockup state E: a UI-only addition, left out to keep the change scoped.
- macOS authorization status in the session, for the mockup's 'Allowed' status row. This is a native-shell addition that needs `getNotificationSettings` on each session; it was not built, and could not be compiled or tested on this Linux machine.
- Sender labels for people who have left the team come from the agent (agent/IPC phase); for now such a sender is shown by a short ID.
- Syncing alert preferences, including the badge setting, across devices: later phase (agent/IPC, client library).

### `desktop-native-linux-parity`

desktop-native-linux-parity ([finding](findings/desktop-native.md#desktop-native-linux-parity))

Follow-ups:

- No automated test for the Linux clipboard hint: it needs a live X11 or Wayland display, which the container does not have.
- The macOS branches (opener /usr/bin/open, open_settings via the opener, and the bounded_name refactor of the computer and user name lookups) were only written for macOS, not compiled: this Linux host has no macOS target.

### `desktop-native-quit-confirmation`

desktop-native-quit-confirmation ([finding](findings/desktop-native.md#desktop-native-quit-confirmation))

Follow-ups:

- Counting connected agent clients (the finding notes AgentStatus has no client count). This would need a new foks-agent-proto field, which is not allowed. For now an agent this app did not launch or adopt is treated as possibly shared, and the app always asks before stopping it.

### `fe-arch-code-splitting`

fe-arch-code-splitting ([finding](findings/fe-arch.md#fe-arch-code-splitting))

Follow-ups:

- Measure cold start in the packaged app with npm run test:foks-desktop:packaged on macOS (WKWebView) and Linux. It needs a cargo release build, which this lane must not run. The numbers in the commits come from WebKitGTK 2.52.6 under Xvfb, without a Tauri host.
- Vite's own warning about chunks over 500 kB still prints, because the entry chunk alone is 530.66 kB. build.chunkSizeWarningLimit is unchanged; the startup budget (STARTUP_BUDGET_BYTES, 900,000 bytes since 36df7a1, against 879,978 bytes of startup JavaScript at HEAD) is the enforced limit.
- Further startup reductions are not done: the Files screen, the write workflows and the details panel are still eager (static imports in src/app/screen-router.tsx, src/app/shell-overlays.tsx and src/app/vault-shell.tsx). Since 6392223 they live in src/features/files, which accounts for about 120 kB of the 880 kB startup JavaScript.

### `fe-arch-details-panel-plaintext-reducer`

fe-arch-details-panel-plaintext-reducer ([finding](findings/fe-arch.md#fe-arch-details-panel-plaintext-reducer))

Follow-ups:

- The finding's optional follow-up of splitting the DetailsPanel view into DetailsHeader, ValueView and EditorView components was not done. It is a separate structural refactor that does not affect the plaintext lifecycle, and leaving it out keeps this commit to the state change.

### `fe-arch-device-storage`

fe-arch-device-storage ([finding](findings/fe-arch.md#fe-arch-device-storage))

Follow-ups:

- Recommendation's generic usePref hook (useSyncExternalStore plus the cross-window 'storage' event) was not added. Modules keep their own hooks and storage-event handling; the shared piece added is storedRecord/subscribeStorage, which purgeProfiles notifies.
- Moving keys under 'foks.ui.*' with a one-time legacy-key migration was not done. Key names are unchanged, to avoid a migration that fixes nothing and to keep theme-init.js (which runs before any module loads) simple.
- foks.recoveryPhraseTests is registered as device-scoped and is not purged. It is keyed by backup-key entity id (it holds times only), and nothing in the webview maps an entity id back to a local profile without the snapshot.

### `fe-arch-files-list-a11y`

fe-arch-files-list-a11y ([finding](findings/fe-arch.md#fe-arch-files-list-a11y))

Follow-ups:

- Adding eslint-plugin-jsx-a11y to apps/desktop was not done. It needs a new npm devDependency and a package-lock.json change.
- In the card view, the per-card Copy, Reveal and Download buttons are no longer tab stops, and Left and Right move between cards. Keyboard users reach those actions through the card's row menu (Shift+F10 or the ContextMenu key), which already lists all of them. There is no separate in-card focus mode such as an Enter or F2 key to step into a card's buttons.
- Shift+Arrow range extension and type-ahead in the list and the tree were not added; they were outside the recommendation.

### `fe-arch-pane-resize-unify`

fe-arch-pane-resize-unify ([finding](findings/fe-arch.md#fe-arch-pane-resize-unify))

Follow-ups:

- The finding says to put the hook in src/ui/, which does not exist yet. The hook is in src/components/ (the folder fe-arch plans to rename to ui/) and moves with that rename.

### `fe-product-attention-centre`

fe-product-attention-centre ([finding](findings/fe-product.md#fe-product-attention-centre))

Follow-ups:

- 'Make rail badges open the relevant filtered view' is not done. No filtered views exist for these badges, so the rail still opens each tab as before; the new list rows link to each team's Requests tab and each account's Devices page.
- Pending invitation and approval operations (the invitation part of a team's Unfinished activity band) and expired-lease notices in Files are not in the attention list; each invitation journal would add an agent read per team. Interrupted membership changes are listed since 976b590, and expired check-ins are covered by the per-server 'Check-in expired' item.
- A 'CLI/MCP connected' checklist step was dropped, as the verifier noted: no non-secret 'clients seen' fact exists, and adding one would need a new agent IPC field (forbidden surface).
- Attention items are not individually dismissible; each one clears when its underlying state resolves.

### `fe-product-exposure-review`

fe-product-exposure-review ([finding](findings/fe-product.md#fe-product-exposure-review))

Follow-ups:

- 'Edit & rotate' opens the item in Files (selected, with the details panel open), not straight into the editor. The editor with the password generator is one click (Edit) away. Opening it directly would need an edit request threaded through vault-shell and the large DetailsPanel, which returns early when no item is loaded. It is not a forbidden surface; it was left out to keep this change contained.
- An entry is not followed to its item's new dirent id when the item is moved or renamed in this app; it reads 'Not listed' and can still be marked. recent-favorites.ts has a noteMoved/followMoves mechanism that could be reused for this.

### `fe-product-import-export`

fe-product-import-export ([finding](findings/fe-product.md#fe-product-import-export))

Follow-ups:

- Bitwarden JSON import (the recommendation's optional extra) is not implemented; only CSV is read.
- Optional Rust-only `foks kv export/import` CLI subcommands are excluded: the user's constraints forbid new CLI commands.
- An encrypted item-archive format as the default export does not exist yet; the export is plain-text CSV with a typed confirmation and a warning, as the recommendation allows until such a format exists.

### `fe-product-keyboard-a11y`

fe-product-keyboard-a11y ([finding](findings/fe-product.md#fe-product-keyboard-a11y))

Follow-ups:

- The macOS-only code in src-tauri/src/lib.rs (the Lock menu item and its handler) was not compiled: it is behind cfg(target_os = "macos") and this Linux machine has no macOS target or SDK. It follows the existing types and calls in that function, and lock_now itself is compiled and tested on Linux through lock_app.
- The finding suggests Space should reveal a row. That was not done: Space keeps its existing behaviour (show details), as the earlier grid commit chose. Reveal is still in the row's actions and menu.
- aria-keyshortcuts on list rows stays the literal 'Shift+F10' (ARIA syntax) instead of being derived from the registry.

### `fe-product-login-fields-copy`

fe-product-login-fields-copy ([finding](findings/fe-product.md#fe-product-login-fields-copy))

Follow-ups:

- Clear now in the copy toast (mockup) is not built; it belongs to gap-toast-timing-announcements, whose follow-ups record it. Open website (⌘O in the palette, 2d88939) and the clear time in the copy toast ('clears in 30 s', 97a7ab2) were built under other changes.

### `fe-product-organize-bulk`

fe-product-organize-bulk ([finding](findings/fe-product.md#fe-product-organize-bulk))

Follow-ups:

- Download of several selected files: download_file opens one save dialog per file, so saving many to one folder needs a native folder picker. Not built.
- An 'Undo moves' button in the progress sheet for a bulk move to a folder is not built; it is not in the finding's recommendation. Undo for several items moved to Trash at once was added in 3f26712 (the closing toast, or the progress sheet when some items stayed).
- Per-item 'Keep both' (move under a new name) for a name already taken in the destination, shown in the mockup. Such items are reported as refused.
- Dropping rows onto the Trash row of the tree, and dragging or multi-selecting folder rows. Only item rows can be chosen and dragged.
- Writing several entries in one kvPut, which the finding itself defers until a measured need appears.

### `fe-product-password-generator`

fe-product-password-generator ([finding](findings/fe-product.md#fe-product-password-generator))

Follow-ups:

- A real estimator for typed passwords (e.g. a lazily loaded zxcvbn-ts) is not included. Adding it means a new npm dependency and a package-lock change in the node_modules install the other lane shares, so typed passwords get common-weakness warnings only and are never given a strength level.
- Reuse detection is not included, as the finding itself says: it needs the agent to compare plaintexts across items, which would mean new agent IPC operations, and the scope forbids those.
- fe-product-exposure-review (ce14eeb) opens an entry's item in Files, where Edit opens the editor with PasswordInput and the generator; opening the editor directly from the review is recorded as a follow-up of that finding.

### `fe-product-search-quick-actions`

fe-product-search-quick-actions ([finding](findings/fe-product.md#fe-product-search-quick-actions))

Follow-ups:

- Clear Recent and Favorites on sign-out: the desktop renderer has no sign-out flow to hook into. Ids for removed accounts or items are dropped from display because they no longer resolve, but they stay in storage until they roll off the 10-item Recent list (Favorites are capped at 100).

### `fe-product-share-to-team`

fe-product-share-to-team ([finding](findings/fe-product.md#fe-product-share-to-team))

Follow-ups:

- Copying a folder recursively. The command refuses folders with an explanation; a recursive copy would need a plan of many creates and how to handle partial failure.
- Permanently removing the original with RemoveKv, as the finding proposed. Instead the original goes to Trash with the existing trash_item command, so it can be restored. In a team where Trash is not set up and this account is not an owner, the move option is disabled and only a copy is made.
- A separate 'Share to team…' entry. The one 'Copy to vault…' sheet covers it and opens on the first team this account can write to.
- The failure of agent::tests::transport::observed_cancellation_and_deadline_leave_the_next_call_healthy recorded during this work (ConnectionReset) was a bug in the test, fixed in 5c8c6a7; a future failure is a regression.

### `fe-product-trash-undo`

fe-product-trash-undo ([finding](findings/fe-product.md#fe-product-trash-undo))

Follow-ups:

- 'Pointing at the toast pauses it' (mockup, Undo state): the kit holds a hovered or focused toast (97a7ab2, gap-toast-timing-announcements), but the Undo toast (durationMs 8000) also passes expiresAt (82115d3), so it closes when ⌘Z stops working, even while held.
- Multi-select bulk Move to Trash (built in 6718c95, with Undo added in 3f26712) and dropping rows onto Trash (not built) belong to fe-product-organize-bulk, not this finding.

### `fe-product-unused-vault-ipc`

fe-product-unused-vault-ipc ([finding](findings/fe-product.md#fe-product-unused-vault-ipc))

Follow-ups:

- Three other bridge methods (and their registered host commands) have no caller outside the bridge, so the new test lists them as exceptions: listParties (list_parties), listFederation (list_federation) and verifyAccountPassphrase (verify_account_passphrase). The finding does not cover them, so they were not removed; they need their own decision (remove, or connect to a feature).

### `gap-mcp-adapter-clock-suspend`

gap-mcp-adapter-clock-suspend ([finding](findings/gap.md#gap-mcp-adapter-clock-suspend))

Follow-ups:

- macOS is untested here: the CLOCK_MONOTONIC_RAW reading and the `sysctl -n kern.bootsessionuuid` boot id only compile under cfg here. If the sysctl call fails, the clock still works but restarts use the 24 h rule. CLOCK_MONOTONIC_RAW is not frequency-corrected, so one agent process running for roughly 70-170 days could drift past the 300 s skew. That fails closed and the desktop or CLI repair fixes it.
- The new desktop command is not in the shared wire-contract fixture inventory (apps/desktop/src-tauri/wire-contract), because no test requires it. The command is covered by its native and renderer tests instead.
- The finding's evidence also cites Merkle roots and compatibility-artifact times as fresh host time; they were not used, per the verifier note.

### `gap-recovery-phrase-test`

gap-recovery-phrase-test ([finding](findings/gap.md#gap-recovery-phrase-test))

Follow-ups:

- Enrollment date in the test result (mockup state C): the agent operation's return shape (RecoveryPhraseCheck: account_alias, matched_backup_id_hex, matched_name, revoked, backup_alias, user_chain_sequence) does not carry a link time, so the result omits it.
- Last-tested times are stored in webview localStorage, keyed by backup key ID, which the mockup names as the alternative to a native-shell store. There is no native device-local store and no purge on device reset; the records hold times only.
- There is no timer to re-evaluate the 180-day overdue state or the 30-day snooze expiry while the app stays open. Both are recomputed on the next render of the shell or the Devices page.
- Deliberate deviation from the mockup note for state D: the 3-token recall at creation is not counted as the phrase's first test, so a new phrase shows 'Last tested: never' and lights the dot until a full test matches.
- The foks-desktop backend crate (crates/foks-desktop/src/main.rs CLI) has no mapping for VerifyRecoveryPhrase. The shipping Tauri command builds the operation directly.
- The failure of agent::tests::transport::observed_cancellation_and_deadline_leave_the_next_call_healthy recorded during this work (ConnectionReset) was a bug in the test, fixed in 5c8c6a7; a future failure is a regression.

### `gap-renderer-fault-capture`

gap-renderer-fault-capture ([finding](findings/gap.md#gap-renderer-fault-capture))

### `gap-toast-timing-announcements`

gap-toast-timing-announcements ([finding](findings/gap.md#gap-toast-timing-announcements))

Follow-ups:

- The mockup's 'Clear now' action and the 'Clipboard cleared' state that follows a host event are not built. They need a new host command around clipboard::clear_pending and a host-to-renderer event, which go beyond what this finding recommends.
- copy_text (copies of non-vault text such as the socket path, host ID and pairing phrase) also auto-clears, but it still returns a plain CommandAck, so those toasts do not state a clear time. Only the vault item copies (copy_item_field and copy_item_value) report it.

### `protocol-box-randomness-secret-api`

protocol-box-randomness-secret-api ([finding](findings/protocol.md#protocol-box-randomness-secret-api))

Follow-ups:

- Making the randomness struct fields private and adding fixture-only constructors behind a cross-crate `fixtures` cargo feature (deferred by triage; foks-server tests and foks-crypto tests build these structs from literal fixture values).
- The X25519 set secret is still passed by value to x25519_dalek::StaticSecret::from in seal_yubi_puk_boxes, which leaves one unzeroized stack copy; avoiding it needs a StaticSecret constructor that takes a reference.

### `protocol-fuzz-schema-and-verification-decoders`

protocol-fuzz-schema-and-verification-decoders ([finding](findings/protocol.md#protocol-fuzz-schema-and-verification-decoders))

Follow-ups:

- Step 5 of the recommendation (factor the foks-rpc response prelude into response_result_slot and keep the three public functions as thin wrappers) is skipped as the triage note directed. The four response decoders are fuzzed as they are, without the refactor.
- The verify corpus holds one seed per verification stage, ten in all (public host, user chain, non-self user chain, user chain increment, non-self user chain increment, team chain, team chain increment, Merkle advance root, Merkle advance history, signed Merkle advance; 4df1a10 added the three increment stages), each a fixture that the stage accepts, not all 137 fixtures behind each selector. Fixtures that a stage would reject early add little.
- cargo-deny for the fuzz graph (README command) was not run locally because cargo-deny was not installed; the foks-dependencies CI job runs it on fuzz/Cargo.toml (matrix entry since ef1a51e). The newly added crates all already appear in the root graph.

### `protocol-seed-chain-plaintext-nonzeroizing-decode`

protocol-seed-chain-plaintext-nonzeroizing-decode ([finding](findings/protocol.md#protocol-seed-chain-plaintext-nonzeroizing-decode))

Follow-ups:

- The finding also offered a scoped clippy disallowed_methods lint as an alternative. It was not added: there is no clippy.toml in the repo, and the include_str! source check (a pattern the repo already uses in foks-server and foks-client-db) gives the same guard without touching other crates.

### `protocol-typed-status-codes-and-detail`

protocol-typed-status-codes-and-detail ([finding](findings/protocol.md#protocol-typed-status-codes-and-detail))

Follow-ups:

- A generated `#[non_exhaustive] Status` enum with `Other(u64)` and `foks_rpc::Error::status()`, as the triage note says.
- Status-code literals in test code are unchanged (foks-server-testkit tests, foks-rpc `tests.rs` and `tests/realtime.rs`, the foks-client `lib.rs` test module, `foks-client/tests/authenticated_user.rs`). They still check wire values independently.
- Agent IPC error text (`foks-agent-proto` `bounded_field` and `foks-agent` `remote_status_response`) is length-bounded but still not filtered for control or bidirectional characters. Text that comes from a server status detail is now cleaned in `check_status`, but other agent error text is not.

### `server-config-surface`

server-config-surface ([finding](findings/server.md#server-config-surface))

Follow-ups:

- Recommendation 3 relationship checks outside the triage scope: descriptor budget vs RLIMIT_NOFILE (still a startup warning only), maximum_pending_writes vs maximum_in_flight_requests (no defined relationship to enforce), and storage reserve < maximum_database_bytes (no storage-reserve setting exists).
- The serve subcommand has no flags for the [storage] limits; it still uses the compiled foks_server_db::Config defaults (ServeArguments::database_limits is #[arg(skip)]; only serve-config applies [storage]). backup and restore read the limits from an optional --config server.toml since 2ad6c53.
- [storage] exposes only the capacity limits the finding names; the other 33 foks_server_db::Config fields (all but the five [storage] limits and the two [waitlist] fields) and the session limits maximum_frame_bytes, maximum_request_memory_bytes, maximum_requests and io_timeout keep their compiled defaults.

### `server-error-status-mapping`

server-error-status-mapping ([finding](findings/server.md#server-error-status-mapping))

Follow-ups:

- DatabaseCorrupt/NotADatabase still map to TransactionRetry rather than a fixed non-retryable status: the v0.1.9 status set (crates/foks-rpc generated codes) has no generic non-retryable server-fault status, and inventing one would change the wire. Documented in status.rs.
- No source check was added to tools/foks-server/check.sh against formatting crate::Error into RpcStatus payloads (recommendation 3); the fixed strings and contract tests cover current code.
- The end-to-end test covers signup, team creation and KV writes. Chat send, identity proof and team edit are covered only by the mapper contract tests: realtime and identity-extension setup in the testkit would make the test much larger.
- Split of Capacity into storage vs concurrency variants (recommendation 2) not done; per-family Capacity handling kept as the triage note specified.
- Writer-level AuthorizationChanged now reports the shared message 'credential authorization changed' on KV, federation and user routes instead of each family's own PermissionDenied text (same status code).

### `server-fatal-thread-exit`

server-fatal-thread-exit ([finding](findings/server.md#server-fatal-thread-exit))

Follow-ups:

- The final process::exit(1) of the real foks-server binary is exercised only on Linux: explicit_binary_exits_with_failure_after_losing_a_listening_socket in crates/foks-server-testkit/tests/binary_release.rs (809daf3) shuts down a pidfd_getfd copy of the authenticated listening socket and asserts exit status 1. It runs only with FOKS_SERVER_BIN set and skips when pidfd_getfd is unavailable. A writer panic cannot be triggered in the binary without a test-only fault hook, which was not added.
- Startup failures inside a monitored thread (for example a failure in TcpListener::from_std, or in building the administration or OIDC listener's runtime) are recorded as fatal exits on the FatalSignal. Nothing can observe this, because start_standalone and RunningServer::start return the error and drop the signal. Left unchanged.
- The descriptor-limit check warns but does not fail startup when the limit stays below the estimate. The recommendation allowed either; a hard failure could stop existing deployments that have low hard limits from starting.
- Two existing behaviours from the review are left unchanged: on a session-task panic, run_listener returns without aborting its acceptor task (the runtime shutdown that follows cancels it), and an admission try_send that fails because the session loop has closed is still counted as connection_rejected.
- On non-unix targets, accept() errors are classified by ErrorKind only, and connection-reset errors are treated as fatal there. The crate is otherwise built for Linux and macOS only.

### `server-metrics-scrape-cost`

server-metrics-scrape-cost ([finding](findings/server.md#server-metrics-scrape-cost))

Follow-ups:

- Pool warming (recommendation item 4) was not implemented. With pooled opens no longer scanning, pre-opening connections is unnecessary.
- Offline readers in installation.rs, sso/operator.rs, web_admin/operator.rs and bin/foks-server.rs keep the validating ReadDatabase::open, per the triage note. Since d7a6308 they pass the server.toml limits (InstallationConfig::database_config) instead of Config::default().

### `server-operator-capacity-console`

server-operator-capacity-console ([finding](findings/server.md#server-operator-capacity-console))

Follow-ups:

- Account suspension (foks-server account suspend|resume, suspended_users table, checks in authorize_principal and the writer's in-queue re-check): excluded by triage because it needs a schema change and a writer authorization hook.
- Web-admin /admin/capacity page: not in the triage subset. The report API (ReadSnapshot::capacity_report, foks_server::usage::report_json) is ready for it.
- Last backup and maintenance results in the usage report: these live only in the in-process ServerMetrics, so the offline CLI cannot read them. /metrics already exports them.
- Prometheus used-value gauges for row counts and KV/realtime totals: left out so /metrics does no table scans. Only the limits are exported; database headroom can be computed from foks_database_bytes.

### `server-request-failure-observability`

server-request-failure-observability ([finding](findings/server.md#server-request-failure-observability))

Follow-ups:

- Converting the remaining roughly 300 inline `.map_err(|_| RpcStatus::TransactionRetry)` closures (and similar `.ok_or(RpcStatus::TransactionRetry)` sites) to the site funnel. Triage judged this too broad; these paths are still not counted, as the README says.
- Histograms: replacing DurationMetric (count/sum/max) with BUCKET_MICROS histograms for request, handler, writer queue-wait and writer execution durations.
- Writer attribution: generalizing WriterHandle::call_observed / ReconcileSource into a WriterOp enum.
- A testkit case that actually exhausts the reader pool. ReadPool::lease, to which ServerData::read_database delegates, is covered by unit tests in read_pool.rs (1dd8d85), but no end-to-end request reaches an exhausted pool.

### `server-session-decomposition`

server-session-decomposition ([finding](findings/server.md#server-session-decomposition))

Follow-ups:

- RequestContext / AuthorizedWriter typing, so that mutation services need a writer authorized for the principal: per-request state is still a full ServerData clone with peer_ip and writer overwritten, and handlers/realtime.rs still builds its own authorized writer. The triage note deferred this because it touches write authorization.
- Splitting serve() into read_frame / admit / dispatch / write_response, to get net/session.rs under about 600 lines of transport (it is now 1,084 lines including tests).
- Splitting services/team_admin.rs (about 1,600 lines) into create, edit and bearer-token helpers.
- Merging the duplicate require_cited_root helpers in services/generic.rs and services/team_admin.rs with the one now in services/user_mutation.rs (left as is to keep this change a pure move).

### `server-source-admission-fairness`

server-source-admission-fairness ([finding](findings/server.md#server-source-admission-fairness))

Follow-ups:

- Per-connection lifetime bound (recommendation 2)
- Per-source cap on concurrent long polls (KexReceive, SSO poll, RtPollInbox) (recommendation 2). KexReceive now has one: a source holds at most a quarter of the KEX receive budget (c3-session-kex-receive-budget-per-source). RtPollInbox is capped per account, not per source; the SSO poll has no per-source cap.
- End-to-end testkit/listener_boundaries.rs cases for one IP holding more than the cap and for IPv6 rotation: the testkit connects only from loopback, which is exempt, so these are covered by unit tests in net/listener.rs and rate_limit.rs against scripted non-loopback peers
- book/docs/research/research-server.md still describes per-IP buckets with old line references; it is a dated research note, and only its KEX relay paragraph was updated (4856674)

### `server-waitlist-unbounded`

server-waitlist-unbounded ([finding](findings/server.md#server-waitlist-unbounded))

Follow-ups:

- Deliberate deviation: the recommendation puts list/export under DatabaseWriterGuard, but they read a ReadDatabase snapshot, as `invite list` and `admin list` do, so the server can keep running. Only purge takes the guard.
- Not addressed: the cap is global, so one unauthenticated client can fill it and every join is refused until entries expire or the operator purges them. This is documented in the README. A per-source budget was not added because it would require storing client IPs, which is more personal data.
- Design decision: a repeated join does not refresh created_at, so retention counts from the first join. This is documented.
- Not done: the Go client was not tested against a disabled host (Unsupported). The disabled path is covered only by the Rust conformance test.

### `server-writer-deadlines`

server-writer-deadlines ([finding](findings/server.md#server-writer-deadlines))

Follow-ups:

- Control and bulk priority lanes with a starvation guard (finding recommendation 2), excluded by triage because it reorders the core write path.
- Per-UID pending writer job cap in for_authenticated_request (recommendation 3), excluded by triage.
- Cost-weighted rate-limit tokens for writer-bound routes via policy-v1.toml (recommendation 4), excluded by triage.
- Testing that a revocation overtakes 60 queued KV chunks (part of recommendation 5) depends on the lanes and is not included.
- The verifier noted that an abandoned blocking handler keeps its execution-semaphore permit until it finishes. With this change, a handler waiting on a queued write is answered once the writer reaches its job, rather than after the job runs. A handler blocked on an already-running write still holds its permit.
