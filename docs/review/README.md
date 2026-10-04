# foks-rs review, October 2026

A review of the whole repository: the chat product and stack, the desktop frontend and native shell, the agent, CLI and MCP server, the client libraries, the server, the protocol and crypto crates, and build, CI and documentation. It ends in feature refinements, missing features, and code-quality and maintainability work, sequenced into a roadmap. Interactive HTML mockups cover the UI-facing refinements.

## Method

Eleven area reviewers read the code and reported findings with file and line evidence. Then a completeness critic looked for areas and concerns they had missed. Separate verifiers checked every finding against the code, and corrected or refuted it. 165 findings survived (20 high, 108 medium, 37 low priority). 2 were refuted (listed at the end). Most surviving findings carry verifier corrections to evidence, priority or recommendation; the per-area files include each verifier note. Line numbers refer to commit `d3ee213`.

## Summary

The review covered twelve areas and verified about 160 findings. The core engineering is sound: Snowpack and RPC decoding, the hybrid crypto, sigchain and Merkle verification, journaled mutations and STRICT SQLite schemas, all pinned to Go v0.1.9 fixtures. The risk sits in four places: product UX, interface contracts (agent IPC, Tauri commands, error classification), secret hygiene at the edges, and server limits that build up over the host's lifetime.

**Chat is integrity-first but MVP for everyday use.** Opening a channel scrolls to the bottom and marks everything read about 300 ms later. The channel list reorders whenever you read a channel. There is one message action, reachable only by right-click. A busy channel hits a hard 1,000-row cap and errors. List rows show no previews, although the inbox already carries them. Most refinements need no protocol change: about two thirds of the chat-UX findings can ship in desktop-ui on today's IPC data (history cursors, read_through, previews, read and write roles, page verification). A few need small agent or IPC fields: sender usernames, a per-message predecessor flag, a rejection reason, and drafts that survive a restart. Edits, reactions, real replies, delete, typing, read receipts and channel management need negotiated Rust-only extensions. Two things block all of them: the fixed-shape capability response, and the fact that one malformed or non-Basic message stops both reads and sends in a channel on FOKS-RS clients.

**Outside chat, several correctness and security gaps rank above polish:**
- In production, Copy puts the whole `user:/password:/url:` text on the clipboard.
- The app never locks on inactivity, although the book says it does.
- Delete is permanent and has no undo.
- Recovery phrases are never checked after they are written down.
- MCP writes stop after any laptop sleep longer than five minutes.
- After an upgrade, an IPC version mismatch is reported as "agent unavailable", and a mutation that never ran is reported as outcome unknown.
- On the server, one account can use up the host's 65,535 lifetime Merkle epochs.
- The unauthenticated waitlist grows without bound.
- A transient accept() error leaves the process alive with no service while /healthz still returns ok.

**Sequencing.** The roadmap has six phases.
- **Phase 0:** a PR-required CI gate, dependency auditing, the root causes of the flaky tests, and the silent server failures.
- **Phase 1:** chat UI on current data, starting with the structural chat refactors every refinement touches: thread decomposition, an explicit outgoing state machine and realistic mock scenarios.
- **Phase 2:** vault and security UX on existing agent operations.
- **Phase 3:** agent/IPC contract hardening, plus the agent additions for chat.
- **Phase 4:** client, protocol and server robustness. The server work can run in parallel from Phase 1 onward.
- **Phase 5:** negotiated chat extensions and distribution.

Twelve interactive mockups are included: eight for chat and four for the most valuable non-chat flows. Each is annotated with the agent, IPC, protocol or server work the real feature needs.

## Top recommendations

