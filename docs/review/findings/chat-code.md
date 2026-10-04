# Chat frontend code quality

Area key `chat-code`. 14 verified findings. Part of the [foks-rs review](../README.md).

## Current state

The desktop chat client (apps/desktop/src/chat) consists of four long-lived services exposed to React through useSyncExternalStore providers: ChatInboxService (account long-poll and team sync), ChatSendService (drafts, per-channel outgoing queue, intent persistence, recovery), ChannelCreationController and NotificationConsumer. Hooks for conversation, history, composer, viewport and read intent sit on top of them. The code is documented in detail, guards stale owners with epoch and generation checks, and has about 12.6k lines of chat tests. The problems are structural. send-service.ts (1509 lines) and inbox-service.ts (1103 lines) each combine scheduling, access gating, error routing, state transitions and publication. The outgoing-message lifecycle is an implicit state machine spread across a `phase` field and seven booleans. Three local access guards, three timer implementations and five backoff formulas have drifted apart. The IPC contract is hand-maintained: the TS decoder and the native validator (crates/foks-desktop/src/chat.rs) duplicate the reply rules without a shared fixture, and the TS mutation classification already disagrees with Rust's `is_mutation`. Several modules are reached only from tests (recover-pending, recovery-schedule, most of the conversation reducer, the use-message-composer alias). There are two plausible correctness risks. First, channel creation throws the agent's own `chat-access-denied` code for a local precondition failure, and the inbox treats that as a team revalidation. Second, inbox job disposal cancels sibling-team syncs without restoring their dirty flag. Neither appears in ISSUES.md or the book.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [Make the outgoing-message lifecycle an explicit state machine instead of `phase` plus seven flags](#chat-code-outgoing-state-machine) | correctness-risk | high | M |
| [Delete dead branches in useChatConversation; give useChatHistory an options object and stable callbacks](#chat-code-conversation-hook-api) | code-quality | medium | S |
| [Inbox job disposal cancels sibling teams' syncs and drops their pending dirty flag](#chat-code-inbox-sibling-sync-cancel) | correctness-risk | medium | S |
| [Chat IPC contract is hand-maintained in five TS tables, and the mutation classification already disagrees with Rust](#chat-code-ipc-contract-parity) | correctness-risk | medium | M |
| [Route mock chat replies through decodeChatReply so render tests exercise the contract](#chat-code-mock-bridge-decoder) | testing | medium | S |
| [Replace the inbox's fixed 250 ms drain loop and three hand-written timers with a shared deadline timer and clock module](#chat-code-scheduling-primitives) | performance | medium | M |
| [Every draft keystroke re-renders the whole chat pane through the send service's global revision](#chat-code-send-revision-rerender) | performance | medium | M |
| [Split ChatSendService (1509 lines) into draft store, outgoing queue, request gateway, recovery and scheduler modules](#chat-code-send-service-decomposition) | maintainability | medium | L |
| [Remove or reconnect modules reached only from tests: recover-pending, recovery-schedule, conversation reducer branches, use-message-composer](#chat-code-vestigial-modules) | maintainability | medium | S |
| [Unify five retry-backoff formulas that already disagree on the first delay](#chat-code-backoff-formulas) | maintainability | low | S |
| [Unify the three local access guards; stop channel creation from throwing the agent's chat-access-denied code](#chat-code-local-access-errors) | correctness-risk | low | M |
| [Move the 256-character notification text bound into chat-limits.json](#chat-code-notification-limit-constant) | code-quality | low | S |
| [Move protocol action selection out of PendingRow and OutgoingRow into the send service](#chat-code-pending-row-actions) | maintainability | low | M |
| [Lint does not catch render-time service mutations and ref writes in chat hooks](#chat-code-render-purity) | tooling | low | S |

### chat-code-outgoing-state-machine

**Make the outgoing-message lifecycle an explicit state machine instead of `phase` plus seven flags**

- Type: correctness-risk
- Priority: high
- Effort: M
- Layers: desktop-ui
- Verification: confirmed

An OutgoingMessage carries a 9-value `phase` plus the flags observed, running, queued, saved, intentPending, ambiguousPreparation and automatic. `phase` is assigned in 14 places and the flags in 13 more, spread across synchronize, apply, drive (try, catch and a detached .then), observeMessage and removeIntent. Which combinations are valid is recorded only in comments. One concrete race: after a failed attempt, drive() sends a detached status request whose .then writes message.phase after checking only the team epoch, not whether a newer attempt is running. A Retry started before that reply lands therefore has its phase overwritten by the stale status.

**Evidence**

- [`apps/desktop/src/chat/send-service.ts:66`](../../../apps/desktop/src/chat/send-service.ts#L66): OutgoingMessage: phase plus 7 booleans and 2 error strings
- [`apps/desktop/src/chat/send-service.ts:1072`](../../../apps/desktop/src/chat/send-service.ts#L1072): detached status request; .then at 1076-1090 sets phase without checking message.running or an attempt token
- [`apps/desktop/src/chat/send-service.ts:1000`](../../../apps/desktop/src/chat/send-service.ts#L1000): ambiguousPreparation set before the request so retries switch to prepare-message; only a comment records this invariant
- [`apps/desktop/src/chat/outgoing-row.tsx:42`](../../../apps/desktop/src/chat/outgoing-row.tsx#L42): UI branches directly on phase strings

**Recommendation**

In chat/send/outgoing.ts, model three orthogonal axes as discriminated unions. Delivery: {stage:'queued'|'saving'} | {stage:'preparing', ambiguous} | {stage:'prepared', op} | {stage:'uncertain', op} | {stage:'terminal', op}. Intent slot: 'none' | 'pending' | 'cleared' | {cleanupFailed}. Plus `observed: boolean`. Derive the display value with one pure `phaseOf(message)` and use it from OutgoingRow. Apply all changes through `transition(message, event)` with events saved, prepared(op), attempted(op), failed(error, stage), status(op), observed, revoked and intent-cleared. Give each drive() an attempt token and discard continuations whose token is stale. Add table-driven tests over (state, event) pairs, including a stale status reply that arrives after Retry.

**Mockup:** [Delivery and integrity](../mockups/chat-delivery-and-integrity.html)

<details><summary>Verifier note</summary>

OutgoingMessage (line 66) has a 9-value phase, 7 booleans (observed, running, queued, saved, intentPending, ambiguousPreparation, automatic) and 2 error strings. `message.phase =` appears in exactly 14 statements (406, 782, 785, 957, 965, 967, 979, 998, 1021, 1042, 1059, 1071, 1082, 1158), and there are about 15 flag assignments. The race is real and somewhat worse than described. The detached status request at 1072 checks only current(team, epoch) in its .then (1076-1090), then overwrites message.operation and message.phase. The chat lane in scheduling/profile-work.ts runs foreground work before background work. When the failed attempt was a background drive (from recover's send job or drain), a foreground Retry's reconcile can overtake the stale background status. The stale reply then lands after the retry has finished and leaves an outdated phase and operation until a later apply or observeHistory corrects it. outgoing-row.tsx branches directly on phase strings from line 42 onward. The recommendation (an explicit transition function, a derived phase, attempt tokens and table-driven tests) is sound, stays inside the desktop UI, and does not affect wire compatibility.

</details>

### chat-code-conversation-hook-api

**Delete dead branches in useChatConversation; give useChatHistory an options object and stable callbacks**

- Type: code-quality
- Priority: medium
- Effort: S
- Layers: desktop-ui
- Verification: adjusted

performRequest returns early for every non-history action. As a result, the ambiguity list always evaluates false, the `action.action === 'history'` test is always true, `transport = client` is never used, and the owner chatClient never sends a request; it serves only as a liveness token. The `request = performRequest` alias carries a comment about send-service profile-queue priority that does not describe this hook. useChatHistory takes nine positional parameters. Its load callback depends on caller callback identities, and the mount effect re-runs on load, so a caller passing an inline callback would reload the first page on every render. Today's only production caller passes memoized callbacks, so this is a latent hazard. The hook also keeps its own copy of the accepted window and re-merges pages, duplicating conversation-model's merge. The copy omits the reset rule, but only its tail sequence is ever read, so this is duplication, not a behavioural difference.

**Evidence**

- [`apps/desktop/src/chat/use-chat-conversation.ts:70`](../../../apps/desktop/src/chat/use-chat-conversation.ts#L70): early return for all non-history actions
- [`apps/desktop/src/chat/use-chat-conversation.ts:98`](../../../apps/desktop/src/chat/use-chat-conversation.ts#L98): ambiguous is always false because the action is always history
- [`apps/desktop/src/chat/use-chat-conversation.ts:119`](../../../apps/desktop/src/chat/use-chat-conversation.ts#L119): condition always true; the owner client (246-256) never issues requests
- [`apps/desktop/src/chat/use-chat-conversation.ts:167`](../../../apps/desktop/src/chat/use-chat-conversation.ts#L167): stale comment and alias
- [`apps/desktop/src/chat/use-chat-history.ts:17`](../../../apps/desktop/src/chat/use-chat-history.ts#L17): nine positional parameters
- [`apps/desktop/src/chat/use-chat-history.ts:188`](../../../apps/desktop/src/chat/use-chat-history.ts#L188): load depends on callback identities; the mount effect at 198 re-runs on load
- [`apps/desktop/src/chat/use-chat-history.ts:154`](../../../apps/desktop/src/chat/use-chat-history.ts#L154): shadow merge duplicating conversation-model.ts:58-117; only its tail sequence is read
- [`apps/desktop/src/chat/chat-thread.tsx:197`](../../../apps/desktop/src/chat/chat-thread.tsx#L197): load is also invoked from click handlers (197, 255, 400), so its callbacks cannot be useEffectEvent functions under rules-of-hooks
- [`eslint.config.mjs:48`](../../../eslint.config.mjs#L48): react-hooks/rules-of-hooks is an error (plugin 7.1.1)

**Recommendation**

In useChatConversation, replace the owner client with an `{alive}` token. Delete the unreachable ambiguity and transport branches and the `request = performRequest` alias. Have markRead call sends.request(storeId, {action: 'mark-read', ...}) directly. Change the signature to `useChatHistory({channel, request, revision, history, position, incremental, onAccepted, onFatal, onLoading})`. Hold request and the callbacks in latest-value refs, as the hook already does for publishedHead, so that load depends only on channel.id and incremental. Do not use useEffectEvent: load is also called from click handlers, and rules-of-hooks rejects that. Replace the shadow re-merge with a tail-sequence ref, or with a `currentHistory()` getter that reads the cache's window after onAccepted.

<details><summary>Verifier note</summary>

The core claims hold. use-chat-conversation.ts:70 returns early for every non-history action. TypeScript then narrows action.action to 'history', so `ambiguous` at :98-102 is always false and `action.action === 'history'` at :119 is always true; `transport = client` is overwritten before use. The owner chatClient created at :246-256 never issues a request: it serves as a liveness and identity token and is disposed, and chatClient construction has no side effects beyond an AbortController (client.ts:55-58). The comment and alias at :167-169 describe send-service profile-queue priority, not anything this hook does. useChatHistory takes nine positional parameters (:17-46). load's dependencies include request, onAccepted, onFatal and onLoading (:188-196), and the mount effect at :198 re-runs when load changes. The current production caller (chat-thread.tsx:133-143) passes memoized callbacks (acceptHistory, blockHistory, capture), so the reload is a latent hazard, not a live bug. The shadow merge at :154-167 duplicates conversation-model's merge without the reset rule. That omission changes no behaviour, because load reads only `latest.current.messages.at(-1)` (the tail cursor and the replace-head check), and the reset only drops older rows; the problem is duplication. Two recommendation details are wrong. First, the code has no 'gateway'; mark-read already reaches sends.request through the early return, so the change is to call sends.request(storeId, ...) directly from markRead. Second, useEffectEvent conflicts with the project's lint setup. eslint-plugin-react-hooks 7.1.1 is configured with rules-of-hooks as an error (eslint.config.mjs:48), and that rule rejects calling an effect event from anything other than an Effect or Effect Event. load is a useCallback that click handlers also call (chat-thread.tsx:197, 255, 400). The latest-value ref pattern this file already uses (publishedHead, latest) is the compatible approach. Not tracked.

</details>

### chat-code-inbox-sibling-sync-cancel

**Inbox job disposal cancels sibling teams' syncs and drops their pending dirty flag**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: desktop-ui
- Verification: confirmed

ChatInboxService records jobs only as client→account. Both updateStores (when one team is removed or re-bound) and team-scoped handleError dispose every job on that account. sync() clears team.dirty when it starts. When a sibling's sync is cancelled this way, its catch path treats 'cancelled' as ignorable and returns without setting dirty again. A hidden window syncs a team only when it is dirty, and the poll that dirtied it has already advanced account.head. In a hidden window, that team's update and its desktop notification therefore wait for the next bump on the account or for the window to be shown. Account.pollClient and pollTeam are written but never read, and look intended for exactly this targeting.

**Evidence**

- [`apps/desktop/src/chat/inbox-service.ts:427`](../../../apps/desktop/src/chat/inbox-service.ts#L427): updateStores disposes all jobs owned by the account
- [`apps/desktop/src/chat/inbox-service.ts:677`](../../../apps/desktop/src/chat/inbox-service.ts#L677): handleError does the same for a team-scoped error
- [`apps/desktop/src/chat/inbox-service.ts:852`](../../../apps/desktop/src/chat/inbox-service.ts#L852): sync clears dirty before the request
- [`apps/desktop/src/chat/inbox-service.ts:948`](../../../apps/desktop/src/chat/inbox-service.ts#L948): a cancelled sync returns through handleError (ignore) without restoring dirty
- [`apps/desktop/src/chat/inbox-service.ts:818`](../../../apps/desktop/src/chat/inbox-service.ts#L818): a hidden window drains only dirty teams
- [`apps/desktop/src/chat/inbox-service.ts:133`](../../../apps/desktop/src/chat/inbox-service.ts#L133): pollClient/pollTeam are write-only (written at 981-982, cleared at 1083-1084)

**Recommendation**

Key jobs by {account, team, kind}. Dispose only the affected team's sync, and dispose the poll only when account.pollTeam is that team; pollClient then becomes redundant and can be deleted. In sync(), track whether a publish happened, and in finally set `team.dirty = true` when nothing was published and valid() still holds. Add a chat-inbox.test.ts case: two teams on one account, window hidden, a bump, t0 removed while t1's sync is in flight; assert that t1 syncs without another bump.

<details><summary>Verifier note</summary>

Verified in inbox-service.ts. this.jobs maps client to account only. updateStores disposes every job owned by the account when one team is removed or re-bound (427-428), and handleError does the same (677-678). sync() sets team.dirty = false at 852. When a sibling's sync is cancelled, its catch runs valid() (still true for the sibling), then handleError(cancelled), which commandRecovery maps to 'ignore', and returns at 948 without setting dirty again. drain() (818) admits a team only when `this.visible || candidate.dirty`, so in a hidden window the sibling waits for the next poll bump; the bump that dirtied it already advanced account.head (1028-1031). pollClient and pollTeam are declared at 133-134, written at 981-982 and cleared at 1083-1084, and nothing reads them. Existing tests (chat-inbox.test.ts 618, 807) do not cover a sibling cancelled while hidden. One minor point: a generation bump re-binds every team on the server at once, all of which are recreated dirty, so the bug mainly affects removal or a per-store re-bind of a single team. The recommendation is feasible and confined to the desktop UI.

</details>

### chat-code-ipc-contract-parity

**Chat IPC contract is hand-maintained in five TS tables, and the mutation classification already disagrees with Rust**

- Type: correctness-risk
- Priority: medium
- Effort: M
- Layers: desktop-ui, desktop-native, agent
- Verification: adjusted

Per-action properties of the chat IPC contract are hand-maintained in several TS tables: client.ts `kinds`, the decodeChatReply branch conditions, chatActionMutates, and send-service's localActions and coalescing list. No test checks the mutation or result-kind classification against Rust. One drift already exists: chatActionMutates treats `cleanup-pending` as a mutation, while Rust ChatAction::is_mutation treats it as a read. Its errors therefore go through checkedMutation and normalizeMutationError and are tagged ambiguous and non-retryable. The only caller (send-service recover) ignores those flags, so this has no effect yet. Reply validation is already cross-checked by the shared chat-replies.json fixture, but that fixture covers only 10 of the 22 actions.

**Evidence**

- [`apps/desktop/src/chat-contract.ts:80`](../../../apps/desktop/src/chat-contract.ts#L80): chatActionMutates omits cleanup-pending (operation-body is already listed at 83)
- [`crates/foks-agent-proto/src/chat.rs:130`](../../../crates/foks-agent-proto/src/chat.rs#L130): is_mutation excludes CleanupPending and OperationBody
- [`apps/desktop/src/bridge/commands-core.ts:64`](../../../apps/desktop/src/bridge/commands-core.ts#L64): chatActionMutates selects checkedMutation for chat_request
- [`apps/desktop/src/bridge/transport.ts:91`](../../../apps/desktop/src/bridge/transport.ts#L91): mutation responses that fail to decode become ambiguous and non-retryable
- [`apps/desktop/src/chat/send-service.ts:1264`](../../../apps/desktop/src/chat/send-service.ts#L1264): sole cleanup-pending caller; ambiguity flags unused, so no current behavioral effect
- [`apps/desktop/src/chat/client.ts:21`](../../../apps/desktop/src/chat/client.ts#L21): action-to-result table (`kinds`)
- [`apps/desktop/src/chat/send-service.ts:123`](../../../apps/desktop/src/chat/send-service.ts#L123): localActions, plus coalescing keys at 805-811
- [`crates/foks-desktop/src/chat.rs:713`](../../../crates/foks-desktop/src/chat.rs#L713): existing shared fixture test runs validate_chat_reply over chat-replies.json
- [`apps/desktop/tests/chat-contract.test.ts:257`](../../../apps/desktop/tests/chat-contract.test.ts#L257): same fixture asserted against decodeChatReply
- [`apps/desktop/src-tauri/wire-contract/inventory.json:176`](../../../apps/desktop/src-tauri/wire-contract/inventory.json#L176): chat listed under uncoveredPublicShapes (golden fixture only)

**Recommendation**

(1) Add a single `CHAT_ACTIONS` record to chat-contract.ts, for example `{ 'cleanup-pending': { result: 'cleanup-pending', mutates: false, local: true, coalesce: true }, ... } satisfies Record<ChatAction['action'], ActionMeta>`. Derive `kinds`, chatActionMutates, localActions and the coalescing set from it, which also fixes the cleanup-pending classification. (2) Extend the existing crates/foks-agent-proto/tests/fixtures/chat-replies.json, or add a sibling chat-actions.json next to it, with one entry per action giving its canonical serialized request, `mutates` and `resultKind`. Assert it in Rust: serialize every variant, compare is_mutation, and compare a new ChatAction::result_kind(). Assert it in TS (chat-contract.test.ts) against CHAT_ACTIONS. Add reply cases for the actions the fixture does not yet cover: inbox, poll-inbox, mark-read, notification-history, pending, cleanup-pending, prepare-channel, prepare-message, attempt, cancel, finalize and reconcile. (3) Once the golden inventory covers chat, update inventory.json and the assertion at wire-contract-domains.test.ts:80. This affects only the local desktop-to-agent IPC; the wire protocol is unchanged.

<details><summary>Verifier note</summary>

Partly wrong. chatActionMutates (chat-contract.ts:80) already lists 'operation-body' as non-mutating (line 83), so it agrees with Rust. Only 'cleanup-pending' disagrees: Rust's is_mutation (foks-agent-proto chat.rs:130) treats CleanupPending as a read, and TS treats it as a mutation. The consequence is also overstated. The only caller is send-service recover() (line 1264), which never reads `ambiguous` or `retryable`, so the misclassification has no behavioral effect today. 'Nothing detects drift' is incorrect for reply validation. A shared fixture, crates/foks-agent-proto/tests/fixtures/chat-replies.json (65 cases across 10 action kinds), is already asserted against Rust validate_chat_reply (foks-desktop chat.rs:713) and against TS decodeChatReply (tests/chat-contract.test.ts:257). The inventory notes that 'uncovered' means only absent from the golden fixture. The remaining claims hold: per-action properties are spread across client.ts `kinds`, the decodeChatReply branches, chatActionMutates, localActions (send-service:123) and the coalescing list (805-811), and no test checks the mutation or result-kind classification across languages. The single-table recommendation is sound, but it should extend the existing shared fixture rather than add a parallel wire-contract/chat.json.

</details>

### chat-code-mock-bridge-decoder

**Route mock chat replies through decodeChatReply so render tests exercise the contract**

- Type: testing
- Priority: medium
- Effort: S
- Layers: desktop-ui
- Verification: adjusted

The native bridge decodes every chat reply with decodeChatReply. The mock bridge returns mockChat replies undecoded, so render suites never run the decoder's invariants, such as the gap rule and the `before`/minimum-sequence rule. NotificationConsumer repeats the row and text bounds to make up for this. The suggested wrap cannot be applied directly: the decoder parses storeId as a JSON TeamStoreRef, while fixture team ids are 'team:eng' and similar, so every mock reply would be rejected. Once the store id matches, mockChat still fails the decoder on notification-history for messages over 256 characters, because it does not truncate as the agent does. In mock mode such a message makes the notification consumer raise a channel integrity error.

**Evidence**

- [`apps/desktop/src/bridge/commands-core.ts:68`](../../../apps/desktop/src/bridge/commands-core.ts#L68): native chat path decodes with decodeChatReply
- [`apps/desktop/src/mock-bridge.ts:478`](../../../apps/desktop/src/mock-bridge.ts#L478): mock bridge exposes mockChat undecoded (created at 432)
- [`apps/desktop/src/chat-contract.ts:344`](../../../apps/desktop/src/chat-contract.ts#L344): decodeChatScope JSON-parses storeId, so fixture ids like 'team:eng' (fixture.ts:99) fail; gap rule at 433-435, before rule at 474-481
- [`apps/desktop/src/chat-mock.ts:180`](../../../apps/desktop/src/chat-mock.ts#L180): notification-history returns full text with no 256-character truncation; the decoder rejects it at chat-contract.ts:465
- [`apps/desktop/src/chat/notification-consumer.ts:470`](../../../apps/desktop/src/chat/notification-consumer.ts#L470): duplicate 50-row and 256-character check added for fake and adapter replies

**Recommendation**

Split decodeChatReply into a scope check and a result decoder, decodeChatResult(result, scope, action). Have the mock bridge run the result decoder on structuredClone(reply), with a scope check that matches the mock's own fixture id mapping. The alternative, migrating fixture team ids to the native JSON TeamStoreRef encoding, touches 40 files. Make mockChat truncate notification-history text to 256 characters, matching the agent. Add a decodedChatBridge(fake) helper in tests for ad-hoc fakes, and a test that runs a scripted mockChat session through the decoder. Remove the consumer's duplicate bounds check, or keep it as an explicit defence-in-depth check, only after every fake goes through decoding.

<details><summary>Verifier note</summary>

The core claim holds. The native bridge decodes chat replies (commands-core.ts:64-69). mock-bridge.ts:478 returns mockChat (created at 432) without decoding, and the client layer does not decode either: decodeChatReply's only src caller is commands-core. notification-consumer.ts:470-479 repeats the 50-row and 256-character checks, and its comment says this is for fake and adapter replies. The proposed one-line wrapper is not feasible as written. decodeChatScope (chat-contract.ts:344-356) parses storeId as a JSON TeamStoreRef, but the mock fixture team ids are 'team:eng', 'team:household' and 'team:homelab', with 508 references across 40 files. I ran mock-bridge replies through decodeChatReply in a scratch script, and every action failed with chat-integrity. With a JSON-form store id, mockChat replies decode for sync-inbox, history (with and without after), submit-message, pending, channels and mark-read. notification-history fails once a message exceeds 256 characters, because mockChat does not truncate the way the agent does (crates/foks-agent/src/chat.rs:726). In mock mode, a long message therefore makes NotificationConsumer throw channelIntegrity. This is a concrete divergence that the decoder would expose. Effort is M, not S.

</details>

### chat-code-scheduling-primitives

**Replace the inbox's fixed 250 ms drain loop and three hand-written timers with a shared deadline timer and clock module**

- Type: performance
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

ChatSendService.arm and NotificationConsumer.kick each implement 'arm for the earliest deadline and only ever move it earlier', with separate timer and timerDue fields. ChatInboxService.drain instead re-arms itself every 250 ms while visible and every 2 s while hidden, whether or not anything is due, and on each pass walks every account and calls flushDegraded for every team; a test pins those intervals. ChatClock and systemChatClock are defined in inbox-service.ts, so send-service and notification-consumer import the 1103-line inbox module at runtime just to get a clock. NotificationProvider builds the consumer with an undefined clock, so the consumer uses Date.now/setTimeout while the inbox and send services use the shell's lease clock.

**Evidence**

- [`apps/desktop/src/chat/inbox-service.ts:842`](../../../apps/desktop/src/chat/inbox-service.ts#L842): drain() always re-arms itself at 250 ms (visible) or 2,000 ms (hidden)
- [`apps/desktop/tests/chat-inbox.test.ts:834`](../../../apps/desktop/tests/chat-inbox.test.ts#L834): test pins the 250 ms and 2,000 ms drain intervals
- [`apps/desktop/src/chat/send-service.ts:1448`](../../../apps/desktop/src/chat/send-service.ts#L1448): arm(): hand-written bring-forward timer with a timerDue field
- [`apps/desktop/src/chat/notification-consumer.ts:156`](../../../apps/desktop/src/chat/notification-consumer.ts#L156): kick(): second hand-written bring-forward timer
- [`apps/desktop/src/chat/inbox-service.ts:41`](../../../apps/desktop/src/chat/inbox-service.ts#L41): ChatClock is defined here and systemChatClock at line 94
- [`apps/desktop/src/chat/send-service.ts:31`](../../../apps/desktop/src/chat/send-service.ts#L31): runtime import of systemChatClock from inbox-service; notification-consumer.ts:6-10 does the same
- [`apps/desktop/src/chat/notification-provider.tsx:82`](../../../apps/desktop/src/chat/notification-provider.tsx#L82): NotificationConsumer is constructed with an `undefined` clock; inbox-provider.tsx:112 passes no clock to NotificationProvider
- [`apps/desktop/src/scheduling/lease-expiry.ts:9`](../../../apps/desktop/src/scheduling/lease-expiry.ts#L9): production lease clock is Date.now()/1000, so the mismatch matters only for injected test clocks
- [`apps/desktop/src/chat/inbox-service.ts:963`](../../../apps/desktop/src/chat/inbox-service.ts#L963): sync() finally block clears busy and profile exclusion without kick(); poll() finally at 1079 also has no kick(); setOpenChannel (352) notifies listeners only

**Recommendation**

Add chat/clock.ts (ChatClock, systemChatClock) and chat/deadline-timer.ts with `DeadlineTimer(clock, fire)` exposing armAt(due) (only moves the deadline earlier), reset(due) and cancel(). Use it in all three services. In inbox drain, compute the next deadline as the minimum of: account.due for accounts that are unblocked, not busy and under poll capacity; team.due for teams that are visible or dirty, not busy and not quarantined; team.accessExpiresAt; and pending degraded deadlines (at + interval, or the base interval for the open channel). Arm nothing when the minimum is infinite. Add the missing wake-ups that the 250 ms loop currently provides: call kick() from the sync() and poll() finally blocks (they release busy, poll capacity and profile exclusion) and from setOpenChannel (it changes the open channel's degraded wait). Pass `clock` from ChatInboxProvider through NotificationProvider to NotificationConsumer. Replace the interval-pinning test with two tests: an idle visible inbox arms no timer earlier than its next resync or access-expiry deadline, and a due team is served at its deadline. Add a third test: a team held back by profile exclusion is synchronized when the sibling sync completes.

**Already tracked:** book/20-desktop.qmd 'The notification consumer' documents deadline arming for the consumer only; the inbox loop is not discussed.

<details><summary>Verifier note</summary>

The core claims hold. inbox-service.ts:842-845 re-arms drain() unconditionally, every 250 ms while visible and every 2,000 ms while hidden. Each drain walks every account and calls flushDegraded for every team. tests/chat-inbox.test.ts:834 pins both intervals. send-service.ts:1448-1463 (arm) and notification-consumer.ts:156-167 (kick) each implement the bring-forward timer with separate timer and timerDue fields. ChatClock and systemChatClock are at inbox-service.ts:41 and :94, and both send-service.ts:31 and notification-consumer.ts:6-10 import systemChatClock at runtime. NotificationProvider passes `undefined` as the clock (notification-provider.tsx:82, in the constructor call that starts at :77). inbox-provider.tsx:112 does not pass the clock to NotificationProvider, while ChatInboxService and ChatSendService receive the shell's chatClock (access-runtime.ts:150). Two qualifications. First, in production the lease clock is Date.now()/1000 (lease-expiry.ts:9-13), so the clock mismatch shows up only when a test injects a clock; it does not cause skew in the shipped app. Second, the recommendation is wrong about the existing wake-ups. sync() and poll() completion (the finally blocks at inbox-service.ts:963 and :1079) and setOpenChannel (:352-361, which only notifies listeners) do not call kick(); they currently rely on the 250 ms loop. Profile mutual exclusion (`profiles.has(server)`) is released only in the sync finally block, so a sibling team waiting on that profile would stall until some other event. The proposed test is also inconsistent: a visible idle team keeps team.due = now + RESYNC_IDLE_MS (300 s), so under the proposed minimum-deadline rule a timer stays armed. Neither book/20-desktop.qmd nor ISSUES.md tracks the inbox loop.

</details>

### chat-code-send-revision-rerender

**Every draft keystroke re-renders the whole chat pane through the send service's global revision**

- Type: performance
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

On each keystroke, setDraft re-encodes every draft of every team to enforce the global quota, then publish() increments one global revision. ChatScreen, useChatConversation and useChatComposer all subscribe to that revision, and ChatThread is not memoized. Each keystroke therefore re-renders the screen and maps the full history window (up to 1000 rows) through MessageText. canRetain and observeHistory likewise re-sum all byte lengths on every call.

**Evidence**

- [`apps/desktop/src/chat/send-service.ts:443`](../../../apps/desktop/src/chat/send-service.ts#L443): setDraft encodes all drafts of all teams on every call
- [`apps/desktop/src/chat/send-service.ts:224`](../../../apps/desktop/src/chat/send-service.ts#L224): publish(): single global revision; also re-runs arm() on every keystroke
- [`apps/desktop/src/chat/send-provider.tsx:96`](../../../apps/desktop/src/chat/send-provider.tsx#L96): useChatSends subscribes every caller to that revision
- [`apps/desktop/src/screens/chat-screen.tsx:67`](../../../apps/desktop/src/screens/chat-screen.tsx#L67): ChatScreen subscribes; it reads sends.drafts() at line 135
- [`apps/desktop/src/chat/chat-thread.tsx:112`](../../../apps/desktop/src/chat/chat-thread.tsx#L112): ChatThread subscribes itself through useMessageComposer, so memoizing ChatThread would not help
- [`apps/desktop/src/chat/chat-thread.tsx:292`](../../../apps/desktop/src/chat/chat-thread.tsx#L292): full message list (up to 1000 rows) rendered inline without memoization
- [`apps/desktop/src/chat/send-service.ts:636`](../../../apps/desktop/src/chat/send-service.ts#L636): canRetain re-encodes every retained message
- [`apps/desktop/src/chat/send-service.ts:1177`](../../../apps/desktop/src/chat/send-service.ts#L1177): observeHistory recomputes observation bytes on every call

**Recommendation**

Keep drafts in a separate draft store inside the send service. Give it per-(store, channel) subscriptions, a per-store 'has draft' subscription for ChatScreen's channel list, and a total byte count maintained by delta, so setDraft encodes only the edited text. Do not route draft changes through publish(), so typing no longer re-arms the tick or notifies every subscriber. Expose selector hooks useChatDraft(store, channel) and useOutgoing(store, channel), each returning a per-key memoized snapshot that changes only when that channel's draft or outgoing messages change. Keep the global revision for operation and unsentCount changes. Extract the message list into a React.memo component whose props are the history window and the stable row inputs (actor, senderNames, newFrom). Add a render-count test to chat-composer.render.test.tsx showing that typing does not re-render the message list. Optionally, maintain retained and observation byte totals incrementally in canRetain and observeHistory.

**Mockup:** [Chat composer refinements](../mockups/chat-composer.html)

<details><summary>Verifier note</summary>

The core claims hold. setDraft (send-service.ts:443-466) TextEncodes every draft of every team on each call, then calls publish(). publish() (:224-230) increments one global revision, calls arm() (which runs nextTickDelay/outstanding across all teams) and notifies every listener. useChatSends (send-provider.tsx:96-104) subscribes every caller to that revision through useSyncExternalStore. Subscribers: ChatScreen (chat-screen.tsx:67), useChatConversation (:40), and useChatComposer (:24), which ChatThread calls itself at chat-thread.tsx:112. ChatThread renders the history window inline (:292) with BigInt comparisons, messageDayKey and a non-memoized MessageText per row; CHAT_HISTORY_ROWS is 1000. canRetain (:636) and observeHistory (:1177) re-sum byte lengths on every call. Not tracked. Adjustments: ChatThread subscribes through useMessageComposer, so memoizing ChatThread would not stop the re-render; only extracting the message list works, which the recommendation already proposes. ChatScreen reads sends.drafts(location.ref) during render (chat-screen.tsx:135), so it needs a per-store draft-presence subscription rather than none. The recommendation depends on a DraftStore from another finding that this review cannot verify, so it is restated here on its own terms.

</details>

### chat-code-send-service-decomposition

**Split ChatSendService (1509 lines) into draft store, outgoing queue, request gateway, recovery and scheduler modules**

- Type: maintainability
- Priority: medium
- Effort: L
- Layers: desktop-ui
- Verification: adjusted

ChatSendService (1509 lines) holds seven concerns that all mutate one 21-field Team record: draft storage with a global byte quota, the per-channel outgoing queue and intent persistence, the generic request gateway (the conversation sends every non-history action through it), reply reconciliation, history observation, recovery job planning with its own backoff, and the tick scheduler. Teardown exists in two copies that differ. stop() truncates messages and operations in place and leaves the per-team bookkeeping alone. synchronize() reassigns those arrays and resets loaded, loadErrors, recovered and jobs. operations() returns the internal array while messages() returns copies. Today every renderer filters it into a new array, so the in-place truncation has no visible effect, but the API is inconsistent.

**Evidence**

- [`apps/desktop/src/chat/send-service.ts:94`](../../../apps/desktop/src/chat/send-service.ts#L94): Team record with 21 fields shared by every subsystem
- [`apps/desktop/src/chat/send-service.ts:439`](../../../apps/desktop/src/chat/send-service.ts#L439): setDraft: draft storage plus a global quota recomputed across all teams on every keystroke
- [`apps/desktop/src/chat/send-service.ts:707`](../../../apps/desktop/src/chat/send-service.ts#L707): apply(): reply reconciliation into operations and messages
- [`apps/desktop/src/chat/send-service.ts:799`](../../../apps/desktop/src/chat/send-service.ts#L799): request()/perform(): gateway with access gating and coalescing; use-chat-conversation.ts:70 sends every non-history action through it
- [`apps/desktop/src/chat/send-service.ts:1257`](../../../apps/desktop/src/chat/send-service.ts#L1257): recover(): recovery job planner and executor
- [`apps/desktop/src/chat/send-service.ts:1448`](../../../apps/desktop/src/chat/send-service.ts#L1448): arm()/tick(): scheduler
- [`apps/desktop/src/chat/send-service.ts:256`](../../../apps/desktop/src/chat/send-service.ts#L256): stop() truncates messages/operations in place (`length = 0`)
- [`apps/desktop/src/chat/send-service.ts:354`](../../../apps/desktop/src/chat/send-service.ts#L354): synchronize() repeats the teardown with reassignment and also resets loaded/loadErrors/recovered/jobs
- [`apps/desktop/src/chat/send-service.ts:477`](../../../apps/desktop/src/chat/send-service.ts#L477): operations() returns the internal array, while messages() at 464 returns copies
- [`apps/desktop/src/screens/chat-screen.tsx:375`](../../../apps/desktop/src/screens/chat-screen.tsx#L375): ChatThread receives a filtered copy of pending, so in-place truncation is not observed

**Recommendation**

Keep ChatSendService as a facade with its current public methods so that the providers and tests do not change. Extract the following modules. chat/send/team.ts: Team creation plus a single `retire(team, reason)` used by stop() and by the removal, replacement and rejection paths in synchronize(). chat/send/draft-store.ts: per-(store, channel) drafts with a running byte total updated by delta. chat/send/outgoing.ts: the message type and pure transitions (see chat-code-outgoing-state-machine). chat/send/gateway.ts: perform, authorize and coalescing. chat/send/recovery.ts: a pure `planRecoveryJobs(team) -> Job[]` plus an executor. chat/send/scheduler.ts: outstanding, nextTickDelay and tick on the injected ChatClock. Order of work: (1) team.retire and DraftStore, which change no behavior and are covered by chat-send-service.test.ts and chat-send-recovery.test.ts; (2) gateway; (3) outgoing reducer; (4) recovery and scheduler. Return copies from operations(), consistent with messages().

<details><summary>Verifier note</summary>

The core claims hold. send-service.ts has 1509 lines. One class mixes drafts and their quota (setDraft:439), the queue and intents, the generic request gateway (request:799; use-chat-conversation.ts:70 sends every non-history action through sends.request), reply reconciliation (apply:707), recovery (recover:1257) and the scheduler (arm:1448, tick:1464). The two teardown copies do differ: stop() truncates at 256-257, while synchronize() reassigns at 362-363 and also resets loaded, loadErrors, recovered and jobs. Corrections: Team has 21 fields, not 20. The claim that 'stop() mutates an array that a rendered component still holds' has no observable effect. use-chat-conversation.ts:280 returns sends.operations(storeId), but chat-screen.tsx:207 and :375 pass ChatThread a fresh `pending.filter(...)` copy. stop() is called only from the provider teardown (send-provider.tsx:51), so no rendered consumer reads the truncated array. The recommendation names a 'shared DeadlineTimer', which does not exist in apps/desktop/src. The scheduler should use the injected ChatClock. This is a maintainability refactor and no defect is shown, so priority is lowered to medium.

</details>

### chat-code-vestigial-modules

**Remove or reconnect modules reached only from tests: recover-pending, recovery-schedule, conversation reducer branches, use-message-composer**

- Type: maintainability
- Priority: medium
- Effort: S
- Layers: desktop-ui
- Verification: adjusted

Only tests/chat-recovery.test.ts imports recoverPending, RecoverySchedule and coalesceStatus; production recovery is ChatSendService.recover, which has its own planner. The fairness rule those tests assert ('new arrivals join the tail') is therefore not what production does: send-service sorts jobs by due time, and jobs it has never seen have due 0. In conversationResult, the 'reset', 'access' and 'status-unresolved' branches and the operations half of ConversationState are unreachable. Its only production caller passes operations: [] with a history event, and eventFromReply never emits those kinds. use-message-composer.ts is a one-line alias, and its only test asserts the alias.

**Evidence**

- [`apps/desktop/src/chat/recover-pending.ts:13`](../../../apps/desktop/src/chat/recover-pending.ts#L13): recoverPending: no production importer; only tests/chat-recovery.test.ts imports it
- [`apps/desktop/src/chat/recovery-schedule.ts:51`](../../../apps/desktop/src/chat/recovery-schedule.ts#L51): tail-fairness rule; select() is limited to the status/body lanes over TrackedOperation
- [`apps/desktop/src/chat/send-service.ts:1379`](../../../apps/desktop/src/chat/send-service.ts#L1379): production job order: by checks due time; unseen keys default to 0 and run first; five job kinds keyed by string
- [`apps/desktop/src/chat/conversation-model.ts:28`](../../../apps/desktop/src/chat/conversation-model.ts#L28): reset/access/status-unresolved branches; no producer of these kinds exists in src or tests
- [`apps/desktop/src/chat/history-cache.ts:157`](../../../apps/desktop/src/chat/history-cache.ts#L157): sole production caller; passes operations: [] with a history event
- [`apps/desktop/src/chat/conversation-events.ts:34`](../../../apps/desktop/src/chat/conversation-events.ts#L34): access/status-unresolved/reset event kinds never produced
- [`apps/desktop/src/chat/use-message-composer.ts:1`](../../../apps/desktop/src/chat/use-message-composer.ts#L1): alias module; chat-thread.tsx:10-11 imports from both names
- [`apps/desktop/tests/chat-composer.render.test.tsx:178`](../../../apps/desktop/tests/chat-composer.render.test.tsx#L178): test that only checks the alias
- [`apps/desktop/tests/chat-operations.test.ts:124`](../../../apps/desktop/tests/chat-operations.test.ts#L124): uses conversationResult with pending events to test observation; needs porting, not deletion

**Recommendation**

Delete recover-pending.ts and recovery-schedule.ts. The production planner in ChatSendService.recover covers more job kinds than RecoverySchedule can express. Port the properties that production already has (later keys are eventually checked despite permanent failures; removed or terminal work is skipped) to chat-send-recovery.test.ts. If tail fairness for new arrivals is wanted, implement it in send-service's job ordering (for example, give unseen keys a served turn instead of due 0) and test it there. Reduce conversation-model.ts to `mergeHistoryPage(previous, event): HistoryWindow`. Delete the access/status-unresolved/reset event kinds. Rewrite the chat-operations.test.ts observation cases against reconcileOperations/observeMessages directly, and update the render-test harnesses in chat-composer.render.test.tsx (around lines 437 and 660) to call mergeHistoryPage. Delete use-message-composer.ts and its alias test, and import useChatComposer directly. Optionally, extend module-boundaries.test.ts to fail when a src/chat module has no importer outside tests; count type-only imports too, because the existing imports() helper skips them.

<details><summary>Verifier note</summary>

Every core claim holds. recoverPending, RecoverySchedule and coalesceStatus are imported only by tests/chat-recovery.test.ts. Production ChatSendService.recover (send-service.ts:1256-1376) builds its own job list and sorts it by `team.checks.get(key)?.due ?? 0` (lines 1379-1388), so jobs it has never seen sort first, the opposite of the tested 'new arrivals join the tail' rule. In conversation-model.ts, the 'reset', 'access' and 'status-unresolved' branches are unreachable: eventFromReply never emits those kinds, and no code in src/ or tests/ constructs them. history-cache.ts:157/167 is the sole production caller and always passes `operations: []` with a history event, so the non-history and operations paths are also unreachable in production. use-message-composer.ts is a one-line re-export; chat-thread.tsx:10-11 imports from both module names; chat-composer.render.test.tsx:178 only asserts the alias. The 'preferred' recommendation is not feasible as stated. RecoverySchedule.select is hard-wired to two lanes (status, body) over TrackedOperation rows, with its own eligibility predicates. Production recover schedules five string-keyed job kinds (status/reconcile chosen by readability, operation-body, cleanup+finalize, intent cleanup, automatic send) with different eligibility rules. Adopting RecoverySchedule would mean rewriting it as a generic keyed scheduler, so it is not 'already tested' for production use. Porting the tail-fairness case unchanged would fail against current production. chat-operations.test.ts uses conversationResult to exercise operation observation through pending events, so those assertions need to move to reconcileOperations/observeMessages rather than be deleted. Not tracked in ISSUES.md or the book.

</details>

### chat-code-backoff-formulas

**Unify five retry-backoff formulas that already disagree on the first delay**

- Type: maintainability
- Priority: low
- Effort: S
- Layers: desktop-ui
- Verification: adjusted

In production, the first retry delay after a failure is 2 s in send-service checkedWork (1000·2^failures with failures ≥ 1) and 1 s in notification-consumer (1000·2^(failures−1)). recovery-schedule.ts uses the consumer's formula but is reached only from tests. inbox-service uses a separate jittered doubling from 250 ms to 30 s, and the send tick uses flat 2 s/5 s waits. The degraded-fallback interval (5 s doubling to 60 s, reset on movement, open channel exempt) is implemented and documented twice, in inbox-service degradedAdmission and in notification-consumer commit()/fallbackDue. The two store the open-channel exemption differently: the inbox resets the stored interval, while the consumer overrides it when reading.

**Evidence**

- [`apps/desktop/src/chat/send-service.ts:1254`](../../../apps/desktop/src/chat/send-service.ts#L1254): min(1000*2**failures, 30000) with failures >= 1, so the first delay is 2 s; success waits 2,000 ms
- [`apps/desktop/src/chat/notification-consumer.ts:675`](../../../apps/desktop/src/chat/notification-consumer.ts#L675): min(1000*2**(failures-1), 30000), so the first delay is 1 s
- [`apps/desktop/src/chat/recovery-schedule.ts:75`](../../../apps/desktop/src/chat/recovery-schedule.ts#L75): same formula as the consumer, but the module is imported only by tests/chat-recovery.test.ts
- [`apps/desktop/src/chat/inbox-service.ts:794`](../../../apps/desktop/src/chat/inbox-service.ts#L794): jittered delay(retry), doubling from 250 ms to a 30 s cap
- [`apps/desktop/src/chat/inbox-service.ts:734`](../../../apps/desktop/src/chat/inbox-service.ts#L734): degradedAdmission: 5 s to 60 s geometric interval; the stored interval is reset to the base for the open channel
- [`apps/desktop/src/chat/notification-consumer.ts:492`](../../../apps/desktop/src/chat/notification-consumer.ts#L492): commit(): the same geometric interval again; the open-channel exemption is applied at read time in fallbackDue (line 213)

**Recommendation**

Add chat/backoff.ts with `retryDelay(failures, {base, cap, jitter, random})` and a `GeometricInterval` value type {at, interval} with `next(moved, now)` and `due(presented)`. Apply the open-channel exemption at read time, as the consumer and the book do. Use these at the live retry sites (send-service checkedWork, notification-consumer failure path, inbox delay) and in both degraded paths. Choose one first-failure delay, and keep the constants in one place that the bounds table in book/20-desktop.qmd can cite. Delete recovery-schedule.ts together with its duplicate formula (see chat-code-vestigial-modules) rather than migrating it.

<details><summary>Verifier note</summary>

All formulas match the code. send-service.ts:1249-1254 computes failures = min(prev+1, 6) and a delay of min(1000*2**failures, 30000), so the first delay is 2 s; success waits 2,000 ms. notification-consumer.ts:673-675 and recovery-schedule.ts:75 use min(1000*2**(failures-1), 30000), so the first delay is 1 s. inbox-service.ts:794 uses a jittered delay (retry + up to 25%) that doubles from 250 ms to a 30 s cap. The send tick uses flat 2,000/5,000 ms waits (send-service.ts ~1478/1490). The 5 s-to-60 s geometric fallback is implemented twice: inbox degradedAdmission at :734 and consumer commit()/fallbackDue at :492/:213. Corrections: recovery-schedule.ts has no production importer (only tests/chat-recovery.test.ts, see chat-code-vestigial-modules), so four formulas are live, and the live first-delay disagreement is send-service (2 s) against notification-consumer (1 s). The two degraded implementations also differ in how they hold the open-channel exemption. The inbox writes the base interval back into state when the open channel is admitted; the consumer leaves the stored interval doubling and substitutes the base when it reads it (fallbackDue), which is the behaviour the book describes ('whatever interval is stored'). A shared type has to pick one of these. Neither ISSUES.md nor the book tracks this.

</details>

### chat-code-local-access-errors

**Unify the three local access guards; stop channel creation from throwing the agent's chat-access-denied code**

- Type: correctness-risk
- Priority: low
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

Three places check the same local precondition (shell availability and access generation) and throw different codes: send-service and use-chat-conversation throw 'access-changed' (the conversation may throw availability.reason instead), while ChannelCreationController throws 'chat-access-denied', which is a native agent code. When the channel-creation guard fires inside run(), the error reaches inbox.handleError, which classifies it as {revalidate, team}. The inbox then increments the team's generation, publishes it 'unavailable' with the channel-creation message, and disposes every job on the account, including the poll and sibling teams' syncs. run()'s finally calls inbox.invalidate() immediately afterwards, so the team re-syncs at once rather than after 30 s, and the effect is transient. A plausible trigger: an access-generation bump (an account added on the same server, or a lease expiry) re-binds the team while a creation is in flight. The unit-test stub for handleError reacts only to chat-integrity, so this path is never exercised.

**Evidence**

- [`apps/desktop/src/chat/channel-creation.ts:198`](../../../apps/desktop/src/chat/channel-creation.ts#L198): local guard throws code 'chat-access-denied'
- [`apps/desktop/src/chat/channel-creation.ts:464`](../../../apps/desktop/src/chat/channel-creation.ts#L464): run() catch forwards it to inbox.handleError
- [`apps/desktop/src/chat/channel-creation.ts:496`](../../../apps/desktop/src/chat/channel-creation.ts#L496): finally calls inbox.invalidate(), resetting due to 0, which overrides the 30 s revalidate delay
- [`apps/desktop/src/bridge/errors.ts:67`](../../../apps/desktop/src/bridge/errors.ts#L67): chat-access-denied maps to {kind:'revalidate', scope:'team'}
- [`apps/desktop/src/chat/inbox-service.ts:650`](../../../apps/desktop/src/chat/inbox-service.ts#L650): publishes the team 'unavailable', increments its generation, then disposes every account job (677-678)
- [`apps/desktop/src/chat/actions.ts:5`](../../../apps/desktop/src/chat/actions.ts#L5): preparationCanChange includes chat-access-denied, so a 'before' guard failure currently cancels the creation record
- [`apps/desktop/src/chat/send-service.ts:855`](../../../apps/desktop/src/chat/send-service.ts#L855): same condition thrown as 'access-changed' (also at 990)
- [`apps/desktop/src/chat/use-chat-conversation.ts:89`](../../../apps/desktop/src/chat/use-chat-conversation.ts#L89): third guard (history only), code 'access-changed' or availability.reason
- [`apps/desktop/tests/channel-creation.test.ts:105`](../../../apps/desktop/tests/channel-creation.test.ts#L105): stub handleError ignores everything except chat-integrity; line 654 asserts the 'chat-access-denied' code
- [`apps/desktop/src/chat/channel-creation.ts:469`](../../../apps/desktop/src/chat/channel-creation.ts#L469): error codes classified by regex /(?:stale|key|name-conflict)/

**Recommendation**

Add chat/access-guard.ts with `chatAccessGuard({snapshot, generations, now, inbox, store, scope}) => (phase) => void` and use it in all three callers. It throws one local error, `accessChanged(phase)`, with code 'access-changed' and `origin: 'local'`. Make ChatInboxService.handleError ignore local-origin errors other than integrity errors. Decide explicitly what a 'before' guard failure should do to a channel-creation record. Today preparationCanChange('chat-access-denied') cancels the record and discards its input, while 'access-changed' would leave it 'unresolved'. Cover that decision with a test, and update the assertion at channel-creation.test.ts:654. Replace the regex with an explicit set: chat-key-unavailable, chat-name-conflict, chat-reprepare-required. Move the local error factories (cancelled, integrity, channelIntegrity, limit, accessChanged) into chat/errors.ts as typed CommandError values, replacing the plain-object throws. Add a test that connects ChannelCreationController to a real ChatInboxService and increments the generation in the middle of prepare-channel.

<details><summary>Verifier note</summary>

The core claim holds. ChannelCreationController.authorize (channel-creation.ts:198) throws the native code 'chat-access-denied' for a local precondition. run() passes it to inbox.handleError (464). commandRecovery maps that code to {revalidate, team} (errors.ts:66-71), so handleError increments the team's generation, publishes it as 'unavailable' with the channel-creation message, and disposes every job on the account, the poll and sibling syncs included (677-678). The other two guards use 'access-changed' (send-service 855/990, use-chat-conversation 89). Because generations feed the inbox binding (inbox-service ~403), a generation bump does re-bind the team, and the late authorize('after') then fails on the new entry. The test stub at channel-creation.test.ts:105 does ignore everything except chat-integrity, and the regex is at 469. One claim is incorrect: the sync is not deferred by 30 s. run()'s finally (channel-creation.ts:496) calls inbox.invalidate() in the same synchronous turn, which resets due to 0 and sets dirty. The actual effect is a brief, misleadingly worded 'unavailable' state, cancelled account jobs and a doubled team.retry, so priority drops to low. The recommendation also misses a behavior change. preparationCanChange (actions.ts) includes 'chat-access-denied', so today a 'before' guard failure moves the record to review and then to cancelled, discarding the input. Switching to 'access-changed' would leave the record 'unresolved' instead. That needs a deliberate decision, and the test at channel-creation.test.ts:654, which asserts 'chat-access-denied', needs updating.

</details>

### chat-code-notification-limit-constant

**Move the 256-character notification text bound into chat-limits.json**

- Type: code-quality
- Priority: low
- Effort: S
- Layers: desktop-ui, desktop-native, agent
- Verification: adjusted

The notification-history plaintext bound of 256 Unicode scalar values is a literal in five places: chat-contract.ts:465, notification-consumer.ts:476, notification-policy.ts:34, crates/foks-desktop/src/chat.rs:207 and crates/foks-agent/src/chat.rs:726. chat-mock.ts does not apply it at all. notification-consumer.ts:472 also hard-codes 50 in place of CHAT_PAGE_ROWS. Every other chat bound comes from crates/foks-agent-proto/chat-limits.json, so this one can drift unnoticed.

**Evidence**

- [`apps/desktop/src/chat-contract.ts:465`](../../../apps/desktop/src/chat-contract.ts#L465): 256 literal in decoder
- [`apps/desktop/src/chat/notification-consumer.ts:472`](../../../apps/desktop/src/chat/notification-consumer.ts#L472): 50 literal (472) and 256 literal (476)
- [`apps/desktop/src/chat/notification-policy.ts:34`](../../../apps/desktop/src/chat/notification-policy.ts#L34): slice(0, 256) presentation truncation, not listed in the original finding
- [`crates/foks-desktop/src/chat.rs:207`](../../../crates/foks-desktop/src/chat.rs#L207): 256 literal in native validator
- [`crates/foks-agent/src/chat.rs:726`](../../../crates/foks-agent/src/chat.rs#L726): take(256) where the agent truncates
- [`crates/foks-agent-proto/chat-limits.json:1`](../../../crates/foks-agent-proto/chat-limits.json#L1): shared limit source has no notification bound; build.rs emits any CHAT_* key as a Rust const

**Recommendation**

Add CHAT_NOTIFICATION_TEXT_CHARS: 256 to chat-limits.json; build.rs then emits the Rust constant automatically. Export it from chat-limits.ts and use it at all five sites, including notification-policy.ts's slice, and make chat-mock.ts truncate notification-history text with it. Use CHAT_PAGE_ROWS in the consumer. Add a row to the shared-limits table in book/24-performance.qmd.

<details><summary>Verifier note</summary>

Confirmed: the 256 literal appears at chat-contract.ts:465, notification-consumer.ts:476 (with 50 at 472), crates/foks-desktop/src/chat.rs:207 and crates/foks-agent/src/chat.rs:726. chat-limits.json has no notification bound, and build.rs generates `pub const CHAT_*: usize` for every key. Both Rust crates already consume these through foks_agent_proto::chat, and chat-limits.ts destructures the JSON. book/24-performance.qmd has the shared-limits table at about line 170. The finding misses a fifth site, apps/desktop/src/chat/notification-policy.ts:34, which truncates notification bodies with slice(0, 256). It also misses that chat-mock.ts implements notification-history without the bound. The bound is part of the agent-desktop IPC, not the Go FOKS wire, so the change is compatible. ISSUES.md and the book do not track it.

</details>

### chat-code-pending-row-actions

**Move protocol action selection out of PendingRow and OutgoingRow into the send service**

- Type: maintainability
- Priority: low
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

PendingRow chooses in the component which protocol action to send (attempt, reconcile or cancel), and on failure it sends a status request whose reply it discards. drive() makes its own reconcile-or-attempt choice for outgoing messages using different inputs (observed, unconfirmed phase), and it does apply its status reply. OutgoingRow delegates to service.retry and service.restoreDraft but computes retryable and editable locally. ChatThread renders outgoing messages and orphan operations as two separate lists joined by an outgoingIds filter, and ChatScreen's 'Needs attention' section renders PendingRow again. Rules about when an operation may be retried, checked or cancelled are therefore spread across four sites and already differ.

**Evidence**

- [`apps/desktop/src/chat/pending-row.tsx:54`](../../../apps/desktop/src/chat/pending-row.tsx#L54): run(): component-level action dispatch; status fallback at line 70 discards the reply and does not call onChange
- [`apps/desktop/src/chat/pending-row.tsx:105`](../../../apps/desktop/src/chat/pending-row.tsx#L105): attempt vs reconcile, and cancel availability, chosen in JSX (105-126)
- [`apps/desktop/src/chat/send-service.ts:1016`](../../../apps/desktop/src/chat/send-service.ts#L1016): drive() makes its own reconcile/attempt choice (1016-1036) with different inputs, and applies its status reply (1072-1091)
- [`apps/desktop/src/chat/outgoing-row.tsx:47`](../../../apps/desktop/src/chat/outgoing-row.tsx#L47): retryable and editable computed in the component; actions go through service.retry and service.restoreDraft
- [`apps/desktop/src/chat/chat-thread.tsx:144`](../../../apps/desktop/src/chat/chat-thread.tsx#L144): two parallel row lists joined by outgoingIds (rendered at 368-404)
- [`apps/desktop/src/screens/chat-screen.tsx:444`](../../../apps/desktop/src/screens/chat-screen.tsx#L444): PendingRow rendered again in the 'Needs attention' section

**Recommendation**

Add retryOperation, checkOperation and cancelOperation(store, opId) to ChatSendService. They pick attempt or reconcile with the same predicate drive() uses, apply the status fallback reply to team.operations, and publish. Add a rows(store, channel) view model that combines OutgoingMessage and orphan TrackedOperation, with each row carrying actions: {retry?, check?, cancel?, restore?} computed in the service from a single operation and phase transition table. PendingRow and OutgoingRow then render those actions without choosing protocol verbs or recomputing availability. Drop the unused 'finalize' action from PendingRow.

**Mockup:** [Delivery and integrity](../mockups/chat-delivery-and-integrity.html)

<details><summary>Verifier note</summary>

Confirmed for PendingRow and drive(). pending-row.tsx:54-79 dispatches attempt, cancel or reconcile from the component and issues a status request on failure. The JSX at 105-126 chooses reconcile or attempt, and when cancel is available. send-service.ts:1016-1036 makes its own reconcile-or-attempt choice using different inputs (message.observed and phase 'unconfirmed'), so the two sites already differ. chat-thread.tsx:144-146 builds outgoingIds, and 368-404 renders the outgoing rows and the orphan PendingRows as separate lists. chat-screen.tsx:444 renders PendingRow a second time, in the 'Needs attention' section of the same ChatScreen rather than on a separate screen. The title overstates OutgoingRow's part: it chooses no protocol verbs and calls service.retry and service.restoreDraft (outgoing-row.tsx:36-37), though it does compute retryable and editable locally (47-54). One concrete defect the finding omits: PendingRow's status fallback (line 70) discards the reply and never calls onChange, so a failed action does not refresh the row. drive() applies the status reply (1072-1091). The 'finalize' member of the run() action union is never used. The recommendation also depends on a separate finding (chat-code-outgoing-state-machine), so it should be stated so that it stands on its own.

</details>

### chat-code-render-purity

**Lint does not catch render-time service mutations and ref writes in chat hooks**

- Type: tooling
- Priority: low
- Effort: S
- Layers: desktop-ui, tooling
- Verification: adjusted

Several render paths change external state. ChatScreen render calls sends.drafts() (chat-screen.tsx:135), which calls ensure() (send-service.ts:433) and can create and store a Team record with a chat client. The history() getter (use-chat-conversation.ts:281) calls histories.get(), which reorders the LRU (history-cache.ts:72-78) during render. Refs are assigned during render in five chat modules: chat-new.tsx, use-chat-history.ts, use-chat-conversation.ts, send-provider.tsx and channel-creation-provider.tsx. ESLint enables only rules-of-hooks and exhaustive-deps, although the installed eslint-plugin-react-hooks 7.1.1 includes 'refs'. 'react-hooks/refs' would report 17 sites in the chat scope. 'react-hooks/purity' reports none, because it does not model service method calls, so the drafts()/get() mutations have to be fixed in code. Lint will not catch them. Separately, the ChatScreen draft-reconciliation effect deletes from the service's map directly, without publish().

**Evidence**

- [`apps/desktop/src/screens/chat-screen.tsx:135`](../../../apps/desktop/src/screens/chat-screen.tsx#L135): sends.drafts() called in render; the effect at 140-144 deletes from the returned service map without publish()
- [`apps/desktop/src/chat/send-service.ts:433`](../../../apps/desktop/src/chat/send-service.ts#L433): drafts() (line 425) calls ensure() (line 271), which creates and stores a Team including a chatClient
- [`apps/desktop/src/chat/history-cache.ts:72`](../../../apps/desktop/src/chat/history-cache.ts#L72): get() deletes and re-inserts the entry to update recency
- [`apps/desktop/src/chat/use-chat-conversation.ts:281`](../../../apps/desktop/src/chat/use-chat-conversation.ts#L281): history() calls histories.get(); invoked during render at chat-screen.tsx:368
- [`apps/desktop/src/screens/chat-new.tsx:149`](../../../apps/desktop/src/screens/chat-new.tsx#L149): ref writes during render at 149, 155, 234, 350 and 356; also use-chat-history.ts:68,75, use-chat-conversation.ts:53-54, send-provider.tsx:36, channel-creation-provider.tsx:41
- [`eslint.config.mjs:48`](../../../eslint.config.mjs#L48): only rules-of-hooks (48) and exhaustive-deps (49); plugin 7.1.1 exports refs and purity; refs reports 17 chat-scope sites, purity reports 0

**Recommendation**

Add peek(binding) to ChatHistoryCache that leaves recency unchanged. Use it from history(), and update recency in accept() or an effect. Make drafts() side-effect free by returning a shared empty ReadonlyMap constant instead of calling ensure() (a team with no Team record has no drafts). Better still, move the reconciliation into the service as something like retainDrafts(storeId, liveChannelIds), which deletes entries and publishes, so the ChatScreen effect no longer mutates service state directly. Replace the render-time ref writes with useEffectEvent (React 19.2) or useLayoutEffect assignments. Enable 'react-hooks/refs' as an error for apps/desktop/src/chat/** and src/screens/chat-*.tsx, covering the 17 sites including chat-tab.tsx's sidebar.ref reads, then widen the scope. 'react-hooks/purity' is cheap to enable, but do not rely on it for these service mutations. Add a render test asserting that mounting ChatScreen creates no Team record and leaves history-cache order unchanged.

<details><summary>Verifier note</summary>

The core claims hold. send-service.ts:425-434 drafts() calls ensure() (line 433), which builds and stores a Team record, including a chatClient, and is reached from ChatScreen render at chat-screen.tsx:135. history-cache.ts:72-78 get() deletes and re-inserts the entry to update LRU order, and use-chat-conversation.ts:281-285 calls it from history(), which chat-screen.tsx:368 invokes during render. eslint.config.mjs enables only rules-of-hooks and exhaustive-deps (lines 48-49; line 47 is the comment). eslint-plugin-react-hooks 7.1.1 is installed and exports 'refs' and 'purity', and React is 19.2.8, so useEffectEvent is available. I ran ESLint with both rules on the chat scope using a scratch config. 'refs' reports 17 errors: chat-new.tsx 149, 155, 234, 350 and 356; use-chat-history.ts 68 and 75; use-chat-conversation.ts 53 and 54; send-provider.tsx 36 and 39; channel-creation-provider.tsx 41 and 43; chat-tab.tsx 222-228. The 'five modules' count is correct, but the evidence names only three, and chat-tab.tsx is flagged because it reads sidebar.ref during render. 'purity' reports zero errors: it detects known impure globals such as Date.now and Math.random, not method calls on services. Enabling it therefore would not catch drafts()/ensure() or histories.get(), which is what the title claims. Two smaller points: Object.freeze does not make a Map immutable, and the ChatScreen effect at 140-144 deletes entries from the service-owned drafts map without calling publish().

</details>