1. **Open channels at the first unread message, mark read by what was actually seen, and count arrivals while scrolled up** ([`chat-ux-open-at-first-unread`](findings/chat-ux.md#chat-ux-open-at-first-unread); high priority, effort M, desktop-ui). This is the most visible chat defect. Opening a channel marks the newest message read about 300 ms later whatever the user actually saw, and with more than 50 unread the first unread row is never loaded. The fix is renderer-only: the agent already accepts any `before` cursor and the server read pointer is monotonic. It also adds the anchored/detached history window that search results, quote-reply jumps and future permalinks will reuse.
2. **One malformed message from any channel writer stops reading and sending in that channel** ([`chat-backend-single-message-poisons-channel`](findings/chat-backend.md#chat-backend-single-message-poisons-channel); high priority, effort M, client-lib, agent, protocol). Any channel writer, or a nonconforming client posting through a Go host, can stop every FOKS-RS user from reading and sending in a channel with one undecryptable or non-Basic row. There is no way to delete that row, and some valid v0.1.9 shapes abort the whole team inbox sync. Per-message verdicts are client-local with no wire change. This must land before any new message kind (Pegged replies, attachments) can roll out without making channels unwritable for older FOKS-RS clients.
3. **Channel list: stable ordering, previews and times, draft and unread emphasis, persisted folds, honest context menu** ([`chat-ux-channel-list`](findings/chat-ux.md#chat-ux-channel-list); high priority, effort M, desktop-ui, docs). The list reorders under the cursor because the server bumps inbox_version when you read a channel. Rows show no preview or time, although the inbox carries previews and previewLine exists. The menu shows permanently disabled Edit/Delete, and the README documents a column that does not exist. All of this is desktop-ui on current data and is the first thing every chat user sees.
4. **Make the outgoing-message lifecycle an explicit state machine instead of `phase` plus seven flags** ([`chat-code-outgoing-state-machine`](findings/chat-code.md#chat-code-outgoing-state-machine); high priority, effort M, desktop-ui). There is a confirmed stale-status race: a detached status reply can overwrite the phase of a Retry that started later. Every planned delivery and composer refinement (one row layout, Up-to-restore, rejection copy, a header chip for unsent messages) edits this lifecycle. Making it an explicit transition() with attempt tokens first keeps those UI changes from adding more implicit flag combinations.
5. **Markdown link labels can disguise the destination; show the host and confirm mismatches** ([`chat-ux-link-label-spoofing`](findings/chat-ux.md#chat-ux-link-label-spoofing); medium priority, effort S, desktop-ui, desktop-native). Small and security-relevant. Any member, or a compromised member device, can post `[https://bank.example](https://evil.example)` and it opens with no hint of the real destination, in an app whose purpose is protecting secrets. Ship it in the same message-text.tsx change as chat-ux-message-rendering (bare-URL autolinking, paragraphs, code styling), so links are both safer and more useful in one pass.
6. **Production login classification and Copy act on the whole "user:/password:/url:" text** ([`fe-product-login-fields-copy`](findings/fe-product.md#fe-product-login-fields-copy); high priority, effort M, client-lib, agent, desktop-native, desktop-ui). In production, Password classification depends on an Item.value that only the mock supplies. A login outside /logins shows as a Document, and Copy puts the username, password and URL lines on the clipboard while the toast says 'Password copied'. The mock hides both problems from tests. The host-side copy_item_field command can ship first; the catalog shape bit follows in Phase 3.
7. **App lock is manual-only; add a native inactivity, sleep and screen-lock trigger** ([`desktop-native-auto-lock`](findings/desktop-native.md#desktop-native-auto-lock); high priority, effort M, desktop-native, desktop-ui, docs). The book, the glossary and docs/INTRO.html all say the vault locks after inactivity, but locking is manual-only, so an unattended machine stays unlocked indefinitely. The policy belongs in the native host, which the renderer cannot override. Correct the book now if the feature cannot ship in the same release.
8. **One authenticated account can permanently exhaust the host's 65,535 Merkle epochs (and the 4,096-team cap)** ([`server-merkle-epoch-budget`](findings/server.md#server-merkle-epoch-budget); high priority, effort M, server, docs). One device can loop generic-link posts or team edits until the host's lifetime epoch ceiling is reached. After that, signups, revocations and team edits fail for every user, and no metric shows the remaining headroom. The epoch gauge is S effort. Per-UID budgets and a reserve for revocations are server-local and Go-compatible.
9. **A transient accept() error or a writer panic leaves a live process with no service, and /healthz still returns ok** ([`server-fatal-thread-exit`](findings/server.md#server-fatal-thread-exit); high priority, effort M, server, tooling, docs). A transient EMFILE in accept() or a panic on the writer thread leaves a process with no service and /healthz still returning 200. systemd's Restart=on-failure never fires, and the default file-descriptor budget is close to the 1,024 soft limit. A fatal-exit channel plus accept retry turns a silent outage into an automatic restart.
10. **Unauthenticated joinWaitList inserts unbounded, permanent, unreadable PII rows** ([`server-waitlist-unbounded`](findings/server.md#server-waitlist-unbounded); high priority, effort S, server, protocol, cli, docs). Reg.joinWaitList is unauthenticated, inserts permanent rows of personal data with no cap, deduplication, reader or expiry, and can fill the single database ceiling that stops all writes. An off-by-default config flag, UNIQUE(email), a row cap and expiry are S effort and keep the Go client behaviour unchanged.
11. **MCP write admission treats every laptop sleep over five minutes as an untrusted clock, and a day off needs a CLI-only repair** ([`gap-mcp-adapter-clock-suspend`](findings/gap.md#gap-mcp-adapter-clock-suspend); high priority, effort M, client-lib, agent, mcp, desktop-ui, cli, docs). Instant does not advance during sleep, so every laptop sleep longer than five minutes blocks all MCP writes until the agent restarts, and a weekend off requires a repair that only the CLI can run. The fix reuses the suspend-inclusive clock the server already has (web_admin/clock.rs). It also makes the MCP chat tools and assistant-access work planned for Phase 3 viable.
12. **Recovery phrases are never checked after they are written down; add a local 'Test recovery phrase' and a recall step** ([`gap-recovery-phrase-test`](findings/gap.md#gap-recovery-phrase-test); high priority, effort M, agent, desktop-ui, desktop-native, cli). HESP has no checksum, so a typo in the written phrase is discovered only after every device is lost, at which point the account cannot be recovered. A local check (derive the key ID and compare it with the verified chain) needs no network or wire change, and BackupEnrollmentSummary already returns backup_id_hex.
13. **Permanent delete with no undo, and no folder delete; add a role-aware Trash built on MoveKv** ([`fe-product-trash-undo`](findings/fe-product.md#fe-product-trash-undo); high priority, effort M, client-lib, agent, desktop-native, desktop-ui, docs). Permanent delete with no undo is the most likely way to lose data in a secrets vault, and folders cannot be deleted at all. A role-scoped Trash built on the existing MoveKv and MkdirKv needs no protocol change. Moving a folder into Trash also removes the reason recursive delete is unsafe while other devices are writing to it.
14. **Replace exact-equality IPC versioning with a frozen hello/capability handshake** ([`agent-ipc-version-handshake`](findings/agent.md#agent-ipc-version-handshake); high priority, effort M, protocol, agent, client-lib, cli, mcp, desktop-native, docs). After an upgrade leaves an old agent running, the CLI reports 'initialize and unlock', spawns a sibling agent that cannot get the lock, and MCP reports 'agent unavailable'. A mutation the old agent rejected without running it surfaces as outcome unknown. A frozen hello also gives the capability channel that the chat subscription stream and the other Phase 3 agent additions need.
15. **No dependency advisory, license or source auditing for 749 crates and 280 npm packages** ([`build-deps-supply-chain-audit`](findings/build.md#build-deps-supply-chain-audit); high priority, effort M, ci, tooling). The project ships signed, notarized binaries that handle keys, from 749 crates and 280 npm packages, with no advisory, license or source check and no Dependabot. It is low effort. Land it in Phase 0 together with build-ci-pr-required-gate, so the refactors in the later phases are checked on every PR and not only after a push.

## Roadmap

### Phase 0: Safety net and silent-failure fixes

**Goal.** Make every later change gated on PRs, remove the root causes of the flaky tests, and fix failures that currently happen silently (server outage while /healthz is ok, unbounded growth, epoch exhaustion, MCP writes blocked after sleep, plaintext stored in soft state, wrong security claims in the docs).

Order:
1. Add ci.yml with a `changes` job and a single `ci-ok` required check. It runs fmt, clippy, tsc, eslint and prettier --check, and runs generate-protocol.sh --check --offline plus the Go guards on PRs. Split desktop validation so TS-only PRs stay on Linux, and add caching, concurrency groups and timeouts.
2. deny.toml, npm audit and dependabot.yml. Pin actions by SHA and set persist-credentials: false. Move the local digest-0.10 pins into workspace dependencies with rationale comments; defer the full digest-0.11 migration.
3. A LockGuard type that calls unlock before close, replacing fs2 with std File::lock. This removes --test-threads=1 for foks-desktop-app. Resolve foks-agent for tests from cargo JSON and add a protocol-version check.
4. Server-local fixes, with no wire change: fatal-exit channel and accept retry, LimitNOFILE; waitlist off by default with a cap, dedupe and expiry; epoch gauges, then per-UID budgets and a revocation reserve; a held snapshot for online backup.
5. Client libraries: fix the merkle_gc saga predicate (`state!=3` currently disables Merkle root compaction permanently); switch account and team sync to metadata mode with an in-place scrub of stored plaintext; suspend-inclusive adapter clock (steps 1-2; the repair operation moves to Phase 3).
6. Small desktop fixes: first-run copy rejections, ResetMacSheet partial-progress reporting, the inbox sibling dirty flag, the starter card's nonexistent `foks kv export` text (from fe-product-import-export), and the missing AppStream description that will fail the first Linux release tag (from desktop-native-release-matrix step 0).
7. Docs: root SECURITY.md; GUIDE.html and INTRO.html crypto corrections; the book's inactivity-lock claim (until desktop-native-auto-lock ships); the src-tauri README table; a link checker.

<details><summary>Findings in this phase</summary>

- [`build-ci-pr-required-gate`](findings/build.md#build-ci-pr-required-gate): No single PR-triggered required check; clippy, fmt and protocol-metadata guards for 23 crates run only on push
- [`build-ci-desktop-cost`](findings/build.md#build-ci-desktop-cost): Desktop PR validation rebuilds the full workspace on macOS in release mode, uncached, untimed and uncancelled
- [`build-frontend-format-lint-gate`](findings/build.md#build-frontend-format-lint-gate): Prettier is never checked in CI, and ESLint and tsc cover only apps/desktop
- [`build-deps-supply-chain-audit`](findings/build.md#build-deps-supply-chain-audit): No dependency advisory, license or source auditing for 749 crates and 280 npm packages
- [`protocol-crypto-dependency-hygiene`](findings/protocol.md#protocol-crypto-dependency-hygiene): Collapse duplicate crypto crate versions and add RustSec advisory checks to CI
- [`build-actions-hardening`](findings/build.md#build-actions-hardening): Inconsistent action pinning, persisted checkout credentials in secret-bearing jobs, no workflow linting
- [`build-test-flock-release-race`](findings/build.md#build-test-flock-release-race): Root cause of the first-run receipt-lock flake: flock locks outlive drop while sibling tests spawn processes
- [`build-test-agent-binary-resolution`](findings/build.md#build-test-agent-binary-resolution): CLI integration tests resolve foks-agent by sibling path, which nothing keeps current; six copies of the agent harness
- [`build-contributor-security-onboarding`](findings/build.md#build-contributor-security-onboarding): No SECURITY.md, CONTRIBUTING.md or prerequisite check; onboarding knowledge lives in an agent-oriented AGENTS.md
- [`build-docs-guide-crypto-misstatements`](findings/build.md#build-docs-guide-crypto-misstatements): docs/GUIDE.html and docs/INTRO.html still misdescribe the KDF, the hybrid combiner and recovery-kit stretching
- [`build-docs-link-and-drift-check`](findings/build.md#build-docs-link-and-drift-check): Broken design-doc references and stale READMEs; no automated link or path check
- [`desktop-native-doc-drift`](findings/desktop-native.md#desktop-native-doc-drift): Correct specific stale statements in the desktop's security and architecture docs
- [`server-fatal-thread-exit`](findings/server.md#server-fatal-thread-exit): A transient accept() error or a writer panic leaves a live process with no service, and /healthz still returns ok
- [`server-waitlist-unbounded`](findings/server.md#server-waitlist-unbounded): Unauthenticated joinWaitList inserts unbounded, permanent, unreadable PII rows
- [`server-merkle-epoch-budget`](findings/server.md#server-merkle-epoch-budget): One authenticated account can permanently exhaust the host's 65,535 Merkle epochs (and the 4,096-team cap)
- [`server-backup-snapshot-restart`](findings/server.md#server-backup-snapshot-restart): Online backup steps without a held read snapshot, so continuous writes can restart a large backup indefinitely
- [`client-libs-journal-state-encoding`](findings/client-libs.md#client-libs-journal-state-encoding): Define journal-state SQL encodings and terminal sets once instead of repeating integer literals
- [`gap-sync-persists-unused-plaintext`](findings/gap.md#gap-sync-persists-unused-plaintext): SyncAccount and SyncTeam download every file and store its plaintext in soft state, but production code reads it back only for file sizes
- [`gap-mcp-adapter-clock-suspend`](findings/gap.md#gap-mcp-adapter-clock-suspend): MCP write admission treats every laptop sleep over five minutes as an untrusted clock, and a day off needs a CLI-only repair
- [`chat-code-inbox-sibling-sync-cancel`](findings/chat-code.md#chat-code-inbox-sibling-sync-cancel): Inbox job disposal cancels sibling teams' syncs and drops their pending dirty flag
- [`fe-arch-copy-action`](findings/fe-arch.md#fe-arch-copy-action): Use one clipboard-copy action; four first-run copy buttons silently drop failures
- [`fe-arch-mutation-action-hook`](findings/fe-arch.md#fe-arch-mutation-action-hook): Standardize write actions on command-policy with a shared hook instead of 33 hand-rolled busy flags

</details>

### Phase 1: Chat UI refinement on current IPC data

**Goal.** Move chat past MVP using only desktop-ui and the native shell, on the data the agent already returns. Do the chat refactors first so the UI work does not pile onto the 1,509-line send service and the 458-line ChatThread.

Enabling work first:
- Deterministic mock scenarios (busy, failures, integrity) exported for render tests. They replace ad-hoc reply rewriting and give the mockups and design review realistic data.
- Run mock chat replies through decodeChatReply. This requires the store-id mapping and 256-character truncation.
- A pure threadRows() model, then split ChatThread into ThreadHeader, MessageList, MessageRow and Composer.
- The outgoing transition() function with attempt tokens.
- A DraftStore with per-channel subscriptions, so typing no longer re-renders every row.
- Delete recover-pending, recovery-schedule and the unreachable reducer branches.

Ship the UI in this order:
1. Anchored open at the first unread plus the sliding window. Both use a shared `detached` flag, and gap=true must still mean reset.
2. Channel list: stable sort, previews, persisted folds, an honest menu, a preselected team.
3. Rendering and link safety.
4. Focusable rows with the action bar (Copy, Quote reply, Details) and keyboard navigation with a live region.
5. Composer UI steps 1-6.
6. The channel audience panel.
7. Delivery-state presentation: one row component, a copy table for codes, the unsent chip, Recheck channel.
8. Device-local mute and team alert defaults in the native chat_local settings.
9. Find-in-conversation.

Linux notifications (zbus), channel grouping and the badge are native-only and can run in parallel. The scheduling, backoff and access-guard refactors land as each service file is touched. Nothing here changes the Go wire format. Agent fields that later states need (senders, predecessor_unchecked, rejection_reason, persisted drafts) are annotated in the UI and delivered in Phase 3.

<details><summary>Findings in this phase</summary>

- [`chat-ux-mock-fidelity`](findings/chat-ux.md#chat-ux-mock-fidelity): Give the chat mock realistic scenarios so design review and screenshots reflect production
- [`chat-code-mock-bridge-decoder`](findings/chat-code.md#chat-code-mock-bridge-decoder): Route mock chat replies through decodeChatReply so render tests exercise the contract
- [`chat-ux-thread-decomposition`](findings/chat-ux.md#chat-ux-thread-decomposition): Split ChatThread into header, list, row and composer, and derive display rows as a pure model
- [`chat-code-outgoing-state-machine`](findings/chat-code.md#chat-code-outgoing-state-machine): Make the outgoing-message lifecycle an explicit state machine instead of `phase` plus seven flags
- [`chat-code-send-revision-rerender`](findings/chat-code.md#chat-code-send-revision-rerender): Every draft keystroke re-renders the whole chat pane through the send service's global revision
- [`chat-code-vestigial-modules`](findings/chat-code.md#chat-code-vestigial-modules): Remove or reconnect modules reached only from tests: recover-pending, recovery-schedule, conversation reducer branches, use-message-composer
- [`chat-code-conversation-hook-api`](findings/chat-code.md#chat-code-conversation-hook-api): Delete dead branches in useChatConversation; give useChatHistory an options object and stable callbacks
- [`chat-code-render-purity`](findings/chat-code.md#chat-code-render-purity): Lint does not catch render-time service mutations and ref writes in chat hooks
- [`chat-code-notification-limit-constant`](findings/chat-code.md#chat-code-notification-limit-constant): Move the 256-character notification text bound into chat-limits.json
- [`chat-code-ipc-contract-parity`](findings/chat-code.md#chat-code-ipc-contract-parity): Chat IPC contract is hand-maintained in five TS tables, and the mutation classification already disagrees with Rust
- [`chat-code-local-access-errors`](findings/chat-code.md#chat-code-local-access-errors): Unify the three local access guards; stop channel creation from throwing the agent's chat-access-denied code
- [`chat-code-scheduling-primitives`](findings/chat-code.md#chat-code-scheduling-primitives): Replace the inbox's fixed 250 ms drain loop and three hand-written timers with a shared deadline timer and clock module
- [`chat-code-backoff-formulas`](findings/chat-code.md#chat-code-backoff-formulas): Unify five retry-backoff formulas that already disagree on the first delay
- [`chat-code-pending-row-actions`](findings/chat-code.md#chat-code-pending-row-actions): Move protocol action selection out of PendingRow and OutgoingRow into the send service
- [`chat-code-send-service-decomposition`](findings/chat-code.md#chat-code-send-service-decomposition): Split ChatSendService (1509 lines) into draft store, outgoing queue, request gateway, recovery and scheduler modules
- [`chat-ux-open-at-first-unread`](findings/chat-ux.md#chat-ux-open-at-first-unread): Open channels at the first unread message, mark read by what was actually seen, and count arrivals while scrolled up
- [`chat-ux-history-window-sliding`](findings/chat-ux.md#chat-ux-history-window-sliding): Replace the hard 1,000-row view cap with a sliding window, and auto-load older pages
- [`chat-ux-channel-list`](findings/chat-ux.md#chat-ux-channel-list): Channel list: stable ordering, previews and times, draft and unread emphasis, persisted folds, honest context menu
- [`chat-ux-message-rendering`](findings/chat-ux.md#chat-ux-message-rendering): Message rendering: autolink bare URLs, keep paragraph breaks, style code blocks, highlight @mentions
- [`chat-ux-link-label-spoofing`](findings/chat-ux.md#chat-ux-link-label-spoofing): Markdown link labels can disguise the destination; show the host and confirm mismatches
- [`chat-ux-message-actions`](findings/chat-ux.md#chat-ux-message-actions): Add a hover/focus action bar per message: copy, quote-reply, copy link, details (no protocol change)
- [`chat-ux-keyboard-a11y`](findings/chat-ux.md#chat-ux-keyboard-a11y): Keyboard navigation and screen-reader support for the conversation
- [`chat-ux-composer`](findings/chat-ux.md#chat-ux-composer): Composer refinements: formatting affordances, mention autocomplete, draft indicators, Up-to-edit-unsent, character feedback
- [`chat-ux-channel-audience`](findings/chat-ux.md#chat-ux-channel-audience): Admins-only channels overstate their audience; show the real readers and the roles in channel info
- [`chat-ux-delivery-states`](findings/chat-ux.md#chat-ux-delivery-states): One legible delivery and integrity vocabulary: unify unsent-message rows, map rejection codes, and mark verification per message
- [`chat-ux-local-mute`](findings/chat-ux.md#chat-ux-local-mute): Make mute real: a device-local per-channel and per-team mute that drives badges, plus a 'Mentions only' alert mode
- [`chat-ux-history-search`](findings/chat-ux.md#chat-ux-history-search): Message search: start with find-in-conversation over decrypted history, then an agent-side encrypted index
- [`desktop-native-linux-notifications-and-badges`](findings/desktop-native.md#desktop-native-linux-notifications-and-badges): Linux chat notifications via org.freedesktop.Notifications, plus channel grouping and an unread badge

</details>

### Phase 2: Vault and desktop security UX outside chat

**Goal.** Fix the production correctness and security gaps in the vault UI and the native host, and restructure the frontend one feature at a time, behind a test harness that runs the production decoders.

Harness before moving code:
- Make the /src and /kit aliases resolvable outside Vite and import with real types.
- Run the mock bridge as a wire-level transport through checked/checkedMutation.
- Add a parity test between generate_handler! and the bridge literals.
After that, a moved module breaks at type-check rather than at runtime.

Security and correctness:
- Native idle, sleep and screen-lock lock, plus a Security settings section and ⌘L.
- A host copy_item_field command now; the catalog shape bit waits for Phase 3.
- Role-scoped Trash with Undo. Undo needs either the destination dirent from MoveKv or a catalog reload.
- Recovery-phrase test: a small local agent operation with no network access.
- A root error boundary and fault capture.
- Text size through native set_zoom, not zoomHotkeysEnabled.
- Toasts that live in a live region, pause on hover and focus, and show the clipboard countdown.
- A purge of profile-scoped localStorage on reset.
- A typed ACL AppManifest for the 127 commands, since a remote-origin admin window now exists; require_main_window on the window_state commands.
- Delete the unused create_link and copy_item_path commands.
- Startup failure classes with Try Again, and quarantine instead of deletion.

Structure, one feature per PR, smallest first (devices, teams, files, first-run):
- Layering lint and boundary tests.
- First-run step registry and context.
- DetailsPanel plaintext reducer.
- Shared pane-resize hook.
- CSS layers and the team-info.css fix.
- Shell services context.
- Screen splits.

The chat feature folder moves last, together with the Phase 1 decomposition. The PIN field's PinRejected and PinBlocked codes need agent error codes, which ship with Phase 3's error work.

<details><summary>Findings in this phase</summary>

- [`fe-arch-render-test-harness`](findings/fe-arch.md#fe-arch-render-test-harness): Replace per-file Vite servers in render tests with one typed harness and test folders that mirror the features
- [`fe-arch-mock-bridge-contract`](findings/fe-arch.md#fe-arch-mock-bridge-contract): Run the mock bridge through the production transport and decoders
- [`desktop-native-command-contract-check`](findings/desktop-native.md#desktop-native-command-contract-check): Check command names and argument keys between generate_handler! and the TS bridge
- [`desktop-native-auto-lock`](findings/desktop-native.md#desktop-native-auto-lock): App lock is manual-only; add a native inactivity, sleep and screen-lock trigger
- [`fe-product-auto-lock`](findings/fe-product.md#fe-product-auto-lock): No inactivity, sleep or screen-lock auto-lock; add a Security settings section
- [`fe-product-login-fields-copy`](findings/fe-product.md#fe-product-login-fields-copy): Production login classification and Copy act on the whole "user:/password:/url:" text
- [`fe-product-password-generator`](findings/fe-product.md#fe-product-password-generator): Password generator, reveal toggle and strength feedback in New and Edit
- [`fe-product-trash-undo`](findings/fe-product.md#fe-product-trash-undo): Permanent delete with no undo, and no folder delete; add a role-aware Trash built on MoveKv
- [`fe-product-organize-bulk`](findings/fe-product.md#fe-product-organize-bulk): Organising: folder-picker move, drag-to-folder, and multi-select bulk actions
- [`fe-product-search-quick-actions`](findings/fe-product.md#fe-product-search-quick-actions): ⌘K palette: quick copy actions, recents and favourites, folders and commands
- [`fe-product-keyboard-a11y`](findings/fe-product.md#fe-product-keyboard-a11y): Files keyboard model and ARIA semantics, plus a discoverable shortcut sheet
- [`fe-arch-files-list-a11y`](findings/fe-arch.md#fe-arch-files-list-a11y): Give the Files list and folder tree list/tree semantics and a single tab stop
- [`fe-product-share-to-team`](findings/fe-product.md#fe-product-share-to-team): Share or copy an item into a team vault
- [`fe-product-exposure-review`](findings/fe-product.md#fe-product-exposure-review): Exposure review and rotation checklist after removing a member or device
- [`fe-product-import-export`](findings/fe-product.md#fe-product-import-export): Starter card advertises a nonexistent `foks kv export`; add host-side CSV import and export
- [`fe-product-attention-centre`](findings/fe-product.md#fe-product-attention-centre): Single attention centre with a post-setup checklist and recent-activity history
- [`fe-product-unused-vault-ipc`](findings/fe-product.md#fe-product-unused-vault-ipc): Remove or use the unused `copy_item_path` and `create_link` IPC commands
- [`gap-recovery-phrase-test`](findings/gap.md#gap-recovery-phrase-test): Recovery phrases are never checked after they are written down; add a local 'Test recovery phrase' and a recall step
- [`gap-renderer-fault-capture`](findings/gap.md#gap-renderer-fault-capture): A render error outside the routed screen blanks the whole window and leaves no trace in Copy diagnostics
- [`gap-a11y-text-size`](findings/gap.md#gap-a11y-text-size): Text cannot be enlarged: no zoom, no text-size setting, and pixel font sizes throughout
- [`gap-toast-timing-announcements`](findings/gap.md#gap-toast-timing-announcements): Toasts vanish after 2.6 s with no pause, carry the only explanation for refused actions, and are announced unreliably
- [`gap-security-key-pin-field`](findings/gap.md#gap-security-key-pin-field): Eleven hand-rolled security-key PIN fields disagree with the 6-8 character rule and never show remaining attempts
- [`gap-storage-usage-display`](findings/gap.md#gap-storage-usage-display): Show storage used per vault; the agent already returns usage, but the desktop learns of limits only from a quota refusal
- [`fe-arch-device-storage`](findings/fe-arch.md#fe-arch-device-storage): Register webview localStorage keys in one module and purge profile-scoped entries when the device is reset
- [`fe-arch-details-panel-plaintext-reducer`](findings/fe-arch.md#fe-arch-details-panel-plaintext-reducer): Make DetailsPanel's plaintext lifecycle one reducer instead of four hand-maintained reset paths
- [`fe-arch-first-run-session`](findings/fe-arch.md#fe-arch-first-run-session): Split FirstRunSession into a setup-session context, a typed step registry and per-step components
- [`fe-arch-feature-folders`](findings/fe-arch.md#fe-arch-feature-folders): Move the flat src/ root and mixed screens/ into feature folders with enforced layering
- [`fe-arch-screen-split-shared-primitives`](findings/fe-arch.md#fe-arch-screen-split-shared-primitives): Split the 1,100-1,900-line screen files into model, sheets and view, and deduplicate their local primitives
- [`fe-arch-pane-resize-unify`](findings/fe-arch.md#fe-arch-pane-resize-unify): Replace the shell's private sidebar-resize hook with the shared pane-resize hook
- [`fe-arch-css-structure`](findings/fe-arch.md#fe-arch-css-structure): Split app.css by feature, bring team-info.css into the cascade layers, and auto-discover stylesheets in the style test
- [`fe-arch-shell-services-context`](findings/fe-arch.md#fe-arch-shell-services-context): Provide shell services through context instead of passing bridge, snapshot and error handlers through every screen
- [`fe-arch-code-splitting`](findings/fe-arch.md#fe-arch-code-splitting): Lazy-load first-run, the chat views and the settings screens to shrink the 1.06 MB startup chunk
- [`desktop-native-startup-reset-safety`](findings/desktop-native.md#desktop-native-startup-reset-safety): A startup timeout can lead to a destructive state reset; add retry, failure classification and quarantine
- [`desktop-native-app-acl-for-commands`](findings/desktop-native.md#desktop-native-app-acl-for-commands): Put the 127 custom commands behind Tauri's app ACL now that a remote-origin window exists
- [`desktop-native-quit-confirmation`](findings/desktop-native.md#desktop-native-quit-confirmation): Quit and window close always show a 'stop agent' modal, even when nothing is pending
- [`desktop-native-linux-parity`](findings/desktop-native.md#desktop-native-linux-parity): Small Linux parity gaps: device and user name defaults, clipboard-history hint, link opening
- [`desktop-native-context-split`](findings/desktop-native.md#desktop-native-context-split): Split commands/context.rs and give store identities a typed key
- [`desktop-native-operation-builders-triplicated`](findings/desktop-native.md#desktop-native-operation-builders-triplicated): Agent Operations are built in three places; DesktopModel is dead and the real-agent CI test bypasses the shipping code

</details>

### Phase 3: Agent and IPC contract hardening, plus the agent additions chat needs

**Goal.** Give the agent a versioned, typed and classifiable contract. Then deliver the agent fields and actions that the Phase 1 and Phase 2 UIs annotated as prerequisites, and bring the CLI and MCP up to parity with the desktop under explicit consent.

Contract first. Batch PROTOCOL_VERSION bumps behind the new handshake.
- Hello with a frozen version-mismatch envelope, never wrapped as Ambiguous.
- One outcome and recovery table: a client-lib ErrorClass with exhaustive matches, then agent ErrorCode with outcome and retry, then desktop, CLI exit codes and MCP structuredContent. It replaces the message-regex classification.
- Proto-typed responses replacing the 44 mirror structs.
- A Busy reply when the agent is at connection capacity, instead of dropping the socket.
- A per-request budget_ms.
- A paged catalog, and a typed response-too-large error.
- Policy-table snapshot, lib split, diagnostics and rotated logs.
- A checkpoint-deferred warning, so a committed change is not reported as rejected.

Chat agent additions:
- Per-message verdicts and an `unverifiable` content kind, so the stopped-channel pane becomes per-row placeholders.
- Read-side Pegged open; sending stays deferred to Phase 5.
- senders[] with former-member labels.
- predecessor_unchecked per message and rejection_reason.
- Agent-side draft save and load.
- Agent-owned alert and hide preferences, synced through the personal KV namespace.
- A bounded verified-history search scan that persists no plaintext.
- A subscription stream that moves polling out of the renderer. The CLI chat commands and MCP chat tools depend on it, and MCP chat ships only behind client sessions, audit and revocation.

Vault agent additions:
- The catalog shape bit for fe-product-login-fields-copy.
- server_time and a bounded version ring for item history.
- Signed chain events for the activity timeline.
- Checkpoint display and compare.
- An advisory clock-skew estimate.
- An offline read-only mode with sealed key material. This must come after Phase 0 removed plaintext from soft state.
- The adapter clock repair operation, kept off the MCP tool surface (gap-mcp-adapter-clock-suspend step 4).

Move the CLI onto the agent one command group at a time, starting with kv, and keep an --offline mode that takes the exclusive lock.

<details><summary>Findings in this phase</summary>

- [`agent-ipc-version-handshake`](findings/agent.md#agent-ipc-version-handshake): Replace exact-equality IPC versioning with a frozen hello/capability handshake
- [`agent-error-disposition`](findings/agent.md#agent-error-disposition): Define error outcome/retry/category in foks-agent-proto and use it for CLI exit codes and MCP tool errors
- [`desktop-native-error-code-registry`](findings/desktop-native.md#desktop-native-error-code-registry): Replace free-form error-code strings with a registry shared by Rust and TypeScript
- [`client-libs-error-classification`](findings/client-libs.md#client-libs-error-classification): Replace catch-all string errors with an exhaustive error class so the agent stops classifying by downcast
- [`agent-capacity-drop-false-ambiguity`](findings/agent.md#agent-capacity-drop-false-ambiguity): Answer connection-capacity rejections with a definite Busy instead of silently closing the socket
- [`agent-request-deadline`](findings/agent.md#agent-request-deadline): Carry the client's deadline in each request instead of an agent-wide timeout set by whichever front end launched the agent
- [`agent-data-catalog-unbounded`](findings/agent.md#agent-data-catalog-unbounded): Page DataRead::Catalog, add single-path lookup for MCP, and turn oversize responses into a typed error
- [`desktop-native-typed-agent-responses`](findings/desktop-native.md#desktop-native-typed-agent-responses): Stop mirroring agent response types in the desktop; decode into foks-agent-proto types
- [`client-libs-agent-dto-duplication`](findings/client-libs.md#client-libs-agent-dto-duplication): Stop duplicating foks-agent-proto DTOs as Serialize-only report types in client-app
- [`agent-bot-contract-typed`](findings/agent.md#agent-bot-contract-typed): Define bot replies as typed proto structs, generate the TypeScript decoder, and report loaded state
- [`agent-operation-policy-table`](findings/agent.md#agent-operation-policy-table): Snapshot every operation's policy row so the eight classifiers cannot drift apart
- [`agent-main-decomposition-lib`](findings/agent.md#agent-main-decomposition-lib): Split foks-agent into a library with domain dispatch modules and an explicit AgentContext
- [`agent-observability-logs-diagnostics`](findings/agent.md#agent-observability-logs-diagnostics): Give the agent its own bounded, timestamped log, counters and a Diagnostics operation
- [`desktop-native-agent-log-and-crash-markers`](findings/desktop-native.md#desktop-native-agent-log-and-crash-markers): agent.log grows without bound and crash markers are never read
- [`client-libs-checked-session-result-loss`](findings/client-libs.md#client-libs-checked-session-result-loss): Checked-session wrapper reports a committed operation as failed when checkpoint publication fails
- [`chat-backend-single-message-poisons-channel`](findings/chat-backend.md#chat-backend-single-message-poisons-channel): One malformed message from any channel writer stops reading and sending in that channel
- [`protocol-chat-pegged-reply-bodies`](findings/protocol.md#protocol-chat-pegged-reply-bodies): Open the pinned Go Pegged bodies (Reply first) and stop blocking sends after an unsupported message
- [`chat-ux-sender-identity`](findings/chat-ux.md#chat-ux-sender-identity): Resolve every sender to a verified username, including departed members, and mark former members
- [`chat-backend-notification-prefs-mentions-names`](findings/chat-backend.md#chat-backend-notification-prefs-mentions-names): Agent-owned per-channel alert/hide preferences synced via personal KV, local mentions and sender labels
- [`chat-backend-history-search`](findings/chat-backend.md#chat-backend-history-search): Bounded verified history search in the agent, without server or plaintext persistence
- [`agent-chat-subscription-stream`](findings/agent.md#agent-chat-subscription-stream): Move chat inbox polling into the agent and expose a subscription stream to all front ends
- [`agent-cli-chat-and-parity`](findings/agent.md#agent-cli-chat-and-parity): Add CLI chat commands and close remaining CLI parity gaps (team KV, mv/stat, pending work, agent status, completions)
- [`agent-cli-direct-state-access`](findings/agent.md#agent-cli-direct-state-access): Route the remaining direct-access CLI commands through the agent
- [`agent-mcp-consent-audit`](findings/agent.md#agent-mcp-consent-audit): Add agent-enforced assistant grants, an audit log, revocation and desktop-lock propagation for MCP sessions
- [`agent-mcp-chat-tools`](findings/agent.md#agent-mcp-chat-tools): Expose a guarded MCP chat tool set on the existing chat operations
- [`agent-mcp-tool-metadata`](findings/agent.md#agent-mcp-tool-metadata): Write real MCP tool descriptions and correct the tool annotations
- [`gap-clock-skew-diagnosis`](findings/gap.md#gap-clock-skew-diagnosis): A wrong system clock surfaces as an opaque check-in or compatibility failure
- [`gap-fork-consistency-compare`](findings/gap.md#gap-fork-consistency-compare): Let users compare signed checkpoints, the out-of-band root comparison book chapter 13 recommends
- [`fe-product-signed-activity`](findings/fe-product.md#fe-product-signed-activity): Show signed device and team history from the verified sigchains
- [`fe-product-version-history`](findings/fe-product.md#fe-product-version-history): Item version history and restore from versions this device has seen (no protocol change)
- [`gap-offline-read-only-vault`](findings/gap.md#gap-offline-read-only-vault): No read access to vault items while the server is unreachable; add an explicit read-only offline mode with sealed key material

</details>

### Phase 4: Client, protocol and server robustness

**Goal.** Harden the client libraries and protocol crates, and build the server-side mechanisms that the retention and quota stages in ISSUES.md require. The server track can start in parallel with Phase 1.

Client libraries:
- Split HardStateStore::open into open_existing and create_or_open first. This stops read paths from recreating hard state after a reset.
- Then an OperationContext that replaces the thread-local session material and the raw master-key parameters.
- A SecretArray<N> newtype, so vault records redact Debug and zeroize on drop.
- A stepwise migration ladder with v38 and v39 predecessors.
- ActingCredential, which also gives Yubi accounts view-grant cache reuse.
- A pure function for resolving team rekey intents.

Protocol:
- Count-only prefix decode for seed chains.
- Private randomness structs with generate() and distinctness checks.
- Shared hybrid seal helpers.
- Schema, argument and verify fuzz targets seeded with the 137 Go fixtures, with a persisted corpus.
- A generated Status enum and sanitised status detail.
- A type-ID registry with collision tests.
- A differential test before the verifier refactor.

Server, with no wire changes:
- Per-source and per-UID admission caps, and a per-source KEX share.
- One status classifier that maps the database-full condition and capacity refusals to 1060; add 1060 to definite_rejection on the client.
- A transactional usage ledger with a reserve tier, plus Stage 1 envelope deduplication and counters.
- Writer deadlines and priority lanes.
- Incremental Merkle prepare, using the full rebuild as a differential oracle.
- Per-route status metrics and latency histograms.
- A cheap /metrics endpoint.
- server.toml with deny_unknown_fields and limit tables.
- An operator usage, suspension and waitlist console.
- A migration framework with frozen schema snapshots.

Land the extensible capability response shape and the negotiation rules now, while nothing calls them, together with the durable method namespace ISSUES.md requires. They gate Phase 5.

<details><summary>Findings in this phase</summary>

- [`client-libs-operation-scoped-context`](findings/client-libs.md#client-libs-operation-scoped-context): Give CheckedProfileSession an operation-scoped context instead of per-call DB opens, thread-local session material and raw master-key parameters
- [`client-libs-secret-record-redaction`](findings/client-libs.md#client-libs-secret-record-redaction): Vault records with raw seeds derive Debug and rely on hand-written Drop zeroization
- [`client-libs-hard-state-migration-ladder`](findings/client-libs.md#client-libs-hard-state-migration-ladder): Replace ad-hoc hard-state version branches with a stepwise migration ladder and frozen historical fixtures
- [`client-libs-acting-credential`](findings/client-libs.md#client-libs-acting-credential): Collapse the _yubi / _with_credential / _as_local_team method matrix behind one acting-credential type
- [`client-libs-team-intent-layering`](findings/client-libs.md#client-libs-team-intent-layering): Move team rekey/member-edit/expulsion intent state machines out of client-app runtime into foks-client
- [`client-libs-soft-stage-reclaim`](findings/client-libs.md#client-libs-soft-stage-reclaim): Wire orphaned large-file stage reclamation into exclusive session entry
- [`client-libs-kv-scope-api`](findings/client-libs.md#client-libs-kv-scope-api): Unify personal/team KV methods behind a KvScope and a shared write-session prologue
- [`client-libs-oidc-deadline`](findings/client-libs.md#client-libs-oidc-deadline): OIDC provider discovery ignores the FoksClient deadline and cancellation token
- [`protocol-seed-chain-plaintext-nonzeroizing-decode`](findings/protocol.md#protocol-seed-chain-plaintext-nonzeroizing-decode): Seed-chain and KV plaintexts go through the non-zeroizing decode_prefix, contrary to the book's zeroization rule
- [`protocol-box-randomness-secret-api`](findings/protocol.md#protocol-box-randomness-secret-api): Box randomness structs expose secret material as plain Copy arrays and leave key/nonce uniqueness to the caller
- [`protocol-hybrid-seal-duplication`](findings/protocol.md#protocol-hybrid-seal-duplication): hybrid.rs repeats the hybrid KDF and typed-nonce secretbox nine times, with inconsistent input validation
- [`protocol-fuzz-schema-and-verification-decoders`](findings/protocol.md#protocol-fuzz-schema-and-verification-decoders): Fuzz the schema decoders, server argument decoders and verifiers, and seed the corpora with the Go fixtures
- [`build-fuzz-corpus-persistence`](findings/build.md#build-fuzz-corpus-persistence): Weekly fuzz campaigns discard their corpus and run on a floating nightly
- [`protocol-typed-status-codes-and-detail`](findings/protocol.md#protocol-typed-status-codes-and-detail): Replace magic status-code literals with typed statuses, and limit server-supplied status text
- [`protocol-type-id-registry-and-drift`](findings/protocol.md#protocol-type-id-registry-and-drift): Hand-written 64-bit type IDs have no upstream drift check and no collision registry
- [`protocol-verify-shared-chain-step`](findings/protocol.md#protocol-verify-shared-chain-step): Share one per-link step between the full and incremental user/team verifiers, and test that they agree
- [`protocol-parcel-open-expectation-struct`](findings/protocol.md#protocol-parcel-open-expectation-struct): Replace the six-function parcel-opening ladder with a named-field expectation struct
- [`protocol-crypto-public-api-docs-and-ct-eq`](findings/protocol.md#protocol-crypto-public-api-docs-and-ct-eq): Document the foks-crypto root API, type its raw-seed signers, and use constant-time equality on secret types
- [`protocol-crypto-test-organization`](findings/protocol.md#protocol-crypto-test-organization): Split the 2853-line foks-crypto tests.rs and consolidate the Yubi test doubles and fixture loaders
- [`build-crate-layer-boundaries`](findings/build.md#build-crate-layer-boundaries): Replace vacuous monorepo path-boundary checks with an enforced crate-layer allowlist; document the desktop's direct client-app use
- [`build-benchmark-provenance`](findings/build.md#build-benchmark-provenance): README and book benchmark numbers have no committed result file or revision
- [`server-error-status-mapping`](findings/server.md#server-error-status-mapping): About 13 divergent map_write_error functions: a full database is reported as retryable, and internal error text leaks to clients
- [`chat-backend-capacity-refusals-become-uncertain`](findings/chat-backend.md#chat-backend-capacity-refusals-become-uncertain): Send-side capacity refusals are reported as generic 12001, leaving operations permanently Uncertain
- [`server-source-admission-fairness`](findings/server.md#server-source-admission-fairness): No per-source concurrency caps: one IP can hold every public connection slot, and one account can take every realtime poll slot
- [`server-storage-usage-ledger`](findings/server.md#server-storage-usage-ledger): Replace per-write KV namespace scans with a transactional usage ledger and add a storage reserve for identity writes
- [`chat-backend-stage1-redundant-envelope-and-counters`](findings/chat-backend.md#chat-backend-stage1-redundant-envelope-and-counters): Stage 1 quota: drop the redundant envelope column and keep retained-byte counters in the send transaction
- [`server-writer-deadlines`](findings/server.md#server-writer-deadlines): Writer queue runs abandoned jobs and has no priority lanes or per-principal fairness
- [`server-merkle-incremental-prepare`](findings/server.md#server-merkle-incremental-prepare): Full Merkle rebuild on the single writer thread grows to seconds per identity/team mutation
- [`server-request-failure-observability`](findings/server.md#server-request-failure-observability): Request failures are invisible: no per-route/status metrics, no internal-error reasons, no latency histograms
- [`server-metrics-scrape-cost`](findings/server.md#server-metrics-scrape-cost): Every /metrics scrape and every new pooled reader full-scans the KV tables
- [`server-config-surface`](findings/server.md#server-config-surface): server.toml cannot set any limit, ignores unknown keys, and gives misleading validation errors
- [`server-operator-capacity-console`](findings/server.md#server-operator-capacity-console): Operators have no capacity, headroom or abuse controls (usage report, account suspension, waitlist)
- [`server-session-decomposition`](findings/server.md#server-session-decomposition): net/session.rs mixes transport and domain logic, and request authorization relies on cloning ServerData by convention
- [`server-migration-framework`](findings/server.md#server-migration-framework): Schema upgrades are one ad-hoc branch block tested only against fixtures built by reversing the current schema
- [`protocol-chat-capabilities-extensible-shape`](findings/protocol.md#protocol-chat-capabilities-extensible-shape): Make the chat capability response extensible before any extended-chat feature is advertised
- [`chat-backend-capability-negotiation-not-forward-compatible`](findings/chat-backend.md#chat-backend-capability-negotiation-not-forward-compatible): Chat capability discovery fails closed on version skew, forces format 2 for every feature, and is unused by the agent

</details>

### Phase 5: Negotiated chat extensions and distribution

**Goal.** Ship extended chat only through capability-gated, Go-interop-safe Rust-only methods, and give every product a signed update and release path.

Prerequisites: the Phase 3 per-message verdicts, so a new message kind can never stop a channel, and the Phase 4 capability shape plus method namespace.

Order:
1. An exact_filtered_inbox capability bit, so a revocation no longer degrades the inbox on the Rust server.
2. rtGetChannel at position 1. Measure first; an agent-side name cache may be enough.
3. foksChatUpdateChannel: rename, description, posting role and archive. Before relying on it, add oracle cases showing that Go v0.1.9 tolerates channel metadata with sequence > 1. Archive is enforced as read-only only by Rust clients and servers.
4. Opt-in presence: typing, plus read states that are off by default.
5. Sending Pegged Reply, Edit and Reaction, only when negotiation excludes Go v0.1.9 readers and the Rust server's Basic-only rule is lifted.
6. KV-backed attachments. This is Go-compatible and can start once Phase 3 lands, but it needs an owner to provision /.chat.
7. A retention history floor, then Stage 2 time-ordered submission IDs with a durable floor that never passes a saved intent.
8. Delete via a format-2 tombstone, only after the floor exists.

Distribution:
- A signed macOS updater, and a notice-only update path for .deb.
- A universal macOS build, rpm, aarch64 Linux, and Windows recorded as a non-goal.
- Standalone server, agent and CLI tarballs with cargo-auditable builds, checksums, attestations and verified systemd units. Reconcile the server.toml path in the unit with the one the init flow writes.

<details><summary>Findings in this phase</summary>

- [`chat-backend-revocation-degrades-inbox`](findings/chat-backend.md#chat-backend-revocation-degrades-inbox): The FOKS-RS server itself puts an inbox into the degraded state on every access revocation
- [`chat-backend-get-channel-read-path`](findings/chat-backend.md#chat-backend-get-channel-read-path): Every chat read lists and decrypts all channels; implement upstream rtGetChannel and reuse reads for mark-read
- [`chat-backend-channel-management-format1`](findings/chat-backend.md#chat-backend-channel-management-format1): Channel rename, description, posting role and archive via a Rust-only update method that keeps Go wire metadata
- [`chat-backend-presence-typing-receipts`](findings/chat-backend.md#chat-backend-presence-typing-receipts): Typing indicators and other members' read positions as capability-gated ephemeral extensions
- [`chat-backend-attachments-via-team-kv`](findings/chat-backend.md#chat-backend-attachments-via-team-kv): Attachments stored in the team KV under channel-scoped roles, referenced from Basic text
- [`chat-backend-retention-floor-vs-anchor-ledger`](findings/chat-backend.md#chat-backend-retention-floor-vs-anchor-ledger): Define retention as an explicit history floor; the client's gap, predecessor and recovery logic assume dense sequences
- [`chat-backend-stage2-submission-floor`](findings/chat-backend.md#chat-backend-stage2-submission-floor): Stage 2 compaction: time-ordered submission IDs plus a durable floor make expiry safe
- [`desktop-native-updater`](findings/desktop-native.md#desktop-native-updater): No update channel: add signed in-app updates for macOS and an update notice for the .deb
- [`desktop-native-release-matrix`](findings/desktop-native.md#desktop-native-release-matrix): Widen the release matrix (universal macOS, rpm, aarch64 Linux) and record Windows as an explicit non-goal
- [`build-release-standalone-binaries`](findings/build.md#build-release-standalone-binaries): No release pipeline for foks-server, foks-agent and foks-rs CLI despite shipped systemd units

</details>

## Chat capability matrix

What each chat capability looks like today, which layer blocks the next step, and the path to it.

| Capability | Today | Blocked at | Path | Findings |
|---|---|---|---|---|
| Composer | Auto-growing textarea. Enter sends; Shift/Alt+Enter adds a newline, with no hint. No formatting shortcuts or @-completion. The size meter appears only above 75% of 64 KiB. Drafts are kept per channel in memory and lost on lock or quit. An unsent message can be restored only through a small 'Edit' link. Every keystroke re-renders the whole pane. | ui-only | Add a hint row and a cheat sheet for the subset message-text.tsx renders, Ctrl/⌘+B/I/E, @-autocomplete from partyNames filtered to channel readers, Up in an empty composer to restore the newest unsent message, a byte-based remaining budget (not characters), and a draft glyph in the column. A DraftStore with per-channel subscriptions removes the per-keystroke re-render. Drafts that survive lock or restart need agent save/load-draft actions in the protected store (agent-ipc). | [`chat-ux-composer`](findings/chat-ux.md#chat-ux-composer), [`chat-code-send-revision-rerender`](findings/chat-code.md#chat-code-send-revision-rerender), [`chat-code-send-service-decomposition`](findings/chat-code.md#chat-code-send-service-decomposition) |
| Message rendering | A bounded markdown subset: `code`, **bold**, *italic*, [label](url), fences, lists and quotes. Bare URLs are not linked. Blank lines are dropped, so paragraphs collapse. Code blocks have no styling and an always-visible Copy code button. List bullets hang outside the text column. @names get no styling. | ui-only | Add a bare-URL alternative passed through safeChatLink with trailing-punctuation trimming, paragraph spacing, a styled pre with hover copy and a line clamp, indented lists, and mention/mention-self classes from partyNames. Keep the renderer non-recursive. | [`chat-ux-message-rendering`](findings/chat-ux.md#chat-ux-message-rendering) |
| Link safety | [label](url) shows only the label, with no title attribute. open_chat_link opens any http(s) URL through open/xdg-open without confirmation. | ui-only | Set title={url}, add a muted host suffix when the label differs from the URL, and warn and ask for confirmation, naming both hosts, when the label parses as a different host. If a host-enforced step is wanted, show a native dialog on every chat link; a renderer-controlled confirm flag would enforce nothing. | [`chat-ux-link-label-spoofing`](findings/chat-ux.md#chat-ux-link-label-spoofing) |
| Sender identity | Names come only from the current roster. Former members, and every sender while the roster is not loaded, show as a truncated hex ID. The mock shows 'Team member' from a null sender that production cannot produce. | agent-ipc | Add senders[{uid, username?, member}] to ChatResult::History. The agent loads each sender's user chain with the team view token (check Go v0.1.9 behaviour) and caches it in a bounded LRU. The desktop falls back from roster name, to the history name with a 'former member' tag, to 'Unknown member' with Copy ID. | [`chat-ux-sender-identity`](findings/chat-ux.md#chat-ux-sender-identity), [`chat-backend-notification-prefs-mentions-names`](findings/chat-backend.md#chat-backend-notification-prefs-mentions-names), [`chat-ux-mock-fidelity`](findings/chat-ux.md#chat-ux-mock-fidelity) |
| Unread, read marking and jump | A channel opens pinned to the bottom, and the newest message is marked read about 300 ms later if the window is focused, whatever was on screen. With more than 50 unread, the first unread row is never loaded. The NEW divider is set once and never cleared. Jump to latest is an icon-only chevron with no count. | ui-only | Add an anchored open (before = read_through+41) as its own load mode, a detached window with explicit forward paging, and IntersectionObserver read marking (the server pointer is monotonic). Add a sticky 'N new since' banner with Esc to mark read, and a 'N new messages ↓' pill. | [`chat-ux-open-at-first-unread`](findings/chat-ux.md#chat-ux-open-at-first-unread), [`chat-ux-keyboard-a11y`](findings/chat-ux.md#chat-ux-keyboard-a11y) |
| History paging and scrollback | The view throws once it passes 1,000 rows or 8 MiB. On the tail path the window collapses to the incoming page, which can be one row. The cache budget is shared by all 16 channels, so one long scrollback evicts the rest. Older history loads only through an explicit button. Scroll position is not restored when switching back. | ui-only | Trim instead of throwing, with a per-channel target of about 400 rows / 3 MiB. Add a top IntersectionObserver sentinel and keep the button as a fallback. Store {anchor, offset} per channel and restore it when the thread remounts. | [`chat-ux-history-window-sliding`](findings/chat-ux.md#chat-ux-history-window-sliding) |
| Channel list | Ordered by inbox_version, which the server bumps on your own reads and sends, so the list reorders as you read. Rows show no preview or time, although the inbox carries previews. Folds reset on remount. Edit and Delete are permanently disabled. 'New chat' does not preselect the open team. The README describes a different column. | ui-only | Sort in listChannels: general first, then alphabetical, with an optional Recent activity sort by preview.insert_time. Add a preview line behind a density toggle, persisted folds, a menu without the dead items plus Mark as read and Mute, a preselected team, 'New channel' naming, and a README update. | [`chat-ux-channel-list`](findings/chat-ux.md#chat-ux-channel-list) |
| Message actions | The only action is 'Copy message', in a context menu opened from a non-focusable div. | ui-only | Add a MessageActions bar on hover and focus-within with Copy, Quote reply (Basic text blockquote into the composer) and Details (receive time, sender clock, sequence, page-level verification wording). Copy link waits for a portable identifier and a registered URI scheme, because store refs are local JSON. | [`chat-ux-message-actions`](findings/chat-ux.md#chat-ux-message-actions), [`chat-ux-keyboard-a11y`](findings/chat-ux.md#chat-ux-keyboard-a11y) |
| Edit | None. Edit=2 is a pinned v0.1.9 Pegged body, but the Rust server rejects non-Basic sends, foks-crypto opens only Basic bodies, history shows them as Unsupported, and a non-Basic latest message blocks sending. Go v0.1.9 clients fail the whole history page on a non-Basic body. | protocol-server | First, per-message verdicts so an unsupported row never blocks a channel. Then open Pegged bodies on the read side, with no wire change. Send only under a negotiated capability that excludes Go v0.1.9 readers and lifts the server's Basic-only rule. | [`chat-backend-single-message-poisons-channel`](findings/chat-backend.md#chat-backend-single-message-poisons-channel), [`protocol-chat-pegged-reply-bodies`](findings/protocol.md#protocol-chat-pegged-reply-bodies), [`chat-backend-capability-negotiation-not-forward-compatible`](findings/chat-backend.md#chat-backend-capability-negotiation-not-forward-compatible), [`protocol-chat-capabilities-extensible-shape`](findings/protocol.md#protocol-chat-capabilities-extensible-shape) |
| Delete | Only an enum value, with no body arm. Client gap, predecessor and recovery logic assume dense sequences, and the anchor ledger rejects any change to a stored digest. | deferred-by-design | A format-2 Delete purpose, plus an anchor-ledger rule that lets a digest move once to a tombstone under an authorized delete event. It depends on the retention history floor existing first. | [`chat-backend-retention-floor-vs-anchor-ledger`](findings/chat-backend.md#chat-backend-retention-floor-vs-anchor-ledger) |
| Reactions | None. Reaction=4 is a pinned Pegged kind; the server rejects it and clients show it as Unsupported. | protocol-server | Same route as Edit: render reactions read-only after the read-side Pegged open, and send only under negotiation that excludes Go v0.1.9 readers. | [`protocol-chat-pegged-reply-bodies`](findings/protocol.md#protocol-chat-pegged-reply-bodies), [`chat-backend-capability-negotiation-not-forward-compatible`](findings/chat-backend.md#chat-backend-capability-negotiation-not-forward-compatible) |
| Replies and threads | No structured replies. Blockquotes render, so a quote-reply as Basic text is possible now. | protocol-server | Quote-reply as Basic text now (ui-only). Read-side rendering of Reply bodies, with a parent strip, after the client-lib change. Sending Replies under negotiation. Threads are not designed yet. | [`chat-ux-message-actions`](findings/chat-ux.md#chat-ux-message-actions), [`protocol-chat-pegged-reply-bodies`](findings/protocol.md#protocol-chat-pegged-reply-bodies) |
| Mentions | None. Protocol mentions are deferred in ISSUES.md:187-188. | ui-only | @-autocomplete and highlighting as plain Basic text. A 'Mentions only' alert mode, labelled as a local text match within the 256 characters of notification text. Later, an agent mentions_actor flag resolved against the verified roster. Structured mentions are a format-2 item. | [`chat-ux-composer`](findings/chat-ux.md#chat-ux-composer), [`chat-ux-message-rendering`](findings/chat-ux.md#chat-ux-message-rendering), [`chat-ux-local-mute`](findings/chat-ux.md#chat-ux-local-mute), [`chat-backend-notification-prefs-mentions-names`](findings/chat-backend.md#chat-backend-notification-prefs-mentions-names) |
| Attachments | None. Only an enum value and a capability bit exist. Message bodies are capped near 1 MiB. | agent-ipc | Go-compatible phase 1. An owner provisions /.chat once. The channel creator makes /.chat/<channel-id> with the channel's roles. The agent journals a KV put followed by a Basic send that carries a foks-kv: reference and the SHA-256 of the plaintext. Clients render a card, verify the hash, and show 'File no longer available' or 'Hash mismatch'. A format-2 Attachment purpose comes later. | [`chat-backend-attachments-via-team-kv`](findings/chat-backend.md#chat-backend-attachments-via-team-kv) |
| Read receipts (other members) | The server stores every member's read_through but returns only the caller's own. | protocol-server | A capability-gated foksChatReadStates call with a per-user share_read_state opt-in, off by default. | [`chat-backend-presence-typing-receipts`](findings/chat-backend.md#chat-backend-presence-typing-receipts) |
| Typing indicators | None. The poll result has the Go shape {bumped, inbox_version}, and inbox changes are durable writes. | protocol-server | An in-memory per-channel hub (foksChatTyping and foksChatPresencePoll) with about a 6 s TTL, rate-limited and never persisted. The agent polls only the focused channel. | [`chat-backend-presence-typing-receipts`](findings/chat-backend.md#chat-backend-presence-typing-receipts) |
| Channel creation | Works, journaled and recoverable. The header 'New chat' opens 'Choose a team' even when a team is open. The local guard throws the agent's chat-access-denied code. The 256-channel cap comes back as 12001, leaving the operation permanently Uncertain. | ui-only | Preselect the open team and name the action 'New channel'. A unified local access guard. Server returns QuotaExceeded (1060) at the cap, and clients treat 1060 as a definite rejection. | [`chat-ux-channel-list`](findings/chat-ux.md#chat-ux-channel-list), [`chat-code-local-access-errors`](findings/chat-code.md#chat-code-local-access-errors), [`chat-backend-capacity-refusals-become-uncertain`](findings/chat-backend.md#chat-backend-capacity-refusals-become-uncertain) |
| Channel management (rename, description, posting role, archive, delete) | Configuration is immutable by schema, there is no upstream method, and the menu shows disabled Edit and Delete items. | protocol-server | A capability-gated foksChatUpdateChannel (v2 capability response, durable vendor method namespace) with expected_sequence and set_version CAS, a client revision table, and oracle checks that Go v0.1.9 tolerates sequence > 1. Archive is enforced only by Rust clients and servers. Read-role changes are format-2 only. Delete waits for the retention floor. | [`chat-backend-channel-management-format1`](findings/chat-backend.md#chat-backend-channel-management-format1), [`chat-backend-capability-negotiation-not-forward-compatible`](findings/chat-backend.md#chat-backend-capability-negotiation-not-forward-compatible) |
| Members and audience | The header count, the empty-state sentence and the info panel all count the whole team roster, even for admins-only or banded channels. The panel shows only 'Visibility'. | ui-only | channelAudience() reusing admits() and readersOf(), with 'Who can read' and 'Who can post' rows and a reader list tagged can post / read only. | [`chat-ux-channel-audience`](findings/chat-ux.md#chat-ux-channel-audience) |
| Notification preferences and mute | The server always returns muted=false and hidden=false, and nothing can set them. The native shell keeps a per-channel alert override (inherit/all/none), but unread badges ignore it. There is no team default and no mentions-only mode. | ui-only | A device-local muted set and per-team default in the native chat_local settings, merged into the published inbox projection so badges follow it. Later, agent-owned hard-state preferences synced through a reserved path in the personal KV store. Record in ISSUES.md that server-side mute is impossible on the pinned protocol. | [`chat-ux-local-mute`](findings/chat-ux.md#chat-ux-local-mute), [`chat-backend-notification-prefs-mentions-names`](findings/chat-backend.md#chat-backend-notification-prefs-mentions-names) |
| Desktop notifications | macOS only, with a fixed 'FOKS' title, no per-channel grouping and no unread badge on the app icon. Linux has stubs, so Linux users get no alerts. | ui-only | Native shell work: a zbus org.freedesktop.Notifications backend, threadIdentifier per channel, set_badge_count, and un-gating the macOS-only route and activate functions. Alerts while the renderer is not running need agent-held subscriptions (agent-ipc). | [`desktop-native-linux-notifications-and-badges`](findings/desktop-native.md#desktop-native-linux-notifications-and-badges), [`agent-chat-subscription-stream`](findings/agent.md#agent-chat-subscription-stream) |
| Background sync and multi-client chat | The renderer owns the inbox poll loop and the agent allows one poll per account, so detection stops when the renderer stops. The CLI has no chat commands and MCP has no chat tools. | agent-ipc | An agent Subscribe stream with one reference-counted poll per account and OperationChanged events. Then CLI chat commands and MCP chat tools, behind client sessions, audit and revocation. | [`agent-chat-subscription-stream`](findings/agent.md#agent-chat-subscription-stream), [`agent-cli-chat-and-parity`](findings/agent.md#agent-cli-chat-and-parity), [`agent-mcp-chat-tools`](findings/agent.md#agent-mcp-chat-tools), [`agent-mcp-consent-audit`](findings/agent.md#agent-mcp-consent-audit) |
| Search | Names only. The topbar field shows a '#channel' scope badge but filters names. The palette has no Messages scope. | ui-only | Step 1 (ui-only): find-in-conversation over the loaded window, with a transient scan of older pages that does not grow the view. Step 2 (agent): a bounded backward scan over verified pages with no persisted plaintext. An optional sealed index later, after the change to plaintext-at-rest policy is recorded in book/16-chat.qmd. | [`chat-ux-history-search`](findings/chat-ux.md#chat-ux-history-search), [`chat-backend-history-search`](findings/chat-backend.md#chat-backend-history-search) |
| Direct messages | Ad-hoc teams can be created, but both client and server refuse chat on non-named teams. | deferred-by-design | Not proposed by this review. It needs chat support for ad-hoc teams on client and server, membership semantics, and a Go interop check before any UI. |  |
| Delivery states | Unsent work appears in three layouts: OutgoingRow, the bare PendingRow, and a 'Needs attention' section. Rejections show raw codes and send errors show agent developer strings. Capacity refusals become permanent Uncertain operations. The outgoing lifecycle is a phase plus seven flags and has a race with stale status replies. | ui-only | One MessageRow with a delivery slot and a fixed vocabulary, a transition() model with attempt tokens, service-owned action availability, a copy table for codes, and an unsent header chip. An optional rejection_reason in IPC. Server returns 1060 at the caps. | [`chat-ux-delivery-states`](findings/chat-ux.md#chat-ux-delivery-states), [`chat-code-pending-row-actions`](findings/chat-code.md#chat-code-pending-row-actions), [`chat-code-outgoing-state-machine`](findings/chat-code.md#chat-code-outgoing-state-machine), [`chat-backend-capacity-refusals-become-uncertain`](findings/chat-backend.md#chat-backend-capacity-refusals-become-uncertain) |
| Integrity and availability states | A page-level verification flag is copied onto every row, behind one channel-wide band. One malformed or unsupported message from any writer stops reading and sending in that channel, and some valid shapes abort the whole team sync. 'Channel stopped' tells users to lock and unlock FOKS. A revocation degrades the inbox even on the Rust server. | agent-ipc | Per-message ChatContent::Unverifiable verdicts in the client library plus an agent 'unverifiable' kind (no wire change). A predecessor_unchecked flag per message in IPC. A Recheck channel action. An exact_filtered_inbox capability bit for the revocation case. | [`chat-backend-single-message-poisons-channel`](findings/chat-backend.md#chat-backend-single-message-poisons-channel), [`chat-ux-delivery-states`](findings/chat-ux.md#chat-ux-delivery-states), [`chat-backend-revocation-degrades-inbox`](findings/chat-backend.md#chat-backend-revocation-degrades-inbox) |
| Keyboard navigation | Ctrl/⌘+K reaches channels through the palette. There are no channel or next-unread keys, messages are not focusable, and menus open only on right-click. WKWebView has no ContextMenu key. | ui-only | Alt+↑/↓ and Alt+Shift+↑/↓, ignored while a dialog is open and only when the caret is on the first or last line. Roving tabindex across rows. Shift+F10 opens menus anchored to the row rectangle. A '?' sheet listing only bindings that exist, in platform notation. | [`chat-ux-keyboard-a11y`](findings/chat-ux.md#chat-ux-keyboard-a11y) |
| Screen reader and text size | The message list is a plain div, not a log or live region. The app cannot zoom, and font sizes are in px throughout. | ui-only | A rate-limited polite announcer naming sender and channel only. Text size through native set_zoom with View menu items. rem font-size tokens and prefers-contrast overrides. | [`chat-ux-keyboard-a11y`](findings/chat-ux.md#chat-ux-keyboard-a11y), [`gap-a11y-text-size`](findings/gap.md#gap-a11y-text-size) |
| Permalinks and copy link | None. Store refs are local JSON (profile, alias, team id). No URI scheme is registered, and in-app links would open externally. | ui-only | Native shell work: a portable team-id + channel + sequence identifier resolved against the viewer's own stores, routing in MessageText, a registered URI scheme, and an anchored window load with highlight. | [`chat-ux-message-actions`](findings/chat-ux.md#chat-ux-message-actions), [`chat-ux-channel-list`](findings/chat-ux.md#chat-ux-channel-list) |
| Retention and storage limits | No retained-byte budget. The envelope is stored twice. Clients assume dense sequences. Stage 1 is tracked in ISSUES.md without a mechanism. | protocol-server | Transactional counters in the send transaction and the shared usage ledger. A per-channel history floor exposed through a capability-gated call. Clients treat sequences below the floor as expired, not missing. | [`chat-backend-stage1-redundant-envelope-and-counters`](findings/chat-backend.md#chat-backend-stage1-redundant-envelope-and-counters), [`chat-backend-retention-floor-vs-anchor-ledger`](findings/chat-backend.md#chat-backend-retention-floor-vs-anchor-ledger), [`server-storage-usage-ledger`](findings/server.md#server-storage-usage-ledger) |

## Mockups

Self-contained interactive HTML pages under [`mockups/`](mockups/). Open them in a browser; GitHub shows HTML as source. Each page has state tabs, a light, dark and system theme switch, and a notes panel. The notes say what ships on the data the agent already returns, and what needs agent/IPC, native-shell, client-library or protocol/server work. The pages share a kit under [`mockups/_kit/`](mockups/_kit/README.md) that ports the app's design tokens and shell; `docs/review/index.html` is a browsable version of this report.

| Mockup | Covers | Ships on today's data |
|---|---|---|
| [Open at first unread](mockups/chat-unread-anchored-open.html) | [`chat-ux-open-at-first-unread`](findings/chat-ux.md#chat-ux-open-at-first-unread), [`chat-ux-history-window-sliding`](findings/chat-ux.md#chat-ux-history-window-sliding), [`chat-ux-keyboard-a11y`](findings/chat-ux.md#chat-ux-keyboard-a11y) | All of it ships on current IPC data: before/after cursors, read_through, unread counts and monotonic mark-read already exist. Only renderer logic changes. |
| [Chat channel column](mockups/chat-channel-column.html) | [`chat-ux-channel-list`](findings/chat-ux.md#chat-ux-channel-list), [`chat-ux-local-mute`](findings/chat-ux.md#chat-ux-local-mute), [`chat-ux-composer`](findings/chat-ux.md#chat-ux-composer) | Everything except syncing mute across devices. Mute itself needs only a native-shell settings extension. |
| [Message reading and actions](mockups/chat-reading-and-message-actions.html) | [`chat-ux-message-rendering`](findings/chat-ux.md#chat-ux-message-rendering), [`chat-ux-link-label-spoofing`](findings/chat-ux.md#chat-ux-link-label-spoofing), [`chat-ux-message-actions`](findings/chat-ux.md#chat-ux-message-actions), [`chat-ux-keyboard-a11y`](findings/chat-ux.md#chat-ux-keyboard-a11y) | Everything shown except Copy link and per-message verification glyphs ships on current IPC data. |
| [Chat composer refinements](mockups/chat-composer.html) | [`chat-ux-composer`](findings/chat-ux.md#chat-ux-composer), [`chat-code-send-revision-rerender`](findings/chat-code.md#chat-code-send-revision-rerender) | A-F ship on current IPC. Drafts surviving lock or restart (G) need an agent addition. |
| [Delivery and integrity](mockups/chat-delivery-and-integrity.html) | [`chat-ux-delivery-states`](findings/chat-ux.md#chat-ux-delivery-states), [`chat-code-pending-row-actions`](findings/chat-code.md#chat-code-pending-row-actions), [`chat-code-outgoing-state-machine`](findings/chat-code.md#chat-code-outgoing-state-machine), [`chat-ux-sender-identity`](findings/chat-ux.md#chat-ux-sender-identity), [`chat-backend-single-message-poisons-channel`](findings/chat-backend.md#chat-backend-single-message-poisons-channel), [`chat-backend-capacity-refusals-become-uncertain`](findings/chat-backend.md#chat-backend-capacity-refusals-become-uncertain) | A, B, the page-level variant of C, F and G ship on current IPC. D, E, per-row C and H need agent, client-lib or server work. |
| [Channel readers and alerts](mockups/chat-channel-info-and-alerts.html) | [`chat-ux-channel-audience`](findings/chat-ux.md#chat-ux-channel-audience), [`chat-ux-local-mute`](findings/chat-ux.md#chat-ux-local-mute), [`chat-backend-notification-prefs-mentions-names`](findings/chat-backend.md#chat-backend-notification-prefs-mentions-names), [`desktop-native-linux-notifications-and-badges`](findings/desktop-native.md#desktop-native-linux-notifications-and-badges) | A and B ship on current data. The per-channel alert override already exists but does not affect badges. C-F need native-shell work; G needs the agent. |
| [Chat find and search](mockups/chat-find-and-search.html) | [`chat-ux-history-search`](findings/chat-ux.md#chat-ux-history-search), [`chat-backend-history-search`](findings/chat-backend.md#chat-backend-history-search) | A-C ship on current IPC. The Messages scope (D-G) needs the agent scan. |
| [Extended chat preview](mockups/chat-capability-gated-extensions.html) | [`protocol-chat-pegged-reply-bodies`](findings/protocol.md#protocol-chat-pegged-reply-bodies), [`chat-backend-single-message-poisons-channel`](findings/chat-backend.md#chat-backend-single-message-poisons-channel), [`chat-backend-attachments-via-team-kv`](findings/chat-backend.md#chat-backend-attachments-via-team-kv), [`chat-backend-channel-management-format1`](findings/chat-backend.md#chat-backend-channel-management-format1), [`chat-backend-presence-typing-receipts`](findings/chat-backend.md#chat-backend-presence-typing-receipts), [`chat-backend-capability-negotiation-not-forward-compatible`](findings/chat-backend.md#chat-backend-capability-negotiation-not-forward-compatible), [`protocol-chat-capabilities-extensible-shape`](findings/protocol.md#protocol-chat-capabilities-extensible-shape) | None of these can be sent today. Read-side reply rendering needs only a client-library change. Attachments can use the existing team KV once the agent workflow and owner provisioning exist. |
| [Login copy and generator](mockups/vault-login-copy-and-generator.html) | [`fe-product-login-fields-copy`](findings/fe-product.md#fe-product-login-fields-copy), [`fe-product-password-generator`](findings/fe-product.md#fe-product-password-generator), [`fe-product-search-quick-actions`](findings/fe-product.md#fe-product-search-quick-actions), [`gap-toast-timing-announcements`](findings/gap.md#gap-toast-timing-announcements) | The generator, strength meter, palette navigation and masked reveal are UI-only. Field-level copy needs the host command. Shape-based classification needs the client-library and agent catalog field. |
| [Trash and undo](mockups/vault-trash-and-undo.html) | [`fe-product-trash-undo`](findings/fe-product.md#fe-product-trash-undo), [`fe-product-organize-bulk`](findings/fe-product.md#fe-product-organize-bulk) | Builds on existing agent operations and the toast action button. Undo needs a small mutation-response extension or a catalog reload. |
| [Auto-lock and clipboard](mockups/security-auto-lock.html) | [`desktop-native-auto-lock`](findings/desktop-native.md#desktop-native-auto-lock), [`fe-product-auto-lock`](findings/fe-product.md#fe-product-auto-lock) | Manual Lock now (Settings › Storage and the account menu), the lock screen and the fixed clipboard auto-clear already exist. Automatic triggers and policy need native-host work. |
| [Recovery phrase test](mockups/devices-recovery-phrase-test.html) | [`gap-recovery-phrase-test`](findings/gap.md#gap-recovery-phrase-test) | The device list, recovery phrase rows with Revoke, the first-run phrase reveal and BackupEnrollmentSummary.backup_id_hex already exist. The check itself needs one small, local-only agent operation. |

## Findings by area

### [Chat product and UX gap analysis](findings/chat-ux.md)

15 findings: 2 high, 12 medium, 1 low.

- [`chat-ux-channel-list`](findings/chat-ux.md#chat-ux-channel-list) (high, M, feature-refinement): Channel list: stable ordering, previews and times, draft and unread emphasis, persisted folds, honest context menu
- [`chat-ux-open-at-first-unread`](findings/chat-ux.md#chat-ux-open-at-first-unread) (high, M, feature-refinement): Open channels at the first unread message, mark read by what was actually seen, and count arrivals while scrolled up
- [`chat-ux-channel-audience`](findings/chat-ux.md#chat-ux-channel-audience) (medium, S, correctness-risk): Admins-only channels overstate their audience; show the real readers and the roles in channel info
- [`chat-ux-composer`](findings/chat-ux.md#chat-ux-composer) (medium, M, feature-refinement): Composer refinements: formatting affordances, mention autocomplete, draft indicators, Up-to-edit-unsent, character feedback
- [`chat-ux-delivery-states`](findings/chat-ux.md#chat-ux-delivery-states) (medium, M, feature-refinement): One legible delivery and integrity vocabulary: unify unsent-message rows, map rejection codes, and mark verification per message
- [`chat-ux-history-search`](findings/chat-ux.md#chat-ux-history-search) (medium, L, missing-feature): Message search: start with find-in-conversation over decrypted history, then an agent-side encrypted index
- [`chat-ux-history-window-sliding`](findings/chat-ux.md#chat-ux-history-window-sliding) (medium, M, correctness-risk): Replace the hard 1,000-row view cap with a sliding window, and auto-load older pages
- [`chat-ux-keyboard-a11y`](findings/chat-ux.md#chat-ux-keyboard-a11y) (medium, M, feature-refinement): Keyboard navigation and screen-reader support for the conversation
- [`chat-ux-link-label-spoofing`](findings/chat-ux.md#chat-ux-link-label-spoofing) (medium, S, security): Markdown link labels can disguise the destination; show the host and confirm mismatches
- [`chat-ux-local-mute`](findings/chat-ux.md#chat-ux-local-mute) (medium, M, missing-feature): Make mute real: a device-local per-channel and per-team mute that drives badges, plus a 'Mentions only' alert mode
- [`chat-ux-message-actions`](findings/chat-ux.md#chat-ux-message-actions) (medium, M, missing-feature): Add a hover/focus action bar per message: copy, quote-reply, copy link, details (no protocol change)
- [`chat-ux-message-rendering`](findings/chat-ux.md#chat-ux-message-rendering) (medium, S, feature-refinement): Message rendering: autolink bare URLs, keep paragraph breaks, style code blocks, highlight @mentions
- [`chat-ux-sender-identity`](findings/chat-ux.md#chat-ux-sender-identity) (medium, M, feature-refinement): Resolve every sender to a verified username, including departed members, and mark former members
- [`chat-ux-thread-decomposition`](findings/chat-ux.md#chat-ux-thread-decomposition) (medium, M, maintainability): Split ChatThread into header, list, row and composer, and derive display rows as a pure model
- [`chat-ux-mock-fidelity`](findings/chat-ux.md#chat-ux-mock-fidelity) (low, S, tooling): Give the chat mock realistic scenarios so design review and screenshots reflect production

### [Chat frontend code quality](findings/chat-code.md)

14 findings: 1 high, 8 medium, 5 low.

- [`chat-code-outgoing-state-machine`](findings/chat-code.md#chat-code-outgoing-state-machine) (high, M, correctness-risk): Make the outgoing-message lifecycle an explicit state machine instead of `phase` plus seven flags
- [`chat-code-conversation-hook-api`](findings/chat-code.md#chat-code-conversation-hook-api) (medium, S, code-quality): Delete dead branches in useChatConversation; give useChatHistory an options object and stable callbacks
- [`chat-code-inbox-sibling-sync-cancel`](findings/chat-code.md#chat-code-inbox-sibling-sync-cancel) (medium, S, correctness-risk): Inbox job disposal cancels sibling teams' syncs and drops their pending dirty flag
- [`chat-code-ipc-contract-parity`](findings/chat-code.md#chat-code-ipc-contract-parity) (medium, M, correctness-risk): Chat IPC contract is hand-maintained in five TS tables, and the mutation classification already disagrees with Rust
- [`chat-code-mock-bridge-decoder`](findings/chat-code.md#chat-code-mock-bridge-decoder) (medium, S, testing): Route mock chat replies through decodeChatReply so render tests exercise the contract
- [`chat-code-scheduling-primitives`](findings/chat-code.md#chat-code-scheduling-primitives) (medium, M, performance): Replace the inbox's fixed 250 ms drain loop and three hand-written timers with a shared deadline timer and clock module
- [`chat-code-send-revision-rerender`](findings/chat-code.md#chat-code-send-revision-rerender) (medium, M, performance): Every draft keystroke re-renders the whole chat pane through the send service's global revision
- [`chat-code-send-service-decomposition`](findings/chat-code.md#chat-code-send-service-decomposition) (medium, L, maintainability): Split ChatSendService (1509 lines) into draft store, outgoing queue, request gateway, recovery and scheduler modules
- [`chat-code-vestigial-modules`](findings/chat-code.md#chat-code-vestigial-modules) (medium, S, maintainability): Remove or reconnect modules reached only from tests: recover-pending, recovery-schedule, conversation reducer branches, use-message-composer
- [`chat-code-backoff-formulas`](findings/chat-code.md#chat-code-backoff-formulas) (low, S, maintainability): Unify five retry-backoff formulas that already disagree on the first delay
- [`chat-code-local-access-errors`](findings/chat-code.md#chat-code-local-access-errors) (low, M, correctness-risk): Unify the three local access guards; stop channel creation from throwing the agent's chat-access-denied code
- [`chat-code-notification-limit-constant`](findings/chat-code.md#chat-code-notification-limit-constant) (low, S, code-quality): Move the 256-character notification text bound into chat-limits.json
- [`chat-code-pending-row-actions`](findings/chat-code.md#chat-code-pending-row-actions) (low, M, maintainability): Move protocol action selection out of PendingRow and OutgoingRow into the send service
- [`chat-code-render-purity`](findings/chat-code.md#chat-code-render-purity) (low, S, tooling): Lint does not catch render-time service mutations and ref writes in chat hooks

### [Chat protocol and backend stack](findings/chat-backend.md)

13 findings: 1 high, 8 medium, 4 low.

- [`chat-backend-single-message-poisons-channel`](findings/chat-backend.md#chat-backend-single-message-poisons-channel) (high, M, security): One malformed message from any channel writer stops reading and sending in that channel
- [`chat-backend-attachments-via-team-kv`](findings/chat-backend.md#chat-backend-attachments-via-team-kv) (medium, L, missing-feature): Attachments stored in the team KV under channel-scoped roles, referenced from Basic text
- [`chat-backend-capacity-refusals-become-uncertain`](findings/chat-backend.md#chat-backend-capacity-refusals-become-uncertain) (medium, S, correctness-risk): Send-side capacity refusals are reported as generic 12001, leaving operations permanently Uncertain
- [`chat-backend-channel-management-format1`](findings/chat-backend.md#chat-backend-channel-management-format1) (medium, L, missing-feature): Channel rename, description, posting role and archive via a Rust-only update method that keeps Go wire metadata
- [`chat-backend-notification-prefs-mentions-names`](findings/chat-backend.md#chat-backend-notification-prefs-mentions-names) (medium, M, missing-feature): Agent-owned per-channel alert/hide preferences synced via personal KV, local mentions and sender labels
- [`chat-backend-retention-floor-vs-anchor-ledger`](findings/chat-backend.md#chat-backend-retention-floor-vs-anchor-ledger) (medium, M, correctness-risk): Define retention as an explicit history floor; the client's gap, predecessor and recovery logic assume dense sequences
- [`chat-backend-revocation-degrades-inbox`](findings/chat-backend.md#chat-backend-revocation-degrades-inbox) (medium, S, correctness-risk): The FOKS-RS server itself puts an inbox into the degraded state on every access revocation
- [`chat-backend-stage1-redundant-envelope-and-counters`](findings/chat-backend.md#chat-backend-stage1-redundant-envelope-and-counters) (medium, M, performance): Stage 1 quota: drop the redundant envelope column and keep retained-byte counters in the send transaction
- [`chat-backend-stage2-submission-floor`](findings/chat-backend.md#chat-backend-stage2-submission-floor) (medium, M, maintainability): Stage 2 compaction: time-ordered submission IDs plus a durable floor make expiry safe
- [`chat-backend-capability-negotiation-not-forward-compatible`](findings/chat-backend.md#chat-backend-capability-negotiation-not-forward-compatible) (low, S, correctness-risk): Chat capability discovery fails closed on version skew, forces format 2 for every feature, and is unused by the agent
- [`chat-backend-get-channel-read-path`](findings/chat-backend.md#chat-backend-get-channel-read-path) (low, M, performance): Every chat read lists and decrypts all channels; implement upstream rtGetChannel and reuse reads for mark-read
- [`chat-backend-history-search`](findings/chat-backend.md#chat-backend-history-search) (low, M, missing-feature): Bounded verified history search in the agent, without server or plaintext persistence
- [`chat-backend-presence-typing-receipts`](findings/chat-backend.md#chat-backend-presence-typing-receipts) (low, L, missing-feature): Typing indicators and other members' read positions as capability-gated ephemeral extensions

### [Desktop frontend architecture and code quality (non-chat)](findings/fe-arch.md)

14 findings: 1 high, 11 medium, 2 low.

- [`fe-arch-first-run-session`](findings/fe-arch.md#fe-arch-first-run-session) (high, L, maintainability): Split FirstRunSession into a setup-session context, a typed step registry and per-step components
- [`fe-arch-copy-action`](findings/fe-arch.md#fe-arch-copy-action) (medium, S, correctness-risk): Use one clipboard-copy action; four first-run copy buttons silently drop failures
- [`fe-arch-css-structure`](findings/fe-arch.md#fe-arch-css-structure) (medium, M, maintainability): Split app.css by feature, bring team-info.css into the cascade layers, and auto-discover stylesheets in the style test
- [`fe-arch-details-panel-plaintext-reducer`](findings/fe-arch.md#fe-arch-details-panel-plaintext-reducer) (medium, M, correctness-risk): Make DetailsPanel's plaintext lifecycle one reducer instead of four hand-maintained reset paths
- [`fe-arch-device-storage`](findings/fe-arch.md#fe-arch-device-storage) (medium, M, correctness-risk): Register webview localStorage keys in one module and purge profile-scoped entries when the device is reset
- [`fe-arch-feature-folders`](findings/fe-arch.md#fe-arch-feature-folders) (medium, L, maintainability): Move the flat src/ root and mixed screens/ into feature folders with enforced layering
- [`fe-arch-files-list-a11y`](findings/fe-arch.md#fe-arch-files-list-a11y) (medium, M, feature-refinement): Give the Files list and folder tree list/tree semantics and a single tab stop
- [`fe-arch-mock-bridge-contract`](findings/fe-arch.md#fe-arch-mock-bridge-contract) (medium, L, testing): Run the mock bridge through the production transport and decoders
- [`fe-arch-pane-resize-unify`](findings/fe-arch.md#fe-arch-pane-resize-unify) (medium, S, code-quality): Replace the shell's private sidebar-resize hook with the shared pane-resize hook
- [`fe-arch-render-test-harness`](findings/fe-arch.md#fe-arch-render-test-harness) (medium, M, testing): Replace per-file Vite servers in render tests with one typed harness and test folders that mirror the features
- [`fe-arch-screen-split-shared-primitives`](findings/fe-arch.md#fe-arch-screen-split-shared-primitives) (medium, L, maintainability): Split the 1,100-1,900-line screen files into model, sheets and view, and deduplicate their local primitives
- [`fe-arch-shell-services-context`](findings/fe-arch.md#fe-arch-shell-services-context) (medium, M, maintainability): Provide shell services through context instead of passing bridge, snapshot and error handlers through every screen
- [`fe-arch-code-splitting`](findings/fe-arch.md#fe-arch-code-splitting) (low, M, performance): Lazy-load first-run, the chat views and the settings screens to shrink the 1.06 MB startup chunk
- [`fe-arch-mutation-action-hook`](findings/fe-arch.md#fe-arch-mutation-action-hook) (low, M, code-quality): Standardize write actions on command-policy with a shared hook instead of 33 hand-rolled busy flags

### [Desktop product features outside chat](findings/fe-product.md)

14 findings: 3 high, 9 medium, 2 low.

- [`fe-product-auto-lock`](findings/fe-product.md#fe-product-auto-lock) (high, M, security): No inactivity, sleep or screen-lock auto-lock; add a Security settings section
- [`fe-product-login-fields-copy`](findings/fe-product.md#fe-product-login-fields-copy) (high, M, correctness-risk): Production login classification and Copy act on the whole "user:/password:/url:" text
- [`fe-product-trash-undo`](findings/fe-product.md#fe-product-trash-undo) (high, M, missing-feature): Permanent delete with no undo, and no folder delete; add a role-aware Trash built on MoveKv
- [`fe-product-exposure-review`](findings/fe-product.md#fe-product-exposure-review) (medium, M, missing-feature): Exposure review and rotation checklist after removing a member or device
- [`fe-product-import-export`](findings/fe-product.md#fe-product-import-export) (medium, L, missing-feature): Starter card advertises a nonexistent `foks kv export`; add host-side CSV import and export
- [`fe-product-keyboard-a11y`](findings/fe-product.md#fe-product-keyboard-a11y) (medium, M, feature-refinement): Files keyboard model and ARIA semantics, plus a discoverable shortcut sheet
- [`fe-product-organize-bulk`](findings/fe-product.md#fe-product-organize-bulk) (medium, L, feature-refinement): Organising: folder-picker move, drag-to-folder, and multi-select bulk actions
- [`fe-product-password-generator`](findings/fe-product.md#fe-product-password-generator) (medium, S, missing-feature): Password generator, reveal toggle and strength feedback in New and Edit
- [`fe-product-search-quick-actions`](findings/fe-product.md#fe-product-search-quick-actions) (medium, M, feature-refinement): ⌘K palette: quick copy actions, recents and favourites, folders and commands
- [`fe-product-share-to-team`](findings/fe-product.md#fe-product-share-to-team) (medium, M, missing-feature): Share or copy an item into a team vault
- [`fe-product-signed-activity`](findings/fe-product.md#fe-product-signed-activity) (medium, L, missing-feature): Show signed device and team history from the verified sigchains
- [`fe-product-version-history`](findings/fe-product.md#fe-product-version-history) (medium, L, missing-feature): Item version history and restore from versions this device has seen (no protocol change)
- [`fe-product-attention-centre`](findings/fe-product.md#fe-product-attention-centre) (low, M, feature-refinement): Single attention centre with a post-setup checklist and recent-activity history
- [`fe-product-unused-vault-ipc`](findings/fe-product.md#fe-product-unused-vault-ipc) (low, S, code-quality): Remove or use the unused `copy_item_path` and `create_link` IPC commands

### [Tauri native shell and desktop crate](findings/desktop-native.md)

15 findings: 1 high, 9 medium, 5 low.

- [`desktop-native-auto-lock`](findings/desktop-native.md#desktop-native-auto-lock) (high, M, missing-feature): App lock is manual-only; add a native inactivity, sleep and screen-lock trigger
- [`desktop-native-app-acl-for-commands`](findings/desktop-native.md#desktop-native-app-acl-for-commands) (medium, M, security): Put the 127 custom commands behind Tauri's app ACL now that a remote-origin window exists
- [`desktop-native-context-split`](findings/desktop-native.md#desktop-native-context-split) (medium, M, maintainability): Split commands/context.rs and give store identities a typed key
- [`desktop-native-error-code-registry`](findings/desktop-native.md#desktop-native-error-code-registry) (medium, M, code-quality): Replace free-form error-code strings with a registry shared by Rust and TypeScript
- [`desktop-native-linux-notifications-and-badges`](findings/desktop-native.md#desktop-native-linux-notifications-and-badges) (medium, M, missing-feature): Linux chat notifications via org.freedesktop.Notifications, plus channel grouping and an unread badge
- [`desktop-native-operation-builders-triplicated`](findings/desktop-native.md#desktop-native-operation-builders-triplicated) (medium, L, maintainability): Agent Operations are built in three places; DesktopModel is dead and the real-agent CI test bypasses the shipping code
- [`desktop-native-release-matrix`](findings/desktop-native.md#desktop-native-release-matrix) (medium, M, missing-feature): Widen the release matrix (universal macOS, rpm, aarch64 Linux) and record Windows as an explicit non-goal
- [`desktop-native-startup-reset-safety`](findings/desktop-native.md#desktop-native-startup-reset-safety) (medium, M, correctness-risk): A startup timeout can lead to a destructive state reset; add retry, failure classification and quarantine
- [`desktop-native-typed-agent-responses`](findings/desktop-native.md#desktop-native-typed-agent-responses) (medium, L, maintainability): Stop mirroring agent response types in the desktop; decode into foks-agent-proto types
- [`desktop-native-updater`](findings/desktop-native.md#desktop-native-updater) (medium, L, missing-feature): No update channel: add signed in-app updates for macOS and an update notice for the .deb
- [`desktop-native-agent-log-and-crash-markers`](findings/desktop-native.md#desktop-native-agent-log-and-crash-markers) (low, S, code-quality): agent.log grows without bound and crash markers are never read
- [`desktop-native-command-contract-check`](findings/desktop-native.md#desktop-native-command-contract-check) (low, S, testing): Check command names and argument keys between generate_handler! and the TS bridge
- [`desktop-native-doc-drift`](findings/desktop-native.md#desktop-native-doc-drift) (low, S, docs): Correct specific stale statements in the desktop's security and architecture docs
- [`desktop-native-linux-parity`](findings/desktop-native.md#desktop-native-linux-parity) (low, S, feature-refinement): Small Linux parity gaps: device and user name defaults, clipboard-history hint, link opening
- [`desktop-native-quit-confirmation`](findings/desktop-native.md#desktop-native-quit-confirmation) (low, S, feature-refinement): Quit and window close always show a 'stop agent' modal, even when nothing is pending

### [Local agent, IPC protocol, CLI and MCP](findings/agent.md)

15 findings: 1 high, 12 medium, 2 low.

- [`agent-ipc-version-handshake`](findings/agent.md#agent-ipc-version-handshake) (high, M, maintainability): Replace exact-equality IPC versioning with a frozen hello/capability handshake
- [`agent-bot-contract-typed`](findings/agent.md#agent-bot-contract-typed) (medium, M, maintainability): Define bot replies as typed proto structs, generate the TypeScript decoder, and report loaded state
- [`agent-capacity-drop-false-ambiguity`](findings/agent.md#agent-capacity-drop-false-ambiguity) (medium, S, correctness-risk): Answer connection-capacity rejections with a definite Busy instead of silently closing the socket
- [`agent-chat-subscription-stream`](findings/agent.md#agent-chat-subscription-stream) (medium, L, missing-feature): Move chat inbox polling into the agent and expose a subscription stream to all front ends
- [`agent-cli-chat-and-parity`](findings/agent.md#agent-cli-chat-and-parity) (medium, M, missing-feature): Add CLI chat commands and close remaining CLI parity gaps (team KV, mv/stat, pending work, agent status, completions)
- [`agent-cli-direct-state-access`](findings/agent.md#agent-cli-direct-state-access) (medium, L, maintainability): Route the remaining direct-access CLI commands through the agent
- [`agent-data-catalog-unbounded`](findings/agent.md#agent-data-catalog-unbounded) (medium, M, correctness-risk): Page DataRead::Catalog, add single-path lookup for MCP, and turn oversize responses into a typed error
- [`agent-error-disposition`](findings/agent.md#agent-error-disposition) (medium, M, correctness-risk): Define error outcome/retry/category in foks-agent-proto and use it for CLI exit codes and MCP tool errors
- [`agent-main-decomposition-lib`](findings/agent.md#agent-main-decomposition-lib) (medium, L, maintainability): Split foks-agent into a library with domain dispatch modules and an explicit AgentContext
- [`agent-mcp-chat-tools`](findings/agent.md#agent-mcp-chat-tools) (medium, M, missing-feature): Expose a guarded MCP chat tool set on the existing chat operations
- [`agent-mcp-consent-audit`](findings/agent.md#agent-mcp-consent-audit) (medium, L, security): Add agent-enforced assistant grants, an audit log, revocation and desktop-lock propagation for MCP sessions
- [`agent-observability-logs-diagnostics`](findings/agent.md#agent-observability-logs-diagnostics) (medium, M, missing-feature): Give the agent its own bounded, timestamped log, counters and a Diagnostics operation
- [`agent-request-deadline`](findings/agent.md#agent-request-deadline) (medium, S, correctness-risk): Carry the client's deadline in each request instead of an agent-wide timeout set by whichever front end launched the agent
- [`agent-mcp-tool-metadata`](findings/agent.md#agent-mcp-tool-metadata) (low, S, feature-refinement): Write real MCP tool descriptions and correct the tool annotations
- [`agent-operation-policy-table`](findings/agent.md#agent-operation-policy-table) (low, S, testing): Snapshot every operation's policy row so the eight classifiers cannot drift apart

### [Client libraries](findings/client-libs.md)

12 findings: 2 high, 7 medium, 3 low.

- [`client-libs-error-classification`](findings/client-libs.md#client-libs-error-classification) (high, M, code-quality): Replace catch-all string errors with an exhaustive error class so the agent stops classifying by downcast
- [`client-libs-operation-scoped-context`](findings/client-libs.md#client-libs-operation-scoped-context) (high, L, maintainability): Give CheckedProfileSession an operation-scoped context instead of per-call DB opens, thread-local session material and raw master-key parameters
- [`client-libs-acting-credential`](findings/client-libs.md#client-libs-acting-credential) (medium, L, code-quality): Collapse the _yubi / _with_credential / _as_local_team method matrix behind one acting-credential type
- [`client-libs-checked-session-result-loss`](findings/client-libs.md#client-libs-checked-session-result-loss) (medium, S, correctness-risk): Checked-session wrapper reports a committed operation as failed when checkpoint publication fails
- [`client-libs-hard-state-migration-ladder`](findings/client-libs.md#client-libs-hard-state-migration-ladder) (medium, M, maintainability): Replace ad-hoc hard-state version branches with a stepwise migration ladder and frozen historical fixtures
- [`client-libs-journal-state-encoding`](findings/client-libs.md#client-libs-journal-state-encoding) (medium, S, correctness-risk): Define journal-state SQL encodings and terminal sets once instead of repeating integer literals
- [`client-libs-secret-record-redaction`](findings/client-libs.md#client-libs-secret-record-redaction) (medium, S, security): Vault records with raw seeds derive Debug and rely on hand-written Drop zeroization
- [`client-libs-soft-stage-reclaim`](findings/client-libs.md#client-libs-soft-stage-reclaim) (medium, S, correctness-risk): Wire orphaned large-file stage reclamation into exclusive session entry
- [`client-libs-team-intent-layering`](findings/client-libs.md#client-libs-team-intent-layering) (medium, XL, maintainability): Move team rekey/member-edit/expulsion intent state machines out of client-app runtime into foks-client
- [`client-libs-agent-dto-duplication`](findings/client-libs.md#client-libs-agent-dto-duplication) (low, M, maintainability): Stop duplicating foks-agent-proto DTOs as Serialize-only report types in client-app
- [`client-libs-kv-scope-api`](findings/client-libs.md#client-libs-kv-scope-api) (low, M, code-quality): Unify personal/team KV methods behind a KvScope and a shared write-session prologue
- [`client-libs-oidc-deadline`](findings/client-libs.md#client-libs-oidc-deadline) (low, S, correctness-risk): OIDC provider discovery ignores the FoksClient deadline and cancellation token

### [Server, storage and operability](findings/server.md)

15 findings: 5 high, 9 medium, 1 low.

- [`server-fatal-thread-exit`](findings/server.md#server-fatal-thread-exit) (high, M, correctness-risk): A transient accept() error or a writer panic leaves a live process with no service, and /healthz still returns ok
- [`server-merkle-epoch-budget`](findings/server.md#server-merkle-epoch-budget) (high, M, security): One authenticated account can permanently exhaust the host's 65,535 Merkle epochs (and the 4,096-team cap)
- [`server-source-admission-fairness`](findings/server.md#server-source-admission-fairness) (high, M, security): No per-source concurrency caps: one IP can hold every public connection slot, and one account can take every realtime poll slot
- [`server-storage-usage-ledger`](findings/server.md#server-storage-usage-ledger) (high, L, performance): Replace per-write KV namespace scans with a transactional usage ledger and add a storage reserve for identity writes
- [`server-waitlist-unbounded`](findings/server.md#server-waitlist-unbounded) (high, S, security): Unauthenticated joinWaitList inserts unbounded, permanent, unreadable PII rows
- [`server-backup-snapshot-restart`](findings/server.md#server-backup-snapshot-restart) (medium, S, correctness-risk): Online backup steps without a held read snapshot, so continuous writes can restart a large backup indefinitely
- [`server-config-surface`](findings/server.md#server-config-surface) (medium, S, maintainability): server.toml cannot set any limit, ignores unknown keys, and gives misleading validation errors
- [`server-error-status-mapping`](findings/server.md#server-error-status-mapping) (medium, M, correctness-risk): About 13 divergent map_write_error functions: a full database is reported as retryable, and internal error text leaks to clients
- [`server-merkle-incremental-prepare`](findings/server.md#server-merkle-incremental-prepare) (medium, L, performance): Full Merkle rebuild on the single writer thread grows to seconds per identity/team mutation
- [`server-metrics-scrape-cost`](findings/server.md#server-metrics-scrape-cost) (medium, S, performance): Every /metrics scrape and every new pooled reader full-scans the KV tables
- [`server-operator-capacity-console`](findings/server.md#server-operator-capacity-console) (medium, M, missing-feature): Operators have no capacity, headroom or abuse controls (usage report, account suspension, waitlist)
- [`server-request-failure-observability`](findings/server.md#server-request-failure-observability) (medium, M, missing-feature): Request failures are invisible: no per-route/status metrics, no internal-error reasons, no latency histograms
- [`server-session-decomposition`](findings/server.md#server-session-decomposition) (medium, M, maintainability): net/session.rs mixes transport and domain logic, and request authorization relies on cloning ServerData by convention
- [`server-writer-deadlines`](findings/server.md#server-writer-deadlines) (medium, M, performance): Writer queue runs abandoned jobs and has no priority lanes or per-principal fairness
- [`server-migration-framework`](findings/server.md#server-migration-framework) (low, M, testing): Schema upgrades are one ad-hoc branch block tested only against fixtures built by reversing the current schema

### [Protocol, crypto, RPC and verification](findings/protocol.md)

13 findings: 0 high, 8 medium, 5 low.

- [`protocol-box-randomness-secret-api`](findings/protocol.md#protocol-box-randomness-secret-api) (medium, M, security): Box randomness structs expose secret material as plain Copy arrays and leave key/nonce uniqueness to the caller
- [`protocol-chat-pegged-reply-bodies`](findings/protocol.md#protocol-chat-pegged-reply-bodies) (medium, M, feature-refinement): Open the pinned Go Pegged bodies (Reply first) and stop blocking sends after an unsupported message
- [`protocol-crypto-dependency-hygiene`](findings/protocol.md#protocol-crypto-dependency-hygiene) (medium, S, security): Collapse duplicate crypto crate versions and add RustSec advisory checks to CI
- [`protocol-fuzz-schema-and-verification-decoders`](findings/protocol.md#protocol-fuzz-schema-and-verification-decoders) (medium, M, testing): Fuzz the schema decoders, server argument decoders and verifiers, and seed the corpora with the Go fixtures
- [`protocol-hybrid-seal-duplication`](findings/protocol.md#protocol-hybrid-seal-duplication) (medium, M, code-quality): hybrid.rs repeats the hybrid KDF and typed-nonce secretbox nine times, with inconsistent input validation
- [`protocol-seed-chain-plaintext-nonzeroizing-decode`](findings/protocol.md#protocol-seed-chain-plaintext-nonzeroizing-decode) (medium, S, security): Seed-chain and KV plaintexts go through the non-zeroizing decode_prefix, contrary to the book's zeroization rule
- [`protocol-typed-status-codes-and-detail`](findings/protocol.md#protocol-typed-status-codes-and-detail) (medium, S, maintainability): Replace magic status-code literals with typed statuses, and limit server-supplied status text
- [`protocol-verify-shared-chain-step`](findings/protocol.md#protocol-verify-shared-chain-step) (medium, L, maintainability): Share one per-link step between the full and incremental user/team verifiers, and test that they agree
- [`protocol-chat-capabilities-extensible-shape`](findings/protocol.md#protocol-chat-capabilities-extensible-shape) (low, S, feature-refinement): Make the chat capability response extensible before any extended-chat feature is advertised
- [`protocol-crypto-public-api-docs-and-ct-eq`](findings/protocol.md#protocol-crypto-public-api-docs-and-ct-eq) (low, S, docs): Document the foks-crypto root API, type its raw-seed signers, and use constant-time equality on secret types
- [`protocol-crypto-test-organization`](findings/protocol.md#protocol-crypto-test-organization) (low, M, testing): Split the 2853-line foks-crypto tests.rs and consolidate the Yubi test doubles and fixture loaders
- [`protocol-parcel-open-expectation-struct`](findings/protocol.md#protocol-parcel-open-expectation-struct) (low, M, code-quality): Replace the six-function parcel-opening ladder with a named-field expectation struct
- [`protocol-type-id-registry-and-drift`](findings/protocol.md#protocol-type-id-registry-and-drift) (low, M, tooling): Hand-written 64-bit type IDs have no upstream drift check and no collision registry

### [Build, CI, tooling, tests and docs](findings/build.md)

14 findings: 1 high, 9 medium, 4 low.

- [`build-deps-supply-chain-audit`](findings/build.md#build-deps-supply-chain-audit) (high, M, security): No dependency advisory, license or source auditing for 749 crates and 280 npm packages
- [`build-ci-desktop-cost`](findings/build.md#build-ci-desktop-cost) (medium, M, performance): Desktop PR validation rebuilds the full workspace on macOS in release mode, uncached, untimed and uncancelled
- [`build-ci-pr-required-gate`](findings/build.md#build-ci-pr-required-gate) (medium, M, tooling): No single PR-triggered required check; clippy, fmt and protocol-metadata guards for 23 crates run only on push
- [`build-contributor-security-onboarding`](findings/build.md#build-contributor-security-onboarding) (medium, S, docs): No SECURITY.md, CONTRIBUTING.md or prerequisite check; onboarding knowledge lives in an agent-oriented AGENTS.md
- [`build-crate-layer-boundaries`](findings/build.md#build-crate-layer-boundaries) (medium, M, maintainability): Replace vacuous monorepo path-boundary checks with an enforced crate-layer allowlist; document the desktop's direct client-app use
- [`build-docs-guide-crypto-misstatements`](findings/build.md#build-docs-guide-crypto-misstatements) (medium, S, docs): docs/GUIDE.html and docs/INTRO.html still misdescribe the KDF, the hybrid combiner and recovery-kit stretching
- [`build-frontend-format-lint-gate`](findings/build.md#build-frontend-format-lint-gate) (medium, S, tooling): Prettier is never checked in CI, and ESLint and tsc cover only apps/desktop
- [`build-release-standalone-binaries`](findings/build.md#build-release-standalone-binaries) (medium, L, missing-feature): No release pipeline for foks-server, foks-agent and foks-rs CLI despite shipped systemd units
- [`build-test-agent-binary-resolution`](findings/build.md#build-test-agent-binary-resolution) (medium, M, testing): CLI integration tests resolve foks-agent by sibling path, which nothing keeps current; six copies of the agent harness
- [`build-test-flock-release-race`](findings/build.md#build-test-flock-release-race) (medium, M, correctness-risk): Root cause of the first-run receipt-lock flake: flock locks outlive drop while sibling tests spawn processes
- [`build-actions-hardening`](findings/build.md#build-actions-hardening) (low, S, security): Inconsistent action pinning, persisted checkout credentials in secret-bearing jobs, no workflow linting
- [`build-benchmark-provenance`](findings/build.md#build-benchmark-provenance) (low, S, docs): README and book benchmark numbers have no committed result file or revision
- [`build-docs-link-and-drift-check`](findings/build.md#build-docs-link-and-drift-check) (low, M, docs): Broken design-doc references and stale READMEs; no automated link or path check
- [`build-fuzz-corpus-persistence`](findings/build.md#build-fuzz-corpus-persistence) (low, S, testing): Weekly fuzz campaigns discard their corpus and run on a floating nightly

### [Cross-cutting gaps (completeness critic)](findings/gap.md)

11 findings: 2 high, 6 medium, 3 low.

- [`gap-mcp-adapter-clock-suspend`](findings/gap.md#gap-mcp-adapter-clock-suspend) (high, M, correctness-risk): MCP write admission treats every laptop sleep over five minutes as an untrusted clock, and a day off needs a CLI-only repair
- [`gap-recovery-phrase-test`](findings/gap.md#gap-recovery-phrase-test) (high, M, missing-feature): Recovery phrases are never checked after they are written down; add a local 'Test recovery phrase' and a recall step
- [`gap-a11y-text-size`](findings/gap.md#gap-a11y-text-size) (medium, M, feature-refinement): Text cannot be enlarged: no zoom, no text-size setting, and pixel font sizes throughout
- [`gap-fork-consistency-compare`](findings/gap.md#gap-fork-consistency-compare) (medium, M, missing-feature): Let users compare signed checkpoints, the out-of-band root comparison book chapter 13 recommends
- [`gap-offline-read-only-vault`](findings/gap.md#gap-offline-read-only-vault) (medium, L, missing-feature): No read access to vault items while the server is unreachable; add an explicit read-only offline mode with sealed key material
- [`gap-renderer-fault-capture`](findings/gap.md#gap-renderer-fault-capture) (medium, S, correctness-risk): A render error outside the routed screen blanks the whole window and leaves no trace in Copy diagnostics
- [`gap-security-key-pin-field`](findings/gap.md#gap-security-key-pin-field) (medium, S, code-quality): Eleven hand-rolled security-key PIN fields disagree with the 6-8 character rule and never show remaining attempts
- [`gap-sync-persists-unused-plaintext`](findings/gap.md#gap-sync-persists-unused-plaintext) (medium, S, security): SyncAccount and SyncTeam download every file and store its plaintext in soft state, but production code reads it back only for file sizes
- [`gap-clock-skew-diagnosis`](findings/gap.md#gap-clock-skew-diagnosis) (low, S, correctness-risk): A wrong system clock surfaces as an opaque check-in or compatibility failure
- [`gap-storage-usage-display`](findings/gap.md#gap-storage-usage-display) (low, S, missing-feature): Show storage used per vault; the agent already returns usage, but the desktop learns of limits only from a quota refusal
- [`gap-toast-timing-announcements`](findings/gap.md#gap-toast-timing-announcements) (low, S, feature-refinement): Toasts vanish after 2.6 s with no pause, carry the only explanation for refused actions, and are announced unreliably

## Overlapping findings

These groups describe the same underlying change from different areas. Implement them together; the first id is the canonical one.

- [`desktop-native-auto-lock`](findings/desktop-native.md#desktop-native-auto-lock), [`fe-product-auto-lock`](findings/fe-product.md#fe-product-auto-lock)
- [`chat-ux-local-mute`](findings/chat-ux.md#chat-ux-local-mute), [`chat-backend-notification-prefs-mentions-names`](findings/chat-backend.md#chat-backend-notification-prefs-mentions-names)
- [`chat-ux-history-search`](findings/chat-ux.md#chat-ux-history-search), [`chat-backend-history-search`](findings/chat-backend.md#chat-backend-history-search)
- [`chat-backend-capability-negotiation-not-forward-compatible`](findings/chat-backend.md#chat-backend-capability-negotiation-not-forward-compatible), [`protocol-chat-capabilities-extensible-shape`](findings/protocol.md#protocol-chat-capabilities-extensible-shape)
- [`chat-backend-single-message-poisons-channel`](findings/chat-backend.md#chat-backend-single-message-poisons-channel), [`protocol-chat-pegged-reply-bodies`](findings/protocol.md#protocol-chat-pegged-reply-bodies)
- [`server-error-status-mapping`](findings/server.md#server-error-status-mapping), [`chat-backend-capacity-refusals-become-uncertain`](findings/chat-backend.md#chat-backend-capacity-refusals-become-uncertain)
- [`agent-error-disposition`](findings/agent.md#agent-error-disposition), [`desktop-native-error-code-registry`](findings/desktop-native.md#desktop-native-error-code-registry)
- [`desktop-native-typed-agent-responses`](findings/desktop-native.md#desktop-native-typed-agent-responses), [`client-libs-agent-dto-duplication`](findings/client-libs.md#client-libs-agent-dto-duplication), [`agent-bot-contract-typed`](findings/agent.md#agent-bot-contract-typed)
- [`fe-arch-mock-bridge-contract`](findings/fe-arch.md#fe-arch-mock-bridge-contract), [`chat-code-mock-bridge-decoder`](findings/chat-code.md#chat-code-mock-bridge-decoder), [`desktop-native-command-contract-check`](findings/desktop-native.md#desktop-native-command-contract-check)
- [`fe-arch-files-list-a11y`](findings/fe-arch.md#fe-arch-files-list-a11y), [`fe-product-keyboard-a11y`](findings/fe-product.md#fe-product-keyboard-a11y)
- [`agent-observability-logs-diagnostics`](findings/agent.md#agent-observability-logs-diagnostics), [`desktop-native-agent-log-and-crash-markers`](findings/desktop-native.md#desktop-native-agent-log-and-crash-markers)
- [`build-docs-link-and-drift-check`](findings/build.md#build-docs-link-and-drift-check), [`desktop-native-doc-drift`](findings/desktop-native.md#desktop-native-doc-drift)
- [`build-deps-supply-chain-audit`](findings/build.md#build-deps-supply-chain-audit), [`protocol-crypto-dependency-hygiene`](findings/protocol.md#protocol-crypto-dependency-hygiene)
- [`build-ci-pr-required-gate`](findings/build.md#build-ci-pr-required-gate), [`build-ci-desktop-cost`](findings/build.md#build-ci-desktop-cost), [`build-frontend-format-lint-gate`](findings/build.md#build-frontend-format-lint-gate)
- [`chat-ux-delivery-states`](findings/chat-ux.md#chat-ux-delivery-states), [`chat-code-pending-row-actions`](findings/chat-code.md#chat-code-pending-row-actions)

## Refuted during verification

### chat-backend-pegged-replies-edits-reactions

**Use the existing v0.1.9 Pegged body for replies, edits and reactions in format-1 channels**

The wire facts are correct. Go v0.1.9 proto-src/lib/realtime.snowp defines RTMsgBody arm 2 (RTMsgPlaintextPegged{basic, replyTo}) for Reply, Reactji and Edit. FOKS-RS crypto (foks-crypto realtime.rs:135-149), the server (messages.rs:16) and history (history.rs:298) accept or render only Basic. The finding's central claim, a Go-interoperable path in format-1 channels, does not hold. The pinned Go v0.1.9 client rejects any non-Basic body: client/librt/minder.go openMessage returns VersionNotSupportedError("only basic messages are supported") when pt != RTMsgType_Basic. decodeMsgs and decodeAndCacheServerMsgs return that error for the whole fetch. One Pegged reply, edit or reaction in a format-1 channel would therefore make every page containing it unreadable for Go v0.1.9 members, on Go hosts (whose rtSend stores Md.Typ unchecked) and on FOKS-RS hosts that Go clients use. A server-side pegged_content capability cannot protect Go readers who share the channel, so the recommendation breaks the project's v0.1.9 wire-compatibility constraint. The feature gap itself (replies, edits, reactions) is already tracked in ISSUES.md 'Existing disclosed limitations' and the deferred extended-chat section, as format-2 work. The only new contribution here is the Pegged path, and that path is harmful. A replies and reactions refinement should be proposed against format-2 extended channels, which old clients do not read as Basic channels.

### chat-backend-dm-adhoc-teams

**Direct messages via same-host ad-hoc teams**

The cited lines are accurate: session.rs:78 rejects non-named teams, policy.rs:59 has t.team_kind=3, realtime.rs:225-226 RtTeamId admits ENTITY_AD_HOC_TEAM, crypto realtime.rs:57 accepts ad-hoc parties, agent main.rs:5148 calls create_adhoc_team, and status_codes.rs:41 defines 7104. The core premise is still wrong. In v0.1.9 an ad-hoc team only ever contains its creator and cannot be changed. The upstream proto (lib/team.snowp, the ApprovedAdHoc arm) says: 'Ad-hoc teams have fixed, founder-only membership'. The Rust server enforces the same rule. crates/foks-server/src/identity/team_create.rs (~line 180) rejects any founding roster other than exactly the creator as OWNER. crates/foks-server/src/identity/team_edit.rs:54-56 and crates/foks-server-db/src/team.rs:275-283 reject every edit to a non-named team ('ad-hoc teams are immutable'). The oracle fixture tools/foks-v019-oracle/mutation_fixture.go:141 also requires a 'single-owner ad-hoc eldest'. create_adhoc_team (crates/foks-client-app/src/team.rs:65) takes no participants. STATUS_TEAM_ADHOC_DUPLICATE_ERROR is declared but no Rust server path emits it, so 'resolve the existing one on 7104' has nothing to work with. Allowing team_kind 20 in membership_role and chat_session would only give one-person channels. A DM would need multi-member ad-hoc team creation, which is a new team-protocol extension that v0.1.9 Go servers and clients refuse. The alternative is a small named team. The real blocker is ad-hoc membership semantics, not 'two places' plus multi-host routing. The recommendation as written is not feasible under wire compatibility.
